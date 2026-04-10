use super::*;
use crate::protocol::PlayerCommand;
use pierce_sim::{Command, SimVec3};

fn move_cmd(target_sim_id: u64, x: i32, z: i32) -> PlayerCommand {
    PlayerCommand {
        target_sim_id,
        command: Command::Move(SimVec3::new(
            pierce_sim::SimFloat::from_int(x),
            pierce_sim::SimFloat::ZERO,
            pierce_sim::SimFloat::from_int(z),
        )),
    }
}

fn stop_cmd(target_sim_id: u64) -> PlayerCommand {
    PlayerCommand {
        target_sim_id,
        command: Command::Stop,
    }
}

// -----------------------------------------------------------------------
// Basic operation
// -----------------------------------------------------------------------

#[test]
fn advance_always_succeeds() {
    let mut st = ServerTick::new(2);
    // No commands — still advances.
    let frames = st.advance();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].commands.len(), 0);
    assert_eq!(frames[1].commands.len(), 0);
    assert_eq!(st.current_frame, 1);
}

#[test]
fn commands_appear_in_correct_player_frame() {
    let mut st = ServerTick::new(2);
    st.receive_commands(1, &[move_cmd(42, 10, 20)]);

    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 0, "Player 0 sent nothing");
    assert_eq!(frames[1].commands.len(), 1, "Player 1 sent one command");
    assert_eq!(frames[1].commands[0].target_sim_id, 42);
}

#[test]
fn frame_counter_increments() {
    let mut st = ServerTick::new(1);
    assert_eq!(st.current_frame, 0);
    st.advance();
    assert_eq!(st.current_frame, 1);
    st.advance();
    assert_eq!(st.current_frame, 2);
}

// -----------------------------------------------------------------------
// Deduplication
// -----------------------------------------------------------------------

#[test]
fn dedup_same_unit_same_kind_keeps_latest() {
    let mut st = ServerTick::new(1);
    st.receive_commands(0, &[move_cmd(1, 10, 10)]);
    st.receive_commands(0, &[move_cmd(1, 99, 99)]); // overwrites

    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 1);
    // The latest move should win.
    match &frames[0].commands[0].command {
        Command::Move(pos) => {
            assert_eq!(pos.x, pierce_sim::SimFloat::from_int(99));
        }
        _ => panic!("expected Move"),
    }
}

#[test]
fn dedup_different_units_both_kept() {
    let mut st = ServerTick::new(1);
    st.receive_commands(0, &[move_cmd(1, 10, 10), move_cmd(2, 20, 20)]);

    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 2);
}

#[test]
fn dedup_different_kinds_same_unit_both_kept() {
    let mut st = ServerTick::new(1);
    st.receive_commands(0, &[move_cmd(1, 10, 10), stop_cmd(1)]);

    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 2);
}

#[test]
fn dedup_different_players_independent() {
    let mut st = ServerTick::new(2);
    st.receive_commands(0, &[move_cmd(1, 10, 10)]);
    st.receive_commands(1, &[move_cmd(1, 20, 20)]); // same unit, different player

    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 1);
    assert_eq!(frames[1].commands.len(), 1);
}

// -----------------------------------------------------------------------
// Pending count
// -----------------------------------------------------------------------

#[test]
fn pending_count_tracks_deduplicated_entries() {
    let mut st = ServerTick::new(1);
    st.receive_commands(0, &[move_cmd(1, 10, 10)]);
    assert_eq!(st.pending_count(), 1);

    // Same unit+kind → still 1.
    st.receive_commands(0, &[move_cmd(1, 20, 20)]);
    assert_eq!(st.pending_count(), 1);

    // Different unit → 2.
    st.receive_commands(0, &[move_cmd(2, 30, 30)]);
    assert_eq!(st.pending_count(), 2);

    st.advance();
    assert_eq!(st.pending_count(), 0);
}

// -----------------------------------------------------------------------
// Commands cleared after advance
// -----------------------------------------------------------------------

#[test]
fn advance_clears_pending() {
    let mut st = ServerTick::new(1);
    st.receive_commands(0, &[move_cmd(1, 10, 10)]);
    st.advance();

    // Second advance should produce empty frames.
    let frames = st.advance();
    assert_eq!(frames[0].commands.len(), 0);
}

// -----------------------------------------------------------------------
// Determinism
// -----------------------------------------------------------------------

#[test]
fn deterministic_output_order() {
    // Run twice with same inputs, expect identical output.
    fn run() -> Vec<CommandFrame> {
        let mut st = ServerTick::new(2);
        st.receive_commands(1, &[move_cmd(5, 50, 50), stop_cmd(3)]);
        st.receive_commands(0, &[move_cmd(1, 10, 10), move_cmd(2, 20, 20)]);
        st.advance()
    }

    let a = run();
    let b = run();

    // Same player ordering.
    assert_eq!(a.len(), b.len());
    for (fa, fb) in a.iter().zip(b.iter()) {
        assert_eq!(fa.player_id, fb.player_id);
        assert_eq!(fa.frame, fb.frame);
        assert_eq!(fa.commands.len(), fb.commands.len());
        for (ca, cb) in fa.commands.iter().zip(fb.commands.iter()) {
            assert_eq!(ca.target_sim_id, cb.target_sim_id);
        }
    }
}
