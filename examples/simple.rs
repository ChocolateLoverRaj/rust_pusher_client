use dotenvy_macro::dotenv;
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
}
