/// WebSocket transport helpers.
///
/// Each ControlMsg is sent as a single WebSocket Binary frame (JSON payload).
/// The old length-prefixed TCP framing is replaced by WS message boundaries.
use crate::{error::Result, proto::ControlMsg};

/// Serialize a `ControlMsg` to JSON bytes for use as a WS Binary payload.
pub fn encode_msg(msg: &ControlMsg) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(msg)?)
}

/// Deserialize a `ControlMsg` from a WS Binary (or Text) payload.
pub fn decode_msg(data: &[u8]) -> Result<ControlMsg> {
    Ok(serde_json::from_slice(data)?)
}
