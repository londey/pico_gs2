//! Performance figures for `docs/design/host-interface.md` §15, measured on
//! the twin: `just perf` runs it. With `--check` (`just check` passes it)
//! it fails if a scene draws more than 0.5% slower than its figure in
//! `sim/tb/perf-baseline.txt`; `just perf --record` writes the figures
//! there. It also fails if a scene is link-bound or the display ran short.
//!
//! Each scene is one frame (a clear of colour and depth, then the scene's
//! triangles, then a present), which a host on the USB UART sends
//! [`REPEATS`] times once the built-in scene is up. The figure is
//! `STATUS`'s draw time of the last frame (`debug_render_cycles`), which
//! counts every cycle from one frame's end to the next except those waiting
//! for the display with nothing left to draw. The scenes are small enough
//! (at most about 400 transactions) that the link, at 3 Mbaud, gets a whole
//! frame ahead of the GPU in the 512-transaction FIFO within a few frames,
//! so the last frames' draw times are the GPU's alone; the last two must
//! agree, or the scene is reported as link-bound. The last frame shown is
//! checked pixel-exact against the reference renderer, so a figure never
//! comes from a wrong picture.
//!
//! Textures are written straight into the SDRAM model (and the reference's
//! memory), not uploaded; the cache keeps them from frame to frame, so the
//! textured figures are with the cache warm, as for a texture used every
//! frame.

use std::error::Error;
use std::io;
use std::thread;

use gs_proto::{
    host_memory, pack_words, reg, texel_word, Attr0, Frame as FrameReg, Position, RenderMode,
    Tex0Cfg, TexFormat, Transaction, Wrap,
};
use setup_model::render::{render, Frame};
use setup_model::scene::{
    cube_at, cube_texture, textured_cube_at, transactions, CUBE_TEXTURE_LOG2,
};
use tb::{differing, screen, Bench, CLOCK_HZ, FB_HEIGHT, FB_WIDTH, FRAME};

/// Frames each scene is drawn.
const REPEATS: usize = 6;

/// Display frames to run at most per scene (a frame minifying a large
/// texture takes several).
const MAX_FRAMES: usize = 96;

/// The clear colour, RGB565: dark blue.
const BACKGROUND: u16 = 0x0010;

/// The cube's angle, radians: every face but the back one shows.
const CUBE_ANGLE: f32 = 0.6;

/// Q = 1, UQ1.15.
const Q_ONE: u16 = 0x8000;

/// One texture: its dword address, its width and height (log2), and its
/// texels, tiled.
struct Texture {
    /// Its dword address.
    dword: u32,

    /// Width and height, log2.
    log2: (u8, u8),

    /// The texels, tiled.
    words: Vec<u16>,
}

impl Texture {
    /// A texture from a function of the texel's column and row.
    ///
    /// # Arguments
    ///
    /// * `dword` - Where it goes.
    /// * `log2` - Width and height, log2.
    /// * `texel` - The RGB565 texel at a column and row.
    ///
    /// # Returns
    ///
    /// The texture.
    fn new(dword: u32, log2: (u8, u8), texel: impl Fn(usize, usize) -> u16) -> Texture {
        let (w, h) = (1_usize << log2.0, 1_usize << log2.1);
        let mut words = vec![0; w * h];
        for v in 0..h {
            for u in 0..w {
                words[texel_word(u, v, log2.0)] = texel(u, v);
            }
        }
        Texture { dword, log2, words }
    }

    /// `TEX0_CFG` for it, repeating, RGB565.
    ///
    /// # Arguments
    ///
    /// * `bilinear` - Filter bilinear, not nearest.
    ///
    /// # Returns
    ///
    /// The register value.
    fn cfg(&self, bilinear: bool) -> Tex0Cfg {
        Tex0Cfg {
            enable: true,
            format: TexFormat::Rgb565,
            width_log2: self.log2.0,
            height_log2: self.log2.1,
            base: u16::try_from(self.dword / 64).unwrap_or(0),
            u_wrap: Wrap::Repeat,
            v_wrap: Wrap::Repeat,
            bilinear,
        }
    }
}

