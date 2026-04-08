//! Animation driver: connects COB VM instances to gameplay events.
//!
//! Each spawned unit with a COB script gets its own [`CobVm`] instance.
//! The driver translates high-level gameplay events (unit created, started
//! moving, fired weapon) into COB script calls (Create, StartMoving, Walk,
//! StopMoving, FirePrimary) and ticks all VMs each frame.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::loader::{parse_cob, CobScript};
use crate::vm::CobVm;
use pierce_model::{flatten_with_transforms, ModelVertex, PieceTransform, PieceTree};

// ---------------------------------------------------------------------------
// Per-unit animation instance
// ---------------------------------------------------------------------------

struct UnitAnimInstance {
    type_id: u32,
    vm: CobVm,
    is_moving: bool,
    walk_started: bool,
    #[allow(dead_code)] // read in tests
    create_called: bool,
}

// ---------------------------------------------------------------------------
// CobAnimationDriver
// ---------------------------------------------------------------------------

/// Manages per-unit [`CobVm`] instances and translates gameplay events into
/// COB script calls.
pub struct CobAnimationDriver {
    /// Parsed COB scripts indexed by unit type ID.
    scripts: BTreeMap<u32, CobScript>,
    /// Active per-unit animation instances indexed by entity bits.
    units: BTreeMap<u64, UnitAnimInstance>,
}

impl CobAnimationDriver {
    /// Create an empty animation driver.
    pub fn new() -> Self {
        Self {
            scripts: BTreeMap::new(),
            units: BTreeMap::new(),
        }
    }

    /// Parse a COB binary and store it under the given unit type ID.
    pub fn load_script(&mut self, unit_type_id: u32, cob_data: &[u8]) -> Result<()> {
        let script = parse_cob(cob_data)?;
        self.scripts.insert(unit_type_id, script);
        Ok(())
    }

    /// Check whether a COB script has been loaded for the given type.
    pub fn has_script(&self, unit_type_id: u32) -> bool {
        self.scripts.contains_key(&unit_type_id)
    }

    /// Spawn a new animation VM for a unit entity.
    ///
    /// Calls the "Create" script if it exists in the COB file.
    pub fn spawn_unit(&mut self, entity_bits: u64, unit_type_id: u32) {
        let Some(script) = self.scripts.get(&unit_type_id) else {
            return;
        };
        let mut vm = CobVm::new(script);
        let create_called = vm.call_script(script, "Create");
        self.units.insert(
            entity_bits,
            UnitAnimInstance {
                type_id: unit_type_id,
                vm,
                is_moving: false,
                walk_started: false,
                create_called,
            },
        );
    }

    /// Remove the animation VM for a despawned unit.
    pub fn despawn_unit(&mut self, entity_bits: u64) {
        self.units.remove(&entity_bits);
    }

    /// Update the movement state for a unit.
    ///
    /// On idle-to-moving transition: calls "StartMoving" then "Walk".
    /// On moving-to-idle transition: calls "StopMoving".
    pub fn set_moving(&mut self, entity_bits: u64, moving: bool) {
        let Some(inst) = self.units.get_mut(&entity_bits) else {
            return;
        };
        let was_moving = inst.is_moving;
        inst.is_moving = moving;

        if moving && !was_moving {
            // Start moving — the StartMoving script typically sets isMoving=TRUE
            // and calls start-script Walk() internally.
            if let Some(script) = self.scripts.get(&inst.type_id) {
                vm_call_if_exists(&mut inst.vm, script, "StartMoving");
                inst.walk_started = true;
            }
        } else if !moving && was_moving {
            // Stop moving — the StopMoving script typically sets isMoving=FALSE
            // and signals to kill the Walk thread.
            if let Some(script) = self.scripts.get(&inst.type_id) {
                vm_call_if_exists(&mut inst.vm, script, "StopMoving");
                inst.walk_started = false;
            }
        }
    }

    /// Trigger the weapon fire animation.
    pub fn fire(&mut self, entity_bits: u64) {
        let Some(inst) = self.units.get_mut(&entity_bits) else {
            return;
        };
        if let Some(script) = self.scripts.get(&inst.type_id) {
            vm_call_if_exists(&mut inst.vm, script, "FirePrimary");
        }
    }

    /// Tick all active VMs one frame.
    pub fn tick(&mut self) {
        for inst in self.units.values_mut() {
            if let Some(script) = self.scripts.get(&inst.type_id) {
                inst.vm.tick(script);
            }
        }
    }

    /// Get current piece transforms for a unit.
    pub fn get_transforms(&self, entity_bits: u64) -> Option<Vec<PieceTransform>> {
        self.units
            .get(&entity_bits)
            .map(|inst| inst.vm.get_piece_transforms())
    }

    /// Flatten the piece tree with current animation transforms to produce
    /// a renderable mesh.
    ///
    /// Remaps COB piece indices to S3O piece tree indices by matching
    /// piece names (COB and S3O use different orderings).
    pub fn generate_animated_mesh(
        &self,
        entity_bits: u64,
        tree: &PieceTree,
    ) -> Option<(Vec<ModelVertex>, Vec<u16>)> {
        let inst = self.units.get(&entity_bits)?;
        let cob_transforms = inst.vm.get_piece_transforms();
        let script = self.scripts.get(&inst.type_id)?;

        // Build name -> COB index lookup.
        let cob_names: BTreeMap<&str, usize> = script
            .pieces
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();

        // Map S3O piece order -> COB transforms by name.
        let remapped: Vec<PieceTransform> = tree
            .pieces
            .iter()
            .map(|s3o_piece| {
                if let Some(&cob_idx) = cob_names.get(s3o_piece.name.as_str()) {
                    cob_transforms
                        .get(cob_idx)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    PieceTransform::default()
                }
            })
            .collect();

        Some(flatten_with_transforms(tree, &remapped))
    }

    /// Iterate over tracked unit entity bits and their type IDs.
    pub fn units_iter(&self) -> impl Iterator<Item = (&u64, &u32)> {
        self.units.iter().map(|(bits, inst)| (bits, &inst.type_id))
    }

    /// Check if a unit entity has an animation instance.
    pub fn has_unit(&self, entity_bits: u64) -> bool {
        self.units.contains_key(&entity_bits)
    }
}

impl Default for CobAnimationDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Call a named script on the VM if it exists. Returns whether it was found.
fn vm_call_if_exists(vm: &mut CobVm, script: &CobScript, name: &str) -> bool {
    vm.call_script(script, name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/driver_tests.rs"]
mod tests;
