use super::*;

fn sample_replays() -> Vec<ReplayEntry> {
    vec![
        ReplayEntry {
            filename: "replay_001.bin".into(),
            map_name: "Dustbowl".into(),
            num_players: 2,
            total_frames: 18_000,
            date: "2026-04-05 10:00".into(),
        },
        ReplayEntry {
            filename: "replay_002.bin".into(),
            map_name: "Glacier".into(),
            num_players: 4,
            total_frames: 36_000,
            date: "2026-04-06 14:30".into(),
        },
    ]
}

#[test]
fn browser_renders_without_panic() {
    let ctx = egui::Context::default();
    let replays = sample_replays();
    let mut action = ReplayBrowserAction::None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        action = draw_replay_browser(ctx, &replays);
    });
    // No interaction means no action.
    assert_eq!(action, ReplayBrowserAction::None);
}

#[test]
fn controls_render_without_panic() {
    let ctx = egui::Context::default();
    let mut speed = 1;
    let mut paused = false;
    let mut action = ReplayControlAction::None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            action = draw_replay_controls(ui, 100, 18_000, &mut speed, &mut paused);
        });
    });
    assert_eq!(action, ReplayControlAction::None);
    assert_eq!(speed, 1);
    assert!(!paused);
}

#[test]
fn browser_action_variants() {
    // Verify enum variants are distinct.
    assert_ne!(ReplayBrowserAction::None, ReplayBrowserAction::Play(0));
    assert_ne!(ReplayBrowserAction::Play(0), ReplayBrowserAction::Delete(0));
    assert_ne!(ReplayBrowserAction::None, ReplayBrowserAction::Delete(0));
}

#[test]
fn control_action_variants() {
    assert_ne!(ReplayControlAction::None, ReplayControlAction::Seek(0));
    assert_ne!(ReplayControlAction::None, ReplayControlAction::TogglePause);
    assert_ne!(ReplayControlAction::None, ReplayControlAction::SetSpeed(1));
}
