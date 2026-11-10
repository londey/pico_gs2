//! The pico-gs2 host interface (`docs/design/host-interface.md`, ADR 0008):
//! its registers and their fields, the 72-bit transactions that read and
//! write them, and the serial link's framing and credit flow control.
//!
//! The PC tool, the testbenches, the scene generator (which writes the
//! built-in scene as transactions) and, later, the RP2350 firmware all use
//! this crate, so it is `no_std` and has no dependencies.

#![no_std]
#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]
// Register flags are bools, one per bit, as the registers have them.
#![allow(clippy::struct_excessive_bools)]

/// Register addresses (host-interface.md §3).
pub mod reg {
    /// Colour and texture coordinates for the vertices that follow.
    pub const ATTR0: u8 = 0x00;

    /// Reserved: a second colour and texture coordinate set.
    pub const ATTR1: u8 = 0x01;

    /// A vertex; draws nothing.
    pub const VERTEX_NOKICK: u8 = 0x06;

    /// A vertex; draws (two back, one back, this one).
    pub const VERTEX_KICK_012: u8 = 0x07;

    /// A vertex; draws (two back, this one, one back).
    pub const VERTEX_KICK_021: u8 = 0x08;

    /// A vertex; draws (the primitive's first, one back, this one).
    pub const VERTEX_KICK_FAN: u8 = 0x09;

    /// The texture: enable, format, size, filter, wrap modes, address.
    pub const TEX0_CFG: u8 = 0x10;

    /// The colour combiner's inputs.
    pub const CC_MODE: u8 = 0x18;

    /// A constant colour for the combiner, RGBA8888.
    pub const CONST0: u8 = 0x19;

    /// A second constant colour.
    pub const CONST1: u8 = 0x1B;

    /// Cull mode, depth test, depth and colour write enables, alpha test.
    pub const RENDER_MODE: u8 = 0x30;

    /// Present the frame, and clear.
    pub const FRAME: u8 = 0x40;

    /// The GPU memory address `MEM_DATA` reads and writes, in dwords.
    pub const MEM_ADDR: u8 = 0x70;

    /// 64 bits of GPU memory at `MEM_ADDR`, which then moves on by one.
    pub const MEM_DATA: u8 = 0x71;

    /// No effect: reads back what was last written, to test a link.
    pub const SCRATCH: u8 = 0x7D;

    /// Frames shown, the last frame's cycles, free FIFO entries, the
    /// buffers shown and drawn, the link's errors.
    pub const STATUS: u8 = 0x7E;

    /// Identification and interface version.
    pub const ID: u8 = 0x7F;
}

/// What `ID` reads: "PGS2" and interface version 2.
pub const ID_VALUE: u64 = 0x5047_5332_0002_0000;

/// The framebuffer's width in pixels.
pub const FB_WIDTH: usize = 640;

/// The framebuffer's height in pixels.
pub const FB_HEIGHT: usize = 360;

/// The 64-bit units ("dwords") in each SDRAM bank (host-interface.md §9.2).
pub const BANK_DWORDS: u32 = 0x10_0000;

/// The dwords at the start of each bank the GPU keeps for its buffers.
pub const GPU_DWORDS: u32 = 0x1_0000;

/// The first dword of the host's memory in a bank.
///
/// # Arguments
///
/// * `bank` - The bank, 0 to 3.
///
/// # Returns
///
/// Its dword address.
pub const fn host_memory(bank: u32) -> u32 {
    bank * BANK_DWORDS + GPU_DWORDS
}

/// The dword address of a colour buffer.
///
/// # Arguments
///
/// * `buffer` - The buffer, 0 to 2, as `STATUS` names it.
///
/// # Returns
///
/// Its first dword.
pub const fn colour_buffer(buffer: u32) -> u32 {
    buffer * BANK_DWORDS
}

/// The word, within a colour buffer, that holds a pixel: buffers are tiled
/// in 8 × 8 blocks (host-interface.md §9.2).
///
/// # Arguments
///
/// * `x` - The pixel's column, 0 to 639.
/// * `y` - Its line, 0 to 359.
///
/// # Returns
///
/// Its word offset from the buffer's start.
pub const fn pixel_word(x: usize, y: usize) -> usize {
    (y >> 3) * 5120 + (x >> 3) * 64 + (y & 7) * 8 + (x & 7)
}

/// The word, within a texture, that holds a texel: textures are tiled in
/// 4 × 4 blocks, blocks in raster order (host-interface.md §6.2).
///
/// # Arguments
///
/// * `u` - The texel's column.
/// * `v` - Its row.
/// * `width_log2` - log₂ of the texture's width, 2 to 10.
///
/// # Returns
///
/// Its word offset from the texture's start.
pub const fn texel_word(u: usize, v: usize, width_log2: u8) -> usize {
    ((v >> 2) << (width_log2 - 2) | u >> 2) * 16 + (v & 3) * 4 + (u & 3)
}

