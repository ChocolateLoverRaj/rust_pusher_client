use thiserror::Error;

#[derive(Debug)]
pub struct UnexpectedEventError {
    pub channel: String,
    pub event: String,
}

#[derive(Debug, Error)]
pub enum PusherClientError {
    #[error("The WebSocket read stream ended")]
    StreamEndedBeforeConnectionEstablished,
    #[error("WebSocket error when reading data")]
    WebSocketError(nash_ws::Error),
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
    PongError(nash_ws::Error),
    #[error("WebSocket connection closed")]
    ConnectionClosed(Option<String>),
    #[error("This error happened because of a different error elsewhere")]
    LibraryError,
    #[error("This library sent a ping, but no pong was received from Pusher")]
    PongTimeout,
}
