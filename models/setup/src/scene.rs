//! Scenes as the host sends them: register transactions
//! (`docs/design/host-interface.md`, ADR 0008), built from a description as
//! strips, fans, clears and ends of frame; the built-in test card and
//! spinning cube; and [`interpret`], which turns transactions back into the
//! triangles and frame operations the GPU's command parser makes of them.
//!
//! A primitive's vertices go out as `ATTR0` (only when it changes) and a
//! vertex register each: `VERTEX_NOKICK` for the first two, then for a strip
//! `VERTEX_KICK_012` and `VERTEX_KICK_021` in turn, which keeps its winding,
//! and for a fan `VERTEX_KICK_FAN`. `RENDER_MODE` goes out when a
//! primitive's cull mode or program differs from the last. A clear is a
//! `FRAME` with both clears, an end of frame a `FRAME` with `PRESENT`, and
//! an end of frame followed by a clear is one `FRAME` with all three.

use gs_proto::{
    const_channels, reg, Attr0, CcMode, Cull, Frame as FrameReg, Kick, Position, RenderMode,
    Tex0Cfg, Transaction,
};

use crate::formats::Coord;
use crate::triangle::{CullMode, Triangle, Vertex};

/// Transactions the scene ROM holds.
pub const SCENE_ENTRIES: usize = 2048;

/// The scene ROM's end marker, in the address byte: start again from the
/// first entry. (It would be a read of `ID`, which a ROM never needs.)
pub const ROM_END: u8 = 0xFF;

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
    /// every other triangle takes them in the other order (`KICK_021`), so
    /// that all of a strip's triangles wind the same way.
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
                Kind::Strip if (i - 2).is_multiple_of(2) => [v[i - 2], v[i - 1], v[i]],
                Kind::Strip => [v[i - 2], v[i], v[i - 1]],
                Kind::Fan => [v[0], v[i - 1], v[i]],
            })
            .collect()
    }

    /// The kick each vertex is written with.
    fn kick(&self, i: usize) -> Kick {
        match self.kind {
            _ if i < 2 => Kick::None,
            Kind::Strip if (i - 2).is_multiple_of(2) => Kick::K012,
            Kind::Strip => Kick::K021,
            Kind::Fan => Kick::Fan,
        }
    }

    /// The render mode its triangles are drawn with.
    fn mode(&self) -> RenderMode {
        RenderMode {
            cull: match self.cull {
                CullMode::None => Cull::None,
                CullMode::Back => Cull::Anticlockwise,
                CullMode::Front => Cull::Clockwise,
            },
            ..RenderMode::default()
        }
    }
}

/// A vertex's `ATTR0` value.
fn attr0(v: &Vertex) -> Attr0 {
    Attr0 {
        r: v.r,
        g: v.g,
        b: v.b,
        a: v.a,
        s: v.s,
        t: v.t,
    }
}

/// A vertex's position register value.
fn position(v: &Vertex) -> Position {
    let field = |c: Coord| i16::try_from(c.to_count()).expect("a 16-bit coordinate");
    Position {
        x: field(v.x),
        y: field(v.y),
        z: v.z,
        q: v.q,
    }
}

/// A scene as transactions.
///
/// # Arguments
///
/// * `cmds` - The commands, in order.
///
/// # Returns
///
/// The transactions.
///
/// # Panics
///
/// If a primitive has fewer than three vertices.
pub fn transactions(cmds: &[Command]) -> Vec<Transaction> {
    let mut out = Vec::new();
    let mut mode: Option<RenderMode> = None;
    let mut attr: Option<Attr0> = None;
    let mut cmds = cmds.iter().peekable();
    while let Some(c) = cmds.next() {
        match c {
            Command::Clear(rgb) => out.push(frame(false, Some(*rgb))),
            Command::EndFrame => {
                // An end of frame and the next frame's clear are one write.
                if let Some(Command::Clear(rgb)) = cmds.peek() {
                    out.push(frame(true, Some(*rgb)));
                    cmds.next();
                } else {
                    out.push(frame(true, None));
                }
            }
            Command::Draw(p) => {
                assert!(
                    p.vertices.len() >= 3,
                    "a primitive needs 3 or more vertices"
                );
                if mode != Some(p.mode()) {
                    mode = Some(p.mode());
                    out.push(Transaction::write(reg::RENDER_MODE, p.mode().bits()));
                }
                for (i, v) in p.vertices.iter().enumerate() {
                    if attr != Some(attr0(v)) {
                        attr = Some(attr0(v));
                        out.push(Transaction::write(reg::ATTR0, attr0(v).bits()));
                    }
                    out.push(Transaction::write(p.kick(i).register(), position(v).bits()));
                }
            }
        }
    }
    out
}

