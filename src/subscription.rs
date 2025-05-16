use std::sync::Arc;

use futures_util::Stream;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{PusherClientConnection, SubscribeAction, SubscribeActionType, SubscriptionEvent};

pub struct PusherClientConnectionSubscription {
    connection: PusherClientConnection,
    channel: String,
    sender: Arc<mpsc::UnboundedSender<SubscriptionEvent>>,
    receiver: UnboundedReceiver<SubscriptionEvent>,
}

impl PusherClientConnectionSubscription {
    pub(crate) fn new(connection: PusherClientConnection, channel: &str) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let sender = Arc::new(sender);
        connection
            .subscribe_actions
            .send(SubscribeAction {
                action_type: SubscribeActionType::Subscribe,
                channel: channel.to_owned(),
                sender: sender.clone(),
            })
            .unwrap();
        Self {
            connection,
            channel: channel.into(),
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
                action_type: SubscribeActionType::Unsubscribe,
                channel: self.channel.clone(),
                sender: self.sender.clone(),
            })
            .unwrap();
    }
}
