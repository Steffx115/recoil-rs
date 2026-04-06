//! Command panel with action buttons for selected units.

use egui::Ui;

/// A command issued via the UI command panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCommand {
    Move,
    Stop,
    HoldPosition,
    Patrol,
    Attack,
}

/// Draw the command panel and return which button was clicked, if any.
pub fn draw_command_panel(ui: &mut Ui) -> Option<UiCommand> {
    let mut result = None;

    ui.horizontal(|ui| {
        if ui.button("Move").clicked() {
            result = Some(UiCommand::Move);
        }
        if ui.button("Stop").clicked() {
            result = Some(UiCommand::Stop);
        }
        if ui.button("Hold").clicked() {
            result = Some(UiCommand::HoldPosition);
        }
        if ui.button("Patrol").clicked() {
            result = Some(UiCommand::Patrol);
        }
        if ui.button("Attack").clicked() {
            result = Some(UiCommand::Attack);
        }
    });

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_panel_no_click() {
        let ctx = egui::Context::default();
        let mut cmd = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                cmd = draw_command_panel(ui);
            });
        });
        // No interaction => None
        assert_eq!(cmd, None);
    }
}
