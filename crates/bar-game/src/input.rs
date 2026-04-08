//! Input handling, UI data gathering, egui drawing, and instance extraction.

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;

use pierce_math::SimFloat;
use pierce_render::projectile_renderer::ProjectileInstance;
use pierce_render::unit_renderer::UnitInstance;
use pierce_sim::construction::BuildSite;
use pierce_sim::economy::EconomyState;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{Allegiance, Dead, Heading, Health, Position, UnitType, Velocity};

use bar_game_lib::GameState;

use crate::icons::IconAtlas;

// ---------------------------------------------------------------------------
// Instance extraction from GameState
// ---------------------------------------------------------------------------

pub fn unit_instances(
    game: &mut GameState,
    anim_mesh_ids: &std::collections::BTreeMap<u64, u32>,
) -> Vec<UnitInstance> {
    let sel = game.selected();
    game.world
        .query_filtered::<(
            Entity,
            &Position,
            &Heading,
            &Allegiance,
            &Health,
            &UnitType,
            Option<&BuildSite>,
        ), Without<Dead>>()
        .iter(&game.world)
        .map(|(entity, pos, heading, al, hp, ut, bs)| {
            let mut c = if al.team == 0 {
                [0.2f32, 0.5, 0.9]
            } else {
                [0.9f32, 0.2, 0.2]
            };
            if bs.is_some() {
                c[0] *= 0.5;
                c[1] *= 0.5;
                c[2] *= 0.5;
            }
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f;
            c[1] *= f;
            c[2] *= f;
            if sel == Some(entity) {
                c[0] = (c[0] + 0.3).min(1.0);
                c[1] = (c[1] + 0.3).min(1.0);
                c[2] = (c[2] + 0.3).min(1.0);
            }
            let mesh_id = anim_mesh_ids
                .get(&entity.to_bits())
                .copied()
                .unwrap_or(ut.id);
            UnitInstance {
                position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                heading: -heading.angle.to_f32(),
                team_color: c,
                alpha: 1.0,
                mesh_id,
                _pad: [0; 3],
            }
        })
        .collect()
}

pub fn building_instances(game: &mut GameState) -> Vec<UnitInstance> {
    let sel = game.selected();
    game.world
        .query_filtered::<(
            Entity,
            &Position,
            &Allegiance,
            &Health,
            &UnitType,
            Option<&BuildSite>,
        ), (Without<Dead>, Without<Heading>)>()
        .iter(&game.world)
        .map(|(entity, pos, al, hp, ut, bs)| {
            let mut c = if al.team == 0 {
                [0.1f32, 0.8, 0.3]
            } else {
                [0.8f32, 0.1, 0.3]
            };
            if bs.is_some() {
                c[0] *= 0.5;
                c[1] *= 0.5;
                c[2] *= 0.5;
            }
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f;
            c[1] *= f;
            c[2] *= f;
            if sel == Some(entity) {
                c[0] = (c[0] + 0.3).min(1.0);
                c[1] = (c[1] + 0.3).min(1.0);
                c[2] = (c[2] + 0.3).min(1.0);
            }
            UnitInstance {
                position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                heading: 0.0,
                team_color: c,
                alpha: 1.0,
                mesh_id: ut.id,
                _pad: [0; 3],
            }
        })
        .collect()
}

