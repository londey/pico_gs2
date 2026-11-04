//! The reference traversal: which pixels a set-up triangle covers, from
//! setup's outputs alone, as the hardware traversal will decide it.
//!
//! Pixel `(px, py)`'s centre is at `(px + 0.5, py + 0.5)`. Each edge
//! function starts at the start point's pixel centre and steps exactly: by
//! `16 * A` per pixel right and `16 * B` per pixel down (coordinates have 4
//! fraction bits). A pixel is covered when all three are positive, or zero
//! on a top or left edge (the top-left fill rule), so that triangles
//! sharing an edge cover each pixel on it exactly once.

use crate::setup::{Edge, Setup};

/// Whether an edge function value covers its pixel: inside, or exactly on
/// a left edge (`A > 0`: inside is to the right) or a top edge (`A == 0`,
/// `B > 0`: horizontal, inside below, as y increases downwards).
fn covers(e: i64, edge: &Edge) -> bool {
    let (a, b) = (edge.a.to_count(), edge.b.to_count());
    e > 0 || (e == 0 && (a > 0 || (a == 0 && b > 0)))
}

/// The pixels a triangle covers, in raster order.
///
/// # Arguments
///
/// * `s` - The triangle's setup.
///
/// # Returns
///
/// Each covered pixel's `(x, y)`.
pub fn covered(s: &Setup) -> Vec<(i64, i64)> {
    let px0 = s.xs.to_count() >> 4;
    let py0 = s.ys.to_count() >> 4;
    // The last pixels whose centres are within the box.
    let px1 = (s.xmax.to_count() - 8) >> 4;
    let py1 = (s.ymax.to_count() - 8) >> 4;
    let mut pixels = Vec::new();
    for py in py0..=py1 {
        for px in px0..=px1 {
            let inside = s.edges.iter().all(|edge| {
                let e = edge.e.to_count()
                    + 16 * edge.a.to_count() * (px - px0)
                    + 16 * edge.b.to_count() * (py - py0);
                covers(e, edge)
            });
            if inside {
                pixels.push((px, py));
            }
        }
    }
    pixels
}
