use std::{collections::HashMap, rc::Rc, time::Duration};

use futures_util::{SinkExt, Stream, StreamExt, future::try_join, lock::Mutex};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{
        mpsc::{UnboundedReceiver, unbounded_channel},
        oneshot, watch,
    },
    time::{Instant, timeout, timeout_at},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, Utf8Bytes, protocol::CloseFrame},
};

pub struct ConnectInput {
    pub cluster_name: String,
    pub key: String,
    /// Duration since inactivity to send a ping to check that the connection still works
    pub activity_timeout: Duration,
    /// Duration after sending a ping after which it is assumed that the connection is disconnected
    pub pong_timeout: Duration,
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("Error connecting the WebSocket")]
    WebSocketConnectError(tokio_tungstenite::tungstenite::Error),
    #[error("WebSocket stopped streaming incoming data")]
    WebSocketStopped,
    #[error("WebSocket error when reading data")]
    WebSocketError(tokio_tungstenite::tungstenite::Error),
    #[error("Received data from Pusher that was unexpected")]
    UnexpectedData,
    #[error("Error sending a pong after receiving a ping from Pusher")]
    PongError(tokio_tungstenite::tungstenite::Error),
    #[error("WebSocket connection closed")]
    ConnectionClosed(Option<CloseFrame>),
    #[error("Received an invalid message from Pusher")]
    InvalidMessage(serde_json::Error),
    #[error("Received an unexpected event from Pusher")]
    UnexpectedEvent(String),
    #[error("Received the expected event but with invalid data from Pusher")]
    InvalidEventData(serde_json::Error),
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

pub async fn connect(
    input: ConnectInput,
) -> Result<
    (
        impl Future<Output = Result<(), PusherClientError>>,
        PusherClientConnection,
    ),
    ConnectError,
