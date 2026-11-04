//! The bit-exact fixed-point setup (brief §4.2 and §4.5).
//!
//! Each step is one of the reference program's operations, in the format
//! the engine computes it in. On the multiplier the first operand is the
//! 36-bit port and the second the 18-bit port; results reach registers
//! through [`post_shift`](crate::post_shift), via `narrow`.

use qfixed::typenum::{U15, U20, U23, U26, U27, U30, U33, U37, U8};
use qfixed::Q;

use crate::formats::{narrow, wrap, AttrDelta, Coord, Delta, Iter, Wide, K, REG_BITS};
use crate::reciprocal::{normalise, reciprocal};
use crate::triangle::{Attr, AttrSet, CullMode, Triangle, Vertex};

/// The largest x a covered pixel's centre can have: the framebuffer's last
/// column, 639, plus half a pixel. The bounding box is clipped to it.
pub const X_LIMIT: Coord = Coord::from_count(639 * 16 + 8);

/// The largest y a covered pixel's centre can have: line 359 plus half.
pub const Y_LIMIT: Coord = Coord::from_count(359 * 16 + 8);

/// Pixel centre offset: half a pixel.
const HALF: Coord = Coord::from_count(8);

/// Traversal works in 8x8 blocks, so the start point is snapped down to a
/// multiple of 8 pixels: the low 7 bits of a coordinate's count cleared.
const BLOCK_MASK: i64 = !(8 * 16 - 1);

/// One edge function, `E(p) = A * (p.x - x_i) + B * (p.y - y_i)` for the
/// edge from vertex `i` to vertex `j`: positive inside the triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// `dE/dx = y_i - y_j`.
    pub a: Delta,

    /// `dE/dy = x_j - x_i`.
    pub b: Delta,

    /// `E` at the start point's pixel centre, exact.
    pub e: Wide,
}

/// One attribute's plane, as iterator values in native units with 16
/// fraction bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    /// The value at the start point's pixel centre.
    pub start: Iter,

    /// The change per pixel to the right.
    pub dx: Iter,

    /// The change per pixel down.
    pub dy: Iter,
}

/// What setup hands to traversal for a triangle it keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    /// Whether roles 1 and 2 were swapped to make the area positive.
    pub flipped: bool,

    /// The edges 0→1, 1→2 and 2→0, of the vertices after any flip.
    pub edges: [Edge; 3],

    /// The start point: the bounding box's top left, clipped to the
    /// framebuffer and snapped down to the 8x8 block grid.
    pub xs: Coord,

    /// See [`Setup::xs`].
    pub ys: Coord,

    /// The bounding box's right edge, clipped to [`X_LIMIT`].
    pub xmax: Coord,

    /// The bounding box's bottom edge, clipped to [`Y_LIMIT`].
    pub ymax: Coord,

    /// The exponent `E` of twice the area (for reports and tests).
    pub e: u32,

    /// Each attribute's plane, `None` for one the program doesn't compute.
    pub planes: [Option<Plane>; 8],
}

/// Why setup discarded a triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Its area is zero: no pixel centre can be inside it.
    ZeroArea,

    /// Its winding is the one the cull mode discards.
    Culled,
}

/// Sets up a triangle.
///
/// # Arguments
///
/// * `tri` - The vertices, in the order the host sent them.
/// * `cull` - Which winding to discard.
/// * `attrs` - The attributes to compute planes for.
///
/// # Returns
///
/// The edges, bounding box and planes.
///
/// # Errors
///
/// [`Reject::ZeroArea`] or [`Reject::Culled`] for a triangle traversal
/// should never see.
pub fn setup(tri: &Triangle, cull: CullMode, attrs: AttrSet) -> Result<Setup, Reject> {
    let area = twice_area(tri[0], tri[1], tri[2]);
    let count = area.to_count();
    if count == 0 {
        return Err(Reject::ZeroArea);
    }
    let culled = match cull {
        CullMode::None => false,
        CullMode::Back => count < 0,
        CullMode::Front => count > 0,
    };
    if culled {
        return Err(Reject::Culled);
    }
    // A negative area: swap roles 1 and 2 and start again, so that
    // everything after sees a positive one.
    let flipped = count < 0;
    let v = if flipped {
        [tri[0], tri[2], tri[1]]
    } else {
        *tri
    };
    Ok(setup_positive(&v, flipped, attrs))
}

/// Twice the signed area: `dx1 * dy2 - dx2 * dy1` (positive clockwise on
/// the screen, as y increases downwards). Exact.
fn twice_area(v0: Vertex, v1: Vertex, v2: Vertex) -> Wide {
    let (dx1, dy1, dx2, dy2) = deltas(v0, v1, v2);
    let p = dx1.widening_mul::<_, _, U26, U8>(dy2);
    let q = dx2.widening_mul::<_, _, U26, U8>(dy1);
    // MUL dx1, dy2; MSUB dx2, dy1.
    narrow::<U26, U8, U27, U8>(p - q, None, false, false)
}

