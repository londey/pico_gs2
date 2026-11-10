//! Twin test of texturing (`docs/design/host-interface.md` §6): the whole
//! core with the SDRAM model on its bus and a host on its USB UART.
//!
//! Once the built-in scene's first frame is up, a host connects, uploads
//! textures with `MEM_DATA` and draws four frames of its own:
//!
//! 1. A floor in steep perspective (Q from 1 down to 1/8, four repeats of a
//!    32 × 32 RGB565 texture), sampled nearest; a triangle clamping S and
//!    mirroring T, affine, filtered bilinear (so its samples at the clamped
//!    and mirrored edges take one texel twice); a 16 × 16 ARGB4444 texture,
//!    bilinear, mirrored in S and repeating in T, on a triangle whose
//!    vertex colours it is multiplied by; and an untextured triangle
//!    crossing the floor in depth.
//! 2. The floor again, after the 32 × 32 texture is uploaded again with
//!    different texels: the cache must not keep the old ones.
//! 3. A 128 × 32 texture, wider than the cache's 64 × 64-texel window,
//!    across most of the screen in perspective, bilinear (its samples'
//!    four texels are often in two or four blocks, missed together).
//! 4. The textured cube `gs-host` streams (`scene::textured_cube_at`),
//!    bilinear.
//!
//! Every frame shown is whole and pixel-exact against the reference
//! renderer on the same transactions: the ROM's, then the host's in order.
//!
//! The two large textures (frames 3 and 4) are written straight into the
//! SDRAM model, and into the reference renderer's memory, before the run:
//! over the simulated 3 Mbaud link they would take 4.6 million cycles,
//! which the recorded stimulus's co-simulation would replay for nothing.
//! The small ones go over the link, which frame 2 needs.
//!
//! Each host frame is saved as `build/texture-frame<k>.png`. The core's
//! input on every cycle is recorded as
//! `build/stimulus/gpu.texture.csv`, which `just check` replays with
//! `--cosim`.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

use gs_proto::{
    host_memory, mem_write, pack_words, reg, texel_word, Attr0, Frame as FrameReg, Position,
    Status, Tex0Cfg, TexFormat, Transaction, Wrap,
};
use setup_model::render::{render, Frame};
use setup_model::scene::{
    cube_scene, cube_texture, textured_cube_at, transactions, CUBE_TEXTURE_LOG2,
};
use tb::{differing, screen, Bench, FB_HEIGHT, FB_WIDTH, FRAME, H_ACTIVE, V_ACTIVE};

/// Frames to run at most.
const MAX_FRAMES: usize = 32;

/// The host's background colour, RGB565: dark blue.
const HOST_BACKGROUND: u16 = 0x0010;

/// A path under the repository's `build/` directory.
fn build_path(parts: &[&str]) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "build"]
        .iter()
        .chain(parts)
        .collect()
}

/// A texture: its configuration (enabled) and its texels, tiled.
struct Texture {
    /// `TEX0_CFG`.
    cfg: Tex0Cfg,

    /// Its dword address.
    dword: u32,

    /// The texels, tiled, as uploaded.
    words: Vec<u16>,
}

impl Texture {
    /// A texture from a function of the texel's column and row.
    fn new(
        dword: u32,
        log2: (u8, u8),
        format: TexFormat,
        texel: impl Fn(usize, usize) -> u16,
    ) -> Texture {
        let (w, h) = (1_usize << log2.0, 1_usize << log2.1);
        let mut words = vec![0; w * h];
        for v in 0..h {
            for u in 0..w {
                words[texel_word(u, v, log2.0)] = texel(u, v);
            }
        }
        Texture {
            cfg: Tex0Cfg {
                enable: true,
                format,
                width_log2: log2.0,
                height_log2: log2.1,
                base: u16::try_from(dword / 64).unwrap_or(0),
                ..Tex0Cfg::default()
            },
            dword,
            words,
        }
    }

    /// The transactions that upload it.
    fn upload(&self) -> Vec<Transaction> {
        mem_write(self.dword, &self.words).collect()
    }

    /// Writes it straight into the SDRAM model.
    fn preload<W: std::io::Write>(&self, bench: &mut Bench<W>) {
        for (i, &w) in self.words.iter().enumerate() {
            let word = usize::try_from(self.dword).unwrap_or(0) * 4 + i;
            bench
                .bus
                .sdram
                .poke(word >> 22, (word >> 9) & 0x1FFF, word & 0x1FF, w);
        }
    }

    /// Writes it straight into the reference renderer's memory.
    fn preload_reference(&self, frame: &mut Frame) {
        for (k, four) in (0..).zip(self.words.chunks(4)) {
            let mut d = [0; 4];
            d[..four.len()].copy_from_slice(four);
            frame.write_dword(self.dword + k, pack_words(d));
        }
    }

