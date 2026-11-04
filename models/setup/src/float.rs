//! The `f64` reference: the brief's §4.2 mathematics with (next to) no
//! rounding, for measuring the fixed-point version's error.
//!
//! It works from the same snapped, clipped start point as the fixed-point
//! version, so the two describe the same planes and can be compared value
//! for value.

use crate::triangle::{Attr, Triangle};

/// One attribute's plane, in native units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    /// The value at the start point's pixel centre.
    pub start: f64,

    /// The change per pixel to the right.
    pub dx: f64,

    /// The change per pixel down.
    pub dy: f64,
}

/// A triangle's setup, in `f64`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    /// Twice the signed area, in square pixels, of the vertices as given.
    pub area: f64,

    /// The start point's pixel centre.
    pub xc: f64,

    /// See [`Setup::xc`].
    pub yc: f64,

    /// Every attribute's plane, with roles 1 and 2 swapped first if the
    /// area is negative (the planes are the same either way).
    pub planes: [Plane; 8],
}

/// Sets up a triangle in `f64`.
///
/// # Arguments
///
/// * `tri` - The vertices.
///
/// # Returns
///
/// The setup, or `None` if the area is zero.
pub fn setup(tri: &Triangle) -> Option<Setup> {
    let x = tri.map(|v| v.x.to_f64());
    let y = tri.map(|v| v.y.to_f64());
    let (dx1, dy1, dx2, dy2) = (x[1] - x[0], y[1] - y[0], x[2] - x[0], y[2] - y[0]);
    let area = dx1 * dy2 - dx2 * dy1;
    if area == 0.0 {
        return None;
    }
    // The bounding box's top left, clipped to the framebuffer and snapped
    // down to the 8x8 block grid.
    let snap = |v: f64| (v.max(0.0) / 8.0).floor() * 8.0;
    let xc = snap(x[0].min(x[1]).min(x[2])) + 0.5;
    let yc = snap(y[0].min(y[1]).min(y[2])) + 0.5;
    let (cx, cy) = (xc - x[0], yc - y[0]);
    let planes = Attr::ALL.map(|attr| {
        #[allow(clippy::cast_precision_loss)]
        let a = tri.map(|v| attr.of(&v) as f64);
        let (d1, d2) = (a[1] - a[0], a[2] - a[0]);
        let dx = (d1 * dy2 - d2 * dy1) / area;
        let dy = (d2 * dx1 - d1 * dx2) / area;
        Plane {
            start: a[0] + dx * cx + dy * cy,
            dx,
            dy,
        }
    });
    Some(Setup {
        area,
        xc,
        yc,
        planes,
    })
}
