use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
    usize,
};

use futures_util::{
    FutureExt, SinkExt, StreamExt,
    future::{Either, select},
    lock::Mutex,
};
use serde::{Deserialize, Serialize};
use subscription::PusherClientConnectionSubscription;
use thiserror::Error;
use tokio::{
    sync::{
        mpsc::{self, unbounded_channel},
        watch,
    },
    time::{Instant, timeout, timeout_at},
    try_join,
};
use tokio_stream::wrappers::WatchStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, protocol::CloseFrame},
};

pub mod subscription;

pub struct Options {
    pub cluster_name: String,
    pub key: String,
    /// Duration since inactivity to send a ping to check that the connection still works
    pub activity_timeout: Duration,
    /// Duration after sending a ping after which it is assumed that the connection is disconnected
    pub pong_timeout: Duration,
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
    data: T,
}

#[derive(Debug)]
pub struct UnexpectedEventError {
    pub channel: String,
    pub event: String,
}

#[derive(Debug, Error)]
pub enum PusherClientError {
    #[error("The WebSocket read stream ended")]
    StreamEnded,
    #[error("WebSocket error when reading data")]
    WebSocketError(tokio_tungstenite::tungstenite::Error),
    #[error("Received binary data from Pusher that was unexpected")]
    UnexpectedBinaryData,
    #[error("Received a pong when we shouldn't've received one")]
    UnexpectedPong,
    #[error("Received JSON from Pusher in an invalid format")]
    JsonParseError(serde_json::Error),
    #[error("Received data from Pusher in an invalid format")]
    ParseError,
    #[error("Received an unexpected subscribe succeeded event")]
    UnexpectedSubscribeSucceededEvent(String),
    #[error("Received an unexpected custom event")]
    UnexpectedEvent(UnexpectedEventError),
    #[error("Error sending a pong after receiving a ping from Pusher")]
    PongError(tokio_tungstenite::tungstenite::Error),
    #[error("WebSocket connection closed")]
    ConnectionClosed(Option<CloseFrame>),
    #[error("This error happened because of a different error elsewhere")]
    LibraryError,
    #[error("This library sent a ping, but no pong was received from Pusher")]
    PongTimeout,
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
    sender: Rc<mpsc::UnboundedSender<SubscriptionEvent>>,
}

type Subscription = Vec<Rc<mpsc::UnboundedSender<SubscriptionEvent>>>;

type Subscriptions = Rc<Mutex<HashMap<String, Subscription>>>;

#[derive(Clone)]
pub struct PusherClientConnection {
    options: Rc<Options>,
    // subscription_succeeded_channels: SubscriptionSucceededChannels,
    // event_channels: EventChannels,
    // send_queue: tokio::sync::mpsc::UnboundedSender<Message>,
    // last_message_received: tokio::sync::watch::Receiver<Instant>,
    connect_queue: watch::Sender<Option<ConnectOperation>>,
    state: watch::Receiver<ConnectionState>,
    // subscriptions: Subscriptions,
    subscribe_actions: mpsc::UnboundedSender<SubscribeAction>,
}

