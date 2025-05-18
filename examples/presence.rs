use std::time::Duration;

use futures_util::StreamExt;
use pusher_client::{Options, PresenceUserData, PrivateKeyAuthProvider, PusherClientConnection};
use rand::random;
use serde_json::{Number, json};
use tokio::join;

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = PusherClientConnection::new(Options {
        cluster_name: env!("PUSHER_CLUSTER_NAME").into(),
        key: env!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(5),
        auth_provider: Box::new(PrivateKeyAuthProvider {
            app_key: env!("PUSHER_KEY").into(),
            app_secret: env!("PUSHER_SECRET").into(),
        }),
    });
    connection.connect();
    let event_printer = connection
        .subscribe(
            "presence-channel",
            pusher_client::ChannelSubscribe::Presence(PresenceUserData::new(
                Number::from_u128(random::<u64>().into()).unwrap(),
                json!({
                    "language": "Rust",
                    "package_name": env!("CARGO_PKG_NAME"),
                    "package_version": env!("CARGO_PKG_VERSION"),
                    "package_description": env!("CARGO_PKG_DESCRIPTION"),
                })
                .as_object()
                .unwrap()
                .to_owned(),
            )),
        )
        .for_each(async |event| {
            println!("{:?}", event);
        });
    join!(connection_future, event_printer);
}