    /// `TEX0_CFG` with wrap modes and a filter.
    fn select(&self, u_wrap: Wrap, v_wrap: Wrap, bilinear: bool) -> Transaction {
        Transaction::write(
            reg::TEX0_CFG,
            Tex0Cfg {
                u_wrap,
                v_wrap,
                bilinear,
                ..self.cfg
            }
            .bits(),
        )
    }
}

/// An RGB565 colour from 8-bit channels (top bits).
fn rgb(r: usize, g: usize, b: usize) -> u16 {
    let c = |v: usize, bits: u32| u16::try_from((v & 0xFF) >> (8 - bits)).unwrap_or(0);
    c(r, 5) << 11 | c(g, 6) << 5 | c(b, 5)
}

/// A textured vertex: pixel position, depth, Q, texture coordinates (in
/// texture sizes) and colour.
#[derive(Clone, Copy)]
struct V {
    /// Position, pixels.
    x: f64,
    /// Position, pixels.
    y: f64,
    /// Depth.
    z: u16,
    /// Q, 0 to 2.
    q: f64,
    /// S.
    s: f64,
    /// T.
    t: f64,
    /// Colour.
    rgb: [u8; 3],
}

/// `ATTR0` and a vertex register write for a vertex.
fn vertex(v: V, register: u8) -> [Transaction; 2] {
    #[allow(clippy::cast_possible_truncation)]
    let fixed = |f: f64, one: f64| (f * one).round() as i64;
    let attr = Attr0 {
        r: v.rgb[0],
        g: v.rgb[1],
        b: v.rgb[2],
        a: 255,
        s: i16::try_from(fixed(v.s * v.q, 4096.0)).unwrap_or(0),
        t: i16::try_from(fixed(v.t * v.q, 4096.0)).unwrap_or(0),
    };
    let pos = Position {
        x: i16::try_from(fixed(v.x, 16.0)).unwrap_or(0),
        y: i16::try_from(fixed(v.y, 16.0)).unwrap_or(0),
        z: v.z,
        q: u16::try_from(fixed(v.q, 32768.0)).unwrap_or(0),
    };
    [
        Transaction::write(reg::ATTR0, attr.bits()),
        Transaction::write(register, pos.bits()),
    ]
}

/// A triangle (separate, or the start of a strip): three vertices, the last
/// a kick.
fn triangle(v: [V; 3]) -> Vec<Transaction> {
    [
        vertex(v[0], reg::VERTEX_NOKICK),
        vertex(v[1], reg::VERTEX_NOKICK),
        vertex(v[2], reg::VERTEX_KICK_012),
    ]
    .concat()
}

/// A floor in perspective: a strip of two triangles from a near edge across
/// the bottom of the screen (Q = 1) to a far edge (Q = 1/8), `repeats` of
/// the texture across and along it.
fn floor(repeats: f64, z: u16) -> Vec<Transaction> {
    let white = [255, 255, 255];
    let near = |x: f64, s: f64| V {
        x,
        y: 350.0,
        z,
        q: 1.0,
        s,
        t: repeats,
        rgb: white,
    };
    let far = |x: f64, s: f64| V {
        x,
        y: 120.0,
        z,
        q: 0.125,
        s,
        t: 0.0,
        rgb: white,
    };
    [
        vertex(near(10.0, 0.0), reg::VERTEX_NOKICK),
        vertex(far(280.0, 0.0), reg::VERTEX_NOKICK),
        vertex(near(630.0, repeats), reg::VERTEX_KICK_012),
        vertex(far(360.0, repeats), reg::VERTEX_KICK_021),
    ]
    .concat()
}

/// A `FRAME` write.
fn frame(present: bool) -> Transaction {
    Transaction::write(
        reg::FRAME,
        FrameReg {
            present,
            clear_colour: true,
            clear_depth: true,
            colour: HOST_BACKGROUND,
        }
        .bits(),
    )
}

