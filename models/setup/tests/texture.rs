//! The texture divide's accuracy (host-interface.md §6.3): the texel
//! coordinate the GPU computes from the interpolated S′ and Q, against
//! the exact quotient of the same values, for a 256-texel texture, by how
//! small Q is (how far a pixel is against a triangle's nearest point, when
//! the host scales the nearest Q to 1).

use setup_model::texture::{coordinate, reciprocal};

/// The worst error, in texels, over S′ and Q iterator counts with
/// `q_min <= Q < 2 * q_min` (Q from the count over 2^31).
fn worst_error(q_min: f64) -> f64 {
    let mut worst: f64 = 0.0;
    let mut seed: u64 = 0x1234_5678;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        seed >> 11
    };
    for _ in 0..20_000 {
        #[allow(clippy::cast_precision_loss)]
        let f = (next() & 0xFFFF) as f64 / 65536.0;
        let q = q_min * (1.0 + f);
        #[allow(clippy::cast_possible_truncation)]
        let q_count = (q * 2_f64.powi(31)) as i64;
        // S′ anywhere from -8 to 8 repeats at this Q, within Q4.12.
        #[allow(clippy::cast_precision_loss)]
        let s = ((next() & 0xFFFF) as f64 / 65536.0 - 0.5) * 16.0;
        let s_prime = (s * q).clamp(-7.99, 7.99);
        #[allow(clippy::cast_possible_truncation)]
        let s_count = (s_prime * 2_f64.powi(28)) as i64;
        let u = coordinate(s_count, reciprocal(q_count), 8);
        #[allow(clippy::cast_precision_loss)]
        let exact = s_count as f64 / q_count as f64 * 8.0 * 256.0;
        #[allow(clippy::cast_precision_loss)]
        let got = u as f64 / 16.0;
        if exact.abs() < 32_000.0 {
            worst = worst.max((got - exact).abs());
        }
    }
    worst
}

/// Prints the error table, and checks it: within an eighth of a texel
/// (the coordinate keeps 4 fraction bits, so a sixteenth is rounding),
/// growing in proportion to 1/Q only from S′'s 2^-21 below Q = 1/256.
#[test]
fn divide_error_by_q() {
    println!("Q from    worst error (texels, 256-texel texture)");
    for k in 0..=8 {
        let q_min = 1.0 / f64::from(1 << k);
        let e = worst_error(q_min);
        println!("1/{:<6}  {e:.4}", 1 << k);
        let bound = 1.0 / 8.0 + 256.0 / 2_f64.powi(21) / q_min;
        assert!(e < bound, "Q from 1/{}: {e} texels", 1 << k);
    }
}
