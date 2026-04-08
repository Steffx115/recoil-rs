//! Panel showing info about the currently selected unit(s).

use std::collections::BTreeMap;

use egui::Ui;
use pierce_sim::SimFloat;

/// Snapshot of a single selected unit, extracted from ECS components.
#[derive(Debug, Clone)]
pub struct SelectedUnitInfo {
    /// Unique sim entity id.
    pub sim_id: u64,
    /// Unit type id.
    pub unit_type: u32,
    /// Current health.
    pub hp_current: SimFloat,
    /// Maximum health.
    pub hp_max: SimFloat,
    /// World position (x, y, z as f32 for display).
    pub position: [f32; 3],
    /// Human-readable description of the current order, if any.
    pub current_order: Option<String>,
    /// Whether the unit is stunned (and remaining frames).
    pub stunned_frames: Option<u32>,
}

/// Pick a colour for the HP bar based on the fraction remaining.
fn hp_color(fraction: f32) -> egui::Color32 {
    if fraction > 0.6 {
        egui::Color32::from_rgb(60, 200, 60) // green
    } else if fraction > 0.3 {
        egui::Color32::from_rgb(220, 200, 40) // yellow
    } else {
        egui::Color32::from_rgb(220, 50, 50) // red
    }
}

/// Draw info for a single selected unit.
fn draw_single(ui: &mut Ui, info: &SelectedUnitInfo) {
    let hp_cur = info.hp_current.to_f32();
    let hp_max = info.hp_max.to_f32().max(1.0);
    let fraction = (hp_cur / hp_max).clamp(0.0, 1.0);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("ID: {}", info.sim_id))
                .monospace()
                .strong(),
        );
        ui.label(egui::RichText::new(format!("Type: {}", info.unit_type)).monospace());
    });

    // HP bar
    let bar = egui::ProgressBar::new(fraction)
        .text(format!("{:.0} / {:.0}", hp_cur, hp_max))
        .fill(hp_color(fraction));
    ui.add_sized([180.0, 16.0], bar);

    // Position
    ui.label(
        egui::RichText::new(format!(
            "Pos: ({:.1}, {:.1}, {:.1})",
            info.position[0], info.position[1], info.position[2],
        ))
        .monospace(),
    );

    // Current order
    if let Some(order) = &info.current_order {
        ui.label(egui::RichText::new(format!("Order: {order}")).monospace());
    }

    // Stunned
    if let Some(frames) = info.stunned_frames {
        ui.label(
            egui::RichText::new(format!("STUNNED ({frames} frames)"))
                .strong()
                .color(egui::Color32::from_rgb(255, 120, 0)),
        );
    }
}

/// Draw the selection panel for one or more units.
///
/// If `selection` is empty, nothing is drawn.
/// If one unit, show full details.
/// If multiple, show a count + type breakdown, plus the first unit's details.
pub fn draw_selection_panel(ui: &mut Ui, selection: &[SelectedUnitInfo]) {
    match selection.len() {
        0 => {}
        1 => draw_single(ui, &selection[0]),
        n => {
            ui.label(
                egui::RichText::new(format!("{n} units selected"))
                    .strong()
                    .size(14.0),
            );

            // Type breakdown (BTreeMap for deterministic order)
            let mut type_counts: BTreeMap<u32, usize> = BTreeMap::new();
            for info in selection {
                *type_counts.entry(info.unit_type).or_default() += 1;
            }
            ui.horizontal_wrapped(|ui| {
                for (ut, count) in &type_counts {
                    ui.label(egui::RichText::new(format!("T{ut}: {count}")).monospace());
                }
            });

            ui.separator();
            draw_single(ui, &selection[0]);
        }
    }
}

#[cfg(test)]
#[path = "tests/selection_panel_tests.rs"]
mod tests;
