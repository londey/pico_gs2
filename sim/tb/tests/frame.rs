//! Twin test of the whole core (`hw/gpu.snt`) with the SDRAM model on its
//! bus: from reset, through the SDRAM's power-up sequence and the
//! framebuffer fill, until the first frames of the spinning cube have been
//! drawn into the triple-buffered framebuffer and shown. The test decodes
//! the TMDS symbols as a DVI monitor would, checks the sync timing at every
//! pixel and that each frame shown is one of the cube's, exactly as the
//! reference renderer draws it (each framebuffer pixel as 2x2 screen
//! pixels), and in order; checks the SDRAM's contents and that the
//! controller broke none of the datasheet's rules; reports how long a frame
//! takes to draw; and writes the first frame to `build/framebuffer.png`.
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
use setup_model::render::{run as run_scene, Frame};
use setup_model::scene::{cube_scene, CUBE_FRAMES};
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
/// quarter of a frame, the first cube frame is drawn and shown after that,
/// and the frames shown from then on are checked.
const MAX_FRAMES: usize = 12;

/// Frames shown from SDRAM to check.
const CHECKED_FRAMES: usize = 4;

/// LED bits: SDRAM initialised, framebuffer filled, scan-out underflow.
const LED_SDRAM_READY: u8 = 1 << 1;
const LED_FILLED: u8 = 1 << 2;
const LED_UNDERFLOW: u8 = 1 << 3;

/// The core clock, in hertz.
const CLOCK_HZ: f64 = 75e6;

/// The frames the scene ends, in order, as the reference renderer draws
/// them: the spinning cube, a frame per step of its turn.
fn expected() -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    run_scene(&mut frame, &cube_scene()).0
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

    /// The (blue, green, red) TMDS symbols of the first frames shown from
    /// SDRAM once the first cube frame is, each in raster order.
    frames: Vec<Vec<[u16; 3]>>,

    /// The bank shown in the last of them.
    shown_bank: usize,

    /// Core cycles the last finished frame took to draw.
    render_cycles: u32,

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
/// shown, recording the core's inputs.
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
    let mut render_cycles = 0;
    // The frame being captured started after the first cube frame was
    // presented, so shows a cube frame.
    let mut capturing = false;
    let mut shown_bank = 0;
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
        render_cycles = out.debug_render_cycles_q;
        if cycle < SYMBOL_LATENCY {
            continue;
        }
        // A frame presented is shown from the next frame start; the symbols
        // follow the beam by SYMBOL_LATENCY cycles.
        if frame.is_empty() {
            capturing = out.debug_frames_q >= 1;
            shown_bank = usize::try_from(sim.register("disp").unwrap_or(0)).unwrap_or(0);
        }
        frame.push([out.tmds_blue_q, out.tmds_green_q, out.tmds_red_q]);
        if frame.len() == FRAME {
            if capturing {
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
        shown_bank,
        render_cycles,
        led,
    })
}

