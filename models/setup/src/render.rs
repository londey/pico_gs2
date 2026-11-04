//! The reference renderer: what the GPU draws into its framebuffer for a
//! scene, pixel for pixel. Setup is the bit-exact model; coverage is the
//! reference traversal (top-left rule); a pixel's colour is each plane's
//! start plus its gradients times the pixel's offset, modulo `2^36` as the
//! rasteriser's stepping adders give it, then rounded to RGB565 and
//! clamped. Triangles are drawn in order, each over what is there; no
//! depth test (Phase 12).

use crate::formats::{wrap, REG_BITS};
use crate::scene::{decode, Primitive};
use crate::setup::setup;
use crate::traverse::covered;
use crate::triangle::{Attr, AttrSet, Triangle};
use crate::Plane;

/// Framebuffer width, in pixels.
pub const FB_WIDTH: usize = 640;

/// Framebuffer height, in pixels.
pub const FB_HEIGHT: usize = 360;

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

/// Draws one triangle into a framebuffer.
///
/// # Arguments
///
/// * `fb` - The framebuffer, `FB_WIDTH * FB_HEIGHT` RGB565 words in raster
///   order.
/// * `tri` - The vertices, in role order.
/// * `prim` - The primitive it belongs to (cull mode and program).
///
/// # Returns
///
/// Whether setup kept the triangle.
pub fn draw(fb: &mut [u16], tri: &Triangle, prim: &Primitive) -> bool {
    let attrs = if prim.untextured {
        AttrSet::UNTEXTURED
    } else {
        AttrSet::FULL
    };
    let Ok(s) = setup(tri, prim.cull, attrs) else {
        return false;
    };
    let plane = |a: Attr| s.planes[a.index()].expect("colour is always computed");
    let (r, g, b) = (plane(Attr::R), plane(Attr::G), plane(Attr::B));
    let px0 = s.xs.to_count() >> 4;
    let py0 = s.ys.to_count() >> 4;
    for (px, py) in covered(&s) {
        let (i, j) = (px - px0, py - py0);
        let word = quantise(value(&r, i, j), 5) << 11
            | quantise(value(&g, i, j), 6) << 5
            | quantise(value(&b, i, j), 5);
        let index = usize::try_from(py).expect("on screen") * FB_WIDTH
            + usize::try_from(px).expect("on screen");
        fb[index] = word;
    }
    true
}

/// Renders a scene over a framebuffer.
///
/// # Arguments
///
/// * `fb` - The framebuffer, `FB_WIDTH * FB_HEIGHT` RGB565 words in raster
///   order, holding what the scene is drawn over.
/// * `words` - The scene's command words.
///
/// # Returns
///
/// How many triangles setup kept.
pub fn render(fb: &mut [u16], words: &[u32]) -> usize {
    let mut kept = 0;
    for prim in decode(words) {
        for tri in prim.triangles() {
            kept += usize::from(draw(fb, &tri, &prim));
        }
    }
    kept
}