/// A `FRAME` write.
fn frame(present: bool, clear: Option<u16>) -> Transaction {
    let f = FrameReg {
        present,
        clear_colour: clear.is_some(),
        clear_depth: clear.is_some(),
        colour: clear.unwrap_or(0),
    };
    Transaction::write(reg::FRAME, f.bits())
}

/// The scene ROM's contents: a scene's transactions, then the end marker.
///
/// # Arguments
///
/// * `cmds` - The scene, one loop of it.
///
/// # Returns
///
/// [`SCENE_ENTRIES`] (address byte, data) pairs, padded with end markers.
///
/// # Panics
///
/// If the scene doesn't fit.
pub fn rom(cmds: &[Command]) -> Vec<(u8, u64)> {
    let mut entries: Vec<(u8, u64)> = transactions(cmds)
        .into_iter()
        .map(|t| (t.addr_byte(), t.data()))
        .collect();
    assert!(
        entries.len() < SCENE_ENTRIES,
        "the scene is {} transactions",
        entries.len()
    );
    entries.resize(SCENE_ENTRIES, (ROM_END, 0));
    entries
}

/// What the command parser makes of a transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A triangle, its vertices in role order, and the state it is drawn
    /// with.
    Draw(Triangle, DrawState),

    /// A `FRAME` write.
    Frame(FrameReg),

    /// A `MEM_DATA` write: 64 bits at a dword address.
    Mem(u32, u64),
}

/// The render state a triangle is drawn with: the registers as they were at
/// its kick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawState {
    /// `RENDER_MODE`.
    pub mode: RenderMode,

    /// `TEX0_CFG`.
    pub tex: Tex0Cfg,

    /// `CC_MODE`.
    pub cc: CcMode,

    /// `CONST0`, R, G, B, A.
    pub const0: [u8; 4],

    /// `CONST1`.
    pub const1: [u8; 4],
}

impl Default for DrawState {
    /// The registers' reset values.
    fn default() -> DrawState {
        DrawState {
            mode: RenderMode::default(),
            tex: Tex0Cfg::default(),
            cc: CcMode::default(),
            const0: [255; 4],
            const1: [255; 4],
        }
    }
}

