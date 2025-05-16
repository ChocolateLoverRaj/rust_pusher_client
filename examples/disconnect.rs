use std::time::Duration;

use futures_util::StreamExt;
use pusher_client::{Options, PusherClientConnection};
use tokio::{join, time::sleep};

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = PusherClientConnection::new(Options {
        cluster_name: env!("PUSHER_CLUSTER_NAME").into(),
        key: env!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(5),
    });
    let event_printer = connection.subscribe("my-channel").for_each(async |event| {
        println!("{:?}", event);
    });
    let connect_future = async {
        // This demonstrates how you can disconnect the connection without having to explicitly re-subscribe to all the channels that you subscribed to
        loop {
            connection.connect();
            sleep(Duration::from_secs(5)).await;
            connection.disconnect();
            sleep(Duration::from_secs(5)).await;
        }
    };
    join!(connection_future, event_printer, connect_future);
}
