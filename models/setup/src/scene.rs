//! Scenes as the host sends them: command packets of 32-bit words
//! (`docs/design/triangle-pipeline.md` §6.1), the built-in test card and
//! spinning cube, and the triangles a scene makes, in the order and with
//! the roles the command parser (`hw/raster.snt`) gives them.
//!
//! A command is a header word, then, for a strip or fan, its vertices:
//!
//! | Bits | Field |
//! |---|---|
//! | 31:28 | opcode: 1 strip, 2 fan, 3 clear, 4 end of frame, 15 end of scene |
//! | 27:26 | strip, fan: cull mode: 0 none, 1 back, 2 front |
//! | 25 | strip, fan: 1 for the untextured program |
//! | 15:0 | strip, fan: vertices that follow; clear: the RGB565 colour |
//!
//! Clear fills the frame being drawn with a colour and its depth with the
//! farthest value; end of frame hands it to the display; end of scene goes
//! back to the start (the animation loops).
//!
//! A vertex is four words: `x @ y` (two `Q12.4` counts), `R @ G @ B @ A`,
//! `Z @ Q`, `S @ T`.

use crate::formats::Coord;
use crate::triangle::{CullMode, Triangle, Vertex};

/// Words in the scene ROM.
pub const SCENE_WORDS: usize = 8192;

/// Header opcodes.
pub mod op {
    /// A triangle strip.
    pub const STRIP: u32 = 1;
    /// A triangle fan.
    pub const FAN: u32 = 2;
    /// Clear the frame being drawn.
    pub const CLEAR: u32 = 3;
    /// The frame being drawn is done: hand it to the display.
    pub const END_FRAME: u32 = 4;
    /// The end of the scene: start again.
    pub const END: u32 = 15;
}

/// One command of a scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Draw a strip or a fan.
    Draw(Primitive),
    /// Fill the frame being drawn with an RGB565 colour, and its depth
    /// with the farthest value, 0xFFFF.
    Clear(u16),
    /// The frame being drawn is done.
    EndFrame,
}