/// Packs four 16-bit words as `MEM_DATA` holds them: the first (lowest
/// address) in bits 15:0.
///
/// # Arguments
///
/// * `words` - The words, in address order.
///
/// # Returns
///
/// The 64-bit value.
pub fn pack_words(words: [u16; 4]) -> u64 {
    words
        .iter()
        .rev()
        .fold(0, |acc, &w| acc << 16 | u64::from(w))
}

/// Unpacks a `MEM_DATA` value into its four words, in address order.
///
/// # Arguments
///
/// * `d` - The value.
///
/// # Returns
///
/// The words.
pub fn unpack_words(d: u64) -> [u16; 4] {
    [
        unsigned16(d, 0),
        unsigned16(d, 16),
        unsigned16(d, 32),
        unsigned16(d, 48),
    ]
}

/// The transactions that write 16-bit words into GPU memory: a `MEM_ADDR`
/// write, then a `MEM_DATA` write for every four words (the last padded
/// with zeros).
///
/// # Arguments
///
/// * `dword` - The dword address to start at.
/// * `words` - The words.
///
/// # Returns
///
/// The transactions, in order.
pub fn mem_write(dword: u32, words: &[u16]) -> impl Iterator<Item = Transaction> + '_ {
    let start = core::iter::once(Transaction::write(reg::MEM_ADDR, u64::from(dword)));
    start.chain(words.chunks(4).map(|c| {
        let mut four = [0; 4];
        four[..c.len()].copy_from_slice(c);
        Transaction::write(reg::MEM_DATA, pack_words(four))
    }))
}

/// The transactions that read `n` dwords of GPU memory: a `MEM_ADDR` write,
/// then `n` reads of `MEM_DATA`, whose replies come back in order.
///
/// # Arguments
///
/// * `dword` - The dword address to start at.
/// * `n` - Dwords to read.
///
/// # Returns
///
/// The transactions, in order.
pub fn mem_read(dword: u32, n: usize) -> impl Iterator<Item = Transaction> {
    core::iter::once(Transaction::write(reg::MEM_ADDR, u64::from(dword))).chain(
        core::iter::repeat_n(
            Transaction::Read {
                addr: reg::MEM_DATA,
            },
            n,
        ),
    )
}

/// The serial link's line rate: 3 Mbaud, 8N1 (host-interface.md §10).
pub const BAUD: u32 = 3_000_000;

/// Transactions the GPU's command FIFO holds: the host's starting credits.
pub const FIFO_ENTRIES: u32 = 512;

/// Transactions each credit byte returns.
pub const CREDIT_BATCH: u32 = 64;

/// The tag byte of a credit from the GPU.
pub const TAG_CREDIT: u8 = 0x01;

/// The tag byte of a read reply from the GPU; 8 data bytes follow.
pub const TAG_REPLY: u8 = 0x02;

/// A 16-bit field as the low half of a word.
fn bits16(v: i16) -> u64 {
    u64::from(u16::from_ne_bytes(v.to_ne_bytes()))
}

/// The 16 bits of a word at `shift`, as signed.
fn signed16(w: u64, shift: u32) -> i16 {
    i16::from_ne_bytes(unsigned16(w, shift).to_ne_bytes())
}

/// The 16 bits of a word at `shift`.
fn unsigned16(w: u64, shift: u32) -> u16 {
    // Masked to 16 bits, so the conversion can't fail.
    u16::try_from((w >> shift) & 0xFFFF).unwrap_or(0)
}

/// The 8 bits of a word at `shift`.
fn byte(w: u64, shift: u32) -> u8 {
    u8::try_from((w >> shift) & 0xFF).unwrap_or(0)
}

/// `ATTR0`: the colour and texture coordinates the following vertices take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr0 {
    /// Red, 0 to 255.
    pub r: u8,

    /// Green.
    pub g: u8,

    /// Blue.
    pub b: u8,

    /// Alpha (carried, unused until blending).
    pub a: u8,

    /// S′ = S / W (carried, unused until texturing).
    pub s: i16,

    /// T′ = T / W.
    pub t: i16,
}

impl Attr0 {
    /// The register's value.
    ///
    /// # Returns
    ///
    /// R in bits 63:56, G, B, A, S′ in 31:16, T′ in 15:0.
    pub fn bits(self) -> u64 {
        u64::from(self.r) << 56
            | u64::from(self.g) << 48
            | u64::from(self.b) << 40
            | u64::from(self.a) << 32
            | bits16(self.s) << 16
            | bits16(self.t)
    }

    /// The fields of a register value.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields.
    pub fn from_bits(w: u64) -> Attr0 {
        Attr0 {
            r: byte(w, 56),
            g: byte(w, 48),
            b: byte(w, 40),
            a: byte(w, 32),
            s: signed16(w, 16),
            t: signed16(w, 0),
        }
    }
}

/// A `VERTEX_*` register's value: a vertex's position, depth and Q.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    /// X, in 1/16 pixels (Q12.4).
    pub x: i16,

    /// Y, in 1/16 pixels, increasing downwards.
    pub y: i16,

    /// Depth, 0 nearest to 0xFFFF farthest.
    pub z: u16,

    /// Q ∝ 1/W (carried, unused until texturing).
    pub q: u16,
}