/// The cube's texture, where `gs-host` puts it.
fn cube_tex() -> Texture {
    let size = 1 << CUBE_TEXTURE_LOG2;
    let texels = cube_texture();
    Texture::new(
        host_memory(0),
        (CUBE_TEXTURE_LOG2, CUBE_TEXTURE_LOG2),
        |u, v| texels[v * size + u],
    )
}

/// A 256 × 256 texture, four times the cache's 64 × 64-texel window each
/// way.
fn large_tex() -> Texture {
    Texture::new(host_memory(1), (8, 8), |u, v| {
        let c = |x: usize, bits: u32| u16::try_from((x & 0xFF) >> (8 - bits)).unwrap_or(0);
        c(u, 5) << 11 | c(v, 6) << 5 | c(u ^ v, 5)
    })
}

/// What a scene's figure is divided by.
#[derive(Clone, Copy)]
enum Per {
    /// Nothing: the frame time is the figure.
    Frame,
    /// Pixels drawn beyond the clear.
    Pixels(usize),
    /// Triangles drawn beyond the clear.
    Triangles(usize),
}

/// A scene to measure.
struct Scene {
    /// Its name in the table.
    name: &'static str,

    /// One frame's transactions after the clear, up to its present.
    body: Vec<Transaction>,

    /// Whether its frames clear colour as well as depth.
    clear_colour: bool,

    /// What its time beyond the clear is divided by.
    per: Per,

    /// Textures it reads.
    textures: Vec<Texture>,
}

/// A vertex: position (pixels), depth, texture coordinates in texture
/// sizes (Q = 1), and colour.
#[derive(Clone, Copy)]
struct V {
    /// X, pixels.
    x: f64,
    /// Y, pixels.
    y: f64,
    /// Depth.
    z: u16,
    /// S.
    s: f64,
    /// T.
    t: f64,
}

/// `ATTR0` and a vertex register write for a white vertex, Q = 1.
fn vertex(v: V, register: u8) -> [Transaction; 2] {
    #[allow(clippy::cast_possible_truncation)]
    let fixed = |f: f64, one: f64| (f * one).round() as i64;
    let attr = Attr0 {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
        s: i16::try_from(fixed(v.s, 4096.0)).unwrap_or(0),
        t: i16::try_from(fixed(v.t, 4096.0)).unwrap_or(0),
    };
    let pos = Position {
        x: i16::try_from(fixed(v.x, 16.0)).unwrap_or(0),
        y: i16::try_from(fixed(v.y, 16.0)).unwrap_or(0),
        z: v.z,
        q: Q_ONE,
    };
    [
        Transaction::write(reg::ATTR0, attr.bits()),
        Transaction::write(register, pos.bits()),
    ]
}

/// A strip through vertices, written as `NOKICK, NOKICK, KICK_012,
/// KICK_021, …`; `ATTR0` only before each vertex whose texture coordinates
/// differ from the last.
fn strip(vs: &[V]) -> Vec<Transaction> {
    let mut out = Vec::new();
    let mut last_attr = None;
    for (i, &v) in vs.iter().enumerate() {
        let register = match i {
            0 | 1 => reg::VERTEX_NOKICK,
            _ if i % 2 == 0 => reg::VERTEX_KICK_012,
            _ => reg::VERTEX_KICK_021,
        };
        let [attr, pos] = vertex(v, register);
        if last_attr != Some(attr) {
            out.push(attr);
            last_attr = Some(attr);
        }
        out.push(pos);
    }
    out
}

/// The render state every scene starts its frame with: the reset
/// `RENDER_MODE` (depth test and writes, no culling), and a `TEX0_CFG`.
fn state(tex: Tex0Cfg) -> Vec<Transaction> {
    vec![
        Transaction::write(reg::RENDER_MODE, RenderMode::default().bits()),
        Transaction::write(reg::TEX0_CFG, tex.bits()),
    ]
}

