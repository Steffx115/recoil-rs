use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use eframe::egui;
use tracing_subscriber::EnvFilter;

use recoil_math::{SimFloat, SimVec2, SimVec3};
use recoil_sim::collision::collision_system;
use recoil_sim::combat_data::WeaponSet;
use recoil_sim::combat_data::{ArmorClass, DamageTable, WeaponDef, WeaponInstance};
use recoil_sim::commands::{command_system, CommandQueue};
use recoil_sim::components::Stunned;
use recoil_sim::damage::{damage_system, stun_system};
use recoil_sim::economy::{economy_system, init_economy, ResourceProducer};
use recoil_sim::lifecycle::{cleanup_dead, init_lifecycle, spawn_unit};
use recoil_sim::movement::movement_system;
use recoil_sim::pathfinding::TerrainGrid;
use recoil_sim::projectile::{
    projectile_movement_system, spawn_projectile_system, ImpactEventQueue,
};
use recoil_sim::spatial::SpatialGrid;
use recoil_sim::targeting::{reload_system, targeting_system, FireEventQueue, WeaponRegistry};
use recoil_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SimId,
    Target, UnitType, Velocity,
};

// ---------------------------------------------------------------------------
// Seeded LCG (no rand crate)
// ---------------------------------------------------------------------------

struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        // LCG constants from Numerical Recipes
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.state >> 33) as u32
    }

    /// Random f32 in [0, max).
    fn next_f32(&mut self, max: f32) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * max
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NUM_UNITS: usize = 18;
const WORLD_SIZE: f32 = 600.0;
const GRID_CELL_SIZE: i32 = 10;
const GRID_DIM: i32 = 64;
const SELECT_RADIUS: f32 = 20.0;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct RecoilDebugApp {
    world: World,
    paused: bool,
    frame_count: u64,
    selected: Option<Entity>,
    sim_speed: i32,
    rng_seed: u64,
}

