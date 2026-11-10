//! The whole core on a bench: the generated simulator (`gpu-sim`) with the
//! SDRAM model on its bus and a host on its USB UART, one call per core
//! cycle; its input on every cycle recorded as a `sinter sim` stimulus file;
//! and the displayed frames handed back whole, with the helpers to decode
//! and compare them as a DVI monitor would.

use std::io::{self, Write};

use gpu_sim::{Gpu, Inputs, Outputs};
use sdram_model::Pins;

use crate::host::Host;
use crate::sdram::SdramBus;
use crate::tmds::{decode, Symbol};

/// Active pixels per line (CEA-861 1280x720 at 60 Hz).
pub const H_ACTIVE: usize = 1280;

/// First pixel of the horizontal sync pulse.
pub const H_SYNC_START: usize = 1390;

/// First pixel after the horizontal sync pulse.
pub const H_SYNC_END: usize = 1430;

/// Pixels per line, including blanking.
pub const H_TOTAL: usize = 1650;

/// Active lines per frame.
pub const V_ACTIVE: usize = 720;

/// First line of the vertical sync pulse.
pub const V_SYNC_START: usize = 725;

/// First line after the vertical sync pulse.
pub const V_SYNC_END: usize = 730;

/// Lines per frame, including blanking.
pub const V_TOTAL: usize = 750;

/// Pixel clocks (and core cycles) per frame.
pub const FRAME: usize = H_TOTAL * V_TOTAL;

/// Framebuffer width, in pixels.
pub const FB_WIDTH: usize = 640;

/// Framebuffer height, in pixels.
pub const FB_HEIGHT: usize = 360;

/// Screen pixels per framebuffer pixel, across and down.
pub const SCALE: usize = 2;

/// Core cycles before the symbol outputs show pixel (0, 0): one to read the
/// line buffer, four in the encoder pipeline, and one in the core's symbol
/// registers.
pub const SYMBOL_LATENCY: usize = 6;

/// The core clock, in hertz.
pub const CLOCK_HZ: f64 = 75e6;

/// LED bits: SDRAM initialised, framebuffer filled, scan-out underflow.
pub const LED_SDRAM_READY: u8 = 1 << 1;

/// The framebuffer has been filled.
pub const LED_FILLED: u8 = 1 << 2;

/// A pixel's half block wasn't loaded in time.
pub const LED_UNDERFLOW: u8 = 1 << 3;

/// Blocks across the framebuffer.
const BLOCKS_X: usize = FB_WIDTH / 8;

/// The word address of framebuffer pixel `(x, y)` in its bank: the
/// framebuffer is tiled in 8x8 blocks of 64 words, in raster order of blocks
/// and of the pixels in each (ADR 0003).
///
/// # Arguments
///
/// * `x` - The pixel's column.
/// * `y` - The pixel's line.
///
/// # Returns
///
/// The word address.
pub fn tiled(x: usize, y: usize) -> usize {
    ((y / 8) * BLOCKS_X + x / 8) * 64 + (y % 8) * 8 + x % 8
}

