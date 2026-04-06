use std::collections::VecDeque;
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
use recoil_sim::combat_data::{ArmorClass, WeaponInstance, WeaponSet};
use recoil_sim::commands::CommandQueue;
use recoil_sim::components::Stunned;
use recoil_sim::construction::{BuildSite, BuildTarget, Builder, Reclaimable};
use recoil_sim::economy::{init_economy, EconomyState, ResourceProducer};
use recoil_sim::factory::{BuildQueue, UnitBlueprint, UnitRegistry};
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

const SELECT_RADIUS_SQ: f32 = 400.0; // 20^2

// Camera movement
const PAN_SPEED: f32 = 5.0;
const ZOOM_SPEED: f32 = 10.0;
const MIN_HEIGHT: f32 = 50.0;
const MAX_HEIGHT: f32 = 800.0;

// Building costs
const SOLAR_METAL_COST: i32 = 100;
const SOLAR_ENERGY_COST: i32 = 400;
const SOLAR_BUILD_TIME: i32 = 600;

const MEX_METAL_COST: i32 = 50;
const MEX_ENERGY_COST: i32 = 500;
const MEX_BUILD_TIME: i32 = 600;

const FACTORY_METAL_COST: i32 = 650;
const FACTORY_ENERGY_COST: i32 = 2800;
const FACTORY_BUILD_TIME: i32 = 1800;

// Synthetic UnitType IDs for buildings (high range to avoid collision with real defs)
const BUILDING_SOLAR_ID: u32 = 50000;
const BUILDING_MEX_ID: u32 = 50001;
const BUILDING_FACTORY_ID: u32 = 50002;

// AI
const AI_TICK_INTERVAL: u64 = 300;

// ---------------------------------------------------------------------------
// Placement mode
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlacementType {
    Solar,
    Mex,
    Factory,
}

impl PlacementType {
    fn label(self) -> &'static str {
        match self {
            PlacementType::Solar => "Build Solar",
            PlacementType::Mex => "Build Mex",
            PlacementType::Factory => "Build Factory",
        }
    }
}

// ---------------------------------------------------------------------------
// Camera controller
// ---------------------------------------------------------------------------