impl Position {
    /// The register's value.
    ///
    /// # Returns
    ///
    /// X in bits 63:48, Y, Z, Q in 15:0.
    pub fn bits(self) -> u64 {
        bits16(self.x) << 48 | bits16(self.y) << 32 | u64::from(self.z) << 16 | u64::from(self.q)
    }

    /// The fields of a register value.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields.
    pub fn from_bits(w: u64) -> Position {
        Position {
            x: signed16(w, 48),
            y: signed16(w, 32),
            z: unsigned16(w, 16),
            q: unsigned16(w, 0),
        }
    }
}

/// Which way a kick draws its triangle, by vertex register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kick {
    /// `VERTEX_NOKICK`: no triangle.
    None,

    /// `VERTEX_KICK_012`: (two back, one back, this one).
    K012,

    /// `VERTEX_KICK_021`: (two back, this one, one back).
    K021,

    /// `VERTEX_KICK_FAN`: (the primitive's first, one back, this one).
    Fan,
}

impl Kick {
    /// The vertex register for this kick.
    ///
    /// # Returns
    ///
    /// Its address.
    pub fn register(self) -> u8 {
        match self {
            Kick::None => reg::VERTEX_NOKICK,
            Kick::K012 => reg::VERTEX_KICK_012,
            Kick::K021 => reg::VERTEX_KICK_021,
            Kick::Fan => reg::VERTEX_KICK_FAN,
        }
    }

    /// The kick a vertex register makes.
    ///
    /// # Arguments
    ///
    /// * `addr` - A register address.
    ///
    /// # Returns
    ///
    /// The kick, or `None` if `addr` isn't a vertex register.
    pub fn of(addr: u8) -> Option<Kick> {
        match addr {
            reg::VERTEX_NOKICK => Some(Kick::None),
            reg::VERTEX_KICK_012 => Some(Kick::K012),
            reg::VERTEX_KICK_021 => Some(Kick::K021),
            reg::VERTEX_KICK_FAN => Some(Kick::Fan),
            _ => None,
        }
    }
}

/// Which winding setup discards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cull {
    /// Keep both.
    #[default]
    None,

    /// Discard triangles anticlockwise on the screen (setup's back faces).
    Anticlockwise,

    /// Discard triangles clockwise on the screen.
    Clockwise,
}

/// The alpha test: which pixels it passes, by the combiner's alpha against
/// `ALPHA_REF`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaTest {
    /// Every pixel.
    #[default]
    Always,

    /// Alpha below the reference.
    Less,

    /// Alpha at or above the reference: cut-outs.
    GEqual,

    /// Alpha other than the reference.
    NotEqual,
}

impl AlphaTest {
    /// Whether a pixel passes.
    ///
    /// # Arguments
    ///
    /// * `alpha` - The combiner's alpha.
    /// * `reference` - `ALPHA_REF`.
    ///
    /// # Returns
    ///
    /// `true` if it is drawn.
    pub fn passes(self, alpha: u8, reference: u8) -> bool {
        match self {
            AlphaTest::Always => true,
            AlphaTest::Less => alpha < reference,
            AlphaTest::GEqual => alpha >= reference,
            AlphaTest::NotEqual => alpha != reference,
        }
    }
}

/// `RENDER_MODE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderMode {
    /// The alpha test's reference.
    pub alpha_ref: u8,

    /// The alpha test.
    pub alpha_test: AlphaTest,

    /// Which winding to discard.
    pub cull: Cull,

    /// Write the colour buffer.
    pub colour_write: bool,

    /// Write the depth buffer where a pixel is drawn.
    pub z_write: bool,

    /// Draw a pixel only if it is nearer than the depth buffer's.
    pub z_test: bool,
}

impl Default for RenderMode {
    /// The reset value: no culling, depth tested and written, colour
    /// written.
    fn default() -> RenderMode {
        RenderMode {
            alpha_ref: 0,
            alpha_test: AlphaTest::Always,
            cull: Cull::None,
            colour_write: true,
            z_write: true,
            z_test: true,
        }
    }
}

impl RenderMode {
    /// The register's value.
    ///
    /// # Returns
    ///
    /// `ALPHA_REF` in 26:19, `ALPHA_TEST` 18:17, `CULL` 6:5, `COLOR_WRITE`
    /// 4, `Z_WRITE` 3, `Z_TEST` 2.
    pub fn bits(self) -> u64 {
        let cull = match self.cull {
            Cull::None => 0,
            Cull::Anticlockwise => 1,
            Cull::Clockwise => 2,
        };
        let test = match self.alpha_test {
            AlphaTest::Always => 0,
            AlphaTest::Less => 1,
            AlphaTest::GEqual => 2,
            AlphaTest::NotEqual => 3,
        };
        u64::from(self.alpha_ref) << 19
            | test << 17
            | cull << 5
            | u64::from(self.colour_write) << 4
            | u64::from(self.z_write) << 3
            | u64::from(self.z_test) << 2
    }

