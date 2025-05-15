use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectionState, Options, PusherClientConnection};
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
    let reconnect_future = async {
        connection.connect();
        WatchStream::new(connection.state().clone())
            .for_each(async |state| {
                if let ConnectionState::NotConnected(_) = state {
                    // Do not use up too much CPU from constantly failing
                    sleep(Duration::from_secs(1)).await;
                    connection.connect();
                }
            })
            .await;
    };
    join!(connection_future, event_printer, reconnect_future);
}
