use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::Response,
    routing::any,
};
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use common::proto::ControlMsg;
use crate::{http_registry::HttpRegistry, state::ServerState};

pub async fn run_http_proxy(registry: Arc<HttpRegistry>, bind_port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/t/*rest", any(proxy_handler))
        .with_state(registry);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", bind_port)).await?;
    tracing::info!("HTTP proxy on 0.0.0.0:{bind_port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn proxy_handler(
    Path(rest): Path<String>,
    State(registry): State<Arc<HttpRegistry>>,
    req: Request<Body>,
) -> Response<Body> {
    // Split "uuid" from "uuid/path/to/resource"
    let (uuid, fwd_path) = match rest.split_once('/') {
        Some((u, p)) => (u.to_string(), format!("/{p}")),
        None => (rest.clone(), "/".to_string()),
    };

    let (state, tunnel_id) = match registry.lookup(&uuid) {
        Some(v) => v,
        None => return not_found(),
    };
    registry.touch(&uuid);

    let stream_id = state.next_stream_id();
    let (stream_tx, mut stream_rx) = mpsc::channel::<Bytes>(128);
    state.streams.insert(stream_id, stream_tx);

    send_ctrl(&state, ControlMsg::NewConn { stream_id, tunnel_id }).await;

    // Decompose request
    let (parts, body) = req.into_parts();

    let full_path = match parts.uri.query() {
        Some(q) => format!("{fwd_path}?{q}"),
        None => fwd_path,
    };

    let body_bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to read request body: {e}");
            state.streams.remove(&stream_id);
            return error_resp(StatusCode::BAD_REQUEST);
        }
    };

    // Build raw HTTP/1.1 request
    let mut raw: Vec<u8> = Vec::new();
    raw.extend_from_slice(format!("{} {} HTTP/1.1\r\n", parts.method, full_path).as_bytes());
    for (name, value) in &parts.headers {
        if name == axum::http::header::CONNECTION {
            continue; // replaced below
        }
        raw.extend_from_slice(name.as_str().as_bytes());
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(value.as_bytes());
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(b"Connection: close\r\n\r\n");
    raw.extend_from_slice(&body_bytes);

    debug!("stream {stream_id}: forwarding {} bytes to tunnel", raw.len());
    send_ctrl(&state, ControlMsg::Data { stream_id, payload: Bytes::from(raw) }).await;

    // Collect response bytes until the client closes the stream
    let mut response_bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = stream_rx.recv().await {
        response_bytes.extend_from_slice(&chunk);
    }
    state.streams.remove(&stream_id);
    debug!("stream {stream_id}: received {} response bytes", response_bytes.len());

    parse_http_response(response_bytes)
}

fn parse_http_response(data: Vec<u8>) -> Response<Body> {
    if data.is_empty() {
        return error_resp(StatusCode::BAD_GATEWAY);
    }

    let mut header_buf = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut header_buf);

    match resp.parse(&data) {
        Ok(httparse::Status::Complete(body_offset)) => {
            let status_code = resp.code.unwrap_or(200);
            let status =
                StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            let mut builder = Response::builder().status(status);

            for header in resp.headers.iter() {
                if let Ok(v) = axum::http::HeaderValue::from_bytes(header.value) {
                    builder = builder.header(header.name, v);
                }
            }

            let body = data[body_offset..].to_vec();
            builder
                .body(Body::from(body))
                .unwrap_or_else(|_| error_resp(StatusCode::INTERNAL_SERVER_ERROR))
        }
        _ => {
            // Parse failed or incomplete — return raw bytes as-is
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(data))
                .unwrap_or_else(|_| error_resp(StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

async fn send_ctrl(state: &ServerState, msg: ControlMsg) {
    let _ = state.control_tx.send(msg).await;
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("tunnel not found\n"))
        .unwrap()
}

fn error_resp(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}
