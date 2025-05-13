use std::{cell::RefCell, rc::Rc};

use futures_util::{
    SinkExt, Stream, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{Message, protocol::CloseFrame},
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

pub async fn connect(input: ConnectInput) -> Result<PusherClientConnection, ConnectError> {
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
    #[derive(Debug, Serialize, Deserialize)]
    struct PusherEvent {
        event: String,
        data: String,
    }
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
    let event =
        serde_json::from_str::<PusherEvent>(text.as_str()).map_err(ConnectError::InvalidMessage)?;
    if event.event == "pusher:connection_established" {
        let event_data = serde_json::from_str::<ConnectionInfo>(&event.data)
            .map_err(ConnectError::InvalidEventData)?;
        Ok(PusherClientConnection {
            connection_info: event_data,
            write_stream,
            read_stream,
        })
    } else {
        Err(ConnectError::UnexpectedEvent(event.event))
    }
}

pub struct PusherClientConnection {
    connection_info: ConnectionInfo,
    write_stream: SplitSink<
        WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >,
    read_stream:
        SplitStream<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
}

#[derive(Debug, Error)]
pub enum KeepAliveError {
    #[error("WebSocket error when reading data")]
    WebSocketError(tokio_tungstenite::tungstenite::Error),
    #[error("Received data from Pusher that was unexpected")]
    UnexpectedData,
    #[error("Error sending a pong after receiving a ping from Pusher")]
    PongError(tokio_tungstenite::tungstenite::Error),
    #[error("WebSocket connection closed")]
    ConnectionClosed(Option<CloseFrame>),
}

impl PusherClientConnection {
    pub fn connection_info(&self) -> &ConnectionInfo {
        &self.connection_info
    }

    /// Keeps a connection alive by waiting for Ping messages from Pusher and responding with Pong messages. Pusher says that the client must also send a ping if no message was received from Pusher since a certain amount of time. This is not currently implemented by this stream. If the stream returns `None`, that means that you should assume that the connection is closed.
    pub fn keep_alive(&mut self) -> impl Stream<Item = Result<(), KeepAliveError>> {
        let connection = Rc::new(RefCell::new(self));
        futures_util::stream::unfold(false, move |is_closed| {
            let connection = connection.clone();
            async move {
                if is_closed {
                    None
                } else {
                    let mut connection = connection.borrow_mut();
                    match connection.read_stream.next().await? {
                        Ok(message) => {
                            match message {
                                Message::Binary(_) | Message::Text(_) => {
                                    Some((Err(KeepAliveError::UnexpectedData), false))
                                }
                                Message::Ping(data) => {
                                    match connection.write_stream.send(Message::Pong(data)).await {
                                        Ok(()) => Some((Ok(()), false)),
                                        Err(e) => Some((Err(KeepAliveError::PongError(e)), false)),
                                    }
                                }
                                Message::Pong(_) => {
                                    // We never sent a ping, so we shouldn't receive a pong
                                    Some((Err(KeepAliveError::UnexpectedData), false))
                                }
                                Message::Close(close_frame) => {
                                    Some((Err(KeepAliveError::ConnectionClosed(close_frame)), true))
                                }
                                Message::Frame(_) => unreachable!(),
                            }
                        }
                        Err(e) => Some((Err(KeepAliveError::WebSocketError(e)), false)),
                    }
                }
            }
        })
    }
}
