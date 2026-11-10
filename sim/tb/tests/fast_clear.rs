//! Twin test of the fast clear (`docs/design/draw-speed.md` §3.1): a clear
//! only marks the buffer's blocks cleared, and everything that reads a
//! buffer (the depth test, the writes, scan-out and `MEM_DATA`) must see
//! exactly what a clear that wrote every word would have left.
//!
//! Once the built-in scene's first frame is up, a host connects and, in a
//! frame it doesn't present yet:
//!
//! 1. clears colour and depth, then draws a triangle that touches only the
//!    top half of block (5, 5) (the other half must be written with the
//!    clear colour, not left as the SDRAM had it); one that writes depth
//!    but not colour over block (8, 5) (its colour stays cleared); and one
//!    that writes colour but not depth over block (11, 5) (its depth
//!    stays cleared). It reads back those blocks, and an untouched one, of
//!    the colour buffer and of the depth buffer.
//! 2. clears depth only, then draws a far triangle over the bottom half of
//!    block (5, 5): it passes the cleared depth, and the top half keeps the
//!    first triangle. It reads the blocks back again.
//!
//! Then it presents that frame and two more, each with its own clear
//! colour, so that the frame shown and the one waiting were cleared to
//! different colours; the third clears colour only, so the second frame's
//! depth hides part of its triangle. Every block read back and every
//! frame shown is the reference renderer's, which clears by writing every
//! word.
//!
//! The core's input on every cycle is recorded as
//! `build/stimulus/gpu.fast_clear.csv`, which `just check` replays with
//! `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use gs_proto::{
    colour_buffer, mem_read, pixel_word, reg, unpack_words, Attr0, Frame as FrameReg, Position,
    RenderMode, Status, Transaction,
};
use setup_model::render::{render, Frame};
use setup_model::scene::{cube_scene, transactions};
use tb::{differing, screen, Bench, FB_HEIGHT, FB_WIDTH, FRAME};

/// Frames to run at most.
const MAX_FRAMES: usize = 16;

/// The depth buffer's bank.
const DEPTH: u32 = 3;

/// The blocks read back: the half-drawn one, the depth-only one, the
/// colour-only one, and an untouched one.
const BLOCKS: [(usize, usize); 4] = [(5, 5), (8, 5), (11, 5), (20, 20)];

/// The clear colours of the frames, RGB565: dark blue, dark green, dark red.
const CLEAR: [u16; 3] = [0x0010, 0x0200, 0x8000];

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// A `FRAME` write.
fn frame(present: bool, colour: Option<u16>, depth: bool) -> Transaction {
    Transaction::write(
        reg::FRAME,
        FrameReg {
            present,
            clear_colour: colour.is_some(),
            clear_depth: depth,
            colour: colour.unwrap_or(0),
        }
        .bits(),
    )
}

/// A separate untextured triangle of one colour and depth, with a render
/// mode.
fn triangle(v: [(f64, f64); 3], z: u16, rgb: [u8; 3], mode: RenderMode) -> Vec<Transaction> {
    #[allow(clippy::cast_possible_truncation)]
    let fixed = |f: f64| i16::try_from((f * 16.0).round() as i64).unwrap_or(0);
    let mut out = vec![
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
    ];
    for (i, &(x, y)) in v.iter().enumerate() {
        let register = if i == 2 {
            reg::VERTEX_KICK_012
        } else {
            reg::VERTEX_NOKICK
        };
        let pos = Position {
            x: fixed(x),
            y: fixed(y),
            z,
            q: 0x8000,
        };
        out.push(Transaction::write(register, pos.bits()));
    }
    out
}

/// Step 1: a frame cleared, and three triangles that leave parts of their
/// blocks as cleared.
fn step1() -> Vec<Transaction> {
    let normal = RenderMode::default();
    [
        vec![frame(false, Some(CLEAR[0]), true)],
        // Rows 40 to 43 only: block (5, 5)'s top half.
        triangle(
            [(40.0, 40.0), (48.0, 40.0), (40.0, 44.0)],
            2000,
            [255, 200, 0],
            normal,
        ),
        // Depth only.
        triangle(
            [(64.0, 40.0), (72.0, 40.0), (64.0, 48.0)],
            1000,
            [0, 255, 0],
            RenderMode {
                colour_write: false,
                ..normal
            },
        ),
        // Colour only.
        triangle(
            [(88.0, 40.0), (96.0, 40.0), (88.0, 48.0)],
            1000,
            [0, 128, 255],
            RenderMode {
                z_write: false,
                ..normal
            },
        ),
    ]
    .concat()
}

/// Step 2: depth cleared alone, and a far triangle over block (5, 5)'s
/// bottom half.
fn step2() -> Vec<Transaction> {
    [
        vec![frame(false, None, true)],
        triangle(
            [(40.0, 44.0), (48.0, 44.0), (40.0, 48.0)],
            60000,
            [255, 0, 255],
            RenderMode::default(),
        ),
    ]
    .concat()
}

/// The frames after: present step 1 and 2's frame, draw a second with its
/// own clear colour, then a third that clears colour only, so the
/// second's near triangle hides part of its far one.
fn frames() -> Vec<Transaction> {
    let near = triangle(
        [(100.0, 100.0), (300.0, 100.0), (100.0, 300.0)],
        1000,
        [255, 255, 255],
        RenderMode::default(),
    );
    let far = triangle(
        [(150.0, 150.0), (400.0, 150.0), (150.0, 340.0)],
        5000,
        [255, 128, 0],
        RenderMode::default(),
    );
    [
        vec![frame(true, Some(CLEAR[1]), true)],
        near,
        vec![frame(true, Some(CLEAR[2]), false)],
        far,
        vec![frame(true, None, false)],
    ]
    .concat()
}

