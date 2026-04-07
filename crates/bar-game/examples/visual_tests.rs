//! Visual test runner — renders headless test scenarios sequentially in a window.
//!
//! Run with: `cargo run -p bar-game --example visual_tests`
//!
//! Each test runs for a fixed number of frames, renders every frame,
//! then asserts its result and moves to the next test.

use std::path::Path;
use std::sync::Arc;

use bevy_ecs::entity::Entity;
use bevy_ecs::query::Without;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::ElementState;
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

use bar_game_lib::building::{PlacementType, BUILDING_FACTORY_ID, BUILDING_SOLAR_ID};
use bar_game_lib::GameState;

const BAR_UNITS_PATH: &str = "../Beyond-All-Reason-Sandbox/units";
const MAP_MANIFEST_PATH: &str = "assets/maps/small_duel/manifest.ron";

// ---------------------------------------------------------------------------
// Test definition
// ---------------------------------------------------------------------------

type SetupFn = Box<dyn FnOnce() -> GameState>;
type StepFn = Box<dyn FnMut(&mut GameState, u64)>;
type CheckFn = Box<dyn FnOnce(&mut GameState) -> (bool, String)>;

struct VisualTest {
    name: &'static str,
    frames: u64,
    setup: SetupFn,
    step: StepFn,
    check: CheckFn,
}

fn make_game() -> GameState {
    GameState::new(Path::new(BAR_UNITS_PATH), Path::new(MAP_MANIFEST_PATH))
}

