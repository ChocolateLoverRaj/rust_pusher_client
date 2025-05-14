use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectInput, connect};
use tokio::join;

#[tokio::main]
pub async fn main() {
    let (connection_future, connection) = connect(ConnectInput {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
    })
    .await
    .unwrap();
    println!("Connection info: {:?}", connection.connection_info());
    let subscribe_and_print = async |channel: &str| {
        println!("Subscribing to channel: {}", channel);
        connection
            .subscribe(channel)
            .await
            .unwrap()
            .for_each(async |event| {
                println!("{}: {:?}", channel, event);
            })
            .await;
    };
    let f2 = async {
        println!("Keeping connection alive and will panic if an error happens");
        connection_future.await.unwrap();
    };
    join!(
        subscribe_and_print("my-channel"),
        subscribe_and_print("my-channel-2"),
        f2
    );
}
