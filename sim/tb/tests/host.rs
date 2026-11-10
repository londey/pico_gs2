//! Twin test of the host link (`docs/design/host-interface.md`): the whole
//! core with the SDRAM model on its bus and a host on its USB UART.
//!
//! 1. No host (DTR# high): the built-in scene plays; its first frame shows.
//! 2. A host connects (DTR# low), reads `ID`, `STATUS` and `RENDER_MODE`;
//!    reads `SCRATCH` (0 from reset), writes it and reads it back, and
//!    writes its complement; streams two frames of its own over the link
//!    at 3 Mbaud, both over a dark blue background; then reads `SCRATCH`
//!    (the complement, every bit having been both 0 and 1) and `STATUS`
//!    again. The first
//!    exercises `RENDER_MODE` and `FRAME`: the test card with the depth test
//!    off (so painter's order), a depth-only clear, a triangle that writes
//!    only depth (hiding part of the one behind it) and one that doesn't
//!    write depth (so a farther one paints over it). The second is a cube
//!    frame. The core takes over from the ROM at its next present,
//!    so every frame shown is whole: the ROM's, then the host's, each pixel-
//!    exact against the reference renderer. Credits come back for every 64
//!    transactions taken, the reads answer in order, and the FIFO never
//!    overflows.
//! 3. The host disconnects (DTR# high): the ROM scene starts again from its
//!    first frame.
//!
//! The core's input on every cycle is recorded as
//! `build/stimulus/gpu.host.csv`, which `just check` replays with `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

/// What the host writes to `SCRATCH`, then its complement: every byte
/// different, every bit both ways.
const SCRATCH_PATTERN: u64 = 0x0123_4567_89AB_CDEF;

use gs_proto::{
    reg, Attr0, Frame as FrameReg, Position, RenderMode, Status, Transaction, CREDIT_BATCH,
    FIFO_ENTRIES, ID_VALUE,
};
use setup_model::render::{render, Frame};
use setup_model::scene::{
    cube_frame, cube_scene, draws, test_card, transactions, Command, BACKGROUND,
};
use tb::{differing, screen, Bench, FB_HEIGHT, FB_WIDTH, FRAME, SYMBOL_LATENCY};

/// Frames to run at most.
const MAX_FRAMES: usize = 16;

/// The host's background colour, RGB565: dark blue, unlike the ROM's grey.
const HOST_BACKGROUND: u16 = 0x0010;

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// One triangle, drawn with a render mode: corners in pixels, a depth and
/// a colour.
fn triangle(corners: [(i16, i16); 3], z: u16, rgb: [u8; 3], mode: RenderMode) -> Vec<Transaction> {
    let colour = Attr0 {
        r: rgb[0],
        g: rgb[1],
        b: rgb[2],
        a: 255,
        ..Attr0::default()
    };
    let at = |(x, y): (i16, i16)| Position {
        x: x * 16,
        y: y * 16,
        z,
        q: 0,
    };
    vec![
        Transaction::write(reg::RENDER_MODE, mode.bits()),
        Transaction::write(reg::ATTR0, colour.bits()),
        Transaction::write(reg::VERTEX_NOKICK, at(corners[0]).bits()),
        Transaction::write(reg::VERTEX_NOKICK, at(corners[1]).bits()),
        Transaction::write(reg::VERTEX_KICK_012, at(corners[2]).bits()),
    ]
}

/// The host's transactions: the first frame exercising the render mode's
/// switches and a depth-only clear, then cube frame 5, over its background.
fn host_transactions() -> Vec<Transaction> {
    let frame = |f: FrameReg| Transaction::write(reg::FRAME, f.bits());
    let clear_all = FrameReg {
        clear_colour: true,
        clear_depth: true,
        colour: HOST_BACKGROUND,
        ..FrameReg::default()
    };
    let normal = RenderMode::default();
    let mut txns = vec![frame(clear_all)];
    // The test card with no depth test: later triangles win (its triangles
    // share one depth, so with the test the earlier would).
    txns.extend(transactions(&draws(test_card())).into_iter().map(|t| {
        match t {
            Transaction::Write {
                addr: reg::RENDER_MODE,
                data,
            } => Transaction::write(
                reg::RENDER_MODE,
                RenderMode {
                    z_test: false,
                    ..RenderMode::from_bits(data)
                }
                .bits(),
            ),
            t => t,
        }
    }));
    // Depth only, then a near triangle that writes only depth, hiding part
    // of a farther red one; and a near green one that writes no depth, which
    // a farther blue one paints over.
    txns.push(frame(FrameReg {
        clear_depth: true,
        ..FrameReg::default()
    }));
    let depth_only = RenderMode {
        colour_write: false,
        ..normal
    };
    txns.extend(triangle(
        [(20, 20), (300, 20), (20, 340)],
        100,
        [0, 0, 0],
        depth_only,
    ));
    txns.extend(triangle(
        [(10, 10), (330, 10), (10, 350)],
        2000,
        [255, 0, 0],
        normal,
    ));
    let no_z = RenderMode {
        z_write: false,
        ..normal
    };
    txns.extend(triangle(
        [(340, 20), (620, 20), (340, 340)],
        50,
        [0, 255, 0],
        no_z,
    ));
    txns.extend(triangle(
        [(330, 10), (630, 10), (330, 350)],
        3000,
        [0, 0, 255],
        normal,
    ));
    txns.push(frame(FrameReg {
        present: true,
        ..clear_all
    }));
    // A cube frame over the host's background (its own clear is the one
    // above).
    let cube: Vec<Command> = cube_frame(5)
        .into_iter()
        .filter(|c| !matches!(c, Command::Clear(BACKGROUND)))
        .collect();
    txns.extend(transactions(&cube));
    txns
}

