use nash_ws::Message;
use tokio::sync::{mpsc, watch};

use crate::{
    ConnectionState, PusherClientError, PusherClientEvent, PusherSubscribeEvent,
    message_to_send::{MessageToSend, SubscribeMessage},
};

pub async fn sender(
    mut write_stream: nash_ws::WebSocketSender,
    mut rx: mpsc::UnboundedReceiver<MessageToSend>,
    state_tx: watch::Sender<ConnectionState>,
) -> Result<(), PusherClientError> {
    while let Some(message) = rx.recv().await {
        match message {
            MessageToSend::Disconnect => {
                state_tx.send_replace(ConnectionState::Disconnecting);
                write_stream
                    .close(None)
                    .await
                    .map_err(PusherClientError::WebSocketError)?;
                break;
            }
            MessageToSend::Ping => {
                write_stream
                    .send(&Message::Text(
                        serde_json::to_string(&PusherClientEvent {
                            event: "pusher:ping".into(),
                            data: None::<()>,
                        })
                        .unwrap(),
                    ))
                    .await
                    .map_err(PusherClientError::WebSocketError)?;
            }
            MessageToSend::Subscribe(subscribe) => {
                write_stream
                    .send(&Message::Text(
                        match subscribe {
                            SubscribeMessage::Normal(subscribe) => {
                                serde_json::to_string(&PusherClientEvent {
                                    event: "pusher:subscribe".to_owned(),
                                    data: Some(subscribe),
                                })
                            }
                            SubscribeMessage::Presence(subscribe) => {
                                serde_json::to_string(&PusherClientEvent {
                                    event: "pusher:subscribe".to_owned(),
                                    data: Some(subscribe),
                                })
                            }
                            SubscribeMessage::Private(subscribe) => {
                                serde_json::to_string(&PusherClientEvent {
                                    event: "pusher:subscribe".to_owned(),
                                    data: Some(subscribe),
                                })
                            }
                        }
                        .unwrap(),
                    ))
                    .await
                    .map_err(PusherClientError::WebSocketError)?;
            }
            MessageToSend::Unsubscribe(channel) => {
                write_stream
                    .send(&Message::Text(
                        serde_json::to_string(&PusherClientEvent {
                            event: "pusher:unsubscribe".into(),
                            data: Some(PusherSubscribeEvent { channel }),
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await
                    .map_err(PusherClientError::WebSocketError)?;
            }
        }
    }
    Ok(())
}
