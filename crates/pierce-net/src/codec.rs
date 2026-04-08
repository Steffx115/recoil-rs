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
mod tests {
    use super::*;
    use crate::protocol::{CommandFrame, PlayerCommand};

    fn roundtrip(msg: &NetMessage) {
        let encoded = encode(msg);
        let decoded = decode(&encoded).expect("decode should succeed");
        // Re-encode to verify structural equality.
        let re_encoded = encode(&decoded);
        assert_eq!(encoded, re_encoded, "roundtrip encode mismatch");
    }

    fn roundtrip_framed(msg: &NetMessage) {
        let framed = encode_framed(msg);
        let decoded = decode_framed(&framed).expect("decode_framed should succeed");
        let re_encoded = encode(&decoded);
        let original = encode(msg);
        assert_eq!(original, re_encoded, "framed roundtrip encode mismatch");
    }

    #[test]
    fn roundtrip_hello() {
        let msg = NetMessage::Hello {
            player_id: 1,
            game_id: 12345,
        };
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_ack() {
        let msg = NetMessage::Ack { frame: 42 };
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_checksum() {
        let msg = NetMessage::Checksum {
            frame: 100,
            hash: 0xDEAD_BEEF,
        };
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_disconnect() {
        let msg = NetMessage::Disconnect { player_id: 3 };
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_command_frame_msg() {
        use pierce_sim::{Command, SimVec3};
        let msg = NetMessage::CommandFrameMsg(CommandFrame {
            frame: 5,
            player_id: 0,
            commands: vec![PlayerCommand {
                target_sim_id: 99,
                command: Command::Move(SimVec3::ZERO),
            }],
        });
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_command_frame_msg_empty() {
        let msg = NetMessage::CommandFrameMsg(CommandFrame {
            frame: 0,
            player_id: 0,
            commands: vec![],
        });
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn roundtrip_frame_advance() {
        let msg = NetMessage::FrameAdvance {
            frame: 10,
            commands: vec![
                CommandFrame {
                    frame: 10,
                    player_id: 0,
                    commands: vec![],
                },
                CommandFrame {
                    frame: 10,
                    player_id: 1,
                    commands: vec![],
                },
            ],
        };
        roundtrip(&msg);
        roundtrip_framed(&msg);
    }

    #[test]
    fn framed_length_prefix_is_correct() {
        let msg = NetMessage::Ack { frame: 1 };
        let framed = encode_framed(&msg);
        let payload_len = u32::from_le_bytes([framed[0], framed[1], framed[2], framed[3]]);
        assert_eq!(payload_len as usize, framed.len() - 4);
    }

    #[test]
    fn decode_framed_too_short() {
        let result = decode_framed(&[0, 1]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_framed_length_mismatch() {
        // Header says 100 bytes, but payload is only 2 bytes.
        let data = vec![100, 0, 0, 0, 0xAA, 0xBB];
        let result = decode_framed(&data);
        assert!(result.is_err());
    }
}
