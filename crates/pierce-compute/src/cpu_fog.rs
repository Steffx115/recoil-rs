//! CPU implementation of fog-of-war computation.

use std::collections::BTreeMap;

use pierce_sim::compute::{FogCompute, FogGridParams, FogUnitInput};

/// CPU fog-of-war backend. Same algorithm as the inline code in pierce-sim.
pub struct CpuFogCompute;

impl FogCompute for CpuFogCompute {
    fn compute_fog(
        &mut self,
        params: &FogGridParams,
        units: &[FogUnitInput],
        previous_grids: &BTreeMap<u8, Vec<u8>>,
    ) -> BTreeMap<u8, Vec<u8>> {
        let w = params.width;
        let h = params.height;
        let cell_count = (w as usize) * (h as usize);
        let cell_raw = params.cell_size_raw;
        let half_cell_raw = cell_raw >> 1;

        // Initialize grids: copy previous, downgrade Visible(2) -> Explored(1).
        let mut grids: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
        for &team in &params.teams {
            let grid = if let Some(prev) = previous_grids.get(&team) {
                prev.iter()
                    .map(|&v| if v == 2 { 1 } else { v })
                    .collect()
            } else {
                vec![0u8; cell_count]
            };
            grids.insert(team, grid);
        }

        // Reveal cells around each unit.
        for unit in units {
            let grid = match grids.get_mut(&unit.team) {
                Some(g) => g,
                None => continue,
            };

            let cell_x = (unit.pos_x_raw >> 32) as i32;
            let cell_z = (unit.pos_z_raw >> 32) as i32;
            let range_cells = (unit.range_raw / cell_raw) as i32 + 1;
            let range_sq = unit.range_raw as i128 * unit.range_raw as i128;

            let min_x = cell_x.saturating_sub(range_cells).max(0) as u32;
            let max_x = ((cell_x + range_cells) as u32).min(w.saturating_sub(1));
            let min_y = cell_z.saturating_sub(range_cells).max(0) as u32;
            let max_y = ((cell_z + range_cells) as u32).min(h.saturating_sub(1));

            for gy in min_y..=max_y {
                let row_offset = gy * w;
                let center_z = (gy as i64) * cell_raw + half_cell_raw;
                let dz = center_z - unit.pos_z_raw;

                for gx in min_x..=max_x {
                    let center_x = (gx as i64) * cell_raw + half_cell_raw;
                    let dx = center_x - unit.pos_x_raw;

                    let dist_sq = (dx as i128) * (dx as i128) + (dz as i128) * (dz as i128);
                    if dist_sq <= range_sq {
                        let idx = (row_offset + gx) as usize;
                        if idx < grid.len() {
                            grid[idx] = 2; // Visible
                        }
                    }
                }
            }
        }

        grids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_fog_reveals_cells_within_range() {
        let params = FogGridParams {
            width: 10,
            height: 10,
            cell_size_raw: 1i64 << 32, // cell_size = 1.0
            teams: vec![0],
        };
        let units = vec![FogUnitInput {
            pos_x_raw: 5i64 << 32, // x=5.0
            pos_z_raw: 5i64 << 32, // z=5.0
            range_raw: 2i64 << 32, // range=2.0
            team: 0,
        }];
        let prev = BTreeMap::new();

        let mut compute = CpuFogCompute;
        let result = compute.compute_fog(&params, &units, &prev);

        let grid = result.get(&0).unwrap();
        // Center cell (5,5) should be visible
        assert_eq!(grid[5 * 10 + 5], 2);
        // Far corner (0,0) should be unexplored
        assert_eq!(grid[0], 0);
    }

    #[test]
    fn cpu_fog_preserves_explored() {
        let params = FogGridParams {
            width: 10,
            height: 10,
            cell_size_raw: 1i64 << 32,
            teams: vec![0],
        };

        // Previous frame had cell (5,5) visible
        let mut prev = BTreeMap::new();
        let mut prev_grid = vec![0u8; 100];
        prev_grid[5 * 10 + 5] = 2; // was Visible
        prev.insert(0u8, prev_grid);

        // No units this frame
        let units = vec![];

        let mut compute = CpuFogCompute;
        let result = compute.compute_fog(&params, &units, &prev);

        let grid = result.get(&0).unwrap();
        // Should be downgraded to Explored (1), not Unexplored (0)
        assert_eq!(grid[5 * 10 + 5], 1);
    }
}
