use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Maximum allowed frame payload (16 MiB).
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

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
    HelloOk { assigned_ports: Vec<u16> },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelSpec {
    /// Human-readable label (e.g. "minecraft").
    pub name: String,
    /// Port opened on the VPS for public traffic.
    pub remote_port: u16,
    /// Port on the local machine to forward to.
    pub local_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
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
