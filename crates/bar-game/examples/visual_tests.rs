//! Visual test runner — renders headless test scenarios in a window.
//!
//! Run with: `cargo run -p bar-game --example visual_tests -- [scenario]`
//!
//! Available scenarios:
//!   bot_vs_bot     — Two AIs fight (default)
//!   build_solar    — Player builds a solar collector
//!   factory_queue  — Player builds factory, queues units
//!   full_game      — Full game: build, produce, fight
//!   select_move    — Select commander and move it

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use bevy_ecs::query::Without;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use recoil_math::SimFloat;
use recoil_render::camera::Camera;
use recoil_render::projectile_renderer::ProjectileInstance;
use recoil_render::unit_renderer::UnitInstance;
use recoil_render::Renderer;
use recoil_sim::construction::BuildSite;
use recoil_sim::economy::EconomyState;
use recoil_sim::{Allegiance, Dead, Heading, Health, Position, Velocity};

use bar_game_lib::building::{PlacementType, BUILDING_SOLAR_ID, BUILDING_FACTORY_ID};
use bar_game_lib::GameState;

const BAR_UNITS_PATH: &str = "../Beyond-All-Reason-Sandbox/units";
const MAP_MANIFEST_PATH: &str = "assets/maps/small_duel/manifest.ron";

// ---------------------------------------------------------------------------
// Scenarios — each returns a GameState and a step function
// ---------------------------------------------------------------------------

type StepFn = Box<dyn FnMut(&mut GameState, u64)>;

fn scenario_bot_vs_bot() -> (GameState, StepFn) {
    let mut game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
    fund_both_teams(&mut game);

    // Create a second AI for team 0.
    let mut ai0 = bar_game_lib::ai::AiState::new(
        99, 0, 1, game.commander_team0, game.commander_team1,
    );

    let step: StepFn = Box::new(move |game, _frame| {
        game.tick();
        bar_game_lib::ai::ai_tick(&mut game.world, &mut ai0, game.frame_count);
        game.frame_count += 1;
    });

    (game, step)
}

fn scenario_build_solar() -> (GameState, StepFn) {
    let mut game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
    fund_both_teams(&mut game);

    let cmd = game.commander_team0.unwrap();
    game.selected = Some(cmd);

    // Place a solar near the commander on frame 30.
    let step: StepFn = Box::new(move |game, frame| {
        if frame == 30 {
            game.handle_build_command(PlacementType(BUILDING_SOLAR_ID));
            let pos = game.world.get::<Position>(cmd).unwrap().pos;
            game.handle_place(pos.x.to_f32() + 15.0, pos.z.to_f32());
            eprintln!("[frame {}] Placed solar", frame);
        }
        game.tick();
        game.frame_count += 1;
    });

    (game, step)
}

fn scenario_factory_queue() -> (GameState, StepFn) {
    let mut game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
    fund_both_teams(&mut game);

    let cmd = game.commander_team0.unwrap();
    game.selected = Some(cmd);

    let step: StepFn = Box::new(move |game, frame| {
        // Build factory at frame 30.
        if frame == 30 {
            game.handle_build_command(PlacementType(BUILDING_FACTORY_ID));
            let pos = game.world.get::<Position>(cmd).unwrap().pos;
            game.handle_place(pos.x.to_f32() + 40.0, pos.z.to_f32());
            eprintln!("[frame {}] Placed factory", frame);
        }
        game.tick();
        game.frame_count += 1;
    });

    (game, step)
}

fn scenario_full_game() -> (GameState, StepFn) {
    let mut game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
    fund_both_teams(&mut game);

    let cmd = game.commander_team0.unwrap();
    game.selected = Some(cmd);

    let step: StepFn = Box::new(move |game, frame| {
        // Build solar at frame 30.
        if frame == 30 {
            game.handle_build_command(PlacementType(BUILDING_SOLAR_ID));
            let pos = game.world.get::<Position>(cmd).unwrap().pos;
            game.handle_place(pos.x.to_f32() + 15.0, pos.z.to_f32());
            eprintln!("[frame {}] Placed solar", frame);
        }
        // Build factory at frame 60.
        if frame == 60 {
            game.handle_build_command(PlacementType(BUILDING_FACTORY_ID));
            let pos = game.world.get::<Position>(cmd).unwrap().pos;
            game.handle_place(pos.x.to_f32() + 40.0, pos.z.to_f32());
            eprintln!("[frame {}] Placed factory", frame);
        }
        game.tick();
        game.frame_count += 1;

        if game.is_game_over() && frame % 300 == 0 {
            let go = game.game_over.as_ref().unwrap();
            eprintln!(
                "[frame {}] GAME OVER: winner={:?} reason={}",
                frame, go.winner, go.reason
            );
        }
    });

    (game, step)
}

