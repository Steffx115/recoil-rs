//! CPU implementation of targeting computation.

use std::collections::BTreeMap;

use pierce_sim::compute::{TargetCompute, TargetingCandidateInput, TargetingShooterInput};

/// CPU targeting backend. Mirrors the scoring logic from pierce-sim's targeting_system.
pub struct CpuTargetCompute;

impl TargetCompute for CpuTargetCompute {
    fn compute_targets(
        &mut self,
        shooters: &[TargetingShooterInput],
        candidates: &[TargetingCandidateInput],
        fog_grids: Option<&BTreeMap<u8, Vec<u8>>>,
        fog_width: u32,
        _fog_height: u32,
        fog_cell_size_raw: i64,
    ) -> Vec<i32> {
        let mut results = Vec::with_capacity(shooters.len());

        for shooter in shooters {
            // HoldFire: no auto-target.
            if shooter.fire_mode == 2 && shooter.manual_target_idx < 0 {
                results.push(-1);
                continue;
            }

            // Manual target override.
            if shooter.manual_target_idx >= 0 {
                results.push(shooter.manual_target_idx);
                continue;
            }

            // ReturnFire: only target last attacker.
            if shooter.fire_mode == 1 {
                if shooter.last_attacker_idx >= 0 {
                    let idx = shooter.last_attacker_idx as usize;
                    if idx < candidates.len() {
                        let c = &candidates[idx];
                        if !c.is_dead && c.health_raw > 0 && c.team != shooter.team {
                            let dx = shooter.pos_x_raw - c.pos_x_raw;
                            let dz = shooter.pos_z_raw - c.pos_z_raw;
                            let dist_sq = (dx as i128) * (dx as i128) + (dz as i128) * (dz as i128);
                            let range_sq = (shooter.max_range_raw as i128) * (shooter.max_range_raw as i128);
                            if dist_sq <= range_sq {
                                results.push(shooter.last_attacker_idx);
                                continue;
                            }
                        }
                    }
                }
                results.push(-1);
                continue;
            }

            // FireAtWill: find best target.
            let range_sq = (shooter.max_range_raw as i128) * (shooter.max_range_raw as i128);

            // Score tuple: (priority_bonus, threat, neg_dist_sq, neg_sim_id)
            // Higher is better. We use i128 for distance comparison.
            let mut best_idx: i32 = -1;
            let mut best_priority: i64 = i64::MIN;
            let mut best_threat: i64 = i64::MIN;
            let mut best_neg_dist_sq: i128 = i128::MIN;
            let mut best_sim_id: u64 = u64::MAX;

            for (ci, c) in candidates.iter().enumerate() {
                if c.team == shooter.team {
                    continue;
                }
                if c.is_dead || c.health_raw <= 0 {
                    continue;
                }

                // Fog visibility check.
                if let Some(grids) = fog_grids {
                    if fog_cell_size_raw > 0 {
                        let cx = (c.pos_x_raw / fog_cell_size_raw) as i32;
                        let cz = (c.pos_z_raw / fog_cell_size_raw) as i32;
                        if cx >= 0 && cz >= 0 {
                            let ucx = cx as u32;
                            let ucz = cz as u32;
                            if ucx < fog_width {
                                if let Some(grid) = grids.get(&shooter.team) {
                                    let idx = (ucz as usize) * (fog_width as usize) + (ucx as usize);
                                    if idx < grid.len() && grid[idx] != 2 {
                                        continue; // Not visible
                                    }
                                }
                            }
                        }
                    }
                }

                let dx = shooter.pos_x_raw - c.pos_x_raw;
                let dz = shooter.pos_z_raw - c.pos_z_raw;
                let dist_sq = (dx as i128) * (dx as i128) + (dz as i128) * (dz as i128);

                // Range check.
                if dist_sq > range_sq {
                    continue;
                }

                // Min range check: at least one weapon must be in range.
                let mut any_in_range = false;
                for wi in 0..shooter.weapon_count as usize {
                    let min_r = shooter.weapon_min_ranges[wi];
                    let min_sq = (min_r as i128) * (min_r as i128);
                    if dist_sq >= min_sq {
                        any_in_range = true;
                        break;
                    }
                }
                if shooter.weapon_count > 0 && !any_in_range {
                    continue;
                }

                // Overkill avoidance.
                if c.pending_damage_raw >= c.health_raw {
                    continue;
                }

                // Scoring: priority bonus (fixed-point raw values).
                let priority: i64 = if c.has_weapons && !c.is_building {
                    10i64 << 32 // AntiAir-like bonus
                } else if c.is_building {
                    5i64 << 32
                } else {
                    0
                };

                // Threat: armed mobile > armed building > unarmed.
                let threat: i64 = if c.has_weapons && !c.is_building {
                    3i64 << 32
                } else if c.has_weapons && c.is_building {
                    2i64 << 32
                } else {
                    1i64 << 32
                };

                let neg_dist_sq = -dist_sq;

                // Compare: priority > threat > closer > lower sim_id.
                let better = priority > best_priority
                    || (priority == best_priority && threat > best_threat)
                    || (priority == best_priority
                        && threat == best_threat
                        && neg_dist_sq > best_neg_dist_sq)
                    || (priority == best_priority
                        && threat == best_threat
                        && neg_dist_sq == best_neg_dist_sq
                        && c.sim_id < best_sim_id);

                if better {
                    best_idx = ci as i32;
                    best_priority = priority;
                    best_threat = threat;
                    best_neg_dist_sq = neg_dist_sq;
                    best_sim_id = c.sim_id;
                }
            }

            results.push(best_idx);
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shooter(x: i32, z: i32, team: u8, range: i32) -> TargetingShooterInput {
        TargetingShooterInput {
            index: 0,
            pos_x_raw: (x as i64) << 32,
            pos_y_raw: 0,
            pos_z_raw: (z as i64) << 32,
            team,
            max_range_raw: (range as i64) << 32,
            fire_mode: 0, // FireAtWill
            has_indirect: false,
            manual_target_idx: -1,
            last_attacker_idx: -1,
            weapon_min_ranges: [0; 4],
            weapon_count: 1,
        }
    }

    fn make_candidate(
        x: i32,
        z: i32,
        team: u8,
        hp: i32,
        sim_id: u64,
    ) -> TargetingCandidateInput {
        TargetingCandidateInput {
            pos_x_raw: (x as i64) << 32,
            pos_y_raw: 0,
            pos_z_raw: (z as i64) << 32,
            team,
            is_dead: false,
            health_raw: (hp as i64) << 32,
            sim_id,
            has_weapons: true,
            is_building: false,
            pending_damage_raw: 0,
        }
    }

    #[test]
    fn targets_nearest_enemy() {
        let shooters = vec![make_shooter(0, 0, 0, 500)];
        let candidates = vec![
            make_candidate(100, 0, 1, 100, 1), // closer
            make_candidate(200, 0, 1, 100, 2), // farther
        ];

        let mut compute = CpuTargetCompute;
        let results = compute.compute_targets(&shooters, &candidates, None, 0, 0, 0);

        assert_eq!(results[0], 0); // nearest enemy
    }

    #[test]
    fn skips_allies() {
        let shooters = vec![make_shooter(0, 0, 0, 500)];
        let candidates = vec![
            make_candidate(50, 0, 0, 100, 1),  // ally
            make_candidate(100, 0, 1, 100, 2), // enemy
        ];

        let mut compute = CpuTargetCompute;
        let results = compute.compute_targets(&shooters, &candidates, None, 0, 0, 0);

        assert_eq!(results[0], 1); // skips ally, targets enemy
    }

    #[test]
    fn hold_fire_no_target() {
        let mut shooter = make_shooter(0, 0, 0, 500);
        shooter.fire_mode = 2; // HoldFire
        let candidates = vec![make_candidate(50, 0, 1, 100, 1)];

        let mut compute = CpuTargetCompute;
        let results = compute.compute_targets(&[shooter], &candidates, None, 0, 0, 0);

        assert_eq!(results[0], -1);
    }
}
