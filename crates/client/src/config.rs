use serde::Deserialize;

use common::proto::{Protocol, TunnelSpec};

#[derive(Debug, Deserialize)]
pub struct ClientConfig {
    pub server_host: String,
    #[serde(default = "default_control_port")]
    pub control_port: u16,
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
    pub remote_port: u16,
    pub local_port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: Protocol,
}

fn default_control_port() -> u16 {
    7000
}

fn default_protocol() -> Protocol {
    Protocol::Tcp
}
