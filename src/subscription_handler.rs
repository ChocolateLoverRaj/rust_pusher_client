use std::{collections::HashSet, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    AuthProvider, AuthRequest, PrivateChannelAuthRequest, PusherClientError, SubscribeAction,
    SubscribeActionType, SubscriptionEvent, Subscriptions,
    message_to_send::{MessageToSend, SubscribeMessage, SubscribeNormalChannel},
};

pub async fn subscription_handler(
    message_tx: &mpsc::UnboundedSender<MessageToSend>,
    subscriptions: Subscriptions,
    subscribed_channels: &mut HashSet<String>,
    subscribe_actions_rx: &mut mpsc::UnboundedReceiver<SubscribeAction>,
    socket_id: &str,
    auth_provider: &Box<dyn AuthProvider>,
) -> Result<(), PusherClientError> {
    let subscribe = |channel: String| {
        let is_private = channel.starts_with("private-");
        let is_presence = channel.starts_with("presence-");
        if is_private || is_presence {
            let auth_request = if is_private {
                AuthRequest::Private(PrivateChannelAuthRequest {
                    channel,
                    socket_id: socket_id.to_owned(),
                })
            } else {
                todo!("Presence authentication")
            };
            auth_provider.get_auth_signature(auth_request);
        } else {
            message_tx
                .send(MessageToSend::Subscribe(SubscribeMessage::Normal(
                    SubscribeNormalChannel { channel },
                )))
                .unwrap();
        }
    };

    // Re-subscribe to every channel that has a subscription
    for (channel, subscription) in subscriptions.lock().await.iter() {
        subscribe(channel.to_owned());
        subscribed_channels.insert(channel.clone());
        for sender in subscription {
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
            match subscribe_action.action_type {
                SubscribeActionType::Subscribe => {
                    let subscription = subscriptions
                        .entry(subscribe_action.channel.clone())
                        .or_insert(Default::default());
                    let channel_key = subscription.len();
                    subscription.insert(channel_key, subscribe_action.sender);
                }
                SubscribeActionType::Unsubscribe => {
                    let subscription = subscriptions.get_mut(&subscribe_action.channel).unwrap();
                    subscription.swap_remove(
                        subscription
                            .iter()
                            .position(|sender| Arc::ptr_eq(sender, &subscribe_action.sender))
                            .unwrap(),
                    );
                    if subscription.is_empty() {
                        subscriptions.remove(&subscribe_action.channel);
                    }
                }
            }
        }

        // Actually subscribe / unsubscribe
        for subscribed_channel in subscribed_channels.clone() {
            if !subscriptions.contains_key(&subscribed_channel) {
                subscribed_channels.remove(&subscribed_channel);
                message_tx
                    .send(MessageToSend::Unsubscribe(subscribed_channel))
                    .unwrap();
            }
        }
        for (channel, subscription) in subscriptions.iter() {
            if !subscribed_channels.contains(channel) {
                subscribe(channel.to_owned());
                subscribed_channels.insert(channel.to_owned());
                for sender in subscription {
                    sender.send(SubscriptionEvent::Connecting).unwrap();
                }
            }
        }
    }
    Ok(())
}
