//! Twin test of overlapped blocks (`docs/design/draw-speed.md` §3.3): the
//! rasteriser hands a finished block to its writer and goes on to the next
//! block while the writer writes it, so a later triangle's block can be
//! one the writer still holds. It must then wait, and see the earlier
//! triangle's depths and the block's flags as written.
//!
//! Once the built-in scene's first frame is up, a host connects and draws
//! two frames, each cleared, whose triangles keep landing in blocks the one
//! before has just drawn:
//!
//! 1. Rows of strips of small right triangles (8 and 5 pixels on a side,
//!    so most share blocks with the triangles either side); the same
//!    strips again, farther, which the first layer's depths must hide
//!    entirely; and a nearer layer over every other row.
//! 2. Pairs of triangles over the same few blocks, one after the other, at
//!    alternating depths, some writing depth only or colour only, so each
//!    depends on the depth or the colour the one before left in the block.
//!
//! Both frames are pixel-exact against the reference renderer, which
//! draws one triangle at a time.
//!
//! The core's input on every cycle is recorded as
//! `build/stimulus/gpu.overlap.csv`, which `just check` replays with
//! `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use gs_proto::{reg, Attr0, Frame as FrameReg, Position, RenderMode, Transaction};
use setup_model::render::{render, Frame};
use setup_model::scene::{cube_scene, transactions};
use tb::{differing, screen, Bench, FB_HEIGHT, FB_WIDTH, FRAME};

/// Frames to run at most.
const MAX_FRAMES: usize = 16;

/// The clear colour, RGB565: dark blue.
const BACKGROUND: u16 = 0x0010;

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// A `FRAME` write: present (or not), clearing colour and depth.
fn frame(present: bool) -> Transaction {
    Transaction::write(
        reg::FRAME,
        FrameReg {
            present,
            clear_colour: true,
            clear_depth: true,
            colour: BACKGROUND,
        }
        .bits(),
    )
}

/// `RENDER_MODE` and `ATTR0` for what follows: a colour and a mode.
fn state(rgb: [u8; 3], mode: RenderMode) -> [Transaction; 2] {
    [
        Transaction::write(reg::RENDER_MODE, mode.bits()),
        Transaction::write(
            reg::ATTR0,
            Attr0 {
                r: rgb[0],
                g: rgb[1],
                b: rgb[2],
                a: 255,
                s: 0,
                t: 0,
            }
            .bits(),
        ),
    ]
}

/// A vertex register write at a position (pixels) and depth.
fn vertex(register: u8, x: f64, y: f64, z: u16) -> Transaction {
    #[allow(clippy::cast_possible_truncation)]
    let fixed = |f: f64| i16::try_from((f * 16.0).round() as i64).unwrap_or(0);
    Transaction::write(
        register,
        Position {
            x: fixed(x),
            y: fixed(y),
            z,
            q: 0x8000,
        }
        .bits(),
    )
}

/// A strip of `n` right triangles `size` pixels on a side along a row
/// from (x, y), at one depth.
fn strip(x: f64, y: f64, size: f64, n: usize, z: u16) -> Vec<Transaction> {
    (0..n + 2)
        .map(|i| {
            let register = match i {
                0 | 1 => reg::VERTEX_NOKICK,
                _ if i % 2 == 0 => reg::VERTEX_KICK_012,
                _ => reg::VERTEX_KICK_021,
            };
            #[allow(clippy::cast_precision_loss)]
            let col = (i / 2) as f64;
            let bottom = i % 2 == 1;
            vertex(
                register,
                x + col * size,
                if bottom { y + size } else { y },
                z,
            )
        })
        .collect()
}

/// A separate triangle.
fn triangle(v: [(f64, f64); 3], z: u16) -> Vec<Transaction> {
    vec![
        vertex(reg::VERTEX_NOKICK, v[0].0, v[0].1, z),
        vertex(reg::VERTEX_NOKICK, v[1].0, v[1].1, z),
        vertex(reg::VERTEX_KICK_012, v[2].0, v[2].1, z),
    ]
}