/// Interprets transactions as the GPU's command parser does: `ATTR0`,
/// `RENDER_MODE`, `TEX0_CFG`, `CC_MODE`, `CONST0` and `CONST1` are latched; a vertex takes the latched
/// `ATTR0`; a `VERTEX_NOKICK` after a kick starts a primitive; a kick draws
/// once its primitive has three vertices; `MEM_DATA` writes at `MEM_ADDR`
/// and moves it on. Reads and other registers do nothing.
///
/// # Arguments
///
/// * `txns` - The transactions.
///
/// # Returns
///
/// The triangles and frame operations, in order.
pub fn interpret(txns: &[Transaction]) -> Vec<Event> {
    let mut events = Vec::new();
    let mut attr = Attr0::default();
    let mut state = DrawState::default();
    let mut mem_addr: u32 = 0;
    // The last vertex was a kick (as after reset), the primitive's vertices
    // so far (saturating at 3), its first, and the two before this one.
    let mut kicked = true;
    let mut count = 0;
    let mut first = Vertex::default();
    let (mut back2, mut back1) = (Vertex::default(), Vertex::default());
    for t in txns {
        let Transaction::Write { addr, data } = *t else {
            continue;
        };
        if let Some(kick) = Kick::of(addr) {
            let p = Position::from_bits(data);
            let v = Vertex {
                x: Coord::from_count(i64::from(p.x)),
                y: Coord::from_count(i64::from(p.y)),
                r: attr.r,
                g: attr.g,
                b: attr.b,
                a: attr.a,
                z: p.z,
                q: p.q,
                s: attr.s,
                t: attr.t,
            };
            if kick == Kick::None && kicked {
                count = 0;
            }
            if count == 0 {
                first = v;
            }
            count = (count + 1).min(3);
            if count == 3 {
                let tri = match kick {
                    Kick::None => None,
                    Kick::K012 => Some([back2, back1, v]),
                    Kick::K021 => Some([back2, v, back1]),
                    Kick::Fan => Some([first, back1, v]),
                };
                if let Some(tri) = tri {
                    events.push(Event::Draw(tri, state));
                }
            }
            kicked = kick != Kick::None;
            back2 = back1;
            back1 = v;
        } else {
            match addr {
                reg::ATTR0 => attr = Attr0::from_bits(data),
                reg::RENDER_MODE => state.mode = RenderMode::from_bits(data),
                reg::FRAME => events.push(Event::Frame(FrameReg::from_bits(data))),
                reg::TEX0_CFG => state.tex = Tex0Cfg::from_bits(data),
                reg::CC_MODE => state.cc = CcMode::from_bits(data),
                reg::CONST0 => state.const0 = const_channels(data),
                reg::CONST1 => state.const1 = const_channels(data),
                reg::MEM_ADDR => mem_addr = u32::try_from(data & 0x3F_FFFF).unwrap_or(0),
                reg::MEM_DATA => {
                    events.push(Event::Mem(mem_addr, data));
                    mem_addr = (mem_addr + 1) & 0x3F_FFFF;
                }
                _ => {}
            }
        }
    }
    events
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
/// * `textured` - Give each face the whole texture, perspective-correct:
///   S and T from 0 to 1 across it, Q = 4 / z.
///
/// # Returns
///
/// The faces, back faces culled.
fn cube(angle: f32, textured: bool) -> Vec<Primitive> {
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
            // Texture coordinates of the corners, round the face.
            const ST: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
            let strip: Vec<Vertex> = order
                .iter()
                .map(|&k| {
                    let v = projected(world[k], colours[k]);
                    if textured {
                        let q = 4.0 / world[k][2];
                        let (s, t) = ST[k];
                        // In range by construction: Q under 2, S′ and T′ under 2.
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let fixed = |f: f32, one: f32| (f * one).round() as i32;
                        Vertex {
                            q: u16::try_from(fixed(q, 32768.0)).unwrap_or(u16::MAX),
                            s: i16::try_from(fixed(s * q, 4096.0)).unwrap_or(0),
                            t: i16::try_from(fixed(t * q, 4096.0)).unwrap_or(0),
                            ..v
                        }
                    } else {
                        v
                    }
                })
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
    cube_at(angle, BACKGROUND)
}

/// One frame of the cube at any turn, over any background: cleared, the
/// cube, the two crossing triangles, then handed to the display. What a
/// host animating it live sends.
///
/// # Arguments
///
/// * `angle` - The cube's turn about the vertical, in radians.
/// * `background` - The clear colour, RGB565.
///
/// # Returns
///
/// The frame's commands.
pub fn cube_at(angle: f32, background: u16) -> Vec<Command> {
    let mut cmds = vec![Command::Clear(background)];
    cmds.extend(draws(cube(angle, false)));
    cmds.extend(draws(crossing()));
    cmds.push(Command::EndFrame);
    cmds
}

/// log₂ of the textured cube's texture's size: 64 × 64 texels.
pub const CUBE_TEXTURE_LOG2: u8 = 6;

/// The textured cube's texture, RGB565 in raster order: light tiles with
/// dark mortar, and a diagonal stripe, so its perspective shows.
///
/// # Returns
///
/// `64 * 64` texels.
pub fn cube_texture() -> Vec<u16> {
    let size = 1_usize << CUBE_TEXTURE_LOG2;
    let rgb = |r: usize, g: usize, b: usize| {
        let c = |v: usize, bits: u32| u16::try_from(v.min(255) >> (8 - bits)).unwrap_or(0);
        c(r, 5) << 11 | c(g, 6) << 5 | c(b, 5)
    };
    (0..size * size)
        .map(|i| {
            let (u, v) = (i % size, i / size);
            let row = v / 16;
            let shifted = if row.is_multiple_of(2) { u } else { u + 8 };
            if v % 16 == 0 || shifted % 16 == 0 {
                rgb(40, 40, 48)
            } else if (u + v) % 32 < 4 {
                rgb(255, 255, 255)
            } else {
                rgb(
                    200 + (u * 3 + v * 5) % 56,
                    200 + (u * 7) % 56,
                    200 + (v * 3) % 56,
                )
            }
        })
        .collect()
}

/// A frame of the cube textured, as transactions: a clear, `TEX0_CFG` with
/// the texture, the cube (each face's texture multiplied by its colours),
/// texturing off, the two crossing triangles, and a present.
///
/// # Arguments
///
/// * `angle` - The turn, in radians.
/// * `background` - The clear colour, RGB565.
/// * `tex` - The texture's `TEX0_CFG` (enabled; [`cube_texture`] uploaded).
///
/// # Returns
///
/// The frame's transactions.
pub fn textured_cube_at(angle: f32, background: u16, tex: Tex0Cfg) -> Vec<Transaction> {
    let mut out = transactions(&[Command::Clear(background)]);
    out.push(Transaction::write(reg::TEX0_CFG, tex.bits()));
    out.extend(transactions(&draws(cube(angle, true))));
    out.push(Transaction::write(reg::TEX0_CFG, Tex0Cfg::default().bits()));
    out.extend(transactions(&draws(crossing())));
    out.extend(transactions(&[Command::EndFrame]));
    out
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
