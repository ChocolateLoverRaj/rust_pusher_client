use std::{collections::HashMap, rc::Rc};

use futures_util::{
    SinkExt, Stream, StreamExt,
    future::{BoxFuture, Shared, join, try_join},
    lock::Mutex,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{OnceCell, mpsc::unbounded_channel};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
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
}

#[derive(Debug, Serialize, Deserialize)]
struct PusherClientEvent<T> {
    event: String,
    data: T,
}

pub async fn connect(
    input: ConnectInput,
) -> Result<(impl Future<Output = ()>, PusherClientConnection), ConnectError> {
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
            let error = Rc::new(OnceCell::<PusherClientError>::default());
            let (send_queue_sender, mut receive_queue_receiver) = unbounded_channel();
            let future = {
                let error = error.clone();
                let send_queue_sender = send_queue_sender.clone();
                async move {
                    error
                        .get_or_init(async || {
                            (async || -> Result<(), PusherClientError> {
                                let future1 = async {
                                    while let Some(message) = receive_queue_receiver.recv().await {
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
                                                Err(PusherClientError::UnexpectedTextData(text))?
                                            }
                                            Message::Ping(data) => {
                                                println!("Received ping");
                                                let _ = send_queue_sender.send(Message::Pong(data));
                                            }
                                            Message::Pong(_) => {
                                                // We never sent a ping, so we shouldn't receive a pong
                                                Err(PusherClientError::UnexpectedBinaryData)?;
                                            }
                                            Message::Close(close_frame) => Err(
                                                PusherClientError::ConnectionClosed(close_frame),
                                            )?,
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
                        .await;
                }
            };
            (
                future,
                PusherClientConnection {
                    connection_info: event_data,
                    event_handlers: Default::default(),
                    send_queue: send_queue_sender,
                    error,
                },
            )
        })
    } else {
        Err(ConnectError::UnexpectedEvent(event.event))
    }
}

enum Event {}

#[derive(Debug, Error)]
pub enum PusherClientError {
    #[error("The WebSocket read stream ended")]
    StreamEnded,
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

pub struct PusherClientConnection {
    connection_info: ConnectionInfo,
    event_handlers: Mutex<HashMap<Event, tokio::sync::mpsc::UnboundedSender<Utf8Bytes>>>,
    send_queue: tokio::sync::mpsc::UnboundedSender<Message>,
    error: Rc<OnceCell<PusherClientError>>,
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
    #[error("WebSocket error when reading data")]
    WebSocketError(tokio_tungstenite::tungstenite::Error),
}

impl PusherClientConnection {
    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
    }

    // pub async fn subscribe(
    //     &self,
    //     channel: &str,
    // ) -> Result<PusherClientConnectionSubscription, SubscribeError> {
    //     #[derive(Debug, Serialize, Deserialize)]
    //     struct PusherSubscribeEvent {
    //         channel: String,
    //     }
    //     let event = PusherClientEvent {
    //         event: "pusher:subscribe".into(),
    //         data: PusherSubscribeEvent {
    //             channel: channel.into(),
    //         },
    //     };
    //     self.write_stream
    //         .lock()
    //         .await
    //         .send(Message::Text(
    //             serde_json::to_string(&event)
    //                 .map_err(SubscribeError::SerializeError)?
    //                 .into(),
    //         ))
    //         .await
    //         .map_err(SubscribeError::WebSocketError)?;
    //     Ok(PusherClientConnectionSubscription {})
    // }
}

pub struct PusherClientConnectionSubscription {}
