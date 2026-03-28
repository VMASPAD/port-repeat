mod config;
mod tunnel;

use std::{fs, path::PathBuf, time::Duration};

use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use dashmap::DashMap;
use tokio::{
    io::BufReader,
    net::TcpStream,
    sync::mpsc,
};
use tracing::{info, warn};

use common::{
    codec::{read_msg, write_msg},
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
        info!(
            "connecting to {}:{}",
            cfg.server_host, cfg.control_port
        );
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
    let addr = format!("{}:{}", cfg.server_host, cfg.control_port);
    let stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("connecting to {addr}"))?;

    let tunnels = cfg.tunnel_specs();
    let (reader_half, writer_half) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut writer = tokio::io::BufWriter::new(writer_half);

    // ── send Hello ───────────────────────────────────────────────────────────
    write_msg(
        &mut writer,
        &ControlMsg::Hello {
            auth_token: cfg.auth_token.clone(),
            tunnels: tunnels.clone(),
        },
    )
    .await?;

    // ── wait for HelloOk ─────────────────────────────────────────────────────
    match read_msg(&mut reader).await? {
        ControlMsg::HelloOk { assigned_ports } => {
            info!("authenticated. assigned ports: {assigned_ports:?}");
        }
        other => {
            return Err(anyhow::anyhow!("expected HelloOk, got {other:?}"));
        }
    }

    // ── shared state: active streams ─────────────────────────────────────────
    // stream_id → sender of bytes destined for the local service
    let streams: std::sync::Arc<DashMap<u32, mpsc::Sender<Bytes>>> =
        std::sync::Arc::new(DashMap::<u32, mpsc::Sender<Bytes>>::new());

    // Channel for frames the proxy tasks want to send to the server.
    let (to_server_tx, mut to_server_rx) = mpsc::channel::<ControlMsg>(256);

    // ── writer task: drains to_server_rx into the TCP writer ─────────────────
    tokio::spawn(async move {
        while let Some(msg) = to_server_rx.recv().await {
            if write_msg(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });

    // ── reader loop ──────────────────────────────────────────────────────────
    loop {
        match read_msg(&mut reader).await? {
            ControlMsg::NewConn {
                stream_id,
                tunnel_id,
            } => {
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
                let streams_clone: std::sync::Arc<DashMap<u32, mpsc::Sender<Bytes>>> =
                    std::sync::Arc::clone(&streams);
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