/// Draw commands for primitives.
///
/// # Arguments
///
/// * `prims` - The primitives.
///
/// # Returns
///
/// One draw command per primitive.
pub fn draws(prims: Vec<Primitive>) -> Vec<Command> {
    prims.into_iter().map(Command::Draw).collect()
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
/// * `cmds` - The commands, in order.
///
/// # Returns
///
/// The words, for the scene ROM.
///
/// # Panics
///
/// If the scene doesn't fit [`SCENE_WORDS`].
pub fn encode(cmds: &[Command]) -> Vec<u32> {
    let mut words = Vec::new();
    for c in cmds {
        match c {
            Command::Clear(rgb) => words.push(op::CLEAR << 28 | u32::from(*rgb)),
            Command::EndFrame => words.push(op::END_FRAME << 28),
            Command::Draw(p) => {
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
/// The commands up to END.
pub fn decode(words: &[u32]) -> Vec<Command> {
    let signed = |w: u32| {
        i64::from(i16::from_ne_bytes(
            u16::try_from(w & 0xFFFF).expect("16 bits").to_ne_bytes(),
        ))
    };
    let mut cmds = Vec::new();
    let mut i = 0;
    loop {
        let h = words[i];
        i += 1;
        let kind = match h >> 28 {
            op::STRIP => Kind::Strip,
            op::FAN => Kind::Fan,
            op::CLEAR => {
                cmds.push(Command::Clear(u16::try_from(h & 0xFFFF).expect("16 bits")));
                continue;
            }
            op::END_FRAME => {
                cmds.push(Command::EndFrame);
                continue;
            }
            _ => return cmds,
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
        cmds.push(Command::Draw(Primitive {
            kind,
            cull,
            untextured: (h >> 25) & 1 == 1,
            vertices,
        }));
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

/// Frames in the cube animation: a quarter turn, which loops seamlessly.
pub const CUBE_FRAMES: usize = 32;

/// The background colour, RGB565: dark grey.
pub const BACKGROUND: u16 = 0x2104;

/// A point projected to the screen: x and y in pixels, and the depth value.
fn project(p: [f32; 3]) -> (f32, f32, u16) {
    // Perspective from the camera at the origin looking along +z; the cube
    // sits at z = 5. Depth is 1/z mapped to 0 (near, z = 3) to 65535 (far,
    // z = 8): linear across the screen, as the rasteriser interpolates it.
    const FOCAL: f32 = 250.0;
    const NEAR: f32 = 3.0;
    const FAR: f32 = 8.0;
    let [x, y, z] = p;
    let depth = (1.0 / NEAR - 1.0 / z) / (1.0 / NEAR - 1.0 / FAR) * 65535.0;
    // The values are on screen and in range by construction.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let depth = depth.round().clamp(0.0, 65535.0) as u16;
    (210.0 + FOCAL * x / z, 180.0 - FOCAL * y / z, depth)
}

/// A vertex from a projected point, with colour `rgb`.
fn projected(p: [f32; 3], rgb: [u8; 3]) -> Vertex {
    let (x, y, z) = project(p);
    Vertex {
        z,
        ..at(f64::from(x), f64::from(y), rgb)
    }
}

/// A face of the cube: its corners in order round it, and their colours.
type Face = ([[f32; 3]; 4], [[u8; 3]; 4]);

/// One frame of the cube: its six faces, each a strip of four vertices,
/// turned by `angle` about the vertical and seen from above.
///
/// # Arguments
///
/// * `angle` - The turn, in radians.
///
/// # Returns
///
/// The faces, back faces culled.
fn cube(angle: f32) -> Vec<Primitive> {
    const TILT: f32 = -0.45;
    const SIDE: [[u8; 3]; 4] = [[255, 40, 40], [255, 230, 40], [40, 200, 80], [40, 80, 255]];
    const TOP: [u8; 3] = [230, 230, 240];
    const BOTTOM: [u8; 3] = [90, 90, 90];
    // A point of the unit cube, turned and tilted, at z = 5.
    let place = |[x, y, z]: [f32; 3]| -> [f32; 3] {
        let (s, c) = angle.sin_cos();
        let (x, z) = (c * x + s * z, -s * x + c * z);
        let (st, ct) = TILT.sin_cos();
        let (y, z) = (ct * y - st * z, st * y + ct * z);
        [x, y, z + 5.0]
    };
    // A face as its corners in order round it, and their colours.
    let mut faces: Vec<Face> = Vec::new();
    // The four sides: one side, turned a quarter at a time, so the colouring
    // is the same after a quarter turn of the cube.
    for q in 0..4 {
        let (s, c) =
            (std::f32::consts::FRAC_PI_2 * f32::from(u8::try_from(q).expect("small"))).sin_cos();
        let turn = |[x, y, z]: [f32; 3]| [c * x + s * z, y, -s * x + c * z];
        let corners = [
            [-1.0, 1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, -1.0, -1.0],
            [-1.0, -1.0, -1.0],
        ]
        .map(turn);
        faces.push((corners, SIDE));
    }
    faces.push((
        [
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
        ],
        [TOP; 4],
    ));
    faces.push((
        [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
        ],
        [BOTTOM; 4],
    ));
    faces
        .into_iter()
        .map(|(corners, colours)| {
            let world = corners.map(place);
            // Wind the face so that it faces the camera exactly when its
            // screen area is positive (the rasteriser's front).
            let centre: [f32; 3] = [0, 1, 2].map(|k| world.iter().map(|p| p[k]).sum::<f32>() / 4.0);
            let (u, v) = (sub(world[1], world[0]), sub(world[3], world[0]));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            // Outward: away from the cube's centre.
            let out = sub(centre, place([0.0, 0.0, 0.0]));
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let normal = if dot(normal, out) < 0.0 {
                normal.map(|n| -n)
            } else {
                normal
            };
            // Facing the camera (at the origin): the normal points back at it.
            let facing = dot(normal, centre) < 0.0;
            let order = [0, 1, 3, 2];
            let strip: Vec<Vertex> = order
                .iter()
                .map(|&k| projected(world[k], colours[k]))
                .collect();
            let area = twice_area(&strip[0], &strip[1], &strip[2]);
            let strip = if (area > 0.0) == facing {
                strip
            } else {
                vec![strip[1], strip[0], strip[3], strip[2]]
            };
            prim(Kind::Strip, CullMode::Back, strip)
        })
        .collect()
}

/// `a - b`.
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Twice the signed screen area of a triangle (positive clockwise on the
/// screen, as setup has it).
fn twice_area(a: &Vertex, b: &Vertex, c: &Vertex) -> f64 {
    let (ax, ay) = (a.x.to_f64(), a.y.to_f64());
    (b.x.to_f64() - ax) * (c.y.to_f64() - ay) - (c.x.to_f64() - ax) * (b.y.to_f64() - ay)
}

/// Two triangles that cross in depth, beside the cube: each is nearer at
/// one end, so the depth test cuts them along a line.
fn crossing() -> Vec<Primitive> {
    let v = |x: f64, y: f64, z: u16, rgb: [u8; 3]| Vertex { z, ..at(x, y, rgb) };
    vec![
        prim(
            Kind::Strip,
            CullMode::None,
            vec![
                v(400.0, 70.0, 8_000, [255, 120, 0]),
                v(610.0, 150.0, 56_000, [255, 220, 120]),
                v(420.0, 300.0, 8_000, [200, 40, 0]),
            ],
        ),
        prim(
            Kind::Strip,
            CullMode::None,
            vec![
                v(390.0, 160.0, 56_000, [0, 120, 255]),
                v(615.0, 60.0, 8_000, [120, 255, 255]),
                v(600.0, 310.0, 8_000, [0, 60, 200]),
            ],
        ),
    ]
}

/// One frame of the spinning cube: cleared to [`BACKGROUND`], the cube
/// turned by `k` quarter-turn steps of [`CUBE_FRAMES`], back faces culled,
/// two triangles crossing in depth beside it, then handed to the display.
///
/// # Arguments
///
/// * `k` - The frame; [`CUBE_FRAMES`] is a quarter turn.
///
/// # Returns
///
/// The frame's commands.
pub fn cube_frame(k: usize) -> Vec<Command> {
    let k = f32::from(u16::try_from(k).expect("small"));
    #[allow(clippy::cast_precision_loss)]
    let angle = std::f32::consts::FRAC_PI_2 * k / CUBE_FRAMES as f32;
    let mut cmds = vec![Command::Clear(BACKGROUND)];
    cmds.extend(draws(cube(angle)));
    cmds.extend(draws(crossing()));
    cmds.push(Command::EndFrame);
    cmds
}

/// The spinning cube (ADR 0002): [`CUBE_FRAMES`] frames of a quarter turn,
/// which loops seamlessly.
///
/// # Returns
///
/// The commands, for one loop of the animation.
pub fn cube_scene() -> Vec<Command> {
    (0..CUBE_FRAMES).flat_map(cube_frame).collect()
}
