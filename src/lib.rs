use std::{collections::HashMap, rc::Rc, sync::atomic::AtomicUsize, time::Duration};

use futures_util::{
    future::try_join3, lock::Mutex, SinkExt, Stream, StreamExt
};
use serde::{Deserialize, Serialize};
use subscription::PusherClientConnectionSubscription;
use thiserror::Error;
use tokio::{
    sync::{
        mpsc::{self, error::TrySendError, unbounded_channel}, watch,
    },
    time::{Instant, timeout, timeout_at},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, protocol::CloseFrame},
};

mod subscription;

pub struct Options {
    pub cluster_name: String,
    pub key: String,
    /// Duration since inactivity to send a ping to check that the connection still works
    pub activity_timeout: Duration,
    /// Duration after sending a ping after which it is assumed that the connection is disconnected
    pub pong_timeout: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Default)]
pub struct NotConnectedState {
    pub error: Option<PusherClientError>,
}

#[derive(Debug)]
pub enum ConnectionState {
    NotConnected(NotConnectedState),
    Connecting,
    Connected(ConnectionInfo),
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherSubscribeEvent {
    channel: String,
}

enum SubscriptionChange {
    Subscribe(usize, mpsc::UnboundedSender<CustomEvent>),
    Unsubscribe(usize),
}

#[derive(Debug)]
pub struct CustomEventData {
    pub event: String,
    pub data: String
}

#[derive(Debug)]
pub enum CustomEvent {
    SuccessfullySubscribed,
    /// Because we're no longer connected to Pusher, we are effectively unsubscribed. But on reconnect, it will subscribe again.
    EffectivelyUnsubscribed,
    Event(CustomEventData),
}

struct SubscribeData {
    change: SubscriptionChange,
    channel: String,
}

#[derive(Clone)]
pub struct PusherClientConnection {
    options: Rc<Options>,
    // subscription_succeeded_channels: SubscriptionSucceededChannels,
    // event_channels: EventChannels,
    // send_queue: tokio::sync::mpsc::UnboundedSender<Message>,
    // last_message_received: tokio::sync::watch::Receiver<Instant>,
    connect_queue: tokio::sync::mpsc::Sender<()>,
    state: watch::Receiver<ConnectionState>,
    subscribe_queue: mpsc::UnboundedSender<SubscribeData>,
    subscribe_channel_id: Rc<AtomicUsize>
}

impl PusherClientConnection {
    pub fn new(options: Options) -> (impl Future<Output = ()>, Self) {
        let (send_queue_sender, mut receive_queue_receiver) = unbounded_channel();
        // let subscription_succeeded_channels = SubscriptionSucceededChannels::default();
        // let event_channels = EventChannels::default();
        let (connect_queue_tx, mut connect_queue_rx) = mpsc::channel(1);
        let (state_tx, state_rx) =
            watch::channel(ConnectionState::NotConnected(Default::default()));
        let (subscribe_tx, mut subscribe_rx) = mpsc::unbounded_channel();

        let connection = Self {
            options: Rc::new(options),
            // event_channels: event_channels.clone(),
            // subscription_succeeded_channels: subscription_succeeded_channels.clone(),
            // send_queue: send_queue_sender.clone(),
            connect_queue: connect_queue_tx,
            state: state_rx,
            subscribe_queue: subscribe_tx,
            subscribe_channel_id: Default::default()
        };
        (
            {
                let connection = connection.clone();
                async move {
                    #[derive(Debug, Default)]
                    struct ChannelSubscriptions {
                        is_successfully_subscribed: bool,
                        senders: HashMap<usize, mpsc::UnboundedSender<CustomEvent>>
                    }
                    let mut subscription_senders = Mutex::new(HashMap::<String, ChannelSubscriptions>::default());
                    loop {
                        connect_queue_rx.recv().await.unwrap();
                        let error = (async || {
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
                            state_tx.send_replace(ConnectionState::Connected(connection_info));
                            // Re-subscribe to every channel that has a subscription
                            // TODO: Since we are not processing the subscribe channel, channels could get unnecessarily subscribed to and immediately unsubscribed from if the subscription unsubscribed while we were disconnected
                            subscription_senders.get_mut().keys().for_each(|channel| {
                                send_queue_sender.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                    event: "pusher:subscribe".into(),
                                    data: PusherSubscribeEvent {
                                        channel: channel.clone(),
                                    },
                                }).unwrap().into())).unwrap();
                            });
                            let sender_future = async {
                                while let Some(message) = receive_queue_receiver.recv().await {
                                    write_stream
                                        .send(message)
                                        .await
                                        .map_err(PusherClientError::WebSocketError)?;
                                }
                                Result::<_, PusherClientError>::Ok(())
                            };
                            let receiver_future = async {
                                let mut last_message_received = Instant::now();
                                loop {
                                    let message = {
                                        match timeout_at(
                                            last_message_received.checked_add(connection.options.activity_timeout).unwrap(),
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
                                            let event =
                                                serde_json::from_str::<PusherServerEvent>(
                                                    text.as_str(),
                                                )
                                                .map_err(PusherClientError::JsonParseError)?;
                                            if event.event
                                                == "pusher_internal:subscription_succeeded"
                                            {
                                                let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                                                if let Some(subscriptions) = subscription_senders.lock().await.get_mut(&channel) {
                                                    subscriptions.is_successfully_subscribed = true;
                                                    subscriptions.senders.values().for_each(|value| value.send(CustomEvent::SuccessfullySubscribed).unwrap());
                                                }
                                            } else {
                                                let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                                                if let Some(subscriptions) = subscription_senders.lock().await.get_mut(&channel) {
                                                    subscriptions.is_successfully_subscribed = true;
                                                    subscriptions.senders.values()
                                                        .for_each(|value| value.send(CustomEvent::Event(CustomEventData { event: event.event.clone(), data: event.data.clone() })).unwrap());
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
                                Ok(())
                            };
                            let subscribe_future = async {
                                while let Some(SubscribeData { change, channel }) = subscribe_rx.recv().await {
                                    let mut subscription_senders = subscription_senders.lock().await;
                                    let subscriptions = subscription_senders.entry(channel.clone()).or_default();
                                    match change {
                                        SubscriptionChange::Subscribe(sender_id, sender) => {
                                            if subscriptions.senders.is_empty() {
                                                send_queue_sender.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                                    event: "pusher:subscribe".into(),
                                                    data: PusherSubscribeEvent {
                                                        channel: channel.clone(),
                                                    },
                                                }).unwrap().into())).unwrap();
                                            }
                                            if subscriptions.is_successfully_subscribed {
                                                sender.send(CustomEvent::SuccessfullySubscribed).unwrap();
                                            }
                                            subscriptions.senders.insert(sender_id, sender);
                                        },
                                        SubscriptionChange::Unsubscribe(sender_id) => {
                                            subscriptions.senders.remove(&sender_id);
                                            if subscriptions.senders.is_empty() {
                                                send_queue_sender.send(Message::Text(serde_json::to_string(&PusherClientEvent {
                                                    event: "pusher:unsubscribe".into(),
                                                    data: PusherSubscribeEvent {
                                                        channel: channel.clone(),
                                                    },
                                                }).unwrap().into())).unwrap();
                                                subscription_senders.remove(&channel);
                                            }
                                        }
                                    }
                                    
                                }
                                Ok(())
                            };
                            try_join3(sender_future, receiver_future, subscribe_future).await?;
                            Ok(())
                        })().await.err();
                        state_tx.send_replace(ConnectionState::NotConnected(NotConnectedState {
                            error,
                        }));
                        subscription_senders.get_mut().values_mut().for_each(|subscriptions| {
                            if subscriptions.is_successfully_subscribed {
                                subscriptions.is_successfully_subscribed = false;
                                subscriptions.senders.values().for_each(|sender| sender.send(CustomEvent::EffectivelyUnsubscribed).unwrap());
                            }
                        });
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
        if let Err(error) = self.connect_queue.try_send(()) {
            match error {
                TrySendError::Closed(_) => unreachable!(),
                TrySendError::Full(_) => {
                    // It's anyways going to connect
                }
            }
        };
    }

    pub fn subscribe(
        &self,
        channel: &str,
    ) -> impl Stream<Item = CustomEvent> {
        PusherClientConnectionSubscription::new(self, channel)
    }
}
