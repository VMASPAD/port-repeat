/// Handles one connected client after the Hello handshake.
///
/// Spawns:
///   - a writer task that drains the outbound mpsc queue into the TCP writer
///   - a reader loop that dispatches incoming Data/Close/Ping frames
use std::sync::Arc;

use bytes::Bytes;
use tokio::{
    io::BufReader,
    net::TcpStream,
    sync::mpsc,
};
use tracing::warn;

use common::{
    codec::{read_msg, write_msg},
    error::ProtoError,
    proto::ControlMsg,
};

use crate::state::ServerState;

/// Create a `ServerState` + spawn writer + return reader future.
///
/// Returns `(state, read_fut)`. Caller should `.await` the read_fut (or spawn it).
pub fn make_session(
    stream: TcpStream,
    tunnels: Vec<common::proto::TunnelSpec>,
) -> (Arc<ServerState>, impl std::future::Future<Output = anyhow::Result<()>>) {
    let (out_tx, mut out_rx) = mpsc::channel::<Bytes>(256);
    let state = ServerState::new(out_tx, tunnels);
    let state_clone = Arc::clone(&state);

    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = tokio::io::BufWriter::new(write_half);

    // Spawn writer task.
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        while let Some(frame) = out_rx.recv().await {
            if writer.write_all(&frame).await.is_err() {
                break;
            }
            let _ = writer.flush().await;
        }
    });

    let fut = async move {
        loop {
            match read_msg(&mut reader).await {
                Ok(msg) => handle_msg(msg, &state_clone).await,
                Err(ProtoError::ConnectionClosed) => break,
                Err(e) => {
                    warn!("proto error: {e}");
                    break;
                }
            }
        }
        state_clone.streams.clear();
        Ok(())
    };

    (state, fut)
}

async fn handle_msg(msg: ControlMsg, state: &ServerState) {
    match msg {
        ControlMsg::Data { stream_id, payload } => {
            if let Some(tx) = state.streams.get(&stream_id) {
                let _ = tx.send(payload).await;
            }
        }
        ControlMsg::CloseStream { stream_id } => {
            state.streams.remove(&stream_id);
        }
        ControlMsg::Ping => {
            send_control(state, ControlMsg::Pong).await;
        }
        ControlMsg::DataAck { .. } => {}
        other => {
            warn!("unexpected msg: {other:?}");
        }
    }
}

pub async fn send_control(state: &ServerState, msg: ControlMsg) {
    let mut buf = Vec::new();
    if write_msg(&mut buf, &msg).await.is_ok() {
        let _ = state.control_tx.send(Bytes::from(buf)).await;
    }
}
