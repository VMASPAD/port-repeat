mod config;
mod tunnel;

use std::{fs, path::PathBuf, time::Duration};

use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

use common::{
    codec::{decode_msg, encode_msg},
    proto::ControlMsg,
};
use config::ClientConfig;

#[derive(Parser)]
#[command(name = "rustunnel-client", about = "RusTunnel local client")]
struct Cli {
    #[arg(short, long, default_value = "config/client.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rustunnel_client=debug".parse()?) 
                .add_directive("info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    let raw = fs::read_to_string(&cli.config)
        .with_context(|| format!("reading {}", cli.config.display()))?;
    let cfg: ClientConfig = toml::from_str(&raw)?;

    // Exponential backoff: 1s → 2s → 4s … cap at 60s.
    let mut delay = Duration::from_secs(1);

    loop {
        info!("connecting to {}", cfg.ws_url);
        match connect_and_run(&cfg).await {
            Ok(()) => {
                info!("session ended cleanly");
                break;
            }
            Err(e) => {
                warn!("disconnected: {e}. retrying in {delay:?}...");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(60));
            }
        }
    }

    Ok(())
}

async fn connect_and_run(cfg: &ClientConfig) -> anyhow::Result<()> {
    let (ws, _) = connect_async(&cfg.ws_url)
        .await
        .with_context(|| format!("connecting to {}", cfg.ws_url))?;

    let tunnels = cfg.tunnel_specs();
    let (mut ws_sink, mut ws_stream) = ws.split();

    // ── send Hello ───────────────────────────────────────────────────────────
    ws_sink
        .send(Message::Binary(encode_msg(&ControlMsg::Hello {
            auth_token: cfg.auth_token.clone(),
            tunnels: tunnels.clone(),
        })?))
        .await?;

    // ── wait for HelloOk ─────────────────────────────────────────────────────
    let assignments = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Binary(data))) => match decode_msg(&data)? {
                ControlMsg::HelloOk { assignments } => break assignments,
                other => return Err(anyhow::anyhow!("expected HelloOk, got {other:?}")),
            },
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<ControlMsg>(&text)? {
                ControlMsg::HelloOk { assignments } => break assignments,
                other => return Err(anyhow::anyhow!("expected HelloOk, got {other:?}")),
            },
            Some(Ok(Message::Close(_))) | None => {
                return Err(anyhow::anyhow!("connection closed before HelloOk"));
            }
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(_)) => continue,
        }
    };

    // Log tunnel assignments
    for a in &assignments {
        let name = tunnels
            .get(a.tunnel_id as usize)
            .map(|t| t.name.as_str())
            .unwrap_or("?");
        if let Some(url) = &a.http_url {
            info!("");
            info!("  ┌─ HTTP TUNNEL READY ─────────────────────────────┐");
            info!("  │  name : {name}");
            info!("  │  url  : {url}");
            info!("  └────────────────────────────────────────────────────┘");
            info!("");
        } else if let Some(port) = a.remote_port {
            info!("tunnel '{name}' → port {port}");
        }
    }

    // ── shared state: active streams ─────────────────────────────────────────
    let streams: std::sync::Arc<DashMap<u32, mpsc::Sender<Bytes>>> =
        std::sync::Arc::new(DashMap::new());

    let (to_server_tx, mut to_server_rx) = mpsc::channel::<ControlMsg>(256);

    // ── writer task: drains to_server_rx into the WS sink ────────────────────
    tokio::spawn(async move {
        while let Some(msg) = to_server_rx.recv().await {
            match encode_msg(&msg) {
                Ok(encoded) => {
                    if ws_sink.send(Message::Binary(encoded)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // ── reader loop ──────────────────────────────────────────────────────────
    loop {
        let msg = loop {
            match ws_stream.next().await {
                Some(Ok(Message::Binary(data))) => break decode_msg(&data)?,
                Some(Ok(Message::Text(text))) => {
                    break serde_json::from_str::<ControlMsg>(&text)?
                }
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(_)) => continue,
            }
        };

        match msg {
            ControlMsg::NewConn { stream_id, tunnel_id } => {
                let local_port = match tunnels.get(tunnel_id as usize) {
                    Some(t) => t.local_port,
                    None => {
                        warn!("unknown tunnel_id {tunnel_id}");
                        continue;
                    }
                };

                let (from_server_tx, from_server_rx) = mpsc::channel::<Bytes>(128);
                streams.insert(stream_id, from_server_tx);

                let to_srv = to_server_tx.clone();
                let streams_clone = std::sync::Arc::clone(&streams);
                tokio::spawn(async move {
                    tunnel::proxy_stream(stream_id, local_port, from_server_rx, to_srv).await;
                    streams_clone.remove(&stream_id);
                });
            }

            ControlMsg::Data { stream_id, payload } => {
                if let Some(tx) = streams.get(&stream_id) {
                    let _ = tx.send(payload).await;
                }
            }

            ControlMsg::CloseStream { stream_id } => {
                streams.remove(&stream_id);
            }

            ControlMsg::Ping => {
                let _ = to_server_tx.send(ControlMsg::Pong).await;
            }

            ControlMsg::HelloOk { .. } | ControlMsg::Pong => {}

            other => {
                warn!("unexpected msg: {other:?}");
            }
        }
    }
}
