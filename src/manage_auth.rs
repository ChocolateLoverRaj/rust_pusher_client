use std::collections::HashMap;

use futures_util::{
    FutureExt,
    future::{BoxFuture, Either, select, select_all},
};
use tokio::sync::mpsc;

use crate::{
    AuthProvider, AuthRequest, AuthResult, ChannelSubscribe, PresenceChannelAuthRequest,
    PresenceUserData, PrivateChannelAuthRequest, PusherClientError, SubscribeActionData,
    SubscribeActionType,
    message_to_send::{
        MessageToSend, SubscribeMessage, SubscribeNormalChannel, SubscribePresenceChannel,
        SubscribePrivateChannel,
    },
};

/// Manages the futures for authentication and sending subscribe and unsubscribe messages to Pusher
pub async fn manage_auth(
    mut rx: mpsc::UnboundedReceiver<SubscribeActionData>,
    tx: &mpsc::UnboundedSender<MessageToSend>,
    socket_id: &str,
    auth_provider: &Box<dyn AuthProvider>,
) -> Result<(), PusherClientError> {
    struct PendingAuthOutput {
        result: AuthResult,
        channel: String,
        presence_data: Option<PresenceUserData>,
    }
    let mut pending_auths: HashMap<String, BoxFuture<'_, PendingAuthOutput>> = Default::default();
    loop {
        let handle_message = |message: SubscribeActionData, pending_auths: &mut HashMap<_, _>| {
            let SubscribeActionData {
                channel,
                action_type,
            } = message;
            match action_type {
                SubscribeActionType::Subscribe(subscribe) => {
                    let mut start_auth = |auth_request: AuthRequest| {
                        pending_auths.insert(channel.to_owned(), {
                            let channel = channel.to_owned();
                            async move {
                                PendingAuthOutput {
                                    channel: channel.clone(),
                                    presence_data: match &auth_request {
                                        AuthRequest::Presence(auth_request) => {
                                            Some(auth_request.user_data.clone())
                                        }
                                        AuthRequest::Private(_) => None,
                                    },
                                    result: auth_provider.get_auth_signature(auth_request).await,
                                }
                            }
                            .boxed()
                        });
                    };
                    match subscribe {
                        ChannelSubscribe::Normal => {
                            tx.send(MessageToSend::Subscribe(SubscribeMessage::Normal(
                                SubscribeNormalChannel { channel },
                            )))
                            .unwrap();
                        }
                        ChannelSubscribe::Private => {
                            start_auth(AuthRequest::Private(PrivateChannelAuthRequest {
                                channel: channel.clone(),
                                socket_id: socket_id.into(),
                            }));
                        }
                        ChannelSubscribe::Presence(user_data) => {
                            start_auth(AuthRequest::Presence(PresenceChannelAuthRequest {
                                channel: channel.clone(),
                                socket_id: socket_id.into(),
                                user_data,
                            }))
                        }
                    }
                }
                SubscribeActionType::Unsubscribe => {
                    // Only have to tell pusher to unsubscribe if we already told Pusher to subscribe
                    // And if it's not pending, we already told Pusher to subscribe
                    if pending_auths.remove(&channel).is_none() {
                        tx.send(MessageToSend::Unsubscribe(channel)).unwrap();
                    };
                }
            }
        };
        if pending_auths.is_empty() {
            match rx.recv().await {
                Some(message) => handle_message(message, &mut pending_auths),
                None => break,
            }
        } else {
            match select(Box::pin(rx.recv()), select_all(pending_auths.values_mut())).await {
                Either::Left((message, _)) => match message {
                    Some(message) => handle_message(message, &mut pending_auths),
                    None => break,
                },
                Either::Right((
                    (
                        PendingAuthOutput {
                            result,
                            channel,
                            presence_data,
                        },
                        _,
                        _,
                    ),
                    _,
                )) => {
                    let auth = result.map_err(PusherClientError::AuthError)?;
                    pending_auths.remove(&channel);
                    tx.send(MessageToSend::Subscribe(match presence_data {
                        None => {
                            SubscribeMessage::Private(SubscribePrivateChannel { auth, channel })
                        }
                        Some(user_data) => SubscribeMessage::Presence(SubscribePresenceChannel {
                            channel,
                            auth,
                            channel_data: serde_json::to_string(&user_data.map_for_pusher())
                                .unwrap(),
                        }),
                    }))
                    .unwrap();
                }
            }
        }
    }
    Ok(())
}
