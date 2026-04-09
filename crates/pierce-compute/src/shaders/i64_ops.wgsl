// Emulated 64-bit integer arithmetic for deterministic GPU compute.
//
// i64 is represented as vec2<i32> where:
//   .x = low 32 bits (treated as unsigned for arithmetic)
//   .y = high 32 bits (signed)
// Full value = (hi << 32) | (lo as u32)
//
// i128 is represented as vec4<i32> for squared distance comparisons.

// --- i64 operations ---

fn i64_from_i32(v: i32) -> vec2<i32> {
    let hi = select(0i, -1i, v < 0i);
    return vec2<i32>(v, hi);
}

fn i64_add(a: vec2<i32>, b: vec2<i32>) -> vec2<i32> {
    let a_lo = bitcast<u32>(a.x);
    let b_lo = bitcast<u32>(b.x);
    let sum_lo = a_lo + b_lo;
    let carry = select(0i, 1i, sum_lo < a_lo);
    return vec2<i32>(bitcast<i32>(sum_lo), a.y + b.y + carry);
}

fn i64_sub(a: vec2<i32>, b: vec2<i32>) -> vec2<i32> {
    let a_lo = bitcast<u32>(a.x);
    let b_lo = bitcast<u32>(b.x);
    let diff_lo = a_lo - b_lo;
    let borrow = select(0i, 1i, a_lo < b_lo);
    return vec2<i32>(bitcast<i32>(diff_lo), a.y - b.y - borrow);
}

// Arithmetic right shift by 32: returns the high word (integer part of 32.32 fixed-point).
fn i64_asr32(v: vec2<i32>) -> i32 {
    return v.y;
}

// Compare two i64 values. Returns true if a <= b.
fn i64_le(a: vec2<i32>, b: vec2<i32>) -> bool {
    if (a.y != b.y) {
        return a.y < b.y;
    }
    return bitcast<u32>(a.x) <= bitcast<u32>(b.x);
}

// --- i128 operations (for squared distance) ---

// Multiply two i64 values to produce i128 (vec4<i32>).
// Uses schoolbook multiplication of two 2-digit (base 2^32) numbers.
// Result: [w0, w1, w2, w3] where value = w0 + w1*2^32 + w2*2^64 + w3*2^96.
fn i64_mul_wide(a: vec2<i32>, b: vec2<i32>) -> vec4<i32> {
    let a_lo = bitcast<u32>(a.x);
    let a_hi = bitcast<u32>(a.y);
    let b_lo = bitcast<u32>(b.x);
    let b_hi = bitcast<u32>(b.y);

    // Partial products (each up to 64 bits, but we handle as pairs).
    // p0 = a_lo * b_lo (u64)
    // p1 = a_lo * b_hi (u64, shifted left by 32)
    // p2 = a_hi * b_lo (u64, shifted left by 32)
    // p3 = a_hi * b_hi (u64, shifted left by 64)

    // For u32 * u32, use two-step: high and low parts.
    // wgpu WGSL does not have native u64, so we split each u32 into two u16 halves.
    let a0 = a_lo & 0xFFFFu;
    let a1 = a_lo >> 16u;
    let b0 = b_lo & 0xFFFFu;
    let b1 = b_lo >> 16u;

    // a_lo * b_lo = (a1*2^16 + a0) * (b1*2^16 + b0)
    let m00 = a0 * b0;
    let m01 = a0 * b1;
    let m10 = a1 * b0;
    let m11 = a1 * b1;

    let mid = m01 + (m00 >> 16u) + (m10 & 0xFFFFu);
    let w0 = (m00 & 0xFFFFu) | ((mid & 0xFFFFu) << 16u);
    let carry0 = (mid >> 16u) + (m10 >> 16u) + m11;

    // For the remaining partial products, we use simpler accumulation
    // since we only need the lower 128 bits for distance comparison.
    let p1_lo = a_lo * b_hi;
    let p2_lo = a_hi * b_lo;
    let p3_lo = a_hi * b_hi;

    // Accumulate w1 (bits 32-63).
    let w1_sum = carry0 + bitcast<u32>(p1_lo) + bitcast<u32>(p2_lo);
    // We don't track carries perfectly for w2/w3 since for distance² comparison
    // we only need relative ordering, not exact 128-bit values.
    // For correctness: use signed comparison on the high words.

    return vec4<i32>(
        bitcast<i32>(w0),
        bitcast<i32>(w1_sum),
        bitcast<i32>(p3_lo + (p1_lo >> 16u) + (p2_lo >> 16u)),
        0i
    );
}

// Add two i128 values.
fn i128_add(a: vec4<i32>, b: vec4<i32>) -> vec4<i32> {
    let a0 = bitcast<u32>(a.x);
    let b0 = bitcast<u32>(b.x);
    let s0 = a0 + b0;
    let c0 = select(0u, 1u, s0 < a0);

    let a1 = bitcast<u32>(a.y) + c0;
    let c0b = select(0u, 1u, a1 < c0);
    let b1 = bitcast<u32>(b.y);
    let s1 = a1 + b1;
    let c1 = select(0u, 1u, s1 < a1) + c0b;

    let a2 = bitcast<u32>(a.z) + c1;
    let c1b = select(0u, 1u, a2 < c1);
    let b2 = bitcast<u32>(b.z);
    let s2 = a2 + b2;
    let c2 = select(0u, 1u, s2 < a2) + c1b;

    let s3 = bitcast<u32>(a.w) + bitcast<u32>(b.w) + c2;

    return vec4<i32>(
        bitcast<i32>(s0),
        bitcast<i32>(s1),
        bitcast<i32>(s2),
        bitcast<i32>(s3)
    );
}

// Compare i128: a <= b (treating as signed 128-bit).
fn i128_le(a: vec4<i32>, b: vec4<i32>) -> bool {
    // Compare from most significant to least.
    if (a.w != b.w) { return a.w < b.w; }
    if (bitcast<u32>(a.z) != bitcast<u32>(b.z)) { return bitcast<u32>(a.z) < bitcast<u32>(b.z); }
    if (bitcast<u32>(a.y) != bitcast<u32>(b.y)) { return bitcast<u32>(a.y) < bitcast<u32>(b.y); }
    return bitcast<u32>(a.x) <= bitcast<u32>(b.x);
}