/// An RGB565 word as the 8-bit components the core sends, each widened by
/// repeating its top bits.
///
/// # Arguments
///
/// * `word` - The colour.
///
/// # Returns
///
/// Red, green and blue.
pub fn rgb888(word: u16) -> [u8; 3] {
    let (r, g, b) = (word >> 11, (word >> 5) & 0x3F, word & 0x1F);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
    .map(|c| u8::try_from(c).unwrap_or(u8::MAX))
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

/// A frame the display showed.
#[derive(Clone, Debug)]
pub struct Shown {
    /// Its (blue, green, red) TMDS symbols, in raster order.
    pub symbols: Vec<[u16; 3]>,

    /// Frames the core had presented when it began (`debug_frames`).
    pub presented: u16,

    /// The colour buffer bank it was scanned out of.
    pub bank: usize,

    /// With [`Bench::snapshot`] set, that bank as the GPU read it when the
    /// frame began (RGB565, raster order): the buffer on screen, which the
    /// GPU never draws into while it is shown.
    pub pixels: Option<Vec<u16>>,
}

/// The core, the SDRAM and a host.
pub struct Bench<W: Write> {
    /// The core.
    pub sim: Gpu,

    /// The SDRAM and its bus.
    pub bus: SdramBus,

    /// The host on the USB UART.
    pub host: Host,

    /// DTR#, as the core sees it: 1 while no host has the port open.
    pub dtr_n: u8,

    /// Core cycles stepped.
    pub cycle: usize,

    /// Whether each frame shown carries the buffer it was shown from
    /// ([`Shown::pixels`]).
    pub snapshot: bool,

    /// The snapshot of the frame being collected.
    pixels: Option<Vec<u16>>,

    /// The core's transmit line, from the last cycle.
    gpu_tx: u8,

    /// The frame being collected, and what it began with.
    frame: Vec<[u16; 3]>,

    /// `debug_frames` and the bank shown when the frame being collected began.
    frame_start: (u16, usize),

    /// Where the stimulus goes.
    stimulus: W,
}

impl<W: Write> Bench<W> {
    /// A core out of reset, no host connected.
    ///
    /// # Arguments
    ///
    /// * `stimulus` - Where to record the core's inputs.
    /// * `recorder` - The test recording them, for the file's header.
    ///
    /// # Returns
    ///
    /// The bench.
    ///
    /// # Errors
    ///
    /// Any error writing the stimulus header.
    pub fn new(mut stimulus: W, recorder: &str) -> io::Result<Bench<W>> {
        writeln!(
            stimulus,
            "# Recorded by {recorder}; replayed by `just check`."
        )?;
        writeln!(stimulus, "sdram_dq, usb_rx, usb_dtr_n")?;
        Ok(Bench {
            sim: Gpu::new(),
            bus: SdramBus::new(),
            host: Host::default(),
            dtr_n: 1,
            cycle: 0,
            snapshot: false,
            pixels: None,
            gpu_tx: 1,
            frame: Vec::with_capacity(FRAME),
            frame_start: (0, 0),
            stimulus,
        })
    }

    /// One core cycle.
    ///
    /// # Returns
    ///
    /// The core's outputs, and a displayed frame if this cycle completes one.
    ///
    /// # Errors
    ///
    /// Any error writing the stimulus.
    pub fn step(&mut self) -> io::Result<(Outputs, Option<Shown>)> {
        // The pins are registered: the SDRAM answers in the same cycle. The
        // host sees the core's transmit line from the last cycle.
        let usb_rx = self.host.clock(self.gpu_tx);
        let pins_now = pins(&self.sim.eval(&Inputs {
            sdram_dq: 0,
            usb_rx,
            usb_dtr_n: self.dtr_n,
        }));
        let inputs = Inputs {
            sdram_dq: self.bus.clock(&pins_now),
            usb_rx,
            usb_dtr_n: self.dtr_n,
        };
        writeln!(
            self.stimulus,
            "{}, {}, {}",
            inputs.sdram_dq, inputs.usb_rx, inputs.usb_dtr_n
        )?;
        let out = self.sim.step(&inputs);
        self.gpu_tx = out.out_usb_tx;
        let cycle = self.cycle;
        self.cycle += 1;
        if cycle < SYMBOL_LATENCY {
            return Ok((out, None));
        }
        if self.frame.is_empty() {
            let bank = self
                .sim
                .register("disp")
                .and_then(|b| usize::try_from(b).ok())
                .unwrap_or(0);
            self.frame_start = (out.debug_frames_q, bank);
            self.pixels = self.snapshot.then(|| self.buffer(bank));
        }
        self.frame
            .push([out.tmds_blue_q, out.tmds_green_q, out.tmds_red_q]);
        if self.frame.len() < FRAME {
            return Ok((out, None));
        }
        let shown = Shown {
            symbols: std::mem::replace(&mut self.frame, Vec::with_capacity(FRAME)),
            presented: self.frame_start.0,
            bank: self.frame_start.1,
            pixels: self.pixels.take(),
        };
        Ok((out, Some(shown)))
    }

    /// Whether the display has ever run short of pixels: an active pixel
    /// whose half-block row wasn't in scan-out's ring (the core's sticky
    /// `underflow`, `led[3]`). It never should.
    ///
    /// # Returns
    ///
    /// True if it has.
    pub fn underflow(&self) -> bool {
        self.sim.register("underflow").unwrap_or(0) != 0
    }

    /// Flushes the stimulus.
    ///
    /// # Errors
    ///
    /// Any error writing it.
    pub fn finish(&mut self) -> io::Result<()> {
        self.stimulus.flush()
    }

    /// A word of one of the GPU's buffers as the GPU reads it (scan-out,
    /// `MEM_DATA`): the clear value where its block's cleared flag is set
    /// (`docs/design/draw-speed.md` §3.1), else the SDRAM's.
    ///
    /// # Arguments
    ///
    /// * `bank` - The buffer's bank: 0 to 2 colour, 3 depth.
    /// * `addr` - The word in the bank, below 3,600 blocks of 64.
    ///
    /// # Returns
    ///
    /// The word.
    pub fn buffer_word(&self, bank: usize, addr: usize) -> u16 {
        let flags = self
            .sim
            .memory("raster_cflags")
            .and_then(|m| m.get(addr / 64).copied())
            .unwrap_or(0);
        if flags >> bank & 1 == 0 {
            return self.bus.sdram.peek(bank, addr >> 9, addr & 0x1FF);
        }
        if bank == 3 {
            return 0xFFFF;
        }
        let colours = self.sim.register("raster_clear_colours").unwrap_or(0);
        u16::try_from(colours >> (16 * bank) & 0xFFFF).unwrap_or(0)
    }

    /// A colour buffer as the GPU reads it ([`Bench::buffer_word`]).
    ///
    /// # Arguments
    ///
    /// * `bank` - Its bank.
    ///
    /// # Returns
    ///
    /// Its pixels, RGB565, `FB_WIDTH * FB_HEIGHT` in raster order.
    pub fn buffer(&self, bank: usize) -> Vec<u16> {
        (0..FB_HEIGHT)
            .flat_map(|y| (0..FB_WIDTH).map(move |x| (x, y)))
            .map(|(x, y)| self.buffer_word(bank, tiled(x, y)))
            .collect()
    }
}

/// Decodes one frame's symbols: checks the sync control tokens in the
/// blanking, and returns the active picture.
///
/// # Arguments
///
/// * `frame` - The frame's (blue, green, red) symbols, in raster order.
///
/// # Returns
///
/// The active picture: `H_ACTIVE * V_ACTIVE` RGB pixels in raster order.
///
/// # Errors
///
/// A description of the first symbol that isn't the one expected.
pub fn screen(frame: &[[u16; 3]]) -> Result<Vec<[u8; 3]>, String> {
    let mut picture = Vec::with_capacity(H_ACTIVE * V_ACTIVE);
    for (i, symbols) in frame.iter().enumerate() {
        let (x, y) = (i % H_TOTAL, i / H_TOTAL);
        let [blue, green, red] = symbols.map(decode);
        if x < H_ACTIVE && y < V_ACTIVE {
            let (Symbol::Data(r), Symbol::Data(g), Symbol::Data(b)) = (red, green, blue) else {
                return Err(format!("pixel ({x}, {y}): {red:?} {green:?} {blue:?}"));
            };
            picture.push([r, g, b]);
        } else {
            // 720p's syncs are active high.
            let hsync = u8::from((H_SYNC_START..H_SYNC_END).contains(&x));
            let vsync = u8::from((V_SYNC_START..V_SYNC_END).contains(&y));
            let want = (
                Symbol::Control((vsync << 1) | hsync),
                Symbol::Control(0),
                Symbol::Control(0),
            );
            if (blue, green, red) != want {
                return Err(format!(
                    "blanking at ({x}, {y}): {blue:?} {green:?} {red:?}"
                ));
            }
        }
    }
    Ok(picture)
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
pub fn differing(picture: &[[u8; 3]], fb: &[u16]) -> usize {
    picture
        .iter()
        .enumerate()
        .filter(|&(i, p)| {
            let (x, y) = (i % H_ACTIVE, i / H_ACTIVE);
            *p != rgb888(fb[(y / SCALE) * FB_WIDTH + x / SCALE])
        })
        .count()
}
