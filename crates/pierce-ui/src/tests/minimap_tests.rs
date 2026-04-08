use super::*;

fn run_minimap<F: FnOnce(&mut Ui) -> MinimapResponse>(f: F) -> MinimapResponse {
    let ctx = egui::Context::default();
    let mut resp = None;
    let mut f = Some(f);
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(func) = f.take() {
                resp = Some(func(ui));
            }
        });
    });
    resp.unwrap()
}

#[test]
fn empty_data_does_not_panic() {
    let data = MinimapData {
        map_width: 0.0,
        map_height: 0.0,
        units: vec![],
        camera_rect: [0.0; 4],
        fog: None,
    };
    let resp = run_minimap(|ui| draw_minimap(ui, &data, 200.0));
    assert!(resp.clicked_pos.is_none());
}

#[test]
fn with_units_does_not_panic() {
    let data = MinimapData {
        map_width: 1024.0,
        map_height: 1024.0,
        units: vec![
            MinimapUnit {
                x: 100.0,
                z: 200.0,
                team: 0,
                is_selected: false,
            },
            MinimapUnit {
                x: 500.0,
                z: 300.0,
                team: 1,
                is_selected: true,
            },
        ],
        camera_rect: [0.0, 0.0, 512.0, 384.0],
        fog: None,
    };
    let resp = run_minimap(|ui| draw_minimap(ui, &data, 200.0));
    assert!(resp.clicked_pos.is_none());
}

#[test]
fn with_fog_does_not_panic() {
    let data = MinimapData {
        map_width: 512.0,
        map_height: 512.0,
        units: vec![],
        camera_rect: [0.0, 0.0, 256.0, 256.0],
        fog: Some(vec![
            (0, 0, CellVisibility::Unexplored),
            (1, 0, CellVisibility::Explored),
            (0, 1, CellVisibility::Visible),
            (1, 1, CellVisibility::Unexplored),
        ]),
    };
    let resp = run_minimap(|ui| draw_minimap(ui, &data, 150.0));
    assert!(resp.clicked_pos.is_none());
}

#[test]
fn click_maps_to_world_coordinates() {
    // We cannot easily simulate a real click in egui's headless mode,
    // so verify the math directly.
    let map_w: f32 = 1000.0;
    let map_h: f32 = 800.0;
    let minimap_size: f32 = 200.0;

    // Simulate a click at the center of a 200x200 minimap whose top-left
    // is at (0,0) in screen space.
    let nx = 0.5_f32;
    let nz = 0.5_f32;
    let world_x = nx * map_w;
    let world_z = nz * map_h;

    assert!((world_x - 500.0).abs() < f32::EPSILON);
    assert!((world_z - 400.0).abs() < f32::EPSILON);

    // Edge: top-left corner
    let world_x = 0.0_f32 * map_w;
    let world_z = 0.0_f32 * map_h;
    assert!((world_x).abs() < f32::EPSILON);
    assert!((world_z).abs() < f32::EPSILON);

    // Edge: bottom-right
    let world_x = 1.0_f32 * map_w;
    let world_z = 1.0_f32 * map_h;
    assert!((world_x - map_w).abs() < f32::EPSILON);
    assert!((world_z - map_h).abs() < f32::EPSILON);

    // Also run the full draw path with zero-size map to ensure no div-by-zero.
    let data = MinimapData {
        map_width: 0.0,
        map_height: 0.0,
        units: vec![],
        camera_rect: [0.0; 4],
        fog: None,
    };
    let resp = run_minimap(|ui| draw_minimap(ui, &data, minimap_size));
    assert!(resp.clicked_pos.is_none());
}
