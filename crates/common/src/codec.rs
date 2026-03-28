/// Length-prefixed framing over an async TCP stream.
///
/// Wire format: `[u32 LE payload_len][payload_bytes]`
///
/// All public functions are async and cancel-safe at the frame boundary.
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    error::{ProtoError, Result},
    proto::{ControlMsg, MAX_FRAME_SIZE},
};

/// Read one `ControlMsg` from `reader`.
pub async fn read_msg<R: AsyncRead + Unpin>(reader: &mut R) -> Result<ControlMsg> {
    let len = reader.read_u32_le().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::ConnectionClosed
        } else {
            ProtoError::Io(e)
        }
    })?;

    if len > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge(len));
    }

    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::ConnectionClosed
        } else {
            ProtoError::Io(e)
        }
    })?;

    Ok(serde_json::from_slice(&buf)?)
}

/// Write one `ControlMsg` to `writer`.
pub async fn write_msg<W: AsyncWrite + Unpin>(writer: &mut W, msg: &ControlMsg) -> Result<()> {
    let payload = serde_json::to_vec(msg)?;
    let len = payload.len() as u32;

    writer.write_u32_le(len).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}
