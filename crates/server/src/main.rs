mod client_session;
mod config;
mod public_listener;
mod state;

use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use common::{
    codec::{read_msg, write_msg},
    proto::ControlMsg,
};

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
        info!("new control connection from {peer}");

        let auth_token = cfg.auth_token.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_control(stream, &auth_token).await {
                error!("client {peer} error: {e}");
            }
        });
    }
}

async fn handle_control(
    mut stream: tokio::net::TcpStream,
    auth_token: &str,
) -> anyhow::Result<()> {
    // ── authentication handshake ────────────────────────────────────────────
    let hello = read_msg(&mut stream).await?;
    let tunnels = match hello {
        ControlMsg::Hello {
            auth_token: token,
            tunnels,
        } => {
            if token != auth_token {
                warn!("bad auth token");
                let _ = write_msg(&mut stream, &ControlMsg::CloseStream { stream_id: 0 }).await;
                return Err(anyhow::anyhow!("auth failed"));
            }
            tunnels
        }
        other => {
            return Err(anyhow::anyhow!("expected Hello, got {other:?}"));
        }
    };

    let assigned_ports: Vec<u16> = tunnels.iter().map(|t| t.remote_port).collect();
    write_msg(&mut stream, &ControlMsg::HelloOk { assigned_ports }).await?;

    info!(
        "client authenticated, {} tunnel(s)",
        tunnels.len()
    );

    // ── spin up public listeners ────────────────────────────────────────────
    let (state, session_fut) = make_session(stream, tunnels.clone());

    for (idx, tunnel) in tunnels.iter().enumerate() {
        let state_clone = std::sync::Arc::clone(&state);
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