fn scenario_select_move() -> (GameState, StepFn) {
    let mut game = GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH));
    fund_both_teams(&mut game);

    let cmd = game.commander_team0.unwrap();
    let cmd_pos = game.world.get::<Position>(cmd).unwrap().pos;
    let cx = cmd_pos.x.to_f32();
    let cz = cmd_pos.z.to_f32();

    let step: StepFn = Box::new(move |game, frame| {
        if frame == 10 {
            game.click_select(cx, cz, 20.0);
            eprintln!("[frame {}] Selected commander", frame);
        }
        if frame == 30 {
            game.click_move(cx + 100.0, cz + 50.0);
            eprintln!("[frame {}] Move command to ({:.0}, {:.0})", frame, cx + 100.0, cz + 50.0);
        }
        if frame == 300 {
            game.click_move(cx, cz);
            eprintln!("[frame {}] Move back to start", frame);
        }
        game.tick();
        game.frame_count += 1;
    });

    (game, step)
}

fn fund_both_teams(game: &mut GameState) {
    let mut economy = game.world.resource_mut::<EconomyState>();
    for team in [0u8, 1] {
        if let Some(res) = economy.teams.get_mut(&team) {
            res.metal = SimFloat::from_int(50000);
            res.energy = SimFloat::from_int(100000);
            res.metal_storage = SimFloat::from_int(100000);
            res.energy_storage = SimFloat::from_int(200000);
        }
    }
}

// ---------------------------------------------------------------------------
// Instance extraction from GameState
// ---------------------------------------------------------------------------

