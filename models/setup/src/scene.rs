//! Scenes as the host sends them: command packets of 32-bit words
//! (`docs/design/triangle-pipeline.md` §6.1), the built-in test card, and
//! the triangles a scene makes, in the order and with the roles the command
//! parser (`hw/raster.snt`) gives them.
//!
//! A command is a header word, then its vertices:
//!
//! | Bits | Field |
//! |---|---|
//! | 31:28 | opcode: 1 strip, 2 fan, 15 end of scene |
//! | 27:26 | cull mode: 0 none, 1 back, 2 front |
//! | 25 | 1 for the untextured program |
//! | 15:0 | vertices that follow |
//!
//! A vertex is four words: `x @ y` (two `Q12.4` counts), `R @ G @ B @ A`,
//! `Z @ Q`, `S @ T`.

use crate::formats::Coord;
use crate::triangle::{CullMode, Triangle, Vertex};

/// Words in the scene ROM.
pub const SCENE_WORDS: usize = 1024;

/// Header opcodes.
pub mod op {
    /// A triangle strip.
    pub const STRIP: u32 = 1;
    /// A triangle fan.
    pub const FAN: u32 = 2;
    /// The end of the scene.
    pub const END: u32 = 15;
}

/// How a primitive's vertices make triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Each vertex after the second makes a triangle with the two before it;
    /// every other triangle has roles 1 and 2 swapped, so that all of a
    /// strip's triangles wind the same way.
    Strip,
    /// Each vertex after the second makes a triangle with the one before it
    /// and the first.
    Fan,
}

/// One primitive: a strip or a fan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Primitive {
    /// Strip or fan.
    pub kind: Kind,

    /// Which winding setup discards.
    pub cull: CullMode,

    /// Use the untextured program.
    pub untextured: bool,

    /// The vertices, at least three.
    pub vertices: Vec<Vertex>,
}

impl Primitive {
    /// The primitive's triangles, each with its vertices in role order.
    ///
    /// # Returns
    ///
    /// One triangle per vertex after the second.
    pub fn triangles(&self) -> Vec<Triangle> {
        let v = &self.vertices;
        (2..v.len())
            .map(|i| match self.kind {
                Kind::Strip if (i - 2) % 2 == 0 => [v[i - 2], v[i - 1], v[i]],
                Kind::Strip => [v[i - 1], v[i - 2], v[i]],
                Kind::Fan => [v[0], v[i - 1], v[i]],
            })
            .collect()
    }
}

/// The 16-bit field of a signed count.
fn half(x: i64) -> u32 {
    u32::from(u16::from_ne_bytes(
        i16::try_from(x).expect("16-bit field").to_ne_bytes(),
    ))
}

/// Encodes a scene as command words, ending with END.
///
/// # Arguments
///
/// * `prims` - The primitives, drawn in order.
///
/// # Returns
///
/// The words, for the scene ROM.
///
/// # Panics
///
/// If the scene doesn't fit [`SCENE_WORDS`].
pub fn encode(prims: &[Primitive]) -> Vec<u32> {
    let mut words = Vec::new();
    for p in prims {
        let opcode = match p.kind {
            Kind::Strip => op::STRIP,
            Kind::Fan => op::FAN,
        };
        let cull = match p.cull {
            CullMode::None => 0,
            CullMode::Back => 1,
            CullMode::Front => 2,
        };
        let n = u32::try_from(p.vertices.len()).expect("vertex count");
        assert!(
            (3..=0xFFFF).contains(&n),
            "a primitive needs 3 or more vertices"
        );
        words.push(opcode << 28 | cull << 26 | u32::from(p.untextured) << 25 | n);
        for v in &p.vertices {
            words.push(half(v.x.to_count()) << 16 | half(v.y.to_count()));
            words.push(u32::from_be_bytes([v.r, v.g, v.b, v.a]));
            words.push(u32::from(v.z) << 16 | u32::from(v.q));
            words.push(half(v.s.into()) << 16 | half(v.t.into()));
        }
    }
    words.push(op::END << 28);
    assert!(
        words.len() <= SCENE_WORDS,
        "the scene is {} words",
        words.len()
    );
    words
}

/// Decodes command words, as the command parser does.
///
/// # Arguments
///
/// * `words` - The scene ROM's words, from the first.
///
/// # Returns
///
/// The primitives up to END.
pub fn decode(words: &[u32]) -> Vec<Primitive> {
    let signed = |w: u32| {
        i64::from(i16::from_ne_bytes(
            u16::try_from(w & 0xFFFF).expect("16 bits").to_ne_bytes(),
        ))
    };
    let mut prims = Vec::new();
    let mut i = 0;
    loop {
        let h = words[i];
        i += 1;
        let kind = match h >> 28 {
            op::STRIP => Kind::Strip,
            op::FAN => Kind::Fan,
            _ => return prims,
        };
        let cull = match (h >> 26) & 3 {
            1 => CullMode::Back,
            2 => CullMode::Front,
            _ => CullMode::None,
        };
        let n = usize::try_from(h & 0xFFFF).expect("16 bits");
        let vertices = (0..n)
            .map(|k| {
                let w = &words[i + 4 * k..i + 4 * k + 4];
                let [r, g, b, a] = w[1].to_be_bytes();
                Vertex {
                    x: Coord::from_count(signed(w[0] >> 16)),
                    y: Coord::from_count(signed(w[0])),
                    r,
                    g,
                    b,
                    a,
                    z: u16::try_from(w[2] >> 16).expect("16 bits"),
                    q: u16::try_from(w[2] & 0xFFFF).expect("16 bits"),
                    s: i16::try_from(signed(w[3] >> 16)).expect("16 bits"),
                    t: i16::try_from(signed(w[3])).expect("16 bits"),
                }
            })
            .collect();
        i += 4 * n;
        prims.push(Primitive {
            kind,
            cull,
            untextured: (h >> 25) & 1 == 1,
            vertices,
        });
    }
}

