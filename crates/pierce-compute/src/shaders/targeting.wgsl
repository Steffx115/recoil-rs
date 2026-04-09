// Targeting compute shader — one thread per shooter, finds best target.
// Integer-only arithmetic for determinism.
//
// Dispatch: workgroups(ceil(shooter_count/64), 1, 1)

struct TargetingParams {
    shooter_count: u32,
    candidate_count: u32,
    fog_width: u32,
    fog_height: u32,
    fog_cell_size_lo: i32,
    fog_cell_size_hi: i32,
    has_fog: u32,
    _pad: u32,
}

struct Shooter {
    pos_x_lo: i32,
    pos_x_hi: i32,
    pos_y_lo: i32,
    pos_y_hi: i32,
    pos_z_lo: i32,
    pos_z_hi: i32,
    max_range_lo: i32,
    max_range_hi: i32,
    // min_ranges for up to 4 weapons (lo/hi pairs)
    min_range_0_lo: i32,
    min_range_0_hi: i32,
    min_range_1_lo: i32,
    min_range_1_hi: i32,
    min_range_2_lo: i32,
    min_range_2_hi: i32,
    min_range_3_lo: i32,
    min_range_3_hi: i32,
    team: u32,
    fire_mode: u32,        // 0=FireAtWill, 1=ReturnFire, 2=HoldFire
    has_indirect: u32,
    weapon_count: u32,
    manual_target_idx: i32,
    last_attacker_idx: i32,
    _pad0: u32,
    _pad1: u32,
}

struct Candidate {
    pos_x_lo: i32,
    pos_x_hi: i32,
    pos_y_lo: i32,
    pos_y_hi: i32,
    pos_z_lo: i32,
    pos_z_hi: i32,
    health_lo: i32,
    health_hi: i32,
    pending_damage_lo: i32,
    pending_damage_hi: i32,
    sim_id_lo: u32,
    sim_id_hi: u32,
    team: u32,
    flags: u32,  // bit 0: is_dead, bit 1: has_weapons, bit 2: is_building
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: TargetingParams;
@group(0) @binding(1) var<storage, read> shooters: array<Shooter>;
@group(0) @binding(2) var<storage, read> candidates: array<Candidate>;
@group(0) @binding(3) var<storage, read> fog_grid: array<u32>;
@group(0) @binding(4) var<storage, read_write> results: array<i32>;

// --- Inline i64/i128 helpers ---

fn sub64(a_lo: i32, a_hi: i32, b_lo: i32, b_hi: i32) -> vec2<i32> {
    let al = bitcast<u32>(a_lo);
    let bl = bitcast<u32>(b_lo);
    let diff = al - bl;
    let borrow = select(0i, 1i, al < bl);
    return vec2<i32>(bitcast<i32>(diff), a_hi - b_hi - borrow);
}

// i64 multiply to i128 (for distance²).
fn mul_wide(a_lo: i32, a_hi: i32, b_lo: i32, b_hi: i32) -> vec4<i32> {
    let al = bitcast<u32>(a_lo);
    let ah = bitcast<u32>(a_hi);
    let bl = bitcast<u32>(b_lo);
    let bh = bitcast<u32>(b_hi);

    let a0 = al & 0xFFFFu;
    let a1 = al >> 16u;
    let b0 = bl & 0xFFFFu;
    let b1 = bl >> 16u;

    let m00 = a0 * b0;
    let m01 = a0 * b1;
    let m10 = a1 * b0;
    let m11 = a1 * b1;

    let mid = m01 + (m00 >> 16u) + (m10 & 0xFFFFu);
    let w0 = (m00 & 0xFFFFu) | ((mid & 0xFFFFu) << 16u);
    let w1_base = (mid >> 16u) + (m10 >> 16u) + m11;

    let cross = al * bh + ah * bl;
    let w1 = w1_base + bitcast<u32>(cross);
    let w2 = ah * bh;

    return vec4<i32>(bitcast<i32>(w0), bitcast<i32>(w1), bitcast<i32>(w2), 0i);
}

fn add_128(a: vec4<i32>, b: vec4<i32>) -> vec4<i32> {
    let a0 = bitcast<u32>(a.x); let b0 = bitcast<u32>(b.x);
    let s0 = a0 + b0;
    let c0 = select(0u, 1u, s0 < a0);
    let s1 = bitcast<u32>(a.y) + bitcast<u32>(b.y) + c0;
    let c1 = select(0u, 1u, s1 < bitcast<u32>(a.y) + c0);
    let s2 = bitcast<u32>(a.z) + bitcast<u32>(b.z) + c1;
    let s3 = bitcast<u32>(a.w) + bitcast<u32>(b.w);
    return vec4<i32>(bitcast<i32>(s0), bitcast<i32>(s1), bitcast<i32>(s2), bitcast<i32>(s3));
}

// a <= b (signed i128)
fn le_128(a: vec4<i32>, b: vec4<i32>) -> bool {
    if (a.w != b.w) { return a.w < b.w; }
    if (bitcast<u32>(a.z) != bitcast<u32>(b.z)) { return bitcast<u32>(a.z) < bitcast<u32>(b.z); }
    if (bitcast<u32>(a.y) != bitcast<u32>(b.y)) { return bitcast<u32>(a.y) < bitcast<u32>(b.y); }
    return bitcast<u32>(a.x) <= bitcast<u32>(b.x);
}

// a > b (signed i128) — just !(a <= b)
fn gt_128(a: vec4<i32>, b: vec4<i32>) -> bool {
    return !le_128(a, b);
}

// Negate i128.
fn neg_128(a: vec4<i32>) -> vec4<i32> {
    // Two's complement: invert all bits, add 1.
    let n = vec4<u32>(~bitcast<u32>(a.x), ~bitcast<u32>(a.y), ~bitcast<u32>(a.z), ~bitcast<u32>(a.w));
    let s0 = n.x + 1u;
    let c0 = select(0u, 1u, s0 < n.x);
    let s1 = n.y + c0;
    let c1 = select(0u, 1u, s1 < n.y);
    let s2 = n.z + c1;
    let c2 = select(0u, 1u, s2 < n.z);
    let s3 = n.w + c2;
    return vec4<i32>(bitcast<i32>(s0), bitcast<i32>(s1), bitcast<i32>(s2), bitcast<i32>(s3));
}

// i64 a <= b (signed).
fn le_64(a_lo: i32, a_hi: i32, b_lo: i32, b_hi: i32) -> bool {
    if (a_hi != b_hi) { return a_hi < b_hi; }
    return bitcast<u32>(a_lo) <= bitcast<u32>(b_lo);
}

// Check fog visibility for a candidate position.
fn is_fog_visible(team: u32, pos_x_hi: i32, pos_z_hi: i32) -> bool {
    if (params.has_fog == 0u) { return true; }
    let cx = pos_x_hi; // cell = floor(pos / cell_size), for cell_size=1.0 this is pos_hi
    let cz = pos_z_hi;
    if (cx < 0 || cz < 0) { return true; }
    let ucx = u32(cx);
    let ucz = u32(cz);
    if (ucx >= params.fog_width || ucz >= params.fog_height) { return true; }
    // fog_grid stores one u32 per cell, indexed as: team*width*height + z*width + x
    let idx = team * params.fog_width * params.fog_height + ucz * params.fog_width + ucx;
    return fog_grid[idx] == 2u; // 2 = Visible
}

@compute @workgroup_size(64, 1, 1)
fn targeting_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let sid = gid.x;
    if (sid >= params.shooter_count) { return; }