pub fn projectile_instances(game: &mut GameState) -> Vec<ProjectileInstance> {
    use pierce_sim::projectile::Projectile;
    game.world
        .query::<(&Position, &Velocity, &Projectile)>()
        .iter(&game.world)
        .map(|(pos, vel, _)| ProjectileInstance {
            position: [
                pos.pos.x.to_f32(),
                pos.pos.y.to_f32() + 2.0,
                pos.pos.z.to_f32(),
            ],
            size: 2.0,
            velocity_dir: [vel.vel.x.to_f32(), vel.vel.y.to_f32(), vel.vel.z.to_f32()],
            _pad: 0.0,
            color: [1.0, 0.8, 0.2],
            _pad2: 0.0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// UI data structures
// ---------------------------------------------------------------------------

/// Screen-space health bar for a unit.
pub struct HealthBarInfo {
    pub screen_x: f32,
    pub screen_y: f32,
    pub hp_frac: f32,
    pub build_frac: Option<f32>,
    pub is_selected: bool,
}

/// Minimap dot.
pub struct MinimapDot {
    /// Normalized position 0..1 on the map.
    pub nx: f32,
    pub nz: f32,
    pub team: u8,
    pub is_building: bool,
}

/// Rich build option data for the build menu grid.
pub struct BuildOption {
    pub key: String,
    pub name: String,
    /// Icon lookup key (from `icon_path` stem or lowercased name).
    pub icon_key: String,
    pub unit_type_id: u32,
    pub metal_cost: f64,
    pub energy_cost: f64,
    pub build_time: u32,
}

/// Entry in the factory production queue for display.
pub struct QueueEntry {
    pub name: String,
    /// Icon lookup key (from `icon_path` stem or lowercased name).
    pub icon_key: String,
}

pub struct UiData {
    pub metal: f32,
    pub metal_storage: f32,
    pub metal_income: f32,
    pub metal_expense: f32,
    pub energy: f32,
    pub energy_storage: f32,
    pub energy_income: f32,
    pub energy_expense: f32,
    pub frame_count: u64,
    pub fps: f32,
    pub paused: bool,
    pub blue_count: usize,
    pub red_count: usize,
    pub selected_name: Option<String>,
    pub selected_hp: Option<(f32, f32)>,
    pub selected_is_factory: bool,
    pub selected_is_builder: bool,
    pub factory_queue_len: usize,
    /// Progress of the currently building item (0..1), if any.
    pub factory_progress: f32,
    pub placement_label: Option<String>,
    pub builder_options: Vec<BuildOption>,
    pub factory_options: Vec<BuildOption>,
    /// Full factory production queue entries.
    pub factory_queue: Vec<QueueEntry>,
    pub game_over: Option<bar_game_lib::GameOver>,
    pub health_bars: Vec<HealthBarInfo>,
    pub minimap_dots: Vec<MinimapDot>,
    /// Camera position on minimap (normalized).
    pub cam_nx: f32,
    pub cam_nz: f32,
}

/// Project a world position to screen coordinates.
pub fn world_to_screen(
    vp: &[[f32; 4]; 4],
    wx: f32,
    wy: f32,
    wz: f32,
    sw: f32,
    sh: f32,
) -> Option<(f32, f32)> {
    let x = vp[0][0] * wx + vp[1][0] * wy + vp[2][0] * wz + vp[3][0];
    let y = vp[0][1] * wx + vp[1][1] * wy + vp[2][1] * wz + vp[3][1];
    let w = vp[0][3] * wx + vp[1][3] * wy + vp[2][3] * wz + vp[3][3];
    if w.abs() < 1e-6 {
        return None;
    }
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

pub const MAP_SIZE: f32 = 1024.0; // world units

pub fn gather_ui_data(
    game: &mut GameState,
    fps: f32,
    vp: &[[f32; 4]; 4],
    screen_size: [f32; 2],
    cam_center: [f32; 2],
) -> UiData {
    let (metal, metal_storage, energy, energy_storage) = {
        let eco = game.world.resource::<EconomyState>();
        eco.teams
            .get(&0)
            .map(|r| {
                (
                    r.metal.to_f32(),
                    r.metal_storage.to_f32(),
                    r.energy.to_f32(),
                    r.energy_storage.to_f32(),
                )
            })
            .unwrap_or_default()
    };
    let (metal_income, energy_income) = {
        use pierce_sim::economy::ResourceProducer;
        let (mut mi, mut ei) = (0.0f32, 0.0f32);
        for (prod, al) in game
            .world
            .query_filtered::<(&ResourceProducer, &Allegiance), Without<Dead>>()
            .iter(&game.world)
        {
            if al.team == 0 {
                mi += prod.metal_per_tick.to_f32();
                ei += prod.energy_per_tick.to_f32();
            }
        }
        (mi, ei)
    };
    let (metal_expense, energy_expense) = {
        let (mut me, mut ee) = (0.0f32, 0.0f32);
        for (bs, al) in game
            .world
            .query_filtered::<(&BuildSite, &Allegiance), Without<Dead>>()
            .iter(&game.world)
        {
            if al.team == 0 && bs.total_build_time > SimFloat::ZERO {
                me += bs.metal_cost.to_f32() / bs.total_build_time.to_f32();
                ee += bs.energy_cost.to_f32() / bs.total_build_time.to_f32();
            }
        }
        (me, ee)
    };

    let blue_count = game
        .world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&game.world)
        .filter(|a| a.team == 0)
        .count();
    let red_count = game
        .world
        .query_filtered::<&Allegiance, Without<Dead>>()
        .iter(&game.world)
        .filter(|a| a.team == 1)
        .count();

    let selected_is_factory = game.selected_is_factory();
    let selected_is_builder = game.selected_is_builder();
    let mut selected_name = None;
    let mut selected_hp = None;
    let mut factory_queue_len = 0;
    let mut factory_progress = 0.0f32;
    let mut builder_options = Vec::new();
    let mut factory_options = Vec::new();
    let mut factory_queue = Vec::new();

    if let Some(sel) = game.selected() {
        if game.world.get_entity(sel).is_ok() {
            if let Some(hp) = game.world.get::<Health>(sel) {
                selected_hp = Some((hp.current.to_f32(), hp.max.to_f32()));
            }
            if let Some(ut) = game.world.get::<UnitType>(sel) {
                let registry = game.world.resource::<UnitDefRegistry>();
                selected_name = Some(
                    registry
                        .get(ut.id)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("#{}", ut.id)),
                );

                let keys = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
                if let Some(def) = registry.get(ut.id) {
                    let list = &def.can_build;
                    let target = if selected_is_builder {
                        &mut builder_options
                    } else if selected_is_factory {
                        &mut factory_options
                    } else {
                        &mut builder_options
                    };
                    for (i, &bid) in list.iter().enumerate() {
                        if i >= keys.len() {
                            break;
                        }
                        if let Some(bd) = registry.get(bid) {
                            let icon_key = icon_key_for_def(bd);
                            target.push(BuildOption {
                                key: keys[i].to_string(),
                                name: bd.name.clone(),
                                icon_key,
                                unit_type_id: bid,
                                metal_cost: bd.metal_cost,
                                energy_cost: bd.energy_cost,
                                build_time: bd.build_time,
                            });
                        }
                    }
                }
            }
            if let Some(bq) = game.world.get::<pierce_sim::factory::BuildQueue>(sel) {
                factory_queue_len = bq.queue.len();
                factory_progress = bq.current_progress.to_f32();
                let registry = game.world.resource::<UnitDefRegistry>();
                for &uid in &bq.queue {
                    let def = registry.get(uid);
                    let name = def
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("#{}", uid));
                    let icon_key = def
                        .map(icon_key_for_def)
                        .unwrap_or_else(|| name.to_lowercase());
                    factory_queue.push(QueueEntry { name, icon_key });
                }
            }
        }
    }

    let placement_label = game.placement_mode.map(|pt| {
        let registry = game.world.resource::<UnitDefRegistry>();
        pt.label(registry)
    });

    // --- Health bars: project world positions to screen ---
    let selected_set: std::collections::HashSet<Entity> =
        game.selection.selected.iter().copied().collect();
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
    for (pos, al, heading) in game
        .world
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
        metal,
        metal_storage,
        metal_income,
        metal_expense,
        energy,
        energy_storage,
        energy_income,
        energy_expense,
        frame_count: game.frame_count,
        fps,
        paused: game.paused,
        blue_count,
        red_count,
        selected_name,
        selected_hp,
        selected_is_factory,
        selected_is_builder,
        factory_queue_len,
        factory_progress,
        placement_label,
        builder_options,
        factory_options,
        factory_queue,
        game_over: game.game_over.clone(),
        health_bars,
        minimap_dots,
        cam_nx: cam_center[0] / MAP_SIZE,
        cam_nz: cam_center[1] / MAP_SIZE,
    }
}

