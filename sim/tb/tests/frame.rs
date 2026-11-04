//! Twin test of the whole core (`hw/gpu.snt`) with the SDRAM model on its
//! bus: from reset, through the SDRAM's power-up sequence and the
//! framebuffer fill, through the first two 1280x720 frames scanned out of
//! SDRAM (the second reuses the line buffer the first filled). The test
//! decodes the TMDS symbols as a DVI monitor would, checks the picture
//! (each framebuffer pixel as 2x2 screen pixels) and the sync timing at every
//! pixel, checks the SDRAM's contents and that the controller broke none of
//! the datasheet's rules, and writes the picture to `build/framebuffer.png`.
//!
//! The core's input on every cycle is recorded as a `sinter sim` stimulus
//! file, `build/stimulus/gpu.frame.csv`, which `just check` replays with
//! `--cosim` to check the generated Verilog over the same run.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use gpu_sim::{Gpu, Inputs, Outputs};
use sdram_model::Pins;
use setup_model::render::render;
use setup_model::scene::{encode, test_card};
use tb::{decode, SdramBus, Symbol};

/// Active pixels per line (CEA-861 1280x720 at 60 Hz).
const H_ACTIVE: usize = 1280;

/// First pixel of the horizontal sync pulse.
const H_SYNC_START: usize = 1390;

/// First pixel after the horizontal sync pulse.
const H_SYNC_END: usize = 1430;

/// Pixels per line, including blanking.
const H_TOTAL: usize = 1650;

/// Active lines per frame.
const V_ACTIVE: usize = 720;

/// First line of the vertical sync pulse.
const V_SYNC_START: usize = 725;

/// First line after the vertical sync pulse.
const V_SYNC_END: usize = 730;

/// Lines per frame, including blanking.
const V_TOTAL: usize = 750;

/// Pixel clocks (and core cycles) per frame.
const FRAME: usize = H_TOTAL * V_TOTAL;

/// Framebuffer width, in pixels.
const FB_WIDTH: usize = 640;

/// Framebuffer height, in pixels.
const FB_HEIGHT: usize = 360;

/// Screen pixels per framebuffer pixel, across and down.
const SCALE: usize = 2;

/// Core cycles before the symbol outputs show pixel (0, 0): one to read the
/// line buffer, four in the encoder pipeline, and one in the core's symbol
/// registers.
const SYMBOL_LATENCY: usize = 6;

/// Frames to run at most: the SDRAM's power-up and the fill take about a
/// quarter of a frame, the scene is drawn after that, and two frames that
/// start after it is drawn are checked.
const MAX_FRAMES: usize = 8;

/// Frames scanned out of SDRAM to check.
const CHECKED_FRAMES: usize = 2;

/// LED bits: SDRAM initialised, framebuffer filled, scan-out underflow,
/// scene drawn.
const LED_SDRAM_READY: u8 = 1 << 1;
const LED_FILLED: u8 = 1 << 2;
const LED_UNDERFLOW: u8 = 1 << 3;
const LED_DRAWN: u8 = 1 << 4;

/// What the framebuffer should hold once the scene is drawn: the test
/// pattern, with the built-in test card drawn over it by the reference
/// renderer.
fn expected() -> Vec<u16> {
    let mut fb: Vec<u16> = (0..FB_HEIGHT)
        .flat_map(|y| (0..FB_WIDTH).map(move |x| pattern(x, y)))
        .collect();
    render(&mut fb, &encode(&test_card()));
    fb
}

/// The test pattern `hw/gpu.snt` writes, as RGB565: a white border, red
/// rising to the right, green rising downwards, blue on alternate 32x32
/// squares.
fn pattern(x: usize, y: usize) -> u16 {
    if x == 0 || x == FB_WIDTH - 1 || y == 0 || y == FB_HEIGHT - 1 {
        return 0xFFFF;
    }
    let red = (x >> 5) & 0x1F;
    let green = (y >> 3) & 0x3F;
    let blue = if (x ^ y) & 32 != 0 { 0x1F } else { 0 };
    u16::try_from((red << 11) | (green << 5) | blue).expect("16 bits")
}

/// Blocks across the framebuffer.
const BLOCKS_X: usize = FB_WIDTH / 8;