/// Frame 1: three layers of strips of small triangles.
fn strips() -> Vec<Transaction> {
    let normal = RenderMode::default();
    let mut out = Vec::new();
    // The near layer first, then the same strips farther: hidden.
    for (rgb, z) in [([255, 64, 0], 1000), ([0, 255, 255], 3000)] {
        out.extend(state(rgb, normal));
        for row in 0..8 {
            #[allow(clippy::cast_precision_loss)]
            let y = 20.0 + row as f64 * 20.0;
            out.extend(strip(20.0, y, 8.0, 24, z));
            out.extend(strip(240.0, y + 3.0, 5.0, 40, z));
        }
    }
    // Nearer again, over every other row, offset by half a triangle.
    out.extend(state([255, 255, 0], normal));
    for row in (0..8).step_by(2) {
        #[allow(clippy::cast_precision_loss)]
        let y = 22.0 + row as f64 * 20.0;
        out.extend(strip(24.0, y, 8.0, 24, 500));
    }
    out
}

/// Frame 2: pairs of triangles over the same blocks, each depending on
/// the one before.
fn pairs() -> Vec<Transaction> {
    let normal = RenderMode::default();
    let depth_only = RenderMode {
        colour_write: false,
        ..normal
    };
    let colour_only = RenderMode {
        z_write: false,
        ..normal
    };
    let mut out = Vec::new();
    for k in 0..12 {
        #[allow(clippy::cast_precision_loss)]
        let (x, y) = (16.0 + (k % 6) as f64 * 96.0, 40.0 + (k / 6) as f64 * 120.0);
        // A square of two triangles over 2 x 2 blocks, near.
        let sq = |z: u16| {
            [
                triangle([(x, y), (x + 16.0, y), (x, y + 16.0)], z),
                triangle([(x + 16.0, y), (x + 16.0, y + 16.0), (x, y + 16.0)], z),
            ]
            .concat()
        };
        out.extend(state([200, 0, 0], normal));
        out.extend(sq(2000));
        // Farther: hidden by the first's depth.
        out.extend(state([0, 200, 0], normal));
        out.extend(sq(4000));
        // Depth only, nearer, over the top-left: then a colour behind it
        // there must be hidden, and in front of it shown.
        out.extend(state([0, 0, 0], depth_only));
        out.extend(triangle([(x, y), (x + 12.0, y), (x, y + 12.0)], 1000));
        out.extend(state([0, 0, 255], normal));
        out.extend(sq(1500));
        out.extend(state([255, 255, 255], colour_only));
        out.extend(triangle(
            [(x + 4.0, y + 4.0), (x + 16.0, y + 4.0), (x + 4.0, y + 16.0)],
            500,
        ));
        // Behind the colour-only triangle (which wrote no depth) but in
        // front of the rest: shown where the colour-only one was.
        out.extend(state([255, 0, 255], normal));
        out.extend(triangle(
            [(x + 6.0, y + 6.0), (x + 14.0, y + 6.0), (x + 6.0, y + 14.0)],
            800,
        ));
    }
    out
}

/// The host's transactions: a clear, frame 1, a present with a clear,
/// frame 2, a present.
fn host_transactions() -> Vec<Transaction> {
    [
        vec![frame(false)],
        strips(),
        vec![frame(true)],
        pairs(),
        vec![Transaction::write(
            reg::FRAME,
            FrameReg {
                present: true,
                ..FrameReg::default()
            }
            .bits(),
        )],
    ]
    .concat()
}

/// The frames a scene presents, as the reference renderer draws them.
fn frames_of(txns: &[Transaction]) -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    render(&mut frame, txns).0
}

/// The ROM scene, then the host's two frames, pixel-exact.
#[test]
fn blocks_overlap_without_hazards() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.overlap.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(
        BufWriter::new(File::create(path)?),
        "sim/tb/tests/overlap.rs",
    )?;
    let rom = frames_of(&transactions(&cube_scene()));
    let host_txns = host_transactions();
    let host = frames_of(&host_txns);
    assert_eq!(host.len(), 2);

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
        match allowed.into_iter().find(|&k| host.get(k).is_some_and(is)) {
            Some(k) => {
                println!("cycle {}: host frame {k}", bench.cycle);
                last = Some(k);
                if k == host.len() - 1 {
                    break;
                }
            }
            None if last.is_none() && rom.iter().any(is) => {}
            None => {
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
    assert!(!bench.underflow(), "the display ran short of pixels");
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