fn fund(game: &mut GameState) {
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
// Test catalogue
// ---------------------------------------------------------------------------

fn all_tests() -> Vec<VisualTest> {
    vec![
        // --- 1. Select and move commander ---
        VisualTest {
            name: "select_and_move",
            frames: 200,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                g
            }),
            step: {
                let mut cmd_entity: Option<Entity> = None;
                Box::new(move |game, frame| {
                    if frame == 0 {
                        let cmd = game.commander_team0.unwrap();
                        cmd_entity = Some(cmd);
                        let pos = game.world.get::<Position>(cmd).unwrap().pos;
                        game.click_select(pos.x.to_f32(), pos.z.to_f32(), 20.0);
                    }
                    if frame == 10 {
                        let cmd = cmd_entity.unwrap();
                        let pos = game.world.get::<Position>(cmd).unwrap().pos;
                        game.click_move(pos.x.to_f32() + 50.0, pos.z.to_f32() + 30.0);
                    }
                    game.tick();
                    game.frame_count += 1;
                })
            },
            check: Box::new(|game| {
                let cmd = game.commander_team0.unwrap();
                let pos = game.world.get::<Position>(cmd).unwrap().pos;
                let moved = pos.x.to_f32() > 210.0 || pos.z.to_f32() > 210.0;
                (moved, format!("Commander at ({:.0},{:.0})", pos.x.to_f32(), pos.z.to_f32()))
            }),
        },
        // --- 2. Place solar building ---
        VisualTest {
            name: "place_solar",
            frames: 300,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                g
            }),
            step: Box::new(|game, frame| {
                if frame == 10 {
                    let cmd = game.commander_team0.unwrap();
                    game.selection.select_single(cmd);
                    game.handle_build_command(PlacementType(BUILDING_SOLAR_ID));
                    let pos = game.world.get::<Position>(cmd).unwrap().pos;
                    game.handle_place(pos.x.to_f32() + 15.0, pos.z.to_f32());
                }
                game.tick();
                game.frame_count += 1;
            }),
            check: Box::new(|game| {
                let sites: usize = game.world.query::<&BuildSite>().iter(&game.world).count();
                let producers: usize = game.world
                    .query::<&recoil_sim::economy::ResourceProducer>()
                    .iter(&game.world)
                    .count();
                // Either still building or completed
                let ok = sites > 0 || producers > 1; // commander is 1 producer
                (ok, format!("sites={} producers={}", sites, producers))
            }),
        },
        // --- 3. Factory production ---
        VisualTest {
            name: "factory_production",
            frames: 500,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                g
            }),
            step: Box::new(|game, frame| {
                if frame == 10 {
                    let cmd = game.commander_team0.unwrap();
                    game.selection.select_single(cmd);
                    game.handle_build_command(PlacementType(BUILDING_FACTORY_ID));
                    let pos = game.world.get::<Position>(cmd).unwrap().pos;
                    game.handle_place(pos.x.to_f32() + 40.0, pos.z.to_f32());
                }
                game.tick();
                game.frame_count += 1;
            }),
            check: Box::new(|game| {
                let t0_count: usize = game.world
                    .query_filtered::<&Allegiance, Without<Dead>>()
                    .iter(&game.world)
                    .filter(|a| a.team == 0)
                    .count();
                // Should have commander + at least a building
                (t0_count >= 2, format!("team0 alive={}", t0_count))
            }),
        },
        // --- 4. AI builds and produces ---
        VisualTest {
            name: "ai_builds_army",
            frames: 1500,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                g
            }),
            step: Box::new(|game, _frame| {
                game.tick();
                game.frame_count += 1;
            }),
            check: Box::new(|game| {
                let t1_count: usize = game.world
                    .query_filtered::<&Allegiance, Without<Dead>>()
                    .iter(&game.world)
                    .filter(|a| a.team == 1)
                    .count();
                (t1_count > 1, format!("team1 alive={}", t1_count))
            }),
        },
        // --- 5. Bot vs bot full battle ---
        VisualTest {
            name: "bot_vs_bot",
            frames: 3000,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                g
            }),
            step: {
                let mut ai0: Option<bar_game_lib::ai::AiState> = None;
                Box::new(move |game, frame| {
                    if frame == 0 {
                        ai0 = Some(bar_game_lib::ai::AiState::new(
                            99, 0, 1, game.commander_team0, game.commander_team1,
                        ));
                    }
                    game.tick();
                    if let Some(ref mut ai) = ai0 {
                        bar_game_lib::ai::ai_tick(&mut game.world, ai, game.frame_count);
                    }
                    game.frame_count += 1;
                })
            },
            check: Box::new(|game| {
                let total: usize = game.world
                    .query_filtered::<&Allegiance, Without<Dead>>()
                    .iter(&game.world)
                    .count();
                let over = game.is_game_over();
                (total > 0, format!("alive={} game_over={}", total, over))
            }),
        },
        // --- 6. Win condition ---
        VisualTest {
            name: "win_condition",
            frames: 100,
            setup: Box::new(|| {
                let mut g = make_game();
                fund(&mut g);
                // Kill team 1 commander immediately
                if let Some(cmd1) = g.commander_team1 {
                    if let Some(mut hp) = g.world.get_mut::<Health>(cmd1) {
                        hp.current = SimFloat::ZERO;
                    }
                }
                g
            }),
            step: Box::new(|game, _frame| {
                game.tick();
                game.frame_count += 1;
            }),
            check: Box::new(|game| {
                let over = game.is_game_over();
                let winner = game.game_over.as_ref().map(|go| go.winner);
                (over && winner == Some(Some(0)), format!("over={} winner={:?}", over, winner))
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Instance extraction
// ---------------------------------------------------------------------------

fn extract_units(game: &mut GameState) -> Vec<UnitInstance> {
    let sel = game.selected();
    let mut out: Vec<UnitInstance> = game.world
        .query_filtered::<(Entity, &Position, &Heading, &Allegiance, &Health), Without<Dead>>()
        .iter(&game.world)
        .map(|(e, pos, hd, al, hp)| {
            let mut c = if al.team == 0 { [0.2, 0.5, 0.9] } else { [0.9, 0.2, 0.2] };
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f; c[1] *= f; c[2] *= f;
            if sel == Some(e) { c[0] = (c[0]+0.3).min(1.0); c[1] = (c[1]+0.3).min(1.0); c[2] = (c[2]+0.3).min(1.0); }
            UnitInstance { position: [pos.pos.x.to_f32(), 0.0, pos.pos.z.to_f32()], heading: hd.angle.to_f32(), team_color: c, _pad: 0.0 }
        })
        .collect();

    // Buildings (no Heading).
    let buildings: Vec<UnitInstance> = game.world
        .query_filtered::<(Entity, &Position, &Allegiance, &Health), (Without<Dead>, Without<Heading>)>()
        .iter(&game.world)
        .map(|(e, pos, al, hp)| {
            let mut c = if al.team == 0 { [0.1, 0.8, 0.3] } else { [0.8, 0.1, 0.3] };
            let f = (hp.current.to_f32() / hp.max.to_f32().max(1.0)).clamp(0.2, 1.0);
            c[0] *= f; c[1] *= f; c[2] *= f;
            if sel == Some(e) { c[0] = (c[0]+0.3).min(1.0); c[1] = (c[1]+0.3).min(1.0); c[2] = (c[2]+0.3).min(1.0); }
            UnitInstance { position: [pos.pos.x.to_f32(), 0.0, pos.pos.z.to_f32()], heading: 0.0, team_color: c, _pad: 0.0 }
        })
        .collect();

    out.extend(buildings);
    out
}

fn extract_projectiles(game: &mut GameState) -> Vec<ProjectileInstance> {
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
// App
// ---------------------------------------------------------------------------

struct App {
    tests: Vec<VisualTest>,
    current_idx: usize,
    game: Option<GameState>,
    current_step: Option<StepFn>,
    current_check: Option<CheckFn>,
    current_name: &'static str,
    current_max_frames: u64,
    frame: u64,
    results: Vec<(&'static str, bool, String)>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    window_size: [f32; 2],
    done: bool,
}

impl App {
    fn new() -> Self {
        Self {
            tests: all_tests(),
            current_idx: 0,
            game: None,
            current_step: None,
            current_check: None,
            current_name: "",
            current_max_frames: 0,
            frame: 0,
            results: Vec::new(),
            window: None,
            renderer: None,
            window_size: [1280.0, 720.0],
            done: false,
        }
    }

    fn start_next_test(&mut self) {
        if self.current_idx >= self.tests.len() {
            self.done = true;
            self.print_results();
            return;
        }

        let test = self.tests.remove(0);
        self.current_name = test.name;
        self.current_max_frames = test.frames;
        self.frame = 0;
        self.game = Some((test.setup)());
        self.current_step = Some(test.step);
        self.current_check = Some(test.check);
        self.current_idx += 1;

        eprintln!("\n--- [{}/{}] {} ({} frames) ---",
            self.current_idx, self.current_idx + self.tests.len(),
            self.current_name, self.current_max_frames);

        if let Some(window) = &self.window {
            window.set_title(&format!("Visual Test [{}/{}]: {}",
                self.current_idx, self.current_idx + self.tests.len(), self.current_name));
        }
    }

    fn finish_current_test(&mut self) {
        let mut game = self.game.take().unwrap();
        let check = self.current_check.take().unwrap();
        let (passed, msg) = check(&mut game);
        let status = if passed { "PASS" } else { "FAIL" };
        eprintln!("  {} {} — {}", status, self.current_name, msg);
        self.results.push((self.current_name, passed, msg));
        self.current_step = None;
    }

    fn print_results(&self) {
        eprintln!("\n========== RESULTS ==========");
        let mut pass = 0;
        let mut fail = 0;
        for (name, passed, msg) in &self.results {
            let tag = if *passed { "PASS" } else { "FAIL" };
            eprintln!("  [{}] {} — {}", tag, name, msg);
            if *passed { pass += 1; } else { fail += 1; }
        }
        eprintln!("=============================");
        eprintln!("{} passed, {} failed", pass, fail);
        if fail > 0 {
            eprintln!("SOME TESTS FAILED");
        } else {
            eprintln!("ALL TESTS PASSED");
        }
    }

    fn camera(&self) -> Camera {
        // Center on the action: follow team 0 commander or map center.
        let (cx, cz) = self.game.as_ref()
            .and_then(|g| g.commander_team0)
            .and_then(|e| self.game.as_ref().unwrap().world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()))
            .unwrap_or((512.0, 512.0));
        let aspect = self.window_size[0] / self.window_size[1];
        Camera {
            eye: [cx, 300.0, cz + 225.0],
            target: [cx, 0.0, cz],
            up: [0.0, 1.0, 0.0],
            fov_y: std::f32::consts::FRAC_PI_4,
            aspect, near: 1.0, far: 2000.0,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("Visual Tests")
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let renderer = pollster::block_on(Renderer::new(Arc::clone(&window))).expect("renderer");
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.start_next_test();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: winit::event::WindowEvent) {
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(size) => {
                self.window_size = [size.width as f32, size.height as f32];
                if let Some(r) = self.renderer.as_mut() { r.resize(size.width, size.height); }
            }
            winit::event::WindowEvent::KeyboardInput {
                event: winit::event::KeyEvent {
                    physical_key: PhysicalKey::Code(KeyCode::Escape),
                    state: ElementState::Pressed, ..
                }, ..
            } => event_loop.exit(),
            winit::event::WindowEvent::RedrawRequested => {
                if self.done {
                    event_loop.exit();
                    return;
                }

                if self.game.is_none() {
                    self.start_next_test();
                    if self.done { event_loop.exit(); return; }
                }

                // Step the test.
                if self.frame < self.current_max_frames {
                    if let Some(ref mut step) = self.current_step {
                        step(self.game.as_mut().unwrap(), self.frame);
                    }
                    self.frame += 1;
                } else {
                    // Test finished — check and move on.
                    self.finish_current_test();
                    self.start_next_test();
                    if self.done { event_loop.exit(); return; }
                }

                // Render.
                let cam = self.camera();
                if let (Some(game), Some(renderer)) = (self.game.as_mut(), self.renderer.as_mut()) {
                    let units = extract_units(game);
                    let projs = extract_projectiles(game);
                    renderer.update_camera(&cam);
                    renderer.update_units(&units);
                    renderer.update_projectiles(&projs);
                    let _ = renderer.render();
                }

                if let Some(w) = &self.window { w.request_redraw(); }
            }
            _ => {}
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    eprintln!("Visual Test Runner — runs all test scenarios sequentially with rendering.");
    eprintln!("Press Escape to abort.\n");

    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run");
}
