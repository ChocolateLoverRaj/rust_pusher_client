use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
    usize,
};

use disconnect_handler::disconnect_handler;
pub use error::PusherClientError;
use futures_util::{FutureExt, StreamExt, future::BoxFuture};
use iced_futures::{MaybeSend, MaybeSync};
use nash_ws::WebSocket;
use receiver::receiver;
use sender::sender;
use serde::{Deserialize, Serialize};
use subscription_handler::subscription_handler;
use tokio::{
    sync::{Mutex, mpsc, watch},
    try_join,
};
use tokio_stream::wrappers::WatchStream;

mod disconnect_handler;
mod error;
mod message_to_send;
mod receiver;
mod sender;
mod subscription;
mod subscription_handler;
mod wait_for_connection_established;

pub use subscription::PusherClientConnectionSubscription;
use wait_for_connection_established::wait_for_connection_established;

pub struct PrivateChannelAuthRequest {
    pub socket_id: String,
    pub channel: String,
}

pub struct PresenceChannelAuthRequest {
    pub socket_id: String,
    pub channel: String,
    pub user_data: String,
}

pub enum AuthRequest {
    Private(PrivateChannelAuthRequest),
    Presence(PresenceChannelAuthRequest),
}

pub trait AuthProvider: MaybeSend + MaybeSync {
    fn get_auth_signature(
        &self,
        auth_request: AuthRequest,
    ) -> BoxFuture<Result<String, Box<dyn std::error::Error>>>;
}

pub struct UnimplementedAuthProvider;

type AuthResult = Result<String, Box<dyn std::error::Error>>;

impl AuthProvider for UnimplementedAuthProvider {
    fn get_auth_signature(&self, _auth_request: AuthRequest) -> BoxFuture<AuthResult> {
        async { Err("Authentication not implemented".into()) }.boxed()
    }
}

