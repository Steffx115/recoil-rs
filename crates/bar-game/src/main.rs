use std::sync::Arc;
use std::time::Instant;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::*;
use tracing_subscriber::EnvFilter;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use recoil_math::{SimFloat, SimVec2, SimVec3};
use recoil_render::camera::Camera;
use recoil_render::gpu::GpuContext;
use recoil_render::particles::ParticleSystem;
use recoil_render::projectile_renderer::{ProjectileInstance, ProjectileRenderer};
use recoil_render::terrain::TerrainResources;
use recoil_render::unit_renderer::{UnitInstance, UnitRenderer};

use recoil_sim::collision::collision_system;
use recoil_sim::combat_data::{ArmorClass, DamageTable, WeaponDef, WeaponInstance, WeaponSet};
use recoil_sim::commands::{command_system, CommandQueue};
use recoil_sim::components::Stunned;
use recoil_sim::damage::{damage_system, stun_system};
use recoil_sim::economy::{economy_system, init_economy, ResourceProducer};
use recoil_sim::lifecycle::{cleanup_dead, init_lifecycle, spawn_unit};
use recoil_sim::movement::movement_system;
use recoil_sim::pathfinding::TerrainGrid;
use recoil_sim::projectile::{
    projectile_movement_system, spawn_projectile_system, ImpactEventQueue, Projectile,
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
const SELECT_RADIUS: f32 = 20.0;
const CAMERA_PAN_SPEED: f32 = 5.0;
const ZOOM_SPEED: f32 = 0.1;
const ROTATE_SENSITIVITY: f32 = 0.005;

/// (sim_id, hp_current, hp_max, pos_x, pos_z, move_state, stunned, has_target)
type SelectedUnitInfo = (u64, f32, f32, f32, f32, String, bool, bool);

// ---------------------------------------------------------------------------
// Sim state
// ---------------------------------------------------------------------------

struct SimState {
    world: World,
    paused: bool,
    frame_count: u64,
    selected: Option<Entity>,
    sim_speed: i32,
    rng_seed: u64,
}

impl SimState {
    fn new() -> Self {
        let mut s = Self {
            world: World::new(),
            paused: false,
            frame_count: 0,
            selected: None,
            sim_speed: 1,
            rng_seed: 12345,
        };
        s.reset();
        s
    }

    fn reset(&mut self) {
        self.world = World::new();
        self.selected = None;
        self.frame_count = 0;

        init_lifecycle(&mut self.world);

        self.world.insert_resource(SpatialGrid::new(
            SimFloat::from_int(GRID_CELL_SIZE),
            GRID_DIM,
            GRID_DIM,
        ));
        self.world
            .insert_resource(TerrainGrid::new(64, 64, SimFloat::ONE));
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
                    current: SimFloat::from_int(100),
                    max: SimFloat::from_int(100),
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

    fn sim_tick(&mut self) {
        {
            let entities: Vec<(Entity, SimVec3)> = self
                .world
                .query_filtered::<(Entity, &Position), (
                    Without<recoil_sim::Dead>,
                    Without<recoil_sim::construction::Reclaimable>,
                )>()
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

    fn unit_count(&mut self) -> usize {
        self.world
            .query_filtered::<Entity, With<Position>>()
            .iter(&self.world)
            .count()
    }

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

// ---------------------------------------------------------------------------
// Camera controller
// ---------------------------------------------------------------------------

struct CameraController {
    camera: Camera,
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    rotating: bool,
}

impl CameraController {
    fn new() -> Self {
        Self {
            camera: Camera {
                eye: [300.0, 400.0, 300.0],
                target: [300.0, 0.0, 300.0],
                up: [0.0, 1.0, 0.0],
                fov_y: std::f32::consts::FRAC_PI_4,
                aspect: 16.0 / 9.0,
                near: 0.1,
                far: 2000.0,
            },
            forward: false,
            backward: false,
            left: false,
            right: false,
            rotating: false,
        }
    }

    fn update(&mut self) {
        let dx = self.camera.target[0] - self.camera.eye[0];
        let dz = self.camera.target[2] - self.camera.eye[2];
        let len = (dx * dx + dz * dz).sqrt().max(0.001);
        let fwd_x = dx / len;
        let fwd_z = dz / len;
        let right_x = fwd_z;
        let right_z = -fwd_x;

        let mut mx = 0.0f32;
        let mut mz = 0.0f32;

        if self.forward {
            mx += fwd_x;
            mz += fwd_z;
        }
        if self.backward {
            mx -= fwd_x;
            mz -= fwd_z;
        }
        if self.right {
            mx += right_x;
            mz += right_z;
        }
        if self.left {
            mx -= right_x;
            mz -= right_z;
        }

        self.camera.eye[0] += mx * CAMERA_PAN_SPEED;
        self.camera.eye[2] += mz * CAMERA_PAN_SPEED;
        self.camera.target[0] += mx * CAMERA_PAN_SPEED;
        self.camera.target[2] += mz * CAMERA_PAN_SPEED;
    }

    fn zoom(&mut self, delta: f32) {
        let dx = self.camera.eye[0] - self.camera.target[0];
        let dy = self.camera.eye[1] - self.camera.target[1];
        let dz = self.camera.eye[2] - self.camera.target[2];
        let factor = (1.0 - delta * ZOOM_SPEED).clamp(0.1, 10.0);
        self.camera.eye[0] = self.camera.target[0] + dx * factor;
        self.camera.eye[1] = self.camera.target[1] + dy * factor;
        self.camera.eye[2] = self.camera.target[2] + dz * factor;
    }

    fn rotate(&mut self, dx: f64, dy: f64) {
        let angle_x = -dx as f32 * ROTATE_SENSITIVITY;
        let angle_y = -dy as f32 * ROTATE_SENSITIVITY;

        let ox = self.camera.eye[0] - self.camera.target[0];
        let oz = self.camera.eye[2] - self.camera.target[2];
        let cos_a = angle_x.cos();
        let sin_a = angle_x.sin();
        self.camera.eye[0] = self.camera.target[0] + ox * cos_a - oz * sin_a;
        self.camera.eye[2] = self.camera.target[2] + ox * sin_a + oz * cos_a;

        let new_y = self.camera.eye[1] + angle_y * 100.0;
        self.camera.eye[1] = new_y.clamp(20.0, 800.0);
    }

    fn screen_to_ground(
        &self,
        screen_x: f32,
        screen_y: f32,
        screen_w: f32,
        screen_h: f32,
    ) -> Option<(f32, f32)> {
        let ndc_x = (2.0 * screen_x / screen_w) - 1.0;
        let ndc_y = 1.0 - (2.0 * screen_y / screen_h);

        let vp = self.camera.view_projection();
        let inv_vp = mat4_inverse(vp)?;

        let near_w = mat4_mul_vec4(inv_vp, [ndc_x, ndc_y, 0.0, 1.0]);
        let far_w = mat4_mul_vec4(inv_vp, [ndc_x, ndc_y, 1.0, 1.0]);

        if near_w[3].abs() < 1e-10 || far_w[3].abs() < 1e-10 {
            return None;
        }

        let np = [
            near_w[0] / near_w[3],
            near_w[1] / near_w[3],
            near_w[2] / near_w[3],
        ];
        let fp = [
            far_w[0] / far_w[3],
            far_w[1] / far_w[3],
            far_w[2] / far_w[3],
        ];

        let dir = [fp[0] - np[0], fp[1] - np[1], fp[2] - np[2]];
        if dir[1].abs() < 1e-10 {
            return None;
        }
        let t = -np[1] / dir[1];
        if t < 0.0 {
            return None;
        }
        Some((np[0] + dir[0] * t, np[2] + dir[2] * t))
    }
}

// ---------------------------------------------------------------------------
// Matrix helpers
// ---------------------------------------------------------------------------

fn mat4_mul_vec4(m: [[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for row in 0..4 {
        out[row] = m[0][row] * v[0] + m[1][row] * v[1] + m[2][row] * v[2] + m[3][row] * v[3];
    }
    out
}

#[allow(clippy::excessive_precision)]
fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let mut s = [0.0f32; 16];
    let mut inv = [0.0f32; 16];

    for col in 0..4 {
        for row in 0..4 {
            s[row * 4 + col] = m[col][row];
        }
    }

    inv[0] = s[5] * s[10] * s[15] - s[5] * s[11] * s[14] - s[9] * s[6] * s[15]
        + s[9] * s[7] * s[14]
        + s[13] * s[6] * s[11]
        - s[13] * s[7] * s[10];
    inv[4] = -s[4] * s[10] * s[15] + s[4] * s[11] * s[14] + s[8] * s[6] * s[15]
        - s[8] * s[7] * s[14]
        - s[12] * s[6] * s[11]
        + s[12] * s[7] * s[10];
    inv[8] = s[4] * s[9] * s[15] - s[4] * s[11] * s[13] - s[8] * s[5] * s[15]
        + s[8] * s[7] * s[13]
        + s[12] * s[5] * s[11]
        - s[12] * s[7] * s[9];
    inv[12] = -s[4] * s[9] * s[14] + s[4] * s[10] * s[13] + s[8] * s[5] * s[14]
        - s[8] * s[6] * s[13]
        - s[12] * s[5] * s[10]
        + s[12] * s[6] * s[9];
    inv[1] = -s[1] * s[10] * s[15] + s[1] * s[11] * s[14] + s[9] * s[2] * s[15]
        - s[9] * s[3] * s[14]
        - s[13] * s[2] * s[11]
        + s[13] * s[3] * s[10];
    inv[5] = s[0] * s[10] * s[15] - s[0] * s[11] * s[14] - s[8] * s[2] * s[15]
        + s[8] * s[3] * s[14]
        + s[12] * s[2] * s[11]
        - s[12] * s[3] * s[10];
    inv[9] = -s[0] * s[9] * s[15] + s[0] * s[11] * s[13] + s[8] * s[1] * s[15]
        - s[8] * s[3] * s[13]
        - s[12] * s[1] * s[11]
        + s[12] * s[3] * s[9];
    inv[13] = s[0] * s[9] * s[14] - s[0] * s[10] * s[13] - s[8] * s[1] * s[14]
        + s[8] * s[2] * s[13]
        + s[12] * s[1] * s[10]
        - s[12] * s[2] * s[9];
    inv[2] = s[1] * s[6] * s[15] - s[1] * s[7] * s[14] - s[5] * s[2] * s[15]
        + s[5] * s[3] * s[14]
        + s[13] * s[2] * s[7]
        - s[13] * s[3] * s[6];
    inv[6] = -s[0] * s[6] * s[15] + s[0] * s[7] * s[14] + s[4] * s[2] * s[15]
        - s[4] * s[3] * s[14]
        - s[12] * s[2] * s[7]
        + s[12] * s[3] * s[6];
    inv[10] = s[0] * s[5] * s[15] - s[0] * s[7] * s[13] - s[4] * s[1] * s[15]
        + s[4] * s[3] * s[13]
        + s[12] * s[1] * s[7]
        - s[12] * s[3] * s[5];
    inv[14] = -s[0] * s[5] * s[14] + s[0] * s[6] * s[13] + s[4] * s[1] * s[14]
        - s[4] * s[2] * s[13]
        - s[12] * s[1] * s[6]
        + s[12] * s[2] * s[5];
    inv[3] = -s[1] * s[6] * s[11] + s[1] * s[7] * s[10] + s[5] * s[2] * s[11]
        - s[5] * s[3] * s[10]
        - s[9] * s[2] * s[7]
        + s[9] * s[3] * s[6];
    inv[7] = s[0] * s[6] * s[11] - s[0] * s[7] * s[10] - s[4] * s[2] * s[11]
        + s[4] * s[3] * s[10]
        + s[8] * s[2] * s[7]
        - s[8] * s[3] * s[6];
    inv[11] = -s[0] * s[5] * s[11] + s[0] * s[7] * s[9] + s[4] * s[1] * s[11]
        - s[4] * s[3] * s[9]
        - s[8] * s[1] * s[7]
        + s[8] * s[3] * s[5];
    inv[15] = s[0] * s[5] * s[10] - s[0] * s[6] * s[9] - s[4] * s[1] * s[10]
        + s[4] * s[2] * s[9]
        + s[8] * s[1] * s[6]
        - s[8] * s[2] * s[5];

    let det = s[0] * inv[0] + s[1] * inv[4] + s[2] * inv[8] + s[3] * inv[12];
    if det.abs() < 1e-10 {
        return None;
    }
    let inv_det = 1.0 / det;
    for v in &mut inv {
        *v *= inv_det;
    }

    let mut result = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            result[col][row] = inv[row * 4 + col];
        }
    }
    Some(result)
}

// ---------------------------------------------------------------------------
// GPU render state (composed from recoil-render primitives)
// ---------------------------------------------------------------------------

struct RenderState {
    gpu: GpuContext,
    terrain: TerrainResources,
    unit_renderer: UnitRenderer,
    projectile_renderer: ProjectileRenderer,
    camera: Camera,
}

impl RenderState {
    fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let gpu = pollster::block_on(GpuContext::new(window))?;

        let camera = Camera {
            aspect: gpu.config.width as f32 / gpu.config.height as f32,
            ..Camera::default()
        };

        let terrain = TerrainResources::new(&gpu, &camera)?;
        let unit_renderer =
            UnitRenderer::new(&gpu.device, gpu.config.format, terrain.bind_group_layout());
        let projectile_renderer =
            ProjectileRenderer::new(&gpu.device, gpu.config.format, terrain.bind_group_layout());

        Ok(Self {
            gpu,
            terrain,
            unit_renderer,
            projectile_renderer,
            camera,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        self.camera.aspect = width as f32 / height as f32;
    }

    fn update_camera(&mut self, cam: &Camera) {
        self.camera = Camera {
            eye: cam.eye,
            target: cam.target,
            up: cam.up,
            fov_y: cam.fov_y,
            aspect: cam.aspect,
            near: cam.near,
            far: cam.far,
        };
        self.terrain.update_camera(&self.gpu.queue, &self.camera);
    }

    fn update_units(&mut self, instances: &[UnitInstance]) {
        self.unit_renderer
            .prepare(&self.gpu.device, &self.gpu.queue, instances);
    }

    fn update_projectiles(&mut self, instances: &[ProjectileInstance]) {
        self.projectile_renderer
            .prepare(&self.gpu.device, &self.gpu.queue, instances);
    }

    /// Render the 3D scene and the egui overlay in a single frame.
    fn render_frame(
        &mut self,
        egui_renderer: &mut egui_wgpu::Renderer,
        egui_tris: &[egui::ClippedPrimitive],
        egui_screen: &egui_wgpu::ScreenDescriptor,
    ) -> anyhow::Result<()> {
        let output = self
            .gpu
            .surface
            .get_current_texture()
            .map_err(|e| anyhow::anyhow!("Failed to acquire swapchain texture: {}", e))?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_encoder"),
            });

        // --- 3D render pass (forget_lifetime for compatibility with sub-renderers) ---
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.05,
                                g: 0.05,
                                b: 0.08,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.gpu.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();

            // Terrain
            pass.set_pipeline(&self.terrain.pipeline);
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.terrain.vertex_buffer.slice(..));
            pass.set_index_buffer(
                self.terrain.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.terrain.index_count, 0, 0..1);

            // Units
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            self.unit_renderer.render(&mut pass);

            // Projectiles + particles
            pass.set_bind_group(0, &self.terrain.camera_bind_group, &[]);
            self.projectile_renderer.render(&mut pass);
        }

        // --- egui overlay pass (no depth, load existing color) ---
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();

            egui_renderer.render(&mut pass, egui_tris, egui_screen);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

struct App {
    window: Option<Arc<Window>>,
    render_state: Option<RenderState>,
    egui_winit: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,

    sim: SimState,
    camera_ctrl: CameraController,
    particle_system: ParticleSystem,

    last_frame_time: Instant,
    fps: f32,
    frame_time_accum: f32,
    frame_count_for_fps: u32,

    cursor_pos: (f64, f64),
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            render_state: None,
            egui_winit: None,
            egui_renderer: None,

            sim: SimState::new(),
            camera_ctrl: CameraController::new(),
            particle_system: ParticleSystem::new(2048),

            last_frame_time: Instant::now(),
            fps: 0.0,
            frame_time_accum: 0.0,
            frame_count_for_fps: 0,

            cursor_pos: (0.0, 0.0),
        }
    }

    fn extract_unit_instances(&mut self) -> Vec<UnitInstance> {
        self.sim
            .world
            .query_filtered::<(
                &Position,
                &Heading,
                &Allegiance,
                &CollisionRadius,
            ), Without<recoil_sim::Dead>>()
            .iter(&self.sim.world)
            .map(|(pos, heading, allegiance, _cr)| {
                let team_color = if allegiance.team == 0 {
                    [0.3, 0.5, 1.0]
                } else {
                    [1.0, 0.3, 0.3]
                };
                UnitInstance {
                    position: [
                        pos.pos.x.to_f32(),
                        pos.pos.y.to_f32(),
                        pos.pos.z.to_f32(),
                    ],
                    heading: heading.angle.to_f32(),
                    team_color,
                    _pad: 0.0,
                }
            })
            .collect()
    }

    fn extract_projectile_instances(&mut self) -> Vec<ProjectileInstance> {
        self.sim
            .world
            .query::<(&Position, &Velocity, &Projectile)>()
            .iter(&self.sim.world)
            .map(|(pos, vel, _proj)| {
                let vx = vel.vel.x.to_f32();
                let vy = vel.vel.y.to_f32();
                let vz = vel.vel.z.to_f32();
                let speed = (vx * vx + vy * vy + vz * vz).sqrt().max(0.001);
                ProjectileInstance {
                    position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                    size: 2.0,
                    velocity_dir: [vx / speed, vy / speed, vz / speed],
                    _pad: 0.0,
                    color: [1.0, 1.0, 0.3],
                    _pad2: 0.0,
                }
            })
            .collect()
    }

    fn emit_impact_particles(&mut self) {
        let impacts: Vec<[f32; 3]> = self
            .sim
            .world
            .resource::<ImpactEventQueue>()
            .events
            .iter()
            .map(|e| {
                [
                    e.position.x.to_f32(),
                    e.position.y.to_f32(),
                    e.position.z.to_f32(),
                ]
            })
            .collect();

        for pos in impacts {
            self.particle_system.emit(
                pos,
                12,
                [1.0, 0.6, 0.1, 1.0],
                (5.0, 15.0),
                (0.3, 0.8),
                (1.0, 3.0),
            );
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Recoil RTS")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create window"),
        );

        let rs = RenderState::new(window.clone()).expect("Failed to init GPU");

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let egui_rend =
            egui_wgpu::Renderer::new(&rs.gpu.device, rs.gpu.config.format, None, 1, false);

        self.window = Some(window);
        self.egui_renderer = Some(egui_rend);
        self.egui_winit = Some(egui_state);
        self.render_state = Some(rs);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Let egui consume events first.
        if let Some(ref mut egui_state) = self.egui_winit {
            let response = egui_state.on_window_event(self.window.as_ref().unwrap(), &event);
            if response.consumed {
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(ref mut rs) = self.render_state {
                    rs.resize(size.width, size.height);
                    self.camera_ctrl.camera.aspect = size.width as f32 / size.height as f32;
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::KeyW) | PhysicalKey::Code(KeyCode::ArrowUp) => {
                        self.camera_ctrl.forward = pressed;
                    }
                    PhysicalKey::Code(KeyCode::KeyS) | PhysicalKey::Code(KeyCode::ArrowDown) => {
                        self.camera_ctrl.backward = pressed;
                    }
                    PhysicalKey::Code(KeyCode::KeyA) | PhysicalKey::Code(KeyCode::ArrowLeft) => {
                        self.camera_ctrl.left = pressed;
                    }
                    PhysicalKey::Code(KeyCode::KeyD) | PhysicalKey::Code(KeyCode::ArrowRight) => {
                        self.camera_ctrl.right = pressed;
                    }
                    PhysicalKey::Code(KeyCode::Space) if pressed => {
                        self.sim.paused = !self.sim.paused;
                    }
                    PhysicalKey::Code(KeyCode::KeyR) if pressed => {
                        self.sim.reset();
                    }
                    PhysicalKey::Code(KeyCode::Escape) if pressed => event_loop.exit(),
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 50.0,
                };
                self.camera_ctrl.zoom(scroll);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let old = self.cursor_pos;
                self.cursor_pos = (position.x, position.y);
                if self.camera_ctrl.rotating {
                    self.camera_ctrl
                        .rotate(position.x - old.0, position.y - old.1);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match button {
                MouseButton::Middle => {
                    self.camera_ctrl.rotating = state == ElementState::Pressed;
                }
                MouseButton::Left if state == ElementState::Released => {
                    if let Some(ref rs) = self.render_state {
                        let w = rs.gpu.config.width as f32;
                        let h = rs.gpu.config.height as f32;
                        if let Some((gx, gz)) = self.camera_ctrl.screen_to_ground(
                            self.cursor_pos.0 as f32,
                            self.cursor_pos.1 as f32,
                            w,
                            h,
                        ) {
                            self.sim.selected = self.sim.find_nearest_unit(gx, gz);
                        }
                    }
                }
                MouseButton::Right if state == ElementState::Released => {
                    if let Some(sel) = self.sim.selected {
                        if let Some(ref rs) = self.render_state {
                            let w = rs.gpu.config.width as f32;
                            let h = rs.gpu.config.height as f32;
                            if let Some((gx, gz)) = self.camera_ctrl.screen_to_ground(
                                self.cursor_pos.0 as f32,
                                self.cursor_pos.1 as f32,
                                w,
                                h,
                            ) {
                                if self.sim.world.get::<MoveState>(sel).is_some() {
                                    let target = SimVec3::new(
                                        SimFloat::from_f32(gx),
                                        SimFloat::ZERO,
                                        SimFloat::from_f32(gz),
                                    );
                                    *self.sim.world.get_mut::<MoveState>(sel).unwrap() =
                                        MoveState::MovingTo(target);
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                // FPS tracking
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame_time).as_secs_f32();
                self.last_frame_time = now;
                self.frame_time_accum += dt;
                self.frame_count_for_fps += 1;
                if self.frame_time_accum >= 1.0 {
                    self.fps = self.frame_count_for_fps as f32 / self.frame_time_accum;
                    self.frame_time_accum = 0.0;
                    self.frame_count_for_fps = 0;
                }

                // Sim tick
                if !self.sim.paused {
                    for _ in 0..self.sim.sim_speed {
                        self.sim.sim_tick();
                    }
                    self.sim.frame_count += 1;
                }

                // Particles from impacts
                self.emit_impact_particles();
                self.particle_system.update(dt);

                // Camera
                self.camera_ctrl.update();

                // Extract render data
                let unit_instances = self.extract_unit_instances();
                let mut projectile_instances = self.extract_projectile_instances();
                projectile_instances.extend(self.particle_system.instances());

                // Update GPU data
                let rs = self.render_state.as_mut().unwrap();
                rs.update_camera(&self.camera_ctrl.camera);
                rs.update_units(&unit_instances);
                rs.update_projectiles(&projectile_instances);

                // Build egui frame
                let window = self.window.as_ref().unwrap();
                let egui_state = self.egui_winit.as_mut().unwrap();
                let egui_renderer = self.egui_renderer.as_mut().unwrap();

                let raw_input = egui_state.take_egui_input(window);
                let ctx = egui_state.egui_ctx().clone();

                let fps = self.fps;
                let frame_count = self.sim.frame_count;
                let unit_count = self.sim.unit_count();
                let paused = self.sim.paused;

                // Gather selected unit info before the closure to avoid borrow issues.
                let selected_info: Option<SelectedUnitInfo> =
                    self.sim.selected.and_then(|entity| {
                        let sim_id = self.sim.world.get::<SimId>(entity)?.id;
                        let health = self.sim.world.get::<Health>(entity)?;
                        let hp_cur = health.current.to_f32();
                        let hp_max = health.max.to_f32();
                        let pos = self.sim.world.get::<Position>(entity)?;
                        let px = pos.pos.x.to_f32();
                        let pz = pos.pos.z.to_f32();
                        let state_str = self
                            .sim
                            .world
                            .get::<MoveState>(entity)
                            .map(|s| format!("{:?}", *s))
                            .unwrap_or_default();
                        let stunned = self.sim.world.get::<Stunned>(entity).is_some();
                        let has_target = self
                            .sim
                            .world
                            .get::<Target>(entity)
                            .and_then(|t| t.entity)
                            .is_some();
                        Some((
                            sim_id, hp_cur, hp_max, px, pz, state_str, stunned, has_target,
                        ))
                    });

                let mut sim_speed = self.sim.sim_speed;
                let mut toggle_pause = false;

                let full_output = ctx.run(raw_input, |ctx| {
                    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("FPS: {:.0}", fps));
                            ui.separator();
                            ui.label(format!("Frame: {}", frame_count));
                            ui.separator();
                            ui.label(format!("Units: {}", unit_count));
                            ui.separator();
                            if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
                                toggle_pause = true;
                            }
                            ui.add(egui::Slider::new(&mut sim_speed, 1..=10).text("Speed"));
                        });
                    });

                    egui::SidePanel::left("selected_panel")
                        .min_width(180.0)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.heading("Selected Unit");
                            ui.separator();
                            if let Some((sid, hp, hp_max, px, pz, ref state, stunned, has_target)) =
                                selected_info
                            {
                                ui.label(format!("SimId: {}", sid));
                                ui.label(format!("HP: {:.0}/{:.0}", hp, hp_max));
                                ui.label(format!("Pos: ({:.1}, {:.1})", px, pz));
                                ui.label(format!("State: {}", state));
                                if stunned {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(200, 100, 255),
                                        "STUNNED",
                                    );
                                }
                                if has_target {
                                    ui.label("Target: engaged");
                                }
                            } else {
                                ui.label("No unit selected");
                            }
                        });
                });

                if toggle_pause {
                    self.sim.paused = !self.sim.paused;
                }
                self.sim.sim_speed = sim_speed;

                egui_state.handle_platform_output(window, full_output.platform_output);

                let tris = ctx.tessellate(full_output.shapes, ctx.pixels_per_point());
                for (id, delta) in &full_output.textures_delta.set {
                    egui_renderer.update_texture(&rs.gpu.device, &rs.gpu.queue, *id, delta);
                }

                let screen_desc = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [rs.gpu.config.width, rs.gpu.config.height],
                    pixels_per_point: ctx.pixels_per_point(),
                };

                // Render 3D + egui overlay in a single submission
                if let Err(e) = rs.render_frame(egui_renderer, &tris, &screen_desc) {
                    tracing::error!("Render error: {}", e);
                }

                for id in &full_output.textures_delta.free {
                    egui_renderer.free_texture(id);
                }

                window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Recoil RTS starting...");

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("Event loop error");
}
