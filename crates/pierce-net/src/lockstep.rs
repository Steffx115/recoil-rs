//! Lockstep state machine for deterministic multiplayer.
//!
//! Tracks per-player command submissions and gates frame advancement
//! until all players have submitted their commands.

use std::collections::BTreeMap;

use crate::protocol::CommandFrame;

/// Manages the lockstep synchronisation state.
///
/// The simulation cannot advance to the next frame until every player
/// has submitted their [`CommandFrame`] for the current frame.
pub struct LockstepState {
    /// The frame the simulation is currently on (waiting to execute).
    pub current_frame: u64,
    /// How many frames ahead the local player submits commands.
    /// Default is 2, meaning when current_frame is N the local player
    /// submits commands for frame N + input_delay.
    pub input_delay: u64,
    /// Number of players in the session.
    pub num_players: u8,
    /// Commands received per frame, per player.
    received: BTreeMap<u64, BTreeMap<u8, CommandFrame>>,
}

impl LockstepState {
    /// Create a new lockstep state for the given number of players.
    pub fn new(num_players: u8, input_delay: u64) -> Self {
        Self {
            current_frame: 0,
            input_delay,
            num_players,
            received: BTreeMap::new(),
        }
    }

    /// Buffer the local player's commands for the given frame.
    pub fn submit_local_commands(&mut self, frame: u64, player_id: u8, commands: CommandFrame) {
        self.received
            .entry(frame)
            .or_default()
            .insert(player_id, commands);
    }

    /// Buffer a remote player's commands for the given frame.
    pub fn receive_remote_commands(&mut self, frame: u64, player_id: u8, commands: CommandFrame) {
        self.received
            .entry(frame)
            .or_default()
            .insert(player_id, commands);
    }

    /// Returns `true` when all players have submitted commands for `current_frame`.
    pub fn can_advance(&self) -> bool {
        match self.received.get(&self.current_frame) {
            Some(players) => players.len() as u8 >= self.num_players,
            None => false,
        }
    }

    /// Consume all commands for `current_frame`, increment the frame counter,
    /// and return the collected command frames (sorted by player id).
    ///
    /// # Panics
    ///
    /// Panics if called when [`can_advance`](Self::can_advance) is `false`.
    pub fn advance(&mut self) -> Vec<CommandFrame> {
        assert!(
            self.can_advance(),
            "cannot advance: not all players have submitted for frame {}",
            self.current_frame
        );

        let players = self
            .received
            .remove(&self.current_frame)
            .expect("frame entry must exist after can_advance check");

        self.current_frame += 1;

        // Return commands sorted by player id (BTreeMap iterates in order).
        players.into_values().collect()
    }

    /// The frame the local player should submit commands for.
    ///
    /// This is always `current_frame + input_delay`, allowing the network
    /// time to deliver commands before the sim needs them.
    pub fn pending_frame(&self) -> u64 {
        self.current_frame + self.input_delay
    }
}

#[cfg(test)]
#[path = "tests/lockstep_tests.rs"]
mod tests;