    /// The fields of a register value.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields; a `CULL` of 3 is none.
    pub fn from_bits(w: u64) -> RenderMode {
        RenderMode {
            alpha_ref: byte(w, 19),
            alpha_test: match w >> 17 & 3 {
                1 => AlphaTest::Less,
                2 => AlphaTest::GEqual,
                3 => AlphaTest::NotEqual,
                _ => AlphaTest::Always,
            },
            cull: match w >> 5 & 3 {
                1 => Cull::Anticlockwise,
                2 => Cull::Clockwise,
                _ => Cull::None,
            },
            colour_write: w >> 4 & 1 == 1,
            z_write: w >> 3 & 1 == 1,
            z_test: w >> 2 & 1 == 1,
        }
    }
}

/// A colour combiner input (host-interface.md §7), at pico-gs's codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CcSource {
    /// The texel (opaque white if the triangle isn't textured).
    Tex0,

    /// The interpolated vertex colour.
    Shade0,

    /// `CONST0`.
    Const0,

    /// `CONST1`.
    Const1,

    /// 1 (255).
    One,

    /// 0.
    Zero,

    /// The texel's alpha, in each colour channel (`RGB_C` only).
    Tex0Alpha,

    /// The vertex colour's alpha, in each colour channel (`RGB_C` only).
    Shade0Alpha,

    /// `CONST0`'s alpha, in each colour channel (`RGB_C` only).
    Const0Alpha,

    /// A code with no source here (reserved, or pico-gs's second pass,
    /// second texture or destination colour): reads 0.
    Reserved(u8),
}

impl CcSource {
    /// The source's code.
    pub fn code(self) -> u64 {
        match self {
            CcSource::Tex0 => 1,
            CcSource::Shade0 => 3,
            CcSource::Const0 => 4,
            CcSource::Const1 => 5,
            CcSource::One => 6,
            CcSource::Zero => 7,
            CcSource::Tex0Alpha => 8,
            CcSource::Shade0Alpha => 10,
            CcSource::Const0Alpha => 11,
            CcSource::Reserved(c) => u64::from(c & 0xF),
        }
    }

    /// The source a code picks.
    ///
    /// # Arguments
    ///
    /// * `code` - The 4-bit field.
    /// * `alpha_broadcasts` - The field is `RGB_C`, where codes 8, 10 and
    ///   11 broadcast an alpha.
    ///
    /// # Returns
    ///
    /// The source.
    pub fn from_code(code: u64, alpha_broadcasts: bool) -> CcSource {
        match (code & 0xF, alpha_broadcasts) {
            (1, _) => CcSource::Tex0,
            (3, _) => CcSource::Shade0,
            (4, _) => CcSource::Const0,
            (5, _) => CcSource::Const1,
            (6, _) => CcSource::One,
            (7, _) => CcSource::Zero,
            (8, true) => CcSource::Tex0Alpha,
            (10, true) => CcSource::Shade0Alpha,
            (11, true) => CcSource::Const0Alpha,
            (c, _) => CcSource::Reserved(u8::try_from(c).unwrap_or(0)),
        }
    }
}

/// `CC_MODE`: the combiner's `(A − B) × C + D`, for colour and for alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcMode {
    /// Colour's A, B, C and D.
    pub rgb: [CcSource; 4],

    /// Alpha's A, B, C and D.
    pub alpha: [CcSource; 4],
}

impl CcMode {
    /// Texture × shade, colour and alpha: the reset value.
    pub const MODULATE: CcMode = CcMode::same([
        CcSource::Tex0,
        CcSource::Zero,
        CcSource::Shade0,
        CcSource::Zero,
    ]);

    /// The texture alone.
    pub const DECAL: CcMode = CcMode::same([
        CcSource::Zero,
        CcSource::Zero,
        CcSource::Zero,
        CcSource::Tex0,
    ]);

    /// The shade alone.
    pub const SHADE: CcMode = CcMode::same([
        CcSource::Zero,
        CcSource::Zero,
        CcSource::Zero,
        CcSource::Shade0,
    ]);

    /// The texture × `CONST0`.
    pub const TINT: CcMode = CcMode::same([
        CcSource::Tex0,
        CcSource::Zero,
        CcSource::Const0,
        CcSource::Zero,
    ]);

    /// The texture + shade (a cheap specular), alpha modulated.
    pub const ADD_SHADE: CcMode = CcMode {
        rgb: [
            CcSource::Tex0,
            CcSource::Zero,
            CcSource::One,
            CcSource::Shade0,
        ],
        alpha: [
            CcSource::Tex0,
            CcSource::Zero,
            CcSource::Shade0,
            CcSource::Zero,
        ],
    };

    /// From the texture to `CONST1` as the shade's alpha falls (fog);
    /// alpha the texture's.
    pub const FADE: CcMode = CcMode {
        rgb: [
            CcSource::Tex0,
            CcSource::Const1,
            CcSource::Shade0Alpha,
            CcSource::Const1,
        ],
        alpha: [
            CcSource::Tex0,
            CcSource::Zero,
            CcSource::One,
            CcSource::Zero,
        ],
    };

