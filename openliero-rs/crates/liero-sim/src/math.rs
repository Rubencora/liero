//! Math primitives — exact port of `src/game/math.cpp`.
//!
//! # cossinTable
//! 128-entry direction table computed with the same Taylor-series integer
//! arithmetic as C++ `precomputeTables()`.  Every entry `(x, y)` satisfies:
//!   x = round(-sin(i * 2π/128) * 65536)
//!   y = round( cos(i * 2π/128) * 65536)
//!
//! Computed as a `const` so the values live in the binary and require no
//! runtime initialisation.
//!
//! # vectorLength
//! Integer Euclidean distance using the same digit-by-digit sqrt as C++ `sqr()`.

// ---------------------------------------------------------------------------
// cossinTable
// ---------------------------------------------------------------------------

/// Advance `s` (with `bits` fractional bits) toward `tobits` fractional bits.
/// Mirrors C++ `FP::reducedfrac(int tobits)`.
const fn fp_reducedfrac(mut s: i64, mut bits: i32, tobits: i32) -> i64 {
    while bits > 60 {
        s >>= 1;
        bits -= 1;
    }
    // tobits - bits must be non-negative here (tobits = 60, bits <= 60).
    s << (tobits - bits)
}

/// Reduce `s` until it fits within `[-2^tobits - 1, 2^tobits]`.
/// Mirrors C++ `FP::reduce(int tobits)`.
const fn fp_reduce(mut s: i64, mut bits: i32, tobits: i32) -> (i64, i32) {
    let lim = 1i64 << tobits;
    while s < (-lim - 1) || s > lim {
        s >>= 1;
        bits -= 1;
    }
    (s, bits)
}

/// Compute the 128-entry cossin table at compile time.
/// Mirrors C++ `precomputeTables()` in `src/game/math.cpp`.
const fn compute_cossin_table() -> [(i32, i32); 128] {
    // (2π / 128) << 28, matching the C++ constant.
    const SCALE_BITS: i32 = 28;
    const SCALE: i32 = 13176795;

    let mut table = [(0i32, 0i32); 128];
    let mut i: usize = 0;

    while i < 128 {
        let mut rf: i64 = 0;
        let mut c: i32 = -1;
        // xf = i * (2π/128) in fixed-point with SCALE_BITS fractional bits.
        let xf: i32 = (i as i32).wrapping_mul(SCALE);

        // Taylor series accumulator: starts at (xf, SCALE_BITS).
        let mut fp_s: i64 = xf as i64;
        let mut fp_bits: i32 = SCALE_BITS;

        // Compute -sin(xf) via Taylor series:
        //   -x + x³/3! - x⁵/5! + …
        // Loop variable `t` is the denominator factorial counter.
        let mut t: i32 = 1;
        while t < 26 {
            // Accumulate current term.
            let frac = fp_reducedfrac(fp_s, fp_bits, 60);
            rf = rf.wrapping_add((c as i64).wrapping_mul(frac));

            // Advance to next odd power: divide by t+1, multiply by xf.
            t += 1;
            fp_s /= t as i64;
            let (s, b) = fp_reduce(fp_s, fp_bits, 31);
            fp_s = s;
            fp_bits = b;
            fp_s = fp_s.wrapping_mul(xf as i64);
            fp_bits += SCALE_BITS;

            // Divide by t+1, multiply by xf again (two-step per Taylor term).
            t += 1;
            fp_s /= t as i64;
            let (s, b) = fp_reduce(fp_s, fp_bits, 31);
            fp_s = s;
            fp_bits = b;
            fp_s = fp_s.wrapping_mul(xf as i64);
            fp_bits += SCALE_BITS;

            c = -c;
        }

        // Convert to Q16.16 (shift = 60 - 16 = 44) with rounding.
        const SHIFT: i32 = 44;
        rf = rf.wrapping_add(1i64 << (SHIFT - 1));
        let r = (rf >> SHIFT) as i32;

        // x slot gets -sin; the cos slot is 32 entries later.
        table[i].0 = r;
        table[(i + 32) & 0x7f].1 = r;

        i += 1;
    }

    table
}

/// Precomputed 128-entry cos/sin direction table.
///
/// Entry `i` stores `(-sin(i·2π/128)·65536, cos(i·2π/128)·65536)` as `(i32, i32)`.
///
/// Matches the C++ `fixedvec cossinTable[128]` produced by `precomputeTables()`.
pub const COSSIN_TABLE: [(i32, i32); 128] = compute_cossin_table();

// ---------------------------------------------------------------------------
// Integer square root  (`sqr` in C++ — the name is historical)
// ---------------------------------------------------------------------------

/// Integer square root using the digit-by-digit (Babylonian) method.
///
/// Mirrors C++ `sqr(uint32_t op)` in `src/game/math.cpp`.
/// Computes `floor(sqrt(op))`.
#[inline]
pub fn isqrt(mut op: u32) -> u32 {
    let mut res: u32 = 0;
    let mut one: u32 = 1u32 << 30; // Highest power of 4 that fits in u32.

    // Find the highest power of 4 ≤ op.
    while one > op {
        one >>= 2;
    }

    while one != 0 {
        if op >= res + one {
            op -= res + one;
            res += 2 * one;
        }
        res >>= 1;
        one >>= 2;
    }
    res
}

/// Integer Euclidean distance.
///
/// Mirrors C++ `vectorLength(int x, int y)` in `src/game/math.cpp`.
/// Uses wrapping multiplication to replicate C++ `int` overflow behaviour.
#[inline]
pub fn vector_length(x: i32, y: i32) -> i32 {
    let sq = x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y));
    isqrt(sq as u32) as i32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// angle 0 → x≈0, y≈65536 (pointing "down" in screen coords)
    #[test]
    fn cossin_angle_0() {
        let (x, y) = COSSIN_TABLE[0];
        assert_eq!(x, 0, "angle 0 x should be 0 (= -sin 0)");
        assert_eq!(y, 65536, "angle 0 y should be 65536 (= cos 0)");
    }

    /// angle 32 → x≈-65536, y≈0 (= -sin(π/2), cos(π/2))
    #[test]
    fn cossin_angle_32() {
        let (x, y) = COSSIN_TABLE[32];
        // -sin(π/2) = -1.0 → -65536
        assert!((x + 65536).abs() <= 1, "angle 32 x ≈ -65536, got {x}");
        // cos(π/2) = 0 → 0
        assert!(y.abs() <= 1, "angle 32 y ≈ 0, got {y}");
    }

    /// angle 64 → x≈0, y≈-65536 (= -sin(π), cos(π))
    #[test]
    fn cossin_angle_64() {
        let (x, y) = COSSIN_TABLE[64];
        assert!(x.abs() <= 1, "angle 64 x ≈ 0, got {x}");
        assert!((y + 65536).abs() <= 1, "angle 64 y ≈ -65536, got {y}");
    }

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(25), 5);
        assert_eq!(isqrt(100), 10);
        // floor(sqrt(2)) = 1
        assert_eq!(isqrt(2), 1);
        // floor(sqrt(99)) = 9
        assert_eq!(isqrt(99), 9);
    }

    #[test]
    fn vector_length_pythagoras() {
        // 3-4-5 triangle (in pixel-scale ints)
        assert_eq!(vector_length(3, 4), 5);
        assert_eq!(vector_length(0, 0), 0);
        assert_eq!(vector_length(100, 0), 100);
    }
}
