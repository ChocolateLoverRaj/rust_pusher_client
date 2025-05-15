use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{Options, PusherClientConnection};
use tokio::{join, time::sleep};
use tokio_stream::wrappers::WatchStream;

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = PusherClientConnection::new(Options {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(5),
    });
    let event_printer = async {
        let subscription = connection.subscribe("my-channel").await;
        let status_printer =
            WatchStream::new(subscription.status().to_owned()).for_each(async |status| {
                println!("Subscription status: {:?}", status);
            });
        let event_printer = subscription.for_each(async |event| {
            println!("{:?}", event);
        });
        join!(status_printer, event_printer);
    };
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
