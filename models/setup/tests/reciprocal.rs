//! The reciprocal over every mantissa (brief §8): its worst relative error.
//!
//! The mantissa has 33 fraction bits; every one of its 2^17 top-bit
//! patterns is tried, with pseudo-random low bits and with them all clear
//! and all set.

use setup_model::{reciprocal, Mantissa};

/// Correct bits the reciprocal must have everywhere: the brief asks for 16;
/// one Newton–Raphson step from an 11-bit seed, with 33-bit intermediates,
/// gives more.
const MIN_BITS: f64 = 21.0;

/// For every mantissa `1 <= m < 2`, `r * m` is within `2^-MIN_BITS` of 1.
#[test]
fn reciprocal_is_accurate_for_every_mantissa() {
    let mut worst = 0.0_f64;
    let mut worst_m = 0;
    let mut lcg = 1_u64;
    for top in 1_i64 << 17..1 << 18 {
        lcg = lcg
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let random = i64::try_from(lcg >> 48).expect("16 bits");
        for low in [0, random, (1 << 16) - 1] {
            let bits = top << 16 | low;
            let m = Mantissa::from_count(bits);
            let r = reciprocal(m).to_f64();
            let err = (r * m.to_f64() - 1.0).abs();
            if err > worst {
                worst = err;
                worst_m = bits;
            }
        }
    }
    let good = -worst.log2();
    println!("reciprocal: worst relative error 2^-{good:.2} at m = {worst_m:#x}");
    assert!(
        good >= MIN_BITS,
        "only {good:.2} correct bits at m = {worst_m:#x}"
    );
}
