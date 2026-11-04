//! Edge cases of the fixed-point setup (brief §8): zero areas, culling and
//! flipping, exact edge functions, the start point and bounding box, and
//! triangles at the extremes of the guard band.

use setup_model::{
    float, setup, Attr, AttrSet, Coord, CullMode, Reject, Triangle, Vertex, X_LIMIT, Y_LIMIT,
};

/// A vertex at `(x, y)` pixels with the given red and depth.
fn v(x: f64, y: f64, r: u8, z: u16) -> Vertex {
    Vertex {
        x: Coord::from_f64(x),
        y: Coord::from_f64(y),
        r,
        z,
        ..Vertex::default()
    }
}

/// A clockwise triangle on the screen (positive area, y down).
fn clockwise() -> Triangle {
    [
        v(10.0, 10.0, 0, 0),
        v(50.0, 12.0, 200, 30_000),
        v(20.0, 40.0, 100, 60_000),
    ]
}

/// Coincident and collinear vertices are rejected as having no area.
#[test]
fn zero_area_is_rejected() {
    let p = v(100.25, 50.5, 0, 0);
    for tri in [
        [p, p, p],
        [p, p, v(3.0, 4.0, 0, 0)],
        [v(0.0, 0.0, 0, 0), v(10.0, 5.0, 0, 0), v(20.0, 10.0, 0, 0)],
        [
            v(-2048.0, 7.0, 0, 0),
            v(0.0, 7.0, 0, 0),
            v(2047.9375, 7.0, 0, 0),
        ],
    ] {
        for cull in [CullMode::None, CullMode::Back, CullMode::Front] {
            assert_eq!(
                setup(&tri, cull, AttrSet::FULL),
                Err(Reject::ZeroArea),
                "{tri:?}"
            );
        }
    }
}

/// Each cull mode keeps and discards the windings it should, and a kept
/// anticlockwise triangle is flipped.
#[test]
fn culling_and_flipping() {
    let cw = clockwise();
    let ccw = [cw[0], cw[2], cw[1]];
    let keeps = |tri: &Triangle, cull| setup(tri, cull, AttrSet::FULL).ok().map(|s| s.flipped);
    assert_eq!(keeps(&cw, CullMode::None), Some(false));
    assert_eq!(keeps(&ccw, CullMode::None), Some(true));
    assert_eq!(keeps(&cw, CullMode::Back), Some(false));
    assert_eq!(keeps(&ccw, CullMode::Back), None);
    assert_eq!(keeps(&cw, CullMode::Front), None);
    assert_eq!(keeps(&ccw, CullMode::Front), Some(true));
    assert_eq!(
        setup(&ccw, CullMode::Back, AttrSet::FULL),
        Err(Reject::Culled)
    );
}

/// A flipped triangle has the same planes as its unflipped twin, and the
/// same edges in the other order.
#[test]
fn flipped_triangles_match() {
    let cw = clockwise();
    let ccw = [cw[0], cw[2], cw[1]];
    let a = setup(&cw, CullMode::None, AttrSet::FULL).expect("kept");
    let b = setup(&ccw, CullMode::None, AttrSet::FULL).expect("kept");
    assert_eq!(a.planes, b.planes);
    assert_eq!(a.edges, b.edges);
    assert_eq!((a.xs, a.ys, a.xmax, a.ymax), (b.xs, b.ys, b.xmax, b.ymax));
}

