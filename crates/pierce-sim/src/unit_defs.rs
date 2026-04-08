//! Unit definition data format and loader.
//!
//! [`UnitDef`] is the RON-serializable description of a unit type loaded from
//! game data files.  The [`UnitDefRegistry`] resource collects all loaded defs
//! and provides lookup by `unit_type_id`.  Conversion helpers bridge the gap
//! between the data-driven [`UnitDef`] and the runtime types used by the sim
//! ([`UnitBlueprint`], [`WeaponDef`], [`ArmorClass`]).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use bevy_ecs::system::Resource;
use serde::{Deserialize, Serialize};

use crate::combat_data::{ArmorClass, DamageType, WeaponDef};
use crate::factory::UnitBlueprint;
use crate::SimFloat;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// RON-serializable definition of a weapon attached to a unit.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WeaponDefData {
    pub name: String,
    pub damage: f64,
    pub damage_type: String,
    pub range: f64,
    pub reload_time: u32,
    pub projectile_speed: f64,
    pub area_of_effect: f64,
}

/// RON-serializable definition of a unit type.
///
/// Loaded from `.ron` files in a game data directory and stored in the
/// [`UnitDefRegistry`].  Use the conversion helpers to produce runtime sim
/// types.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UnitDef {
    pub name: String,
    pub unit_type_id: u32,
    // Combat
    pub max_health: f64,
    pub armor_class: String,
    pub sight_range: f64,
    pub collision_radius: f64,
    // Movement
    pub max_speed: f64,
    pub acceleration: f64,
    pub turn_rate: f64,
    // Economy
    pub metal_cost: f64,
    pub energy_cost: f64,
    pub build_time: u32,
    // Weapons
    pub weapons: Vec<WeaponDefData>,
    // Assets
    pub model_path: Option<String>,
    pub icon_path: Option<String>,
    // Categories
    pub categories: Vec<String>,
    // Production (factory/builder build list — resolved unit_type_ids)
    pub can_build: Vec<u32>,
    #[serde(default)]
    pub can_build_names: Vec<String>,
    // Builder
    pub build_power: Option<f64>,
    // Resource production
    pub metal_production: Option<f64>,
    pub energy_production: Option<f64>,
    #[serde(default)]
    pub is_building: bool,
    #[serde(default)]
    pub is_builder: bool,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

impl UnitDef {
    /// Convert to a [`UnitBlueprint`] for the factory production system.
    pub fn to_unit_blueprint(&self) -> UnitBlueprint {
        UnitBlueprint {
            unit_type_id: self.unit_type_id,
            metal_cost: SimFloat::from_f64(self.metal_cost),
            energy_cost: SimFloat::from_f64(self.energy_cost),
            build_time: self.build_time,
            max_health: SimFloat::from_f64(self.max_health),
        }
    }

    /// Convert weapon data entries to sim [`WeaponDef`] values.
    pub fn to_weapon_defs(&self) -> Vec<WeaponDef> {
        self.weapons.iter().map(|w| w.to_weapon_def()).collect()
    }

    /// Parse the `armor_class` string to the [`ArmorClass`] enum.
    ///
    /// Defaults to [`ArmorClass::Light`] for unrecognised values.
    pub fn parse_armor_class(&self) -> ArmorClass {
        parse_armor_class_str(&self.armor_class)
    }

    pub fn is_factory(&self) -> bool {
        self.is_building && !self.can_build.is_empty()
    }

    pub fn compute_derived_flags(&mut self) {
        self.is_building = self.max_speed == 0.0 || self.armor_class == "Building";
        self.is_builder = self.build_power.is_some();
    }
}

impl WeaponDefData {
    /// Convert to a sim [`WeaponDef`].
    pub fn to_weapon_def(&self) -> WeaponDef {
        let damage_type = parse_damage_type_str(&self.damage_type);
        WeaponDef {
            damage: SimFloat::from_f64(self.damage),
            damage_type,
            range: SimFloat::from_f64(self.range),
            reload_time: self.reload_time,
            projectile_speed: SimFloat::from_f64(self.projectile_speed),
            area_of_effect: SimFloat::from_f64(self.area_of_effect),
            is_paralyzer: damage_type == DamageType::Paralyzer,
        }
    }
}

fn parse_armor_class_str(s: &str) -> ArmorClass {
    match s {
        "Light" => ArmorClass::Light,
        "Medium" => ArmorClass::Medium,
        "Heavy" => ArmorClass::Heavy,
        "Building" => ArmorClass::Building,
        _ => ArmorClass::Light,
    }
}

fn parse_damage_type_str(s: &str) -> DamageType {
    match s {
        "Normal" => DamageType::Normal,
        "Explosive" => DamageType::Explosive,
        "Laser" => DamageType::Laser,
        "Paralyzer" => DamageType::Paralyzer,
        _ => DamageType::Normal,
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Resource holding all loaded [`UnitDef`] entries, indexed by `unit_type_id`.
#[derive(Resource, Debug, Clone, Default)]
pub struct UnitDefRegistry {
    pub defs: BTreeMap<u32, UnitDef>,
    name_index: BTreeMap<String, u32>,
}

impl UnitDefRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: UnitDef) {
        self.name_index
            .insert(def.name.to_lowercase(), def.unit_type_id);
        self.defs.insert(def.unit_type_id, def);
    }

    pub fn get(&self, unit_type_id: u32) -> Option<&UnitDef> {
        self.defs.get(&unit_type_id)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&UnitDef> {
        self.name_index
            .get(&name.to_lowercase())
            .and_then(|id| self.defs.get(id))
    }

    pub fn resolve_build_options(&mut self) {
        let name_to_id: BTreeMap<String, u32> = self
            .defs
            .values()
            .map(|d| (d.name.to_lowercase(), d.unit_type_id))
            .collect();
        for def in self.defs.values_mut() {
            if !def.can_build_names.is_empty() && def.can_build.is_empty() {
                def.can_build = def
                    .can_build_names
                    .iter()
                    .filter_map(|name| name_to_id.get(&name.to_lowercase()).copied())
                    .collect();
            }
        }
    }

    pub fn compute_derived_flags(&mut self) {
        for def in self.defs.values_mut() {
            def.compute_derived_flags();
        }
    }

    /// Load all `.ron` files from `dir` and return a populated registry.
    pub fn load_directory(dir: &Path) -> Result<Self> {
        let mut registry = Self::new();
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("ron") {
                let contents = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                let def: UnitDef = ron::from_str(&contents)
                    .with_context(|| format!("Failed to parse {}", path.display()))?;
                registry.register(def);
            }
        }

        Ok(registry)
    }

    /// Save a single unit definition to a RON file at `path`.
    pub fn save_def(def: &UnitDef, path: &Path) -> Result<()> {
        let pretty = ron::ser::PrettyConfig::default();
        let contents = ron::ser::to_string_pretty(def, pretty)
            .with_context(|| "Failed to serialize UnitDef to RON")?;
        std::fs::write(path, contents)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "unit_defs_tests.rs"]
mod tests;
