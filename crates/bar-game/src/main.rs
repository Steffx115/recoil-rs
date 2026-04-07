//! Recoil RTS — game binary.
//!
//! Thin shell: window, renderer, input dispatch, egui overlay.
//! All game logic lives in `bar-game-lib`.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use recoil_math::SimFloat;
use recoil_render::camera::Camera;
use recoil_render::particles::ParticleSystem;
use recoil_render::projectile_renderer::ProjectileInstance;
use recoil_render::unit_renderer::UnitInstance;
use recoil_render::Renderer;
use recoil_sim::construction::BuildSite;
use recoil_sim::economy::EconomyState;
use recoil_sim::selection::screen_to_ground_raw;
use recoil_sim::unit_defs::UnitDefRegistry;
use recoil_sim::{Allegiance, Dead, Heading, Health, Position, UnitType, Velocity};

use bar_game_lib::building::PlacementType;
use bar_game_lib::GameState;

use egui_wgpu::ScreenDescriptor;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const BAR_UNITS_PATH: &str = "../Beyond-All-Reason-Sandbox/units";
const MAP_MANIFEST_PATH: &str = "assets/maps/small_duel/manifest.ron";

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

const PAN_SPEED: f32 = 5.0;
const ZOOM_SPEED: f32 = 10.0;
const MIN_HEIGHT: f32 = 50.0;
const MAX_HEIGHT: f32 = 800.0;

struct CameraController {
    center: [f32; 2],
    height: f32,
    forward: bool,
    left: bool,
    backward: bool,
    right: bool,
}

impl CameraController {
    fn new(cx: f32, cz: f32, height: f32) -> Self {
        Self { center: [cx, cz], height, forward: false, left: false, backward: false, right: false }
    }

    fn process_key(&mut self, key: KeyCode, pressed: bool) {
        match key {
            KeyCode::KeyW => self.forward = pressed,
            KeyCode::KeyA => self.left = pressed,
            KeyCode::KeyS => self.backward = pressed,
            KeyCode::KeyD => self.right = pressed,
            _ => {}
        }
    }

    fn process_scroll(&mut self, delta: f32) {
        self.height = (self.height - delta * ZOOM_SPEED).clamp(MIN_HEIGHT, MAX_HEIGHT);
    }

    fn update(&mut self) {
        let speed = PAN_SPEED * (self.height / 400.0);
        if self.forward { self.center[1] -= speed; }
        if self.backward { self.center[1] += speed; }
        if self.left { self.center[0] -= speed; }
        if self.right { self.center[0] += speed; }
    }

    fn camera(&self, aspect: f32) -> Camera {
        Camera {
            eye: [self.center[0], self.height, self.center[1] + self.height * 0.75],
            target: [self.center[0], 0.0, self.center[1]],
            up: [0.0, 1.0, 0.0],
            fov_y: std::f32::consts::FRAC_PI_4,
            aspect,
            near: 1.0,
            far: 2000.0,
        }
    }
}

// ---------------------------------------------------------------------------
// FPS counter
// ---------------------------------------------------------------------------

struct FpsCounter {
    frame_times: VecDeque<Instant>,
}

impl FpsCounter {
    fn new() -> Self { Self { frame_times: VecDeque::with_capacity(120) } }

    fn tick(&mut self) -> f32 {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > 100 { self.frame_times.pop_front(); }
        if self.frame_times.len() < 2 { return 0.0; }
        let elapsed = now.duration_since(*self.frame_times.front().unwrap()).as_secs_f32();
        if elapsed > 0.0 { (self.frame_times.len() - 1) as f32 / elapsed } else { 0.0 }
    }
}

// ---------------------------------------------------------------------------
// Instance extraction from GameState
// ---------------------------------------------------------------------------

fn unit_instances(game: &mut GameState) -> Vec<UnitInstance> {
    let sel = game.selected();
    game.world
        .query_filtered::<(Entity, &Position, &Heading, &Allegiance, &Health, &UnitType, Option<&BuildSite>), Without<Dead>>()
        .iter(&game.world)
        .map(|(entity, pos, heading, al, hp, ut, bs)| {
            let mut c = if al.team == 0 { [0.2f32, 0.5, 0.9] } else { [0.9f32, 0.2, 0.2] };
            if bs.is_some() { c[0] *= 0.5; c[1] *= 0.5; c[2] *= 0.5; }
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f; c[1] *= f; c[2] *= f;
            if sel == Some(entity) { c[0] = (c[0]+0.3).min(1.0); c[1] = (c[1]+0.3).min(1.0); c[2] = (c[2]+0.3).min(1.0); }
            UnitInstance { position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()], heading: heading.angle.to_f32(), team_color: c, mesh_id: ut.id }
        })
        .collect()
}

