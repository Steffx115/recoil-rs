//! Pierce RTS — game binary.
//!
//! Thin shell: window, renderer, input dispatch, egui overlay.
//! All game logic lives in `bar-game-lib`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use bevy_ecs::entity::Entity;

use pierce_cob::CobAnimationDriver;
use pierce_model::{PieceTree, PieceTransform, flatten_with_transforms};
use pierce_render::particles::ParticleSystem;
use pierce_render::unit_renderer::UnitInstance;
use pierce_render::Renderer;
use pierce_s3o::load_s3o_tree;
use pierce_sim::unit_defs::UnitDefRegistry;
use pierce_sim::{Dead, FireEventQueue, MoveState, Position, UnitType};

use bar_game_lib::GameState;

use egui_wgpu::ScreenDescriptor;

mod camera_controller;
mod icons;
mod input;
mod loadtest;

use camera_controller::{CameraController, FpsCounter};
use icons::IconAtlas;
use input::{
    building_instances, draw_egui_ui, gather_ui_data, handle_key_press, handle_mouse_press,
    mat4_inverse, process_ui_actions, projectile_instances, unit_instances,
};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const BAR_UNITS_PATH: &str = "../Beyond-All-Reason-Sandbox/units";
const BAR_UNITPICS_PATH: &str = "../Beyond-All-Reason-Sandbox/unitpics";
const MAP_MANIFEST_PATH: &str = "assets/maps/small_duel/manifest.ron";

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    game: GameState,
    camera_ctrl: CameraController,
    particle_system: ParticleSystem,
    animation_driver: CobAnimationDriver,
    piece_trees: BTreeMap<u32, PieceTree>,
    last_frame: Instant,
    cursor_pos: [f32; 2],
    window_size: [f32; 2],
    modifiers: ModifiersState,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    fps_counter: FpsCounter,
    icon_atlas: Option<IconAtlas>,
    loadtest: loadtest::LoadtestState,
}

const LOADTEST_MAP_PATH: &str = "assets/maps/loadtest/manifest.ron";

impl App {
    fn new() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let is_loadtest = args.iter().any(|a| a == "--loadtest");
        let units_per_wave: usize = args
            .iter()
            .position(|a| a == "--wave-size")
            .and_then(|i| args.get(i + 1)?.parse().ok())
            .unwrap_or(50);
        let max_units: usize = args
            .iter()
            .position(|a| a == "--max-units")
            .and_then(|i| args.get(i + 1)?.parse().ok())
            .unwrap_or(2000);

