//! Watertightness (brief §8): meshes of triangles that share edges, set up
//! and traversed by the reference traversal, cover every pixel exactly
//! once. This also pins the top-left fill rule.

use std::collections::HashMap;

use setup_model::traverse::covered;
use setup_model::{setup, AttrSet, Coord, CullMode, Reject, Triangle, Vertex};

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

/// A vertex at coordinate counts `(x, y)`.
fn at((x, y): (i64, i64)) -> Vertex {
    Vertex {
        x: Coord::from_count(x),
        y: Coord::from_count(y),
        ..Vertex::default()
    }
}

/// Sets up and traverses each triangle, counting how often each pixel is
/// covered.
fn coverage(tris: &[Triangle]) -> HashMap<(i64, i64), u32> {
    let mut count = HashMap::new();
    for tri in tris {
        match setup(tri, CullMode::None, AttrSet::UNTEXTURED) {
            Ok(s) => {
                for p in covered(&s) {
                    *count.entry(p).or_insert(0) += 1;
                }
            }
            Err(Reject::ZeroArea) => {}
            Err(Reject::Culled) => unreachable!("no culling"),
        }
    }
    count
}

/// A triangle in a random rotation of its vertices, sometimes reversed (so
/// setup flips it).
fn shuffled(rng: &mut Lcg, mut t: [(i64, i64); 3]) -> Triangle {
    t.rotate_left(usize::try_from(rng.next() % 3).expect("small"));
    if rng.next().is_multiple_of(2) {
        t.swap(1, 2);
    }
    t.map(at)
}

/// A grid of `nx` by `ny` cells over a rectangle of whole pixels, its
/// inner vertices moved at random by up to a quarter of a cell each way
/// (so every cell stays convex and either diagonal splits it), triangulated
/// with random diagonals. With `on_centres`, every vertex is on a pixel centre
/// (a tie for the fill rule wherever an edge is axis-aligned).
fn grid(
    rng: &mut Lcg,
    nx: i64,
    ny: i64,
    on_centres: bool,
) -> ((i64, i64, i64, i64), Vec<Triangle>) {
    let (cell_w, cell_h) = (rng.range(3, 40), rng.range(3, 40));
    let x0 = rng.range(-60, 600);
    let y0 = rng.range(-60, 340);
    let (x1, y1) = (x0 + nx * cell_w, y0 + ny * cell_h);
    let snap = |c: i64| if on_centres { (c >> 4 << 4) + 8 } else { c };
    let mut pts = vec![
        vec![(0, 0); usize::try_from(ny + 1).expect("small")];
        usize::try_from(nx + 1).expect("small")
    ];
    for i in 0..=nx {
        for j in 0..=ny {
            let mut x = (x0 + i * cell_w) * 16;
            let mut y = (y0 + j * cell_h) * 16;
            if i > 0 && i < nx {
                x = snap(x + rng.range(-cell_w * 4, cell_w * 4));
            }
            if j > 0 && j < ny {
                y = snap(y + rng.range(-cell_h * 4, cell_h * 4));
            }
            pts[usize::try_from(i).expect("small")][usize::try_from(j).expect("small")] = (x, y);
        }
    }
    let mut tris = Vec::new();
    for i in 0..usize::try_from(nx).expect("small") {
        for j in 0..usize::try_from(ny).expect("small") {
            let (a, b, c, d) = (pts[i][j], pts[i + 1][j], pts[i + 1][j + 1], pts[i][j + 1]);
            let pair = if rng.next().is_multiple_of(2) {
                [[a, b, c], [a, c, d]]
            } else {
                [[a, b, d], [b, c, d]]
            };
            for t in pair {
                tris.push(shuffled(rng, t));
            }
        }
    }
    ((x0, y0, x1, y1), tris)
}

/// Every on-screen pixel centre inside a grid's rectangle is covered
/// exactly once, and no pixel outside it is covered.
fn check_grid(on_centres: bool) {
    let mut rng = Lcg(if on_centres { 99 } else { 7 });
    for _ in 0..200 {
        let (nx, ny) = (rng.range(1, 12), rng.range(1, 12));
        let ((x0, y0, x1, y1), tris) = grid(&mut rng, nx, ny, on_centres);
        let count = coverage(&tris);
        for (&(px, py), &n) in &count {
            assert!(
                px >= x0 && px < x1 && py >= y0 && py < y1,
                "pixel ({px}, {py}) outside"
            );
            assert_eq!(n, 1, "pixel ({px}, {py}) covered {n} times");
        }
        for px in x0.max(0)..x1.min(640) {
            for py in y0.max(0)..y1.min(360) {
                assert!(
                    count.contains_key(&(px, py)),
                    "pixel ({px}, {py}) not covered"
                );
            }
        }
    }
}

/// Grids with vertices anywhere.
#[test]
fn grids_are_watertight() {
    check_grid(false);
}

/// Grids with every vertex on a pixel centre.
#[test]
fn grids_on_pixel_centres_are_watertight() {
    check_grid(true);
}

/// Fans of many thin triangles around a shared centre cover no pixel twice.
#[test]
fn fans_cover_each_pixel_once() {
    let mut rng = Lcg(3);
    for _ in 0..100 {
        let centre = (rng.range(0, 640 * 16), rng.range(0, 360 * 16));
        let n = rng.range(8, 64);
        let radius = rng.range(16 * 4, 16 * 200);
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let rim: Vec<(i64, i64)> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * (k as f64 + f64::from(rng.next() % 100) / 200.0)
                    / n as f64;
                (
                    centre.0 + (radius as f64 * a.cos()).round() as i64,
                    centre.1 + (radius as f64 * a.sin()).round() as i64,
                )
            })
            .collect();
        let tris: Vec<Triangle> = (0..rim.len())
            .map(|k| shuffled(&mut rng, [centre, rim[k], rim[(k + 1) % rim.len()]]))
            .collect();
        for (&(px, py), &count) in &coverage(&tris) {
            assert_eq!(count, 1, "pixel ({px}, {py}) covered {count} times");
        }
    }
}
