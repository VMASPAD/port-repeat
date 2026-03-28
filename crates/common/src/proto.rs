use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMsg {
    // ── client → server ──────────────────────────────────────────────────
    /// First message the client sends after connecting.
    Hello {
        auth_token: String,
        tunnels: Vec<TunnelSpec>,
    },
    /// Flow-control acknowledgement: client has consumed `bytes` on `stream_id`.
    DataAck { stream_id: u32, bytes: u64 },

    // ── server → client ──────────────────────────────────────────────────
    /// Sent after a successful Hello.
    HelloOk { assignments: Vec<TunnelAssignment> },
    /// Server tells the client a new public connection arrived on `tunnel_id`.
    NewConn { stream_id: u32, tunnel_id: u8 },

    // ── bidirectional ────────────────────────────────────────────────────
    /// Carries raw TCP payload for a specific stream.
    Data {
        stream_id: u32,
        #[serde(with = "bytes_base64")]
        payload: Bytes,
    },
    /// One side signals EOF / error for a stream.
    CloseStream { stream_id: u32 },
    Ping,
    Pong,
}

/// Per-tunnel assignment returned in HelloOk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelAssignment {
    pub tunnel_id: u8,
    /// Set for TCP/UDP tunnels: the public port on the VPS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
    /// Set for HTTP tunnels: the full URL endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelSpec {
    /// Human-readable label (e.g. "minecraft").
    pub name: String,
    /// Port opened on the VPS for public traffic. None for Http tunnels.
    #[serde(default)]
    pub remote_port: Option<u16>,
    /// Port on the local machine to forward to.
    pub local_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
    Http,
}

// ── serde helper: encode Bytes as base64 ────────────────────────────────────
mod bytes_base64 {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD
            .decode(s)
            .map(Bytes::from)
            .map_err(serde::de::Error::custom)
    }
}
