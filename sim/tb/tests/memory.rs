//! Twin test of GPU memory access (`docs/design/host-interface.md` §9):
//! `MEM_ADDR` and `MEM_DATA` over the serial link, with the whole core and
//! the SDRAM model.
//!
//! Once the built-in scene's first frame is up, a host connects and:
//!
//! 1. writes 40 words into its memory in bank 2, reads `MEM_ADDR` (moved
//!    on by the 10 dwords written), and reads the words back;
//! 2. clears a frame of its own and draws a triangle, without presenting,
//!    then reads `STATUS` for the buffer being drawn;
//! 3. reads the four blocks under the triangle's corner from that buffer.
//!
//! The words read back are the words written, and they are in the SDRAM
//! where §9.2's memory map puts them. The blocks read are the reference
//! renderer's frame after the clear and the triangle: a `MEM_DATA` read
//! waits for drawing before it to finish. The host link reports no errors
//! and `ID` reads interface version 2.
//!
//! The core's input on every cycle is recorded as
//! `build/stimulus/gpu.memory.csv`, which `just check` replays with
//! `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use gs_proto::{
    colour_buffer, host_memory, mem_read, mem_write, pixel_word, reg, unpack_words, Attr0,
    Frame as FrameReg, Position, RenderMode, Status, Transaction, BANK_DWORDS, ID_VALUE,
};
use setup_model::render::{render, Frame};
use tb::{Bench, FB_HEIGHT, FB_WIDTH, FRAME};

/// Frames to run at most.
const MAX_FRAMES: usize = 8;

/// The host's background colour, RGB565: dark blue.
const HOST_BACKGROUND: u16 = 0x0010;

/// Where the test words go: an odd dword in the host's memory in bank 2.
const TEST_DWORD: u32 = host_memory(2) + 0x123;

/// Words written and read back.
const TEST_WORDS: usize = 40;

/// The blocks read back from the frame being drawn: 2 × 2 blocks of 8 × 8
/// pixels from (16, 16), over the triangle's corner and its edges.
const BLOCKS: [(usize, usize); 4] = [(2, 2), (3, 2), (2, 3), (3, 3)];

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// The test words: every bit of each word used, no two alike.
fn test_words() -> Vec<u16> {
    (0..TEST_WORDS)
        .map(|i| {
            let i = u16::try_from(i).unwrap_or(0);
            i.wrapping_mul(0x9E37) ^ 0xA5C3
        })
        .collect()
}

/// The host's frame: a clear, then one red triangle, not presented.
fn host_frame() -> Vec<Transaction> {
    let at = |x: i16, y: i16| Position {
        x: x * 16,
        y: y * 16,
        z: 100,
        q: 0,
    };
    let red = Attr0 {
        r: 255,
        a: 255,
        ..Attr0::default()
    };
    vec![
        Transaction::write(
            reg::FRAME,
            FrameReg {
                clear_colour: true,
                clear_depth: true,
                colour: HOST_BACKGROUND,
                ..FrameReg::default()
            }
            .bits(),
        ),
        Transaction::write(reg::RENDER_MODE, RenderMode::default().bits()),
        Transaction::write(reg::ATTR0, red.bits()),
        Transaction::write(reg::VERTEX_NOKICK, at(20, 18).bits()),
        Transaction::write(reg::VERTEX_NOKICK, at(100, 22).bits()),
        Transaction::write(reg::VERTEX_KICK_012, at(26, 90).bits()),
    ]
}

/// What the host's frame looks like after its triangle, per the reference.
fn host_reference() -> Vec<u16> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    render(&mut frame, &host_frame());
    frame.colour
}

/// Which part of the test a run is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// No host yet: waiting for the ROM's first frame.
    Rom,
    /// Connected: waiting for the replies to the writes, reads and `STATUS`.
    Written,
    /// Reading the blocks back.
    Reading,
}