/// Derive the icon lookup key for a unit def.
///
/// Prefers the file stem of `icon_path` (e.g. "armpw" from "armpw.dds"),
/// falling back to the lowercased display name.
fn icon_key_for_def(def: &pierce_sim::unit_defs::UnitDef) -> String {
    def.icon_path
        .as_ref()
        .and_then(|p| std::path::Path::new(p).file_stem())
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| def.name.to_lowercase())
}

/// Actions returned by the egui UI for the game to process.
pub enum UiAction {
    /// Build/place this unit type (builder selected).
    Build(u32),
    /// Queue this unit type (factory selected).
    QueueUnit(u32),
}

pub fn draw_egui_ui(
    ctx: &egui::Context,
    ui_data: &UiData,
    icon_atlas: &IconAtlas,
) -> Vec<UiAction> {
    let mut actions = Vec::new();
    // --- Game Over overlay ---
    if let Some(ref go) = ui_data.game_over {
        egui::Area::new(egui::Id::new("game_over"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(20.0)
                    .show(ui, |ui| {
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
            let mf = if ui_data.metal_storage > 0.0 {
                ui_data.metal / ui_data.metal_storage
            } else {
                0.0
            };
            ui.label(
                egui::RichText::new("Metal:")
                    .strong()
                    .color(egui::Color32::from_rgb(100, 200, 100)),
            );
            ui.add_sized(
                [200.0, 18.0],
                egui::ProgressBar::new(mf)
                    .text(format!(
                        "{:.0}/{:.0} +{:.1} -{:.1}",
                        ui_data.metal,
                        ui_data.metal_storage,
                        ui_data.metal_income,
                        ui_data.metal_expense
                    ))
                    .fill(egui::Color32::from_rgb(60, 160, 60)),
            );
            ui.separator();
            let ef = if ui_data.energy_storage > 0.0 {
                ui_data.energy / ui_data.energy_storage
            } else {
                0.0
            };
            ui.label(
                egui::RichText::new("Energy:")
                    .strong()
                    .color(egui::Color32::from_rgb(220, 200, 50)),
            );
            ui.add_sized(
                [200.0, 18.0],
                egui::ProgressBar::new(ef)
                    .text(format!(
                        "{:.0}/{:.0} +{:.1} -{:.1}",
                        ui_data.energy,
                        ui_data.energy_storage,
                        ui_data.energy_income,
                        ui_data.energy_expense
                    ))
                    .fill(egui::Color32::from_rgb(180, 160, 30)),
            );
            ui.separator();
            ui.label(format!("B:{} R:{}", ui_data.blue_count, ui_data.red_count));
            ui.separator();
            ui.label(format!("F:{} FPS:{:.0}", ui_data.frame_count, ui_data.fps));
            if ui_data.paused {
                ui.label(
                    egui::RichText::new("PAUSED")
                        .strong()
                        .color(egui::Color32::YELLOW),
                );
            }
        });
    });

    // --- Bottom bar ---
    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            if let Some(label) = &ui_data.placement_label {
                ui.label(
                    egui::RichText::new(format!("Click to place {} | [Esc] Cancel", label))
                        .color(egui::Color32::from_rgb(255, 200, 80)),
                );
            } else if ui_data.selected_is_factory && !ui_data.factory_options.is_empty() {
                ui.label("Queue:");
                for opt in &ui_data.factory_options {
                    ui.label(format!("[{}]{}", opt.key, opt.name));
                }
                if ui_data.factory_queue_len > 0 {
                    ui.separator();
                    ui.label(format!("({} queued)", ui_data.factory_queue_len));
                }
            } else if ui_data.selected_is_builder && !ui_data.builder_options.is_empty() {
                ui.label("Build:");
                for opt in &ui_data.builder_options {
                    ui.label(format!("[{}]{}", opt.key, opt.name));
                }
            } else if ui_data.selected_name.is_some() {
                ui.label("[Right-click] Move | [A] Attack-move");
            } else {
                ui.label("[Left-click] Select | [Space] Pause | [R] Reset");
            }
        });
    });

    // --- Left panel: selection info + build menu grid ---
    egui::SidePanel::left("info_panel")
        .default_width(180.0)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Selection");
            ui.separator();
            if let Some(ref name) = ui_data.selected_name {
                ui.label(egui::RichText::new(name).strong());
                if let Some((hp, max_hp)) = ui_data.selected_hp {
                    let frac = if max_hp > 0.0 { hp / max_hp } else { 0.0 };
                    let color = if frac > 0.5 {
                        egui::Color32::from_rgb(60, 200, 60)
                    } else if frac > 0.25 {
                        egui::Color32::from_rgb(220, 180, 40)
                    } else {
                        egui::Color32::from_rgb(220, 50, 50)
                    };
                    ui.label(format!("HP: {:.0} / {:.0}", hp, max_hp));
                    ui.add_sized([160.0, 14.0], egui::ProgressBar::new(frac).fill(color));
                }
                if ui_data.selected_is_factory && ui_data.factory_queue_len > 0 {
                    ui.separator();
                    ui.label(format!("Queue: {}", ui_data.factory_queue_len));
                }

                // Build menu grid (for builders and factories)
                let opts = if ui_data.selected_is_builder {
                    &ui_data.builder_options
                } else if ui_data.selected_is_factory {
                    &ui_data.factory_options
                } else {
                    &ui_data.builder_options
                };
                if !opts.is_empty() {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(if ui_data.selected_is_factory {
                            "Production"
                        } else {
                            "Build"
                        })
                        .strong(),
                    );
                    let is_factory = ui_data.selected_is_factory;
                    egui::Grid::new("build_grid")
                        .num_columns(2)
                        .spacing([2.0, 2.0])
                        .show(ui, |ui| {
                            for (i, opt) in opts.iter().enumerate() {
                                let response = if let Some(tex) = icon_atlas.get_icon(&opt.icon_key)
                                {
                                    let img =
                                        egui::ImageButton::new(egui::load::SizedTexture::new(
                                            tex.id(),
                                            egui::vec2(64.0, 64.0),
                                        ));
                                    ui.add(img)
                                } else {
                                    let label = format!("[{}] {}", opt.key, opt.name);
                                    ui.small_button(&label)
                                };
                                // Tooltip with unit name, costs, and build time
                                let clicked = response.clicked();
                                response.on_hover_ui(|ui| {
                                    ui.label(egui::RichText::new(&opt.name).strong());
                                    ui.label(format!("Metal: {:.0}", opt.metal_cost));
                                    ui.label(format!("Energy: {:.0}", opt.energy_cost));
                                    ui.label(format!("Build time: {}", opt.build_time));
                                });
                                if clicked {
                                    if is_factory {
                                        actions.push(UiAction::QueueUnit(opt.unit_type_id));
                                    } else {
                                        actions.push(UiAction::Build(opt.unit_type_id));
                                    }
                                }
                                if (i + 1) % 2 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                }

                // Factory production queue display (horizontal row of icons)
                if ui_data.selected_is_factory && !ui_data.factory_queue.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("Queue").strong());
                    ui.horizontal_wrapped(|ui| {
                        for (i, entry) in ui_data.factory_queue.iter().enumerate() {
                            let is_current = i == 0;
                            if let Some(tex) = icon_atlas.get_icon(&entry.icon_key) {
                                let size = egui::vec2(32.0, 32.0);
                                let (rect, _response) =
                                    ui.allocate_exact_size(size, egui::Sense::hover());
                                let img =
                                    egui::Image::new(egui::load::SizedTexture::new(tex.id(), size));
                                img.paint_at(ui, rect);
                                // Progress bar overlay on the currently-building item
                                if is_current && ui_data.factory_progress > 0.0 {
                                    let progress = ui_data.factory_progress.clamp(0.0, 1.0);
                                    let bar_h = 3.0;
                                    let bg_rect = egui::Rect::from_min_size(
                                        egui::pos2(rect.min.x, rect.max.y - bar_h),
                                        egui::vec2(rect.width(), bar_h),
                                    );
                                    ui.painter().rect_filled(
                                        bg_rect,
                                        0.0,
                                        egui::Color32::from_rgb(20, 20, 20),
                                    );
                                    let bar_rect = egui::Rect::from_min_size(
                                        egui::pos2(rect.min.x, rect.max.y - bar_h),
                                        egui::vec2(rect.width() * progress, bar_h),
                                    );
                                    ui.painter().rect_filled(
                                        bar_rect,
                                        0.0,
                                        egui::Color32::from_rgb(80, 200, 80),
                                    );
                                }
                            } else {
                                // Text fallback for queue items
                                let label = if is_current {
                                    format!("[{}]", entry.name)
                                } else {
                                    entry.name.clone()
                                };
                                let _ = ui.small_button(&label);
                            }
                        }
                    });
                }
            } else {
                ui.label("No unit selected");
            }
        });

    // --- Right panel: minimap ---
    egui::SidePanel::right("minimap_panel")
        .default_width(160.0)
        .resizable(false)
        .show(ctx, |ui| {
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
                let color = if dot.team == 0 {
                    egui::Color32::from_rgb(60, 120, 255)
                } else {
                    egui::Color32::from_rgb(255, 60, 60)
                };
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

        let color = if hb.hp_frac > 0.5 {
            egui::Color32::from_rgb(60, 200, 60)
        } else if hb.hp_frac > 0.25 {
            egui::Color32::from_rgb(220, 180, 40)
        } else {
            egui::Color32::from_rgb(220, 50, 50)
        };

        let outline = if hb.is_selected {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(40, 40, 40)
        };

        egui::Area::new(egui::Id::new((
            "hb",
            (hb.screen_x * 100.0) as i32,
            (hb.screen_y * 100.0) as i32,
        )))
        .fixed_pos(egui::pos2(x, y))
        .interactable(false)
        .show(ctx, |ui| {
            let (_, painter) = ui.allocate_painter(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            let r = painter.clip_rect();
            painter.rect_filled(r, 1.0, egui::Color32::from_rgb(30, 30, 30));
            let filled = egui::Rect::from_min_size(r.min, egui::vec2(bar_w * hb.hp_frac, bar_h));
            painter.rect_filled(filled, 1.0, color);
            painter.rect_stroke(
                r,
                1.0,
                egui::Stroke::new(0.5, outline),
                egui::StrokeKind::Outside,
            );

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

    actions
}

// ---------------------------------------------------------------------------
// Matrix inverse (for screen-to-ground picking)
// ---------------------------------------------------------------------------

pub fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let mut a = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            a[row * 4 + col] = m[col][row];
        }
    }
    let mut inv = [0.0f32; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15]
        + a[9] * a[7] * a[14]
        + a[13] * a[6] * a[11]
        - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15]
        - a[8] * a[7] * a[14]
        - a[12] * a[6] * a[11]
        + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15]
        + a[8] * a[7] * a[13]
        + a[12] * a[5] * a[11]
        - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14]
        - a[8] * a[6] * a[13]
        - a[12] * a[5] * a[10]
        + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15]
        - a[9] * a[3] * a[14]
        - a[13] * a[2] * a[11]
        + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15]
        + a[8] * a[3] * a[14]
        + a[12] * a[2] * a[11]
        - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15]
        - a[8] * a[3] * a[13]
        - a[12] * a[1] * a[11]
        + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14]
        + a[8] * a[2] * a[13]
        + a[12] * a[1] * a[10]
        - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15]
        + a[5] * a[3] * a[14]
        + a[13] * a[2] * a[7]
        - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15]
        - a[4] * a[3] * a[14]
        - a[12] * a[2] * a[7]
        + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15]
        + a[4] * a[3] * a[13]
        + a[12] * a[1] * a[7]
        - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14]
        - a[4] * a[2] * a[13]
        - a[12] * a[1] * a[6]
        + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11]
        - a[5] * a[3] * a[10]
        - a[9] * a[2] * a[7]
        + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11]
        + a[4] * a[3] * a[10]
        + a[8] * a[2] * a[7]
        - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11]
        - a[4] * a[3] * a[9]
        - a[8] * a[1] * a[7]
        + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10]
        + a[4] * a[2] * a[9]
        + a[8] * a[1] * a[6]
        - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det.abs() < 1e-10 {
        return None;
    }
    let inv_det = 1.0 / det;
    let mut result = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            result[col][row] = inv[row * 4 + col] * inv_det;
        }
    }
    Some(result)
}