    /// The same inputs for colour and alpha.
    const fn same(abcd: [CcSource; 4]) -> CcMode {
        CcMode {
            rgb: abcd,
            alpha: abcd,
        }
    }

    /// The register's value: pass 1 (bits 63:32) zero.
    ///
    /// # Returns
    ///
    /// Alpha's D, C, B, A in bits 31:16, colour's in 15:0, A lowest.
    pub fn bits(self) -> u64 {
        let nibbles = |abcd: [CcSource; 4]| {
            abcd.iter()
                .enumerate()
                .fold(0, |acc, (i, s)| acc | s.code() << (4 * i))
        };
        nibbles(self.alpha) << 16 | nibbles(self.rgb)
    }

    /// The inputs a register value picks.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The inputs (pass 1 ignored).
    pub fn from_bits(w: u64) -> CcMode {
        let field = |k: u32| w >> (4 * k) & 0xF;
        CcMode {
            rgb: [0, 1, 2, 3].map(|k| CcSource::from_code(field(k), k == 2)),
            alpha: [4, 5, 6, 7].map(|k| CcSource::from_code(field(k), false)),
        }
    }
}

impl Default for CcMode {
    /// [`CcMode::MODULATE`].
    fn default() -> CcMode {
        CcMode::MODULATE
    }
}

/// A `CONST0` or `CONST1` value: R, G, B and A, R in bits 31:24.
///
/// # Arguments
///
/// * `rgba` - The channels.
///
/// # Returns
///
/// The register's value.
pub fn const_colour(rgba: [u8; 4]) -> u64 {
    rgba.iter().fold(0, |acc, &c| acc << 8 | u64::from(c))
}

/// A `CONST0` or `CONST1` value's channels.
///
/// # Arguments
///
/// * `w` - The value.
///
/// # Returns
///
/// R, G, B and A.
pub fn const_channels(w: u64) -> [u8; 4] {
    [byte(w, 24), byte(w, 16), byte(w, 8), byte(w, 0)]
}

/// How a texture coordinate outside the texture is brought back into it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    /// Repeat the texture.
    #[default]
    Repeat,

    /// Clamp to its edge texels.
    ClampToEdge,

    /// Repeat it, every other repeat reflected.
    Mirror,
}

impl Wrap {
    /// The field's value.
    fn bits(self) -> u64 {
        match self {
            Wrap::Repeat => 0,
            Wrap::ClampToEdge => 1,
            Wrap::Mirror => 2,
        }
    }

    /// The mode a field's value picks; the reserved 3 is `Repeat`.
    fn from_bits(w: u64) -> Wrap {
        match w & 3 {
            1 => Wrap::ClampToEdge,
            2 => Wrap::Mirror,
            _ => Wrap::Repeat,
        }
    }
}

/// A texel format (host-interface.md §6.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TexFormat {
    /// R[15:11] G[10:5] B[4:0], opaque.
    #[default]
    Rgb565,

    /// A[15:12] R[11:8] G[7:4] B[3:0].
    Argb4444,
}

/// `TEX0_CFG`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tex0Cfg {
    /// Texture the triangles that follow.
    pub enable: bool,

    /// Bilinear filtering (Phase 18); else nearest.
    pub bilinear: bool,

    /// The texel format.
    pub format: TexFormat,

    /// The width is `2^width_log2` texels, 2 to 10.
    pub width_log2: u8,

    /// The height is `2^height_log2` texels, 2 to 10.
    pub height_log2: u8,

    /// The wrap mode for S.
    pub u_wrap: Wrap,

    /// The wrap mode for T.
    pub v_wrap: Wrap,

    /// The texture's address in units of 256 words (512 bytes).
    pub base: u16,
}

impl Tex0Cfg {
    /// The register's value.
    ///
    /// # Returns
    ///
    /// `BASE_ADDR` in bits 47:32, `V_WRAP` 19:18, `U_WRAP` 17:16,
    /// `HEIGHT_LOG2` 15:12, `WIDTH_LOG2` 11:8, `FORMAT` 7:4, `FILTER` 3:2,
    /// `ENABLE` 0.
    pub fn bits(self) -> u64 {
        u64::from(self.base) << 32
            | self.v_wrap.bits() << 18
            | self.u_wrap.bits() << 16
            | u64::from(self.height_log2 & 0xF) << 12
            | u64::from(self.width_log2 & 0xF) << 8
            | u64::from(self.format == TexFormat::Argb4444) << 4
            | u64::from(self.bilinear) << 2
            | u64::from(self.enable)
    }

    /// The fields of a register value, as the GPU takes them: a size
    /// below 4 texels is 4 and above 1024 is 1024; a reserved format is
    /// RGB565, a reserved filter nearest.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields.
    pub fn from_bits(w: u64) -> Tex0Cfg {
        let log2 = |shift: u32| (byte(w, shift) & 0xF).clamp(2, 10);
        Tex0Cfg {
            enable: w & 1 == 1,
            bilinear: w >> 2 & 3 == 1,
            format: if w >> 4 & 0xF == 1 {
                TexFormat::Argb4444
            } else {
                TexFormat::Rgb565
            },
            width_log2: log2(8),
            height_log2: log2(12),
            u_wrap: Wrap::from_bits(w >> 16),
            v_wrap: Wrap::from_bits(w >> 18),
            base: unsigned16(w, 32),
        }
    }
}

