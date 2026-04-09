// Batch math compute shaders — integer-only for determinism.
// Each operation has its own entry point.

struct BatchParams {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> params: BatchParams;

// --- i64 helpers (vec2<i32> = [lo, hi]) ---

fn i64_add(a: vec2<i32>, b: vec2<i32>) -> vec2<i32> {
    let al = bitcast<u32>(a.x);
    let bl = bitcast<u32>(b.x);
    let s = al + bl;
    let c = select(0i, 1i, s < al);
    return vec2<i32>(bitcast<i32>(s), a.y + b.y + c);
}

fn i64_sub(a: vec2<i32>, b: vec2<i32>) -> vec2<i32> {
    let al = bitcast<u32>(a.x);
    let bl = bitcast<u32>(b.x);
    let d = al - bl;
    let borrow = select(0i, 1i, al < bl);
    return vec2<i32>(bitcast<i32>(d), a.y - b.y - borrow);
}

// Fixed-point multiply: (a * b) >> 32. Result is the middle 64 bits of the 128-bit product.
fn i64_fpmul(a: vec2<i32>, b: vec2<i32>) -> vec2<i32> {
    let al = bitcast<u32>(a.x);
    let ah = bitcast<u32>(a.y);
    let bl = bitcast<u32>(b.x);
    let bh = bitcast<u32>(b.y);

    // Schoolbook: al*bl(64) + (al*bh + ah*bl)(64) << 32 + ah*bh(64) << 64
    // We need bits [32..95] of the 128-bit product.

    // al * bl -> 64-bit, split into 16-bit halves to avoid overflow.
    let a0 = al & 0xFFFFu;
    let a1 = al >> 16u;
    let b0 = bl & 0xFFFFu;
    let b1 = bl >> 16u;
    let m00 = a0 * b0;
    let m01 = a0 * b1;
    let m10 = a1 * b0;
    let m11 = a1 * b1;

    let mid = m01 + (m00 >> 16u) + (m10 & 0xFFFFu);
    let w0_hi = mid >> 16u;  // upper 16 bits of w0 -> carry into w1
    let w1_base = w0_hi + (m10 >> 16u) + m11;

    // Cross terms: al*bh + ah*bl contribute to w1.
    let cross = al * bh + ah * bl;
    let w1 = w1_base + bitcast<u32>(cross);

    // High term: ah*bh contributes to w2.
    let w2 = ah * bh;

    // Result = (w1, w2) = bits [32..95].
    // But we need sign correction: if the product is negative (sign of a XOR sign of b),
    // the upper words need adjustment. For signed fixed-point multiply, the standard
    // approach: compute as unsigned, then subtract if either operand was negative.
    var r = vec2<i32>(bitcast<i32>(w1), bitcast<i32>(w2));

    // Sign correction for signed multiplication.
    if (a.y < 0) {
        r = i64_sub(r, b);
    }
    if (b.y < 0) {
        r = i64_sub(r, a);
    }

    return r;
}

fn i64_neg(a: vec2<i32>) -> vec2<i32> {
    let nl = ~bitcast<u32>(a.x) + 1u;
    let c = select(0i, 1i, nl == 0u);
    return vec2<i32>(bitcast<i32>(nl), ~a.y + c);
}

fn i64_abs(a: vec2<i32>) -> vec2<i32> {
    if (a.y < 0) { return i64_neg(a); }
    return a;
}

fn i64_gt(a: vec2<i32>, b: vec2<i32>) -> bool {
    if (a.y != b.y) { return a.y > b.y; }
    return bitcast<u32>(a.x) > bitcast<u32>(b.x);
}

fn i64_le(a: vec2<i32>, b: vec2<i32>) -> bool {
    return !i64_gt(a, b);
}

fn i64_is_zero(a: vec2<i32>) -> bool {
    return a.x == 0 && a.y == 0;
}

// Arithmetic right shift by N bits (for small N < 32).
fn i64_asr(a: vec2<i32>, n: u32) -> vec2<i32> {
    let lo = (bitcast<u32>(a.x) >> n) | (bitcast<u32>(a.y) << (32u - n));
    let hi = a.y >> n;
    return vec2<i32>(bitcast<i32>(lo), hi);
}

// --- Entry points ---

// ============ batch_distance_sq ============
// Bindings: 1=ax, 2=az, 3=bx, 4=bz, 5=results (all storage<array<vec2<i32>>>)

@group(0) @binding(1) var<storage, read> dist_ax: array<vec2<i32>>;
@group(0) @binding(2) var<storage, read> dist_az: array<vec2<i32>>;
@group(0) @binding(3) var<storage, read> dist_bx: array<vec2<i32>>;
@group(0) @binding(4) var<storage, read> dist_bz: array<vec2<i32>>;
@group(0) @binding(5) var<storage, read_write> dist_results: array<vec2<i32>>;

@compute @workgroup_size(64)
fn batch_distance_sq_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }

    let dx = i64_sub(dist_ax[i], dist_bx[i]);
    let dz = i64_sub(dist_az[i], dist_bz[i]);
    let dx_sq = i64_fpmul(dx, dx);
    let dz_sq = i64_fpmul(dz, dz);
    dist_results[i] = i64_add(dx_sq, dz_sq);
}

// ============ batch_integrate ============
// pos += vel. Bindings: 1=pos_x, 2=pos_y, 3=pos_z, 4=vel_x, 5=vel_y, 6=vel_z (read_write + read)

// Note: integrate uses different bindings than distance_sq.
// Each entry point uses its own bind group, so binding indices can overlap.

// We define a separate bind group layout for integrate.
// For simplicity in this shader, integrate shares the same uniform (binding 0 = params).
// The Rust code creates separate bind groups per operation.

// ============ batch_mul ============
// a * b. Bindings: 1=a, 2=b, 3=results

// ============ batch_sincos ============
// PLACEHOLDER — complex, requires SIN_TABLE upload. Deferred to gpu_batch.rs.

// ============ batch_heading ============
// PLACEHOLDER — CORDIC atan2. Deferred to gpu_batch.rs.