impl RecoilDebugApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            world: World::new(),
            paused: false,
            frame_count: 0,
            selected: None,
            sim_speed: 1,
            rng_seed: 12345,
        };
        app.reset();
        app
    }

    fn reset(&mut self) {
        self.world = World::new();
        self.selected = None;
        self.frame_count = 0;

        // Init lifecycle (SimIdCounter)
        init_lifecycle(&mut self.world);

        // Insert SpatialGrid resource
        let grid = SpatialGrid::new(SimFloat::from_int(GRID_CELL_SIZE), GRID_DIM, GRID_DIM);
        self.world.insert_resource(grid);

        // Insert TerrainGrid (64x64, all cost 1.0)
        let terrain = TerrainGrid::new(64, 64, SimFloat::ONE);
        self.world.insert_resource(terrain);

        // Insert DamageTable
        self.world.insert_resource(DamageTable::default());

        // Insert weapon registry with one weapon type
        let mut registry = WeaponRegistry { defs: Vec::new() };
        registry.defs.push(WeaponDef {
            damage: SimFloat::from_int(10),
            damage_type: recoil_sim::combat_data::DamageType::Normal,
            range: SimFloat::from_int(120),
            reload_time: 30,
            projectile_speed: SimFloat::from_int(8),
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
        });
        self.world.insert_resource(registry);

        // Insert event queues
        self.world
            .insert_resource(FireEventQueue { events: Vec::new() });
        self.world
            .insert_resource(ImpactEventQueue { events: Vec::new() });

        // Init economy (two teams)
        init_economy(&mut self.world, &[0, 1]);

        // Spawn units
        let mut rng = Lcg::new(self.rng_seed);
        for i in 0..NUM_UNITS {
            let x = rng.next_f32(WORLD_SIZE);
            let z = rng.next_f32(WORLD_SIZE);

            let entity = spawn_unit(
                &mut self.world,
                Position {
                    pos: SimVec3::new(SimFloat::from_f32(x), SimFloat::ZERO, SimFloat::from_f32(z)),
                },
                UnitType { id: 1 },
                Allegiance {
                    team: (i % 2) as u8,
                },
                Health {
                    current: SimFloat::from_int(100),
                    max: SimFloat::from_int(100),
                },
            );

            // Insert movement, combat, and command components
            self.world.entity_mut(entity).insert((
                MoveState::Idle,
                MovementParams {
                    max_speed: SimFloat::from_int(2),
                    acceleration: SimFloat::ONE,
                    turn_rate: SimFloat::PI / SimFloat::from_int(30),
                },
                CollisionRadius {
                    radius: SimFloat::from_int(8),
                },
                Heading {
                    angle: SimFloat::ZERO,
                },
                Velocity { vel: SimVec3::ZERO },
                ArmorClass::Light,
                Target { entity: None },
                WeaponSet {
                    weapons: vec![WeaponInstance {
                        def_id: 0,
                        reload_remaining: 0,
                    }],
                },
                CommandQueue::default(),
            ));

            // Give each team a metal producer to keep economy alive
            if i < 2 {
                self.world.entity_mut(entity).insert(ResourceProducer {
                    metal_per_tick: SimFloat::from_int(1),
                    energy_per_tick: SimFloat::from_int(2),
                });
            }
        }

        // Bump seed for next reset
        self.rng_seed = self.rng_seed.wrapping_add(7);
    }

    fn sim_tick(&mut self) {
        // 1. Rebuild SpatialGrid
        {
            let entities: Vec<(Entity, SimVec3)> = self
                .world
                .query::<(Entity, &Position)>()
                .iter(&self.world)
                .map(|(e, p)| (e, p.pos))
                .collect();

            let mut grid = self.world.resource_mut::<SpatialGrid>();
            grid.clear();
            for (e, pos) in entities {
                grid.insert(e, SimVec2::new(pos.x, pos.z));
            }
        }

        // 2. Clear fire event queue for this tick
        self.world.resource_mut::<FireEventQueue>().events.clear();

        // 3. Command processing → sets MoveState from CommandQueue
        command_system(&mut self.world);

        // 4. Economy
        economy_system(&mut self.world);

        // 5. Movement
        movement_system(&mut self.world);

        // 6. Collision
        collision_system(&mut self.world);

        // 7. Targeting → finds enemies, sets Target
        targeting_system(&mut self.world);

        // 8. Reload + fire → produces FireEvents
        reload_system(&mut self.world);

        // 9. Spawn projectiles from fire events (or instant beam impacts)
        spawn_projectile_system(&mut self.world);

        // 10. Projectile movement + impact detection
        projectile_movement_system(&mut self.world);

        // 11. Damage processing → apply impacts, kill units, spawn wreckage
        damage_system(&mut self.world);

        // 12. Stun tick-down
        stun_system(&mut self.world);

        // 13. Cleanup dead entities
        cleanup_dead(&mut self.world);
    }

    fn unit_count(&mut self) -> usize {
        self.world
            .query_filtered::<Entity, With<Position>>()
            .iter(&self.world)
            .count()
    }

    fn selected_info(&self) -> Option<SelectedInfo> {
        let entity = self.selected?;
        let sim_id = self.world.get::<SimId>(entity)?.clone();
        let pos = self.world.get::<Position>(entity)?.clone();
        let state = self.world.get::<MoveState>(entity)?.clone();
        let health = self.world.get::<Health>(entity)?.clone();
        let target = self.world.get::<Target>(entity).and_then(|t| t.entity);
        let stunned = self.world.get::<Stunned>(entity).is_some();
        Some(SelectedInfo {
            sim_id,
            pos,
            state,
            health,
            target,
            stunned,
        })
    }
}

struct SelectedInfo {
    sim_id: SimId,
    pos: Position,
    state: MoveState,
    health: Health,
    target: Option<Entity>,
    stunned: bool,
}