/// `FRAME`: present the frame being drawn, and clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// The frame being drawn is done: show it from the next frame start,
    /// and draw into the next buffer.
    pub present: bool,

    /// Fill the buffer being drawn (after any present) with `colour`.
    pub clear_colour: bool,

    /// Set its depths to the farthest, 0xFFFF.
    pub clear_depth: bool,

    /// The clear colour, RGB565.
    pub colour: u16,
}

impl Frame {
    /// The register's value.
    ///
    /// # Returns
    ///
    /// The colour in bits 31:16, `CLEAR_DEPTH` 2, `CLEAR_COLOUR` 1,
    /// `PRESENT` 0.
    pub fn bits(self) -> u64 {
        u64::from(self.colour) << 16
            | u64::from(self.clear_depth) << 2
            | u64::from(self.clear_colour) << 1
            | u64::from(self.present)
    }

    /// The fields of a register value.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields.
    pub fn from_bits(w: u64) -> Frame {
        Frame {
            present: w & 1 == 1,
            clear_colour: w >> 1 & 1 == 1,
            clear_depth: w >> 2 & 1 == 1,
            colour: unsigned16(w, 16),
        }
    }
}

/// `STATUS`, as read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The colour buffer being drawn, 0 to 2.
    pub drawn: u8,

    /// The colour buffer on screen, 0 to 2.
    pub shown: u8,

    /// A byte arrived malformed (a low stop bit) since the link reset.
    pub link_malformed: bool,

    /// A transaction arrived with the FIFO full since the link reset.
    pub link_overflow: bool,

    /// Free command FIFO entries.
    pub fifo_free: u16,

    /// Core cycles the last finished frame took to draw.
    pub render_cycles: u32,

    /// Frames shown since reset, wrapping.
    pub frames: u16,
}

impl Status {
    /// The fields of a register value.
    ///
    /// # Arguments
    ///
    /// * `w` - The value.
    ///
    /// # Returns
    ///
    /// The fields.
    pub fn from_bits(w: u64) -> Status {
        Status {
            drawn: byte(w, 54) & 3,
            shown: byte(w, 50) & 3,
            link_malformed: w >> 53 & 1 == 1,
            link_overflow: w >> 52 & 1 == 1,
            fifo_free: unsigned16(w, 40) & 0x3FF,
            render_cycles: u32::try_from(w >> 16 & 0xFF_FFFF).unwrap_or(0),
            frames: unsigned16(w, 0),
        }
    }

    /// The register's value.
    ///
    /// # Returns
    ///
    /// The buffer drawn in bits 55:54, shown in 51:50, the link's errors
    /// in 53:52, free entries in 49:40, cycles in 39:16, frames in 15:0.
    pub fn bits(self) -> u64 {
        u64::from(self.drawn & 3) << 54
            | u64::from(self.link_malformed) << 53
            | u64::from(self.link_overflow) << 52
            | u64::from(self.shown & 3) << 50
            | u64::from(self.fifo_free & 0x3FF) << 40
            | u64::from(self.render_cycles & 0xFF_FFFF) << 16
            | u64::from(self.frames)
    }
}

/// One transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transaction {
    /// Write `data` to the register at `addr`.
    Write {
        /// The register's address, 0 to 0x7F.
        addr: u8,

        /// The value.
        data: u64,
    },

    /// Read the register at `addr`.
    Read {
        /// The register's address, 0 to 0x7F.
        addr: u8,
    },
}

impl Transaction {
    /// A write.
    ///
    /// # Arguments
    ///
    /// * `addr` - The register's address.
    /// * `data` - The value.
    ///
    /// # Returns
    ///
    /// The transaction.
    pub fn write(addr: u8, data: u64) -> Transaction {
        Transaction::Write { addr, data }
    }

    /// Its bytes on the serial link: a write is the address byte (bit 7
    /// clear) and the data, MSB first; a read is the address byte with bit
    /// 7 set.
    ///
    /// # Arguments
    ///
    /// * `out` - Where to put them.
    ///
    /// # Returns
    ///
    /// How many there are: 9 or 1.
    pub fn encode(self, out: &mut [u8; 9]) -> usize {
        match self {
            Transaction::Write { addr, data } => {
                out[0] = addr & 0x7F;
                out[1..].copy_from_slice(&data.to_be_bytes());
                9
            }
            Transaction::Read { addr } => {
                out[0] = 0x80 | (addr & 0x7F);
                1
            }
        }
    }

    /// Its 8-bit address field as the scene ROM stores it: bit 7 set for a
    /// read.
    ///
    /// # Returns
    ///
    /// The address byte.
    pub fn addr_byte(self) -> u8 {
        match self {
            Transaction::Write { addr, .. } => addr & 0x7F,
            Transaction::Read { addr } => 0x80 | (addr & 0x7F),
        }
    }

