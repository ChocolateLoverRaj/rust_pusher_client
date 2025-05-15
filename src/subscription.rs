use futures_util::Stream;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver},
    watch,
};

use crate::{
    CustomEventData, PusherClientConnection, Subscription, SubscriptionStatus, SubscriptionsRefresh,
};

pub struct PusherClientConnectionSubscription<'a> {
    connection: &'a PusherClientConnection,
    channel: String,
    channel_key: usize,
    rx: UnboundedReceiver<CustomEventData>,
    status: watch::Receiver<SubscriptionStatus>,
    unsubscribed: bool,
}

impl<'a> PusherClientConnectionSubscription<'a> {
    pub async fn new(connection: &'a PusherClientConnection, channel: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut subscriptions = connection.subscriptions.lock().await;
        let subscription = subscriptions.entry(channel.into()).or_insert(Subscription {
            status: watch::channel(SubscriptionStatus::NotConnected).0,
            senders: Default::default(),
        });
        if subscription.senders.is_empty() {
            connection
                .subscriptions_refresh
                .send(SubscriptionsRefresh::Subscribe(channel.into()))
                .unwrap();
        }
        let channel_key = subscription.senders.len();
        subscription.senders.insert(channel_key, tx);

        Self {
            connection,
            channel: channel.into(),
            channel_key,
            rx,
            status: subscription.status.subscribe(),
            unsubscribed: false,
        }
    }

    pub fn status(&self) -> &watch::Receiver<SubscriptionStatus> {
        &self.status
    }

    pub async fn unsubscribe(mut self) {
        let mut subscriptions = self.connection.subscriptions.lock().await;
        let subscription = subscriptions.get_mut(&self.channel).unwrap();
        subscription.senders.remove(&self.channel_key);
        println!("Senders: {:#?}", subscription.senders);
        if subscription.senders.is_empty() {
            subscriptions.remove(&self.channel);
            self.connection
                .subscriptions_refresh
                .send(SubscriptionsRefresh::Unsubscribe(self.channel.clone()))
                .unwrap();
        }
        self.unsubscribed = true;
    }
}

impl Stream for PusherClientConnectionSubscription<'_> {
    type Item = CustomEventData;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().rx.poll_recv(cx)
    }
}

impl Drop for PusherClientConnectionSubscription<'_> {
    fn drop(&mut self) {
        if !self.unsubscribed {
            panic!("Subscription dropped without unsubscribing!");
        }
    }
}
