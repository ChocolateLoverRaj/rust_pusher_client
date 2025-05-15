use std::{ops::Deref, time::Duration};

use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectionState, CustomEvent, Options, PusherClientConnection};
use tokio::{
    join,
    time::{sleep, timeout},
};

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = PusherClientConnection::new(Options {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
        activity_timeout: Duration::from_secs(1),
        pong_timeout: Duration::from_secs(5),
    });

    let subscribe_and_print = async |channel: &str, limit: Option<usize>| {
        println!("Subscribing to channel: {}", channel);
        let mut subscription = connection.subscribe(channel);
        let mut count = 0;
        loop {
            if let Some(limit) = limit {
                if count == limit {
                    break;
                }
            }
            let event = subscription.next().await.unwrap();
            println!("{}: {:?}", channel, event);
            if let CustomEvent::Event(_) = event {
                count += 1;
            }
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
                    .for_each(async |event| {
                        println!("{}: {:?}", channel, event);
                    })
                    .await
            })
            .await;
        }
    };
    let connection_state_future = async {
        let mut state = connection.state().clone();
        loop {
            state.changed().await.unwrap();
            println!("Connection state: {:?}", state.borrow().deref());
        }
    };
    let reconnect_future = async {
        let mut state = connection.state().clone();
        loop {
            if let ConnectionState::NotConnected(state) = &*state.borrow() {
                if let Some(error) = &state.error {
                    println!("Error: {:?}", error);
                }
                // Do not use up too much CPU from constantly failing
                sleep(Duration::from_secs(1)).await;
                connection.connect();
            }
            state.changed().await.unwrap();
        }
    };
    join!(
        connection_future,
        subscribe_and_print("my-channel", None),
        subscribe_and_print("my-channel-2", Some(2)),
        on_off_future,
        connection_state_future,
        reconnect_future
    );
}
