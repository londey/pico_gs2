//! Twin test of the whole core (`hw/gpu.snt`) with the SDRAM model on its
//! bus and no host on the USB UART: from reset, through the SDRAM's
//! power-up sequence and the framebuffer fill, until the first frames of the
//! built-in scene (the spinning cube, register transactions in ROM) have
//! been drawn into the triple-buffered framebuffer and shown. The test
//! decodes the TMDS symbols as a DVI monitor would, checks the sync timing
//! at every pixel and that each frame shown is one of the cube's, exactly as
//! the reference renderer draws it from the same transactions (each
//! framebuffer pixel as 2x2 screen pixels), and in order; checks the SDRAM's
//! contents and that the controller broke none of the datasheet's rules;
//! reports how long a frame takes to draw; and writes the first frame to
//! `build/framebuffer.png`.
//!
//! The core's input on every cycle is recorded as a `sinter sim` stimulus
//! file, `build/stimulus/gpu.frame.csv`, which `just check` replays with
//! `--cosim` to check the generated Verilog over the same run.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use setup_model::render::{render, Frame};
use setup_model::scene::{cube_scene, transactions, CUBE_FRAMES};
use tb::{
    differing, screen, Bench, CLOCK_HZ, FB_HEIGHT, FB_WIDTH, FRAME, H_ACTIVE, LED_FILLED,
    LED_SDRAM_READY, LED_UNDERFLOW, SYMBOL_LATENCY, V_ACTIVE,
};

/// Frames to run at most: the SDRAM's power-up and the fill take about a
/// quarter of a frame, the first cube frame is drawn and shown after that,
/// and the frames shown from then on are checked.
const MAX_FRAMES: usize = 12;

/// Frames shown from SDRAM to check.
const CHECKED_FRAMES: usize = 4;

/// The frames the scene presents, in order, as the reference renderer
/// draws them from the scene ROM's transactions.
fn expected() -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    render(&mut frame, &transactions(&cube_scene())).0
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

/// The frames shown from SDRAM are the cube's, each exactly as the
/// reference renderer draws it, and in order (each the one before it again,
/// or the next, as drawing a frame may take longer than showing one); the
/// sync timing is right at every pixel; the bank shown holds the last
/// frame, tiled; the SDRAM was driven by the book; the host link saw
/// nothing; and a frame takes less than two frame times to draw.
#[test]
fn cube_frames_scan_out_from_sdram() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.frame.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(BufWriter::new(File::create(path)?), "sim/tb/tests/frame.rs")?;
    bench.snapshot = true;
    let reference = expected();
    assert_eq!(reference.len(), CUBE_FRAMES);

    let mut shown: Option<usize> = None;
    let mut checked = 0;
    let mut last_bank = 0;
    let mut last_pixels = None;
    let mut first_picture = None;
    let mut out = None;
    while checked < CHECKED_FRAMES && bench.cycle < MAX_FRAMES * FRAME + SYMBOL_LATENCY {
        let (o, frame) = bench.step()?;
        out = Some(o);
        // A frame presented is shown from the next frame start.
        let Some(frame) = frame.filter(|f| f.presented >= 1) else {
            continue;
        };
        let picture = screen(&frame.symbols).map_err(|e| format!("frame {checked}: {e}"))?;
        // The first frame shown is the cube's first; after that each is the
        // one before or the next.
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
                "frame {checked} is none of the cube frames expected: (frame, pixels differing) {diffs:?}"
            )
            .into());
        };
        println!("frame {checked} shows cube frame {k}");
        shown = Some(k);
        last_bank = frame.bank;
        last_pixels = frame.pixels;
        first_picture.get_or_insert(picture);
        checked += 1;
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(
        checked, CHECKED_FRAMES,
        "{CHECKED_FRAMES} frames not shown within {MAX_FRAMES}"
    );
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations: {:#?}",
        bench.bus.sdram.violation_count(),
        bench.bus.sdram.violations()
    );

    // The bank shown last held that frame, tiled, while it was shown (read
    // as the GPU reads it, through the cleared flags). Not afterwards: the
    // GPU may already be drawing the next frame into it.
    let last = &reference[shown.ok_or("no frames checked")?];
    let pixels = last_pixels.ok_or("no snapshot of the bank shown")?;
    if let Some(i) = (0..last.len()).find(|&i| pixels[i] != last[i]) {
        return Err(format!(
            "SDRAM bank {last_bank} differs at ({}, {})",
            i % FB_WIDTH,
            i / FB_WIDTH
        )
        .into());
    }

    let cycles = out.debug_render_cycles_q;
    println!(
        "a frame takes {cycles} cycles to draw: {:.1} frames per second at 75 MHz",
        CLOCK_HZ / f64::from(cycles)
    );
    assert!(cycles > 0, "no frame was finished");
    assert!(
        usize::try_from(cycles)? < 2 * FRAME,
        "drawing a frame takes {cycles} cycles, over two frame times"
    );
    assert_eq!(
        out.out_led & (LED_SDRAM_READY | LED_FILLED | LED_UNDERFLOW),
        LED_SDRAM_READY | LED_FILLED,
        "LEDs {:#07b}",
        out.out_led
    );
    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert_eq!(out.out_usb_tx, 1, "the link sent something with no host");
    write_png(
        &first_picture.ok_or("no frames checked")?,
        "framebuffer.png",
    )
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
        u32::try_from(H_ACTIVE)?,
        u32::try_from(V_ACTIVE)?,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(picture.as_flattened())?;
    Ok(())
}
