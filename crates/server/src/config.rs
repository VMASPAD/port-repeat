use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Port the server listens on for client control connections.
    #[serde(default = "default_control_port")]
    pub control_port: u16,

    /// Shared secret used to authenticate clients.
    pub auth_token: String,

    /// Maximum simultaneous streams per client (reserved for future enforcement).
    #[allow(dead_code)]
    #[serde(default = "default_max_streams")]
    pub max_streams: u32,
}

fn default_control_port() -> u16 {
    7000
}

fn default_max_streams() -> u32 {
    1024
}