/// The host's transactions: uploads and four frames; and the textures to
/// preload.
fn host_transactions() -> (Vec<Transaction>, Vec<Texture>) {
    let checker = |u: usize, v: usize| {
        if (u / 4 + v / 4).is_multiple_of(2) {
            rgb(u * 8, 255 - v * 8, 64)
        } else {
            rgb(255, u * 8, v * 8)
        }
    };
    let a = Texture::new(host_memory(0), (5, 5), TexFormat::Rgb565, checker);
    let a2 = Texture::new(host_memory(0), (5, 5), TexFormat::Rgb565, |u, v| {
        if u < 16 {
            checker(u, v) ^ 0xFFFF
        } else {
            checker(u, v)
        }
    });
    let b = Texture::new(
        host_memory(1) + 0x40,
        (4, 4),
        TexFormat::Argb4444,
        |u, v| {
            let n = |x: usize| u16::try_from(x & 0xF).unwrap_or(0);
            0xF000 | n(u) << 8 | n(v) << 4 | n(u ^ v)
        },
    );
    let c = Texture::new(host_memory(2), (7, 5), TexFormat::Rgb565, |u, v| {
        rgb(
            u * 2,
            v * 4,
            if (u / 8 + v / 8).is_multiple_of(2) {
                255
            } else {
                0
            },
        )
    });

    let mut txns = Vec::new();
    txns.extend(a.upload());
    txns.extend(b.upload());
    txns.push(frame(false));

    // 1. The floor; a clamped and mirrored triangle, filtered; a modulated
    // ARGB4444 triangle, filtered; an untextured triangle through the
    // floor.
    txns.push(a.select(Wrap::Repeat, Wrap::Repeat, false));
    txns.extend(floor(4.0, 3000));
    txns.push(a.select(Wrap::ClampToEdge, Wrap::Mirror, true));
    let affine = |x: f64, y: f64, s: f64, t: f64| V {
        x,
        y,
        z: 1000,
        q: 1.0,
        s,
        t,
        rgb: [255; 3],
    };
    txns.extend(triangle([
        affine(400.0, 10.0, -0.5, -1.0),
        affine(630.0, 10.0, 1.5, -1.0),
        affine(630.0, 110.0, 1.5, 1.75),
    ]));
    txns.push(b.select(Wrap::Mirror, Wrap::Repeat, true));
    let tinted = |x: f64, y: f64, s: f64, t: f64, rgb: [u8; 3]| V {
        x,
        y,
        z: 1000,
        q: 1.0,
        s,
        t,
        rgb,
    };
    txns.extend(triangle([
        tinted(10.0, 10.0, 0.0, 0.0, [255, 255, 255]),
        tinted(250.0, 20.0, 2.0, 0.0, [255, 64, 0]),
        tinted(30.0, 110.0, 0.0, 1.5, [0, 128, 255]),
    ]));
    txns.push(Transaction::write(reg::TEX0_CFG, Tex0Cfg::default().bits()));
    txns.extend(triangle([
        affine(200.0, 300.0, 0.0, 0.0),
        affine(440.0, 200.0, 0.0, 0.0),
        V {
            z: 6000,
            ..affine(320.0, 359.0, 0.0, 0.0)
        },
    ]));
    txns.push(frame(true));

    // 2. The floor after the texture is uploaded again.
    txns.extend(a2.upload());
    txns.push(a.select(Wrap::Repeat, Wrap::Repeat, false));
    txns.extend(floor(4.0, 3000));
    txns.push(frame(true));

    // 3. A texture wider than the cache's window, across the screen,
    // filtered.
    let mut preloaded = Vec::new();
    txns.push(c.select(Wrap::Repeat, Wrap::Repeat, true));
    let big = |x: f64, y: f64, q: f64, s: f64, t: f64| V {
        x,
        y,
        z: 100,
        q,
        s,
        t,
        rgb: [255; 3],
    };
    txns.extend(
        [
            vertex(big(20.0, 20.0, 1.0, 0.0, 0.0), reg::VERTEX_NOKICK),
            vertex(big(620.0, 20.0, 0.5, 1.0, 0.0), reg::VERTEX_NOKICK),
            vertex(big(20.0, 340.0, 1.0, 0.0, 1.0), reg::VERTEX_KICK_012),
            vertex(big(620.0, 340.0, 0.5, 1.0, 1.0), reg::VERTEX_KICK_021),
        ]
        .concat(),
    );
    txns.push(frame(true));

    // 4. The textured cube.
    let texels = cube_texture();
    let size = 1 << CUBE_TEXTURE_LOG2;
    let cube = Texture::new(
        host_memory(3),
        (CUBE_TEXTURE_LOG2, CUBE_TEXTURE_LOG2),
        TexFormat::Rgb565,
        |u, v| texels[v * size + u],
    );
    preloaded.push(c);
    preloaded.push(cube);
    txns.extend(textured_cube_at(
        0.3,
        HOST_BACKGROUND,
        Tex0Cfg {
            bilinear: true,
            ..preloaded[1].cfg
        },
    ));
    (txns, preloaded)
}

/// Writes a decoded active picture as an RGB PNG in `build/`.
fn write_png(picture: &[[u8; 3]], name: &str) -> Result<(), Box<dyn Error>> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(build_path(&[name]))?),
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

