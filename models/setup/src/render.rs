//! The reference renderer: what the GPU draws for a scene, pixel for pixel.
//! Setup is the bit-exact model; coverage is the reference traversal
//! (top-left rule); a pixel's colour and depth are each plane's start plus
//! its gradients times the pixel's offset, modulo `2^36` as the
//! rasteriser's stepping adders give it, then rounded and clamped (8 bits a
//! colour channel, and a 16-bit depth). The colour combiner takes that
//! shade and, on a textured triangle, the texture's sample (`texture.rs`:
//! nearest or bilinear, from GPU memory as `MEM_DATA` wrote it); an
//! untextured one's sample is white. The colour is stored as RGB565 by
//! keeping each channel's top bits. With the render mode's depth test on, a
//! pixel is drawn only if its depth is less than the depth buffer's; a drawn
//! pixel writes its colour and depth where the mode's write enables say.

use std::collections::HashMap;

use gs_proto::{unpack_words, Cull, Tex0Cfg, Transaction};

use crate::formats::{wrap, REG_BITS};
use crate::scene::{interpret, DrawState, Event};
use crate::setup::setup;
use crate::texture::{
    combine, coordinate, decode, filter, reciprocal, rgb565, shade8, taps, texel_addr, CcInputs,
};
use crate::traverse::covered;
use crate::triangle::{Attr, AttrSet, CullMode, Triangle};
use crate::Plane;

/// Framebuffer width, in pixels.
pub const FB_WIDTH: usize = 640;

/// Framebuffer height, in pixels.
pub const FB_HEIGHT: usize = 360;

/// The farthest depth: what a clear sets the depth buffer to.
pub const FAR: u16 = 0xFFFF;

/// A frame being drawn: colour and depth, each `FB_WIDTH * FB_HEIGHT`
/// words in raster order; and the GPU memory the host has written, for
/// textures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// RGB565 colours.
    pub colour: Vec<u16>,

    /// Depths, 0 nearest.
    pub depth: Vec<u16>,

    /// GPU memory written with `MEM_DATA`, by word address; unwritten
    /// words read 0.
    pub memory: HashMap<u32, u16>,
}

impl Frame {
    /// A frame over the given colours, its depth all [`FAR`].
    ///
    /// # Arguments
    ///
    /// * `colour` - The colours, `FB_WIDTH * FB_HEIGHT` in raster order.
    ///
    /// # Returns
    ///
    /// The frame.
    pub fn new(colour: Vec<u16>) -> Frame {
        let depth = vec![FAR; colour.len()];
        Frame {
            colour,
            depth,
            memory: HashMap::new(),
        }
    }

    /// A word of GPU memory.
    ///
    /// # Arguments
    ///
    /// * `word` - Its address.
    ///
    /// # Returns
    ///
    /// The word, 0 if never written.
    pub fn word(&self, word: u32) -> u16 {
        self.memory.get(&(word & 0xFF_FFFF)).copied().unwrap_or(0)
    }

    /// Writes 64 bits of GPU memory, as `MEM_DATA` does.
    ///
    /// # Arguments
    ///
    /// * `dword` - The dword address.
    /// * `data` - The four words, the first in bits 15:0.
    pub fn write_dword(&mut self, dword: u32, data: u64) {
        for (k, w) in (0..).zip(unpack_words(data)) {
            self.memory.insert((dword << 2 | k) & 0xFF_FFFF, w);
        }
    }
}

/// A plane's value at a pixel `(i, j)` pixels right of and below its start
/// point, as an iterator count (16 fraction bits), wrapped to 36 bits.
fn value(p: &Plane, i: i64, j: i64) -> i64 {
    wrap(
        p.start.to_count() + p.dx.to_count() * i + p.dy.to_count() * j,
        REG_BITS,
    )
}

/// The depth iterator's value as a 16-bit depth: rounded half up, clamped.
///
/// # Arguments
///
/// * `v` - The iterator count.
///
/// # Returns
///
/// The depth.
pub fn depth(v: i64) -> u16 {
    u16::try_from(((v + (1 << 15)) >> 16).clamp(0, 0xFFFF)).expect("in range")
}

