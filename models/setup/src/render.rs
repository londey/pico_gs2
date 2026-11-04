//! The reference renderer: what the GPU draws for a scene, pixel for pixel.
//! Setup is the bit-exact model; coverage is the reference traversal
//! (top-left rule); a pixel's colour and depth are each plane's start plus
//! its gradients times the pixel's offset, modulo `2^36` as the
//! rasteriser's stepping adders give it, then rounded and clamped (RGB565,
//! and a 16-bit depth). A pixel is drawn if its depth is less than the
//! depth buffer's, which then takes it.

use crate::formats::{wrap, REG_BITS};
use crate::scene::{decode, Command, Primitive};
use crate::setup::setup;
use crate::traverse::covered;
use crate::triangle::{Attr, AttrSet, Triangle};
use crate::Plane;

/// Framebuffer width, in pixels.
pub const FB_WIDTH: usize = 640;

/// Framebuffer height, in pixels.
pub const FB_HEIGHT: usize = 360;

/// The farthest depth: what a clear sets the depth buffer to.
pub const FAR: u16 = 0xFFFF;

/// A frame being drawn: colour and depth, each `FB_WIDTH * FB_HEIGHT`
/// words in raster order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// RGB565 colours.
    pub colour: Vec<u16>,

    /// Depths, 0 nearest.
    pub depth: Vec<u16>,
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
        Frame { colour, depth }
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

/// An 8-bit channel's iterator value as a `bits`-wide RGB565 field: the
/// value over `2^(24 - bits)` (8 integer bits down to `bits`, and the 16
/// fraction bits), rounded half up, clamped.
///
/// # Arguments
///
/// * `v` - The iterator count.
/// * `bits` - 5 or 6.
///
/// # Returns
///
/// The field, `0` to `2^bits - 1`.
pub fn quantise(v: i64, bits: u32) -> u16 {
    let shift = 24 - bits;
    let q = (v + (1 << (shift - 1))) >> shift;
    u16::try_from(q.clamp(0, (1 << bits) - 1)).expect("in range")
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

/// Draws one triangle, depth tested.
///
/// # Arguments
///
/// * `frame` - The frame.
/// * `tri` - The vertices, in role order.
/// * `prim` - The primitive it belongs to (cull mode and program).
///
/// # Returns
///
/// Whether setup kept the triangle.
pub fn draw(frame: &mut Frame, tri: &Triangle, prim: &Primitive) -> bool {
    let attrs = if prim.untextured {
        AttrSet::UNTEXTURED
    } else {
        AttrSet::FULL
    };
    let Ok(s) = setup(tri, prim.cull, attrs) else {
        return false;
    };
    let plane = |a: Attr| s.planes[a.index()].expect("colour and depth are always computed");
    let (r, g, b, z) = (
        plane(Attr::R),
        plane(Attr::G),
        plane(Attr::B),
        plane(Attr::Z),
    );
    let px0 = s.xs.to_count() >> 4;
    let py0 = s.ys.to_count() >> 4;
    for (px, py) in covered(&s) {
        let (i, j) = (px - px0, py - py0);
        let index = usize::try_from(py).expect("on screen") * FB_WIDTH
            + usize::try_from(px).expect("on screen");
        let d = depth(value(&z, i, j));
        if d < frame.depth[index] {
            frame.depth[index] = d;
            frame.colour[index] = quantise(value(&r, i, j), 5) << 11
                | quantise(value(&g, i, j), 6) << 5
                | quantise(value(&b, i, j), 5);
        }
    }
    true
}

/// Runs a scene's commands on a frame.
///
/// # Arguments
///
/// * `frame` - The frame, holding what the scene starts from.
/// * `cmds` - The commands.
///
/// # Returns
///
/// The colours at each end of frame, and how many triangles setup kept.
pub fn run(frame: &mut Frame, cmds: &[Command]) -> (Vec<Vec<u16>>, usize) {
    let mut frames = Vec::new();
    let mut kept = 0;
    for c in cmds {
        match c {
            Command::Clear(rgb) => {
                frame.colour.fill(*rgb);
                frame.depth.fill(FAR);
            }
            Command::EndFrame => frames.push(frame.colour.clone()),
            Command::Draw(prim) => {
                for tri in prim.triangles() {
                    kept += usize::from(draw(frame, &tri, prim));
                }
            }
        }
    }
    (frames, kept)
}

/// Renders a scene's command words over a frame.
///
/// # Arguments
///
/// * `frame` - The frame.
/// * `words` - The scene's command words.
///
/// # Returns
///
/// The colours at each end of frame, and how many triangles setup kept.
pub fn render(frame: &mut Frame, words: &[u32]) -> (Vec<Vec<u16>>, usize) {
    run(frame, &decode(words))
}
