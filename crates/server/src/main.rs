mod client_session;
mod config;
mod public_listener;
mod state;

use std::{fs, path::PathBuf, sync::Arc};

use anyhow::Context;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

use common::codec::{decode_msg, encode_msg};
use common::proto::ControlMsg;

use client_session::make_session;
use config::ServerConfig;

#[derive(Parser)]
#[command(name = "rustunnel-server", about = "RusTunnel VPS server")]
struct Cli {
    #[arg(short, long, default_value = "config/server.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rustunnel_server=debug".parse()?)
                .add_directive("info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    let raw = fs::read_to_string(&cli.config)
        .with_context(|| format!("reading {}", cli.config.display()))?;
    let cfg: ServerConfig = toml::from_str(&raw)?;

    let listener = TcpListener::bind(("0.0.0.0", cfg.control_port)).await?;
    info!("control listener on 0.0.0.0:{}", cfg.control_port);

    loop {
        let (stream, peer) = listener.accept().await?;

        let auth_token = cfg.auth_token.clone();
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws) => {
                    info!("new WS connection from {peer}");
                    if let Err(e) = handle_control(ws, &auth_token).await {
                        error!("client {peer} error: {e}");
                    }
                }
                Err(e) => {
                    warn!("WS handshake failed from {peer}: {e}");
                }
            }
        });
    }
}

async fn handle_control(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    auth_token: &str,
) -> anyhow::Result<()> {
    let (mut ws_sink, mut ws_stream) = ws.split();

    // ── authentication handshake ────────────────────────────────────────────
    let tunnels = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Binary(data))) => {
                match decode_msg(&data)? {
                    ControlMsg::Hello { auth_token: token, tunnels } => {
                        if token != auth_token {
                            warn!("bad auth token");
                            return Err(anyhow::anyhow!("auth failed"));
                        }
                        break tunnels;
                    }
                    other => return Err(anyhow::anyhow!("expected Hello, got {other:?}")),
                }
            }
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<ControlMsg>(&text)? {
                    ControlMsg::Hello { auth_token: token, tunnels } => {
                        if token != auth_token {
                            warn!("bad auth token");
                            return Err(anyhow::anyhow!("auth failed"));
                        }
                        break tunnels;
                    }
                    other => return Err(anyhow::anyhow!("expected Hello, got {other:?}")),
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(anyhow::anyhow!("connection closed before Hello"));
            }
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(_)) => continue, // skip Ping/Pong
        }
    };

    let assigned_ports: Vec<u16> = tunnels.iter().map(|t| t.remote_port).collect();
    ws_sink
        .send(Message::Binary(encode_msg(&ControlMsg::HelloOk {
            assigned_ports,
        })?))
        .await?;

    info!("client authenticated, {} tunnel(s)", tunnels.len());

    // ── spin up public listeners ────────────────────────────────────────────
    let (state, session_fut) = make_session(ws_sink, ws_stream, tunnels.clone());

    for (idx, tunnel) in tunnels.iter().enumerate() {
        let state_clone = Arc::clone(&state);
        let remote_port = tunnel.remote_port;
        let tunnel_id = idx as u8;
        tokio::spawn(async move {
            if let Err(e) =
                public_listener::run_public_listener(state_clone, tunnel_id, remote_port).await
            {
                error!("public listener port {remote_port}: {e}");
            }
        });
    }

    // ── run reader/writer until disconnect ──────────────────────────────────
    session_fut.await
}