// ---------------------------------------------------------------------------
// Keyboard/mouse input dispatch
// ---------------------------------------------------------------------------

/// Handle a keyboard press event. Returns `true` if the event loop should exit.
pub fn handle_key_press(
    game: &mut GameState,
    key: winit::keyboard::KeyCode,
    modifiers: winit::keyboard::ModifiersState,
    bar_units_path: &std::path::Path,
    map_manifest_path: &std::path::Path,
) -> bool {
    use bar_game_lib::building::PlacementType;
    use winit::keyboard::KeyCode;

    match key {
        KeyCode::Space => {
            game.paused = !game.paused;
        }
        KeyCode::KeyR => {
            game.reset(bar_units_path, map_manifest_path);
        }
        KeyCode::Escape => {
            if game.placement_mode.is_some() {
                game.placement_mode = None;
            } else {
                return true; // signal exit
            }
        }
        // Digit keys: build (for builders), queue (for factories), control groups (with Ctrl)
        key @ (KeyCode::Digit1
        | KeyCode::Digit2
        | KeyCode::Digit3
        | KeyCode::Digit4
        | KeyCode::Digit5
        | KeyCode::Digit6
        | KeyCode::Digit7
        | KeyCode::Digit8
        | KeyCode::Digit9
        | KeyCode::Digit0) => {
            let idx = match key {
                KeyCode::Digit1 => 0,
                KeyCode::Digit2 => 1,
                KeyCode::Digit3 => 2,
                KeyCode::Digit4 => 3,
                KeyCode::Digit5 => 4,
                KeyCode::Digit6 => 5,
                KeyCode::Digit7 => 6,
                KeyCode::Digit8 => 7,
                KeyCode::Digit9 => 8,
                KeyCode::Digit0 => 9,
                _ => unreachable!(),
            };

            if modifiers.control_key() {
                // Ctrl+N: save control group
                game.save_control_group(idx);
            } else if let Some(sel) = game.selected() {
                if game.selected_is_builder() {
                    let build_id = {
                        let registry = game.world.resource::<UnitDefRegistry>();
                        let ut = game.world.get::<UnitType>(sel);
                        ut.and_then(|ut| registry.get(ut.id))
                            .and_then(|def| def.can_build.get(idx as usize).copied())
                    };
                    if let Some(id) = build_id {
                        game.handle_build_command(PlacementType(id));
                    }
                } else if game.selected_is_factory() {
                    let unit_id = {
                        let registry = game.world.resource::<UnitDefRegistry>();
                        let ut = game.world.get::<UnitType>(sel);
                        ut.and_then(|ut| registry.get(ut.id))
                            .and_then(|def| def.can_build.get(idx as usize).copied())
                    };
                    if let Some(id) = unit_id {
                        bar_game_lib::production::queue_unit(&mut game.world, sel, id);
                    }
                } else {
                    // No builder/factory: recall control group
                    game.recall_control_group(idx);
                }
            } else {
                // Nothing selected: recall control group
                game.recall_control_group(idx);
            }
        }
        _ => {}
    }
    false
}