> {
    let (web_socket, _response) = connect_async(format!(
        "wss://ws-{}.pusher.com/app/{}?protocol=7&client=Rust-{}?version={}",
        input.cluster_name,
        input.key,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    ))
    .await
    .map_err(ConnectError::WebSocketConnectError)?;
    // Wait until we get the pusher:connection_established event
    let (mut write_stream, mut read_stream) = web_socket.split();
    let text = loop {
        let message = read_stream
            .next()
            .await
            .ok_or(ConnectError::WebSocketStopped)?
            .map_err(ConnectError::WebSocketError)?;
        match message {
            Message::Text(text) => break text,
            Message::Binary(_) => return Err(ConnectError::UnexpectedData),
            Message::Ping(data) => {
                write_stream
                    .send(Message::Pong(data))
                    .await
                    .map_err(ConnectError::PongError)?;
            }
            Message::Pong(_) => {
                // We never sent a ping, so we shouldn't receive a pong
                return Err(ConnectError::UnexpectedData);
            }
            Message::Close(close_frame) => return Err(ConnectError::ConnectionClosed(close_frame)),
            Message::Frame(_) => unreachable!(),
        }
    };
    let event = serde_json::from_str::<PusherServerEvent>(text.as_str())
        .map_err(ConnectError::InvalidMessage)?;
    if event.event == "pusher:connection_established" {
        let event_data = serde_json::from_str::<ConnectionInfo>(&event.data)
            .map_err(ConnectError::InvalidEventData)?;
        Ok({
            // let error = Rc::new(OnceCell::<Rc<PusherClientError>>::default());
            let (send_queue_sender, mut receive_queue_receiver) = unbounded_channel();
            let event_handlers = SubscriptionSucceededChannels::default();
            let event_channels = EventChannels::default();
            let (last_message_received_sender, last_message_received_receiver) = watch::channel({
                // We just received pusher:connection_established so now makes sense
                Instant::now()
            });
            let future = {
                // let error = error.clone();
                let send_queue_sender = send_queue_sender.clone();
                let event_handlers = event_handlers.clone();
                let event_channels = event_channels.clone();
                async move {
                    Err({
                        (async || -> Result<(), PusherClientError> {
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
                                loop {
                                    let message = {
                                        let last_seen =
                                            last_message_received_sender.borrow().clone();
                                        match timeout_at(
                                            last_seen.checked_add(input.activity_timeout).unwrap(),
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
                                                println!("Sending a ping");
                                                send_queue_sender
                                                    .send(Message::Ping(Default::default()))
                                                    .map_err(|_| PusherClientError::LibraryError)?;
                                                let message =
                                                    timeout(input.pong_timeout, read_stream.next())
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
                                    last_message_received_sender
                                        .send(Instant::now())
                                        .map_err(|_| PusherClientError::LibraryError)?;
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
                                                match event_handlers.lock().await.entry(channel.clone()) {
                                                    std::collections::hash_map::Entry::Occupied(entry) => {
                                                        let _ = entry.remove().send(());
                                                    },
                                                    std::collections::hash_map::Entry::Vacant(_) => {
                                                        Err(PusherClientError::UnexpectedSubscribeSucceededEvent(channel))?;
                                                    }
                                                }
                                            } else {
                                                let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                                                let _ = event_channels.lock().await.get(&channel).ok_or(PusherClientError::UnexpectedEvent(UnexpectedEventError { channel, event: event.event.clone() }))?.send(ChannelCustomEvent { event: event.event, data: event.data });
                                            }
                                        }
                                        Message::Ping(data) => {
                                            println!("Received ping");
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
                            try_join(sender_future, receiver_future).await.map(|_| ())
                        })()
                        .await
                        .unwrap_err()
                    })
                }
            };
            (
                future,
                PusherClientConnection {
                    connection_info: event_data,
                    subscription_succeeded_channels: event_handlers,
                    event_channels,
                    send_queue: send_queue_sender,
                    last_message_received: last_message_received_receiver,
                },
            )
        })
    } else {
        Err(ConnectError::UnexpectedEvent(event.event))
    }
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

type SubscriptionSucceededChannels = Rc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>;

#[derive(Debug)]
pub struct ChannelCustomEvent {
    pub event: String,
    pub data: String,
}

type EventChannels =
    Rc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<ChannelCustomEvent>>>>;

pub struct PusherClientConnection {
    connection_info: ConnectionInfo,
    subscription_succeeded_channels: SubscriptionSucceededChannels,
    event_channels: EventChannels,
    send_queue: tokio::sync::mpsc::UnboundedSender<Message>,
    last_message_received: tokio::sync::watch::Receiver<Instant>,
}

#[derive(Debug, Error)]
pub enum KeepAliveError {
    #[error("WebSocket error when reading data")]
    WebSocketError(tokio_tungstenite::tungstenite::Error),
    #[error("Received binary data from Pusher that was unexpected")]
    UnexpectedBinaryData,
    #[error("Received text data from Pusher that was unexpected")]
    UnexpectedTextData(Utf8Bytes),
    #[error("Error sending a pong after receiving a ping from Pusher")]
    PongError(tokio_tungstenite::tungstenite::Error),
    #[error("WebSocket connection closed")]
    ConnectionClosed(Option<CloseFrame>),
}

#[derive(Debug, Error)]
pub enum SubscribeError {
    #[error("Error serializing JSON")]
    SerializeError(serde_json::Error),
    #[error("This is when the driver future has an error")]
    LibraryError,
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherSubscribeEvent {
    channel: String,
}

impl PusherClientConnection {
    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
    }

    pub async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<PusherClientConnectionSubscription, SubscribeError> {
        let event = PusherClientEvent {
            event: "pusher:subscribe".into(),
            data: PusherSubscribeEvent {
                channel: channel.into(),
            },
        };
        let (subscription_succeeded_tx, subscription_succeeded_rx) = oneshot::channel();
        self.subscription_succeeded_channels
            .lock()
            .await
            .insert(channel.into(), subscription_succeeded_tx);
        let (event_tx, event_rx) = unbounded_channel();
        self.event_channels
            .lock()
            .await
            .insert(channel.into(), event_tx);
        self.send_queue
            .send(Message::Text(
                serde_json::to_string(&event)
                    .map_err(SubscribeError::SerializeError)?
                    .into(),
            ))
            .map_err(|_| SubscribeError::LibraryError)?;
        subscription_succeeded_rx
            .await
            .map_err(|_| SubscribeError::LibraryError)?;
        Ok(PusherClientConnectionSubscription {
            connection: self,
            channel: channel.into(),
            event_rx,
        })
    }

    pub fn last_message_received(&self) -> &tokio::sync::watch::Receiver<Instant> {
        &self.last_message_received
    }
}

pub struct PusherClientConnectionSubscription<'a> {
    connection: &'a PusherClientConnection,
    channel: String,
    event_rx: UnboundedReceiver<ChannelCustomEvent>,
}

impl Stream for PusherClientConnectionSubscription<'_> {
    type Item = ChannelCustomEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().event_rx.poll_recv(cx)
    }
}

impl Drop for PusherClientConnectionSubscription<'_> {
    fn drop(&mut self) {
        // TODO: Force client of library to handle error instead of panicking. It's tricky when `Drop` though.
        let event = PusherClientEvent {
            event: "pusher:unsubscribe".into(),
            data: PusherSubscribeEvent {
                channel: self.channel.clone(),
            },
        };
        self.connection
            .send_queue
            .send(Message::Text(
                serde_json::to_string(&event)
                    .map_err(SubscribeError::SerializeError)
                    .unwrap()
                    .into(),
            ))
            .map_err(|_| SubscribeError::LibraryError)
            .unwrap();
    }
}
