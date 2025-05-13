use dotenvy_macro::dotenv;
use futures_util::StreamExt;
use pusher_client::{ConnectInput, connect};

#[tokio::main]
pub async fn main() {
    let connection = connect(ConnectInput {
        cluster_name: dotenv!("PUSHER_CLUSTER_NAME").into(),
        key: dotenv!("PUSHER_KEY").into(),
    })
    .await
    .unwrap();
    println!("Connection info: {:?}", connection.connection_info());
    let s = connection.keep_alive();
    s.for_each(async |result| {
        result.unwrap();
        println!("Received a ping from Pusher");
    })
    .await;
    panic!("Connection is no longer alive!");
}
