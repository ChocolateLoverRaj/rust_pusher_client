use std::time::Duration;

use futures_util::StreamExt;
use pusher_client::{ConnectionState, Options, PusherClientConnection};
use tokio::{join, time::sleep};
use tokio_stream::wrappers::WatchStream;

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
