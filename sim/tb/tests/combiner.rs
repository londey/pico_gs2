//! Twin test of the colour combiner and alpha test
//! (`docs/design/host-interface.md` §7, §8): the whole core with the SDRAM
//! model and a host on its USB UART.
//!
//! Once the built-in scene's first frame is up, a host connects and sends
//! `setup_model::demo::combiner_demo`: it uploads
//! two 16 × 16 textures (RGB565, and ARGB4444 whose alpha rises across it)
//! and draws three frames:
//!
//! 1. Nine quads, each with its own `CC_MODE`: the six presets (modulate,
//!    decal, shade, tint, texture + shade, fade), a combination using every
//!    alpha broadcast and a negative `A − B`, reserved codes (which read 0),
//!    and an untextured quad tinted by `CONST0` (the texel reads as white).
//!    The vertex colours vary, alpha included.
//! 2. The four alpha tests on the alpha-ramp texture; and a cut-out (pass
//!    if alpha ≥ 128) drawn in front of a far triangle drawn after it, so
//!    the far one shows through only where the cut-out failed the test:
//!    a pixel failing the alpha test writes no depth.
//! 3. Frame 1's nine quads again with the alpha test passing alpha ≥ 96,
//!    so each combiner setting's alpha shows as a cut-out.
//!
//! Every frame shown is pixel-exact against the reference renderer. The
//! core's input on every cycle is recorded as
//! `build/stimulus/gpu.combiner.csv`, which `just check` replays with
//! `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use gs_proto::Transaction;
use setup_model::demo::combiner_demo;
use setup_model::render::{render, Frame};
use setup_model::scene::{cube_scene, transactions};
use tb::{differing, screen, Bench, FB_HEIGHT, FB_WIDTH, FRAME};

/// Frames to run at most.
const MAX_FRAMES: usize = 16;

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// The frames a scene presents, as the reference renderer draws them.
fn frames_of(txns: &[Transaction]) -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    render(&mut frame, txns).0
}

/// Writes a decoded active picture as an RGB PNG in `build/`.
fn write_png(picture: &[[u8; 3]], name: &str) -> Result<(), Box<dyn Error>> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(build_path(&[name]))?),
        u32::try_from(tb::H_ACTIVE)?,
        u32::try_from(tb::V_ACTIVE)?,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(picture.as_flattened())?;
    Ok(())
}

/// The ROM scene, then the host's three frames, pixel-exact.
#[test]
fn combiner_and_alpha_test() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.combiner.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(
        BufWriter::new(File::create(path)?),
        "sim/tb/tests/combiner.rs",
    )?;
    let rom = frames_of(&transactions(&cube_scene()));
    let host_txns = combiner_demo();
    let host = frames_of(&host_txns);
    assert_eq!(host.len(), 3);
    // The frames aren't trivially alike: every quad drew something.
    assert_ne!(host[0], host[1]);

    let mut connected = false;
    let mut last: Option<usize> = None;
    let mut out = None;
    while bench.cycle < MAX_FRAMES * FRAME {
        let (o, shown) = bench.step()?;
        out = Some(o);
        let Some(shown) = shown.filter(|f| f.presented >= 1) else {
            continue;
        };
        let picture = screen(&shown.symbols)?;
        let is = |fb: &Vec<u16>| differing(&picture, fb) == 0;
        if !connected {
            if !rom.iter().any(is) {
                return Err("the first frame isn't one of the ROM's".into());
            }
            connected = true;
            bench.dtr_n = 0;
            bench.host.reconnect();
            bench.host.send(host_txns.iter().copied());
            continue;
        }
        let allowed: Vec<usize> = last.map_or(vec![0], |k| vec![k, k + 1]);
        match (
            allowed.into_iter().find(|&k| host.get(k).is_some_and(is)),
            last,
        ) {
            (Some(k), _) => {
                println!("cycle {}: host frame {k}", bench.cycle);
                if last != Some(k) {
                    write_png(&picture, &format!("combiner-frame{k}.png"))?;
                }
                last = Some(k);
                if k == host.len() - 1 {
                    break;
                }
            }
            (None, None) if rom.iter().any(is) => {}
            (None, _) => {
                let diffs: Vec<_> = host.iter().map(|fb| differing(&picture, fb)).collect();
                return Err(format!(
                    "after host frame {last:?}, a frame that isn't the next; pixels differing from each host frame: {diffs:?}"
                )
                .into());
            }
        }
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(
        last,
        Some(host.len() - 1),
        "the host's frames were never all shown"
    );
    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
