use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{Options, PusherClientConnection};
use tokio::join;
use tokio_stream::wrappers::WatchStream;

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
    join!(connection_future, event_printer);
}
