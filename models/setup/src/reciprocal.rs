//! The reciprocal of a triangle's area (brief §4.2): normalise it to a
//! mantissa `m` and an exponent `E`, look up a seed, refine it with one
//! Newton–Raphson step.

use qfixed::typenum::{U3, U50};
use qfixed::Q;

use crate::formats::{narrow, Mantissa, Recip, Seed, Wide};

/// Entries in the seed table, indexed by the mantissa's 10 bits after its
/// leading one.
pub const SEEDS: usize = 1024;

/// The seed table: entry `i` is `1 / m` at the middle of its interval,
/// `m = 1 + (i + 0.5) / 1024`, as a [`Seed`] count rounded half up. With
/// `2^17 / m = 2^28 / (2049 + 2i)`, the rounding is exact.
const SEED_TABLE: [i64; SEEDS] = {
    let mut table = [0; SEEDS];
    let mut i = 0;
    while i < SEEDS {
        // i < 1024: the cast is exact (`try_from` isn't const).
        #[allow(clippy::cast_possible_wrap)]
        let d = 2049 + 2 * i as i64;
        table[i] = ((1 << 28) + d / 2) / d;
        i += 1;
    }
    table
};

/// An area split into mantissa and exponent: `area = m * 2^(E - 8)`, the
/// `- 8` being the area's fraction bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Normalised {
    /// The mantissa, `1 <= m < 2`.
    pub m: Mantissa,

    /// The exponent `E`: the position of the area count's leading one,
    /// 0 to 32.
    pub e: u32,
}

/// Normalises a positive area (the ALU's LZC operation).
///
/// # Arguments
///
/// * `area` - Twice the triangle's area; positive.
///
/// # Returns
///
/// Its mantissa and exponent, exactly.
///
/// # Panics
///
/// If `area` isn't positive.
pub fn normalise(area: Wide) -> Normalised {
    let a = area.to_count();
    assert!(a > 0, "normalise needs a positive area, not {a}");
    let e = 63 - a.leading_zeros();
    // a < 2^33, so e <= 32 and the shift is at least 1.
    Normalised {
        m: Mantissa::from_count(a << (33 - e)),
        e,
    }
}

/// The seed for a mantissa (the ALU's RCPLD operation, a table lookup).
///
/// # Arguments
///
/// * `m` - The mantissa.
///
/// # Returns
///
/// `1 / m` to about 11 bits.
pub fn seed(m: Mantissa) -> Seed {
    let index = (m.to_count() >> 23) & 0x3FF;
    Seed::from_count(SEED_TABLE[usize::try_from(index).expect("10 bits")])
}

/// The reciprocal of a mantissa: the seed `x0`, then
/// `r = x0 * (2 - m * x0)`, on the multiplier with `m` and `2 - m * x0` on
/// its 36-bit port and `x0` on its 18-bit port.
///
/// # Arguments
///
/// * `m` - The mantissa.
///
/// # Returns
///
/// `1 / m`, rounded half up to [`Recip`]'s 33 fraction bits.
pub fn reciprocal(m: Mantissa) -> Recip {
    let x0 = seed(m);
    let mx0: Q<U3, U50> = m.widening_mul(x0);
    let t: Recip = narrow(mx0, None, false, false);
    // 2 - t on the ALU: 2.0 is one past Recip's range, so the subtraction
    // is done on counts; the result is near 1.
    let two = 2_i64 << 33;
    let t2 = Recip::from_count(two - t.to_count());
    let r_full: Q<U3, U50> = t2.widening_mul(x0);
    narrow(r_full, None, true, false)
}
