use std::{collections::HashMap, rc::Rc};

use futures_util::{SinkExt, Stream, StreamExt, future::try_join, lock::Mutex};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{
    mpsc::{UnboundedReceiver, unbounded_channel},
    oneshot,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, Utf8Bytes, protocol::CloseFrame},
};

pub struct ConnectInput {
    pub cluster_name: String,
    pub key: String,
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
            let future = {
                // let error = error.clone();
                let send_queue_sender = send_queue_sender.clone();
                let event_handlers = event_handlers.clone();
                let event_channels = event_channels.clone();
                async move {
                    Err({
                        (async || -> Result<(), PusherClientError> {
                            let future1 = async {
                                while let Some(message) =
                                    receive_queue_receiver.recv().await
                                {
                                    write_stream
                                        .send(message)
                                        .await
                                        .map_err(PusherClientError::WebSocketError)?;
                                }
                                Result::<_, PusherClientError>::Ok(())
                            };
                            let future2 = async {
                                loop {
                                    let message = read_stream
                                        .next()
                                        .await
                                        .ok_or(PusherClientError::StreamEnded)?
                                        .map_err(PusherClientError::WebSocketError)?;
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
                                            // We never sent a ping, so we shouldn't receive a pong
                                            Err(PusherClientError::UnexpectedBinaryData)?;
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
                            try_join(future1, future2).await.map(|_| ())
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
                },
            )
        })
    } else {
        Err(ConnectError::UnexpectedEvent(event.event))
    }
}

#[derive(Debug)]
pub struct UnexpectedEventError {
    channel: String,
    event: String,
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
}

type SubscriptionSucceededChannels = Rc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>;

#[derive(Debug)]
pub struct ChannelCustomEvent {
    event: String,
    data: String,
}

type EventChannels =
    Rc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<ChannelCustomEvent>>>>;

pub struct PusherClientConnection {
    connection_info: ConnectionInfo,
    subscription_succeeded_channels: SubscriptionSucceededChannels,
    event_channels: EventChannels,
    send_queue: tokio::sync::mpsc::UnboundedSender<Message>,
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

impl PusherClientConnection {
    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
    }

    pub async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<PusherClientConnectionSubscription, SubscribeError> {
        #[derive(Debug, Serialize, Deserialize)]
        struct PusherSubscribeEvent {
            channel: String,
        }
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
            channel: channel.into(),
            event_rx,
        })
    }
}

pub struct PusherClientConnectionSubscription {
    channel: String,
    event_rx: UnboundedReceiver<ChannelCustomEvent>,
}

// #[derive(Debug, Error)]
// pub enum ReceiveEventError {
//     #[error("Error in driver future")]
//     LibraryError,
// }

impl Stream for PusherClientConnectionSubscription {
    type Item = ChannelCustomEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.as_mut().event_rx.poll_recv(cx)
    }
}
