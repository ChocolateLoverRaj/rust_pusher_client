use std::rc::Rc;

use futures_util::Stream;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{PusherClientConnection, SubscribeAction, SubscribeActionType, SubscriptionEvent};

pub struct PusherClientConnectionSubscription<'a> {
    connection: &'a PusherClientConnection,
    channel: String,
    sender: Rc<mpsc::UnboundedSender<SubscriptionEvent>>,
    receiver: UnboundedReceiver<SubscriptionEvent>,
}

impl<'a> PusherClientConnectionSubscription<'a> {
    pub(crate) fn new(connection: &'a PusherClientConnection, channel: &str) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let sender = Rc::new(sender);
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

impl Stream for PusherClientConnectionSubscription<'_> {
    type Item = SubscriptionEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().receiver.poll_recv(cx)
    }
}

impl Drop for PusherClientConnectionSubscription<'_> {
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
