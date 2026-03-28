/// Listens on the public port (VPS-side) and forwards each new TCP connection
/// to the client over the control channel as a new stream.
use std::sync::Arc;

use bytes::Bytes;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tracing::{debug, info, warn};

use common::proto::ControlMsg;

use crate::{client_session::send_control, state::ServerState};

pub async fn run_public_listener(
    state: Arc<ServerState>,
    tunnel_id: u8,
    remote_port: u16,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", remote_port)).await?;
    info!("public listener on 0.0.0.0:{remote_port} (tunnel_id={tunnel_id})");

    loop {
        let (conn, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("accept error on port {remote_port}: {e}");
                continue;
            }
        };

        let stream_id = state.next_stream_id();
        debug!("new public conn from {peer} → stream_id={stream_id}");

        // Register the stream sender before telling the client.
        let (stream_tx, stream_rx) = mpsc::channel::<Bytes>(128);
        state.streams.insert(stream_id, stream_tx);

        // Notify the client.
        send_control(&state, ControlMsg::NewConn { stream_id, tunnel_id }).await;

        let state_clone = Arc::clone(&state);
        tokio::spawn(handle_public_conn(conn, stream_id, stream_rx, state_clone));
    }
}

/// Bidirectional proxy between the public TCP connection and the tunnel stream.
async fn handle_public_conn(
    conn: tokio::net::TcpStream,
    stream_id: u32,
    mut stream_rx: mpsc::Receiver<Bytes>,
    state: Arc<ServerState>,
) {
    let (mut pub_read, mut pub_write) = conn.into_split();

    // pub → client
    let state_up = Arc::clone(&state);
    let upload = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match pub_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let payload = Bytes::copy_from_slice(&buf[..n]);
                    send_control(&state_up, ControlMsg::Data { stream_id, payload }).await;
                }
                Err(_) => break,
            }
        }
        send_control(&state_up, ControlMsg::CloseStream { stream_id }).await;
    });

    // client → pub
    let download = tokio::spawn(async move {
        while let Some(data) = stream_rx.recv().await {
            if pub_write.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(upload, download);
    state.streams.remove(&stream_id);
}
