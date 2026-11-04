//! A triangle's vertices and attributes, and the per-triangle settings.

use crate::formats::Coord;

/// One vertex, with every attribute in its native format (brief §4.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vertex {
    /// Screen x, in pixels.
    pub x: Coord,

    /// Screen y, in pixels, increasing downwards.
    pub y: Coord,

    /// Red, 0 to 255.
    pub r: u8,

    /// Green, 0 to 255.
    pub g: u8,

    /// Blue, 0 to 255.
    pub b: u8,

    /// Alpha, 0 to 255.
    pub a: u8,

    /// Depth, 0 (near) to 65535 (far).
    pub z: u16,

    /// Texture coordinate S′ = S / w, provisional format (brief D6).
    pub s: i16,

    /// Texture coordinate T′ = T / w, provisional format (brief D6).
    pub t: i16,

    /// Q ∝ 1 / w, `u0.16`, provisional format (brief D6).
    pub q: u16,
}

/// A triangle: three vertices, in the order the host sent them.
pub type Triangle = [Vertex; 3];

/// An attribute that setup computes a plane for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attr {
    /// Red.
    R,
    /// Green.
    G,
    /// Blue.
    B,
    /// Alpha.
    A,
    /// Depth.
    Z,
    /// Texture coordinate S′.
    S,
    /// Texture coordinate T′.
    T,
    /// Q.
    Q,
}

impl Attr {
    /// Every attribute, in output order.
    pub const ALL: [Attr; 8] = [
        Attr::R,
        Attr::G,
        Attr::B,
        Attr::A,
        Attr::Z,
        Attr::S,
        Attr::T,
        Attr::Q,
    ];

    /// The attribute's position in [`Attr::ALL`].
    ///
    /// # Returns
    ///
    /// 0 to 7.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The attribute's value at a vertex, as a count of its native units.
    ///
    /// # Arguments
    ///
    /// * `v` - The vertex.
    ///
    /// # Returns
    ///
    /// The raw value: 0 to 255 for colour, 0 to 65535 for Z and Q,
    /// −32768 to 32767 for S′ and T′.
    pub fn of(self, v: &Vertex) -> i64 {
        match self {
            Attr::R => v.r.into(),
            Attr::G => v.g.into(),
            Attr::B => v.b.into(),
            Attr::A => v.a.into(),
            Attr::Z => v.z.into(),
            Attr::S => v.s.into(),
            Attr::T => v.t.into(),
            Attr::Q => v.q.into(),
        }
    }
}

/// The attributes a setup program computes (its variant: brief §4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttrSet(u8);

impl AttrSet {
    /// All eight: the full program.
    pub const FULL: AttrSet = AttrSet(0xFF);

    /// Colour, alpha and depth: the untextured program.
    pub const UNTEXTURED: AttrSet = AttrSet(0x1F);

    /// Whether the set includes an attribute.
    ///
    /// # Arguments
    ///
    /// * `attr` - The attribute.
    ///
    /// # Returns
    ///
    /// `true` if setup computes its plane.
    pub fn contains(self, attr: Attr) -> bool {
        self.0 & (1 << attr.index()) != 0
    }
}

/// Which windings setup discards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CullMode {
    /// Keep both.
    None,
    /// Discard back faces: twice the signed area is negative, which with y
    /// increasing downwards is anticlockwise on the screen.
    Back,
    /// Discard front faces: area positive, clockwise on the screen.
    Front,
}
