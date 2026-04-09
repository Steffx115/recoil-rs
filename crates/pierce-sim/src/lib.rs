pub mod animation;
pub mod collision;
pub mod combat_data;
pub mod commands;
pub mod components;
#[cfg(feature = "compute-backends")]
pub mod compute;
pub mod frame_data;
pub mod construction;
pub mod damage;
pub mod determinism;
pub mod economy;
pub mod factory;
pub mod flowfield;
pub mod fog;
pub mod lifecycle;
pub mod lua_unitdefs;
pub mod footprint;
pub mod map;
pub mod movement;
pub mod pathfinding;
pub mod projectile;
pub mod selection;
pub mod shield;
pub mod sim_runner;
pub mod smf;
pub mod spatial;
pub mod targeting;
pub mod unit_defs;

pub use pierce_math::{SimFloat, SimVec2, SimVec3};

pub use animation::{AnimationState, PieceAnimTransform};

pub use commands::{command_system, Command, CommandHandler, CommandQueue, CommandResult};
pub use components::{
    Allegiance, AttackMove, BuildProgress, BuildingFootprint, Cloaked, CollisionRadius, Dead,
    FireMode, Heading, Health, LastAttacker, ManualTarget, MoveState, MovementParams, Position,
    SightRange, SimId, Target, Transport, TurretFacings, TurretState, UnitType, Velocity,
};
pub use fog::{fog_system, is_entity_visible, CellVisibility, FogOfWar};
pub use shield::{shield_absorb_system, shield_regen_system, Shield};
pub use pathfinding::{AStarPathfinder, FlowFieldPathfinder, HybridPathfinder, Pathfinder};
pub use selection::{screen_to_ground_raw, SelectionState};
pub use spatial::SpatialGrid;
pub use map::HeightmapData;
pub use targeting::{turret_system, FireEvent, FireEventQueue, PendingDamage, WeaponRegistry};
