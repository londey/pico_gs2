//! Unit tests of the generated TMDS encoder (`hw/video.snt`,
//! `TmdsEncoder`) against the reference encoder from the DVI specification.

use std::collections::{BTreeMap, VecDeque};

use tb::{Encoder, CONTROL_TOKENS};
use tmds_encoder_sim::{video__TmdsEncoder as TmdsEncoder, Inputs};

/// Width of the hardware's running-disparity register (`disparity: i5`).
const DISPARITY_BITS: u32 = 5;

/// Core cycles per pixel: the encoder takes a pixel every fourth cycle.
const CYCLES_PER_PIXEL: usize = 4;

/// Inputs for the cycles between pixels. They must be ignored, so they are
/// deliberately not zero.
const BETWEEN_PIXELS: Inputs = Inputs {
    data: 0xA5,
    ctrl: 3,
    de: 1,
    en: 0,
};

/// Encodes one pixel: `en` for one cycle, then the rest of the pixel's
/// cycles for the pipeline.
///
/// # Arguments
///
/// * `sim` - The encoder.
/// * `data` - The data byte.
/// * `ctrl` - The control bits (C1, C0), sent while `de` is low.
/// * `de` - Display enable.
///
/// # Returns
///
/// The symbol the encoder then outputs.
fn pixel(sim: &mut TmdsEncoder, data: u8, ctrl: u8, de: u8) -> u16 {
    sim.step(&Inputs {
        data,
        ctrl,
        de,
        en: 1,
    });
    for _ in 1..CYCLES_PER_PIXEL {
        sim.step(&BETWEEN_PIXELS);
    }
    sim.eval(&BETWEEN_PIXELS).out_symbol
}

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
                assert_eq!(
                    pixel(&mut sim, d, 0, 1),
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
        pixel(&mut sim, 0x00, 0, 1);
        assert_eq!(
            pixel(&mut sim, 0x5A, ctrl, 0),
            CONTROL_TOKENS[usize::from(ctrl)]
        );
        assert_eq!(sim.register("disparity"), Some(0));
    }
}

/// The symbol output changes only at pixel boundaries, four cycles after
/// the pixel is taken, so board/top.sv can sample it once per pixel.
#[test]
fn symbol_changes_once_per_pixel() {
    let mut sim = TmdsEncoder::new();
    let mut reference = Encoder::new();
    let mut symbols = Vec::new();
    for data in (0..=255_u8).step_by(7) {
        for cycle in 0..CYCLES_PER_PIXEL {
            let inputs = if cycle == 0 {
                Inputs {
                    data,
                    ctrl: 0,
                    de: 1,
                    en: 1,
                }
            } else {
                BETWEEN_PIXELS
            };
            symbols.push(sim.step(&inputs).out_symbol);
        }
    }
    for (t, pair) in symbols.windows(2).enumerate() {
        if pair[0] != pair[1] {
            assert_eq!(
                (t + 1) % CYCLES_PER_PIXEL,
                0,
                "symbol changed in cycle {}",
                t + 1
            );
        }
    }
    // Pixel n's symbol shows from cycle 4 (n + 1).
    for (n, data) in (0..=255_u8)
        .step_by(7)
        .enumerate()
        .take(symbols.len() / CYCLES_PER_PIXEL - 1)
    {
        assert_eq!(
            symbols[CYCLES_PER_PIXEL * (n + 1)],
            reference.encode(data, 0, true),
            "pixel {n}"
        );
    }
}