    /// Its data, 0 for a read.
    ///
    /// # Returns
    ///
    /// The data.
    pub fn data(self) -> u64 {
        match self {
            Transaction::Write { data, .. } => data,
            Transaction::Read { .. } => 0,
        }
    }
}

/// Something the GPU sent back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// [`CREDIT_BATCH`] more transactions have been taken from the FIFO.
    Credit,

    /// A read's value.
    Data(u64),
}

/// Turns the bytes the GPU sends into [`Reply`]s.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplyDecoder {
    /// Data bytes of a reply still to come (0 between messages).
    remaining: u8,

    /// The reply's data so far.
    data: u64,
}

/// A byte the GPU sent that starts no message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownTag(pub u8);

impl ReplyDecoder {
    /// Takes one byte.
    ///
    /// # Arguments
    ///
    /// * `b` - The byte.
    ///
    /// # Returns
    ///
    /// A reply, if the byte completes one.
    ///
    /// # Errors
    ///
    /// [`UnknownTag`] for a byte between messages that isn't a tag.
    pub fn push(&mut self, b: u8) -> Result<Option<Reply>, UnknownTag> {
        if self.remaining > 0 {
            self.data = self.data << 8 | u64::from(b);
            self.remaining -= 1;
            return Ok((self.remaining == 0).then_some(Reply::Data(self.data)));
        }
        match b {
            TAG_CREDIT => Ok(Some(Reply::Credit)),
            TAG_REPLY => {
                self.remaining = 8;
                self.data = 0;
                Ok(None)
            }
            _ => Err(UnknownTag(b)),
        }
    }
}

/// The host's count of the transactions it may still send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credits {
    /// Transactions the FIFO has room for, as far as the host knows.
    available: u32,
}

impl Default for Credits {
    /// A fresh link: the whole FIFO.
    fn default() -> Credits {
        Credits {
            available: FIFO_ENTRIES,
        }
    }
}

impl Credits {
    /// Transactions that may be sent now.
    ///
    /// # Returns
    ///
    /// The count.
    pub fn available(self) -> u32 {
        self.available
    }

    /// Spends a credit on a transaction, if there is one.
    ///
    /// # Returns
    ///
    /// Whether the transaction may be sent.
    pub fn spend(&mut self) -> bool {
        if self.available == 0 {
            return false;
        }
        self.available -= 1;
        true
    }