/// The frames a scene presents, as the reference renderer draws them.
fn frames_of(txns: &[Transaction]) -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    render(&mut frame, txns).0
}

/// Which part of the test a run is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// No host yet: waiting for the ROM's first frame.
    Rom,
    /// The host is connected: ROM frames until its own appear.
    Host,
    /// The host has gone: its last frame until the ROM's first again.
    Back,
}

/// The ROM scene, a host's frames and then the ROM scene again, as above.
#[test]
fn host_takes_over_and_hands_back() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.host.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(BufWriter::new(File::create(path)?), "sim/tb/tests/host.rs")?;
    let rom = frames_of(&transactions(&cube_scene()));
    let host_txns = host_transactions();
    let host = frames_of(&host_txns);
    assert_eq!(host.len(), 2);

    let mut stage = Stage::Rom;
    // The last frame shown: the ROM's frame k, or the host's frame k.
    let mut last: Option<(bool, usize)> = None;
    let mut sent = 0;
    let mut out = None;
    while bench.cycle < MAX_FRAMES * FRAME + SYMBOL_LATENCY {
        let (o, shown) = bench.step()?;
        out = Some(o);
        let Some(shown) = shown.filter(|f| f.presented >= 1) else {
            continue;
        };
        let picture = screen(&shown.symbols)?;
        let is = |fb: &Vec<u16>| differing(&picture, fb) == 0;
        // What may follow the last frame shown.
        let mut allowed: Vec<(bool, usize)> = Vec::new();
        match (stage, last) {
            (Stage::Rom, _) => allowed.push((false, 0)),
            (Stage::Host, Some((false, k))) => {
                allowed.extend([(false, k), (false, (k + 1) % rom.len()), (true, 0)]);
            }
            (Stage::Host, Some((true, k))) => allowed.extend([(true, k), (true, k + 1)]),
            (Stage::Back, Some((true, k))) => allowed.extend([(true, k), (false, 0)]),
            (Stage::Back, Some((false, k))) => {
                allowed.extend([(false, k), (false, (k + 1) % rom.len())]);
            }
            (_, None) => return Err("no frame shown before connecting".into()),
        }
        let found = allowed.into_iter().find(|&(from_host, k)| {
            let frames = if from_host { &host } else { &rom };
            frames.get(k).is_some_and(is)
        });
        let Some(now) = found else {
            let diffs: Vec<_> = rom
                .iter()
                .chain(&host)
                .map(|fb| differing(&picture, fb))
                .collect();
            return Err(format!(
                "{stage:?}: a frame that is none of those allowed after {last:?}; pixels differing from each ROM then host frame: {diffs:?}"
            )
            .into());
        };
        println!(
            "cycle {}: {} frame {} (bank {})",
            bench.cycle,
            if now.0 { "host" } else { "ROM" },
            now.1,
            shown.bank
        );
        last = Some(now);
        match stage {
            Stage::Rom => {
                // Connect: the host reads, streams its frames, reads again.
                stage = Stage::Host;
                bench.dtr_n = 0;
                bench.host.reconnect();
                bench.host.send([
                    Transaction::Read { addr: reg::ID },
                    Transaction::Read { addr: reg::STATUS },
                    Transaction::Read {
                        addr: reg::RENDER_MODE,
                    },
                    Transaction::Read { addr: reg::SCRATCH },
                    Transaction::write(reg::SCRATCH, SCRATCH_PATTERN),
                    Transaction::Read { addr: reg::SCRATCH },
                    Transaction::write(reg::SCRATCH, !SCRATCH_PATTERN),
                ]);
                bench.host.send(host_txns.iter().copied());
                // STATUS last, so it sees the FIFO empty.
                bench.host.send([
                    Transaction::Read { addr: reg::SCRATCH },
                    Transaction::Read { addr: reg::STATUS },
                ]);
                sent = host_txns.len() + 9;
            }
            Stage::Host if now == (true, 1) && bench.host.replies.len() == 7 => {
                // The host's last frame is up and everything answered:
                // disconnect.
                stage = Stage::Back;
                bench.dtr_n = 1;
            }
            Stage::Back if now == (false, 1) => break,
            _ => {}
        }
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(stage, Stage::Back, "the host's frames were never all shown");
    assert_eq!(last, Some((false, 1)), "the ROM scene didn't start again");

    // The replies, in order.
    let r = &bench.host.replies;
    assert_eq!(r[0], ID_VALUE, "ID");
    assert_eq!(
        RenderMode::from_bits(r[2]),
        RenderMode::default(),
        "RENDER_MODE as the ROM's last primitive left it"
    );
    assert_eq!(r[3], 0, "SCRATCH from reset");
    assert_eq!(r[4], SCRATCH_PATTERN, "SCRATCH written");
    assert_eq!(
        r[5], !SCRATCH_PATTERN,
        "SCRATCH written again, after drawing"
    );
    let (before, after) = (Status::from_bits(r[1]), Status::from_bits(r[6]));
    println!("STATUS before {before:?}, after {after:?}");
    assert!(before.frames >= 1, "STATUS frames {}", before.frames);
    assert_eq!(after.fifo_free, 512, "the FIFO drained");
    assert!(after.render_cycles > 0);
    // The ROM's frame count only grows; the host's two frames were shown.
    assert!(after.frames >= before.frames + 2);

    // Credits for every 64 transactions taken; nothing overflowed.
    assert_eq!(bench.host.sent, sent);
    assert_eq!(
        bench.host.credit_bytes,
        sent / usize::try_from(CREDIT_BATCH)?,
        "credits"
    );
    assert_eq!(bench.host.unknown_bytes, 0);
    assert!(bench.host.credits() <= FIFO_ENTRIES);
    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
