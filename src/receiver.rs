use std::time::Duration;

use fluvio_wasm_timer::Delay;
use futures_util::future::{Either, select};
use nash_ws::Message;
use tokio::sync::mpsc;

use crate::{
    CustomEventData, PusherClientConnection, PusherClientError, PusherServerEvent,
    SubscriptionEvent, Subscriptions, message_to_send::MessageToSend,
};

pub async fn receiver(
    connection: &PusherClientConnection,
    pusher_requested_activity_timeout: Duration,
    mut read_stream: nash_ws::WebSocketReceiver,
    subscriptions: &Subscriptions,
    message_sender: &mpsc::UnboundedSender<MessageToSend>,
) -> Result<(), PusherClientError> {
    let mut last_message_received = fluvio_wasm_timer::Instant::now();
    loop {
        let message = {
            match select(
                Delay::new_at(
                    last_message_received
                        + connection
                            .options
                            .activity_timeout
                            .min(pusher_requested_activity_timeout),
                ),
                Box::pin(read_stream.next()),
            )
            .await
            {
                Either::Right((message, _)) => {
                    if let Some(message) = message {
                        message.map_err(PusherClientError::WebSocketError)?
                    } else {
                        break;
                    }
                }
                Either::Left((_, read_stream_next)) => {
                    // println!("Sending a ping");
                    message_sender
                        .send(MessageToSend::Ping)
                        .map_err(|_| PusherClientError::LibraryError)?;
                    match select(
                        Delay::new(connection.options.pong_timeout),
                        read_stream_next,
                    )
                    .await
                    {
                        Either::Left(_) => Err(PusherClientError::PongTimeout),
                        Either::Right((message, _)) => {
                            if let Some(message) = message {
                                message.map_err(PusherClientError::WebSocketError)
                            } else {
                                break;
                            }
                        }
                    }?
                }
            }
        };
        last_message_received = fluvio_wasm_timer::Instant::now();
        match message {
            Message::Binary(_) => {
                Err(PusherClientError::UnexpectedBinaryData)?;
            }
            Message::Text(text) => {
                // println!("Received text");
                let event = serde_json::from_str::<PusherServerEvent>(text.as_str())
                    .map_err(PusherClientError::JsonParseError)?;
                match event.event.as_str() {
                    "pusher_internal:subscription_succeeded" => {
                        let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                        // println!("Successfully subscribed to {:?}", channel);
                        if let Some(subscription) = subscriptions.lock().await.get(&channel) {
                            // println!("Senders: {}", subscription.len());
                            for sender in &subscription.senders {
                                sender
                                    .send(SubscriptionEvent::SuccessfullySubscribed)
                                    .unwrap();
                            }
                        }
                    }
                    "pusher:pong" => {
                        // println!("Received pong");
                        // We just use pong to know that the connection is still alive
                        // We already updated the last message received
                        // So no need to do anything here
                    }
                    event_name => {
                        let channel = event.channel.ok_or(PusherClientError::ParseError)?;
                        // println!("Event on channel: {:?}", channel);
                        if let Some(subscription) = subscriptions.lock().await.get_mut(&channel) {
                            subscription.senders.iter().for_each(|sender| {
                                sender
                                    .send(SubscriptionEvent::Event(CustomEventData {
                                        event: event_name.to_owned(),
                                        data: event.data.to_owned(),
                                    }))
                                    .unwrap()
                            });
                        }
                    }
                }
            }
            Message::Close(close_frame) => Err(PusherClientError::ConnectionClosed(close_frame))?,
        }
    }
    #[allow(unreachable_code)]
    Ok::<_, PusherClientError>(())
}
