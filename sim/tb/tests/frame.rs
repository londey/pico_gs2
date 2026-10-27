//! Twin test of the whole core (`hw/gpu.snt`): steps the generated simulator
//! through one 640x480 frame, decodes the TMDS symbols as a DVI monitor
//! would, checks the picture and the sync timing at every pixel, and writes
//! the picture to `build/test_pattern.png`.

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use gpu_sim::{Gpu, Inputs};
use tb::{decode, Symbol};

/// Active pixels per line (VESA 640x480 at 60 Hz).
const H_ACTIVE: usize = 640;

/// First pixel of the horizontal sync pulse.
const H_SYNC_START: usize = 656;

/// First pixel after the horizontal sync pulse.
const H_SYNC_END: usize = 752;

/// Pixels per line, including blanking.
const H_TOTAL: usize = 800;

/// Active lines per frame.
const V_ACTIVE: usize = 480;

/// First line of the vertical sync pulse.
const V_SYNC_START: usize = 490;

/// First line after the vertical sync pulse.
const V_SYNC_END: usize = 492;

/// Lines per frame, including blanking.
const V_TOTAL: usize = 525;

/// Index of the pixel domain in the simulator's `ticked()` (declaration
/// order: core, pixel).
const PIXEL_DOMAIN: usize = 1;

/// First line of the bottom quarter, where the bars are shown reversed.
const REVERSED_FROM_LINE: usize = 360;

/// The colour bars, left to right, as (R, G, B).
const BARS: [[u8; 3]; 8] = [
    [255, 255, 255],
    [255, 255, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [255, 0, 0],
    [0, 0, 255],
    [0, 0, 0],
];

/// Steps the simulator from reset and records the (blue, green, red) TMDS
/// symbols for each pixel of the first frame.
///
/// # Returns
///
/// `H_TOTAL * V_TOTAL` symbol triples in raster order.
fn first_frame() -> Vec<[u16; 3]> {
    let mut sim = Gpu::new();
    let mut symbols = Vec::with_capacity(H_TOTAL * V_TOTAL + 1);
    while symbols.len() < H_TOTAL * V_TOTAL + 1 {
        let out = sim.step(&Inputs::default());
        if sim.ticked()[PIXEL_DOMAIN] {
            symbols.push([out.tmds_blue_q, out.tmds_green_q, out.tmds_red_q]);
        }
    }
    // The symbol registers lag the beam by one pixel clock: the first edge
    // shows their reset value.
    symbols.remove(0);
    symbols
}

/// Every pixel of the first frame carries the right colour or control token.
#[test]
fn frame_matches_spec() -> Result<(), Box<dyn Error>> {
    let frame = first_frame();
    let mut picture = Vec::with_capacity(H_ACTIVE * V_ACTIVE * 3);

    for (i, symbols) in frame.iter().enumerate() {
        let (x, y) = (i % H_TOTAL, i / H_TOTAL);
        let [blue, green, red] = symbols.map(decode);

        if x < H_ACTIVE && y < V_ACTIVE {
            let bar = if y < REVERSED_FROM_LINE {
                x / 80
            } else {
                7 - x / 80
            };
            let [r, g, b] = BARS[bar];
            assert_eq!(
                (red, green, blue),
                (Symbol::Data(r), Symbol::Data(g), Symbol::Data(b)),
                "pixel ({x}, {y})"
            );
            picture.extend([r, g, b]);
        } else {
            let hsync_n = u8::from(!(H_SYNC_START..H_SYNC_END).contains(&x));
            let vsync_n = u8::from(!(V_SYNC_START..V_SYNC_END).contains(&y));
            assert_eq!(
                (blue, green, red),
                (
                    Symbol::Control((vsync_n << 1) | hsync_n),
                    Symbol::Control(0),
                    Symbol::Control(0)
                ),
                "blanking at ({x}, {y})"
            );
        }
    }

    write_png(&picture)
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
        "test_pattern.png",
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
