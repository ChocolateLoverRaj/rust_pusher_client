use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectInput, connect};
use tokio::{
    join,
    time::{sleep, timeout},
};

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = connect(ConnectInput {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
    })
    .await
    .unwrap();
    println!("Connection info: {:?}", connection.connection_info());
    // We MUST poll this future in parallel to any future created by the connection for those futures to work
    let driver_future = async {
        println!("Keeping connection alive and will panic if an error happens");
        connection_future.await.unwrap();
    };
    let subscribe_and_print = async |channel: &str, limit: Option<usize>| {
        println!("Subscribing to channel: {}", channel);
        let mut subscription = connection.subscribe(channel).await.unwrap();
        let mut count = 0;
        loop {
            if let Some(limit) = limit {
                if count < limit {
                    count += 1;
                } else {
                    break;
                }
            }
            let event = subscription.next().await.unwrap();
            println!("{}: {:?}", channel, event);
        }
    };
    // Demonstrates subscribing and unsubscribing
    let on_off_future = async {
        let duration = Duration::from_secs(5);
        let channel = "my-channel-3";
        loop {
            println!(
                "Waiting {:?} until receiving events on {}",
                duration, channel
            );
            sleep(duration).await;
            println!("Receiving events on {} for {:?}", channel, duration);
            let _ = timeout(duration, async {
                connection
                    .subscribe(channel)
                    .await
                    .unwrap()
                    .for_each(async |event| {
                        println!("{}: {:?}", channel, event);
                    })
                    .await
            })
            .await;
        }
    };
    join!(
        driver_future,
        subscribe_and_print("my-channel", None),
        subscribe_and_print("my-channel-2", Some(2)),
        on_off_future
    );
}
