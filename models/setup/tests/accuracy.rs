//! The fixed-point setup against the `f64` reference over random triangles
//! (brief §8): for each attribute, the error of its gradients, and the worst
//! error its plane reaches inside the triangle.
//!
//! A plane's error is linear, so inside the triangle it is worst at a
//! vertex. The plane is evaluated there from the start point, as traversal
//! steps it, modulo the iterators' 36-bit range (`Iter`).

use setup_model::{float, setup, Attr, AttrSet, Coord, CullMode, Iter, Triangle, Vertex};

/// Random triangles per size class.
const TRIANGLES: usize = 20_000;

/// A small linear congruential generator, so the test is repeatable.
struct Lcg(u64);

impl Lcg {
    /// The next 32 pseudo-random bits.
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        u32::try_from(self.0 >> 32).expect("32 bits")
    }

    /// A pseudo-random integer in `lo..hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + i64::from(self.next()) % (hi - lo)
    }
}

/// A random vertex at `(x, y)` (coordinate counts), with random attributes.
fn vertex(rng: &mut Lcg, x: i64, y: i64) -> Vertex {
    let byte = |rng: &mut Lcg| u8::try_from(rng.next() & 0xFF).expect("8 bits");
    let half = |rng: &mut Lcg| u16::try_from(rng.next() & 0xFFFF).expect("16 bits");
    Vertex {
        x: Coord::from_count(x),
        y: Coord::from_count(y),
        r: byte(rng),
        g: byte(rng),
        b: byte(rng),
        a: byte(rng),
        z: half(rng),
        s: i16::from_ne_bytes(half(rng).to_ne_bytes()),
        t: i16::from_ne_bytes(half(rng).to_ne_bytes()),
        q: half(rng),
    }
}

/// A random triangle whose vertices lie within `size` pixels of a random
/// point on or near the screen.
fn triangle(rng: &mut Lcg, size: i64) -> Triangle {
    let cx = rng.range(-32 * 16, 672 * 16);
    let cy = rng.range(-32 * 16, 392 * 16);
    let mut v = || {
        let x = (cx + rng.range(-size * 16, size * 16 + 1)).clamp(-2048 * 16, 2048 * 16 - 1);
        let y = (cy + rng.range(-size * 16, size * 16 + 1)).clamp(-2048 * 16, 2048 * 16 - 1);
        (x, y)
    };
    let p = [v(), v(), v()];
    p.map(|(x, y)| vertex(rng, x, y))
}

/// The span of an iterator's integer part: values wrap modulo this.
const SPAN: f64 = (1_u64 << 20) as f64;

/// Errors seen for one attribute, in its native units.
#[derive(Default)]
struct Errors {
    /// Worst gradient error, per pixel, of a gradient that didn't saturate.
    grad: f64,

    /// Worst error the plane reaches inside the triangle.
    inside: f64,

    /// Sum of squared inside errors, for the RMS.
    sum_sq: f64,

    /// Triangles counted.
    n: usize,

    /// Triangles whose gradient saturated.
    saturated: usize,
}

/// `x` wrapped into `-SPAN / 2 .. SPAN / 2`.
fn wrapped(x: f64) -> f64 {
    x - (x / SPAN).round() * SPAN
}

/// A triangle's shape, by its area and its smallest altitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// At least a square pixel, and no altitude under a pixel.
    Ordinary,
    /// At least a square pixel, but an altitude under a pixel: long and
    /// thin, with steep gradients across it.
    Sliver,
    /// Under a square pixel.
    Tiny,
}

impl Shape {
    /// Classifies a triangle.
    ///
    /// # Arguments
    ///
    /// * `tri` - The triangle.
    /// * `twice_area` - Twice its signed area, in square pixels.
    ///
    /// # Returns
    ///
    /// Its shape.
    fn of(tri: &Triangle, twice_area: f64) -> Shape {
        let longest = (0..3)
            .map(|i| {
                let (a, b) = (tri[i], tri[(i + 1) % 3]);
                (b.x.to_f64() - a.x.to_f64()).hypot(b.y.to_f64() - a.y.to_f64())
            })
            .fold(0.0, f64::max);
        if twice_area.abs() < 2.0 {
            Shape::Tiny
        } else if twice_area.abs() / longest < 1.0 {
            Shape::Sliver
        } else {
            Shape::Ordinary
        }
    }
}

