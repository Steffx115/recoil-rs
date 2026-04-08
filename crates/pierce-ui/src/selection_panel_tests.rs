use super::*;

fn make_info(id: u64, unit_type: u32, hp_frac: f32) -> SelectedUnitInfo {
    let max = SimFloat::from_int(100);
    SelectedUnitInfo {
        sim_id: id,
        unit_type,
        hp_current: SimFloat::from_int((hp_frac * 100.0) as i32),
        hp_max: max,
        position: [10.0, 20.0, 0.0],
        current_order: Some("Move".into()),
        stunned_frames: None,
    }
}

#[test]
fn selection_panel_empty() {
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            draw_selection_panel(ui, &[]);
        });
    });
}

#[test]
fn selection_panel_single() {
    let ctx = egui::Context::default();
    let info = make_info(1, 10, 0.8);
    let selection = [info];
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            draw_selection_panel(ui, &selection);
        });
    });
}

#[test]
fn selection_panel_multiple() {
    let ctx = egui::Context::default();
    let infos = vec![
        make_info(1, 10, 1.0),
        make_info(2, 10, 0.5),
        make_info(3, 20, 0.2),
    ];
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            draw_selection_panel(ui, &infos);
        });
    });
}

#[test]
fn selection_panel_stunned() {
    let ctx = egui::Context::default();
    let mut info = make_info(1, 10, 0.5);
    info.stunned_frames = Some(30);
    let selection = [info];
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            draw_selection_panel(ui, &selection);
        });
    });
}
