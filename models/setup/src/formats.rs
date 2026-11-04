//! The setup engine's number formats, and the multiplier's post-shifter.
//!
//! Formats follow `docs/design/triangle-pipeline.md` §5, with the changes
//! the fixed-point model needed (each noted where it is defined). Raw
//! values are `qfixed` counts: the value times `2^F`.

use qfixed::typenum::Unsigned;
use qfixed::typenum::{U0, U1, U12, U13, U16, U17, U2, U20, U23, U26, U33, U4, U8};
use qfixed::Q;

/// A screen coordinate: x or y, in pixels with 4 fraction bits. The range,
/// −2048 to just under +2048, is the guard band.
pub type Coord = Q<U12, U4>;

/// A difference of two coordinates: dx, dy, an edge's A or B, or the start
/// point relative to a vertex.
pub type Delta = Q<U13, U4>;

/// Twice a triangle's signed area, or an edge function's value: exact, as
/// the product of two [`Delta`]s needs 8 fraction bits and, for coordinates
/// in the guard band, at most 26 integer bits.
pub type Wide = Q<U26, U8>;

/// A normalised area: `1 <= m < 2`, exactly (the area has at most 33
/// significant bits). On the multiplier's 36-bit port. (The brief's
/// `u1.17` drops the area's low bits, a relative error of up to `2^-17` in
/// every gradient.)
pub type Mantissa = Q<U2, U33>;

/// A reciprocal seed: `1 / m` to about 11 bits, `0.5 < x0 < 1`. Signed, as
/// it goes on the multiplier's signed 18-bit port.
pub type Seed = Q<U1, U17>;

/// The reciprocal after one Newton–Raphson step, and its intermediates,
/// `t = m * x0` and `2 - t`, all near 1. 33 fraction bits: they go on the
/// multiplier's 36-bit port, so they keep more precision than the brief's
/// 18-bit `r`.
pub type Recip = Q<U2, U33>;

/// A gradient factor, `k = d * r` for an edge delta `d`: block floating
/// point, its true value `k * 2^(8 - E)` per pixel. On the 36-bit port
/// (the brief put it on the 18-bit port, which leaves a small triangle's
/// factors only a few significant bits). `|d| < 4096` and `r <= 1`, so
/// `|k| < 4096` but for rounding, which the narrowing saturates.
pub type K = Q<U13, U23>;

/// An attribute delta between two vertices, in the attribute's native
/// units: up to 17 bits for a 16-bit attribute, on the 18-bit port.
pub type AttrDelta = Q<U17, U0>;

/// An iterator value (start, d/dx or d/dy) in the attribute's native units
/// with 16 fraction bits, 36 bits in all, for every attribute. Traversal
/// steps it with 36-bit adders that wrap, so a start value is meaningful
/// modulo `2^36` counts: it may be far out of range at a start point
/// outside a thin triangle.
pub type Iter = Q<U20, U16>;

/// Width of the engine's registers and outputs, in bits.
pub const REG_BITS: u32 = 36;

/// Width of the multiplier's accumulator, in bits.
pub const ACC_BITS: u32 = 56;

/// Fixed left shift ahead of the post-shifter's right shift, so that a
/// result whose exponent `E` is small still needs a non-negative shift.
pub const PRE_SHIFT: u32 = 8;

/// The post-shifter: what the multiplier slot writes from its accumulator.
///
/// The accumulator is shifted left by [`PRE_SHIFT`], then right
/// (arithmetically) by `shift`, rounding half up if `round` is set, and
/// then saturated to [`REG_BITS`] if `sat` is set, else wrapped to it.
///
/// # Arguments
///
/// * `acc` - The accumulator, a signed count that fits [`ACC_BITS`].
/// * `shift` - The right shift: a constant, plus the exponent `E` for the
///   operations that use it. At most 63.
/// * `round` - Round half up (toward +∞) instead of flooring.
/// * `sat` - Clamp to the register's range instead of wrapping.
///
/// # Returns
///
/// The register word, as a signed count that fits [`REG_BITS`].
///
/// # Panics
///
/// If `acc` doesn't fit the accumulator or `shift` is over 63.
pub fn post_shift(acc: i64, shift: u32, round: bool, sat: bool) -> i64 {
    let limit = 1_i64 << (ACC_BITS - 1);
    assert!(
        (-limit..limit).contains(&acc),
        "accumulator overflow: {acc}"
    );
    assert!(shift < 64, "shift {shift} out of range");
    let x = acc << PRE_SHIFT;
    let shifted = if round && shift > 0 {
        ((x >> (shift - 1)) + 1) >> 1
    } else {
        x >> shift
    };
    let max = (1_i64 << (REG_BITS - 1)) - 1;
    if sat {
        shifted.clamp(-max - 1, max)
    } else {
        wrap(shifted, REG_BITS)
    }
}

/// How a multiplier result is scaled on its way to a register.
///
/// A product's count has the fraction bits of its operands. For a plain
/// fixed-point result the post-shifter shifts right by
/// `frac_in + PRE_SHIFT - frac_out`; for a block-floating-point result
/// (one with a factor `k`, whose true value is `k * 2^(8 - E)`) it shifts by
/// `frac_in - frac_out + E` instead, applying the exponent.
///
/// # Arguments
///
/// * `acc` - The accumulator: a product or a sum of products, in its exact
///   format.
/// * `e` - `Some(E)` to apply the exponent `E`, `None` for a plain result.
/// * `round` - Round half up.
/// * `sat` - Saturate to the register instead of wrapping.
///
/// # Returns
///
/// The result in format `Q<I, F>`.
pub(crate) fn narrow<I: Unsigned, F: Unsigned, PI: Unsigned, PF: Unsigned>(
    acc: Q<PI, PF>,
    e: Option<u32>,
    round: bool,
    sat: bool,
) -> Q<I, F> {
    let shift = match e {
        None => PF::U32 + PRE_SHIFT - F::U32,
        Some(e) => PF::U32 + e - F::U32,
    };
    Q::from_count(post_shift(acc.to_count(), shift, round, sat))
}

/// `x` wrapped to a signed `bits`-bit value, as a register keeps only its
/// low bits.
///
/// # Arguments
///
/// * `x` - The value.
/// * `bits` - The register width, 1 to 63.
///
/// # Returns
///
/// The low `bits` bits of `x`, sign-extended.
pub(crate) fn wrap(x: i64, bits: u32) -> i64 {
    let unused = 64 - bits;
    (x << unused) >> unused
}
