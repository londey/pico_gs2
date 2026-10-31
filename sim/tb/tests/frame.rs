//! Twin test of the whole core (`hw/gpu.snt`) with the SDRAM model on its
//! bus: from reset, through the SDRAM's power-up sequence and the
//! framebuffer fill, through the first two 1280x720 frames scanned out of
//! SDRAM (the second reuses the line buffer the first filled). The test
//! decodes the TMDS symbols as a DVI monitor would, checks the picture
//! (each framebuffer pixel as 2x2 screen pixels) and the sync timing at every
//! pixel, checks the SDRAM's contents and that the controller broke none of
//! the datasheet's rules, and writes the picture to `build/framebuffer.png`.

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use gpu_sim::{Gpu, Inputs, Outputs};
use sdram_model::Pins;
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

/// Frames to run: the SDRAM's power-up and the fill take about a quarter of
/// a frame, the scan-out starts at the next frame, and two are checked.
const MAX_FRAMES: usize = 4;

/// Frames scanned out of SDRAM to check.
const CHECKED_FRAMES: usize = 2;

/// LED bits: SDRAM initialised, framebuffer filled, scan-out underflow.
const LED_SDRAM_READY: u8 = 1 << 1;
const LED_FILLED: u8 = 1 << 2;
const LED_UNDERFLOW: u8 = 1 << 3;

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

    /// The (blue, green, red) TMDS symbols of the first frames whose first
    /// pixel isn't black, each in raster order.
    frames: Vec<Vec<[u16; 3]>>,

    /// The LEDs at the end.
    led: u8,
}

/// Steps the core and the SDRAM from reset until a frame has been scanned
/// out of the framebuffer.
fn run() -> Run {
    let mut sim = Gpu::new();
    let mut bus = SdramBus::new();
    let mut frames = Vec::new();
    let mut frame = Vec::with_capacity(FRAME);
    let mut led = 0;
    for cycle in 0..MAX_FRAMES * FRAME + SYMBOL_LATENCY {
        let out = sim.step(&Inputs {
            sdram_dq: bus.dq_in(),
        });
        bus.clock(&pins(&out));
        led = out.out_led;
        if cycle < SYMBOL_LATENCY {
            continue;
        }
        frame.push([out.tmds_blue_q, out.tmds_green_q, out.tmds_red_q]);
        if frame.len() == FRAME {
            // Before the scan-out starts the picture is black; the first
            // frame scanned out starts with its white border.
            if !frames.is_empty() || decode(frame[0][2]) != Symbol::Data(0) {
                frames.push(std::mem::take(&mut frame));
                if frames.len() == CHECKED_FRAMES {
                    break;
                }
            }
            frame.clear();
        }
    }
    Run { bus, frames, led }
}

/// The first frame scanned out of SDRAM shows the test pattern, the sync
/// timing is right at every pixel, and the SDRAM was driven by the book.
#[test]
fn framebuffer_scans_out_from_sdram() -> Result<(), Box<dyn Error>> {
    let Run { bus, frames, led } = run();
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
        .map(|(n, frame)| check_frame(n, frame))
        .collect();

    // The framebuffer: word y * 640 + x, in bank 0 from row 0.
    for y in 0..FB_HEIGHT {
        for x in 0..FB_WIDTH {
            let addr = y * FB_WIDTH + x;
            assert_eq!(
                bus.sdram.peek(0, addr >> 9, addr & 0x1FF),
                pattern(x, y),
                "SDRAM word for ({x}, {y})"
            );
        }
    }

    assert_eq!(
        led & (LED_SDRAM_READY | LED_FILLED | LED_UNDERFLOW),
        LED_SDRAM_READY | LED_FILLED,
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
fn check_frame(n: usize, frame: &[[u16; 3]]) -> Vec<u8> {
    let mut picture = Vec::with_capacity(H_ACTIVE * V_ACTIVE * 3);
    for (i, symbols) in frame.iter().enumerate() {
        let (x, y) = (i % H_TOTAL, i / H_TOTAL);
        let [blue, green, red] = symbols.map(decode);
        if x < H_ACTIVE && y < V_ACTIVE {
            let [r, g, b] = rgb888(pattern(x / SCALE, y / SCALE));
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
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "build",
        "framebuffer.png",
    ]
    .iter()
    .collect();
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
