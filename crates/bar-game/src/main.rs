use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use tracing_subscriber::EnvFilter;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use recoil_math::{SimFloat, SimVec3};
use recoil_render::camera::Camera;
use recoil_render::particles::ParticleSystem;
use recoil_render::projectile_renderer::ProjectileInstance;
use recoil_render::unit_renderer::UnitInstance;
use recoil_render::Renderer;
use recoil_sim::combat_data::{WeaponInstance, WeaponSet};
use recoil_sim::commands::CommandQueue;
use recoil_sim::components::Stunned;
use recoil_sim::construction::Reclaimable;
use recoil_sim::economy::{init_economy, EconomyState, ResourceProducer};
use recoil_sim::fog::FogOfWar;
use recoil_sim::lifecycle::spawn_unit;
use recoil_sim::map::load_map_manifest;
use recoil_sim::projectile::{ImpactEventQueue, Projectile};
use recoil_sim::selection::screen_to_ground_raw;
use recoil_sim::sim_runner;
use recoil_sim::targeting::WeaponRegistry;
use recoil_sim::unit_defs::UnitDefRegistry;
use recoil_sim::{
    Allegiance, CollisionRadius, Dead, Heading, Health, MoveState, MovementParams, Position,
    SightRange, Target, UnitType, Velocity,
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
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.state >> 33) as u32
    }

    fn next_f32(&mut self, max: f32) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * max
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NUM_UNITS: usize = 18;
const SELECT_RADIUS_SQ: f32 = 400.0; // 20^2

// Camera movement
const PAN_SPEED: f32 = 5.0;
const ZOOM_SPEED: f32 = 10.0;
const MIN_HEIGHT: f32 = 50.0;
const MAX_HEIGHT: f32 = 800.0;

// ---------------------------------------------------------------------------
// Camera controller
// ---------------------------------------------------------------------------

struct CameraController {
    /// Camera center position on the ground plane (x, z).
    center: [f32; 2],
    /// Height above ground.
    height: f32,
    /// Key states: W, A, S, D
    forward: bool,
    left: bool,
    backward: bool,
    right: bool,
}

impl CameraController {
    fn new(cx: f32, cz: f32, height: f32) -> Self {
        Self {
            center: [cx, cz],
            height,
            forward: false,
            left: false,
            backward: false,
            right: false,
        }
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
        let speed = PAN_SPEED * (self.height / 400.0); // pan faster when zoomed out
        if self.forward {
            self.center[1] -= speed;
        }
        if self.backward {
            self.center[1] += speed;
        }
        if self.left {
            self.center[0] -= speed;
        }
        if self.right {
            self.center[0] += speed;
        }
    }