/// The edge functions at the start point are exact, and positive inside:
/// at each vertex, the edge opposite it has the value `2 * area`.
#[test]
fn edges_are_exact() {
    let tri = [
        v(-100.0625, 3.5, 0, 0),
        v(600.9375, -40.25, 0, 0),
        v(250.1875, 355.8125, 0, 0),
    ];
    let s = setup(&tri, CullMode::None, AttrSet::UNTEXTURED).expect("kept");
    let count = |c: Coord| c.to_count();
    let (xc, yc) = (count(s.xs) + 8, count(s.ys) + 8);
    let p = if s.flipped {
        [tri[0], tri[2], tri[1]]
    } else {
        tri
    };
    for (i, e) in s.edges.iter().enumerate() {
        let (vi, vj) = (p[i], p[(i + 1) % 3]);
        assert_eq!(e.a.to_count(), count(vi.y) - count(vj.y), "A{i}");
        assert_eq!(e.b.to_count(), count(vj.x) - count(vi.x), "B{i}");
        let exact = e.a.to_count() * (xc - count(vi.x)) + e.b.to_count() * (yc - count(vi.y));
        assert_eq!(e.e.to_count(), exact, "E{i}");
        // At the vertex opposite the edge, E is twice the area.
        let vk = p[(i + 2) % 3];
        let at_vk = e.a.to_count() * (count(vk.x) - count(vi.x))
            + e.b.to_count() * (count(vk.y) - count(vi.y));
        let reference = float::setup(&p).expect("non-zero");
        #[allow(clippy::cast_precision_loss)]
        let twice_area = at_vk as f64 / 256.0;
        assert!((twice_area - reference.area).abs() < 1e-9, "edge {i}");
        assert!(at_vk > 0, "edge {i} not positive inside");
    }
}

/// The start point is the bounding box's top left clipped to the
/// framebuffer and snapped to the 8x8 grid; the far corner is clipped to
/// the last pixel centre.
#[test]
fn start_point_and_bounding_box() {
    let tri = [
        v(-30.0, 17.9, 0, 0),
        v(900.0, 23.0, 0, 0),
        v(5.0, 500.0, 0, 0),
    ];
    let s = setup(&tri, CullMode::None, AttrSet::UNTEXTURED).expect("kept");
    assert_eq!((s.xs.to_f64(), s.ys.to_f64()), (0.0, 16.0));
    assert_eq!((s.xmax, s.ymax), (X_LIMIT, Y_LIMIT));
    let inside = [v(13.0, 9.0, 0, 0), v(30.0, 9.5, 0, 0), v(20.0, 30.0, 0, 0)];
    let s = setup(&inside, CullMode::None, AttrSet::UNTEXTURED).expect("kept");
    assert_eq!((s.xs.to_f64(), s.ys.to_f64()), (8.0, 8.0));
    assert_eq!((s.xmax.to_f64(), s.ymax.to_f64()), (30.0, 30.0));
}

/// The untextured program computes colour, alpha and Z only.
#[test]
fn program_variants() {
    let s = setup(&clockwise(), CullMode::None, AttrSet::UNTEXTURED).expect("kept");
    for attr in Attr::ALL {
        let expected = !matches!(attr, Attr::S | Attr::T | Attr::Q);
        assert_eq!(s.planes[attr.index()].is_some(), expected, "{attr:?}");
    }
}

/// Slivers a sixteenth of a pixel high and triangles spanning the whole
/// guard band don't overflow the accumulator (`post_shift` asserts it),
/// and give finite planes.
#[test]
fn extremes_do_not_overflow() {
    let lo = -2048.0;
    let hi = 2047.9375;
    let cases: [Triangle; 6] = [
        [v(lo, lo, 0, 0), v(hi, lo, 255, 65535), v(lo, hi, 0, 65535)],
        [v(hi, hi, 255, 0), v(lo, hi, 0, 65535), v(hi, lo, 0, 0)],
        [
            v(0.0, 100.0, 0, 0),
            v(639.0, 100.0625, 255, 65535),
            v(320.0, 100.0, 0, 65535),
        ],
        [
            v(0.0, 0.0, 0, 0),
            v(0.0625, 0.0, 255, 65535),
            v(0.0, 0.0625, 255, 0),
        ],
        [
            v(lo, 0.0, 0, 0),
            v(hi, 0.0625, 255, 65535),
            v(hi, 0.0, 0, 0),
        ],
        [
            v(10.0, 10.0, 255, 65535),
            v(10.0625, 2000.0, 0, 0),
            v(10.0, 2000.0, 255, 0),
        ],
    ];
    for tri in cases {
        let s = setup(&tri, CullMode::None, AttrSet::FULL).expect("non-zero area");
        for plane in s.planes.iter().flatten() {
            assert!(plane.dx.to_f64().is_finite() && plane.dy.to_f64().is_finite());
        }
    }
}
