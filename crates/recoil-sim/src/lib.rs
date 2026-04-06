pub mod collision;
pub mod combat_data;
pub mod commands;
pub mod components;
pub mod construction;
pub mod damage;
pub mod determinism;
pub mod economy;
pub mod factory;
pub mod flowfield;
pub mod lifecycle;
pub mod movement;
pub mod pathfinding;
pub mod projectile;
pub mod sim_runner;
pub mod spatial;
pub mod targeting;

pub use recoil_math::{SimFloat, SimVec2, SimVec3};

pub use commands::{command_system, Command, CommandQueue};
pub use components::{
    Allegiance, BuildProgress, Cloaked, CollisionRadius, Dead, Heading, Health, MoveState,
    MovementParams, Position, SimId, Target, Transport, UnitType, Velocity,
};
pub use spatial::SpatialGrid;
pub use targeting::{FireEvent, FireEventQueue, WeaponRegistry};