pub struct Options {
    pub cluster_name: String,
    pub key: String,
    /// Duration since inactivity to send a ping to check that the connection still works
    pub activity_timeout: Duration,
    /// Duration after sending a ping after which it is assumed that the connection is disconnected
    pub pong_timeout: Duration,
    pub auth_provider: Box<dyn AuthProvider>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionInfo {
    socket_id: String,
    activity_timeout: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherServerEvent {
    event: String,
    data: String,
    channel: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherClientEvent<T> {
    event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

#[derive(Debug, Default, Clone)]
pub struct NotConnectedState {
    pub error: Option<Arc<PusherClientError>>,
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
    NotConnected(NotConnectedState),
    Connecting,
    Connected(ConnectionInfo),
    Disconnecting,
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherSubscribeEvent {
    channel: String,
}

#[derive(Debug, Clone)]
pub struct CustomEventData {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    Connecting,
    SuccessfullySubscribed,
    Disconnected,
    Event(CustomEventData),
}

#[derive(Debug, Clone, Copy)]
enum ConnectOperation {
    Connect,
    Disconnect,
}

enum SubscribeActionType {
    Subscribe,
    Unsubscribe,
}

struct SubscribeAction {
    action_type: SubscribeActionType,
    channel: String,
    sender: Arc<mpsc::UnboundedSender<SubscriptionEvent>>,
}

type Subscription = Vec<Arc<mpsc::UnboundedSender<SubscriptionEvent>>>;

type Subscriptions = Arc<Mutex<HashMap<String, Subscription>>>;

#[derive(Clone)]
pub struct PusherClientConnection {
    options: Arc<Options>,
    connect_queue: watch::Sender<Option<ConnectOperation>>,
    state: watch::Receiver<ConnectionState>,
    subscribe_actions: mpsc::UnboundedSender<SubscribeAction>,
}

impl PusherClientConnection {
    pub fn new(options: Options) -> (impl Future<Output = ()> + MaybeSend, Self) {
        // let subscription_succeeded_channels = SubscriptionSucceededChannels::default();
        // let event_channels = EventChannels::default();
        let (connect_queue_tx, connect_queue_rx) = watch::channel(None);
        let (state_tx, state_rx) =
            watch::channel(ConnectionState::NotConnected(Default::default()));
        let (subscribe_actions_tx, mut subscribe_actions_rx) = mpsc::unbounded_channel();

        let connection = Self {
            options: Arc::new(options),
            // event_channels: event_channels.clone(),
            // subscription_succeeded_channels: subscription_succeeded_channels.clone(),
            // send_queue: send_queue_sender.clone(),
            connect_queue: connect_queue_tx,
            state: state_rx,
            // subscriptions: subscriptions.clone(),
            subscribe_actions: subscribe_actions_tx,
        };
        (
            {
                let connection = connection.clone();
                async move {
                    let mut connect_queue_rx = WatchStream::new(connect_queue_rx);
                    let subscriptions = Subscriptions::default();
                    // Includes channels that we've sent pusher:subscribe to
                    let mut subscribed_channels = Mutex::<HashSet<String>>::default();
                    loop {
                        if let Some(ConnectOperation::Connect) =
                            connect_queue_rx.next().await.unwrap()
                        {
                            let error = (async || {
                                let mut subscribed_channels = subscribed_channels.lock().await;
                                let (message_tx, message_rx) = mpsc::unbounded_channel();
                                state_tx.send_replace(ConnectionState::Connecting);
                                let (write_stream, mut read_stream) = WebSocket::new(&format!(
                                    "wss://ws-{}.pusher.com/app/{}?protocol=7&client=Rust-{}?version={}",
                                    connection.options.cluster_name,
                                    connection.options.key,
                                    env!("CARGO_PKG_NAME"),
                                    env!("CARGO_PKG_VERSION")
                                ))
                                .await
                                .map_err(PusherClientError::WebSocketError)?;
                                // Wait until we get the pusher:connection_established event
                                let connection_info = wait_for_connection_established(&mut read_stream).await?;
                                let pusher_requested_activity_timeout = Duration::from_secs(connection_info.activity_timeout);
                                let socket_id = connection_info.socket_id.clone();
                                state_tx.send_replace(ConnectionState::Connected(connection_info));
                                // let authorization_futures: Mutex<HashMap<String, Box<dyn Future<Output = String> + Send>>> = Default::default();
                                let sender_future = sender(write_stream, message_rx, state_tx.clone());
                                let receiver_future = receiver(&connection, pusher_requested_activity_timeout, read_stream, &subscriptions, &message_tx);
                                let subscriptions_future = subscription_handler(&message_tx, subscriptions.clone(), &mut subscribed_channels, &mut subscribe_actions_rx, &socket_id, &connection.options.auth_provider);
                                let disconnect_future = disconnect_handler(&mut connect_queue_rx, &message_tx).map(Ok::<_, PusherClientError>);
                                try_join!(sender_future, receiver_future, subscriptions_future, disconnect_future)?;
                                Ok::<_, PusherClientError>(())
                            })().await.err();
                            state_tx.send_replace(ConnectionState::NotConnected(
                                NotConnectedState {
                                    error: error.map(Arc::new),
                                },
                            ));
                            for subscribed_channel in subscribed_channels.get_mut().drain() {
                                for sender in
                                    subscriptions.lock().await.get(&subscribed_channel).unwrap()
                                {
                                    sender.send(SubscriptionEvent::Disconnected).unwrap();
                                }
                            }
                        }
                    }
                }
            },
            connection,
        )
    }

    pub fn state(&self) -> &watch::Receiver<ConnectionState> {
        &self.state
    }

    pub fn connect(&self) {
        self.connect_queue
            .send_replace(Some(ConnectOperation::Connect));
    }

    pub fn disconnect(&self) {
        self.connect_queue
            .send_replace(Some(ConnectOperation::Disconnect));
    }

    pub fn subscribe(&self, channel: &str) -> PusherClientConnectionSubscription {
        PusherClientConnectionSubscription::new(self.clone(), channel)
    }
}
