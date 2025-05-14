use std::time::Duration;

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectInput, connect};
use tokio::{
    join,
    time::{Instant, sleep, timeout, timeout_at},
};

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = connect(ConnectInput {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(30),
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
    let last_message_received_future = async {
        let warning_threshold = Duration::from_secs(3);
        let mut last_message_received = connection.last_message_received().clone();
        loop {
            let instant = connection.last_message_received().borrow().clone();
            // println!("Last received: {:?}", instant);
            // sleep(warning_threshold).await;
            if let Err(_) = timeout_at(
                instant.checked_add(warning_threshold).unwrap(),
                last_message_received.changed(),
            )
            .await
            {
                println!(
                    "Didn't receive a message in {:?}. Are we still connected?",
                    warning_threshold
                );
                // Wait for a message until we check for disconnect again
                last_message_received.changed().await.unwrap();
                let difference = Instant::now().duration_since(instant);
                println!(
                    "Reconnected after being \"disconnected\" for {:?}",
                    difference
                );
            }
        }
    };
    join!(
        driver_future,
        subscribe_and_print("my-channel", None),
        subscribe_and_print("my-channel-2", Some(2)),
        on_off_future,
        last_message_received_future
    );
}