fn building_instances(game: &mut GameState) -> Vec<UnitInstance> {
    let sel = game.selected();
    game.world
        .query_filtered::<(Entity, &Position, &Allegiance, &Health, &UnitType, Option<&BuildSite>), (Without<Dead>, Without<Heading>)>()
        .iter(&game.world)
        .map(|(entity, pos, al, hp, ut, bs)| {
            let mut c = if al.team == 0 { [0.1f32, 0.8, 0.3] } else { [0.8f32, 0.1, 0.3] };
            if bs.is_some() { c[0] *= 0.5; c[1] *= 0.5; c[2] *= 0.5; }
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f; c[1] *= f; c[2] *= f;
            if sel == Some(entity) { c[0] = (c[0]+0.3).min(1.0); c[1] = (c[1]+0.3).min(1.0); c[2] = (c[2]+0.3).min(1.0); }
            UnitInstance { position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()], heading: 0.0, team_color: c, mesh_id: ut.id }
        })
        .collect()
}

fn projectile_instances(game: &mut GameState) -> Vec<ProjectileInstance> {
    use recoil_sim::projectile::Projectile;
    game.world.query::<(&Position, &Velocity, &Projectile)>()
        .iter(&game.world)
        .map(|(pos, vel, _)| ProjectileInstance {
            position: [pos.pos.x.to_f32(), pos.pos.y.to_f32() + 2.0, pos.pos.z.to_f32()],
            size: 2.0,
            velocity_dir: [vel.vel.x.to_f32(), vel.vel.y.to_f32(), vel.vel.z.to_f32()],
            _pad: 0.0, color: [1.0, 0.8, 0.2], _pad2: 0.0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// UI data + egui drawing
// ---------------------------------------------------------------------------

/// Screen-space health bar for a unit.
struct HealthBarInfo {
    screen_x: f32,
    screen_y: f32,
    hp_frac: f32,
    build_frac: Option<f32>,
    is_selected: bool,
}

/// Minimap dot.
struct MinimapDot {
    /// Normalized position 0..1 on the map.
    nx: f32,
    nz: f32,
    team: u8,
    is_building: bool,
}

struct UiData {
    metal: f32, metal_storage: f32, metal_income: f32, metal_expense: f32,
    energy: f32, energy_storage: f32, energy_income: f32, energy_expense: f32,
    frame_count: u64, fps: f32, paused: bool,
    blue_count: usize, red_count: usize,
    selected_name: Option<String>,
    selected_hp: Option<(f32, f32)>,
    selected_is_factory: bool,
    selected_is_builder: bool,
    factory_queue_len: usize,
    placement_label: Option<String>,
    builder_options: Vec<(String, String, u32)>,
    factory_options: Vec<(String, String, u32)>,
    game_over: Option<bar_game_lib::GameOver>,
    health_bars: Vec<HealthBarInfo>,
    minimap_dots: Vec<MinimapDot>,
    /// Camera position on minimap (normalized).
    cam_nx: f32,
    cam_nz: f32,
}

/// Project a world position to screen coordinates.
fn world_to_screen(vp: &[[f32; 4]; 4], wx: f32, wy: f32, wz: f32, sw: f32, sh: f32) -> Option<(f32, f32)> {
    let x = vp[0][0]*wx + vp[1][0]*wy + vp[2][0]*wz + vp[3][0];
    let y = vp[0][1]*wx + vp[1][1]*wy + vp[2][1]*wz + vp[3][1];
    let w = vp[0][3]*wx + vp[1][3]*wy + vp[2][3]*wz + vp[3][3];
    if w.abs() < 1e-6 { return None; }
    let ndc_x = x / w;
    let ndc_y = y / w;
    // NDC is -1..1, convert to screen pixels.
    let sx = (ndc_x * 0.5 + 0.5) * sw;
    let sy = (1.0 - (ndc_y * 0.5 + 0.5)) * sh;
    if sx >= -100.0 && sx <= sw + 100.0 && sy >= -100.0 && sy <= sh + 100.0 {
        Some((sx, sy))
    } else {
        None
    }
}

const MAP_SIZE: f32 = 1024.0; // world units

fn gather_ui_data(game: &mut GameState, fps: f32, vp: &[[f32; 4]; 4], screen_size: [f32; 2], cam_center: [f32; 2]) -> UiData {
    let (metal, metal_storage, energy, energy_storage) = {
        let eco = game.world.resource::<EconomyState>();
        eco.teams.get(&0).map(|r| (r.metal.to_f32(), r.metal_storage.to_f32(), r.energy.to_f32(), r.energy_storage.to_f32())).unwrap_or_default()
    };
    let (metal_income, energy_income) = {
        use recoil_sim::economy::ResourceProducer;
        let (mut mi, mut ei) = (0.0f32, 0.0f32);
        for (prod, al) in game.world.query_filtered::<(&ResourceProducer, &Allegiance), Without<Dead>>().iter(&game.world) {
            if al.team == 0 { mi += prod.metal_per_tick.to_f32(); ei += prod.energy_per_tick.to_f32(); }
        }
        (mi, ei)
    };
    let (metal_expense, energy_expense) = {
        let (mut me, mut ee) = (0.0f32, 0.0f32);
        for (bs, al) in game.world.query_filtered::<(&BuildSite, &Allegiance), Without<Dead>>().iter(&game.world) {
            if al.team == 0 && bs.total_build_time > SimFloat::ZERO {
                me += bs.metal_cost.to_f32() / bs.total_build_time.to_f32();
                ee += bs.energy_cost.to_f32() / bs.total_build_time.to_f32();
            }
        }
        (me, ee)
    };

    let blue_count = game.world.query_filtered::<&Allegiance, Without<Dead>>().iter(&game.world).filter(|a| a.team == 0).count();
    let red_count = game.world.query_filtered::<&Allegiance, Without<Dead>>().iter(&game.world).filter(|a| a.team == 1).count();

    let selected_is_factory = game.selected_is_factory();
    let selected_is_builder = game.selected_is_builder();
    let mut selected_name = None;
    let mut selected_hp = None;
    let mut factory_queue_len = 0;
    let mut builder_options = Vec::new();
    let mut factory_options = Vec::new();

    if let Some(sel) = game.selected() {
        if game.world.get_entity(sel).is_ok() {
            if let Some(hp) = game.world.get::<Health>(sel) {
                selected_hp = Some((hp.current.to_f32(), hp.max.to_f32()));
            }
            if let Some(ut) = game.world.get::<UnitType>(sel) {
                let registry = game.world.resource::<UnitDefRegistry>();
                selected_name = Some(registry.get(ut.id).map(|d| d.name.clone()).unwrap_or_else(|| format!("#{}", ut.id)));

                let keys = ["1","2","3","4","5","6","7","8","9","0"];
                if let Some(def) = registry.get(ut.id) {
                    let list = &def.can_build;
                    let target = if selected_is_builder { &mut builder_options } else if selected_is_factory { &mut factory_options } else { &mut builder_options };
                    for (i, &bid) in list.iter().enumerate() {
                        if i >= keys.len() { break; }
                        if let Some(bd) = registry.get(bid) {
                            target.push((keys[i].to_string(), bd.name.clone(), bid));
                        }
                    }
                }
            }
            if let Some(bq) = game.world.get::<recoil_sim::factory::BuildQueue>(sel) {
                factory_queue_len = bq.queue.len();
            }
        }
    }

    let placement_label = game.placement_mode.map(|pt| {
        let registry = game.world.resource::<UnitDefRegistry>();
        pt.label(registry)
    });

    // --- Health bars: project world positions to screen ---
    let selected_set: std::collections::HashSet<Entity> = game.selection.selected.iter().copied().collect();
    let mut health_bars = Vec::new();
    for (entity, pos, hp, _al, bs) in game.world
        .query_filtered::<(Entity, &Position, &Health, &Allegiance, Option<&BuildSite>), Without<Dead>>()
        .iter(&game.world)
    {
        let frac = if hp.max > SimFloat::ZERO { hp.current.to_f32() / hp.max.to_f32() } else { 1.0 };
        let is_sel = selected_set.contains(&entity);
        // Only show if damaged, selected, or under construction.
        if frac >= 1.0 && !is_sel && bs.is_none() { continue; }
        let wx = pos.pos.x.to_f32();
        let wz = pos.pos.z.to_f32();
        if let Some((sx, sy)) = world_to_screen(vp, wx, 5.0, wz, screen_size[0], screen_size[1]) {
            health_bars.push(HealthBarInfo {
                screen_x: sx, screen_y: sy - 10.0,
                hp_frac: frac,
                build_frac: bs.map(|b| {
                    if b.total_build_time > SimFloat::ZERO { b.progress.to_f32() / SimFloat::ONE.to_f32() } else { 0.0 }
                }),
                is_selected: is_sel,
            });
        }
    }

    // --- Minimap dots ---
    let mut minimap_dots = Vec::new();
    for (pos, al, heading) in game.world
        .query_filtered::<(&Position, &Allegiance, Option<&Heading>), Without<Dead>>()
        .iter(&game.world)
    {
        minimap_dots.push(MinimapDot {
            nx: pos.pos.x.to_f32() / MAP_SIZE,
            nz: pos.pos.z.to_f32() / MAP_SIZE,
            team: al.team,
            is_building: heading.is_none(),
        });
    }

    UiData {
        metal, metal_storage, metal_income, metal_expense,
        energy, energy_storage, energy_income, energy_expense,
        frame_count: game.frame_count, fps, paused: game.paused,
        blue_count, red_count,
        selected_name, selected_hp,
        selected_is_factory, selected_is_builder,
        factory_queue_len, placement_label,
        builder_options, factory_options,
        game_over: game.game_over.clone(),
        health_bars,
        minimap_dots,
        cam_nx: cam_center[0] / MAP_SIZE,
        cam_nz: cam_center[1] / MAP_SIZE,
    }
}

fn draw_egui_ui(ctx: &egui::Context, ui_data: &UiData) {
    // --- Game Over overlay ---
    if let Some(ref go) = ui_data.game_over {
        egui::Area::new(egui::Id::new("game_over")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(20.0).show(ui, |ui| {
                let (text, color) = match go.winner {
                    Some(0) => ("VICTORY", egui::Color32::GREEN),
                    Some(_) => ("DEFEAT", egui::Color32::RED),
                    None => ("DRAW", egui::Color32::YELLOW),
                };
                ui.label(egui::RichText::new(text).heading().strong().color(color));
                ui.label(&go.reason);
                ui.label(format!("Frame: {}", ui_data.frame_count));
                ui.label(egui::RichText::new("[R] Restart").small());
            });
        });
    }

    // --- Top bar ---
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let mf = if ui_data.metal_storage > 0.0 { ui_data.metal / ui_data.metal_storage } else { 0.0 };
            ui.label(egui::RichText::new("Metal:").strong().color(egui::Color32::from_rgb(100, 200, 100)));
            ui.add_sized([200.0, 18.0], egui::ProgressBar::new(mf)
                .text(format!("{:.0}/{:.0} +{:.1} -{:.1}", ui_data.metal, ui_data.metal_storage, ui_data.metal_income, ui_data.metal_expense))
                .fill(egui::Color32::from_rgb(60, 160, 60)));
            ui.separator();
            let ef = if ui_data.energy_storage > 0.0 { ui_data.energy / ui_data.energy_storage } else { 0.0 };
            ui.label(egui::RichText::new("Energy:").strong().color(egui::Color32::from_rgb(220, 200, 50)));
            ui.add_sized([200.0, 18.0], egui::ProgressBar::new(ef)
                .text(format!("{:.0}/{:.0} +{:.1} -{:.1}", ui_data.energy, ui_data.energy_storage, ui_data.energy_income, ui_data.energy_expense))
                .fill(egui::Color32::from_rgb(180, 160, 30)));
            ui.separator();
            ui.label(format!("B:{} R:{}", ui_data.blue_count, ui_data.red_count));
            ui.separator();
            ui.label(format!("F:{} FPS:{:.0}", ui_data.frame_count, ui_data.fps));
            if ui_data.paused { ui.label(egui::RichText::new("PAUSED").strong().color(egui::Color32::YELLOW)); }
        });
    });

    // --- Bottom bar ---
    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            if let Some(label) = &ui_data.placement_label {
                ui.label(egui::RichText::new(format!("Click to place {} | [Esc] Cancel", label)).color(egui::Color32::from_rgb(255, 200, 80)));
            } else if ui_data.selected_is_factory && !ui_data.factory_options.is_empty() {
                ui.label("Queue:");
                for (key, name, _) in &ui_data.factory_options { ui.label(format!("[{}]{}", key, name)); }
                if ui_data.factory_queue_len > 0 { ui.separator(); ui.label(format!("({} queued)", ui_data.factory_queue_len)); }
            } else if ui_data.selected_is_builder && !ui_data.builder_options.is_empty() {
                ui.label("Build:");
                for (key, name, _) in &ui_data.builder_options { ui.label(format!("[{}]{}", key, name)); }
            } else if ui_data.selected_name.is_some() {
                ui.label("[Right-click] Move | [A] Attack-move");
            } else {
                ui.label("[Left-click] Select | [Space] Pause | [R] Reset");
            }
        });
    });

    // --- Left panel: selection info + build menu grid ---
    egui::SidePanel::left("info_panel").default_width(180.0).resizable(false).show(ctx, |ui| {
        ui.heading("Selection");
        ui.separator();
        if let Some(ref name) = ui_data.selected_name {
            ui.label(egui::RichText::new(name).strong());
            if let Some((hp, max_hp)) = ui_data.selected_hp {
                let frac = if max_hp > 0.0 { hp / max_hp } else { 0.0 };
                let color = if frac > 0.5 { egui::Color32::from_rgb(60, 200, 60) }
                    else if frac > 0.25 { egui::Color32::from_rgb(220, 180, 40) }
                    else { egui::Color32::from_rgb(220, 50, 50) };
                ui.label(format!("HP: {:.0} / {:.0}", hp, max_hp));
                ui.add_sized([160.0, 14.0], egui::ProgressBar::new(frac).fill(color));
            }
            if ui_data.selected_is_factory && ui_data.factory_queue_len > 0 {
                ui.separator();
                ui.label(format!("Queue: {}", ui_data.factory_queue_len));
            }

            // Build menu grid (for builders)
            let opts = if ui_data.selected_is_builder { &ui_data.builder_options }
                else if ui_data.selected_is_factory { &ui_data.factory_options }
                else { &ui_data.builder_options };
            if !opts.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(if ui_data.selected_is_factory { "Production" } else { "Build" }).strong());
                egui::Grid::new("build_grid").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
                    for (i, (key, name, _id)) in opts.iter().enumerate() {
                        let label = format!("[{}] {}", key, name);
                        ui.label(egui::RichText::new(label).small());
                        if (i + 1) % 2 == 0 { ui.end_row(); }
                    }
                });
            }
        } else {
            ui.label("No unit selected");
        }
    });

    // --- Right panel: minimap ---
    egui::SidePanel::right("minimap_panel").default_width(160.0).resizable(false).show(ctx, |ui| {
        ui.heading("Map");
        let size = egui::vec2(150.0, 150.0);
        let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
        let rect = response.rect;

        // Background
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 30, 20));

        // Unit dots
        for dot in &ui_data.minimap_dots {
            let x = rect.min.x + dot.nx * rect.width();
            let y = rect.min.y + dot.nz * rect.height();
            let color = if dot.team == 0 { egui::Color32::from_rgb(60, 120, 255) }
                else { egui::Color32::from_rgb(255, 60, 60) };
            let radius = if dot.is_building { 3.0 } else { 2.0 };
            painter.circle_filled(egui::pos2(x, y), radius, color);
        }

        // Camera indicator
        let cx = rect.min.x + ui_data.cam_nx * rect.width();
        let cy = rect.min.y + ui_data.cam_nz * rect.height();
        painter.rect_stroke(
            egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(20.0, 15.0)),
            0.0,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    });

    // --- Health bars (floating above units) ---
    for hb in &ui_data.health_bars {
        let bar_w = 30.0;
        let bar_h = 4.0;
        let x = hb.screen_x - bar_w * 0.5;
        let y = hb.screen_y - bar_h;

        let color = if hb.hp_frac > 0.5 { egui::Color32::from_rgb(60, 200, 60) }
            else if hb.hp_frac > 0.25 { egui::Color32::from_rgb(220, 180, 40) }
            else { egui::Color32::from_rgb(220, 50, 50) };

        let outline = if hb.is_selected { egui::Color32::WHITE } else { egui::Color32::from_rgb(40, 40, 40) };

        egui::Area::new(egui::Id::new(("hb", (hb.screen_x * 100.0) as i32, (hb.screen_y * 100.0) as i32)))
            .fixed_pos(egui::pos2(x, y))
            .interactable(false)
            .show(ctx, |ui| {
                let (_, painter) = ui.allocate_painter(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                let r = painter.clip_rect();
                painter.rect_filled(r, 1.0, egui::Color32::from_rgb(30, 30, 30));
                let filled = egui::Rect::from_min_size(r.min, egui::vec2(bar_w * hb.hp_frac, bar_h));
                painter.rect_filled(filled, 1.0, color);
                painter.rect_stroke(r, 1.0, egui::Stroke::new(0.5, outline), egui::StrokeKind::Outside);

                // Build progress bar (below health bar)
                if let Some(bf) = hb.build_frac {
                    let (_, bp) = ui.allocate_painter(egui::vec2(bar_w, 2.0), egui::Sense::hover());
                    let br = bp.clip_rect();
                    bp.rect_filled(br, 0.0, egui::Color32::from_rgb(20, 20, 20));
                    let bf_rect = egui::Rect::from_min_size(br.min, egui::vec2(bar_w * bf, 2.0));
                    bp.rect_filled(bf_rect, 0.0, egui::Color32::from_rgb(80, 80, 220));
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Matrix inverse (for screen-to-ground picking)
// ---------------------------------------------------------------------------

fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let mut a = [0.0f32; 16];
    for col in 0..4 { for row in 0..4 { a[row * 4 + col] = m[col][row]; } }
    let mut inv = [0.0f32; 16];
    inv[0]  =  a[5]*a[10]*a[15] - a[5]*a[11]*a[14] - a[9]*a[6]*a[15] + a[9]*a[7]*a[14] + a[13]*a[6]*a[11] - a[13]*a[7]*a[10];
    inv[4]  = -a[4]*a[10]*a[15] + a[4]*a[11]*a[14] + a[8]*a[6]*a[15] - a[8]*a[7]*a[14] - a[12]*a[6]*a[11] + a[12]*a[7]*a[10];
    inv[8]  =  a[4]*a[9]*a[15]  - a[4]*a[11]*a[13] - a[8]*a[5]*a[15] + a[8]*a[7]*a[13] + a[12]*a[5]*a[11] - a[12]*a[7]*a[9];
    inv[12] = -a[4]*a[9]*a[14]  + a[4]*a[10]*a[13] + a[8]*a[5]*a[14] - a[8]*a[6]*a[13] - a[12]*a[5]*a[10] + a[12]*a[6]*a[9];
    inv[1]  = -a[1]*a[10]*a[15] + a[1]*a[11]*a[14] + a[9]*a[2]*a[15] - a[9]*a[3]*a[14] - a[13]*a[2]*a[11] + a[13]*a[3]*a[10];
    inv[5]  =  a[0]*a[10]*a[15] - a[0]*a[11]*a[14] - a[8]*a[2]*a[15] + a[8]*a[3]*a[14] + a[12]*a[2]*a[11] - a[12]*a[3]*a[10];
    inv[9]  = -a[0]*a[9]*a[15]  + a[0]*a[11]*a[13] + a[8]*a[1]*a[15] - a[8]*a[3]*a[13] - a[12]*a[1]*a[11] + a[12]*a[3]*a[9];
    inv[13] =  a[0]*a[9]*a[14]  - a[0]*a[10]*a[13] - a[8]*a[1]*a[14] + a[8]*a[2]*a[13] + a[12]*a[1]*a[10] - a[12]*a[2]*a[9];
    inv[2]  =  a[1]*a[6]*a[15] - a[1]*a[7]*a[14] - a[5]*a[2]*a[15] + a[5]*a[3]*a[14] + a[13]*a[2]*a[7] - a[13]*a[3]*a[6];
    inv[6]  = -a[0]*a[6]*a[15] + a[0]*a[7]*a[14] + a[4]*a[2]*a[15] - a[4]*a[3]*a[14] - a[12]*a[2]*a[7] + a[12]*a[3]*a[6];
    inv[10] =  a[0]*a[5]*a[15] - a[0]*a[7]*a[13] - a[4]*a[1]*a[15] + a[4]*a[3]*a[13] + a[12]*a[1]*a[7] - a[12]*a[3]*a[5];
    inv[14] = -a[0]*a[5]*a[14] + a[0]*a[6]*a[13] + a[4]*a[1]*a[14] - a[4]*a[2]*a[13] - a[12]*a[1]*a[6] + a[12]*a[2]*a[5];
    inv[3]  = -a[1]*a[6]*a[11] + a[1]*a[7]*a[10] + a[5]*a[2]*a[11] - a[5]*a[3]*a[10] - a[9]*a[2]*a[7] + a[9]*a[3]*a[6];
    inv[7]  =  a[0]*a[6]*a[11] - a[0]*a[7]*a[10] - a[4]*a[2]*a[11] + a[4]*a[3]*a[10] + a[8]*a[2]*a[7] - a[8]*a[3]*a[6];
    inv[11] = -a[0]*a[5]*a[11] + a[0]*a[7]*a[9]  + a[4]*a[1]*a[11] - a[4]*a[3]*a[9]  - a[8]*a[1]*a[7] + a[8]*a[3]*a[5];
    inv[15] =  a[0]*a[5]*a[10] - a[0]*a[6]*a[9]  - a[4]*a[1]*a[10] + a[4]*a[2]*a[9]  + a[8]*a[1]*a[6] - a[8]*a[2]*a[5];
    let det = a[0]*inv[0] + a[1]*inv[4] + a[2]*inv[8] + a[3]*inv[12];
    if det.abs() < 1e-10 { return None; }
    let inv_det = 1.0 / det;
    let mut result = [[0.0f32; 4]; 4];
    for col in 0..4 { for row in 0..4 { result[col][row] = inv[row * 4 + col] * inv_det; } }
    Some(result)
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    game: GameState,
    camera_ctrl: CameraController,
    particle_system: ParticleSystem,
    last_frame: Instant,
    cursor_pos: [f32; 2],
    window_size: [f32; 2],
    modifiers: ModifiersState,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    fps_counter: FpsCounter,
}

impl App {
    fn new() -> Self {
        let game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
        let (cx, cz) = game.commander_team0
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()))
            .unwrap_or((512.0, 512.0));

        Self {
            window: None,
            renderer: None,
            game,
            camera_ctrl: CameraController::new(cx, cz, 400.0),
            particle_system: ParticleSystem::new(4096),
            last_frame: Instant::now(),
            cursor_pos: [0.0; 2],
            window_size: [1280.0, 720.0],
            modifiers: ModifiersState::empty(),
            egui_state: None,
            egui_renderer: None,
            fps_counter: FpsCounter::new(),
        }
    }

    fn screen_to_ground(&self) -> Option<(f32, f32)> {
        let cam = self.camera_ctrl.camera(self.window_size[0] / self.window_size[1]);
        let vp = cam.view_projection();
        let inv_vp = mat4_inverse(vp)?;
        screen_to_ground_raw(self.cursor_pos[0], self.cursor_pos[1], self.window_size[0], self.window_size[1], &inv_vp)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("Recoil RTS")
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let mut renderer = pollster::block_on(Renderer::new(Arc::clone(&window))).expect("renderer");

        // Load S3O models for all unit types that have a model_path
        let bar_models_dir = Path::new("../Beyond-All-Reason-Sandbox/objects3d/Units");
        if bar_models_dir.exists() {
            let registry = self.game.world.resource::<UnitDefRegistry>();
            let model_entries: Vec<(u32, String)> = registry.defs.values()
                .filter_map(|def| def.model_path.as_ref().map(|p| (def.unit_type_id, p.clone())))
                .collect();

            let scale = 0.4;
            let mut loaded = 0;
            for (type_id, model_path) in &model_entries {
                // model_path from BAR is like "Units/ARMPW.s3o" — strip the "Units/" prefix
                let filename = model_path.strip_prefix("Units/").unwrap_or(model_path);
                let s3o_path = bar_models_dir.join(filename);
                if !s3o_path.exists() { continue; }
                if let Ok((mut verts, indices)) = recoil_render::load_s3o_file(&s3o_path) {
                    for v in &mut verts {
                        let (x, z) = (v.position[0], v.position[2]);
                        v.position[0] = z * scale; v.position[1] *= scale; v.position[2] = -x * scale;
                        let (nx, nz) = (v.normal[0], v.normal[2]);
                        v.normal[0] = nz; v.normal[2] = -nx;
                    }
                    renderer.register_unit_mesh(*type_id, &verts, &indices);
                    loaded += 1;
                }
            }
            // Also set the first loaded model as the placeholder (mesh_id=0)
            if let Some((_first_id, _)) = model_entries.first() {
                let filename = model_entries[0].1.strip_prefix("Units/").unwrap_or(&model_entries[0].1);
                let s3o_path = bar_models_dir.join(filename);
                if let Ok((mut verts, indices)) = recoil_render::load_s3o_file(&s3o_path) {
                    for v in &mut verts {
                        let (x, z) = (v.position[0], v.position[2]);
                        v.position[0] = z * scale; v.position[1] *= scale; v.position[2] = -x * scale;
                        let (nx, nz) = (v.normal[0], v.normal[2]);
                        v.normal[0] = nz; v.normal[2] = -nx;
                    }
                    renderer.set_unit_mesh(&verts, &indices);
                }
            }
            tracing::info!("Loaded {} S3O models for {} unit types", loaded, model_entries.len());
        }

        // egui
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(egui_ctx, egui::ViewportId::ROOT, &*window,
            Some(window.scale_factor() as f32), window.theme(),
            Some(renderer.gpu.device.limits().max_texture_dimension_2d as usize));
        let egui_renderer = egui_wgpu::Renderer::new(&renderer.gpu.device, renderer.gpu.config.format, None, 1, false);

        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // egui first
        if let (Some(es), Some(w)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            if es.on_window_event(w, &event).consumed { return; }
        }

        match event {
            WindowEvent::CloseRequested => std::process::exit(0),

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(r) = self.renderer.as_mut() { r.resize(size.width, size.height); }
                    self.window_size = [size.width as f32, size.height as f32];
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }

            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(key), state, .. }, .. } => {
                let pressed = state == ElementState::Pressed;
                self.camera_ctrl.process_key(key, pressed);

                if !pressed { return; }

                match key {
                    KeyCode::Space => { self.game.paused = !self.game.paused; }
                    KeyCode::KeyR => {
                        self.game.reset(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
                    }
                    KeyCode::Escape => {
                        if self.game.placement_mode.is_some() {
                            self.game.placement_mode = None;
                        } else {
                            event_loop.exit();
                        }
                    }
                    // Digit keys: build (for builders), queue (for factories), control groups (with Ctrl)
                    key @ (KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4
                          | KeyCode::Digit5 | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8
                          | KeyCode::Digit9 | KeyCode::Digit0) => {
                        let idx = match key {
                            KeyCode::Digit1 => 0, KeyCode::Digit2 => 1, KeyCode::Digit3 => 2,
                            KeyCode::Digit4 => 3, KeyCode::Digit5 => 4, KeyCode::Digit6 => 5,
                            KeyCode::Digit7 => 6, KeyCode::Digit8 => 7, KeyCode::Digit9 => 8,
                            KeyCode::Digit0 => 9, _ => unreachable!(),
                        };

                        if self.modifiers.control_key() {
                            // Ctrl+N: save control group
                            self.game.save_control_group(idx);
                        } else if let Some(sel) = self.game.selected() {
                            if self.game.selected_is_builder() {
                                let build_id = {
                                    let registry = self.game.world.resource::<UnitDefRegistry>();
                                    let ut = self.game.world.get::<UnitType>(sel);
                                    ut.and_then(|ut| registry.get(ut.id))
                                        .and_then(|def| def.can_build.get(idx as usize).copied())
                                };
                                if let Some(id) = build_id {
                                    self.game.handle_build_command(PlacementType(id));
                                }
                            } else if self.game.selected_is_factory() {
                                let unit_id = {
                                    let registry = self.game.world.resource::<UnitDefRegistry>();
                                    let ut = self.game.world.get::<UnitType>(sel);
                                    ut.and_then(|ut| registry.get(ut.id))
                                        .and_then(|def| def.can_build.get(idx as usize).copied())
                                };
                                if let Some(id) = unit_id {
                                    bar_game_lib::production::queue_unit(&mut self.game.world, sel, id);
                                }
                            } else {
                                // No builder/factory: recall control group
                                self.game.recall_control_group(idx);
                            }
                        } else {
                            // Nothing selected: recall control group
                            self.game.recall_control_group(idx);
                        }
                    }
                    _ => {}
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = [position.x as f32, position.y as f32];
            }

            WindowEvent::MouseInput { state: ElementState::Pressed, button, .. } => {
                if let Some((wx, wz)) = self.screen_to_ground() {
                    if self.game.placement_mode.is_some() {
                        match button {
                            MouseButton::Left => { self.game.handle_place(wx, wz); }
                            MouseButton::Right => { self.game.placement_mode = None; }
                            _ => {}
                        }
                    } else {
                        match button {
                            MouseButton::Left => {
                                if self.modifiers.shift_key() {
                                    self.game.click_select_toggle(wx, wz, 20.0);
                                } else {
                                    self.game.click_select(wx, wz, 20.0);
                                }
                            }
                            MouseButton::Right => {
                                // Move all selected units
                                let targets: Vec<Entity> = self.game.selection.selected.clone();
                                for e in targets {
                                    if let Some(ms) = self.game.world.get_mut::<recoil_sim::MoveState>(e) {
                                        *ms.into_inner() = recoil_sim::MoveState::MovingTo(
                                            recoil_math::SimVec3::new(
                                                recoil_math::SimFloat::from_f32(wx),
                                                recoil_math::SimFloat::ZERO,
                                                recoil_math::SimFloat::from_f32(wz),
                                            ),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 40.0,
                };
                self.camera_ctrl.process_scroll(scroll);
            }

            WindowEvent::RedrawRequested => {
                let dt = self.last_frame.elapsed().as_secs_f32();
                self.last_frame = Instant::now();
                self.camera_ctrl.update();

                // Tick sim
                if !self.game.paused && !self.game.is_game_over() {
                    let (impacts, deaths) = self.game.tick();
                    self.game.frame_count += 1;

                    for pos in &impacts {
                        self.particle_system.emit(*pos, 6, [1.0, 0.6, 0.2, 1.0], (5.0, 15.0), (0.2, 0.5), (1.0, 2.5));
                    }
                    for pos in &deaths {
                        self.particle_system.emit(*pos, 20, [1.0, 0.3, 0.1, 1.0], (10.0, 30.0), (0.4, 1.0), (2.0, 5.0));
                    }
                }
                self.particle_system.update(dt);

                // Gather render data
                let mut instances = unit_instances(&mut self.game);
                instances.extend(building_instances(&mut self.game));

                // Ghost preview for building placement
                if self.game.placement_mode.is_some() {
                    if let Some((gx, gz)) = self.screen_to_ground() {
                        instances.push(UnitInstance {
                            position: [gx, 0.0, gz],
                            heading: 0.0,
                            team_color: [0.3, 0.9, 0.3], // green ghost
                            mesh_id: 0,
                        });
                    }
                }

                let mut proj = projectile_instances(&mut self.game);
                proj.extend(self.particle_system.instances());

                let fps = self.fps_counter.tick();

                if let (Some(renderer), Some(egui_state), Some(egui_renderer), Some(window)) = (
                    self.renderer.as_mut(), self.egui_state.as_mut(), self.egui_renderer.as_mut(), self.window.as_ref(),
                ) {
                    let cam = self.camera_ctrl.camera(self.window_size[0] / self.window_size[1]);
                    renderer.update_camera(&cam);
                    renderer.update_units(&instances);
                    renderer.update_projectiles(&proj);

                    let render_result = renderer.render_no_present();
                    let (output, view) = match render_result {
                        Ok(v) => v,
                        Err(e) => { tracing::error!("render: {e}"); window.request_redraw(); return; }
                    };

                    // egui
                    let raw_input = egui_state.take_egui_input(window);
                    let egui_ctx = egui_state.egui_ctx().clone();
                    let vp_mat = cam.view_projection();
                    let ui_data = gather_ui_data(&mut self.game, fps, &vp_mat, self.window_size, self.camera_ctrl.center);
                    let full_output = egui_ctx.run(raw_input, |ctx| draw_egui_ui(ctx, &ui_data));
                    egui_state.handle_platform_output(window, full_output.platform_output);

                    let tris = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
                    for (id, delta) in &full_output.textures_delta.set {
                        egui_renderer.update_texture(&renderer.gpu.device, &renderer.gpu.queue, *id, delta);
                    }
                    let screen_desc = ScreenDescriptor {
                        size_in_pixels: [renderer.gpu.config.width, renderer.gpu.config.height],
                        pixels_per_point: full_output.pixels_per_point,
                    };
                    let mut encoder = renderer.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
                    let user_bufs = egui_renderer.update_buffers(&renderer.gpu.device, &renderer.gpu.queue, &mut encoder, &tris, &screen_desc);
                    {
                        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("egui_pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view, resolve_target: None,
                                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                            })],
                            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
                        });
                        let mut pass = pass.forget_lifetime();
                        egui_renderer.render(&mut pass, &tris, &screen_desc);
                    }
                    let mut bufs: Vec<wgpu::CommandBuffer> = vec![encoder.finish()];
                    bufs.extend(user_bufs);
                    renderer.gpu.queue.submit(bufs);
                    for id in &full_output.textures_delta.free { egui_renderer.free_texture(id); }
                    output.present();
                    window.request_redraw();
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("Recoil RTS starting...");

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run");
}
