use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribeNormalChannel {
    pub channel: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribePrivateChannel {
    pub channel: String,
    pub auth: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribePresenceChannel {
    pub channel: String,
    pub auth: String,
    /// Note that this has to be a serialized string of `PresenceUserData`
    pub channel_data: String,
}

pub enum SubscribeMessage {
    Normal(SubscribeNormalChannel),
    Private(SubscribePrivateChannel),
    Presence(SubscribePresenceChannel),
}

pub enum MessageToSend {
    /// Not actually a Pusher message, but disconnect the WebSocket
    Disconnect,
    /// Ping using Pusher ping protocol
    Ping,
    Subscribe(SubscribeMessage),
    Unsubscribe(String),
}