/// A screen-filling quad, `repeats` of the texture across and down.
fn full_screen(repeats: f64) -> Vec<Transaction> {
    #[allow(clippy::cast_precision_loss)]
    let (w, h) = (FB_WIDTH as f64, FB_HEIGHT as f64);
    let at = |x: f64, y: f64| V {
        x,
        y,
        z: 1000,
        s: x / w * repeats,
        t: y / h * repeats,
    };
    strip(&[at(0.0, 0.0), at(0.0, h), at(w, 0.0), at(w, h)])
}

/// Rows of strips of right triangles, `size` pixels on a side, each row's
/// top on a block boundary.
///
/// # Arguments
///
/// * `size` - The triangles' legs, pixels.
/// * `rows` - Strips.
/// * `per_row` - Triangles a strip.
/// * `repeats` - Repeats of the texture across a triangle.
///
/// # Returns
///
/// The transactions.
fn triangles(size: f64, rows: usize, per_row: usize, repeats: f64) -> Vec<Transaction> {
    let mut out = Vec::new();
    for r in 0..rows {
        #[allow(clippy::cast_precision_loss)]
        let top = 16.0 + r as f64 * (size + 16.0);
        let vs: Vec<V> = (0..per_row + 2)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let col = (i / 2) as f64;
                let bottom = i % 2 == 1;
                V {
                    x: 16.0 + col * size,
                    y: if bottom { top + size } else { top },
                    z: 1000,
                    // Untextured (no repeats), one ATTR0 serves them all.
                    s: col * repeats,
                    t: if bottom { repeats } else { 0.0 },
                }
            })
            .collect();
        out.extend(strip(&vs));
    }
    out
}

