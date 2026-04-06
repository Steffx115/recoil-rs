use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use eframe::egui;
use tracing_subscriber::EnvFilter;

use recoil_math::{SimFloat, SimVec2, SimVec3};
use recoil_sim::collision::collision_system;
use recoil_sim::lifecycle::{cleanup_dead, init_lifecycle, spawn_unit};
use recoil_sim::movement::movement_system;
use recoil_sim::pathfinding::TerrainGrid;
use recoil_sim::spatial::SpatialGrid;
use recoil_sim::{
    Allegiance, CollisionRadius, Heading, Health, MoveState, MovementParams, Position, SimId,
    UnitType, Velocity,
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

            // Insert additional movement components
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
            ));
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

        // 2. Movement
        movement_system(&mut self.world);

        // 3. Collision
        collision_system(&mut self.world);

        // 4. Cleanup dead
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
        Some(SelectedInfo {
            sim_id,
            pos,
            state,
            health,
        })
    }
}

struct SelectedInfo {
    sim_id: SimId,
    pos: Position,
    state: MoveState,
    health: Health,
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
                        "Health: {}/{}",
                        info.health.current.to_f32(),
                        info.health.max.to_f32()
                    ));
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
            let unit_data: Vec<(Entity, f32, f32, f32, u8, f32, MoveState)> = self
                .world
                .query::<(
                    Entity,
                    &Position,
                    &CollisionRadius,
                    &Allegiance,
                    &Heading,
                    &MoveState,
                )>()
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

                // Unit circle
                let color = if *team == 0 {
                    egui::Color32::from_rgb(80, 120, 255) // blue
                } else {
                    egui::Color32::from_rgb(255, 80, 80) // red
                };
                painter.circle_filled(center, *radius, color);

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

                // Move target line
                if let MoveState::MovingTo(target) = move_state {
                    let tx = origin.x + target.x.to_f32();
                    let tz = origin.y + target.z.to_f32();
                    let target_pos = egui::pos2(tx, tz);
                    painter.line_segment(
                        [center, target_pos],
                        egui::Stroke::new(0.5, egui::Color32::from_rgb(100, 255, 100)),
                    );
                    // Small dot at target
                    painter.circle_filled(target_pos, 3.0, egui::Color32::from_rgb(100, 255, 100));
                }
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
