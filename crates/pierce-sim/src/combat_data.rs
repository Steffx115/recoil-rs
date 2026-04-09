//! Armor types, damage types, weapon definitions, and damage lookup tables.
//!
//! All simulation-facing values use deterministic fixed-point [`SimFloat`]
//! so that replays and checksums stay identical across platforms.

use bevy_ecs::prelude::Component;
use bevy_ecs::system::Resource;
use serde::{Deserialize, Serialize};

use crate::SimFloat;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Classification of a unit's protective plating.
#[derive(Component, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArmorClass {
    Light,
    Medium,
    Heavy,
    Building,
}

impl ArmorClass {
    /// Total number of variants (used for flat-array indexing).
    pub const COUNT: usize = 4;

    /// Convert to a `usize` index for flat-array lookup.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Classification of the damage a weapon deals.
#[derive(Component, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DamageType {
    #[default]
    Normal,
    Explosive,
    Laser,
    Paralyzer,
}

impl DamageType {
    /// Total number of variants (used for flat-array indexing).
    pub const COUNT: usize = 4;

    /// Convert to a `usize` index for flat-array lookup.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

// ---------------------------------------------------------------------------
// WeaponDef — data definition, NOT an ECS component
// ---------------------------------------------------------------------------

/// Category of weapon for target priority purposes.
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponCategory {
    /// General-purpose weapon.
    #[default]
    General,
    /// Anti-air weapon: prefers air targets.
    AntiAir,
    /// Anti-armor weapon: prefers heavy targets.
    AntiArmor,
}

/// Static definition of a weapon type loaded from game data files.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WeaponDef {
    pub damage: SimFloat,
    pub damage_type: DamageType,
    pub range: SimFloat,
    /// Minimum range: weapon cannot fire at targets closer than this.
    #[serde(default)]
    pub min_range: SimFloat,
    /// Reload time in simulation frames.
    pub reload_time: u32,
    /// Projectile speed.  `ZERO` means instant hit (beam / hitscan).
    pub projectile_speed: SimFloat,
    /// Blast radius.  `ZERO` means single-target.
    pub area_of_effect: SimFloat,
    /// If `true`, damage is applied as stun rather than health loss.
    pub is_paralyzer: bool,
    /// If `true`, weapon can fire over terrain obstacles (artillery).
    #[serde(default)]
    pub indirect_fire: bool,
    /// Weapon category for target priority.
    #[serde(default)]
    pub category: WeaponCategory,
    /// Turret turn rate in radians per tick. `ZERO` means instant aim.
    #[serde(default)]
    pub turret_turn_rate: SimFloat,
    /// Firing arc half-angle in radians. `ZERO` means full 360-degree arc.
    #[serde(default)]
    pub firing_arc: SimFloat,
    /// Angular tolerance for firing: weapon fires when turret is within
    /// this angle of the target. Defaults to ~3 degrees if zero.
    #[serde(default)]
    pub aim_tolerance: SimFloat,
}

impl Default for WeaponDef {
    fn default() -> Self {
        Self {
            damage: SimFloat::ZERO,
            damage_type: DamageType::Normal,
            range: SimFloat::ZERO,
            min_range: SimFloat::ZERO,
            reload_time: 1,
            projectile_speed: SimFloat::ZERO,
            area_of_effect: SimFloat::ZERO,
            is_paralyzer: false,
            indirect_fire: false,
            category: WeaponCategory::General,
            turret_turn_rate: SimFloat::ZERO,
            firing_arc: SimFloat::ZERO,
            aim_tolerance: SimFloat::ZERO,
        }
    }
}

// ---------------------------------------------------------------------------
// DamageTable — Bevy resource
// ---------------------------------------------------------------------------

