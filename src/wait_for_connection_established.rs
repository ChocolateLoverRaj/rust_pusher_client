use nash_ws::Message;

use crate::{ConnectionInfo, PusherClientError, PusherServerEvent};

pub async fn wait_for_connection_established(
    read_stream: &mut nash_ws::WebSocketReceiver,
) -> Result<ConnectionInfo, PusherClientError> {
    // Wait until we get the pusher:connection_established event
    let text = loop {
        let message = read_stream
            .next()
            .await
            .ok_or(PusherClientError::StreamEndedBeforeConnectionEstablished)?
            .map_err(PusherClientError::WebSocketError)?;
        match message {
            Message::Text(text) => break text,
            Message::Binary(_) => return Err(PusherClientError::UnexpectedBinaryData),
            Message::Close(close_frame) => {
                return Err(PusherClientError::ConnectionClosed(close_frame));
            }
        }
    };
    let event = serde_json::from_str::<PusherServerEvent>(text.as_str())
        .map_err(PusherClientError::JsonParseError)?;
    if event.event != "pusher:connection_established" {
        Err(PusherClientError::ParseError)?;
    }
    let connection_info = serde_json::from_str::<ConnectionInfo>(&event.data)
        .map_err(PusherClientError::JsonParseError)?;
    Ok(connection_info)
}