/// The word address of framebuffer pixel `(x, y)`: the framebuffer is tiled
/// in 8x8 blocks of 64 words, in raster order of blocks and of the pixels
/// in each (ADR 0003).
fn tiled(x: usize, y: usize) -> usize {
    ((y / 8) * BLOCKS_X + x / 8) * 64 + (y % 8) * 8 + x % 8
}

/// An RGB565 word as the 8-bit components the core sends, each widened by
/// repeating its top bits.
fn rgb888(word: u16) -> [u8; 3] {
    let (r, g, b) = (word >> 11, (word >> 5) & 0x3F, word & 0x1F);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
    .map(|c| u8::try_from(c).expect("8 bits"))
}

/// The SDRAM pins the core drives.
fn pins(o: &Outputs) -> Pins {
    Pins {
        cke: o.out_sdram_cke == 1,
        cs_n: o.out_sdram_cs_n == 1,
        ras_n: o.out_sdram_ras_n == 1,
        cas_n: o.out_sdram_cas_n == 1,
        we_n: o.out_sdram_we_n == 1,
        ba: o.out_sdram_ba,
        a: o.out_sdram_a,
        dqm: o.out_sdram_dqm,
        dq: (o.out_sdram_dq_oe == 1).then_some(o.out_sdram_dq_out),
    }
}

/// What the run produced.
struct Run {
    /// The SDRAM and its bus, at the end.
    bus: SdramBus,

    /// The (blue, green, red) TMDS symbols of the first frames that start
    /// after the scene is drawn, each in raster order.
    frames: Vec<Vec<[u16; 3]>>,

    /// The cycle the scene was drawn by.
    drawn_at: Option<usize>,

    /// The LEDs at the end.
    led: u8,
}

/// A path under the repository's `build/` directory.
///
/// # Arguments
///
/// * `parts` - The path's components below `build/`.
///
/// # Returns
///
/// The path.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// Steps the core and the SDRAM from reset until the frames have been
/// scanned out of the framebuffer, recording the core's inputs.
///
/// # Arguments
///
/// * `stimulus` - Where to write the stimulus file: a header, then the
///   core's input on each cycle.
///
/// # Returns
///
/// The run's results.
///
/// # Errors
///
/// Returns any error from writing the stimulus.
fn run(stimulus: &mut impl Write) -> std::io::Result<Run> {
    let mut sim = Gpu::new();
    let mut bus = SdramBus::new();
    let mut frames = Vec::new();
    let mut frame = Vec::with_capacity(FRAME);
    let mut led = 0;
    let mut drawn_at = None;
    writeln!(
        stimulus,
        "# Recorded by sim/tb/tests/frame.rs; replayed by `just check`."
    )?;
    writeln!(stimulus, "sdram_dq")?;
    for cycle in 0..MAX_FRAMES * FRAME + SYMBOL_LATENCY {
        // The pins are registered: the SDRAM answers in the same cycle.
        let pins_now = pins(&sim.eval(&Inputs { sdram_dq: 0 }));
        let inputs = Inputs {
            sdram_dq: bus.clock(&pins_now),
        };
        writeln!(stimulus, "{}", inputs.sdram_dq)?;
        let out = sim.step(&inputs);
        led = out.out_led;
        if drawn_at.is_none() && led & LED_DRAWN != 0 {
            drawn_at = Some(cycle);
        }
        if cycle < SYMBOL_LATENCY {
            continue;
        }
        // A frame is checked if the scene was drawn before it started.
        let drawn_before = drawn_at.is_some_and(|c| c + FRAME <= cycle - frame.len());
        frame.push([out.tmds_blue_q, out.tmds_green_q, out.tmds_red_q]);
        if frame.len() == FRAME {
            if drawn_before {
                frames.push(std::mem::take(&mut frame));
                if frames.len() == CHECKED_FRAMES {
                    break;
                }
            }
            frame.clear();
        }
    }
    stimulus.flush()?;
    Ok(Run {
        bus,
        frames,
        drawn_at,
        led,
    })
}