/// `x1 - x0`, `y1 - y0`, `x2 - x0`, `y2 - y0`.
fn deltas(v0: Vertex, v1: Vertex, v2: Vertex) -> (Delta, Delta, Delta, Delta) {
    (v1.x - v0.x, v1.y - v0.y, v2.x - v0.x, v2.y - v0.y)
}

/// Setup of a triangle whose area is positive.
// The names are the brief's (§4.2): v, c, k, r, w, n.
#[allow(clippy::many_single_char_names)]
fn setup_positive(v: &Triangle, flipped: bool, attrs: AttrSet) -> Setup {
    let (dx1, dy1, dx2, dy2) = deltas(v[0], v[1], v[2]);
    let area = twice_area(v[0], v[1], v[2]);

    // Bounding box, clipped to the framebuffer; the start point snapped to
    // the block grid.
    let xmin = v[0].x.min(v[1].x).min(v[2].x).max(Coord::ZERO);
    let ymin = v[0].y.min(v[1].y).min(v[2].y).max(Coord::ZERO);
    let xmax = v[0].x.max(v[1].x).max(v[2].x).min(X_LIMIT);
    let ymax = v[0].y.max(v[1].y).max(v[2].y).min(Y_LIMIT);
    let xs = Coord::from_count(xmin.to_count() & BLOCK_MASK);
    let ys = Coord::from_count(ymin.to_count() & BLOCK_MASK);
    let xc = add(xs, HALF);
    let yc = add(ys, HALF);
    // The pixel centre relative to each vertex.
    let c: [(Delta, Delta); 3] = v.map(|p| (xc - p.x, yc - p.y));

    // Edges.
    let a12 = v[1].y - v[2].y;
    let b12 = v[2].x - v[1].x;
    let edges = [
        edge(neg(dy1), dx1, c[0]),
        edge(a12, b12, c[1]),
        edge(dy2, neg(dx2), c[2]),
    ];

    // 1 / area, as r = 1 / m with area = m * 2^(E - 8).
    let n = normalise(area);
    let r = reciprocal(n.m);
    // Gradient factors: k = d * r, true value k * 2^(8 - E).
    let k = |d: Delta| -> K { narrow(r.widening_mul::<_, _, U15, U37>(d), None, true, true) };
    let (k1, k2, k3, k4) = (k(dy2), k(dy1), k(dx1), k(dx2));
    let (cx, cy) = c[0];

    let mut planes = [None; 8];
    for attr in Attr::ALL {
        if !attrs.contains(attr) {
            continue;
        }
        let a0 = attr.of(&v[0]);
        let d1 = AttrDelta::from_count(attr.of(&v[1]) - a0);
        let d2 = AttrDelta::from_count(attr.of(&v[2]) - a0);
        // da/dx = d1 k1 - d2 k2; da/dy = d2 k3 - d1 k4, saturating.
        let dx: Iter = narrow(prod_ka(k1, d1) - prod_ka(k2, d2), Some(n.e), true, true);
        let dy: Iter = narrow(prod_ka(k3, d2) - prod_ka(k4, d1), Some(n.e), true, true);
        // start = a0 + dx cx + dy cy: the products on the multiplier, the
        // add of a0 (in iterator format) on the ALU (ADDV). Both wrap: the
        // start point can be far outside a thin triangle, where the plane
        // is out of range, but traversal's adders wrap the same way, so
        // stepping back inside gives the right values.
        let p: Iter = narrow(prod_id(dx, cx) + prod_id(dy, cy), None, true, false);
        let start = Iter::from_count(wrap(p.to_count() + (a0 << 16), REG_BITS));
        planes[attr.index()] = Some(Plane { start, dx, dy });
    }

    Setup {
        flipped,
        edges,
        xs,
        ys,
        xmax,
        ymax,
        e: n.e,
        planes,
    }
}

/// An edge's A, B and value at the start point:
/// `E = A * cx + B * cy` (MUL then MAC), exact.
fn edge(a: Delta, b: Delta, (cx, cy): (Delta, Delta)) -> Edge {
    let p = a.widening_mul::<_, _, U26, U8>(cx);
    let q = b.widening_mul::<_, _, U26, U8>(cy);
    Edge {
        a,
        b,
        e: narrow::<U26, U8, U27, U8>(p + q, None, false, false),
    }
}

/// `-d`, which always fits: a coordinate difference is never −4096.
fn neg(d: Delta) -> Delta {
    Delta::from_count(-d.to_count())
}

/// A coordinate plus an offset, wrapping as the 36-bit ALU would not:
/// the sums here stay in range.
fn add(a: Coord, b: Coord) -> Coord {
    let sum = a.to_count() + b.to_count();
    debug_assert_eq!(wrap(sum, 16), sum, "coordinate overflow");
    Coord::from_count(sum)
}

/// `g * c`, exact: an iterator value on the 36-bit port, a [`Delta`] on
/// the 18-bit port.
fn prod_id(g: Iter, c: Delta) -> Q<U33, U20> {
    g.widening_mul(c)
}

/// `k * d`, exact, for an attribute delta `d`.
fn prod_ka(k: K, d: AttrDelta) -> Q<U30, U23> {
    k.widening_mul(d)
}
