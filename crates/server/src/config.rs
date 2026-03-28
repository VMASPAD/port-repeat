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

    /// Port for the HTTP reverse proxy (UUID-based tunnels).
    #[serde(default = "default_http_proxy_port")]
    pub http_proxy_port: u16,

    /// Base URL returned to clients for HTTP tunnels, e.g. "http://rp.hermesbackend.xyz:9000".
    #[serde(default = "default_http_base_url")]
    pub http_base_url: String,

    /// Seconds of inactivity before a UUID endpoint is recycled.
    #[serde(default = "default_http_idle_timeout_secs")]
    pub http_idle_timeout_secs: u64,
}

fn default_control_port() -> u16 {
    7000
}

fn default_max_streams() -> u32 {
    1024
}

fn default_http_proxy_port() -> u16 {
    9000
}

fn default_http_base_url() -> String {
    "http://localhost:9000".to_string()
}

fn default_http_idle_timeout_secs() -> u64 {
    300
}
