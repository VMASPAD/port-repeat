mod client_session;
mod config;
mod http_proxy;
mod http_registry;
mod public_listener;
mod state;

use std::{fs, path::PathBuf, sync::Arc};

use anyhow::Context;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;
use common::codec::{decode_msg, encode_msg};
use common::proto::{ControlMsg, Protocol, TunnelAssignment};

use client_session::make_session;
use config::ServerConfig;
use http_registry::HttpRegistry;

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

    let registry = HttpRegistry::new();

    // Spawn idle reaper for UUID endpoints
    tokio::spawn(Arc::clone(&registry).run_idle_reaper(cfg.http_idle_timeout_secs));

    // Spawn HTTP proxy server
    {
        let reg = Arc::clone(&registry);
        let port = cfg.http_proxy_port;
        tokio::spawn(async move {
            if let Err(e) = http_proxy::run_http_proxy(reg, port).await {
                error!("HTTP proxy error: {e}");
            }
        });
    }

    let listener = TcpListener::bind(("0.0.0.0", cfg.control_port)).await?;
    info!("control listener on 0.0.0.0:{}", cfg.control_port);

    loop {
        let (stream, peer) = listener.accept().await?;

        let auth_token = cfg.auth_token.clone();
        let http_base_url = cfg.http_base_url.clone();
        let reg = Arc::clone(&registry);
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws) => {
                    info!("new WS connection from {peer}");
                    if let Err(e) = handle_control(ws, &auth_token, &http_base_url, reg).await {
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
    http_base_url: &str,
    registry: Arc<HttpRegistry>,
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

    // ── classify tunnels, generate UUIDs for HTTP tunnels ──────────────────
    let mut assignments: Vec<TunnelAssignment> = Vec::new();
    let mut http_entries: Vec<(u8, String)> = Vec::new();

    for (idx, tunnel) in tunnels.iter().enumerate() {
        match tunnel.protocol {
            Protocol::Http => {
                let uuid = Uuid::new_v4().to_string();
                let url = format!("{}/t/{}", http_base_url, uuid);
                http_entries.push((idx as u8, uuid.clone()));
                assignments.push(TunnelAssignment {
                    tunnel_id: idx as u8,
                    remote_port: None,
                    http_url: Some(url),
                });
            }
            Protocol::Tcp | Protocol::Udp => {
                assignments.push(TunnelAssignment {
                    tunnel_id: idx as u8,
                    remote_port: tunnel.remote_port,
                    http_url: None,
                });
            }
        }
    }

    ws_sink
        .send(Message::Binary(encode_msg(&ControlMsg::HelloOk { assignments })?))
        .await?;

    info!("client authenticated, {} tunnel(s)", tunnels.len());

    // ── set up session ──────────────────────────────────────────────────────
    let (state, session_fut) = make_session(ws_sink, ws_stream, tunnels.clone(), http_entries);

    // Register HTTP tunnels in the global registry
    for (tunnel_id, uuid) in &state.http_tunnel_ids {
        registry.register_with_uuid(Arc::clone(&state), *tunnel_id, uuid.clone());
    }

    // Spawn public listeners for TCP/UDP tunnels only
    for (idx, tunnel) in tunnels.iter().enumerate() {
        if tunnel.protocol != Protocol::Http {
            if let Some(port) = tunnel.remote_port {
                let state_clone = Arc::clone(&state);
                let tunnel_id = idx as u8;
                tokio::spawn(async move {
                    if let Err(e) =
                        public_listener::run_public_listener(state_clone, tunnel_id, port).await
                    {
                        error!("public listener port {port}: {e}");
                    }
                });
            }
        }
    }

    // ── run reader/writer until disconnect ──────────────────────────────────
    let state_addr = Arc::as_ptr(&state) as usize;
    let result = session_fut.await;
    registry.deregister_session(state_addr);
    result
}
