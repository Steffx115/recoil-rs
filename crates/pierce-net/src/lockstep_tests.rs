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