impl PusherClientConnection {
    pub fn new(options: Options) -> (impl Future<Output = ()>, Self) {
        // let subscription_succeeded_channels = SubscriptionSucceededChannels::default();
        // let event_channels = EventChannels::default();
        let (connect_queue_tx, connect_queue_rx) = watch::channel(None);
        let (state_tx, state_rx) =
            watch::channel(ConnectionState::NotConnected(Default::default()));
        let (subscribe_actions_tx, mut subscribe_actions_rx) = mpsc::unbounded_channel();

        let connection = Self {
            options: Rc::new(options),
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
                    let mut subscribed_channels = RefCell::<HashSet<String>>::default();
                    loop {
                        if let Some(ConnectOperation::Connect) =
                            connect_queue_rx.next().await.unwrap()
                        {
                            let error = (async || {
                                let mut subscribed_channels = subscribed_channels.borrow_mut();
                                let (send_queue_sender, mut receive_queue_receiver) = unbounded_channel();
                                state_tx.send_replace(ConnectionState::Connecting);
                                let (web_socket, _response) = connect_async(format!(
                                    "wss://ws-{}.pusher.com/app/{}?protocol=7&client=Rust-{}?version={}",
                                    connection.options.cluster_name,
                                    connection.options.key,
                                    env!("CARGO_PKG_NAME"),
                                    env!("CARGO_PKG_VERSION")
                                ))
                                .await
                                .map_err(PusherClientError::WebSocketError)?;
                                // Wait until we get the pusher:connection_established event
                                let (mut write_stream, mut read_stream) = web_socket.split();
                                let text = loop {
                                    let message = read_stream
                                        .next()
                                        .await
                                        .ok_or(PusherClientError::StreamEnded)?
                                        .map_err(PusherClientError::WebSocketError)?;
                                    match message {
                                        Message::Text(text) => break text,
                                        Message::Binary(_) => return Err(PusherClientError::UnexpectedBinaryData),
                                        Message::Ping(data) => {
                                            write_stream
                                                .send(Message::Pong(data))
                                                .await
                                                .map_err(PusherClientError::PongError)?;
                                        }
                                        Message::Pong(_) => {
                                            // We never sent a ping, so we shouldn't receive a pong
                                            return Err(PusherClientError::UnexpectedPong);
                                        }
                                        Message::Close(close_frame) => {
                                            return Err(PusherClientError::ConnectionClosed(close_frame));
                                        }
                                        Message::Frame(_) => unreachable!(),
                                    }
                                };
                                let event = serde_json::from_str::<PusherServerEvent>(text.as_str())
                                    .map_err(PusherClientError::JsonParseError)?;
                                if event.event != "pusher:connection_established" {
                                    Err(PusherClientError::ParseError)?;
                                }
                                let connection_info = serde_json::from_str::<ConnectionInfo>(&event.data)
                                    .map_err(PusherClientError::JsonParseError)?;
                                let pusher_requested_activity_timeout = Duration::from_secs(connection_info.activity_timeout);
                                state_tx.send_replace(ConnectionState::Connected(connection_info));
                                let sender_future = async {
                                    // Re-subscribe to every channel that has a subscription
                                    for (channel, subscription) in subscriptions.lock().await.iter() {
                                        write_stream.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                            event: "pusher:subscribe".into(),
                                            data: PusherSubscribeEvent {
                                                channel: channel.clone(),
                                            },
                                        }).unwrap().into())).await
                                        .map_err(PusherClientError::WebSocketError)?;
                                        subscribed_channels.insert(channel.clone());
                                        for sender in subscription {
                                            sender.send(SubscriptionEvent::Connecting).unwrap()
                                        }
                                    }

                                    loop {
                                        match select(async {
                                            let mut subscribe_actions = Vec::default();
                                            subscribe_actions_rx.recv_many(&mut subscribe_actions, usize::MAX).await; subscribe_actions }.boxed_local(), receive_queue_receiver.recv().boxed_local()).await {
                                        Either::Left((subscribe_actions, _)) => {
                                            let mut subscriptions = subscriptions.lock().await;

                                            // Update senders
                                            for subscribe_action in subscribe_actions {
                                                match subscribe_action.action_type {
                                                    SubscribeActionType::Subscribe => {
                                                        let subscription = subscriptions.entry(subscribe_action.channel.clone()).or_insert(Default::default());
                                                        let channel_key = subscription.len();
                                                        subscription.insert(channel_key, subscribe_action.sender);
                                                    },
                                                    SubscribeActionType::Unsubscribe => {
                                                        let subscription = subscriptions.get_mut(&subscribe_action.channel).unwrap();
                                                        subscription.swap_remove(subscription
                                                            .iter()
                                                            .position(|sender|
                                                                Rc::ptr_eq(sender, &subscribe_action.sender)).unwrap());
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
                                                    write_stream.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                                        event: "pusher:unsubscribe".into(),
                                                        data: PusherSubscribeEvent {
                                                            channel: subscribed_channel,
                                                        },
                                                    }).unwrap().into())).await
                                                    .map_err(PusherClientError::WebSocketError)?;
                                                }
                                            }
                                            for (channel, subscription) in subscriptions.iter() {
                                                if !subscribed_channels.contains(channel) {
                                                    write_stream.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                                        event: "pusher:subscribe".into(),
                                                        data: PusherSubscribeEvent {
                                                            channel: channel.clone(),
                                                        },
                                                    }).unwrap().into())).await
                                                    .map_err(PusherClientError::WebSocketError)?;
                                                    subscribed_channels.insert(channel.to_owned());
                                                    for sender in subscription {
                                                        sender.send(SubscriptionEvent::Connecting).unwrap();
                                                    }
                                                }
                                            }
                                        },
                                        Either::Right((message, _)) => {
                                            write_stream
                                                .send(message.unwrap())
                                                .await
                                                .map_err(PusherClientError::WebSocketError)?;
                                        }
                                    }
                                    }
                                    #[allow(unreachable_code)]
                                    Ok::<_, PusherClientError>(())
                                };
                                let receiver_future = async {
                                    let mut last_message_received = Instant::now();
                                    loop {
                                        let message = {
                                            match timeout_at(
                                                last_message_received.checked_add(connection.options.activity_timeout.min(pusher_requested_activity_timeout)).unwrap(),
                                                read_stream.next(),
                                            )
                                            .await
                                            {
                                                Ok(message) => {
                                                    let message = message
                                                        .ok_or(PusherClientError::StreamEnded)?
                                                        .map_err(PusherClientError::WebSocketError)?;
                                                    message
                                                }
                                                Err(_) => {
                                                    // println!("Sending a ping");
                                                    send_queue_sender
                                                        .send(Message::Ping(Default::default()))
                                                        .map_err(|_| PusherClientError::LibraryError)?;
                                                    let message =
                                                        timeout(connection.options.pong_timeout, read_stream.next())
                                                            .await
                                                            .map_err(|_| {
                                                                PusherClientError::PongTimeout
                                                            })?
                                                            .ok_or(PusherClientError::StreamEnded)?
                                                            .map_err(
                                                                PusherClientError::WebSocketError,
                                                            )?;
                                                    message
                                                }
                                            }
                                        };
                                        last_message_received = Instant::now();
                                        match message {
                                            Message::Binary(_) => {
                                                Err(PusherClientError::UnexpectedBinaryData)?;
                                            }
                                            Message::Text(text) => {
                                                // println!("Received text");
                                                let event =
                                                    serde_json::from_str::<PusherServerEvent>(
                                                        text.as_str(),
                                                    )
                                                    .map_err(PusherClientError::JsonParseError)?;
                                                if event.event
                                                    == "pusher_internal:subscription_succeeded"
                                                {
                                                    let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                                                    // println!("Successfully subscribed to {:?}", channel);
                                                    if let Some(subscription) = subscriptions.lock().await.get(&channel) {
                                                        for sender in subscription {
                                                            sender.send(SubscriptionEvent::SuccessfullySubscribed).unwrap();
                                                        }
                                                    }
                                                } else {
                                                    let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                                                    // println!("Event on channel: {:?}", channel);
                                                    if let Some(subscription) = subscriptions.lock().await.get_mut(&channel) {
                                                        subscription
                                                            .iter()
                                                            .for_each(|sender| sender.send(SubscriptionEvent::Event(CustomEventData { event: event.event.clone(), data: event.data.clone() })).unwrap());
                                                    }
                                                }
                                            }
                                            Message::Ping(data) => {
                                                // println!("Received ping");
                                                let _ =
                                                    send_queue_sender.send(Message::Pong(data));
                                            }
                                            Message::Pong(_) => {
                                                // We just use pong to know that the connection is still alive
                                                // We already updated the last message received
                                                // So no need to do anything here
                                            }
                                            Message::Close(close_frame) => {
                                                Err(PusherClientError::ConnectionClosed(
                                                    close_frame,
                                                ))?
                                            }
                                            Message::Frame(_) => unreachable!(),
                                        }
                                    }
                                    #[allow(unreachable_code)]
                                    Ok::<_, PusherClientError>(())
                                };
                                select(
                                    async {
                                        loop {
                                            if let Some(ConnectOperation::Disconnect) = connect_queue_rx.next().await.unwrap() {
                                                // println!("Disconnected because library was asked to (without error)");
                                                break;
                                            }
                                        }
                                        Ok(())
                                    }.boxed_local(),
                                    async {
                                    try_join!(sender_future, receiver_future)?;
                                    Ok::<_, PusherClientError>(())
                                }.boxed_local()).await.factor_first().0
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
        PusherClientConnectionSubscription::new(self, channel)
    }
}
