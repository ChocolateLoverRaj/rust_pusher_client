use std::{collections::HashSet, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    PusherClientError, SubscribeAction, SubscribeActionData, SubscribeActionType, Subscription,
    SubscriptionEvent, Subscriptions,
};

pub async fn subscription_handler(
    auth_tx: &mpsc::UnboundedSender<SubscribeActionData>,
    subscriptions: Subscriptions,
    subscribed_channels: &mut HashSet<String>,
    subscribe_actions_rx: &mut mpsc::UnboundedReceiver<SubscribeAction>,
) -> Result<(), PusherClientError> {
    // Re-subscribe to every channel that has a subscription
    for (channel, subscription) in subscriptions.lock().await.iter() {
        auth_tx
            .send(SubscribeActionData {
                channel: channel.clone(),
                action_type: SubscribeActionType::Subscribe(subscription.data.clone()),
            })
            .unwrap();
        subscribed_channels.insert(channel.clone());
        for sender in &subscription.senders {
            let _ = sender.send(SubscriptionEvent::Connecting);
        }
    }

    loop {
        let mut subscribe_actions = Vec::default();
        let count = subscribe_actions_rx
            .recv_many(&mut subscribe_actions, usize::MAX)
            .await;
        if count == 0 {
            break;
        }
        let mut subscriptions = subscriptions.lock().await;
        // Update senders
        for subscribe_action in subscribe_actions {
            match subscribe_action.action.action_type {
                SubscribeActionType::Subscribe(channel_subscribe) => {
                    let subscription = subscriptions
                        .entry(subscribe_action.action.channel.clone())
                        .or_insert(Subscription {
                            data: channel_subscribe,
                            senders: Default::default(),
                        });
                    subscription.senders.push(subscribe_action.sender);
                }
                SubscribeActionType::Unsubscribe => {
                    let subscription = subscriptions
                        .get_mut(&subscribe_action.action.channel)
                        .unwrap();
                    subscription.senders.swap_remove(
                        subscription
                            .senders
                            .iter()
                            .position(|sender| Arc::ptr_eq(sender, &subscribe_action.sender))
                            .unwrap(),
                    );
                    if subscription.senders.is_empty() {
                        subscriptions.remove(&subscribe_action.action.channel);
                    }
                }
            }
        }

        // Actually subscribe / unsubscribe
        for subscribed_channel in subscribed_channels.clone() {
            if !subscriptions.contains_key(&subscribed_channel) {
                subscribed_channels.remove(&subscribed_channel);
                auth_tx
                    .send(SubscribeActionData {
                        channel: subscribed_channel,
                        action_type: SubscribeActionType::Unsubscribe,
                    })
                    .unwrap();
            }
        }
        for (channel, subscription) in subscriptions.iter() {
            if !subscribed_channels.contains(channel) {
                auth_tx
                    .send(SubscribeActionData {
                        channel: channel.clone(),
                        action_type: SubscribeActionType::Subscribe(subscription.data.clone()),
                    })
                    .unwrap();
                subscribed_channels.insert(channel.to_owned());
                for sender in &subscription.senders {
                    sender.send(SubscriptionEvent::Connecting).unwrap();
                }
            }
        }
    }
    Ok(())
}
