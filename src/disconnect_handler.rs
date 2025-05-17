use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::WatchStream;

use crate::{ConnectOperation, message_to_send::MessageToSend};

pub async fn disconnect_handler(
    connect_queue_rx: &mut WatchStream<Option<ConnectOperation>>,
    message_tx: &mpsc::UnboundedSender<MessageToSend>,
) {
    loop {
        if let Some(ConnectOperation::Disconnect) = connect_queue_rx.next().await.unwrap() {
            break;
        }
    }
    message_tx.send(MessageToSend::Disconnect).unwrap();
}
