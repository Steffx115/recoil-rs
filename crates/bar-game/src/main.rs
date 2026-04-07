//! Recoil RTS — game binary.
//!
//! Thin shell: window, renderer, input dispatch, egui overlay.
//! All game logic lives in `bar-game-lib`.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use recoil_render::particles::ParticleSystem;
use recoil_render::unit_renderer::UnitInstance;
use recoil_render::Renderer;
use recoil_sim::unit_defs::UnitDefRegistry;
use recoil_sim::Position;

use bar_game_lib::GameState;

use egui_wgpu::ScreenDescriptor;

mod camera_controller;
mod icons;
mod input;

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
    last_frame: Instant,
    cursor_pos: [f32; 2],
    window_size: [f32; 2],
    modifiers: ModifiersState,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    fps_counter: FpsCounter,
    icon_atlas: Option<IconAtlas>,
}

impl App {
    fn new() -> Self {
        let game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
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
            last_frame: Instant::now(),
            cursor_pos: [0.0; 2],
            window_size: [1280.0, 720.0],
            modifiers: ModifiersState::empty(),
            egui_state: None,
            egui_renderer: None,
            fps_counter: FpsCounter::new(),
            icon_atlas: None,
        }
    }

    fn screen_to_ground(&self) -> Option<(f32, f32)> {
        let cam = self
            .camera_ctrl
            .camera(self.window_size[0] / self.window_size[1]);
        let vp = cam.view_projection();
        let inv_vp = mat4_inverse(vp)?;
        recoil_sim::selection::screen_to_ground_raw(
            self.cursor_pos[0],
            self.cursor_pos[1],
            self.window_size[0],
            self.window_size[1],
            &inv_vp,
        )
    }

    fn load_models(renderer: &mut Renderer, registry: &UnitDefRegistry) {
        let bar_models_dir = Path::new("../Beyond-All-Reason-Sandbox/objects3d/Units");
        if !bar_models_dir.exists() {
            return;
        }
        let model_entries: Vec<(u32, String)> = registry
            .defs
            .values()
            .filter_map(|def| {
                def.model_path
                    .as_ref()
                    .map(|p| (def.unit_type_id, p.clone()))
            })
            .collect();

        let scale = 0.2;
        let mut loaded = 0;
        for (type_id, model_path) in &model_entries {
            let filename = model_path.strip_prefix("Units/").unwrap_or(model_path);
            let s3o_path = bar_models_dir.join(filename);
            if !s3o_path.exists() {
                continue;
            }
            if let Ok((mut verts, indices)) = recoil_render::load_s3o_file(&s3o_path) {
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
        if let Some((_, ref first_path)) = model_entries.first() {
            let filename = first_path.strip_prefix("Units/").unwrap_or(first_path);
            let s3o_path = bar_models_dir.join(filename);
            if let Ok((mut verts, indices)) = recoil_render::load_s3o_file(&s3o_path) {
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
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let mut renderer =
            pollster::block_on(Renderer::new(Arc::clone(&window))).expect("renderer");

        let registry = self.game.world.resource::<UnitDefRegistry>();
        Self::load_models(&mut renderer, registry);

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

                // Gather render data
                let mut instances = unit_instances(&mut self.game);
                instances.extend(building_instances(&mut self.game));
                if let Some(ref pt) = self.game.placement_mode {
                    if let Some((gx, gz)) = self.screen_to_ground() {
                        instances.push(UnitInstance {
                            position: [gx, 0.0, gz],
                            heading: 0.0,
                            team_color: [0.3, 0.9, 0.3],
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
        proj: &[recoil_render::projectile_renderer::ProjectileInstance],
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
