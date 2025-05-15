use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{Options, PusherClientConnection};
use tokio::join;

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = PusherClientConnection::new(Options {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(5),
    });
    connection.connect();
    let event_printer = async {
        connection
            .subscribe("my-channel")
            .for_each(async |event| {
                println!("{:?}", event);
            })
            .await;
    };
    join!(connection_future, event_printer);
}
