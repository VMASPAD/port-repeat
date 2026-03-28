/// Handles one proxied stream: bidirectional copy between the local service
/// and the server's stream channel.
use bytes::Bytes;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tracing::debug;

use common::proto::ControlMsg;

pub async fn proxy_stream(
    stream_id: u32,
    local_port: u16,
    mut from_server: mpsc::Receiver<Bytes>,
    to_server: mpsc::Sender<ControlMsg>,
) {
    let addr = format!("127.0.0.1:{local_port}");
    let conn = match TcpStream::connect(&addr).await {
        Ok(c) => c,
        Err(e) => {
            debug!("could not connect to {addr}: {e}");
            let _ = to_server.send(ControlMsg::CloseStream { stream_id }).await;
            return;
        }
    };

    let (mut local_read, mut local_write) = conn.into_split();

    // local → server
    let to_server_up = to_server.clone();
    let upload = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match local_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let payload = Bytes::copy_from_slice(&buf[..n]);
                    if to_server_up
                        .send(ControlMsg::Data { stream_id, payload })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = to_server_up.send(ControlMsg::CloseStream { stream_id }).await;
    });

    // server → local
    let download = tokio::spawn(async move {
        while let Some(data) = from_server.recv().await {
            if local_write.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(upload, download);
}