/// A vertex at `(x, y)` pixels with colour `rgb`.
fn at(x: f64, y: f64, rgb: [u8; 3]) -> Vertex {
    Vertex {
        x: Coord::from_f64(x),
        y: Coord::from_f64(y),
        r: rgb[0],
        g: rgb[1],
        b: rgb[2],
        a: 255,
        z: 0x8000,
        ..Vertex::default()
    }
}

/// An untextured primitive.
fn prim(kind: Kind, cull: CullMode, vertices: Vec<Vertex>) -> Primitive {
    Primitive {
        kind,
        cull,
        untextured: true,
        vertices,
    }
}

/// The built-in test card (ADR 0002): Gouraud triangles over the test
/// pattern that exercise the whole path. A large triangle across many
/// blocks; a strip and a fan whose triangles share edges; overlapping
/// triangles in painter's order; slivers and tiny triangles; triangles
/// crossing the screen's edges into the guard band; a strip that folds over
/// itself, with back faces culled; colours at the ends of their range.
///
/// # Returns
///
/// The primitives, in drawing order.
// One list, read top to bottom, is clearest.
#[allow(clippy::too_many_lines)]
pub fn test_card() -> Vec<Primitive> {
    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const WHITE: [u8; 3] = [255, 255, 255];
    const BLACK: [u8; 3] = [0, 0, 0];
    use CullMode::{Back, None};
    use Kind::{Fan, Strip};
    let mut prims = Vec::new();

    // A large Gouraud triangle.
    prims.push(prim(
        Strip,
        None,
        vec![
            at(24.25, 18.5, RED),
            at(290.75, 40.0, GREEN),
            at(110.5, 300.25, BLUE),
        ],
    ));

    // A ribbon: a strip of 12 vertices with shared edges.
    let ribbon = (0..12)
        .map(|i| {
            let f = f64::from(i);
            let x = 330.0 + f * 25.3;
            // Bottom, top, bottom, ...: every triangle faces forward.
            let y = if i % 2 == 0 {
                96.0 - f * 2.1
            } else {
                34.0 + f * 1.7
            };
            let c = u8::try_from(i * 23).expect("small");
            at(x, y, [c, 255 - c, if i % 2 == 0 { 255 } else { 40 }])
        })
        .collect();
    prims.push(prim(Strip, Back, ribbon));

    // A fan of 16 triangles around a centre, rainbow coloured.
    let (cx, cy, r) = (470.5, 240.0, 92.0);
    let mut fan = vec![at(cx, cy, WHITE)];
    for k in 0..=16 {
        let a = std::f64::consts::TAU * f64::from(k) / 16.0;
        let hue = [
            u8::try_from((k * 16) % 256).expect("byte"),
            u8::try_from((k * 48 + 80) % 256).expect("byte"),
            u8::try_from(255 - (k * 16) % 256).expect("byte"),
        ];
        fan.push(at(cx + r * a.cos(), cy + r * a.sin(), hue));
    }
    prims.push(prim(Fan, Back, fan));

    // Overlapping triangles, drawn in order: the second covers part of the
    // first.
    prims.push(prim(
        Strip,
        None,
        vec![
            at(170.0, 230.0, RED),
            at(330.0, 250.0, RED),
            at(220.0, 345.0, WHITE),
        ],
    ));
    prims.push(prim(
        Strip,
        None,
        vec![
            at(250.0, 215.0, BLUE),
            at(300.0, 340.0, GREEN),
            at(180.0, 300.0, BLUE),
        ],
    ));

    // Slivers: long and under a pixel high.
    for k in 0..4 {
        let y = 150.0 + f64::from(k) * 3.3;
        prims.push(prim(
            Strip,
            None,
            vec![
                at(330.0, y, WHITE),
                at(620.0, y + 0.5, GREEN),
                at(330.0, y + 0.0625 * f64::from(k + 1), RED),
            ],
        ));
    }

    // Tiny triangles: from a fraction of a pixel to a few pixels.
    for k in 0..8 {
        let s = 0.25 + f64::from(k) * 0.6;
        let x = 340.0 + f64::from(k) * 9.0;
        prims.push(prim(
            Strip,
            None,
            vec![
                at(x, 175.0, WHITE),
                at(x + s, 175.3, WHITE),
                at(x + 0.4, 175.0 + s, BLUE),
            ],
        ));
    }

    // Across the right and bottom edges, into the guard band.
    prims.push(prim(
        Strip,
        None,
        vec![
            at(560.0, 280.0, GREEN),
            at(900.0, 320.0, BLUE),
            at(600.0, 700.0, RED),
        ],
    ));
    // Across the left and top edges.
    prims.push(prim(
        Strip,
        None,
        vec![
            at(-300.0, -50.0, BLUE),
            at(60.0, -20.0, WHITE),
            at(-40.0, 140.0, RED),
        ],
    ));

    // A strip that crosses itself: its first three triangles face forward,
    // the last, past the crossing, faces away and is culled.
    prims.push(prim(
        Strip,
        Back,
        vec![
            at(30.0, 350.0, WHITE),
            at(30.0, 318.0, RED),
            at(70.0, 352.0, GREEN),
            at(70.0, 320.0, BLUE),
            at(140.0, 318.0, WHITE),
            at(110.0, 352.0, RED),
        ],
    ));

    // Colours at the ends of their range, where rounding can step outside.
    prims.push(prim(
        Strip,
        None,
        vec![
            at(130.0, 312.0, BLACK),
            at(190.0, 314.0, WHITE),
            at(150.0, 356.0, WHITE),
        ],
    ));

    prims
}
