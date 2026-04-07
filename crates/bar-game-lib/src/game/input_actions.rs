//! Input simulation helpers: selection, movement, building placement.

use bevy_ecs::entity::Entity;

use recoil_sim::{Dead, Position};

use crate::building::{self, PlacementType};

use super::GameState;

impl GameState {
    /// Enter placement mode for a building type.
    pub fn handle_build_command(&mut self, placement_type: PlacementType) {
        self.placement_mode = Some(placement_type);
    }

    /// Execute building placement at the given world position.
    /// Uses the currently selected builder (or commander as fallback).
    pub fn handle_place(&mut self, x: f32, z: f32) {
        if let Some(btype) = self.placement_mode.take() {
            // Use selected entity if it's a builder, otherwise fall back to commander.
            let builder = if self.selected_is_builder() {
                self.selected()
            } else {
                self.commander_team0
            };
            // Determine team from the builder entity.
            let team = builder
                .and_then(|e| self.world.get::<recoil_sim::Allegiance>(e))
                .map(|a| a.team)
                .unwrap_or(0);
            building::place_building(&mut self.world, builder, btype.0, x, z, team);
        }
    }

    /// Simulate left-click at world position: select nearest unit within radius.
    pub fn click_select(&mut self, x: f32, z: f32, radius: f32) -> Option<Entity> {
        let entity = self.find_unit_at(x, z, radius);
        if let Some(e) = entity {
            self.selection.select_single(e);
        } else {
            self.selection.clear();
        }
        entity
    }

    /// Shift-click to toggle a unit in the selection.
    pub fn click_select_toggle(&mut self, x: f32, z: f32, radius: f32) -> Option<Entity> {
        let entity = self.find_unit_at(x, z, radius);
        if let Some(e) = entity {
            self.selection.toggle(e);
        }
        entity
    }

    /// Box-select all units within a world-space rectangle.
    pub fn box_select(&mut self, x1: f32, z1: f32, x2: f32, z2: f32) {
        let min_x = x1.min(x2);
        let max_x = x1.max(x2);
        let min_z = z1.min(z2);
        let max_z = z1.max(z2);
        let entities: Vec<Entity> = self
            .world
            .query_filtered::<(Entity, &Position), bevy_ecs::query::Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, p)| {
                let px = p.pos.x.to_f32();
                let pz = p.pos.z.to_f32();
                px >= min_x && px <= max_x && pz >= min_z && pz <= max_z
            })
            .map(|(e, _)| e)
            .collect();
        self.selection.select_box(entities);
    }

    /// Save current selection to a control group slot (0-9).
    pub fn save_control_group(&mut self, slot: u8) {
        self.selection.save_control_group(slot);
    }

    /// Recall a control group slot (0-9).
    pub fn recall_control_group(&mut self, slot: u8) {
        self.selection.recall_control_group(slot);
    }

    /// Simulate right-click at world position: issue move command to all selected units.
    /// Returns number of units that received a move command.
    pub fn click_move(&mut self, target_x: f32, target_z: f32) -> bool {
        if let Some(sel) = self.selected() {
            if self.world.get_entity(sel).is_err() {
                return false;
            }
            if let Some(ms) = self.world.get_mut::<recoil_sim::MoveState>(sel) {
                *ms.into_inner() = recoil_sim::MoveState::MovingTo(recoil_math::SimVec3::new(
                    recoil_math::SimFloat::from_f32(target_x),
                    recoil_math::SimFloat::ZERO,
                    recoil_math::SimFloat::from_f32(target_z),
                ));
                return true;
            }
        }
        false
    }

    /// Find nearest alive unit at world position within radius.
    pub fn find_unit_at(&mut self, x: f32, z: f32, radius: f32) -> Option<Entity> {
        let radius_sq = radius * radius;
        let mut best: Option<(Entity, f32)> = None;
        for (entity, pos) in self
            .world
            .query_filtered::<(Entity, &Position), bevy_ecs::query::Without<Dead>>()
            .iter(&self.world)
        {
            let dx = pos.pos.x.to_f32() - x;
            let dz = pos.pos.z.to_f32() - z;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq <= radius_sq && (best.is_none() || dist_sq < best.unwrap().1) {
                best = Some((entity, dist_sq));
            }
        }
        best.map(|(e, _)| e)
    }
}