    let s = shooters[sid];

    // HoldFire: no target (unless manual override).
    if (s.fire_mode == 2u && s.manual_target_idx < 0) {
        results[sid] = -1;
        return;
    }

    // Manual target override.
    if (s.manual_target_idx >= 0) {
        results[sid] = s.manual_target_idx;
        return;
    }

    // ReturnFire: only target last attacker.
    if (s.fire_mode == 1u) {
        if (s.last_attacker_idx >= 0) {
            let ci = u32(s.last_attacker_idx);
            if (ci < params.candidate_count) {
                let c = candidates[ci];
                let is_dead = (c.flags & 1u) != 0u;
                if (!is_dead && c.team != s.team) {
                    // Check health > 0 (hi > 0, or hi == 0 && lo > 0).
                    let hp_pos = c.health_hi > 0 || (c.health_hi == 0 && bitcast<u32>(c.health_lo) > 0u);
                    if (hp_pos) {
                        let dx = sub64(s.pos_x_lo, s.pos_x_hi, c.pos_x_lo, c.pos_x_hi);
                        let dz = sub64(s.pos_z_lo, s.pos_z_hi, c.pos_z_lo, c.pos_z_hi);
                        let dist_sq = add_128(mul_wide(dx.x, dx.y, dx.x, dx.y), mul_wide(dz.x, dz.y, dz.x, dz.y));
                        let range_sq = mul_wide(s.max_range_lo, s.max_range_hi, s.max_range_lo, s.max_range_hi);
                        if (le_128(dist_sq, range_sq)) {
                            results[sid] = s.last_attacker_idx;
                            return;
                        }
                    }
                }
            }
        }
        results[sid] = -1;
        return;
    }

    // FireAtWill: find best target.
    let range_sq = mul_wide(s.max_range_lo, s.max_range_hi, s.max_range_lo, s.max_range_hi);

