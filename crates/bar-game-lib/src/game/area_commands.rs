//! Area commands: reclaim, repair, attack in a radius.

use bevy_ecs::query::Without;

use pierce_sim::{Dead, Health, Position};

use super::GameState;

impl GameState {
    /// Issue area reclaim: selected builders reclaim all reclaimable entities in radius.
    pub fn area_reclaim(&mut self, cx: f32, cz: f32, radius: f32) {
        use pierce_sim::commands::{Command, CommandQueue};
        use pierce_sim::construction::Reclaimable;

        let radius_sq = radius * radius;
        let targets: Vec<bevy_ecs::entity::Entity> = self
            .world
            .query_filtered::<(bevy_ecs::entity::Entity, &Position), bevy_ecs::query::With<Reclaimable>>()
            .iter(&self.world)
            .filter(|(_, p)| {
                let dx = p.pos.x.to_f32() - cx;
                let dz = p.pos.z.to_f32() - cz;
                dx * dx + dz * dz <= radius_sq
            })
            .map(|(e, _)| e)
            .collect();

        for &sel in &self.selection.selected.clone() {
            if let Some(mut cq) = self.world.get_mut::<CommandQueue>(sel) {
                for &target in &targets {
                    cq.push(Command::Reclaim(target));
                }
            }
        }
    }

    /// Issue area repair: selected builders repair all damaged friendlies in radius.
    pub fn area_repair(&mut self, cx: f32, cz: f32, radius: f32) {
        use pierce_sim::commands::{Command, CommandQueue};

        let radius_sq = radius * radius;
        let targets: Vec<bevy_ecs::entity::Entity> = self
            .world
            .query_filtered::<(
                bevy_ecs::entity::Entity,
                &Position,
                &Health,
                &pierce_sim::Allegiance,
            ), Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, p, hp, _)| {
                let dx = p.pos.x.to_f32() - cx;
                let dz = p.pos.z.to_f32() - cz;
                dx * dx + dz * dz <= radius_sq && hp.current < hp.max
            })
            .map(|(e, _, _, _)| e)
            .collect();

        for &sel in &self.selection.selected.clone() {
            if let Some(mut cq) = self.world.get_mut::<CommandQueue>(sel) {
                for &target in &targets {
                    cq.push(Command::Repair(target));
                }
            }
        }
    }

    /// Issue area attack: selected combat units attack all enemies in radius.
    pub fn area_attack(&mut self, cx: f32, cz: f32, radius: f32, my_team: u8) {
        use pierce_sim::commands::{Command, CommandQueue};

        let radius_sq = radius * radius;
        let targets: Vec<bevy_ecs::entity::Entity> = self
            .world
            .query_filtered::<(bevy_ecs::entity::Entity, &Position, &pierce_sim::Allegiance), Without<Dead>>()
            .iter(&self.world)
            .filter(|(_, p, al)| {
                let dx = p.pos.x.to_f32() - cx;
                let dz = p.pos.z.to_f32() - cz;
                dx * dx + dz * dz <= radius_sq && al.team != my_team
            })
            .map(|(e, _, _)| e)
            .collect();

        for &sel in &self.selection.selected.clone() {
            if let Some(mut cq) = self.world.get_mut::<CommandQueue>(sel) {
                for &target in &targets {
                    cq.push(Command::Attack(target));
                }
            }
        }
    }
}