/// Lookup table mapping `(DamageType, ArmorClass)` to a damage multiplier.
///
/// Stored as a flat array of size `DamageType::COUNT * ArmorClass::COUNT`,
/// row-major by damage type.
#[derive(Resource, Debug, Clone)]
pub struct DamageTable {
    /// `multipliers[damage_type.index() * ArmorClass::COUNT + armor_class.index()]`
    multipliers: [SimFloat; DamageType::COUNT * ArmorClass::COUNT],
}

impl DamageTable {
    /// Look up the multiplier for a given damage type vs armor class.
    #[inline]
    pub fn get(&self, damage_type: DamageType, armor: ArmorClass) -> SimFloat {
        self.multipliers[damage_type.index() * ArmorClass::COUNT + armor.index()]
    }

    /// Set the multiplier for a given damage type vs armor class.
    #[inline]
    pub fn set(&mut self, damage_type: DamageType, armor: ArmorClass, value: SimFloat) {
        self.multipliers[damage_type.index() * ArmorClass::COUNT + armor.index()] = value;
    }
}

impl Default for DamageTable {
    /// Returns the canonical default damage table:
    ///
    /// |            | Light | Medium | Heavy | Building |
    /// |------------|-------|--------|-------|----------|
    /// | Normal     |  1.0  |   0.8  |  0.5  |   0.5   |
    /// | Explosive  |  0.5  |   1.0  |  1.5  |   1.5   |
    /// | Laser      |  1.5  |   1.0  |  0.5  |   0.3   |
    /// | Paralyzer  |  1.0  |   1.0  |  1.0  |   1.0   |
    fn default() -> Self {
        use ArmorClass::*;
        use DamageType::*;

        let mut table = Self {
            multipliers: [SimFloat::ONE; DamageType::COUNT * ArmorClass::COUNT],
        };

        // Normal: Light=1.0, Medium=0.8, Heavy=0.5, Building=0.5
        // (Light=1.0 already set by the ONE fill)
        table.set(Normal, Medium, SimFloat::from_ratio(4, 5));
        table.set(Normal, Heavy, SimFloat::HALF);
        table.set(Normal, Building, SimFloat::HALF);

        // Explosive: Light=0.5, Medium=1.0, Heavy=1.5, Building=1.5
        table.set(Explosive, Light, SimFloat::HALF);
        table.set(Explosive, Heavy, SimFloat::from_ratio(3, 2));
        table.set(Explosive, Building, SimFloat::from_ratio(3, 2));

        // Laser: Light=1.5, Medium=1.0, Heavy=0.5, Building=0.3
        table.set(Laser, Light, SimFloat::from_ratio(3, 2));
        table.set(Laser, Heavy, SimFloat::HALF);
        table.set(Laser, Building, SimFloat::from_ratio(3, 10));

        // Paralyzer: all 1.0 — already set by the ONE fill

        table
    }
}

// ---------------------------------------------------------------------------
// calc_damage
// ---------------------------------------------------------------------------

/// Calculate final damage: `weapon.damage * table.get(weapon.damage_type, armor)`.
///
/// For paralyzer weapons the multiplier is still applied (always 1.0 in the
/// default table), but the caller is responsible for routing the result to
/// stun rather than health loss (check `weapon.is_paralyzer`).
pub fn calc_damage(table: &DamageTable, weapon: &WeaponDef, armor: ArmorClass) -> SimFloat {
    let multiplier = table.get(weapon.damage_type, armor);
    weapon.damage * multiplier
}

// ---------------------------------------------------------------------------
// WeaponInstance / WeaponSet — ECS components
// ---------------------------------------------------------------------------

/// Runtime state for a single weapon slot on an entity.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WeaponInstance {
    /// Index into a weapon registry (maps to a [`WeaponDef`]).
    pub def_id: u32,
    /// Frames remaining until the weapon can fire again.
    pub reload_remaining: u32,
}

/// Component holding all weapon slots on an entity.
#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct WeaponSet {
    pub weapons: Vec<WeaponInstance>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/combat_data_tests.rs"]
mod tests;
