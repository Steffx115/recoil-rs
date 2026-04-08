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
mod tests {
    use super::*;
    use crate::protocol::PlayerCommand;
    use pierce_sim::{Command, SimVec3};

    fn sample_header() -> ReplayHeader {
        ReplayHeader {
            version: 1,
            map_hash: 0xCAFE_BABE,
            num_players: 2,
            game_settings: vec![1, 2, 3],
        }
    }

    fn sample_command_frame(frame: u64, player_id: u8) -> CommandFrame {
        CommandFrame {
            frame,
            player_id,
            commands: vec![PlayerCommand {
                target_sim_id: 42,
                command: Command::Move(SimVec3::ZERO),
            }],
        }
    }

    #[test]
    fn record_ten_frames_and_verify_count() {
        let mut recorder = ReplayRecorder::new(sample_header());
        for i in 0..10 {
            recorder.record_frame(vec![sample_command_frame(i, 0)]);
        }
        let replay = recorder.finish();
        assert_eq!(replay.frames.len(), 10);
    }

    #[test]
    fn playback_iterates_all_frames_in_order() {
        let mut recorder = ReplayRecorder::new(sample_header());
        for i in 0..5 {
            recorder.record_frame(vec![sample_command_frame(i, 0)]);
        }
        let replay = recorder.finish();
        let mut player = ReplayPlayer::new(replay);

        for expected_frame in 0..5u64 {
            assert!(!player.is_finished());
            let cmds = player.advance().expect("should have frame");
            assert_eq!(cmds.len(), 1);
            assert_eq!(cmds[0].frame, expected_frame);
        }
        assert!(player.is_finished());
        assert!(player.advance().is_none());
    }

    #[test]
    fn seek_to_specific_frame() {
        let mut recorder = ReplayRecorder::new(sample_header());
        for i in 0..10 {
            recorder.record_frame(vec![sample_command_frame(i, 0)]);
        }
        let replay = recorder.finish();
        let mut player = ReplayPlayer::new(replay);

        // Seek to frame 7
        player.seek(7);
        assert_eq!(player.current_frame(), 7);
        let cmds = player.advance().expect("should have frame 7");
        assert_eq!(cmds[0].frame, 7);
        assert_eq!(player.current_frame(), 8);

        // Seek past end clamps
        player.seek(100);
        assert!(player.is_finished());

        // Seek back to 0
        player.seek(0);
        assert_eq!(player.current_frame(), 0);
        assert!(!player.is_finished());
    }

    #[test]
    fn save_load_roundtrip() {
        let mut recorder = ReplayRecorder::new(sample_header());
        for i in 0..5 {
            recorder.record_frame(vec![sample_command_frame(i, 0), sample_command_frame(i, 1)]);
        }
        let replay = recorder.finish();

        let dir = std::env::temp_dir().join("recoil_replay_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.replay");

        save_replay(&replay, &path).unwrap();
        let loaded = load_replay(&path).unwrap();

        assert_eq!(loaded.header.version, replay.header.version);
        assert_eq!(loaded.header.map_hash, replay.header.map_hash);
        assert_eq!(loaded.header.num_players, replay.header.num_players);
        assert_eq!(loaded.header.game_settings, replay.header.game_settings);
        assert_eq!(loaded.frames.len(), replay.frames.len());
        for (i, (orig, roundtripped)) in replay.frames.iter().zip(loaded.frames.iter()).enumerate()
        {
            assert_eq!(
                orig.len(),
                roundtripped.len(),
                "frame {i} command count mismatch"
            );
        }

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn empty_replay_works() {
        let recorder = ReplayRecorder::new(sample_header());
        let replay = recorder.finish();
        assert_eq!(replay.frames.len(), 0);

        let mut player = ReplayPlayer::new(replay);
        assert_eq!(player.total_frames(), 0);
        assert!(player.is_finished());
        assert!(player.advance().is_none());
    }
}