/// Draws one triangle.
///
/// # Arguments
///
/// * `frame` - The frame.
/// * `tri` - The vertices, in role order.
/// * `state` - The render state: render mode (cull mode, depth test, write
///   enables, alpha test), texture (which also picks the setup program),
///   combiner and its constants.
///
/// # Returns
///
/// Whether setup kept the triangle.
pub fn draw(frame: &mut Frame, tri: &Triangle, state: &DrawState) -> bool {
    let (mode, tex) = (state.mode, &state.tex);
    let attrs = if tex.enable {
        AttrSet::FULL
    } else {
        AttrSet::UNTEXTURED
    };
    let cull = match mode.cull {
        Cull::None => CullMode::None,
        Cull::Anticlockwise => CullMode::Back,
        Cull::Clockwise => CullMode::Front,
    };
    let Ok(s) = setup(tri, cull, attrs) else {
        return false;
    };
    let plane = |a: Attr| s.planes[a.index()].expect("colour and depth are always computed");
    let shade_planes = [Attr::R, Attr::G, Attr::B, Attr::A].map(plane);
    let z = plane(Attr::Z);
    let px0 = s.xs.to_count() >> 4;
    let py0 = s.ys.to_count() >> 4;
    for (px, py) in covered(&s) {
        let (i, j) = (px - px0, py - py0);
        let index = usize::try_from(py).expect("on screen") * FB_WIDTH
            + usize::try_from(px).expect("on screen");
        let d = depth(value(&z, i, j));
        if mode.z_test && d >= frame.depth[index] {
            continue;
        }
        let inputs = CcInputs {
            tex: if tex.enable {
                sample(frame, &s, tex, i, j)
            } else {
                [255; 4]
            },
            shade: shade_planes.map(|p| shade8(value(&p, i, j))),
            const0: state.const0,
            const1: state.const1,
        };
        let out = combine(&state.cc, &inputs);
        if !mode.alpha_test.passes(out[3], mode.alpha_ref) {
            continue;
        }
        if mode.z_write {
            frame.depth[index] = d;
        }
        if mode.colour_write {
            frame.colour[index] = rgb565([out[0], out[1], out[2]]);
        }
    }
    true
}

/// The colour a textured triangle's pixel samples: its texel, nearest,
/// or the bilinear blend of four (host-interface.md §6.3).
///
/// # Arguments
///
/// * `frame` - The frame, for the texture's memory.
/// * `s` - The triangle's setup, with its S′, T′ and Q planes.
/// * `tex` - The texture.
/// * `i`, `j` - The pixel's offset from the planes' start point.
///
/// # Returns
///
/// The sample's R, G, B and A.
fn sample(frame: &Frame, s: &crate::Setup, tex: &Tex0Cfg, i: i64, j: i64) -> [u8; 4] {
    let plane = |a: Attr| s.planes[a.index()].expect("the full program computes S, T and Q");
    let recip = reciprocal(value(&plane(Attr::Q), i, j));
    let u = coordinate(value(&plane(Attr::S), i, j), recip, tex.width_log2);
    let v = coordinate(value(&plane(Attr::T), i, j), recip, tex.height_log2);
    let tu = taps(u, tex.width_log2, tex.u_wrap, tex.bilinear);
    let tv = taps(v, tex.height_log2, tex.v_wrap, tex.bilinear);
    let texel = |u: u32, v: u32| decode(frame.word(texel_addr(u, v, tex).word), tex.format);
    filter(
        [
            [texel(tu.first, tv.first), texel(tu.second, tv.first)],
            [texel(tu.first, tv.second), texel(tu.second, tv.second)],
        ],
        tu.weight,
        tv.weight,
    )
}

/// Runs what the command parser makes of a scene on a frame.
///
/// # Arguments
///
/// * `frame` - The frame, holding what the scene starts from.
/// * `events` - The triangles and frame operations.
///
/// # Returns
///
/// The colours at each present, and how many triangles setup kept.
pub fn run(frame: &mut Frame, events: &[Event]) -> (Vec<Vec<u16>>, usize) {
    let mut frames = Vec::new();
    let mut kept = 0;
    for e in events {
        match e {
            Event::Frame(f) => {
                // A present first, then the clears, of the next buffer.
                if f.present {
                    frames.push(frame.colour.clone());
                }
                if f.clear_colour {
                    frame.colour.fill(f.colour);
                }
                if f.clear_depth {
                    frame.depth.fill(FAR);
                }
            }
            Event::Draw(tri, state) => kept += usize::from(draw(frame, tri, state)),
            Event::Mem(dword, data) => frame.write_dword(*dword, *data),
        }
    }
    (frames, kept)
}

/// Renders a scene's transactions over a frame.
///
/// # Arguments
///
/// * `frame` - The frame.
/// * `txns` - The transactions.
///
/// # Returns
///
/// The colours at each present, and how many triangles setup kept.
pub fn render(frame: &mut Frame, txns: &[Transaction]) -> (Vec<Vec<u16>>, usize) {
    run(frame, &interpret(txns))
}