impl eframe::App for RecoilDebugApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Run sim ticks
        if !self.paused {
            for _ in 0..self.sim_speed {
                self.sim_tick();
            }
            self.frame_count += 1;
        }

        // Continuously repaint for animation
        ctx.request_repaint();

        // ---- Side panel ----
        let unit_count = self.unit_count();
        let sel_info = self.selected_info();

        egui::SidePanel::left("info_panel")
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Recoil Debug View");
                ui.separator();

                ui.label(format!("Frame: {}", self.frame_count));
                ui.label(format!("Units alive: {}", unit_count));
                ui.separator();

                // Selected unit info
                if let Some(info) = &sel_info {
                    ui.label(format!("SimId: {}", info.sim_id.id));
                    ui.label(format!(
                        "Pos: ({:.1}, {:.1})",
                        info.pos.pos.x.to_f32(),
                        info.pos.pos.z.to_f32()
                    ));
                    ui.label(format!("State: {:?}", info.state));
                    ui.label(format!(
                        "HP: {:.0}/{:.0}",
                        info.health.current.to_f32(),
                        info.health.max.to_f32()
                    ));
                    if let Some(_target) = info.target {
                        ui.label("Target: engaged");
                    }
                    if info.stunned {
                        ui.colored_label(egui::Color32::from_rgb(200, 100, 255), "STUNNED");
                    }
                } else {
                    ui.label("No unit selected");
                }

                ui.separator();

                // Pause / Resume
                if ui
                    .button(if self.paused { "Resume" } else { "Pause" })
                    .clicked()
                {
                    self.paused = !self.paused;
                }

                // Reset
                if ui.button("Reset").clicked() {
                    self.reset();
                }

                // Sim speed slider
                ui.add(egui::Slider::new(&mut self.sim_speed, 1..=10).text("Sim speed"));
            });

        // ---- Central panel (game view) ----
        egui::CentralPanel::default().show(ctx, |ui| {
            // Handle keyboard input
            let space_pressed = ui.input(|i| i.key_pressed(egui::Key::Space));
            let r_pressed = ui.input(|i| i.key_pressed(egui::Key::R));

            if space_pressed {
                self.paused = !self.paused;
            }
            if r_pressed {
                self.reset();
                return;
            }

            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click());

            let rect = response.rect;

            // Background
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 40));

            // Offset so world (0,0) maps to top-left of the rect
            let origin = rect.min;

            // Handle mouse clicks
            if response.clicked_by(egui::PointerButton::Primary) {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    let wx = pointer_pos.x - origin.x;
                    let wz = pointer_pos.y - origin.y;
                    self.selected = self.find_nearest_unit(wx, wz);
                }
            }

            if response.clicked_by(egui::PointerButton::Secondary) {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    let wx = pointer_pos.x - origin.x;
                    let wz = pointer_pos.y - origin.y;
                    if let Some(sel) = self.selected {
                        if self.world.get::<MoveState>(sel).is_some() {
                            let target = SimVec3::new(
                                SimFloat::from_f32(wx),
                                SimFloat::ZERO,
                                SimFloat::from_f32(wz),
                            );
                            *self.world.get_mut::<MoveState>(sel).unwrap() =
                                MoveState::MovingTo(target);
                        }
                    }
                }
            }

            // Draw units
            // Draw wreckage first (behind units)
            use recoil_sim::construction::Reclaimable;
            let wreck_data: Vec<(f32, f32)> = self
                .world
                .query::<(&Position, &Reclaimable)>()
                .iter(&self.world)
                .map(|(pos, _)| (pos.pos.x.to_f32(), pos.pos.z.to_f32()))
                .collect();
            for (wx, wz) in &wreck_data {
                let wp = egui::pos2(origin.x + wx, origin.y + wz);
                painter.circle_filled(wp, 5.0, egui::Color32::from_rgb(100, 80, 40));
                painter.circle_stroke(
                    wp,
                    5.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 50, 30)),
                );
            }

            let unit_data: Vec<(Entity, f32, f32, f32, u8, f32, MoveState)> = self
                .world
                .query_filtered::<(
                    Entity,
                    &Position,
                    &CollisionRadius,
                    &Allegiance,
                    &Heading,
                    &MoveState,
                ), bevy_ecs::query::Without<recoil_sim::Dead>>()
                .iter(&self.world)
                .map(|(e, pos, cr, al, h, ms)| {
                    (
                        e,
                        pos.pos.x.to_f32(),
                        pos.pos.z.to_f32(),
                        cr.radius.to_f32(),
                        al.team,
                        h.angle.to_f32(),
                        ms.clone(),
                    )
                })
                .collect();

            for (entity, x, z, radius, team, heading, move_state) in &unit_data {
                let center = egui::pos2(origin.x + x, origin.y + z);

                // Unit circle — dim if dead
                let hp = self
                    .world
                    .get::<Health>(*entity)
                    .map(|h| h.current.to_f32() / h.max.to_f32().max(1.0))
                    .unwrap_or(1.0);
                let stunned = self.world.get::<Stunned>(*entity).is_some();

                let base_color = if *team == 0 {
                    egui::Color32::from_rgb(80, 120, 255) // blue
                } else {
                    egui::Color32::from_rgb(255, 80, 80) // red
                };
                let color = if stunned {
                    egui::Color32::from_rgb(200, 100, 255) // purple when stunned
                } else {
                    base_color
                };
                painter.circle_filled(center, *radius, color);

                // Health bar above unit
                if hp < 1.0 {
                    let bar_w = radius * 2.0;
                    let bar_h = 3.0;
                    let bar_y = center.y - radius - 6.0;
                    let bar_x = center.x - radius;
                    // Background
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(bar_x, bar_y),
                            egui::vec2(bar_w, bar_h),
                        ),
                        0.0,
                        egui::Color32::from_rgb(60, 0, 0),
                    );
                    // Fill
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(bar_x, bar_y),
                            egui::vec2(bar_w * hp.max(0.0), bar_h),
                        ),
                        0.0,
                        egui::Color32::from_rgb(0, 200, 0),
                    );
                }

                // Selected highlight
                if self.selected == Some(*entity) {
                    painter.circle_stroke(
                        center,
                        radius + 3.0,
                        egui::Stroke::new(2.0, egui::Color32::YELLOW),
                    );
                }

                // Heading line
                let head_len = radius + 5.0;
                let hx = heading.cos();
                let hz = heading.sin();
                let head_end = egui::pos2(center.x + hx * head_len, center.y + hz * head_len);
                painter.line_segment(
                    [center, head_end],
                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                );

                // Target line (red to enemy being attacked)
                if let Some(target) = self.world.get::<Target>(*entity) {
                    if let Some(target_entity) = target.entity {
                        if let Some(tpos) = self.world.get::<Position>(target_entity) {
                            let tp = egui::pos2(
                                origin.x + tpos.pos.x.to_f32(),
                                origin.y + tpos.pos.z.to_f32(),
                            );
                            painter.line_segment(
                                [center, tp],
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 50, 50)),
                            );
                        }
                    }
                }

                // Move target line
                if let MoveState::MovingTo(target) = move_state {
                    let tx = origin.x + target.x.to_f32();
                    let tz = origin.y + target.z.to_f32();
                    let target_pos = egui::pos2(tx, tz);
                    painter.line_segment(
                        [center, target_pos],
                        egui::Stroke::new(0.5, egui::Color32::from_rgb(100, 255, 100)),
                    );
                    painter.circle_filled(target_pos, 3.0, egui::Color32::from_rgb(100, 255, 100));
                }
            }

            // Draw projectiles
            let projectile_data: Vec<(f32, f32)> = self
                .world
                .query::<(&Position, &recoil_sim::projectile::Projectile)>()
                .iter(&self.world)
                .map(|(pos, _)| (pos.pos.x.to_f32(), pos.pos.z.to_f32()))
                .collect();
            for (px, pz) in &projectile_data {
                let pp = egui::pos2(origin.x + px, origin.y + pz);
                painter.circle_filled(pp, 2.0, egui::Color32::from_rgb(255, 255, 50));
            }
        });
    }
}

impl RecoilDebugApp {
    fn find_nearest_unit(&mut self, wx: f32, wz: f32) -> Option<Entity> {
        let mut best: Option<(Entity, f32)> = None;

        for (entity, pos) in self.world.query::<(Entity, &Position)>().iter(&self.world) {
            let dx = pos.pos.x.to_f32() - wx;
            let dz = pos.pos.z.to_f32() - wz;
            let dist = (dx * dx + dz * dz).sqrt();
            if dist <= SELECT_RADIUS && (best.is_none() || dist < best.unwrap().1) {
                best = Some((entity, dist));
            }
        }

        best.map(|(e, _)| e)
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Recoil Debug Visualizer starting...");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Recoil Debug View",
        options,
        Box::new(|cc| Ok(Box::new(RecoilDebugApp::new(cc)))),
    )
    .expect("Failed to run eframe");
}