struct CameraController {
    center: [f32; 2],
    height: f32,
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
            KeyCode::KeyD => self.right = pressed,
            _ => {}
        }
    }

    fn process_scroll(&mut self, delta: f32) {
        self.height = (self.height - delta * ZOOM_SPEED).clamp(MIN_HEIGHT, MAX_HEIGHT);
    }

    fn update(&mut self) {
        let speed = PAN_SPEED * (self.height / 400.0);
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
// Simulation state
// ---------------------------------------------------------------------------

struct SimState {
    world: World,
    paused: bool,
    frame_count: u64,
    selected: Option<Entity>,
    rng_seed: u64,
    // Game state
    placement_mode: Option<PlacementType>,
    ai_rng: Lcg,
    // Track commander entities per team
    commander_team0: Option<Entity>,
    commander_team1: Option<Entity>,
    // Track factory entities per team (first factory only for AI)
    factory_team1: Option<Entity>,
    // Weapon def IDs cache (for equipping factory-spawned units)
    weapon_def_ids: std::collections::BTreeMap<u32, Vec<u32>>,
    // Map metal spots for Mex placement
    metal_spots: Vec<(f64, f64)>,
}

impl SimState {
    fn new() -> Self {
        let mut state = Self {
            world: World::new(),
            paused: false,
            frame_count: 0,
            selected: None,
            rng_seed: 12345,
            placement_mode: None,
            ai_rng: Lcg::new(42),
            commander_team0: None,
            commander_team1: None,
            factory_team1: None,
            weapon_def_ids: std::collections::BTreeMap::new(),
            metal_spots: Vec::new(),
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.world = World::new();
        self.selected = None;
        self.frame_count = 0;
        self.placement_mode = None;
        self.commander_team0 = None;
        self.commander_team1 = None;
        self.factory_team1 = None;

        sim_runner::init_sim_world(&mut self.world);

        // Load unit defs
        let bar_units = Path::new("../Beyond-All-Reason-Sandbox/units");
        let mut unit_def_registry = UnitDefRegistry::default();
        if bar_units.exists() {
            let bar_dirs = [
                "ArmBots",
                "ArmVehicles",
                "ArmBuildings",
                "ArmAircraft",
                "CorBots",
                "CorVehicles",
                "CorBuildings",
                "CorAircraft",
            ];
            for dir in &bar_dirs {
                let path = bar_units.join(dir);
                if path.exists() {
                    if let Ok(reg) = recoil_sim::lua_unitdefs::load_bar_unitdefs_directory(&path) {
                        for (_id, def) in reg.defs {
                            unit_def_registry.register(def);
                        }
                    }
                }
            }
            for e in std::fs::read_dir(bar_units).into_iter().flatten().flatten() {
                if e.path().extension().is_some_and(|ext| ext == "lua") {
                    if let Ok(def) = recoil_sim::lua_unitdefs::load_bar_unitdef(&e.path()) {
                        unit_def_registry.register(def);
                    }
                }
            }
            tracing::info!(
                "Loaded {} BAR unit defs from Lua",
                unit_def_registry.defs.len()
            );
        } else {
            unit_def_registry =
                UnitDefRegistry::load_directory(Path::new("assets/unitdefs/armada"))
                    .unwrap_or_default();
            if let Ok(cortex) = UnitDefRegistry::load_directory(Path::new("assets/unitdefs/cortex"))
            {
                for (_id, def) in cortex.defs {
                    unit_def_registry.register(def);
                }
            }
            tracing::info!(
                "Loaded {} RON unit defs (BAR repo not found)",
                unit_def_registry.defs.len()
            );
        }

        // Register weapon defs
        self.weapon_def_ids.clear();
        {
            let mut registry = self.world.resource_mut::<WeaponRegistry>();
            for (unit_type_id, unit_def) in &unit_def_registry.defs {
                let mut ids = Vec::new();
                for weapon_def in unit_def.to_weapon_defs() {
                    let id = registry.defs.len() as u32;
                    registry.defs.push(weapon_def);
                    ids.push(id);
                }
                self.weapon_def_ids.insert(*unit_type_id, ids);
            }
        }

        // Load map manifest
        let map_manifest = load_map_manifest(Path::new("assets/maps/small_duel/manifest.ron")).ok();
        if let Some(ref manifest) = map_manifest {
            tracing::info!(
                "Loaded map '{}' with {} start positions, {} metal spots",
                manifest.name,
                manifest.start_positions.len(),
                manifest.metal_spots.len(),
            );
            self.metal_spots = manifest.metal_spots.iter().map(|ms| (ms.x, ms.z)).collect();
        }

        init_economy(&mut self.world, &[0, 1]);

        // Fog of War
        let fog = FogOfWar::new(64, 64, &[0, 1]);
        self.world.insert_resource(fog);

        // Build UnitRegistry for factory_system from loaded UnitDefs
        let mut unit_registry = UnitRegistry::default();
        for def in unit_def_registry.defs.values() {
            unit_registry.blueprints.push(UnitBlueprint {
                unit_type_id: def.unit_type_id,
                metal_cost: SimFloat::from_f64(def.metal_cost),
                energy_cost: SimFloat::from_f64(def.energy_cost),
                build_time: if def.build_time > 0 {
                    def.build_time
                } else {
                    60
                },
                max_health: SimFloat::from_f64(def.max_health),
            });
        }
        self.world.insert_resource(unit_registry);

        // Determine start positions
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
            start_pos_0 = (200.0, 200.0);
            start_pos_1 = (824.0, 824.0);
        }

        // Spawn Commander for team 0
        self.commander_team0 = Some(self.spawn_commander(&unit_def_registry, start_pos_0, 0));

        // Spawn Commander for team 1
        self.commander_team1 = Some(self.spawn_commander(&unit_def_registry, start_pos_1, 1));

        // Store unit def registry
        self.world.insert_resource(unit_def_registry);

        self.rng_seed = self.rng_seed.wrapping_add(7);
    }

    fn spawn_commander(
        &mut self,
        unit_def_registry: &UnitDefRegistry,
        pos: (f32, f32),
        team: u8,
    ) -> Entity {
        let cmd_name = if team == 0 { "armcom" } else { "corcom" };
        let found_def = unit_def_registry
            .defs
            .values()
            .find(|d| d.name.to_lowercase() == cmd_name);
        let (hp, max_speed, accel, turn_rate, collision_r, sight_r, armor_class, unit_type_id) =
            if let Some(def) = found_def {
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
            } else {
                // Fallback commander stats
                (
                    SimFloat::from_int(3000),
                    SimFloat::from_ratio(3, 2),
                    SimFloat::ONE,
                    SimFloat::PI / SimFloat::from_int(30),
                    SimFloat::from_int(12),
                    SimFloat::from_int(300),
                    ArmorClass::Heavy,
                    9999u32,
                )
            };

        let weapon_ids = self
            .weapon_def_ids
            .get(&unit_type_id)
            .cloned()
            .unwrap_or_default();

        let entity = spawn_unit(
            &mut self.world,
            Position {
                pos: SimVec3::new(
                    SimFloat::from_f32(pos.0),
                    SimFloat::ZERO,
                    SimFloat::from_f32(pos.1),
                ),
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
            // Commander is a builder
            Builder {
                build_power: SimFloat::from_int(300),
            },
            // Commander produces a small trickle of resources
            ResourceProducer {
                metal_per_tick: SimFloat::from_ratio(1, 2),
                energy_per_tick: SimFloat::from_int(20),
            },
        ));

        entity
    }

    /// Place a building: spawn BuildSite and assign commander as builder.
    fn place_building(&mut self, btype: PlacementType, x: f32, z: f32, team: u8) {
        let (metal_cost, energy_cost, build_time) = match btype {
            PlacementType::Solar => (SOLAR_METAL_COST, SOLAR_ENERGY_COST, SOLAR_BUILD_TIME),
            PlacementType::Mex => (MEX_METAL_COST, MEX_ENERGY_COST, MEX_BUILD_TIME),
            PlacementType::Factory => (FACTORY_METAL_COST, FACTORY_ENERGY_COST, FACTORY_BUILD_TIME),
        };

        // Check if team can afford it
        let can_afford = {
            let economy = self.world.resource::<EconomyState>();
            if let Some(res) = economy.teams.get(&team) {
                res.metal >= SimFloat::from_int(metal_cost)
                    && res.energy >= SimFloat::from_int(energy_cost)
            } else {
                false
            }
        };

        if !can_afford {
            tracing::info!("Cannot afford {:?} building", btype);
            return;
        }

        let building_type_id = match btype {
            PlacementType::Solar => BUILDING_SOLAR_ID,
            PlacementType::Mex => BUILDING_MEX_ID,
            PlacementType::Factory => BUILDING_FACTORY_ID,
        };

        // Spawn the build site entity
        let build_site_entity = self
            .world
            .spawn((
                Position {
                    pos: SimVec3::new(SimFloat::from_f32(x), SimFloat::ZERO, SimFloat::from_f32(z)),
                },
                BuildSite {
                    metal_cost: SimFloat::from_int(metal_cost),
                    energy_cost: SimFloat::from_int(energy_cost),
                    total_build_time: SimFloat::from_int(build_time),
                    progress: SimFloat::ZERO,
                },
                Health {
                    current: SimFloat::from_int(1),
                    max: SimFloat::from_int(500),
                },
                Allegiance { team },
                UnitType {
                    id: building_type_id,
                },
                CollisionRadius {
                    radius: SimFloat::from_int(16),
                },
            ))
            .id();

        // Assign the commander as builder
        let commander = if team == 0 {
            self.commander_team0
        } else {
            self.commander_team1
        };

        if let Some(cmd) = commander {
            if self.world.get_entity(cmd).is_ok() {
                // Set build target
                if self.world.get::<BuildTarget>(cmd).is_some() {
                    *self.world.get_mut::<BuildTarget>(cmd).unwrap() = BuildTarget {
                        target: build_site_entity,
                    };
                } else {
                    self.world.entity_mut(cmd).insert(BuildTarget {
                        target: build_site_entity,
                    });
                }

                // Move commander toward build site
                if self.world.get::<MoveState>(cmd).is_some() {
                    *self.world.get_mut::<MoveState>(cmd).unwrap() = MoveState::MovingTo(
                        SimVec3::new(SimFloat::from_f32(x), SimFloat::ZERO, SimFloat::from_f32(z)),
                    );
                }
            }
        }

        tracing::info!("Team {} placed {:?} at ({:.0}, {:.0})", team, btype, x, z);
    }

    /// Equip newly factory-spawned units with movement and combat components.
    /// Factory system only spawns with (SimId, Position, UnitType, Allegiance, Health).
    fn equip_factory_spawned_units(&mut self) {
        // Find entities that have UnitType but no MoveState (not yet equipped)
        // and no BuildSite (not a building under construction).
        // Exclude Dead entities.
        let to_equip: Vec<(Entity, u32, u8)> = self
            .world
            .query_filtered::<(Entity, &UnitType, &Allegiance), (
                Without<MoveState>,
                Without<BuildSite>,
                Without<Dead>,
                Without<Builder>,
                Without<BuildQueue>,
                Without<ResourceProducer>,
            )>()
            .iter(&self.world)
            .filter(|(_, ut, _)| ut.id < BUILDING_SOLAR_ID) // Skip building entities
            .map(|(e, ut, a)| (e, ut.id, a.team))
            .collect();

        for (entity, unit_type_id, _team) in to_equip {
            // Look up the unit def for stats
            let stats = {
                let registry = self.world.resource::<UnitDefRegistry>();
                registry.defs.get(&unit_type_id).map(|def| {
                    (
                        SimFloat::from_f64(def.max_speed),
                        SimFloat::from_f64(def.acceleration),
                        SimFloat::from_f64(def.turn_rate),
                        SimFloat::from_f64(def.collision_radius),
                        SimFloat::from_f64(def.sight_range),
                        def.parse_armor_class(),
                    )
                })
            };

            let (max_speed, accel, turn_rate, collision_r, sight_r, armor_class) =
                stats.unwrap_or((
                    SimFloat::from_int(2),
                    SimFloat::ONE,
                    SimFloat::PI / SimFloat::from_int(30),
                    SimFloat::from_int(8),
                    SimFloat::from_int(80),
                    ArmorClass::Light,
                ));

            let weapon_ids = self
                .weapon_def_ids
                .get(&unit_type_id)
                .cloned()
                .unwrap_or_default();

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
        }
    }

    /// Queue a unit for production in a factory.
    fn queue_factory_unit(&mut self, factory_entity: Entity, unit_name: &str) {
        // Find the unit type ID by name
        let unit_type_id = {
            let registry = self.world.resource::<UnitDefRegistry>();
            registry
                .defs
                .values()
                .find(|d| d.name.to_lowercase() == unit_name)
                .map(|d| d.unit_type_id)
        };

        if let Some(type_id) = unit_type_id {
            if let Some(mut bq) = self.world.get_mut::<BuildQueue>(factory_entity) {
                bq.queue.push_back(type_id);
                tracing::info!("Queued {} (id={}) in factory", unit_name, type_id);
            }
        } else {
            tracing::warn!("Unit def '{}' not found", unit_name);
        }
    }

    /// Simple AI for team 1.
    fn ai_tick(&mut self) {
        if !self.frame_count.is_multiple_of(AI_TICK_INTERVAL) {
            return;
        }

        let cmd1 = match self.commander_team1 {
            Some(e) if self.world.get_entity(e).is_ok() && self.world.get::<Dead>(e).is_none() => e,
            _ => return, // AI commander is dead
        };

        // Check if team 1 has a factory
        let has_factory = self.factory_team1.is_some()
            && self
                .factory_team1
                .map(|f| self.world.get_entity(f).is_ok() && self.world.get::<Dead>(f).is_none())
                .unwrap_or(false);

        if !has_factory {
            // Build a factory near the commander
            if let Some(cmd_pos) = self.world.get::<Position>(cmd1) {
                let fx = cmd_pos.pos.x.to_f32() + 40.0;
                let fz = cmd_pos.pos.z.to_f32();
                self.place_building(PlacementType::Factory, fx, fz, 1);
            }
        } else if let Some(factory) = self.factory_team1 {
            // Queue a random combat unit
            let unit_names = ["armpw", "armflash", "armstump", "armham"];
            let idx = (self.ai_rng.next_u32() as usize) % unit_names.len();
            self.queue_factory_unit(factory, unit_names[idx]);

            // Also build some economy buildings occasionally
            if self.frame_count.is_multiple_of(AI_TICK_INTERVAL * 3) {
                if let Some(cmd_pos) = self.world.get::<Position>(cmd1) {
                    let offset = self.ai_rng.next_f32(80.0) - 40.0;
                    let sx = cmd_pos.pos.x.to_f32() + offset;
                    let sz = cmd_pos.pos.z.to_f32() + self.ai_rng.next_f32(80.0) - 40.0;
                    self.place_building(PlacementType::Solar, sx, sz, 1);
                }
            }
        }

        // Move idle combat units toward enemy commander
        let enemy_pos = self
            .commander_team0
            .and_then(|e| self.world.get::<Position>(e))
            .map(|p| (p.pos.x.to_f32(), p.pos.z.to_f32()));

        if let Some((ex, ez)) = enemy_pos {
            let idle_units: Vec<Entity> = self
                .world
                .query_filtered::<(Entity, &Allegiance, &MoveState), Without<Dead>>()
                .iter(&self.world)
                .filter(|(e, a, ms)| {
                    a.team == 1 && matches!(ms, MoveState::Idle) && Some(*e) != self.commander_team1
                })
                .map(|(e, _, _)| e)
                .collect();

            for unit in idle_units {
                if let Some(ms) = self.world.get_mut::<MoveState>(unit) {
                    let target = SimVec3::new(
                        SimFloat::from_f32(ex),
                        SimFloat::ZERO,
                        SimFloat::from_f32(ez),
                    );
                    *ms.into_inner() = MoveState::MovingTo(target);
                }
            }
        }
    }

    /// Run one simulation tick.
    fn tick(&mut self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        // Snapshot entities with low health before sim_tick to detect deaths
        let pre_death: Vec<[f32; 3]> = self
            .world
            .query_filtered::<(&Position, &Health), Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, h)| h.current <= SimFloat::ZERO)
            .map(|(p, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
            .collect();

        // Capture impact positions
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

        // Run construction_system (not included in sim_tick)
        recoil_sim::construction::construction_system(&mut self.world);

        // Run all systems via sim_runner
        sim_runner::sim_tick(&mut self.world);

        // Equip factory-spawned units with full components
        self.equip_factory_spawned_units();

        // Check for completed build sites (construction_system removes BuildSite component)
        // and convert them into functional buildings
        self.finalize_completed_buildings();

        // AI tick
        self.ai_tick();

        // Detect newly dead entities
        let new_deaths: Vec<[f32; 3]> = {
            let mut q = self.world.query::<(&Position, &Dead, &Health)>();
            q.iter(&self.world)
                .filter(|(_, _, h)| h.current <= SimFloat::ZERO)
                .map(|(p, _, _)| [p.pos.x.to_f32(), p.pos.y.to_f32() + 5.0, p.pos.z.to_f32()])
                .collect()
        };

        let death_positions = if new_deaths.is_empty() {
            pre_death
        } else {
            new_deaths
        };

        (impact_positions, death_positions)
    }

    /// After construction_system completes a building (removes BuildSite),
    /// add the appropriate functional components based on UnitType ID.
    fn finalize_completed_buildings(&mut self) {
        // Find completed buildings: have a building UnitType ID, no BuildSite,
        // no ResourceProducer/BuildQueue yet (not finalized).
        let candidates: Vec<(Entity, u32, u8, f32, f32)> = self
            .world
            .query_filtered::<(Entity, &UnitType, &Allegiance, &Position), (
                Without<BuildSite>,
                Without<MoveState>,
                Without<ResourceProducer>,
                Without<BuildQueue>,
                Without<Dead>,
                Without<Builder>,
            )>()
            .iter(&self.world)
            .filter(|(_, ut, _, _)| {
                ut.id == BUILDING_SOLAR_ID
                    || ut.id == BUILDING_MEX_ID
                    || ut.id == BUILDING_FACTORY_ID
            })
            .map(|(e, ut, a, p)| (e, ut.id, a.team, p.pos.x.to_f32(), p.pos.z.to_f32()))
            .collect();

        for (entity, building_id, team, x, z) in candidates {
            match building_id {
                BUILDING_SOLAR_ID => {
                    self.world.entity_mut(entity).insert(ResourceProducer {
                        metal_per_tick: SimFloat::ZERO,
                        energy_per_tick: SimFloat::from_int(20),
                    });
                    tracing::info!("Team {} Solar completed at ({:.0}, {:.0})", team, x, z);
                }
                BUILDING_MEX_ID => {
                    self.world.entity_mut(entity).insert(ResourceProducer {
                        metal_per_tick: SimFloat::from_int(3),
                        energy_per_tick: SimFloat::ZERO,
                    });
                    tracing::info!("Team {} Mex completed at ({:.0}, {:.0})", team, x, z);
                }
                BUILDING_FACTORY_ID => {
                    let rally = SimVec3::new(
                        SimFloat::from_f32(x + 30.0),
                        SimFloat::ZERO,
                        SimFloat::from_f32(z),
                    );
                    self.world.entity_mut(entity).insert(BuildQueue {
                        queue: VecDeque::new(),
                        current_progress: SimFloat::ZERO,
                        rally_point: rally,
                    });
                    tracing::info!("Team {} Factory completed at ({:.0}, {:.0})", team, x, z);

                    // Track first factory for AI
                    if team == 1 && self.factory_team1.is_none() {
                        self.factory_team1 = Some(entity);
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract unit instances for rendering (exclude Dead entities).
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
                Option<&BuildSite>,
            ), Without<Dead>>()
            .iter(&self.world)
            .map(
                |(entity, pos, heading, allegiance, health, stunned, build_site)| {
                    // Base team color
                    let mut color = if allegiance.team == 0 {
                        [0.3f32, 0.5, 1.0] // blue
                    } else {
                        [1.0f32, 0.3, 0.3] // red
                    };

                    // Buildings under construction: yellow-ish
                    if build_site.is_some() {
                        color = if allegiance.team == 0 {
                            [0.2, 0.4, 0.6]
                        } else {
                            [0.6, 0.2, 0.2]
                        };
                    }

                    // Scale brightness by HP fraction
                    let hp_frac = if health.max > SimFloat::ZERO {
                        (health.current.to_f32() / health.max.to_f32()).clamp(0.2, 1.0)
                    } else {
                        1.0
                    };
                    color[0] *= hp_frac;
                    color[1] *= hp_frac;
                    color[2] *= hp_frac;

                    if stunned.is_some() {
                        color[0] = color[0] * 0.5 + 0.5 * 0.6;
                        color[1] *= 0.3;
                        color[2] = color[2] * 0.5 + 0.5 * 0.8;
                    }

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
                },
            )
            .collect()
    }

    /// Also render entities with Position + Allegiance but no Heading (buildings).
    fn building_instances(&mut self) -> Vec<UnitInstance> {
        let selected = self.selected;

        self.world
            .query_filtered::<(
                Entity,
                &Position,
                &Allegiance,
                &Health,
                Option<&BuildSite>,
            ), (Without<Dead>, Without<Heading>)>()
            .iter(&self.world)
            .map(|(entity, pos, allegiance, health, build_site)| {
                let mut color = if allegiance.team == 0 {
                    [0.1f32, 0.8, 0.3] // green for friendly buildings
                } else {
                    [0.8f32, 0.1, 0.3] // dark red for enemy buildings
                };

                if build_site.is_some() {
                    // Under construction: dimmer
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
                    color[0] = (color[0] + 0.4).min(1.0);
                    color[1] = (color[1] + 0.4).min(1.0);
                    color[2] = (color[2] + 0.4).min(1.0);
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

    /// Extract wreckage instances.
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
                color: [0.4, 0.35, 0.25],
                _pad2: 0.0,
            })
            .collect()
    }

    /// Draw target lines.
    fn target_line_instances(&mut self) -> Vec<ProjectileInstance> {
        let mut instances = Vec::new();

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
                        color: [1.0, 0.1, 0.1],
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

    /// Get HUD text for window title.
    fn hud_text(&mut self) -> String {
        let (blue, red) = self.unit_counts();

        let (metal, metal_storage, energy, energy_storage) = {
            let economy = self.world.resource::<EconomyState>();
            if let Some(res) = economy.teams.get(&0) {
                (
                    res.metal.to_f32(),
                    res.metal_storage.to_f32(),
                    res.energy.to_f32(),
                    res.energy_storage.to_f32(),
                )
            } else {
                (0.0, 0.0, 0.0, 0.0)
            }
        };

        let mode = match self.placement_mode {
            Some(pt) => pt.label(),
            None => {
                if let Some(sel) = self.selected {
                    if self.world.get::<BuildQueue>(sel).is_some() {
                        "Factory [1-4 queue]"
                    } else if self.world.get::<Builder>(sel).is_some() {
                        "Commander [S/M/F build]"
                    } else {
                        "Select"
                    }
                } else {
                    "Select"
                }
            }
        };

        let pause_str = if self.paused { " PAUSED" } else { "" };

        format!(
            "Recoil RTS | M:{:.0}/{:.0} E:{:.0}/{:.0} | B:{} R:{} | {}{} | F:{}",
            metal,
            metal_storage,
            energy,
            energy_storage,
            blue,
            red,
            mode,
            pause_str,
            self.frame_count
        )
    }

    /// Check if the selected entity is a factory.
    fn selected_is_factory(&self) -> bool {
        self.selected
            .map(|e| self.world.get::<BuildQueue>(e).is_some())
            .unwrap_or(false)
    }

    /// Check if the selected entity is a commander/builder.
    fn selected_is_builder(&self) -> bool {
        self.selected
            .map(|e| self.world.get::<Builder>(e).is_some())
            .unwrap_or(false)
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
    cursor_pos: [f32; 2],
    window_size: [f32; 2],
    // Keyboard modifier state
    a_held: bool,
    // S key is used for both camera backward and Solar placement.
    // We use S only when no unit is selected or non-builder is selected for camera.
    // When a builder is selected, S enters solar placement mode.
    backward_held: bool,
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
            a_held: false,
            backward_held: false,
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
                Ok((mut verts, indices)) => {
                    tracing::info!(
                        "Loaded .s3o model: {} verts, {} indices",
                        verts.len(),
                        indices.len()
                    );
                    let scale = 0.4;
                    for v in &mut verts {
                        let x = v.position[0];
                        let z = v.position[2];
                        v.position[0] = z * scale;
                        v.position[1] *= scale;
                        v.position[2] = -x * scale;
                        let nx = v.normal[0];
                        let nz = v.normal[2];
                        v.normal[0] = nz;
                        v.normal[2] = -nx;
                    }
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

                // Camera movement keys (W, A, D always work; S works as camera backward)
                self.camera_ctrl.process_key(key, pressed);

                // Track S key for camera backward movement
                if key == KeyCode::KeyS {
                    self.backward_held = pressed;
                }

                // Track A key for attack-move modifier
                if key == KeyCode::KeyA {
                    self.a_held = pressed;
                }

                // Use S for backward camera only when NOT in builder context
                // Camera backward is handled separately
                self.camera_ctrl.backward = self.backward_held && !self.sim.selected_is_builder();

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
                            if self.sim.placement_mode.is_some() {
                                self.sim.placement_mode = None;
                            } else {
                                event_loop.exit();
                            }
                        }

                        // Building placement keys (only when builder selected)
                        KeyCode::KeyS if self.sim.selected_is_builder() => {
                            self.sim.placement_mode = Some(PlacementType::Solar);
                        }
                        KeyCode::KeyM if self.sim.selected_is_builder() => {
                            self.sim.placement_mode = Some(PlacementType::Mex);
                        }
                        KeyCode::KeyF if self.sim.selected_is_builder() => {
                            self.sim.placement_mode = Some(PlacementType::Factory);
                        }

                        // Factory production keys (1-4 when factory selected)
                        KeyCode::Digit1 if self.sim.selected_is_factory() => {
                            let sel = self.sim.selected.unwrap();
                            self.sim.queue_factory_unit(sel, "armpw");
                        }
                        KeyCode::Digit2 if self.sim.selected_is_factory() => {
                            let sel = self.sim.selected.unwrap();
                            self.sim.queue_factory_unit(sel, "armflash");
                        }
                        KeyCode::Digit3 if self.sim.selected_is_factory() => {
                            let sel = self.sim.selected.unwrap();
                            self.sim.queue_factory_unit(sel, "armstump");
                        }
                        KeyCode::Digit4 if self.sim.selected_is_factory() => {
                            let sel = self.sim.selected.unwrap();
                            self.sim.queue_factory_unit(sel, "armham");
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
                    if self.sim.placement_mode.is_some() {
                        match button {
                            MouseButton::Left => {
                                let btype = self.sim.placement_mode.unwrap();
                                self.sim.place_building(btype, wx, wz, 0);
                                self.sim.placement_mode = None;
                            }
                            MouseButton::Right => {
                                self.sim.placement_mode = None;
                            }
                            _ => {}
                        }
                    } else {
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

                let mut impact_positions = Vec::new();
                let mut death_positions = Vec::new();
                if !self.sim.paused {
                    let (impacts, deaths) = self.sim.tick();
                    impact_positions = impacts;
                    death_positions = deaths;
                    self.sim.frame_count += 1;
                }

                // Emit particles
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

                self.particle_system.update(dt);

                // Build render data
                let mut unit_instances = self.sim.unit_instances();
                unit_instances.extend(self.sim.building_instances());
                let mut proj_instances = self.sim.projectile_instances();
                proj_instances.extend(self.sim.wreckage_instances());
                proj_instances.extend(self.sim.target_line_instances());

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

                // Update window title HUD
                if let Some(window) = self.window.as_ref() {
                    let title = self.sim.hud_text();
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

fn mat4_inverse(m: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
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

    tracing::info!("Recoil RTS starting (playable RTS mode)...");

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app).expect("event loop error");
}
