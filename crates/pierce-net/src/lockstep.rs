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
mod tests {
    use super::*;
    use crate::protocol::PlayerCommand;

    fn make_command_frame(frame: u64, player_id: u8) -> CommandFrame {
        CommandFrame {
            frame,
            player_id,
            commands: vec![],
        }
    }

    fn make_command_frame_with_cmd(frame: u64, player_id: u8) -> CommandFrame {
        use pierce_sim::{Command, SimVec3};
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
    fn pending_frame_returns_current_plus_delay() {
        let state = LockstepState::new(2, 3);
        assert_eq!(state.pending_frame(), 3);
    }

    #[test]
    fn cannot_advance_without_all_players() {
        let mut state = LockstepState::new(2, 2);
        // Only player 0 submits.
        state.submit_local_commands(0, 0, make_command_frame(0, 0));
        assert!(!state.can_advance());
    }

    #[test]
    fn can_advance_when_all_players_submitted() {
        let mut state = LockstepState::new(2, 2);
        state.submit_local_commands(0, 0, make_command_frame(0, 0));
        state.receive_remote_commands(0, 1, make_command_frame(0, 1));
        assert!(state.can_advance());
    }

    #[test]
    fn advance_returns_commands_and_increments_frame() {
        let mut state = LockstepState::new(2, 2);
        state.submit_local_commands(0, 0, make_command_frame(0, 0));
        state.receive_remote_commands(0, 1, make_command_frame(0, 1));

        let cmds = state.advance();
        assert_eq!(cmds.len(), 2);
        assert_eq!(state.current_frame, 1);
    }

    #[test]
    fn advance_returns_commands_sorted_by_player_id() {
        let mut state = LockstepState::new(3, 2);
        // Submit out of order.
        state.receive_remote_commands(0, 2, make_command_frame(0, 2));
        state.submit_local_commands(0, 0, make_command_frame(0, 0));
        state.receive_remote_commands(0, 1, make_command_frame(0, 1));

        let cmds = state.advance();
        assert_eq!(cmds[0].player_id, 0);
        assert_eq!(cmds[1].player_id, 1);
        assert_eq!(cmds[2].player_id, 2);
    }

    #[test]
    fn multiple_frames_queued() {
        let mut state = LockstepState::new(2, 2);

        // Queue frames 0 and 1 for both players.
        for frame in 0..2 {
            state.submit_local_commands(frame, 0, make_command_frame(frame, 0));
            state.receive_remote_commands(frame, 1, make_command_frame(frame, 1));
        }

        assert!(state.can_advance());
        let cmds0 = state.advance();
        assert_eq!(cmds0.len(), 2);
        assert_eq!(cmds0[0].frame, 0);

        assert!(state.can_advance());
        let cmds1 = state.advance();
        assert_eq!(cmds1.len(), 2);
        assert_eq!(cmds1[0].frame, 1);

        assert_eq!(state.current_frame, 2);
    }

    #[test]
    fn pending_frame_updates_after_advance() {
        let mut state = LockstepState::new(2, 2);
        assert_eq!(state.pending_frame(), 2);

        state.submit_local_commands(0, 0, make_command_frame(0, 0));
        state.receive_remote_commands(0, 1, make_command_frame(0, 1));
        state.advance();

        assert_eq!(state.pending_frame(), 3);
    }

    #[test]
    fn commands_with_data_are_preserved() {
        let mut state = LockstepState::new(1, 0);
        state.submit_local_commands(0, 0, make_command_frame_with_cmd(0, 0));

        let cmds = state.advance();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].commands.len(), 1);
        assert_eq!(cmds[0].commands[0].target_sim_id, 42);
    }

    #[test]
    #[should_panic(expected = "cannot advance")]
    fn advance_panics_when_not_ready() {
        let mut state = LockstepState::new(2, 2);
        state.advance();
    }
}