/// The reads of the blocks of a buffer.
fn block_reads(bank: u32) -> Vec<Transaction> {
    BLOCKS
        .iter()
        .flat_map(|&(bx, by)| {
            let word = u32::try_from(pixel_word(bx * 8, by * 8)).unwrap_or(0);
            mem_read(colour_buffer(bank) + word / 4, 16)
        })
        .collect()
}

/// The blocks of a frame, as `block_reads` returns them: each block's 64
/// words in raster order, colour blocks then depth blocks.
fn blocks_of(frame: &Frame) -> Vec<u16> {
    let pick = |buf: &Vec<u16>| -> Vec<u16> {
        BLOCKS
            .iter()
            .flat_map(|&(bx, by)| {
                (0..64).map(move |i| buf[(by * 8 + i / 8) * FB_WIDTH + bx * 8 + i % 8])
            })
            .collect()
    };
    [pick(&frame.colour), pick(&frame.depth)].concat()
}

/// The reference frame after some transactions, from a black start.
fn reference(txns: &[Transaction]) -> (Vec<Vec<u16>>, Frame) {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    let shown = render(&mut frame, txns).0;
    (shown, frame)
}

/// Where the test is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// The built-in scene, no host yet.
    Rom,
    /// Step 1 sent; waiting for `STATUS`.
    Status,
    /// The reads sent and the frames after; checking the frames shown.
    Frames,
}

/// Steps 1 and 2 read back as the reference renderer has them, and the
/// three frames after shown as it draws them.
#[test]
fn fast_clear_reads_and_shows_as_cleared() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.fast_clear.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(
        BufWriter::new(File::create(path)?),
        "sim/tb/tests/fast_clear.rs",
    )?;
    let rom: Vec<Vec<u16>> = reference(&transactions(&cube_scene())).0;
    let s1 = step1();
    let s2 = step2();
    let (_, after1) = reference(&s1);
    let (_, after2) = reference(&[s1.clone(), s2.clone()].concat());
    let (shown_ref, _) = reference(&[s1.clone(), s2.clone(), frames()].concat());
    assert_eq!(shown_ref.len(), 3);

    let reads = BLOCKS.len() * 16 * 2;
    let mut stage = Stage::Rom;
    let mut last: Option<usize> = None;
    let mut out = None;
    while bench.cycle < MAX_FRAMES * FRAME {
        let (o, shown) = bench.step()?;
        out = Some(o);
        if stage == Stage::Status && bench.host.replies.len() == 1 {
            // The buffer being drawn: read its blocks after step 1, then
            // after step 2, then present and go on.
            let drawn = u32::from(Status::from_bits(bench.host.replies[0]).drawn);
            bench.host.send(block_reads(drawn));
            bench.host.send(block_reads(DEPTH));
            bench.host.send(s2.iter().copied());
            bench.host.send(block_reads(drawn));
            bench.host.send(block_reads(DEPTH));
            bench.host.send(frames());
            stage = Stage::Frames;
        }
        let Some(shown) = shown.filter(|f| f.presented >= 1) else {
            continue;
        };
        let picture = screen(&shown.symbols)?;
        let is = |fb: &Vec<u16>| differing(&picture, fb) == 0;
        match stage {
            Stage::Rom => {
                if !rom.iter().any(is) {
                    return Err("the first frame isn't one of the ROM's".into());
                }
                stage = Stage::Status;
                bench.dtr_n = 0;
                bench.host.reconnect();
                bench.host.send(s1.iter().copied());
                bench.host.send([Transaction::Read { addr: reg::STATUS }]);
            }
            Stage::Status => {}
            Stage::Frames => {
                let allowed: Vec<usize> = last.map_or(vec![0], |k| vec![k, k + 1]);
                match allowed
                    .into_iter()
                    .find(|&k| shown_ref.get(k).is_some_and(is))
                {
                    Some(k) => {
                        println!("cycle {}: host frame {k}", bench.cycle);
                        last = Some(k);
                        if k == shown_ref.len() - 1 {
                            break;
                        }
                    }
                    None if last.is_none() && rom.iter().any(is) => {}
                    None => {
                        let diffs: Vec<_> =
                            shown_ref.iter().map(|fb| differing(&picture, fb)).collect();
                        return Err(format!(
                            "after host frame {last:?}, a frame that isn't the next; pixels differing: {diffs:?}"
                        )
                        .into());
                    }
                }
            }
        }
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(
        last,
        Some(shown_ref.len() - 1),
        "the frames were never all shown"
    );

    // The blocks: replies 1.. are step 1's reads, then step 2's.
    let r = &bench.host.replies;
    assert_eq!(r.len(), 1 + 2 * reads, "replies");
    let words = |from: usize| -> Vec<u16> {
        r[from..from + reads]
            .iter()
            .flat_map(|&d| unpack_words(d))
            .collect()
    };
    let check = |got: Vec<u16>, want: Vec<u16>, step: &str| {
        let half = got.len() / 2;
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let buffer = if i < half { "colour" } else { "depth" };
            let k = i % half;
            let (bx, by) = BLOCKS[k / 64];
            assert_eq!(
                g,
                w,
                "{step}: {buffer} block ({bx}, {by}) pixel ({}, {})",
                k % 64 % 8,
                k % 64 / 8
            );
        }
    };
    check(words(1), blocks_of(&after1), "step 1");
    check(words(1 + reads), blocks_of(&after2), "step 2");

    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert!(!bench.underflow(), "the display ran short of pixels");
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