    fn camera(&self, aspect: f32) -> Camera {
        Camera {
            eye: [
                self.center[0],
                self.height,
                self.center[1] + self.height * 0.75,
            ],
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
// Simulation state (extracted from the old 2D version)
// ---------------------------------------------------------------------------

struct SimState {
    world: World,
    paused: bool,
    frame_count: u64,
    selected: Option<Entity>,
    rng_seed: u64,
}

impl SimState {
    fn new() -> Self {
        let mut state = Self {
            world: World::new(),
            paused: false,
            frame_count: 0,
            selected: None,
            rng_seed: 12345,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.world = World::new();
        self.selected = None;
        self.frame_count = 0;

        // Use sim_runner to initialize all resources (lifecycle, spatial grid,
        // terrain, weapon registry, damage table, fire/impact queues, economy).
        sim_runner::init_sim_world(&mut self.world);

        // Load unit defs from both factions
        let mut unit_def_registry =
            UnitDefRegistry::load_directory(Path::new("assets/unitdefs/armada"))
                .unwrap_or_default();
        let armada_count = unit_def_registry.defs.len();
        if let Ok(cortex) = UnitDefRegistry::load_directory(Path::new("assets/unitdefs/cortex")) {
            for (id, def) in cortex.defs {
                unit_def_registry.register(def);
            }
        }
        tracing::info!(
            "Loaded {} unit defs ({} Armada, {} Cortex)",
            unit_def_registry.defs.len(),
            armada_count,
            unit_def_registry.defs.len() - armada_count,
        );

        // Register all weapon defs from loaded unit defs into the WeaponRegistry.
        // Build a map from (unit_type_id, weapon_index) -> weapon_def_id for spawning.
        let mut weapon_def_ids: std::collections::BTreeMap<u32, Vec<u32>> =
            std::collections::BTreeMap::new();
        {
            let mut registry = self.world.resource_mut::<WeaponRegistry>();
            for (unit_type_id, unit_def) in &unit_def_registry.defs {
                let mut ids = Vec::new();
                for weapon_def in unit_def.to_weapon_defs() {
                    let id = registry.defs.len() as u32;
                    registry.defs.push(weapon_def);
                    ids.push(id);
                }
                weapon_def_ids.insert(*unit_type_id, ids);
            }
        }

        // Load map manifest for start positions
        let map_manifest = load_map_manifest(Path::new("assets/maps/small_duel/manifest.ron")).ok();
        if let Some(ref manifest) = map_manifest {
            tracing::info!(
                "Loaded map '{}' with {} start positions",
                manifest.name,
                manifest.start_positions.len()
            );
        }

        init_economy(&mut self.world, &[0, 1]);

        // Fog of War — grid covers 64x64 cells for the two teams
        let fog = FogOfWar::new(64, 64, &[0, 1]);
        self.world.insert_resource(fog);

        // Helper to extract stats from a unit def
        fn extract_stats(
            def: &recoil_sim::unit_defs::UnitDef,
        ) -> (
            SimFloat,
            SimFloat,
            SimFloat,
            SimFloat,
            SimFloat,
            SimFloat,
            recoil_sim::combat_data::ArmorClass,
            u32,
        ) {
            (
                SimFloat::from_f64(def.max_health),
                SimFloat::from_f64(def.max_speed),
                SimFloat::from_f64(def.acceleration),
                SimFloat::from_f64(def.turn_rate),
                SimFloat::from_f64(def.collision_radius),
                SimFloat::from_f64(def.sight_range),
                def.parse_armor_class(),
                def.unit_type_id,
            )
        }

        let fallback = (
            SimFloat::from_int(500),
            SimFloat::from_int(2),
            SimFloat::ONE,
            SimFloat::PI / SimFloat::from_int(30),
            SimFloat::from_int(8),
            SimFloat::from_int(80),
            recoil_sim::combat_data::ArmorClass::Light,
            1u32,
        );

        // Team 0 = Armada Peewee (id 100), Team 1 = Cortex AK (id 150)
        let stats_team0 = unit_def_registry
            .get(100)
            .map(extract_stats)
            .unwrap_or(fallback);
        let stats_team1 = unit_def_registry
            .get(150)
            .map(extract_stats)
            .unwrap_or(fallback);
        let weapons_team0 = weapon_def_ids.get(&100).cloned().unwrap_or_default();
        let weapons_team1 = weapon_def_ids.get(&150).cloned().unwrap_or_default();

        // Determine spawn positions from map start positions or fallback to random
        let start_pos_0: (f32, f32);
        let start_pos_1: (f32, f32);
        if let Some(ref manifest) = map_manifest {
            let sp0 = manifest.start_positions.iter().find(|sp| sp.team == 0);
            let sp1 = manifest.start_positions.iter().find(|sp| sp.team == 1);
            start_pos_0 = sp0
                .map(|sp| (sp.x as f32, sp.z as f32))
                .unwrap_or((200.0, 200.0));
            start_pos_1 = sp1
                .map(|sp| (sp.x as f32, sp.z as f32))
                .unwrap_or((824.0, 824.0));
        } else {
            start_pos_0 = (150.0, 150.0);
            start_pos_1 = (450.0, 450.0);
        }

        // Spawn units around start positions
        let mut rng = Lcg::new(self.rng_seed);
        for i in 0..NUM_UNITS {
            let team = (i % 2) as u8;
            let (cx, cz) = if team == 0 { start_pos_0 } else { start_pos_1 };
            let (hp, max_speed, accel, turn_rate, collision_r, sight_r, armor_class, unit_type_id) =
                if team == 0 { stats_team0 } else { stats_team1 };
            let weapon_ids = if team == 0 {
                &weapons_team0
            } else {
                &weapons_team1
            };

            // Spread units in a cluster around the start position
            let x = cx + rng.next_f32(120.0) - 60.0;
            let z = cz + rng.next_f32(120.0) - 60.0;

            let entity = spawn_unit(
                &mut self.world,
                Position {
                    pos: SimVec3::new(SimFloat::from_f32(x), SimFloat::ZERO, SimFloat::from_f32(z)),
                },
                UnitType { id: unit_type_id },
                Allegiance { team },
                Health {
                    current: hp,
                    max: hp,
                },
            );

            let weapons: Vec<WeaponInstance> = weapon_ids
                .iter()
                .map(|&def_id| WeaponInstance {
                    def_id,
                    reload_remaining: 0,
                })
                .collect();

            self.world.entity_mut(entity).insert((
                MoveState::Idle,
                MovementParams {
                    max_speed,
                    acceleration: accel,
                    turn_rate,
                },
                CollisionRadius {
                    radius: collision_r,
                },
                Heading {
                    angle: SimFloat::ZERO,
                },
                Velocity { vel: SimVec3::ZERO },
                armor_class,
                Target { entity: None },
                WeaponSet { weapons },
                CommandQueue::default(),
                SightRange { range: sight_r },
            ));

            // First unit per team is a resource producer
            if i < 2 {
                self.world.entity_mut(entity).insert(ResourceProducer {
                    metal_per_tick: SimFloat::from_int(1),
                    energy_per_tick: SimFloat::from_int(2),
                });
            }
        }

        // Store unit def registry as a resource for potential future use
        self.world.insert_resource(unit_def_registry);

        self.rng_seed = self.rng_seed.wrapping_add(7);
    }

    /// Run one simulation tick. Returns impact positions and death positions
    /// for particle effects.
    fn tick(&mut self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        // Snapshot entities with low health before sim_tick to detect deaths
        let pre_death: Vec<[f32; 3]> = self
            .world
            .query_filtered::<(&Position, &Health), Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, h)| h.current <= SimFloat::ZERO)
            .map(|(p, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
            .collect();

        // Capture impact positions before sim_tick processes them
        let impact_positions: Vec<[f32; 3]> = self
            .world
            .resource::<ImpactEventQueue>()
            .events
            .iter()
            .map(|e| {
                [
                    e.position.x.to_f32(),
                    e.position.y.to_f32() + 5.0,
                    e.position.z.to_f32(),
                ]
            })
            .collect();

        // Run all 14 systems via the canonical sim_runner
        sim_runner::sim_tick(&mut self.world);

        // Detect newly dead entities (marked Dead by damage_system this frame)
        let new_deaths: Vec<[f32; 3]> = {
            let mut q = self.world.query::<(&Position, &Dead, &Health)>();
            q.iter(&self.world)
                .filter(|(_, _, h)| h.current <= SimFloat::ZERO)
                .map(|(p, _, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
                .collect()
        };

        // Combine pre-death captures with newly-dead entities (avoid duplicates
        // by using new_deaths which is authoritative)
        let death_positions = if new_deaths.is_empty() {
            pre_death
        } else {
            new_deaths
        };

        (impact_positions, death_positions)
    }

    /// Extract unit instances for rendering (exclude Dead entities).
    /// Health scales brightness; stunned units get a purple tint;
    /// selected units are scaled up via a heading trick (rendered bigger).
    fn unit_instances(&mut self) -> Vec<UnitInstance> {
        let selected = self.selected;

        self.world
            .query_filtered::<(
                Entity,
                &Position,
                &Heading,
                &Allegiance,
                &Health,
                Option<&Stunned>,
            ), Without<Dead>>()
            .iter(&self.world)
            .map(|(entity, pos, heading, allegiance, health, stunned)| {
                // Base team color
                let mut color = if allegiance.team == 0 {
                    [0.3f32, 0.5, 1.0] // blue
                } else {
                    [1.0f32, 0.3, 0.3] // red
                };

                // Scale brightness by HP fraction (min 0.2 so units are never invisible)
                let hp_frac = if health.max > SimFloat::ZERO {
                    (health.current.to_f32() / health.max.to_f32()).clamp(0.2, 1.0)
                } else {
                    1.0
                };
                color[0] *= hp_frac;
                color[1] *= hp_frac;
                color[2] *= hp_frac;

                // Stunned units: blend toward purple
                if stunned.is_some() {
                    color[0] = color[0] * 0.5 + 0.5 * 0.6;
                    color[1] *= 0.3;
                    color[2] = color[2] * 0.5 + 0.5 * 0.8;
                }

                // Selected unit: render slightly brighter (boost toward white)
                if selected == Some(entity) {
                    color[0] = (color[0] + 0.4).min(1.0);
                    color[1] = (color[1] + 0.4).min(1.0);
                    color[2] = (color[2] + 0.4).min(1.0);
                }

                UnitInstance {
                    position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                    heading: heading.angle.to_f32(),
                    team_color: color,
                    _pad: 0.0,
                }
            })
            .collect()
    }

    /// Extract projectile instances for rendering.
    fn projectile_instances(&mut self) -> Vec<ProjectileInstance> {
        self.world
            .query::<(&Position, &Velocity, &Projectile)>()
            .iter(&self.world)
            .map(|(pos, vel, _proj)| {
                let vx = vel.vel.x.to_f32();
                let vy = vel.vel.y.to_f32();
                let vz = vel.vel.z.to_f32();
                let len = (vx * vx + vy * vy + vz * vz).sqrt();
                let dir = if len > 1e-6 {
                    [vx / len, vy / len, vz / len]
                } else {
                    [0.0, 1.0, 0.0]
                };
                ProjectileInstance {
                    position: [
                        pos.pos.x.to_f32(),
                        pos.pos.y.to_f32() + 5.0,
                        pos.pos.z.to_f32(),
                    ],
                    size: 3.0,
                    velocity_dir: dir,
                    _pad: 0.0,
                    color: [1.0, 1.0, 0.3],
                    _pad2: 0.0,
                }
            })
            .collect()
    }

    /// Extract wreckage (Reclaimable, not Dead) as grey flattened billboards.
    fn wreckage_instances(&mut self) -> Vec<ProjectileInstance> {
        self.world
            .query_filtered::<(&Position, &Reclaimable), Without<Dead>>()
            .iter(&self.world)
            .map(|(pos, _recl)| ProjectileInstance {
                position: [
                    pos.pos.x.to_f32(),
                    pos.pos.y.to_f32() + 1.0,
                    pos.pos.z.to_f32(),
                ],
                size: 4.0,
                velocity_dir: [0.0, 1.0, 0.0],
                _pad: 0.0,
                color: [0.4, 0.35, 0.25], // grey-brown wreckage
                _pad2: 0.0,
            })
            .collect()
    }

    /// Draw thin red lines from units to their targets (rendered as small billboards
    /// along the line at intervals).
    fn target_line_instances(&mut self) -> Vec<ProjectileInstance> {
        let mut instances = Vec::new();

        // Collect targeting pairs: (source_pos, target_entity)
        let pairs: Vec<([f32; 3], Entity)> = self
            .world
            .query_filtered::<(&Position, &Target), Without<Dead>>()
            .iter(&self.world)
            .filter_map(|(pos, target)| {
                target.entity.map(|t| {
                    (
                        [
                            pos.pos.x.to_f32(),
                            pos.pos.y.to_f32() + 5.0,
                            pos.pos.z.to_f32(),
                        ],
                        t,
                    )
                })
            })
            .collect();

        for (src, target_entity) in pairs {
            if let Some(target_pos) = self.world.get::<Position>(target_entity) {
                let dst = [
                    target_pos.pos.x.to_f32(),
                    target_pos.pos.y.to_f32() + 5.0,
                    target_pos.pos.z.to_f32(),
                ];
                // Place 3 small dots along the line
                for i in 1..=3 {
                    let t = i as f32 / 4.0;
                    let p = [
                        src[0] + (dst[0] - src[0]) * t,
                        src[1] + (dst[1] - src[1]) * t,
                        src[2] + (dst[2] - src[2]) * t,
                    ];
                    instances.push(ProjectileInstance {
                        position: p,
                        size: 1.0,
                        velocity_dir: [0.0, 1.0, 0.0],
                        _pad: 0.0,
                        color: [1.0, 0.1, 0.1], // red targeting line dots
                        _pad2: 0.0,
                    });
                }
            }
        }

        instances
    }

    /// Count alive units per team.
    fn unit_counts(&mut self) -> (usize, usize) {
        let mut blue = 0usize;
        let mut red = 0usize;
        for allegiance in self
            .world
            .query_filtered::<&Allegiance, Without<Dead>>()
            .iter(&self.world)
        {
            if allegiance.team == 0 {
                blue += 1;
            } else {
                red += 1;
            }
        }
        (blue, red)
    }

    /// Log economy info every N frames.
    fn log_economy(&mut self) {
        let unit_count: usize = self
            .world
            .query_filtered::<Entity, Without<Dead>>()
            .iter(&self.world)
            .count();

        if let Some(economy) = self.world.get_resource::<EconomyState>() {
            let t0 = economy.teams.get(&0);
            let t1 = economy.teams.get(&1);
            let (m0, e0) = t0
                .map(|t| (t.metal.to_f32(), t.energy.to_f32()))
                .unwrap_or((0.0, 0.0));
            let (m1, e1) = t1
                .map(|t| (t.metal.to_f32(), t.energy.to_f32()))
                .unwrap_or((0.0, 0.0));

            tracing::info!(
                "Frame {} | Team 0: M:{:.0} E:{:.0} | Team 1: M:{:.0} E:{:.0} | Units: {}",
                self.frame_count,
                m0,
                e0,
                m1,
                e1,
                unit_count,
            );
        }
    }

    /// Find the nearest unit to a world (x, z) position.
    fn find_nearest_unit(&mut self, wx: f32, wz: f32) -> Option<Entity> {
        let mut best: Option<(Entity, f32)> = None;

        for (entity, pos) in self
            .world
            .query_filtered::<(Entity, &Position), Without<Dead>>()
            .iter(&self.world)
        {
            let dx = pos.pos.x.to_f32() - wx;
            let dz = pos.pos.z.to_f32() - wz;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq <= SELECT_RADIUS_SQ && (best.is_none() || dist_sq < best.unwrap().1) {
                best = Some((entity, dist_sq));
            }
        }

        best.map(|(e, _)| e)
    }

    /// Issue a move command to the selected unit.
    fn move_selected_to(&mut self, wx: f32, wz: f32) {
        if let Some(sel) = self.selected {
            if self.world.get::<MoveState>(sel).is_some() {
                let target = SimVec3::new(
                    SimFloat::from_f32(wx),
                    SimFloat::ZERO,
                    SimFloat::from_f32(wz),
                );
                *self.world.get_mut::<MoveState>(sel).unwrap() = MoveState::MovingTo(target);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Application (winit 0.30 ApplicationHandler)
// ---------------------------------------------------------------------------

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    sim: SimState,
    camera_ctrl: CameraController,
    particle_system: ParticleSystem,
    last_frame: Instant,
    // Input state
    cursor_pos: [f32; 2],
    window_size: [f32; 2],
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            sim: SimState::new(),
            camera_ctrl: CameraController::new(512.0, 512.0, 800.0),
            particle_system: ParticleSystem::new(4096),
            last_frame: Instant::now(),
            cursor_pos: [0.0; 2],
            window_size: [1280.0, 720.0],
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Recoil RTS")
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );

        let mut renderer = pollster::block_on(Renderer::new(Arc::clone(&window)))
            .expect("failed to create renderer");

        // Try to load BAR .s3o model for units (Peewee)
        let bar_models_dir = Path::new("../Beyond-All-Reason-Sandbox/objects3d/Units");
        let s3o_path = bar_models_dir.join("armpw.s3o");
        if s3o_path.exists() {
            match recoil_render::load_s3o_file(&s3o_path) {
                Ok((verts, indices)) => {
                    tracing::info!(
                        "Loaded .s3o model: {} verts, {} indices",
                        verts.len(),
                        indices.len()
                    );
                    renderer.set_unit_mesh(&verts, &indices);
                }
                Err(e) => tracing::warn!("Failed to load .s3o: {}", e),
            }
        } else {
            tracing::info!("No BAR models found at {:?}, using placeholder", s3o_path);
        }

        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // Hard exit to avoid wgpu surface cleanup crash on Windows.
                // The OS reclaims all resources anyway.
                std::process::exit(0);
            }

            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if size.width > 0 && size.height > 0 {
                        renderer.resize(size.width, size.height);
                        self.window_size = [size.width as f32, size.height as f32];
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;

                // Camera movement keys
                self.camera_ctrl.process_key(key, pressed);

                // Action keys (on press only)
                if pressed {
                    match key {
                        KeyCode::Space => {
                            self.sim.paused = !self.sim.paused;
                        }
                        KeyCode::KeyR => {
                            self.sim.reset();
                        }
                        KeyCode::Escape => {
                            event_loop.exit();
                        }
                        _ => {}
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = [position.x as f32, position.y as f32];
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                if self.renderer.is_none() {
                    return;
                }

                // Compute inverse VP for screen-to-ground ray cast
                let cam = self
                    .camera_ctrl
                    .camera(self.window_size[0] / self.window_size[1]);
                let vp = cam.view_projection();
                let inv_vp = match mat4_inverse(vp) {
                    Some(m) => m,
                    None => return,
                };

                let ground = screen_to_ground_raw(
                    self.cursor_pos[0],
                    self.cursor_pos[1],
                    self.window_size[0],
                    self.window_size[1],
                    &inv_vp,
                );

                if let Some((wx, wz)) = ground {
                    match button {
                        MouseButton::Left => {
                            self.sim.selected = self.sim.find_nearest_unit(wx, wz);
                        }
                        MouseButton::Right => {
                            self.sim.move_selected_to(wx, wz);
                        }
                        _ => {}
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

                // Update camera movement
                self.camera_ctrl.update();

                // Sim tick
                let mut impact_positions = Vec::new();
                let mut death_positions = Vec::new();
                if !self.sim.paused {
                    let (impacts, deaths) = self.sim.tick();
                    impact_positions = impacts;
                    death_positions = deaths;
                    self.sim.frame_count += 1;

                    // Log economy every 60 frames
                    if self.sim.frame_count.is_multiple_of(60) {
                        self.sim.log_economy();
                    }
                }

                // Emit particles for projectile impacts (small orange sparks)
                for pos in &impact_positions {
                    self.particle_system.emit(
                        *pos,
                        6,
                        [1.0, 0.6, 0.2, 1.0],
                        (5.0, 15.0),
                        (0.2, 0.5),
                        (1.0, 2.5),
                    );
                }

                // Emit explosion particles for unit deaths (bigger red-orange burst)
                for pos in &death_positions {
                    self.particle_system.emit(
                        *pos,
                        20,
                        [1.0, 0.3, 0.1, 1.0],
                        (10.0, 30.0),
                        (0.4, 1.0),
                        (2.0, 5.0),
                    );
                }

                // Particle system update
                self.particle_system.update(dt);

                // Build render data
                let unit_instances = self.sim.unit_instances();
                let mut proj_instances = self.sim.projectile_instances();

                // Add wreckage rendering
                proj_instances.extend(self.sim.wreckage_instances());

                // Add target line dots
                proj_instances.extend(self.sim.target_line_instances());

                // Add particle effects
                let particle_instances = self.particle_system.instances();
                proj_instances.extend(particle_instances);

                if let Some(renderer) = self.renderer.as_mut() {
                    let aspect = self.window_size[0] / self.window_size[1];
                    let cam = self.camera_ctrl.camera(aspect);
                    renderer.update_camera(&cam);
                    renderer.update_units(&unit_instances);
                    renderer.update_projectiles(&proj_instances);

                    match renderer.render() {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::error!("render error: {e}");
                        }
                    }
                }

                // Update window title with unit counts and frame number
                if let Some(window) = self.window.as_ref() {
                    let (blue, red) = self.sim.unit_counts();
                    let title = format!(
                        "Recoil RTS - Blue: {} Red: {} Frame: {}",
                        blue, red, self.sim.frame_count
                    );
                    window.set_title(&title);
                    window.request_redraw();
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Matrix inverse (for screen-to-ground picking)
// ---------------------------------------------------------------------------

/// 4x4 matrix inverse via cofactor expansion (column-major).
fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    // Flatten to row-major for easier indexing.
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
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Recoil RTS starting (3D renderer)...");

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
