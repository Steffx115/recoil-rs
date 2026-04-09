//! Loadtest mode: spawns waves of armed units for stress testing.
//!
//! When enabled, periodically spawns batches of units for both teams
//! and sends them toward the center of the map to fight.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use pierce_math::{SimFloat, SimVec3};
use pierce_sim::combat_data::{ArmorClass, DamageType, WeaponDef, WeaponInstance, WeaponSet};
use pierce_sim::commands::CommandQueue;
use pierce_sim::targeting::WeaponRegistry;
use pierce_sim::{
    Allegiance, CollisionRadius, Command, Heading, Health, MoveState, MovementParams, Position,
    SightRange, Target, UnitType, Velocity,
};

/// Loadtest state, inserted as a field on App.
pub struct LoadtestState {
    pub enabled: bool,
    pub weapon_id: Option<u32>,
    pub units_per_wave: usize,
    pub wave_interval: u64,
    pub total_spawned: usize,
    pub max_units: usize,
}

impl Default for LoadtestState {
    fn default() -> Self {
        Self {
            enabled: false,
            weapon_id: None,
            units_per_wave: 50,
            wave_interval: 120, // every 2 seconds at 60fps
            total_spawned: 0,
            max_units: 2000,
        }
    }
}

impl LoadtestState {
    pub fn new(units_per_wave: usize, max_units: usize) -> Self {
        Self {
            enabled: true,
            units_per_wave,
            max_units,
            ..Default::default()
        }
    }

    /// Register a test weapon if not already done.
    fn ensure_weapon(&mut self, world: &mut World) {
        if self.weapon_id.is_some() {
            return;
        }
        let mut registry = world.resource_mut::<WeaponRegistry>();
        let id = registry.defs.len() as u32;
        registry.defs.push(WeaponDef {
            damage: SimFloat::from_int(30),
            damage_type: DamageType::Normal,
            range: SimFloat::from_int(200),
            reload_time: 15,
            projectile_speed: SimFloat::from_int(8),
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
            ..Default::default()
        });
        self.weapon_id = Some(id);
    }

    /// Run one loadtest tick. Call after game.tick().
    pub fn tick(&mut self, world: &mut World, frame: u64) {
        if !self.enabled || self.total_spawned >= self.max_units {
            return;
        }
        if frame % self.wave_interval != 0 {
            return;
        }

        self.ensure_weapon(world);
        let weapon_id = self.weapon_id.unwrap();

        let batch = self.units_per_wave.min(self.max_units - self.total_spawned);
        let half = batch / 2;

        // Spawn team 0 on the left, team 1 on the right.
        // Stagger positions within the wave using the spawn count as seed.
        let base_offset = self.total_spawned as i32;

        for i in 0..half {
            let idx = base_offset + i as i32;
            let row = idx / 20;
            let col = idx % 20;
            // Team 0: left side, spread vertically.
            spawn_loadtest_unit(
                world,
                300 + col * 16,
                800 + row * 16,
                0,
                weapon_id,
            );
            // Team 1: right side.
            spawn_loadtest_unit(
                world,
                1700 - col * 16,
                800 + row * 16,
                1,
                weapon_id,
            );
        }

        // Send all units toward center.
        issue_attack_move(world);

        self.total_spawned += batch;
        tracing::info!(
            "Loadtest wave: spawned {} units ({} total)",
            batch,
            self.total_spawned
        );
    }
}

fn spawn_loadtest_unit(
    world: &mut World,
    x: i32,
    z: i32,
    team: u8,
    weapon_id: u32,
) -> Entity {
    let entity = pierce_sim::lifecycle::spawn_unit(
        world,
        Position {
            pos: SimVec3::new(SimFloat::from_int(x), SimFloat::ZERO, SimFloat::from_int(z)),
        },
        UnitType { id: 1 },
        Allegiance { team },
        Health {
            current: SimFloat::from_int(500),
            max: SimFloat::from_int(500),
        },
    );
    world.entity_mut(entity).insert((
        MoveState::Idle,
        MovementParams {
            max_speed: SimFloat::from_int(2),
            acceleration: SimFloat::ONE,
            turn_rate: SimFloat::ONE,
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
                def_id: weapon_id,
                reload_remaining: 0,
            }],
        },
        SightRange {
            range: SimFloat::from_int(300),
        },
        CommandQueue::default(),
    ));
    entity
}

/// Send all idle units toward the map center.
fn issue_attack_move(world: &mut World) {
    let units: Vec<(Entity, u8)> = world
        .query::<(Entity, &Allegiance, &MoveState, &CommandQueue)>()
        .iter(world)
        .filter(|(_, _, ms, _)| matches!(ms, MoveState::Idle))
        .map(|(e, a, _, _)| (e, a.team))
        .collect();

    for (entity, team) in units {
        let target_x = if team == 0 { 1200 } else { 800 };
        if let Some(mut cq) = world.get_mut::<CommandQueue>(entity) {
            cq.replace(Command::Move(SimVec3::new(
                SimFloat::from_int(target_x),
                SimFloat::ZERO,
                SimFloat::from_int(1024),
            )));
        }
    }
}
