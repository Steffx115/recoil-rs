//! Bottom status bar showing FPS, frame count, unit count, and paused state.

use egui::Ui;

/// Draw the bottom status bar.
pub fn draw_status_bar(ui: &mut Ui, fps: f32, frame: u64, unit_count: usize, paused: bool) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("FPS: {fps:.0}"))
                .monospace()
                .color(if fps < 30.0 {
                    egui::Color32::from_rgb(220, 80, 80)
                } else {
                    egui::Color32::from_rgb(160, 220, 160)
                }),
        );

        ui.separator();
        ui.label(egui::RichText::new(format!("Frame: {frame}")).monospace());

        ui.separator();
        ui.label(egui::RichText::new(format!("Units: {unit_count}")).monospace());

        if paused {
            ui.separator();
            ui.label(
                egui::RichText::new("PAUSED")
                    .strong()
                    .color(egui::Color32::YELLOW),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bar_running() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_status_bar(ui, 60.0, 1234, 42, false);
            });
        });
    }

    #[test]
    fn status_bar_paused_low_fps() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_status_bar(ui, 15.0, 9999, 200, true);
            });
        });
    }
}