/// The scenes, in the table's order.
fn scenes() -> Vec<Scene> {
    let cube = cube_tex();
    let large = large_tex();
    let off = Tex0Cfg::default();
    let pixels = FB_WIDTH * FB_HEIGHT;
    let cube_frame = |tex: Option<Tex0Cfg>| -> Vec<Transaction> {
        // The cube's own frame, less its clear and present (added for
        // every scene alike).
        let all = match tex {
            Some(cfg) => textured_cube_at(CUBE_ANGLE, BACKGROUND, cfg),
            None => transactions(&cube_at(CUBE_ANGLE, BACKGROUND)),
        };
        all[1..all.len() - 1].to_vec()
    };
    vec![
        Scene {
            name: "clear only",
            body: Vec::new(),
            clear_colour: true,
            per: Per::Frame,
            textures: Vec::new(),
        },
        Scene {
            name: "depth clear only",
            body: Vec::new(),
            clear_colour: false,
            per: Per::Frame,
            textures: Vec::new(),
        },
        Scene {
            name: "full screen, untextured",
            body: [state(off), full_screen(4.0)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: Vec::new(),
        },
        Scene {
            name: "full screen, 64x64 nearest",
            body: [state(cube.cfg(false)), full_screen(4.0)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: vec![cube_tex()],
        },
        Scene {
            name: "full screen, 64x64 bilinear",
            body: [state(cube.cfg(true)), full_screen(4.0)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: vec![cube_tex()],
        },
        Scene {
            name: "full screen, 256x256 nearest, minified",
            body: [state(large.cfg(false)), full_screen(4.0)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: vec![large_tex()],
        },
        Scene {
            name: "full screen, 256x256 bilinear, minified",
            body: [state(large.cfg(true)), full_screen(4.0)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: vec![large_tex()],
        },
        Scene {
            name: "full screen, 256x256 bilinear, magnified",
            body: [state(large.cfg(true)), full_screen(0.5)].concat(),
            clear_colour: true,
            per: Per::Pixels(pixels),
            textures: vec![large_tex()],
        },
        Scene {
            name: "320 triangles of 8 px, untextured",
            body: [state(off), triangles(8.0, 10, 32, 0.0)].concat(),
            clear_colour: true,
            per: Per::Triangles(320),
            textures: Vec::new(),
        },
        Scene {
            name: "160 triangles of 8 px, nearest",
            body: [state(cube.cfg(false)), triangles(8.0, 10, 16, 0.125)].concat(),
            clear_colour: true,
            per: Per::Triangles(160),
            textures: vec![cube_tex()],
        },
        Scene {
            name: "120 triangles of 32 px, untextured",
            body: [state(off), triangles(32.0, 6, 20, 0.0)].concat(),
            clear_colour: true,
            per: Per::Triangles(120),
            textures: Vec::new(),
        },
        Scene {
            name: "cube, untextured",
            body: cube_frame(None),
            clear_colour: true,
            per: Per::Frame,
            textures: Vec::new(),
        },
        Scene {
            name: "cube, nearest",
            body: cube_frame(Some(cube.cfg(false))),
            clear_colour: true,
            per: Per::Frame,
            textures: vec![cube_tex()],
        },
        Scene {
            name: "cube, bilinear",
            body: cube_frame(Some(cube.cfg(true))),
            clear_colour: true,
            per: Per::Frame,
            textures: vec![cube_tex()],
        },
    ]
}

/// A `FRAME` write clearing depth and perhaps colour, presenting first or
/// not.
fn frame(present: bool, clear_colour: bool) -> Transaction {
    Transaction::write(
        reg::FRAME,
        FrameReg {
            present,
            clear_colour,
            clear_depth: true,
            colour: BACKGROUND,
        }
        .bits(),
    )
}

/// The rasteriser's front half's states (`hw/raster.snt`, `R_*`), by
/// value; 11 is unused.
const RASTER_STATES: [&str; 13] = [
    "idle", "copy", "init", "test", "pixels", "handoff", "next", "decide", "zread", "clear",
    "drain", "-", "flush",
];

/// What the stages after the front half were doing, counted in
/// [`Measured::stages`].
const STAGES: [&str; 4] = [
    "back: texels",
    "back: fetching",
    "back: waiting for the writer",
    "writer: busy",
];

/// One scene's result.
struct Measured {
    /// The draw times of the frames the GPU finished after the host
    /// connected, in order.
    cycles: Vec<u32>,

    /// For each of those frames, the cycles the parser waited for a
    /// transaction with the FIFO empty: the link not keeping up.
    starved: Vec<u32>,

    /// Cycles in each rasteriser state over the last frame.
    states: [u32; 16],

    /// Cycles of the last frame in each of [`STAGES`].
    stages: [u32; 4],

    /// The SDRAM over the last frame: cycles in a run of each owner
    /// (`hw/gpu.snt`'s `OWNER_*`), and at 0 the cycles in no run (idle,
    /// refresh, or opening and closing rows between runs).
    bus: [u32; 8],

    /// Whether the display ever ran short of pixels.
    underflow: bool,

    /// Transactions a frame.
    transactions: usize,
}

/// Runs a scene on the twin.
///
/// # Arguments
///
/// * `scene` - The scene.
///
/// # Returns
///
/// The draw times and transactions a frame.
///
/// # Errors
///
/// If the picture isn't the reference's, or the run doesn't finish.
fn measure(scene: &Scene) -> Result<Measured, Box<dyn Error + Send + Sync>> {
    let mut one = scene.body.clone();
    one.push(frame(true, scene.clear_colour));
    let mut txns = vec![frame(false, true)];
    for _ in 0..REPEATS {
        txns.extend(one.iter().copied());
    }

    let mut reference = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    let mut bench = Bench::new(io::sink(), "sim/tb/examples/perf.rs")?;
    for tex in &scene.textures {
        for (i, &w) in tex.words.iter().enumerate() {
            let word = usize::try_from(tex.dword)? * 4 + i;
            bench
                .bus
                .sdram
                .poke(word >> 22, (word >> 9) & 0x1FFF, word & 0x1FF, w);
        }
        for (k, four) in (0..).zip(tex.words.chunks(4)) {
            let mut d = [0; 4];
            d[..four.len()].copy_from_slice(four);
            reference.write_dword(tex.dword + k, pack_words(d));
        }
    }
    let expected = render(&mut reference, &txns)
        .0
        .pop()
        .ok_or("the scene presents nothing")?;

    let mut connected = false;
    let mut cycles = Vec::new();
    let mut starved = Vec::new();
    let mut starving = 0;
    let mut states = [0_u32; 16];
    let mut last_states = [0_u32; 16];
    let mut stages = [0_u32; 4];
    let mut last_stages = [0_u32; 4];
    let mut bus = [0_u32; 8];
    let mut last_bus = [0_u32; 8];
    let mut counting = 0;
    let mut last_done = 0;
    let mut last_picture = None;
    while bench.cycle < MAX_FRAMES * FRAME {
        let (_, shown) = bench.step()?;
        if connected {
            // A frame finishes when its draw-time counter restarts.
            let reg = |name: &str| bench.sim.register(name).unwrap_or(0);
            let count = reg("render_count");
            if count < counting {
                cycles.push(u32::try_from(reg("debug_render_cycles"))?);
                starved.push(starving);
                last_states = states;
                last_stages = stages;
                stages = [0; 4];
                last_bus = bus;
                bus = [0; 8];
                starving = 0;
                states = [0; 16];
                last_done = bench.cycle;
            } else if count > counting {
                // A cycle counted towards the frame.
                if reg("parser_state") == 0 && reg("link_count") == 0 {
                    starving += 1;
                }
                states[usize::try_from(reg("raster_state") & 0xF)?] += 1;
                let filling = reg("raster_filling") == 1;
                match reg("raster_b_state") {
                    1 if filling => stages[1] += 1,
                    1 => stages[0] += 1,
                    2 => stages[2] += 1,
                    _ => {}
                }
                if reg("raster_w_state") != 0 {
                    stages[3] += 1;
                }
                // The controller's Access and Close states: in a run.
                let in_run = matches!(reg("sdram_state"), 3 | 4);
                let owner = if in_run { reg("data_owner") & 7 } else { 0 };
                bus[usize::try_from(owner)?] += 1;
            }
            counting = count;
        }
        let Some(shown) = shown.filter(|f| f.presented >= 1) else {
            continue;
        };
        if !connected {
            connected = true;
            last_done = bench.cycle;
            bench.dtr_n = 0;
            bench.host.reconnect();
            bench.host.send(txns.iter().copied());
            continue;
        }
        last_picture = Some(screen(&shown.symbols)?);
        // Done when every frame has finished (the host's, and perhaps the
        // built-in scene's last) and two whole frames have been shown since
        // the last.
        if bench.host.done() && cycles.len() >= REPEATS && bench.cycle - last_done > 3 * FRAME {
            break;
        }
    }
    if cycles.len() < REPEATS {
        return Err(format!("{}: only {} frames finished", scene.name, cycles.len()).into());
    }
    let picture = last_picture.ok_or("no frame shown")?;
    // (Without a colour clear the buffers keep the built-in scene's frames;
    // such a scene draws nothing to check.)
    let wrong = differing(&picture, &expected);
    if scene.clear_colour && wrong != 0 {
        return Err(format!("{}: {wrong} pixels differ from the reference", scene.name).into());
    }
    Ok(Measured {
        cycles,
        starved,
        states: last_states,
        stages: last_stages,
        bus: last_bus,
        underflow: bench.underflow(),
        transactions: one.len(),
    })
}

/// The draw time a frame may take for 60 fps: one display frame, 1650 ×
/// 750 cycles.
const FRAME_60: u32 = 1_237_500;

/// The scene v1.1 must draw within [`FRAME_60`]
/// (`docs/design/draw-speed.md` §1).
const GOAL: &str = "full screen, 64x64 bilinear";

/// The recorded figures `--check` compares against, one `name<TAB>cycles`
/// a line.
const BASELINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/perf-baseline.txt");

/// How much slower than its recorded figure a scene may be before
/// `--check` fails, as a fraction: 1/200, 0.5%.
const SLOWER_ALLOWED: u32 = 200;

/// What to do with the recorded figures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Print the tables only.
    Report,
    /// Fail if a scene is slower than recorded.
    Check,
    /// Write the figures as the new record.
    Record,
}

/// Prints a table of what the back half and the writer did in each
/// scene's last frame.
fn print_stages(scenes: &[Scene], results: &[Measured]) {
    println!();
    println!("The back half and the writer over the last frame (% of its cycles):");
    println!();
    println!("| Scene | {} |", STAGES.join(" | "));
    println!("|---|{}", "---|".repeat(STAGES.len()));
    for (scene, m) in scenes.iter().zip(results) {
        let total: u32 = m.states.iter().sum();
        let cells: Vec<String> = m
            .stages
            .iter()
            .map(|&c| format!("{:.1}", f64::from(c) * 100.0 / f64::from(total.max(1))))
            .collect();
        println!("| {} | {} |", scene.name, cells.join(" | "));
    }
}

/// Prints a table of each scene's SDRAM use by owner.
fn print_bus(scenes: &[Scene], results: &[Measured]) {
    println!();
    println!("The SDRAM over the last frame (% of its cycles):");
    println!();
    println!("| Scene | scan-out | drawing | memory access | no run |");
    println!("|---|---|---|---|---|");
    for (scene, m) in scenes.iter().zip(results) {
        let total = f64::from(m.bus.iter().sum::<u32>().max(1));
        let pc = |k: usize| f64::from(m.bus[k]) * 100.0 / total;
        // OWNER_SCAN 2, OWNER_RASTER 3, OWNER_MEM 4; 0 no run.
        println!(
            "| {} | {:.1} | {:.1} | {:.1} | {:.1} |",
            scene.name,
            pc(2),
            pc(3),
            pc(4),
            pc(0)
        );
    }
}

/// Compares the figures with the record, or writes them as the record.
///
/// # Errors
///
/// In `Check` mode, if a scene is slower than recorded or has no record;
/// any error reading or writing the record.
fn baseline(mode: Mode, figures: &[(&str, u32)]) -> Result<(), Box<dyn Error + Send + Sync>> {
    if mode == Mode::Record {
        let text: String = figures
            .iter()
            .map(|(name, c)| format!("{name}\t{c}\n"))
            .collect();
        std::fs::write(BASELINE, text)?;
        println!();
        println!("recorded {} figures in {BASELINE}", figures.len());
        return Ok(());
    }
    if mode != Mode::Check {
        return Ok(());
    }
    let text = std::fs::read_to_string(BASELINE)?;
    let recorded: Vec<(&str, u32)> = text
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(n, c)| c.trim().parse().ok().map(|c| (n, c)))
        .collect();
    let mut failures = Vec::new();
    println!();
    for &(name, now) in figures {
        match recorded.iter().find(|(n, _)| *n == name) {
            None => failures.push(format!("{name}: no recorded figure")),
            Some(&(_, was)) if now > was + was / SLOWER_ALLOWED => {
                failures.push(format!("{name}: {now} cycles, recorded {was}"));
            }
            Some(&(_, was)) if now + was / SLOWER_ALLOWED < was => {
                println!("{name}: {now} cycles, faster than the recorded {was}: `just perf --record` keeps it");
            }
            Some(_) => {}
        }
    }
    if failures.is_empty() {
        println!("perf check: every scene within 0.5% of its recorded figure or faster");
        Ok(())
    } else {
        Err(format!("slower than recorded:\n  {}", failures.join("\n  ")).into())
    }
}

/// Measures every scene, in parallel, and prints the tables; with
/// `--check`, fails if a scene is slower than its recorded figure; with
/// `--record`, records the figures.
fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mode = match std::env::args().nth(1).as_deref() {
        None => Mode::Report,
        Some("--check") => Mode::Check,
        Some("--record") => Mode::Record,
        Some(a) => return Err(format!("unknown argument {a}: --check or --record").into()),
    };
    let scenes = scenes();
    let results: Vec<_> = thread::scope(|s| {
        let handles: Vec<_> = scenes
            .iter()
            .map(|scene| s.spawn(move || measure(scene)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().map_err(|_| "a scene panicked")?)
            .collect::<Result<Vec<_>, _>>()
    })?;

    #[allow(clippy::cast_precision_loss)]
    let ms = |c: u32| f64::from(c) * 1000.0 / CLOCK_HZ;
    let clear = results
        .first()
        .and_then(|m| m.cycles.last().copied())
        .ok_or("no clear figure")?;
    let mut figures = Vec::new();
    let mut problems = Vec::new();
    println!("| Scene | Transactions a frame | Cycles a frame | ms | fps | Beyond the clear |");
    println!("|---|---|---|---|---|---|");
    for (scene, m) in scenes.iter().zip(&results) {
        let n = m.cycles.len();
        let (last, starved) = (m.cycles[n - 1], m.starved[n - 1]);
        // Link-bound if the parser waited on the link for over 0.5% of it.
        let steady = starved * 200 <= last;
        if !steady {
            problems.push(format!("{}: link-bound", scene.name));
        }
        if m.underflow {
            problems.push(format!("{}: the display ran short of pixels", scene.name));
        }
        figures.push((scene.name, last));
        let extra = last.saturating_sub(clear);
        #[allow(clippy::cast_precision_loss)]
        let per = match scene.per {
            Per::Frame => format!("{extra} cycles"),
            Per::Pixels(p) => format!("{:.2} cycles a pixel", f64::from(extra) / p as f64),
            Per::Triangles(t) => format!("{:.0} cycles a triangle", f64::from(extra) / t as f64),
        };
        println!(
            "| {}{} | {} | {} | {:.2} | {:.1} | {}{} |",
            scene.name,
            if scene.name == GOAL {
                " (v1.1 goal)"
            } else {
                ""
            },
            m.transactions,
            last,
            ms(last),
            1000.0 / ms(last),
            per,
            if steady {
                String::new()
            } else {
                format!(" (link-bound: {starved} cycles waiting for it)")
            }
        );
    }

    println!();
    println!("Where the last frame's cycles went, by rasteriser state (%):");
    println!();
    let used: Vec<usize> = (0..RASTER_STATES.len())
        .filter(|&k| results.iter().any(|m| m.states[k] > 0))
        .collect();
    let names: Vec<&str> = used.iter().map(|&k| RASTER_STATES[k]).collect();
    println!("| Scene | {} |", names.join(" | "));
    println!("|---|{}", "---|".repeat(used.len()));
    for (scene, m) in scenes.iter().zip(&results) {
        let total: u32 = m.states.iter().sum();
        let cells: Vec<String> = used
            .iter()
            .map(|&k| {
                format!(
                    "{:.1}",
                    f64::from(m.states[k]) * 100.0 / f64::from(total.max(1))
                )
            })
            .collect();
        println!("| {} | {} |", scene.name, cells.join(" | "));
    }
    print_stages(&scenes, &results);
    print_bus(&scenes, &results);

    let goal = figures
        .iter()
        .find(|(n, _)| *n == GOAL)
        .map(|&(_, c)| c)
        .ok_or("no goal scene")?;
    println!();
    println!(
        "v1.1 goal ({GOAL}, within {FRAME_60} cycles for 60 fps): {goal} cycles, {}",
        if goal <= FRAME_60 {
            "met"
        } else {
            "not yet met"
        }
    );
    if !problems.is_empty() {
        return Err(problems.join("; ").into());
    }
    baseline(mode, &figures)
}