/// Memory written and read back over the link; a frame being drawn read
/// back, as above.
#[test]
fn memory_reads_back() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.memory.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(
        BufWriter::new(File::create(path)?),
        "sim/tb/tests/memory.rs",
    )?;
    let words = test_words();
    let dwords = TEST_WORDS / 4;
    // Replies before the blocks: ID, MEM_ADDR, the words, STATUS.
    let first_replies = 2 + dwords + 1;
    let block_dwords = 16;

    let mut stage = Stage::Rom;
    let mut drawn = 0;
    let mut out = None;
    while bench.cycle < MAX_FRAMES * FRAME {
        let (o, shown) = bench.step()?;
        out = Some(o);
        match stage {
            Stage::Rom if shown.is_some_and(|f| f.presented >= 1) => {
                stage = Stage::Written;
                bench.dtr_n = 0;
                bench.host.reconnect();
                bench.host.send([Transaction::Read { addr: reg::ID }]);
                bench.host.send(mem_write(TEST_DWORD, &words));
                bench.host.send([Transaction::Read {
                    addr: reg::MEM_ADDR,
                }]);
                bench.host.send(mem_read(TEST_DWORD, dwords));
                bench.host.send(host_frame());
                bench.host.send([Transaction::Read { addr: reg::STATUS }]);
            }
            Stage::Written if bench.host.replies.len() == first_replies => {
                let status = Status::from_bits(bench.host.replies[first_replies - 1]);
                drawn = u32::from(status.drawn);
                // Every field where it should be: the banks the core is
                // using, no link errors, the FIFO empty behind the read.
                let bank = |name: &str| bench.sim.register(name).and_then(|b| u8::try_from(b).ok());
                assert_eq!(Some(status.drawn), bank("draw"), "STATUS drawn");
                assert_eq!(Some(status.shown), bank("disp"), "STATUS shown");
                assert!(
                    !status.link_malformed && !status.link_overflow,
                    "{status:?}"
                );
                assert_eq!(status.fifo_free, 512, "{status:?}");
                println!("cycle {}: {status:?}", bench.cycle);
                for &(bx, by) in &BLOCKS {
                    let word = pixel_word(bx * 8, by * 8);
                    let dword = colour_buffer(drawn) + u32::try_from(word / 4)?;
                    bench.host.send(mem_read(dword, block_dwords));
                }
                stage = Stage::Reading;
            }
            Stage::Reading
                if bench.host.replies.len() == first_replies + BLOCKS.len() * block_dwords =>
            {
                break;
            }
            _ => {}
        }
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(stage, Stage::Reading, "the replies never all came back");
    let r = &bench.host.replies;
    assert_eq!(
        r.len(),
        first_replies + BLOCKS.len() * block_dwords,
        "the blocks were never all read back"
    );

    // 1. The words, MEM_ADDR moved on, and the SDRAM holding them where the
    // memory map says.
    assert_eq!(r[0], ID_VALUE, "ID");
    assert_eq!(
        r[1],
        u64::from(TEST_DWORD) + u64::try_from(dwords)?,
        "MEM_ADDR after the writes"
    );
    let read: Vec<u16> = r[2..2 + dwords]
        .iter()
        .flat_map(|&d| unpack_words(d))
        .collect();
    assert_eq!(read, words, "the words read back");
    let bank = usize::try_from(TEST_DWORD / BANK_DWORDS)?;
    for (i, &w) in words.iter().enumerate() {
        let addr = usize::try_from(TEST_DWORD % BANK_DWORDS)? * 4 + i;
        assert_eq!(
            bench.bus.sdram.peek(bank, addr >> 9, addr & 0x1FF),
            w,
            "word {i} in the SDRAM"
        );
    }

    // 2 and 3. The frame being drawn, read back under the triangle.
    let reference = host_reference();
    let mut red = 0;
    for (k, &(bx, by)) in BLOCKS.iter().enumerate() {
        let start = first_replies + k * block_dwords;
        let block: Vec<u16> = r[start..start + block_dwords]
            .iter()
            .flat_map(|&d| unpack_words(d))
            .collect();
        for (i, &got) in block.iter().enumerate() {
            let (x, y) = (bx * 8 + i % 8, by * 8 + i / 8);
            assert_eq!(
                got,
                reference[y * FB_WIDTH + x],
                "pixel ({x}, {y}) of buffer {drawn}"
            );
            red += usize::from(got == 0xF800);
        }
    }
    println!(
        "{red} of {} pixels read are the triangle's",
        BLOCKS.len() * 64
    );
    assert!(
        red > 30 && red < 200,
        "the blocks cover the triangle's corner"
    );

    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert_eq!(bench.host.unknown_bytes, 0);
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
