// Fog of war compute shader — integer-only arithmetic for determinism.
//
// Dispatch: workgroups(ceil(max_diameter/64), unit_count, 1)
// Each workgroup row handles one unit. Threads within the row process
// columns of the sight circle.

struct FogParams {
    width: u32,
    height: u32,
    cell_size_lo: i32,
    cell_size_hi: i32,
    half_cell_lo: i32,
    half_cell_hi: i32,
    unit_count: u32,
    team_count: u32,
}

struct FogUnit {
    pos_x_lo: i32,
    pos_x_hi: i32,
    pos_z_lo: i32,
    pos_z_hi: i32,
    range_lo: i32,
    range_hi: i32,
    team: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: FogParams;
@group(0) @binding(1) var<storage, read> units: array<FogUnit>;
@group(0) @binding(2) var<storage, read_write> grid: array<atomic<u32>>;

// --- Inline i64 helpers (minimal set for fog) ---

fn i64_sub_pair(a_lo: i32, a_hi: i32, b_lo: i32, b_hi: i32) -> vec2<i32> {
    let al = bitcast<u32>(a_lo);
    let bl = bitcast<u32>(b_lo);
    let diff = al - bl;
    let borrow = select(0i, 1i, al < bl);
    return vec2<i32>(bitcast<i32>(diff), a_hi - b_hi - borrow);
}

// Multiply two i64 (as hi/lo pairs) → i128 (vec4<i32>).
// Simplified: only computes enough for distance² comparison.
fn mul_wide(a_lo: i32, a_hi: i32, b_lo: i32, b_hi: i32) -> vec4<i32> {
    let al = bitcast<u32>(a_lo);
    let ah = bitcast<u32>(a_hi);
    let bl = bitcast<u32>(b_lo);
    let bh = bitcast<u32>(b_hi);

    // Split into 16-bit halves for a_lo * b_lo.
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

    // Cross terms: al*bh + ah*bl (contribute to w1).
    let cross = al * bh + ah * bl;
    let w1 = w1_base + bitcast<u32>(cross);

    // High term: ah*bh (contributes to w2).
    let w2 = ah * bh;

    return vec4<i32>(
        bitcast<i32>(w0),
        bitcast<i32>(w1),
        bitcast<i32>(w2),
        0i
    );
}

// Add two i128.
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

// i128 a <= b (signed).
fn le_128(a: vec4<i32>, b: vec4<i32>) -> bool {
    if (a.w != b.w) { return a.w < b.w; }
    if (bitcast<u32>(a.z) != bitcast<u32>(b.z)) { return bitcast<u32>(a.z) < bitcast<u32>(b.z); }
    if (bitcast<u32>(a.y) != bitcast<u32>(b.y)) { return bitcast<u32>(a.y) < bitcast<u32>(b.y); }
    return bitcast<u32>(a.x) <= bitcast<u32>(b.x);
}

@compute @workgroup_size(64, 1, 1)
fn fog_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let unit_idx = gid.y;
    if (unit_idx >= params.unit_count) {
        return;
    }

    let unit = units[unit_idx];
    let cell_size_lo = params.cell_size_lo;
    let cell_size_hi = params.cell_size_hi;

    // Cell coordinates of the unit (arithmetic right shift by 32 = take hi word).
    let cell_x = unit.pos_x_hi;
    let cell_z = unit.pos_z_hi;

    // Range in cells.
    // range / cell_size: for cell_size = 1.0 (raw = 1<<32), this is just range_hi.
    // For general cell_size, we'd need division. For now assume cell_size = 1.0.
    let range_cells = unit.range_hi + 1;

    // Squared range as i128.
    let range_sq = mul_wide(unit.range_lo, unit.range_hi, unit.range_lo, unit.range_hi);

    // This thread handles one column offset within the sight box.
    let col_offset = i32(gid.x);
    let gy = cell_z - range_cells + col_offset;
    if (gy < 0 || gy >= i32(params.height)) {
        return;
    }
    if (col_offset >= 2 * range_cells + 1) {
        return;
    }

    // Center Z of this cell row in fixed-point.
    let center_z_lo = i32(u32(gy) * bitcast<u32>(cell_size_lo)) + params.half_cell_lo;
    let center_z_hi = gy * cell_size_hi + params.half_cell_hi;
    let dz = i64_sub_pair(center_z_lo, center_z_hi, unit.pos_z_lo, unit.pos_z_hi);

    let min_x = max(cell_x - range_cells, 0);
    let max_x_val = min(cell_x + range_cells, i32(params.width) - 1);

    for (var gx = min_x; gx <= max_x_val; gx = gx + 1) {
        let center_x_lo = i32(u32(gx) * bitcast<u32>(cell_size_lo)) + params.half_cell_lo;
        let center_x_hi = gx * cell_size_hi + params.half_cell_hi;
        let dx = i64_sub_pair(center_x_lo, center_x_hi, unit.pos_x_lo, unit.pos_x_hi);

        let dx_sq = mul_wide(dx.x, dx.y, dx.x, dx.y);
        let dz_sq = mul_wide(dz.x, dz.y, dz.x, dz.y);
        let dist_sq = add_128(dx_sq, dz_sq);

        if (le_128(dist_sq, range_sq)) {
            let idx = u32(gy) * params.width + u32(gx);
            atomicMax(&grid[idx], 2u);
        }
    }
}