/// Measures one class of triangles and prints a report.
///
/// # Arguments
///
/// * `name` - The class's name, for the report.
/// * `size` - How far, in pixels, vertices may be from a random centre.
/// * `shape` - The shape of triangle to count; others are skipped.
///
/// # Returns
///
/// For each attribute, the worst error inside a triangle, in native LSBs,
/// and how many triangles' gradients saturated.
fn measure(name: &str, size: i64, shape: Shape) -> [(f64, usize); 8] {
    let mut rng = Lcg(u64::try_from(size).expect("positive") * 7919);
    let mut errors: [Errors; 8] = Default::default();
    let mut counted = 0;
    while counted < TRIANGLES {
        let tri = triangle(&mut rng, size);
        let Some(reference) = float::setup(&tri) else {
            continue;
        };
        if Shape::of(&tri, reference.area) != shape {
            continue;
        }
        let fixed = setup(&tri, CullMode::None, AttrSet::FULL).expect("non-zero area");
        // Only triangles with a pixel on the screen reach traversal.
        if fixed.xmax < fixed.xs || fixed.ymax < fixed.ys {
            continue;
        }
        counted += 1;
        let (xc, yc) = (fixed.xs.to_f64() + 0.5, fixed.ys.to_f64() + 0.5);
        for attr in Attr::ALL {
            let f = fixed.planes[attr.index()].expect("full program");
            let r = reference.planes[attr.index()];
            let e = &mut errors[attr.index()];
            if [f.dx, f.dy]
                .iter()
                .any(|g| *g == Iter::MAX || *g == Iter::MIN)
            {
                e.saturated += 1;
                continue;
            }
            let (start, dx, dy) = (f.start.to_f64(), f.dx.to_f64(), f.dy.to_f64());
            #[allow(clippy::cast_precision_loss)]
            let inside = tri
                .iter()
                .map(|v| {
                    let value = start + dx * (v.x.to_f64() - xc) + dy * (v.y.to_f64() - yc);
                    wrapped(value - attr.of(v) as f64).abs()
                })
                .fold(0.0, f64::max);
            e.grad = e.grad.max((dx - r.dx).abs().max((dy - r.dy).abs()));
            e.inside = e.inside.max(inside);
            e.sum_sq += inside * inside;
            e.n += 1;
        }
    }
    println!("{name} {shape:?} triangles: errors in native LSBs");
    println!("  attr       grad     inside    rms   saturated");
    for attr in Attr::ALL {
        let e = &errors[attr.index()];
        #[allow(clippy::cast_precision_loss)]
        let rms = (e.sum_sq / e.n.max(1) as f64).sqrt();
        println!(
            "  {:<4} {:>10.2e} {:>10.2e} {:>10.2e} {:>6}",
            format!("{attr:?}"),
            e.grad,
            e.inside,
            rms,
            e.saturated
        );
    }
    errors.map(|e| (e.inside, e.saturated))
}

/// The worst error allowed inside an ordinary triangle, in native LSBs:
/// a sixteenth, so that rounding to the output's LSB is right but for
/// values within a sixteenth of a tie. What's left is the reciprocal's
/// 22 bits times the gradient times the triangle's size.
const TOLERANCE: f64 = 1.0 / 16.0;

/// Ordinary triangles: anywhere inside the triangle, the fixed-point plane
/// is within [`TOLERANCE`] of the exact one, and no gradient saturates.
#[test]
fn ordinary_triangles_are_accurate() {
    for (name, size) in [("small", 8), ("medium", 64), ("large", 400)] {
        let worst = measure(name, size, Shape::Ordinary);
        for attr in Attr::ALL {
            let (inside, saturated) = worst[attr.index()];
            assert_eq!(
                saturated, 0,
                "{name} {attr:?}: {saturated} saturated gradients"
            );
            assert!(inside < TOLERANCE, "{name} {attr:?}: {inside:.3e} LSB");
        }
    }
}

/// Slivers and tiny triangles: steep gradients amplify the gradient
/// factors' rounding, and the steepest saturate. Colour stays within an
/// LSB; the 16-bit attributes, with random deltas up to 65535, are
/// reported only.
#[test]
fn slivers_and_tiny_triangles_report() {
    for (name, size, shape) in [
        ("medium", 64, Shape::Sliver),
        ("large", 400, Shape::Sliver),
        ("small", 1, Shape::Tiny),
    ] {
        let worst = measure(name, size, shape);
        for attr in [Attr::R, Attr::G, Attr::B, Attr::A] {
            let (inside, _) = worst[attr.index()];
            assert!(inside < 1.0, "{name} {shape:?} {attr:?}: {inside:.3e} LSB");
        }
    }
}
