/// Manages the control WebSocket session after the Hello handshake.
///
/// Spawns a writer task that drains the outbound mpsc queue into the WS sink.
/// Returns a reader future that dispatches incoming Data/Close/Ping frames.
use std::sync::Arc;

use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tracing::warn;

use common::{
    codec::{decode_msg, encode_msg},
    proto::ControlMsg, 
};
 
use crate::state::ServerState;

type WsSink = SplitSink<WebSocketStream<TcpStream>, Message>;
type WsStream = SplitStream<WebSocketStream<TcpStream>>;

/// Create a `ServerState`, spawn the writer task, return (state, reader_future).
/// Caller should `.await` the reader future (or spawn it).
pub fn make_session(
    ws_sink: WsSink,
    ws_stream: WsStream,
    tunnels: Vec<common::proto::TunnelSpec>,
    http_tunnel_ids: Vec<(u8, String)>,
) -> (Arc<ServerState>, impl std::future::Future<Output = anyhow::Result<()>>) {
    let (out_tx, mut out_rx) = mpsc::channel::<ControlMsg>(256);
    let state = ServerState::new(out_tx, tunnels, http_tunnel_ids);
    let state_clone = Arc::clone(&state);

    let mut sink = ws_sink;

    // Writer task: serializes each ControlMsg and sends it as a WS Binary frame.
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            match encode_msg(&msg) {
                Ok(encoded) => {
                    if sink.send(Message::Binary(encoded)).await.is_err() {
                        break;
                    }
                }
                Err(e) => warn!("encode error: {e}"),
            }
        }
    });

    let mut stream = ws_stream;
    let fut = async move {
        loop {
            match stream.next().await {
                Some(Ok(Message::Binary(data))) => match decode_msg(&data) {
                    Ok(msg) => handle_msg(msg, &state_clone).await,
                    Err(e) => {
                        warn!("decode error: {e}");
                        break;
                    }
                },
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ControlMsg>(&text) {
                        Ok(msg) => handle_msg(msg, &state_clone).await,
                        Err(e) => {
                            warn!("decode error: {e}");
                            break;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => {
                    warn!("ws error: {e}");
                    break;
                }
                Some(Ok(_)) => {} // skip Ping/Pong/Frame
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
    let _ = state.control_tx.send(msg).await;
}
