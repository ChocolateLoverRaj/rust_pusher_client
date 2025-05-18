use std::sync::Arc;

use futures_util::Stream;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{
    ChannelSubscribe, PusherClientConnection, SubscribeAction, SubscribeActionData,
    SubscribeActionType, SubscriptionEvent,
};

pub struct PusherClientConnectionSubscription {
    connection: PusherClientConnection,
    channel: String,
    sender: Arc<mpsc::UnboundedSender<SubscriptionEvent>>,
    receiver: UnboundedReceiver<SubscriptionEvent>,
}

impl PusherClientConnectionSubscription {
    pub(crate) fn new(
        connection: PusherClientConnection,
        channel: &str,
        channel_type: ChannelSubscribe,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let sender = Arc::new(sender);
        connection
            .subscribe_actions
            .send(SubscribeAction {
                action: SubscribeActionData {
                    channel: channel.to_owned(),
                    action_type: SubscribeActionType::Subscribe(channel_type),
                },
                sender: sender.clone(),
            })
            .unwrap();
        Self {
            connection,
            channel: channel.to_owned(),
            receiver,
            sender,
        }
    }
}

impl Stream for PusherClientConnectionSubscription {
    type Item = SubscriptionEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().receiver.poll_recv(cx)
    }
}

impl Drop for PusherClientConnectionSubscription {
    fn drop(&mut self) {
        self.connection
            .subscribe_actions
            .send(SubscribeAction {
                action: SubscribeActionData {
                    channel: self.channel.clone(),
                    action_type: SubscribeActionType::Unsubscribe,
                },
                sender: self.sender.clone(),
            })
            .unwrap();
    }
}
