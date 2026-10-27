//! Unit tests of the generated TMDS encoder (`hw/video.snt`,
//! `TmdsEncoder`) against the reference encoder from the DVI specification.

use std::collections::{BTreeMap, VecDeque};

use tb::{Encoder, CONTROL_TOKENS};
use tmds_encoder_sim::{video__TmdsEncoder as TmdsEncoder, Inputs};

/// Width of the hardware's running-disparity register (`disparity: i5`).
const DISPARITY_BITS: u32 = 5;

/// Every running disparity the reference encoder can reach from reset, each
/// with the shortest byte sequence that reaches it.
///
/// # Returns
///
/// A map from disparity to the data bytes that lead there from zero.
fn reachable_disparities() -> BTreeMap<i32, Vec<u8>> {
    let mut paths = BTreeMap::from([(0, Vec::new())]);
    let mut queue = VecDeque::from([0]);
    while let Some(cnt) = queue.pop_front() {
        let path = paths[&cnt].clone();
        for data in 0..=255_u8 {
            let mut enc = Encoder::new();
            for &d in &path {
                enc.encode(d, 0, true);
            }
            enc.encode(data, 0, true);
            paths.entry(enc.disparity()).or_insert_with(|| {
                queue.push_back(enc.disparity());
                let mut p = path.clone();
                p.push(data);
                p
            });
        }
    }
    paths
}

/// Sign-extends the hardware's disparity register to an `i32`.
///
/// # Arguments
///
/// * `raw` - The register's raw bits, as `register("disparity")` returns.
///
/// # Returns
///
/// The disparity as a signed value.
fn disparity_of(raw: u64) -> i32 {
    let shift = 64 - DISPARITY_BITS;
    ((raw << shift) as i64 >> shift) as i32
}

/// The running disparity stays inside the range of the hardware's 5-bit
/// register for every input sequence.
#[test]
fn disparity_fits_register() {
    let limit = 1 << (DISPARITY_BITS - 1);
    for &cnt in reachable_disparities().keys() {
        assert!(
            (-limit..limit).contains(&cnt),
            "disparity {cnt} overflows i{DISPARITY_BITS}"
        );
    }
}

/// From every reachable running disparity, every data byte encodes to the
/// reference symbol and leaves the same disparity.
#[test]
fn data_matches_reference_from_every_disparity() {
    for (cnt, path) in reachable_disparities() {
        for data in 0..=255_u8 {
            let mut sim = TmdsEncoder::new();
            let mut reference = Encoder::new();
            for &d in path.iter().chain(std::iter::once(&data)) {
                let out = sim.step(&Inputs {
                    data: d,
                    ctrl: 0,
                    de: 1,
                });
                assert_eq!(
                    out.out_symbol,
                    reference.encode(d, 0, true),
                    "data {d:#04x} from disparity {cnt}"
                );
            }
            let hw = sim.register("disparity").map(disparity_of);
            assert_eq!(hw, Some(reference.disparity()));
        }
    }
}

/// Blanking sends the control token for `ctrl` and resets the disparity.
#[test]
fn control_tokens() {
    for ctrl in 0..4_u8 {
        let mut sim = TmdsEncoder::new();
        sim.step(&Inputs {
            data: 0x00,
            ctrl: 0,
            de: 1,
        });
        let out = sim.step(&Inputs {
            data: 0x5A,
            ctrl,
            de: 0,
        });
        assert_eq!(out.out_symbol, CONTROL_TOKENS[usize::from(ctrl)]);
        assert_eq!(sim.register("disparity"), Some(0));
    }
}
