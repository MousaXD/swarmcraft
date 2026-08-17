use std::time::Duration;

use swarm_ipc::{FabricBridgeListener, IpcTransportError};
use tokio::{io::AsyncWriteExt, net::TcpStream};

#[tokio::test]
async fn launch_config_debug_never_exposes_authentication_token() {
    let listener = FabricBridgeListener::bind().await.unwrap();
    let config = listener.launch_config().unwrap();
    let token = config.token().to_owned();
    let debug = format!("{config:?}");

    assert!(!debug.contains(&token));
    assert!(debug.contains("[REDACTED]"));
}

#[tokio::test]
async fn oversized_unauthenticated_control_line_is_rejected() {
    let listener = FabricBridgeListener::bind().await.unwrap();
    let config = listener.launch_config().unwrap();
    let client = tokio::spawn(async move {
        let mut stream = TcpStream::connect((config.host.as_str(), config.port)).await.unwrap();
        stream.write_all(&vec![b'A'; 17 * 1024]).await.unwrap();
        stream.flush().await.unwrap();
    });

    let error = listener.accept(Duration::from_secs(5)).await.unwrap_err();
    assert!(matches!(error, IpcTransportError::LineTooLong));
    client.await.unwrap();
}