/// Once the scene is drawn, the frames scanned out of SDRAM show the test
/// pattern with the test card drawn over it exactly as the reference
/// renderer draws it, the sync timing is right at every pixel, the
/// framebuffer in SDRAM is exactly the reference's, and the SDRAM was
/// driven by the book.
#[test]
fn framebuffer_scans_out_from_sdram() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.frame.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let Run {
        bus,
        frames,
        drawn_at,
        led,
    } = run(&mut BufWriter::new(File::create(path)?))?;
    let drawn_at = drawn_at.ok_or("the scene was never drawn")?;
    println!(
        "scene drawn by cycle {drawn_at} ({:.2} frames)",
        f64::from(u32::try_from(drawn_at).expect("small"))
            / f64::from(u32::try_from(FRAME).expect("small"))
    );
    let fb = expected();
    assert_eq!(
        frames.len(),
        CHECKED_FRAMES,
        "{CHECKED_FRAMES} frames not scanned out within {MAX_FRAMES}"
    );
    assert!(
        bus.sdram.violations().is_empty(),
        "{} SDRAM violations: {:#?}",
        bus.sdram.violation_count(),
        bus.sdram.violations()
    );

    let pictures: Vec<_> = frames
        .iter()
        .enumerate()
        .map(|(n, frame)| check_frame(n, frame, &fb))
        .collect();

    // The framebuffer, tiled, in bank 0 from row 0.
    for y in 0..FB_HEIGHT {
        for x in 0..FB_WIDTH {
            let addr = tiled(x, y);
            assert_eq!(
                bus.sdram.peek(0, addr >> 9, addr & 0x1FF),
                fb[y * FB_WIDTH + x],
                "SDRAM word for ({x}, {y})"
            );
        }
    }

    assert_eq!(
        led & (LED_SDRAM_READY | LED_FILLED | LED_UNDERFLOW | LED_DRAWN),
        LED_SDRAM_READY | LED_FILLED | LED_DRAWN,
        "LEDs {led:#07b}"
    );
    write_png(&pictures[0])
}

/// Checks one frame's symbols: the test pattern in the active picture, and
/// the sync control tokens in the blanking.
///
/// # Arguments
///
/// * `n` - The frame's number, for messages.
/// * `frame` - The frame's (blue, green, red) symbols, in raster order.
///
/// # Returns
///
/// The decoded active picture: `H_ACTIVE * V_ACTIVE` pixels, 3 bytes each.
///
/// # Panics
///
/// If any symbol isn't the one expected.
fn check_frame(n: usize, frame: &[[u16; 3]], fb: &[u16]) -> Vec<u8> {
    let mut picture = Vec::with_capacity(H_ACTIVE * V_ACTIVE * 3);
    for (i, symbols) in frame.iter().enumerate() {
        let (x, y) = (i % H_TOTAL, i / H_TOTAL);
        let [blue, green, red] = symbols.map(decode);
        if x < H_ACTIVE && y < V_ACTIVE {
            let [r, g, b] = rgb888(fb[(y / SCALE) * FB_WIDTH + x / SCALE]);
            assert_eq!(
                (red, green, blue),
                (Symbol::Data(r), Symbol::Data(g), Symbol::Data(b)),
                "frame {n}, pixel ({x}, {y})"
            );
            picture.extend([r, g, b]);
        } else {
            // 720p's syncs are active high.
            let hsync = u8::from((H_SYNC_START..H_SYNC_END).contains(&x));
            let vsync = u8::from((V_SYNC_START..V_SYNC_END).contains(&y));
            assert_eq!(
                (blue, green, red),
                (
                    Symbol::Control((vsync << 1) | hsync),
                    Symbol::Control(0),
                    Symbol::Control(0)
                ),
                "frame {n}, blanking at ({x}, {y})"
            );
        }
    }
    picture
}

/// Writes the decoded active picture as an RGB PNG in `build/`.
///
/// # Arguments
///
/// * `rgb` - `H_ACTIVE * V_ACTIVE` pixels, 3 bytes each, in raster order.
///
/// # Errors
///
/// Returns any error from creating or encoding the file.
fn write_png(rgb: &[u8]) -> Result<(), Box<dyn Error>> {
    let path = build_path(&["framebuffer.png"]);
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        H_ACTIVE as u32,
        V_ACTIVE as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
    Ok(())
}
