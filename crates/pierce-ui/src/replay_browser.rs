//! Replay browser and playback controls UI.

use egui::{Context, Ui};

/// Metadata for a single replay file shown in the browser.
#[derive(Debug, Clone)]
pub struct ReplayEntry {
    pub filename: String,
    pub map_name: String,
    pub num_players: u8,
    pub total_frames: u64,
    /// Human-readable date string (e.g. "2026-04-06 14:30").
    pub date: String,
}

/// Action returned by [`draw_replay_browser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayBrowserAction {
    /// No action taken.
    None,
    /// User chose to play the replay at the given index.
    Play(usize),
    /// User chose to delete the replay at the given index.
    Delete(usize),
}

/// Action returned by [`draw_replay_controls`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayControlAction {
    /// No action taken.
    None,
    /// Seek to the given frame.
    Seek(u64),
    /// Toggle pause/play.
    TogglePause,
    /// Set playback speed multiplier.
    SetSpeed(i32),
}

/// Draw the replay browser window. Returns the user's action (play, delete, or none).
///
/// `replays` is the list of available replays. The selected replay index is
/// tracked internally via egui's transient memory.
pub fn draw_replay_browser(ctx: &Context, replays: &[ReplayEntry]) -> ReplayBrowserAction {
    let mut action = ReplayBrowserAction::None;

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Replay Browser");
        ui.separator();

        // Persist selected index across frames via egui memory.
        let selected_id = ui.id().with("replay_selected");
        let mut selected: Option<usize> = ui.data(|d| d.get_temp(selected_id));

        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                egui::Grid::new("replay_table")
                    .striped(true)
                    .num_columns(5)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        // Header row
                        ui.strong("Filename");
                        ui.strong("Map");
                        ui.strong("Players");
                        ui.strong("Frames");
                        ui.strong("Date");
                        ui.end_row();

                        for (i, entry) in replays.iter().enumerate() {
                            let is_selected = selected == Some(i);
                            let label = egui::SelectableLabel::new(is_selected, &entry.filename);
                            if ui.add(label).clicked() {
                                selected = Some(i);
                            }
                            ui.label(&entry.map_name);
                            ui.label(entry.num_players.to_string());
                            ui.label(entry.total_frames.to_string());
                            ui.label(&entry.date);
                            ui.end_row();
                        }
                    });
            });

        ui.separator();

        ui.horizontal(|ui| {
            let has_selection = selected.is_some();
            if ui
                .add_enabled(has_selection, egui::Button::new("Play"))
                .clicked()
            {
                if let Some(idx) = selected {
                    action = ReplayBrowserAction::Play(idx);
                }
            }
            if ui
                .add_enabled(has_selection, egui::Button::new("Delete"))
                .clicked()
            {
                if let Some(idx) = selected {
                    action = ReplayBrowserAction::Delete(idx);
                }
            }
        });

        // Store selection back.
        ui.data_mut(|d| d.insert_temp(selected_id, selected));
    });

    action
}

/// Draw replay playback controls (timeline, speed, pause).
///
/// Mutates `speed` and `paused` in place and returns a [`ReplayControlAction`]
/// when the user interacts with the controls.
pub fn draw_replay_controls(
    ui: &mut Ui,
    current_frame: u64,
    total_frames: u64,
    speed: &mut i32,
    paused: &mut bool,
) -> ReplayControlAction {
    let mut action = ReplayControlAction::None;

    ui.horizontal(|ui| {
        // Pause / play toggle
        let pause_label = if *paused {
            "\u{25B6} Play"
        } else {
            "\u{23F8} Pause"
        };
        if ui.button(pause_label).clicked() {
            *paused = !*paused;
            action = ReplayControlAction::TogglePause;
        }

        ui.separator();

        // Speed buttons
        for s in [1, 2, 4, 8] {
            let label = format!("{s}x");
            let btn = egui::SelectableLabel::new(*speed == s, label);
            if ui.add(btn).clicked() {
                *speed = s;
                action = ReplayControlAction::SetSpeed(s);
            }
        }
    });

    // Timeline scrub bar
    let mut seek_frame = current_frame;
    let slider = egui::Slider::new(&mut seek_frame, 0..=total_frames)
        .text("Frame")
        .clamping(egui::SliderClamping::Always);
    if ui.add(slider).changed() && seek_frame != current_frame {
        action = ReplayControlAction::Seek(seek_frame);
    }

    // Frame counter
    ui.label(format!("Frame {current_frame} / {total_frames}"));

    action
}

#[cfg(test)]
#[path = "replay_browser_tests.rs"]
mod tests;