/// The frames shown from SDRAM are the cube's, each exactly as the
/// reference renderer draws it, and in order (each the one before it again,
/// or the next, as drawing a frame may take longer than showing one); the
/// sync timing is right at every pixel; the bank shown holds the last
/// frame, tiled; the SDRAM was driven by the book; and a frame takes less
/// than two frame times to draw.
#[test]
fn cube_frames_scan_out_from_sdram() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.frame.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let Run {
        bus,
        frames,
        shown_bank,
        render_cycles,
        led,
    } = run(&mut BufWriter::new(File::create(path)?))?;
    let reference = expected();
    assert_eq!(reference.len(), CUBE_FRAMES);
    assert!(
        bus.sdram.violations().is_empty(),
        "{} SDRAM violations: {:#?}",
        bus.sdram.violation_count(),
        bus.sdram.violations()
    );
    assert_eq!(
        frames.len(),
        CHECKED_FRAMES,
        "{CHECKED_FRAMES} frames not shown within {MAX_FRAMES}"
    );

    let mut shown: Option<usize> = None;
    let mut first_picture = None;
    for (n, symbols) in frames.iter().enumerate() {
        let picture = screen(n, symbols);
        let candidates = match shown {
            None => vec![0],
            Some(k) => vec![k, (k + 1) % CUBE_FRAMES],
        };
        let Some(k) = candidates
            .iter()
            .copied()
            .find(|&k| differing(&picture, &reference[k]) == 0)
        else {
            write_png(&picture, "framebuffer.png")?;
            let diffs: Vec<_> = candidates
                .iter()
                .map(|&k| (k, differing(&picture, &reference[k])))
                .collect();
            return Err(format!(
                "frame {n} is none of the cube frames expected: (frame, pixels differing) {diffs:?}"
            )
            .into());
        };
        println!("frame {n} shows cube frame {k}");
        shown = Some(k);
        first_picture.get_or_insert(picture);
    }

    // The bank shown holds the last frame, tiled.
    let last = &reference[shown.ok_or("no frames checked")?];
    for y in 0..FB_HEIGHT {
        for x in 0..FB_WIDTH {
            let addr = tiled(x, y);
            assert_eq!(
                bus.sdram.peek(shown_bank, addr >> 9, addr & 0x1FF),
                last[y * FB_WIDTH + x],
                "SDRAM bank {shown_bank} word for ({x}, {y})"
            );
        }
    }

    println!(
        "a frame takes {render_cycles} cycles to draw: {:.1} frames per second at 75 MHz",
        CLOCK_HZ / f64::from(render_cycles)
    );
    assert!(render_cycles > 0, "no frame was finished");
    assert!(
        usize::try_from(render_cycles)? < 2 * FRAME,
        "drawing a frame takes {render_cycles} cycles, over two frame times"
    );
    assert_eq!(
        led & (LED_SDRAM_READY | LED_FILLED | LED_UNDERFLOW),
        LED_SDRAM_READY | LED_FILLED,
        "LEDs {led:#07b}"
    );
    write_png(
        &first_picture.ok_or("no frames checked")?,
        "framebuffer.png",
    )
}

/// How many screen pixels of a decoded picture differ from a framebuffer's
/// pixels, each shown as 2x2 screen pixels.
///
/// # Arguments
///
/// * `picture` - The picture, `H_ACTIVE * V_ACTIVE` pixels in raster order.
/// * `fb` - The framebuffer, RGB565, `FB_WIDTH * FB_HEIGHT` in raster order.
///
/// # Returns
///
/// The number of screen pixels that differ.
fn differing(picture: &[[u8; 3]], fb: &[u16]) -> usize {
    picture
        .iter()
        .enumerate()
        .filter(|&(i, p)| {
            let (x, y) = (i % H_ACTIVE, i / H_ACTIVE);
            *p != rgb888(fb[(y / SCALE) * FB_WIDTH + x / SCALE])
        })
        .count()
}

/// Decodes one frame's symbols: checks the sync control tokens in the
/// blanking, and returns the active picture.
///
/// # Arguments
///
/// * `n` - The frame's number, for messages.
/// * `frame` - The frame's (blue, green, red) symbols, in raster order.
///
/// # Returns
///
/// The active picture: `H_ACTIVE * V_ACTIVE` RGB pixels in raster order.
///
/// # Panics
///
/// If a blanking symbol isn't the one expected, or an active one isn't
/// data.
fn screen(n: usize, frame: &[[u16; 3]]) -> Vec<[u8; 3]> {
    let mut picture = Vec::with_capacity(H_ACTIVE * V_ACTIVE);
    for (i, symbols) in frame.iter().enumerate() {
        let (x, y) = (i % H_TOTAL, i / H_TOTAL);
        let [blue, green, red] = symbols.map(decode);
        if x < H_ACTIVE && y < V_ACTIVE {
            let (Symbol::Data(r), Symbol::Data(g), Symbol::Data(b)) = (red, green, blue) else {
                panic!("frame {n}, pixel ({x}, {y}): {red:?} {green:?} {blue:?}");
            };
            picture.push([r, g, b]);
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

/// Writes a decoded active picture as an RGB PNG in `build/`.
///
/// # Arguments
///
/// * `picture` - `H_ACTIVE * V_ACTIVE` pixels in raster order.
/// * `name` - The file's name.
///
/// # Errors
///
/// Returns any error from creating or encoding the file.
fn write_png(picture: &[[u8; 3]], name: &str) -> Result<(), Box<dyn Error>> {
    let path = build_path(&[name]);
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        H_ACTIVE as u32,
        V_ACTIVE as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(picture.as_flattened())?;
    Ok(())
}
