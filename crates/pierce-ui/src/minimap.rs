//! Minimap widget for the Pierce RTS engine HUD.

use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2};
use pierce_sim::fog::CellVisibility;

/// Data needed to draw the minimap, extracted from ECS before drawing.
pub struct MinimapData {
    pub map_width: f32,
    pub map_height: f32,
    pub units: Vec<MinimapUnit>,
    /// `[x, z, w, h]` of the camera viewport projected onto the ground plane.
    pub camera_rect: [f32; 4],
    /// Optional fog-of-war overlay: `(cell_x, cell_y, visibility)`.
    pub fog: Option<Vec<(u32, u32, CellVisibility)>>,
}

/// A single unit dot on the minimap.
pub struct MinimapUnit {
    pub x: f32,
    pub z: f32,
    pub team: u8,
    pub is_selected: bool,
}

/// Response returned after drawing the minimap.
pub struct MinimapResponse {
    /// If the player clicked the minimap, this holds the corresponding world
    /// coordinates `(x, z)`.
    pub clicked_pos: Option<(f32, f32)>,
}

/// Team color lookup.
fn team_color(team: u8) -> Color32 {
    match team {
        0 => Color32::from_rgb(60, 120, 255), // blue
        1 => Color32::from_rgb(230, 50, 50),  // red
        2 => Color32::from_rgb(40, 200, 40),  // green
        3 => Color32::from_rgb(255, 200, 40), // gold
        _ => Color32::from_rgb(180, 180, 180),
    }
}

/// Draw the minimap widget.
///
/// `size` is the side-length of the square minimap in UI pixels.
/// Returns a [`MinimapResponse`] indicating whether the player clicked on it.
pub fn draw_minimap(ui: &mut Ui, data: &MinimapData, size: f32) -> MinimapResponse {
    let (response, painter) = ui.allocate_painter(Vec2::splat(size), Sense::click());
    let minimap_rect = response.rect;

    // --- background (terrain green) ---
    painter.rect_filled(minimap_rect, 0.0, Color32::from_rgb(30, 80, 30));

    // Helper: world coords -> minimap pixel position.
    let to_screen = |wx: f32, wz: f32| -> Pos2 {
        let nx = if data.map_width > 0.0 {
            wx / data.map_width
        } else {
            0.0
        };
        let nz = if data.map_height > 0.0 {
            wz / data.map_height
        } else {
            0.0
        };
        Pos2::new(
            minimap_rect.min.x + nx * minimap_rect.width(),
            minimap_rect.min.y + nz * minimap_rect.height(),
        )
    };

    // --- fog overlay ---
    if let Some(fog) = &data.fog {
        if data.map_width > 0.0 && data.map_height > 0.0 {
            // Determine grid dimensions from the data so we can size cells.
            let max_cx = fog.iter().map(|(cx, _, _)| *cx).max().unwrap_or(0) + 1;
            let max_cy = fog.iter().map(|(_, cy, _)| *cy).max().unwrap_or(0) + 1;

            let cell_w = minimap_rect.width() / max_cx as f32;
            let cell_h = minimap_rect.height() / max_cy as f32;

            for &(cx, cy, vis) in fog {
                let alpha = match vis {
                    CellVisibility::Unexplored => 200,
                    CellVisibility::Explored => 100,
                    CellVisibility::Visible => continue,
                };
                let cell_rect = Rect::from_min_size(
                    Pos2::new(
                        minimap_rect.min.x + cx as f32 * cell_w,
                        minimap_rect.min.y + cy as f32 * cell_h,
                    ),
                    Vec2::new(cell_w, cell_h),
                );
                painter.rect_filled(cell_rect, 0.0, Color32::from_black_alpha(alpha));
            }
        }
    }

    // --- unit dots ---
    let dot_radius = (size / 80.0).max(2.0);
    for unit in &data.units {
        let center = to_screen(unit.x, unit.z);
        let color = team_color(unit.team);
        painter.circle_filled(center, dot_radius, color);
        if unit.is_selected {
            painter.circle_stroke(
                center,
                dot_radius + 1.5,
                egui::Stroke::new(1.0, Color32::YELLOW),
            );
        }
    }

    // --- camera viewport rectangle ---
    let cam_min = to_screen(data.camera_rect[0], data.camera_rect[1]);
    let cam_max = to_screen(
        data.camera_rect[0] + data.camera_rect[2],
        data.camera_rect[1] + data.camera_rect[3],
    );
    let cam_rect = Rect::from_min_max(cam_min, cam_max);
    painter.rect_stroke(
        cam_rect,
        0.0,
        egui::Stroke::new(1.0, Color32::WHITE),
        egui::StrokeKind::Outside,
    );

    // --- click handling ---
    let clicked_pos = if response.clicked() {
        response.interact_pointer_pos().map(|pos| {
            let nx = ((pos.x - minimap_rect.min.x) / minimap_rect.width()).clamp(0.0, 1.0);
            let nz = ((pos.y - minimap_rect.min.y) / minimap_rect.height()).clamp(0.0, 1.0);
            (nx * data.map_width, nz * data.map_height)
        })
    } else {
        None
    };

    MinimapResponse { clicked_pos }
}

#[cfg(test)]
#[path = "minimap_tests.rs"]
mod tests;
