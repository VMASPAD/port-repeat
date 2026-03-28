use serde::Deserialize;

use common::proto::{Protocol, TunnelSpec};

#[derive(Debug, Deserialize)]
pub struct ClientConfig {
    /// Full WebSocket URL, e.g. "wss://rp.hermesbackend.xyz/tunnel"
    pub ws_url: String,
    pub auth_token: String,
    #[serde(default)]
    pub tunnel: Vec<TunnelEntry>,
}

impl ClientConfig {
    pub fn tunnel_specs(&self) -> Vec<TunnelSpec> {
        self.tunnel
            .iter()
            .map(|t| TunnelSpec {
                name: t.name.clone(),
                remote_port: t.remote_port,
                local_port: t.local_port,
                protocol: t.protocol,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct TunnelEntry {
    pub name: String,
    /// Required for tcp/udp tunnels. Omit for http tunnels (server assigns UUID endpoint).
    #[serde(default)]
    pub remote_port: Option<u16>,
    pub local_port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: Protocol,
}

fn default_protocol() -> Protocol {
    Protocol::Tcp
}
