//! Core network protocol types for the networking layer.

use serde::{Deserialize, Serialize};

/// A single player's commands for one sim frame.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CommandFrame {
    pub frame: u64,
    pub player_id: u8,
    pub commands: Vec<PlayerCommand>,
}

/// A command issued by a player (entity target + command).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerCommand {
    /// `SimId` value of the entity to command.
    pub target_sim_id: u64,
    pub command: pierce_sim::Command,
}

/// Network messages exchanged between server and clients.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetMessage {
    /// Client sends commands (fire-and-forget, no frame number needed).
    Commands {
        player_id: u8,
        commands: Vec<PlayerCommand>,
    },
    /// Client sends its commands for a frame (lockstep mode).
    CommandFrameMsg(CommandFrame),
    /// Server broadcasts all players' commands for a frame (authoritative).
    FrameAdvance {
        frame: u64,
        commands: Vec<CommandFrame>,
    },
    /// Checksum exchange for desync detection.
    Checksum { frame: u64, hash: u64 },
    /// Connection handshake.
    Hello { player_id: u8, game_id: u64 },
    /// Acknowledgement.
    Ack { frame: u64 },
    /// Player disconnected.
    Disconnect { player_id: u8 },
}