        let map_path = if is_loadtest { LOADTEST_MAP_PATH } else { MAP_MANIFEST_PATH };
        let game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(map_path));
        let (cx, cz) = game
            .commander_team0
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()))
            .unwrap_or((512.0, 512.0));

        Self {
            window: None,
            renderer: None,
            game,
            camera_ctrl: CameraController::new(cx, cz, 400.0),
            particle_system: ParticleSystem::new(4096),
            animation_driver: CobAnimationDriver::new(),
            piece_trees: BTreeMap::new(),
            last_frame: Instant::now(),
            cursor_pos: [0.0; 2],
            window_size: [1280.0, 720.0],
            modifiers: ModifiersState::empty(),
            egui_state: None,
            egui_renderer: None,
            fps_counter: FpsCounter::new(),
            icon_atlas: None,
            loadtest: if is_loadtest {
                loadtest::LoadtestState::new(units_per_wave, max_units)
            } else {
                loadtest::LoadtestState::default()
            },
        }
    }

    fn screen_to_ground(&self) -> Option<(f32, f32)> {
        let cam = self
            .camera_ctrl
            .camera(self.window_size[0] / self.window_size[1]);
        let vp = cam.view_projection();
        let inv_vp = mat4_inverse(vp)?;
        pierce_sim::selection::screen_to_ground_raw(
            self.cursor_pos[0],
            self.cursor_pos[1],
            self.window_size[0],
            self.window_size[1],
            &inv_vp,
        )
    }

    fn load_models(
        renderer: &mut Renderer,
        animation_driver: &mut CobAnimationDriver,
        piece_trees: &mut BTreeMap<u32, PieceTree>,
        registry: &UnitDefRegistry,
    ) {
        let bar_models_dir = Path::new("../Beyond-All-Reason-Sandbox/objects3d/Units");
        if !bar_models_dir.exists() {
            return;
        }
        let model_entries: Vec<(u32, String, String)> = registry
            .defs
            .values()
            .filter_map(|def| {
                def.model_path
                    .as_ref()
                    .map(|p| (def.unit_type_id, p.clone(), def.name.clone()))
            })
            .collect();

        let scale = 0.2;
        let mut loaded = 0;
        for (type_id, model_path, _unit_name) in &model_entries {
            let filename = model_path.strip_prefix("Units/").unwrap_or(model_path);
            let s3o_path = bar_models_dir.join(filename);
            if !s3o_path.exists() {
                continue;
            }
            if let Ok((mut verts, indices)) = pierce_render::load_s3o_file(&s3o_path) {
                for v in &mut verts {
                    let (x, z) = (v.position[0], v.position[2]);
                    v.position[0] = z * scale;
                    v.position[1] *= scale;
                    v.position[2] = -x * scale;
                    let (nx, nz) = (v.normal[0], v.normal[2]);
                    v.normal[0] = nz;
                    v.normal[2] = -nx;
                }
                renderer.register_unit_mesh(*type_id, &verts, &indices);
                loaded += 1;
            }
        }
        // Set the first loaded model as the placeholder (mesh_id=0)
        if let Some((_, ref first_path, _)) = model_entries.first() {
            let filename = first_path.strip_prefix("Units/").unwrap_or(first_path);
            let s3o_path = bar_models_dir.join(filename);
            if let Ok((mut verts, indices)) = pierce_render::load_s3o_file(&s3o_path) {
                for v in &mut verts {
                    let (x, z) = (v.position[0], v.position[2]);
                    v.position[0] = z * scale;
                    v.position[1] *= scale;
                    v.position[2] = -x * scale;
                    let (nx, nz) = (v.normal[0], v.normal[2]);
                    v.normal[0] = nz;
                    v.normal[2] = -nx;
                }
                renderer.set_unit_mesh(&verts, &indices);
            }
        }
        tracing::info!(
            "Loaded {} S3O models for {} unit types",
            loaded,
            model_entries.len()
        );

        // Load COB animation scripts and piece trees.
        let cob_dir = Path::new("../Beyond-All-Reason-Sandbox/scripts/Units");
        if !cob_dir.exists() {
            tracing::info!("COB scripts directory not found, skipping animation loading");
            return;
        }
        let mut cob_loaded = 0;
        for (type_id, model_path, unit_name) in &model_entries {
            let cob_path = cob_dir.join(format!("{}.cob", unit_name.to_lowercase()));
            if !cob_path.exists() {
                continue;
            }
            match std::fs::read(&cob_path) {
                Ok(cob_data) => {
                    if let Err(e) = animation_driver.load_script(*type_id, &cob_data) {
                        tracing::warn!("Failed to parse COB {}: {}", cob_path.display(), e);
                        continue;
                    }
                    cob_loaded += 1;
                    // Load the piece tree for this animated unit type.
                    let filename = model_path.strip_prefix("Units/").unwrap_or(model_path);
                    let s3o_path = bar_models_dir.join(filename);
                    if let Ok(tree) = load_s3o_tree(&std::fs::read(&s3o_path).unwrap_or_default()) {
                        piece_trees.insert(*type_id, tree);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read COB {}: {}", cob_path.display(), e);
                }
            }
        }
        tracing::info!("Loaded {} COB animation scripts", cob_loaded);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Pierce RTS")
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let mut renderer =
            pollster::block_on(Renderer::new(Arc::clone(&window))).expect("renderer");

        let registry = self.game.world.resource::<UnitDefRegistry>();
        Self::load_models(&mut renderer, &mut self.animation_driver, &mut self.piece_trees, registry);

        // egui
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(renderer.gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &renderer.gpu.device,
            renderer.gpu.config.format,
            None,
            1,
            false,
        );

        let icon_atlas =
            IconAtlas::load_unitpics(egui_state.egui_ctx(), Path::new(BAR_UNITPICS_PATH));
        self.icon_atlas = Some(icon_atlas);

        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.window = Some(window);

        // Wire GPU compute backends for fog and targeting.
        #[cfg(feature = "gpu-compute")]
        {
            let device = renderer.gpu.device.clone();
            let queue = renderer.gpu.queue.clone();
            let fog_compute = pierce_compute::GpuFogCompute::new(device.clone(), queue.clone());
            let targeting_compute = pierce_compute::GpuTargetingCompute::new(device, queue);
            self.game.world.insert_resource(
                pierce_sim::compute::ComputeBackends {
                    fog: Box::new(fog_compute),
                    targeting: Box::new(targeting_compute),
                },
            );
            // Refresh sim capabilities since we added ComputeBackends.
            self.game.refresh_sim_caps();
        }

        self.renderer = Some(renderer);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // egui first
        if let (Some(es), Some(w)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            if es.on_window_event(w, &event).consumed {
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested => std::process::exit(0),

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(r) = self.renderer.as_mut() {
                        r.resize(size.width, size.height);
                    }
                    self.window_size = [size.width as f32, size.height as f32];
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
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
                self.camera_ctrl.process_key(key, pressed);
                if pressed {
                    let should_exit = handle_key_press(
                        &mut self.game,
                        key,
                        self.modifiers,
                        Path::new(BAR_UNITS_PATH),
                        Path::new(MAP_MANIFEST_PATH),
                    );
                    if should_exit {
                        event_loop.exit();
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
                let ground = self.screen_to_ground();
                handle_mouse_press(&mut self.game, button, ground, self.modifiers.shift_key());
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
                    self.loadtest
                        .tick(&mut self.game.world, self.game.frame_count);
                    for pos in &impacts {
                        self.particle_system.emit(
                            *pos,
                            6,
                            [1.0, 0.6, 0.2, 1.0],
                            (5.0, 15.0),
                            (0.2, 0.5),
                            (1.0, 2.5),
                        );
                    }
                    for pos in &deaths {
                        self.particle_system.emit(
                            *pos,
                            20,
                            [1.0, 0.3, 0.1, 1.0],
                            (10.0, 30.0),
                            (0.4, 1.0),
                            (2.0, 5.0),
                        );
                    }
                }
                self.particle_system.update(dt);

                // Sync animation driver with game state.
                self.sync_animations();

                // Generate animated meshes and register with renderer.
                let anim_mesh_ids = self.update_animated_meshes();

                // Gather render data
                let mut instances = unit_instances(&mut self.game, &anim_mesh_ids);
                instances.extend(building_instances(&mut self.game));
                if let Some(ref pt) = self.game.placement_mode {
                    if let Some((gx, gz)) = self.screen_to_ground() {
                        // Check if placement is valid.
                        let can_place = {
                            use pierce_sim::footprint::can_place_building;
                            use pierce_sim::pathfinding::TerrainGrid;
                            use pierce_sim::SimFloat;
                            let grid = self.game.world.resource::<TerrainGrid>();
                            let pos = pierce_sim::SimVec2::new(
                                SimFloat::from_f32(gx),
                                SimFloat::from_f32(gz),
                            );
                            let radius = self.game.world.resource::<pierce_sim::unit_defs::UnitDefRegistry>()
                                .get(pt.0)
                                .map(|d| SimFloat::from_f64(d.collision_radius))
                                .unwrap_or(SimFloat::from_int(2));
                            can_place_building(grid, pos, radius)
                        };
                        let color = if can_place {
                            [0.3, 0.9, 0.3] // green = valid
                        } else {
                            [0.9, 0.3, 0.3] // red = invalid
                        };
                        instances.push(UnitInstance {
                            position: [gx, 0.0, gz],
                            heading: 0.0,
                            team_color: color,
                            alpha: 0.5,
                            mesh_id: pt.0,
                            _pad: [0; 3],
                        });
                    }
                }
                let mut proj = projectile_instances(&mut self.game);
                proj.extend(self.particle_system.instances());

                let fps = self.fps_counter.tick();
                self.render_frame(&instances, &proj, fps);
            }

            _ => {}
        }
    }
}

impl App {
    fn render_frame(
        &mut self,
        instances: &[UnitInstance],
        proj: &[pierce_render::projectile_renderer::ProjectileInstance],
        fps: f32,
    ) {
        let (Some(renderer), Some(egui_state), Some(egui_renderer), Some(window)) = (
            self.renderer.as_mut(),
            self.egui_state.as_mut(),
            self.egui_renderer.as_mut(),
            self.window.as_ref(),
        ) else {
            return;
        };

        let cam = self
            .camera_ctrl
            .camera(self.window_size[0] / self.window_size[1]);
        renderer.update_camera(&cam);
        renderer.update_units(instances);
        renderer.update_projectiles(proj);

        let (output, view) = match renderer.render_no_present() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("render: {e}");
                window.request_redraw();
                return;
            }
        };

        // egui
        let raw_input = egui_state.take_egui_input(window);
        let egui_ctx = egui_state.egui_ctx().clone();
        let vp_mat = cam.view_projection();
        let ui_data = gather_ui_data(
            &mut self.game,
            fps,
            &vp_mat,
            self.window_size,
            self.camera_ctrl.center,
        );
        let empty_atlas = IconAtlas::empty();
        let atlas = self.icon_atlas.as_ref().unwrap_or(&empty_atlas);
        let mut ui_actions = Vec::new();
        let full_output = egui_ctx.run(raw_input, |ctx| {
            ui_actions = draw_egui_ui(ctx, &ui_data, atlas);
        });
        egui_state.handle_platform_output(window, full_output.platform_output);

        let tris = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, delta) in &full_output.textures_delta.set {
            egui_renderer.update_texture(&renderer.gpu.device, &renderer.gpu.queue, *id, delta);
        }
        let screen_desc = ScreenDescriptor {
            size_in_pixels: [renderer.gpu.config.width, renderer.gpu.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        let mut encoder =
            renderer
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("egui"),
                });
        let user_bufs = egui_renderer.update_buffers(
            &renderer.gpu.device,
            &renderer.gpu.queue,
            &mut encoder,
            &tris,
            &screen_desc,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            });
            let mut pass = pass.forget_lifetime();
            egui_renderer.render(&mut pass, &tris, &screen_desc);
        }
        let mut bufs: Vec<wgpu::CommandBuffer> = vec![encoder.finish()];
        bufs.extend(user_bufs);
        renderer.gpu.queue.submit(bufs);
        for id in &full_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }
        output.present();

        process_ui_actions(&mut self.game, ui_actions);
        window.request_redraw();
    }

    /// Synchronise the animation driver with the current game state.
    fn sync_animations(&mut self) {
        use bevy_ecs::query::Without;

        let units: Vec<(Entity, u32, bool)> = self
            .game
            .world
            .query_filtered::<(Entity, &UnitType, &MoveState), Without<Dead>>()
            .iter(&self.game.world)
            .map(|(e, ut, ms)| {
                let moving = matches!(ms, MoveState::MovingTo(_));
                (e, ut.id, moving)
            })
            .collect();

        for (entity, type_id, moving) in &units {
            let bits = entity.to_bits();
            if !self.animation_driver.has_unit(bits) {
                self.animation_driver.spawn_unit(bits, *type_id);
            }
            self.animation_driver.set_moving(bits, *moving);
        }

        // Forward fire events.
        let fire_events: Vec<u64> = self
            .game
            .world
            .get_resource::<FireEventQueue>()
            .map(|q| q.events.iter().map(|e| e.shooter.to_bits()).collect())
            .unwrap_or_default();
        for bits in fire_events {
            self.animation_driver.fire(bits);
        }

        self.animation_driver.tick();
    }

    /// Generate animated meshes and register with the renderer.
    fn update_animated_meshes(&mut self) -> BTreeMap<u64, u32> {
        const ANIMATED_MESH_BASE: u32 = 1_000_000;
        let scale = 0.2;
        let mut mesh_ids = BTreeMap::new();

        let renderer = match self.renderer.as_mut() {
            Some(r) => r,
            None => return mesh_ids,
        };

        let units: Vec<(u64, u32)> = self
            .game
            .world
            .query_filtered::<(Entity, &UnitType), bevy_ecs::query::Without<Dead>>()
            .iter(&self.game.world)
            .filter(|(e, _)| self.animation_driver.has_unit(e.to_bits()))
            .map(|(e, ut)| (e.to_bits(), ut.id))
            .collect();

        for (bits, type_id) in units {
            let tree = match self.piece_trees.get(&type_id) {
                Some(t) => t,
                None => continue,
            };
            if let Some((mut verts, indices)) = self.animation_driver.generate_animated_mesh(bits, tree) {
                for v in &mut verts {
                    let (x, z) = (v.position[0], v.position[2]);
                    v.position[0] = z * scale;
                    v.position[1] *= scale;
                    v.position[2] = -x * scale;
                    let (nx, nz) = (v.normal[0], v.normal[2]);
                    v.normal[0] = nz;
                    v.normal[2] = -nx;
                }
                let mesh_id = ANIMATED_MESH_BASE + (bits as u32);
                renderer.register_unit_mesh(mesh_id, &verts, &indices);
                mesh_ids.insert(bits, mesh_id);
            }
        }

        mesh_ids
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("Pierce RTS starting...");

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run");
}
