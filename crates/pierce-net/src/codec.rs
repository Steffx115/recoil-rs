//! Binary serialization and length-prefixed framing for [`NetMessage`].

use crate::protocol::NetMessage;

/// Errors that can occur during encoding/decoding.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("bincode serialization failed: {0}")]
    Serialize(#[from] bincode::Error),
    #[error("frame too short: need at least 4 bytes for length header, got {0}")]
    TooShort(usize),
    #[error("frame length mismatch: header says {expected} bytes but payload has {actual}")]
    LengthMismatch { expected: usize, actual: usize },
}

/// Serialize a [`NetMessage`] into bytes using bincode.
pub fn encode(msg: &NetMessage) -> Vec<u8> {
    bincode::serialize(msg).expect("NetMessage serialization should not fail")
}

/// Deserialize a [`NetMessage`] from bincode bytes.
pub fn decode(data: &[u8]) -> Result<NetMessage, CodecError> {
    Ok(bincode::deserialize(data)?)
}

/// Encode a [`NetMessage`] with a 4-byte little-endian length prefix.
///
/// Wire format: `[len: u32 LE][payload: len bytes]`
pub fn encode_framed(msg: &NetMessage) -> Vec<u8> {
    let payload = encode(msg);
    let len = payload.len() as u32;
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&payload);
    buf
}

/// Decode a length-prefixed frame back into a [`NetMessage`].
///
/// Expects exactly `[len: u32 LE][payload: len bytes]`.
pub fn decode_framed(data: &[u8]) -> Result<NetMessage, CodecError> {
    if data.len() < 4 {
        return Err(CodecError::TooShort(data.len()));
    }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let payload = &data[4..];
    if payload.len() != len {
        return Err(CodecError::LengthMismatch {
            expected: len,
            actual: payload.len(),
        });
    }
    decode(payload)
}

#[cfg(test)]
#[path = "tests/codec_tests.rs"]
mod tests;
