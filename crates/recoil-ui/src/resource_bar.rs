//! Top-of-screen resource bar showing metal and energy economy.

use egui::Ui;
use recoil_sim::economy::TeamResources;
use recoil_sim::SimFloat;

/// Parameters for drawing a single resource row.
struct ResourceRowParams {
    current: SimFloat,
    storage: SimFloat,
    income: SimFloat,
    expense: SimFloat,
    stall_ratio: SimFloat,
    bar_color: egui::Color32,
    stall_color: egui::Color32,
}

/// Draw a single resource row (metal or energy).
fn draw_resource_row(ui: &mut Ui, label: &str, params: &ResourceRowParams) {
    let cur_f = params.current.to_f32();
    let max_f = params.storage.to_f32().max(1.0);
    let fraction = (cur_f / max_f).clamp(0.0, 1.0);
    let inc_f = params.income.to_f32();
    let exp_f = params.expense.to_f32();
    let stall_f = params.stall_ratio.to_f32();
    let is_stalling = stall_f < 1.0;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).strong().monospace());

        // Progress bar
        let bar = egui::ProgressBar::new(fraction)
            .text(format!("{:.0} / {:.0}", cur_f, max_f))
            .fill(if is_stalling {
                params.stall_color
            } else {
                params.bar_color
            });
        ui.add_sized([160.0, 18.0], bar);

        // Income / expense
        ui.label(
            egui::RichText::new(format!("+{:.1}", inc_f))
                .monospace()
                .color(egui::Color32::from_rgb(100, 220, 100)),
        );
        ui.label(
            egui::RichText::new(format!("-{:.1}", exp_f))
                .monospace()
                .color(egui::Color32::from_rgb(220, 100, 100)),
        );

        // Stall indicator
        if is_stalling {
            ui.label(
                egui::RichText::new(format!("STALL {:.0}%", stall_f * 100.0))
                    .strong()
                    .color(egui::Color32::YELLOW),
            );
        }
    });
}

/// Draw the resource bar for a team. Place this in a top-panel.
pub fn draw_resource_bar(ui: &mut Ui, team_res: &TeamResources) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;

        // Metal
        draw_resource_row(
            ui,
            "Metal:",
            &ResourceRowParams {
                current: team_res.metal,
                storage: team_res.metal_storage,
                income: team_res.metal_income,
                expense: team_res.metal_expense,
                stall_ratio: team_res.stall_ratio_metal,
                bar_color: egui::Color32::from_rgb(180, 180, 180), // silver
                stall_color: egui::Color32::YELLOW,
            },
        );

        ui.separator();

        // Energy
        draw_resource_row(
            ui,
            "Energy:",
            &ResourceRowParams {
                current: team_res.energy,
                storage: team_res.energy_storage,
                income: team_res.energy_income,
                expense: team_res.energy_expense,
                stall_ratio: team_res.stall_ratio_energy,
                bar_color: egui::Color32::from_rgb(255, 220, 50), // gold/yellow
                stall_color: egui::Color32::YELLOW,
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_bar_does_not_panic_default() {
        let ctx = egui::Context::default();
        let team_res = TeamResources::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_resource_bar(ui, &team_res);
            });
        });
    }

    #[test]
    fn resource_bar_does_not_panic_stalling() {
        let ctx = egui::Context::default();
        let team_res = TeamResources {
            metal: SimFloat::from_int(100),
            metal_storage: SimFloat::from_int(2000),
            metal_income: SimFloat::from_int(5),
            metal_expense: SimFloat::from_int(50),
            energy: SimFloat::from_int(50),
            energy_storage: SimFloat::from_int(2000),
            energy_income: SimFloat::ZERO,
            energy_expense: SimFloat::from_int(100),
            stall_ratio_metal: SimFloat::HALF,
            stall_ratio_energy: SimFloat::from_ratio(1, 4),
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_resource_bar(ui, &team_res);
            });
        });
    }
}