/// Handle a mouse button press event.
pub fn handle_mouse_press(
    game: &mut GameState,
    button: winit::event::MouseButton,
    ground_pos: Option<(f32, f32)>,
    shift_held: bool,
) {
    use winit::event::MouseButton;

    let Some((wx, wz)) = ground_pos else {
        return;
    };

    if game.placement_mode.is_some() {
        match button {
            MouseButton::Left => {
                game.handle_place(wx, wz);
            }
            MouseButton::Right => {
                game.placement_mode = None;
            }
            _ => {}
        }
    } else {
        match button {
            MouseButton::Left => {
                if shift_held {
                    game.click_select_toggle(wx, wz, 20.0);
                } else {
                    game.click_select(wx, wz, 20.0);
                }
            }
            MouseButton::Right => {
                // Move all selected units
                let targets: Vec<bevy_ecs::entity::Entity> = game.selection.selected.clone();
                if targets.is_empty() {
                    tracing::debug!("Right-click: no units selected");
                }
                for e in targets {
                    let has_ms = game.world.get::<pierce_sim::MoveState>(e).is_some();
                    if !has_ms {
                        tracing::warn!("Selected entity {:?} has no MoveState — cannot move", e);
                    }
                    if let Some(ms) = game.world.get_mut::<pierce_sim::MoveState>(e) {
                        *ms.into_inner() =
                            pierce_sim::MoveState::MovingTo(pierce_math::SimVec3::new(
                                pierce_math::SimFloat::from_f32(wx),
                                pierce_math::SimFloat::ZERO,
                                pierce_math::SimFloat::from_f32(wz),
                            ));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Process UI actions from the egui overlay.
pub fn process_ui_actions(game: &mut GameState, actions: Vec<UiAction>) {
    use bar_game_lib::building::PlacementType;

    for action in actions {
        match action {
            UiAction::Build(id) => {
                game.handle_build_command(PlacementType(id));
            }
            UiAction::QueueUnit(id) => {
                if let Some(sel) = game.selected() {
                    bar_game_lib::production::queue_unit(&mut game.world, sel, id);
                }
            }
        }
    }
}
