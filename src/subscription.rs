use std::sync::atomic::Ordering;

use futures_util::Stream;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{CustomEvent, PusherClientConnection, SubscribeData, SubscriptionChange};

pub struct PusherClientConnectionSubscription<'a> {
    connection: &'a PusherClientConnection,
    channel: String,
    sender_id: usize,
    rx: UnboundedReceiver<CustomEvent>,
}

impl<'a> PusherClientConnectionSubscription<'a> {
    pub fn new(connection: &'a PusherClientConnection, channel: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let sender_id = connection
            .subscribe_channel_id
            .fetch_add(1, Ordering::Relaxed);
        connection
            .subscribe_queue
            .send(SubscribeData {
                change: SubscriptionChange::Subscribe(sender_id, tx),
                channel: channel.into(),
            })
            .unwrap();
        Self {
            connection,
            channel: channel.into(),
            sender_id,
            rx,
        }
    }
}

impl Stream for PusherClientConnectionSubscription<'_> {
    type Item = CustomEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().rx.poll_recv(cx)
    }
}

impl Drop for PusherClientConnectionSubscription<'_> {
    fn drop(&mut self) {
        self.connection
            .subscribe_queue
            .send(SubscribeData {
                change: SubscriptionChange::Unsubscribe(self.sender_id),
                channel: self.channel.clone(),
            })
            .unwrap();
    }
}