/// The frames a scene presents, as the reference renderer draws them.
fn frames_of(txns: &[Transaction], preloaded: &[Texture]) -> Vec<Vec<u16>> {
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    for t in preloaded {
        t.preload_reference(&mut frame);
    }
    render(&mut frame, txns).0
}

/// The ROM scene, then the host's textured frames in order, pixel-exact.
#[test]
fn textured_frames() -> Result<(), Box<dyn Error>> {
    let path = build_path(&["stimulus", "gpu.texture.csv"]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut bench = Bench::new(
        BufWriter::new(File::create(path)?),
        "sim/tb/tests/texture.rs",
    )?;
    let rom = frames_of(&transactions(&cube_scene()), &[]);
    let (host_txns, preloaded) = host_transactions();
    let host = frames_of(&host_txns, &preloaded);
    for t in &preloaded {
        t.preload(&mut bench);
    }
    assert_eq!(host.len(), 4);

    // The cube frame's texture cache use: drawn pixels looked up (a
    // pixel looked up again after a miss counts again), misses, and blocks
    // through the texel pass.
    let (mut lookups, mut misses, mut blocks) = (0_u64, 0_u64, 0_u64);
    let (mut was_filling, mut was_texel) = (false, false);
    let mut host_presents = 0;
    let mut connected = false;
    // The last host frame shown, if any.
    let mut last: Option<usize> = None;
    let mut out = None;
    while bench.cycle < MAX_FRAMES * FRAME {
        let (o, shown) = bench.step()?;
        out = Some(o);
        let reg = |name: &str| bench.sim.register(name).unwrap_or(0);
        if reg("parser_from_host") == 1 && reg("parser_frame_done") == 1 {
            host_presents += 1;
        }
        // The cube is drawn after the host's third present (the fourth
        // counted: the ROM's last, at which the host takes over, is one).
        if host_presents == 4 {
            let texel = reg("raster_b_state") == 1;
            let filling = reg("raster_filling") == 1;
            lookups += u64::from(
                texel && !filling && reg("raster_tx3_valid") == 1 && reg("raster_tx3_drawn") == 1,
            );
            misses += u64::from(filling && !was_filling);
            blocks += u64::from(texel && !was_texel);
            (was_filling, was_texel) = (filling, texel);
        }
        let Some(shown) = shown.filter(|f| f.presented >= 1) else {
            continue;
        };
        let picture = screen(&shown.symbols)?;
        let is = |fb: &Vec<u16>| differing(&picture, fb) == 0;
        if !connected {
            if !rom.iter().any(is) {
                return Err("the first frame isn't one of the ROM's".into());
            }
            connected = true;
            bench.dtr_n = 0;
            bench.host.reconnect();
            bench.host.send(host_txns.iter().copied());
            bench.host.send([Transaction::Read { addr: reg::STATUS }]);
            continue;
        }
        let allowed: Vec<usize> = match last {
            None => vec![0],
            Some(k) => vec![k, k + 1],
        };
        let now = allowed.into_iter().find(|&k| host.get(k).is_some_and(is));
        match (now, last) {
            (Some(k), _) => {
                println!("cycle {}: host frame {k}", bench.cycle);
                if last != Some(k) {
                    write_png(&picture, &format!("texture-frame{k}.png"))?;
                }
                last = Some(k);
                if k == host.len() - 1 {
                    break;
                }
            }
            (None, None) if rom.iter().any(is) => {}
            (None, _) => {
                let diffs: Vec<_> = host.iter().map(|fb| differing(&picture, fb)).collect();
                return Err(format!(
                    "after host frame {last:?}, a frame that isn't the next; pixels differing from each host frame: {diffs:?}"
                )
                .into());
            }
        }
    }
    bench.finish()?;
    let out = out.ok_or("no cycles run")?;
    assert_eq!(
        last,
        Some(host.len() - 1),
        "the host's frames were never all shown"
    );
    #[allow(clippy::cast_precision_loss)]
    let hit_rate = 100.0 * (lookups - misses) as f64 / lookups.max(1) as f64;
    println!(
        "cube frame: {blocks} textured blocks, {lookups} lookups, {misses} misses: {hit_rate:.1}% hits"
    );
    let status = Status::from_bits(*bench.host.replies.first().ok_or("no STATUS reply")?);
    println!("{status:?}");
    assert!(
        !status.link_malformed && !status.link_overflow,
        "{status:?}"
    );
    assert_eq!(out.debug_link_errors_q, 0, "host link errors");
    assert_eq!(bench.host.unknown_bytes, 0);
    assert!(!bench.underflow(), "the display ran short of pixels");
    assert!(
        bench.bus.sdram.violations().is_empty(),
        "{} SDRAM violations",
        bench.bus.sdram.violation_count()
    );
    Ok(())
}
