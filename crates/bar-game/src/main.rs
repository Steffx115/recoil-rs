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

use recoil_math::{SimFloat, SimVec2, SimVec3};
use recoil_render::camera::Camera;
use recoil_render::particles::ParticleSystem;
use recoil_render::projectile_renderer::ProjectileInstance;
use recoil_render::unit_renderer::UnitInstance;
use recoil_render::Renderer;
use recoil_sim::collision::collision_system;
use recoil_sim::combat_data::WeaponSet;
use recoil_sim::combat_data::{ArmorClass, DamageTable, WeaponDef, WeaponInstance};
use recoil_sim::commands::{command_system, CommandQueue};
use recoil_sim::damage::{damage_system, stun_system};
use recoil_sim::economy::{economy_system, init_economy, ResourceProducer};
use recoil_sim::lifecycle::{cleanup_dead, init_lifecycle, spawn_unit};
use recoil_sim::movement::movement_system;
use recoil_sim::pathfinding::TerrainGrid;
use recoil_sim::projectile::{
    projectile_movement_system, spawn_projectile_system, ImpactEventQueue, Projectile,
};
use recoil_sim::selection::screen_to_ground_raw;
use recoil_sim::spatial::SpatialGrid;
use recoil_sim::targeting::{reload_system, targeting_system, FireEventQueue, WeaponRegistry};
use recoil_sim::{
    Allegiance, CollisionRadius, Dead, Heading, Health, MoveState, MovementParams, Position,
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
const WORLD_SIZE: f32 = 600.0;
const GRID_CELL_SIZE: i32 = 10;
const GRID_DIM: i32 = 64;
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

        init_lifecycle(&mut self.world);

        let grid = SpatialGrid::new(SimFloat::from_int(GRID_CELL_SIZE), GRID_DIM, GRID_DIM);
        self.world.insert_resource(grid);

        let terrain = TerrainGrid::new(64, 64, SimFloat::ONE);
        self.world.insert_resource(terrain);

        self.world.insert_resource(DamageTable::default());

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

        self.world
            .insert_resource(FireEventQueue { events: Vec::new() });
        self.world
            .insert_resource(ImpactEventQueue { events: Vec::new() });

        init_economy(&mut self.world, &[0, 1]);

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
                    current: SimFloat::from_int(500),
                    max: SimFloat::from_int(500),
                },
            );

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

            if i < 2 {
                self.world.entity_mut(entity).insert(ResourceProducer {
                    metal_per_tick: SimFloat::from_int(1),
                    energy_per_tick: SimFloat::from_int(2),
                });
            }
        }

        self.rng_seed = self.rng_seed.wrapping_add(7);
    }

    fn tick(&mut self) {
        // 1. Rebuild SpatialGrid
        {
            let entities: Vec<(Entity, SimVec3)> = self
                .world
                .query_filtered::<(Entity, &Position), Without<Dead>>()
                .iter(&self.world)
                .map(|(e, p)| (e, p.pos))
                .collect();

            let mut grid = self.world.resource_mut::<SpatialGrid>();
            grid.clear();
            for (e, pos) in entities {
                grid.insert(e, SimVec2::new(pos.x, pos.z));
            }
        }

        self.world.resource_mut::<FireEventQueue>().events.clear();

        command_system(&mut self.world);
        economy_system(&mut self.world);
        movement_system(&mut self.world);
        collision_system(&mut self.world);
        targeting_system(&mut self.world);
        reload_system(&mut self.world);
        spawn_projectile_system(&mut self.world);
        projectile_movement_system(&mut self.world);
        damage_system(&mut self.world);
        stun_system(&mut self.world);
        cleanup_dead(&mut self.world);
    }

    /// Extract unit instances for rendering (exclude Dead entities).
    fn unit_instances(&mut self) -> Vec<UnitInstance> {
        self.world
            .query_filtered::<(&Position, &Heading, &Allegiance), Without<Dead>>()
            .iter(&self.world)
            .map(|(pos, heading, allegiance)| {
                let team_color = if allegiance.team == 0 {
                    [0.3, 0.5, 1.0] // blue
                } else {
                    [1.0, 0.3, 0.3] // red
                };
                UnitInstance {
                    position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                    heading: heading.angle.to_f32(),
                    team_color,
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
            camera_ctrl: CameraController::new(300.0, 300.0, 400.0),
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

        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window)))
            .expect("failed to create renderer");

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
                if !self.sim.paused {
                    self.sim.tick();
                    self.sim.frame_count += 1;
                }

                // Particle system update
                self.particle_system.update(dt);

                // Build render data
                let unit_instances = self.sim.unit_instances();
                let mut proj_instances = self.sim.projectile_instances();
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

                // Request next frame
                if let Some(window) = self.window.as_ref() {
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