    var best_idx: i32 = -1;
    var best_priority: i32 = -2147483647; // i32::MIN + 1
    var best_threat: i32 = -2147483647;
    var best_neg_dist_sq: vec4<i32> = vec4<i32>(-2147483647, -2147483647, -2147483647, -2147483647);
    var best_sim_id_lo: u32 = 0xFFFFFFFFu;
    var best_sim_id_hi: u32 = 0xFFFFFFFFu;

    for (var ci = 0u; ci < params.candidate_count; ci = ci + 1u) {
        let c = candidates[ci];

        // Skip allies.
        if (c.team == s.team) { continue; }

        // Skip dead.
        let is_dead = (c.flags & 1u) != 0u;
        if (is_dead) { continue; }

        // Skip zero health.
        let hp_pos = c.health_hi > 0 || (c.health_hi == 0 && bitcast<u32>(c.health_lo) > 0u);
        if (!hp_pos) { continue; }

        // Fog visibility.
        if (!is_fog_visible(s.team, c.pos_x_hi, c.pos_z_hi)) { continue; }

        // Distance check.
        let dx = sub64(s.pos_x_lo, s.pos_x_hi, c.pos_x_lo, c.pos_x_hi);
        let dz = sub64(s.pos_z_lo, s.pos_z_hi, c.pos_z_lo, c.pos_z_hi);
        let dist_sq = add_128(mul_wide(dx.x, dx.y, dx.x, dx.y), mul_wide(dz.x, dz.y, dz.x, dz.y));

        if (gt_128(dist_sq, range_sq)) { continue; }

        // Min range check.
        if (s.weapon_count > 0u) {
            var any_in_range = false;
            // Unrolled for up to 4 weapons.
            if (s.weapon_count >= 1u) {
                let mr_sq = mul_wide(s.min_range_0_lo, s.min_range_0_hi, s.min_range_0_lo, s.min_range_0_hi);
                if (!gt_128(mr_sq, dist_sq)) { any_in_range = true; }
            }
            if (!any_in_range && s.weapon_count >= 2u) {
                let mr_sq = mul_wide(s.min_range_1_lo, s.min_range_1_hi, s.min_range_1_lo, s.min_range_1_hi);
                if (!gt_128(mr_sq, dist_sq)) { any_in_range = true; }
            }
            if (!any_in_range && s.weapon_count >= 3u) {
                let mr_sq = mul_wide(s.min_range_2_lo, s.min_range_2_hi, s.min_range_2_lo, s.min_range_2_hi);
                if (!gt_128(mr_sq, dist_sq)) { any_in_range = true; }
            }
            if (!any_in_range && s.weapon_count >= 4u) {
                let mr_sq = mul_wide(s.min_range_3_lo, s.min_range_3_hi, s.min_range_3_lo, s.min_range_3_hi);
                if (!gt_128(mr_sq, dist_sq)) { any_in_range = true; }
            }
            if (!any_in_range) { continue; }
        }

        // Overkill avoidance: pending_damage >= health.
        if (le_64(c.health_lo, c.health_hi, c.pending_damage_lo, c.pending_damage_hi)) { continue; }

        // Scoring.
        let has_weapons = (c.flags & 2u) != 0u;
        let is_building = (c.flags & 4u) != 0u;

        var priority: i32 = 0;
        if (has_weapons && !is_building) { priority = 10; }
        else if (is_building) { priority = 5; }

        var threat: i32 = 1;
        if (has_weapons && !is_building) { threat = 3; }
        else if (has_weapons && is_building) { threat = 2; }

        let neg_dist_sq = neg_128(dist_sq);

        // Compare: priority > threat > closer > lower sim_id.
        var better = false;
        if (priority > best_priority) {
            better = true;
        } else if (priority == best_priority) {
            if (threat > best_threat) {
                better = true;
            } else if (threat == best_threat) {
                if (gt_128(neg_dist_sq, best_neg_dist_sq)) {
                    better = true;
                } else if (neg_dist_sq.x == best_neg_dist_sq.x && neg_dist_sq.y == best_neg_dist_sq.y
                        && neg_dist_sq.z == best_neg_dist_sq.z && neg_dist_sq.w == best_neg_dist_sq.w) {
                    // Tie on distance: lower sim_id wins.
                    if (c.sim_id_hi < best_sim_id_hi || (c.sim_id_hi == best_sim_id_hi && c.sim_id_lo < best_sim_id_lo)) {
                        better = true;
                    }
                }
            }
        }

        if (better) {
            best_idx = i32(ci);
            best_priority = priority;
            best_threat = threat;
            best_neg_dist_sq = neg_dist_sq;
            best_sim_id_lo = c.sim_id_lo;
            best_sim_id_hi = c.sim_id_hi;
        }
    }

    results[sid] = best_idx;
}