fn extract_unit_instances(game: &mut GameState) -> Vec<UnitInstance> {
    let selected = game.selected;
    game.world
        .query_filtered::<(
            bevy_ecs::entity::Entity,
            &Position,
            &Heading,
            &Allegiance,
            &Health,
            Option<&recoil_sim::components::Stunned>,
            Option<&BuildSite>,
        ), Without<Dead>>()
        .iter(&game.world)
        .map(|(entity, pos, heading, allegiance, health, stunned, build_site)| {
            let mut color = if allegiance.team == 0 {
                [0.2f32, 0.5, 0.9]
            } else {
                [0.9f32, 0.2, 0.2]
            };
            if build_site.is_some() {
                color = [color[0] * 0.5, color[1] * 0.5, color[2] * 0.5];
            }
            let hp_frac = if health.max > SimFloat::ZERO {
                (health.current.to_f32() / health.max.to_f32()).clamp(0.2, 1.0)
            } else {
                1.0
            };
            color[0] *= hp_frac;
            color[1] *= hp_frac;
            color[2] *= hp_frac;
            if stunned.is_some() {
                color[2] = 0.8;
            }
            if selected == Some(entity) {
                color[0] = (color[0] + 0.3).min(1.0);
                color[1] = (color[1] + 0.3).min(1.0);
                color[2] = (color[2] + 0.3).min(1.0);
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

fn extract_building_instances(game: &mut GameState) -> Vec<UnitInstance> {
    let selected = game.selected;
    game.world
        .query_filtered::<(
            bevy_ecs::entity::Entity,
            &Position,
            &Allegiance,
            &Health,
            Option<&BuildSite>,
        ), (Without<Dead>, Without<Heading>)>()
        .iter(&game.world)
        .map(|(entity, pos, allegiance, health, build_site)| {
            let mut color = if allegiance.team == 0 {
                [0.1f32, 0.8, 0.3]
            } else {
                [0.8f32, 0.1, 0.3]
            };
            if build_site.is_some() {
                color[0] *= 0.5;
                color[1] *= 0.5;
                color[2] *= 0.5;
            }
            let hp_frac = if health.max > SimFloat::ZERO {
                (health.current.to_f32() / health.max.to_f32()).clamp(0.2, 1.0)
            } else {
                1.0
            };
            color[0] *= hp_frac;
            color[1] *= hp_frac;
            color[2] *= hp_frac;
            if selected == Some(entity) {
                color[0] = (color[0] + 0.3).min(1.0);
                color[1] = (color[1] + 0.3).min(1.0);
                color[2] = (color[2] + 0.3).min(1.0);
            }
            UnitInstance {
                position: [pos.pos.x.to_f32(), pos.pos.y.to_f32(), pos.pos.z.to_f32()],
                heading: 0.0,
                team_color: color,
                _pad: 0.0,
            }
        })
        .collect()
}

fn extract_projectile_instances(game: &mut GameState) -> Vec<ProjectileInstance> {
    use recoil_sim::projectile::Projectile;
    game.world
        .query::<(&Position, &Velocity, &Projectile)>()
        .iter(&game.world)
        .map(|(pos, vel, _)| {
            ProjectileInstance {
                position: [pos.pos.x.to_f32(), pos.pos.y.to_f32() + 2.0, pos.pos.z.to_f32()],
                size: 2.0,
                velocity_dir: [vel.vel.x.to_f32(), vel.vel.y.to_f32(), vel.vel.z.to_f32()],
                _pad: 0.0,
                color: [1.0, 0.8, 0.2],
                _pad2: 0.0,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct VisualTestApp {
    game: GameState,
    step: StepFn,
    frame: u64,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    window_size: [f32; 2],
    camera_center: [f32; 2],
    camera_height: f32,
    last_frame: Instant,
    scenario_name: String,
}

impl VisualTestApp {
    fn new(scenario: &str) -> Self {
        let (game, step) = match scenario {
            "build_solar" => scenario_build_solar(),
            "factory_queue" => scenario_factory_queue(),
            "full_game" => scenario_full_game(),
            "select_move" => scenario_select_move(),
            _ => scenario_bot_vs_bot(),
        };

        // Center camera on team 0 commander.
        let (cx, cz) = game
            .commander_team0
            .and_then(|e| game.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()))
            .unwrap_or((512.0, 512.0));

        Self {
            game,
            step,
            frame: 0,
            window: None,
            renderer: None,
            window_size: [1280.0, 720.0],
            camera_center: [cx, cz],
            camera_height: 300.0,
            last_frame: Instant::now(),
            scenario_name: scenario.to_string(),
        }
    }

}

impl ApplicationHandler for VisualTestApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let title = format!("Visual Test: {}", self.scenario_name);
        let attrs = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window)))
            .expect("create renderer");

        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.window_size = [size.width as f32, size.height as f32];
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput {
                event: winit::event::KeyEvent {
                    physical_key: PhysicalKey::Code(key),
                    state: ElementState::Pressed,
                    ..
                },
                ..
            } => {
                if key == KeyCode::Escape {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32();
                self.last_frame = now;

                // Compute camera from copied values to avoid borrow issues.
                let aspect = self.window_size[0] / self.window_size[1];
                let cc = self.camera_center;
                let ch = self.camera_height;
                let cam = Camera {
                    eye: [cc[0], ch, cc[1] + ch * 0.75],
                    target: [cc[0], 0.0, cc[1]],
                    up: [0.0, 1.0, 0.0],
                    fov_y: std::f32::consts::FRAC_PI_4,
                    aspect,
                    near: 1.0,
                    far: 2000.0,
                };

                // Run sim step (target ~30fps sim speed).
                if dt < 0.1 {
                    (self.step)(&mut self.game, self.frame);
                    self.frame += 1;
                }

                // Gather instances.
                let mut instances = extract_unit_instances(&mut self.game);
                instances.extend(extract_building_instances(&mut self.game));
                let proj_instances = extract_projectile_instances(&mut self.game);

                // Render.
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.update_camera(&cam);
                    renderer.update_units(&instances);
                    renderer.update_projectiles(&proj_instances);
                    let _ = renderer.render();
                }

                // Log progress periodically.
                if self.frame.is_multiple_of(300) {
                    let alive: usize = self.game.world
                        .query_filtered::<&Allegiance, Without<Dead>>()
                        .iter(&self.game.world)
                        .count();
                    eprintln!(
                        "[frame {}] {} alive entities, game_over={}",
                        self.frame,
                        alive,
                        self.game.is_game_over()
                    );
                }

                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let scenario = std::env::args().nth(1).unwrap_or_else(|| "bot_vs_bot".into());

    eprintln!("Visual Test Runner — scenario: {}", scenario);
    eprintln!("Available: bot_vs_bot, build_solar, factory_queue, full_game, select_move");
    eprintln!("Press Escape to exit.");

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = VisualTestApp::new(&scenario);
    event_loop.run_app(&mut app).expect("run");
}
