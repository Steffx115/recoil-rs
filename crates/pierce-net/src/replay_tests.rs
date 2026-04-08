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
