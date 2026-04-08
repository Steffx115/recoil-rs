//! Replay recording and playback for deterministic game replays.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::protocol::CommandFrame;

/// Header metadata for a replay file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReplayHeader {
    /// Replay format version.
    pub version: u32,
    /// Hash of map data for validation.
    pub map_hash: u64,
    /// Number of players in the game.
    pub num_players: u8,
    /// Opaque game settings blob.
    pub game_settings: Vec<u8>,
}

/// A complete replay: header plus per-frame command data.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Replay {
    /// Metadata about the replay.
    pub header: ReplayHeader,
    /// `frames[i]` contains all players' commands for simulation frame `i`.
    pub frames: Vec<Vec<CommandFrame>>,
}

/// Records commands frame-by-frame, then finalizes into a [`Replay`].
pub struct ReplayRecorder {
    header: ReplayHeader,
    frames: Vec<Vec<CommandFrame>>,
}

impl ReplayRecorder {
    /// Create a new recorder with the given header.
    pub fn new(header: ReplayHeader) -> Self {
        Self {
            header,
            frames: Vec::new(),
        }
    }

    /// Append one frame's worth of commands.
    pub fn record_frame(&mut self, commands: Vec<CommandFrame>) {
        self.frames.push(commands);
    }

    /// Finalize the recording into a [`Replay`].
    pub fn finish(self) -> Replay {
        Replay {
            header: self.header,
            frames: self.frames,
        }
    }
}

/// Plays back a recorded [`Replay`] frame by frame.
pub struct ReplayPlayer {
    replay: Replay,
    current: u64,
}

impl ReplayPlayer {
    /// Create a new player starting at frame 0.
    pub fn new(replay: Replay) -> Self {
        Self { replay, current: 0 }
    }

    /// The frame index that will be returned by the next call to [`advance`](Self::advance).
    pub fn current_frame(&self) -> u64 {
        self.current
    }

    /// Total number of frames in the replay.
    pub fn total_frames(&self) -> u64 {
        self.replay.frames.len() as u64
    }

    /// Get commands for the current frame and advance the pointer.
    ///
    /// Returns `None` when all frames have been consumed.
    pub fn advance(&mut self) -> Option<&Vec<CommandFrame>> {
        let idx = self.current as usize;
        if idx < self.replay.frames.len() {
            self.current += 1;
            Some(&self.replay.frames[idx])
        } else {
            None
        }
    }

    /// Jump to a specific frame for scrubbing.
    ///
    /// If `frame` exceeds the total frame count, the pointer is clamped to
    /// the end (making [`is_finished`](Self::is_finished) return `true`).
    pub fn seek(&mut self, frame: u64) {
        self.current = frame.min(self.total_frames());
    }

    /// Returns `true` when all frames have been consumed.
    pub fn is_finished(&self) -> bool {
        self.current >= self.total_frames()
    }
}

/// Errors that can occur during replay file I/O.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("replay I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("replay serialization error: {0}")]
    Bincode(#[from] bincode::Error),
}

/// Serialize a [`Replay`] to a file using bincode.
pub fn save_replay(replay: &Replay, path: &Path) -> Result<(), ReplayError> {
    let data = bincode::serialize(replay)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Deserialize a [`Replay`] from a file.
pub fn load_replay(path: &Path) -> Result<Replay, ReplayError> {
    let data = std::fs::read(path)?;
    let replay = bincode::deserialize(&data)?;
    Ok(replay)
}

#[cfg(test)]
#[path = "tests/replay_tests.rs"]
mod tests;