    /// Takes a credit byte: [`CREDIT_BATCH`] more transactions.
    pub fn grant(&mut self) {
        self.available = (self.available + CREDIT_BATCH).min(FIFO_ENTRIES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Register values decode to what was encoded.
    #[test]
    fn fields_round_trip() {
        let a = Attr0 {
            r: 1,
            g: 2,
            b: 250,
            a: 255,
            s: -3,
            t: 32767,
        };
        assert_eq!(Attr0::from_bits(a.bits()), a);
        let p = Position {
            x: -16,
            y: 5000,
            z: 0xFFFF,
            q: 0x8000,
        };
        assert_eq!(Position::from_bits(p.bits()), p);
        assert_eq!(p.bits() >> 48, 0xFFF0);
        for cull in [Cull::None, Cull::Anticlockwise, Cull::Clockwise] {
            let m = RenderMode {
                cull,
                colour_write: false,
                z_write: true,
                z_test: false,
                ..RenderMode::default()
            };
            assert_eq!(RenderMode::from_bits(m.bits()), m);
        }
        assert_eq!(RenderMode::default().bits(), 0x1C);
        let f = Frame {
            present: true,
            clear_colour: false,
            clear_depth: true,
            colour: 0x2104,
        };
        assert_eq!(Frame::from_bits(f.bits()), f);
        let s = Status {
            drawn: 2,
            shown: 1,
            link_malformed: true,
            link_overflow: false,
            fifo_free: 512,
            render_cycles: 1_103_542,
            frames: 7,
        };
        assert_eq!(Status::from_bits(s.bits()), s);
        assert_eq!(s.bits() >> 50, 0b10_10_01);
    }

    /// `CC_MODE`'s presets have the values the interface document gives;
    /// `RENDER_MODE`'s alpha test is at pico-gs's bits.
    #[test]
    fn combiner_and_alpha_test_fields() {
        assert_eq!(CcMode::MODULATE.bits(), 0x7371_7371);
        assert_eq!(CcMode::DECAL.bits(), 0x1777_1777);
        assert_eq!(CcMode::SHADE.bits(), 0x3777_3777);
        assert_eq!(CcMode::TINT.bits(), 0x7471_7471);
        assert_eq!(CcMode::ADD_SHADE.bits(), 0x7371_3671);
        assert_eq!(CcMode::FADE.bits(), 0x7671_5A51);
        for m in [CcMode::MODULATE, CcMode::ADD_SHADE, CcMode::FADE] {
            assert_eq!(CcMode::from_bits(m.bits()), m);
        }
        // Codes 8, 10 and 11 broadcast an alpha only in RGB_C.
        assert_eq!(CcMode::from_bits(0x8).rgb[0], CcSource::Reserved(8));
        assert_eq!(CcMode::from_bits(0x800).rgb[2], CcSource::Tex0Alpha);
        let m = RenderMode {
            alpha_test: AlphaTest::GEqual,
            alpha_ref: 0x80,
            ..RenderMode::default()
        };
        assert_eq!(m.bits(), 0x80 << 19 | 2 << 17 | 0x1C);
        assert_eq!(RenderMode::from_bits(m.bits()), m);
        assert_eq!(const_channels(const_colour([1, 2, 3, 4])), [1, 2, 3, 4]);
        assert_eq!(const_colour([0xFF; 4]), 0xFFFF_FFFF);
    }

    /// `TEX0_CFG`'s fields at pico-gs's positions; sizes clamped to 4 to
    /// 1024 texels.
    #[test]
    fn tex0_cfg_fields() {
        let t = Tex0Cfg {
            enable: true,
            bilinear: false,
            format: TexFormat::Argb4444,
            width_log2: 6,
            height_log2: 10,
            u_wrap: Wrap::Mirror,
            v_wrap: Wrap::ClampToEdge,
            base: 0x1234,
        };
        assert_eq!(t.bits(), 0x1234_0006_A611);
        assert_eq!(Tex0Cfg::from_bits(t.bits()), t);
        let small = Tex0Cfg::from_bits(0x1F00 | 3 << 18);
        assert_eq!((small.width_log2, small.height_log2), (10, 2));
        assert_eq!(small.v_wrap, Wrap::Repeat);
    }

    /// Memory words pack with the first in the low bits; a write is
    /// `MEM_ADDR` then a `MEM_DATA` per four words, the last padded.
    #[test]
    fn memory_words() {
        let d = pack_words([1, 2, 3, 4]);
        assert_eq!(d, 0x0004_0003_0002_0001);
        assert_eq!(unpack_words(d), [1, 2, 3, 4]);
        let mut w = mem_write(host_memory(1), &[1, 2, 3, 4, 5]);
        assert_eq!(w.next(), Some(Transaction::write(reg::MEM_ADDR, 0x11_0000)));
        assert_eq!(w.next(), Some(Transaction::write(reg::MEM_DATA, d)));
        assert_eq!(w.next(), Some(Transaction::write(reg::MEM_DATA, 5)));
        assert_eq!(w.next(), None);
        assert_eq!(mem_read(colour_buffer(2), 3).count(), 4);
    }

    /// Texels are tiled in 4 × 4 blocks, blocks across the texture.
    #[test]
    fn texels_are_tiled() {
        assert_eq!(texel_word(0, 0, 5), 0);
        assert_eq!(texel_word(3, 1, 5), 7);
        assert_eq!(texel_word(4, 0, 5), 16);
        assert_eq!(texel_word(0, 4, 5), 8 * 16);
        assert_eq!(texel_word(5, 6, 5), 9 * 16 + 2 * 4 + 1);
    }

    /// Pixels are tiled in 8 × 8 blocks: raster order within a block,
    /// blocks across, block rows of 5,120 words.
    #[test]
    fn pixels_are_tiled() {
        assert_eq!(pixel_word(0, 0), 0);
        assert_eq!(pixel_word(7, 0), 7);
        assert_eq!(pixel_word(0, 1), 8);
        assert_eq!(pixel_word(8, 0), 64);
        assert_eq!(pixel_word(0, 8), 5120);
        assert_eq!(pixel_word(639, 359), 44 * 5120 + 79 * 64 + 63);
    }

    /// A write is 9 bytes, MSB first; a read is one byte with bit 7 set.
    #[test]
    fn transactions_encode() {
        let mut b = [0; 9];
        let n = Transaction::write(reg::FRAME, 0x0102_0304_0506_0708).encode(&mut b);
        assert_eq!(n, 9);
        assert_eq!(b, [0x40, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(Transaction::Read { addr: reg::ID }.encode(&mut b), 1);
        assert_eq!(b[0], 0xFF);
    }

    /// Credit bytes and replies decode, interleaved at message boundaries.
    #[test]
    fn replies_decode() {
        let mut d = ReplyDecoder::default();
        assert_eq!(d.push(TAG_CREDIT), Ok(Some(Reply::Credit)));
        assert_eq!(d.push(TAG_REPLY), Ok(None));
        let mut last = None;
        for b in ID_VALUE.to_be_bytes() {
            last = d.push(b).expect("data");
        }
        assert_eq!(last, Some(Reply::Data(ID_VALUE)));
        assert_eq!(d.push(0x55), Err(UnknownTag(0x55)));
    }

    /// The host never has more credits than the FIFO has entries.
    #[test]
    fn credits_are_bounded() {
        let mut c = Credits::default();
        for _ in 0..FIFO_ENTRIES {
            assert!(c.spend());
        }
        assert!(!c.spend());
        c.grant();
        assert_eq!(c.available(), CREDIT_BATCH);
        for _ in 0..20 {
            c.grant();
        }
        assert_eq!(c.available(), FIFO_ENTRIES);
    }
}
