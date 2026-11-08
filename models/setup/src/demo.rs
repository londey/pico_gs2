//! A demonstration of the colour combiner and alpha test
//! (`docs/design/host-interface.md` §7, §8), as register transactions: the
//! twin test `combiner.rs` checks it pixel for pixel, and `gs-host
//! --combiner` shows it on the board.

use gs_proto::{
    const_colour, host_memory, mem_write, reg, texel_word, AlphaTest, Attr0, CcMode, CcSource,
    Frame as FrameReg, Position, RenderMode, Tex0Cfg, TexFormat, Transaction,
};

/// The background colour, RGB565: dark blue.
const HOST_BACKGROUND: u16 = 0x0010;

/// A 16 × 16 texture's upload and its `TEX0_CFG`.
fn texture(
    dword: u32,
    format: TexFormat,
    texel: impl Fn(usize, usize) -> u16,
) -> (Vec<Transaction>, Transaction) {
    let mut words = vec![0; 256];
    for v in 0..16 {
        for u in 0..16 {
            words[texel_word(u, v, 4)] = texel(u, v);
        }
    }
    let cfg = Tex0Cfg {
        enable: true,
        format,
        width_log2: 4,
        height_log2: 4,
        base: u16::try_from(dword / 64).unwrap_or(0),
        ..Tex0Cfg::default()
    };
    (
        mem_write(dword, &words).collect(),
        Transaction::write(reg::TEX0_CFG, cfg.bits()),
    )
}

/// A quad as a strip: its top left corner and size in pixels, its depth,
/// and its four corners' colours (RGBA: top left, top right, bottom left,
/// bottom right); S and T from 0 to 1 across it.
fn quad(x: i16, y: i16, w: i16, h: i16, z: u16, rgba: [[u8; 4]; 4]) -> Vec<Transaction> {
    let corner = |k: usize, register: u8| {
        let (cx, cy) = (k % 2, k / 2);
        let attr = Attr0 {
            r: rgba[k][0],
            g: rgba[k][1],
            b: rgba[k][2],
            a: rgba[k][3],
            s: if cx == 1 { 4096 } else { 0 },
            t: if cy == 1 { 4096 } else { 0 },
        };
        let pos = Position {
            x: (x + if cx == 1 { w } else { 0 }) * 16,
            y: (y + if cy == 1 { h } else { 0 }) * 16,
            z,
            q: 0x8000,
        };
        [
            Transaction::write(reg::ATTR0, attr.bits()),
            Transaction::write(register, pos.bits()),
        ]
    };
    [
        corner(0, reg::VERTEX_NOKICK),
        corner(1, reg::VERTEX_NOKICK),
        corner(2, reg::VERTEX_KICK_012),
        corner(3, reg::VERTEX_KICK_021),
    ]
    .concat()
}

/// Corner colours varying in every channel, alpha included.
const VARIED: [[u8; 4]; 4] = [
    [255, 255, 255, 255],
    [255, 64, 0, 192],
    [0, 128, 255, 64],
    [128, 255, 64, 0],
];

/// A `FRAME` write: clear, and present if asked.
fn frame(present: bool) -> Transaction {
    Transaction::write(
        reg::FRAME,
        FrameReg {
            present,
            clear_colour: true,
            clear_depth: true,
            colour: HOST_BACKGROUND,
        }
        .bits(),
    )
}

/// A `CC_MODE` write.
fn cc(m: CcMode) -> Transaction {
    Transaction::write(reg::CC_MODE, m.bits())
}

/// A `RENDER_MODE` write with an alpha test.
fn alpha(test: AlphaTest, reference: u8) -> Transaction {
    Transaction::write(
        reg::RENDER_MODE,
        RenderMode {
            alpha_test: test,
            alpha_ref: reference,
            ..RenderMode::default()
        }
        .bits(),
    )
}

/// The combiner and alpha test demonstration: two 16 × 16 textures
/// uploaded (RGB565, and ARGB4444 whose alpha rises across it), then three
/// frames:
///
/// 1. Nine quads, each with its own `CC_MODE`: the six presets, a
///    combination using every alpha broadcast and a negative `A − B`,
///    reserved codes (which read 0); and an untextured quad tinted by
///    `CONST0`'s alpha (the texel reads as white).
/// 2. The four alpha tests on the alpha-ramp texture, and a cut-out drawn
///    in front of a later, farther triangle that shows through its holes.
/// 3. The nine quads again with the alpha test passing alpha ≥ 96, so
///    each combiner setting's alpha shows as a cut-out.
///
/// # Returns
///
/// The transactions; each frame ends with a present.
pub fn combiner_demo() -> Vec<Transaction> {
    let n = |x: usize| u16::try_from(x & 0xF).unwrap_or(0);
    let (rgb_up, rgb) = texture(host_memory(0), TexFormat::Rgb565, |u, v| {
        n(u) << 12 | n(v) << 7 | n(u ^ v) << 1
    });
    // Alpha rises left to right, 0 to 15.
    let (ramp_up, ramp) = texture(host_memory(1), TexFormat::Argb4444, |u, v| {
        n(u) << 12 | n(15 - v) << 8 | n(v) << 4 | 0x8
    });
    let mut t = Vec::new();
    // The nine quads, each with its combiner setting and texture.
    let nine = |t: &mut Vec<Transaction>, modes: &[(CcMode, Transaction)]| {
        for (k, (m, tex)) in modes.iter().enumerate() {
            let (col, row) = (
                i16::try_from(k % 3).unwrap_or(0),
                i16::try_from(k / 3).unwrap_or(0),
            );
            t.push(cc(*m));
            t.push(*tex);
            t.extend(quad(20 + col * 200, 15 + row * 115, 180, 100, 1000, VARIED));
        }
    };
    t.extend(rgb_up);
    t.extend(ramp_up);
    t.push(frame(false));
    t.push(Transaction::write(
        reg::CONST0,
        const_colour([255, 128, 0, 160]),
    ));
    t.push(Transaction::write(
        reg::CONST1,
        const_colour([40, 200, 120, 0]),
    ));

    // 1. Nine combiner settings.
    use CcSource as S;
    let custom = CcMode {
        // (CONST0 - CONST1) x texel alpha + shade: negative in green.
        rgb: [S::Const0, S::Const1, S::Tex0Alpha, S::Shade0],
        // (shade - texel) x CONST0, the alpha going negative where the
        // texel is more opaque than the shade.
        alpha: [S::Shade0, S::Tex0, S::Const0, S::Zero],
    };
    let broadcasts = CcMode {
        rgb: [S::Tex0, S::Zero, S::Shade0Alpha, S::Zero],
        alpha: [S::One, S::Zero, S::Const0, S::Zero],
    };
    let reserved = CcMode {
        rgb: [S::Reserved(2), S::Zero, S::One, S::Reserved(9)],
        alpha: [S::Reserved(12), S::Zero, S::One, S::One],
    };
    let tint_const_alpha = CcMode {
        rgb: [S::Tex0, S::Zero, S::Const0Alpha, S::Zero],
        alpha: [S::Tex0, S::Zero, S::One, S::Zero],
    };
    let modes = [
        (CcMode::MODULATE, rgb),
        (CcMode::DECAL, rgb),
        (CcMode::SHADE, rgb),
        (CcMode::TINT, rgb),
        (CcMode::ADD_SHADE, ramp),
        (CcMode::FADE, ramp),
        (custom, ramp),
        (broadcasts, ramp),
        (reserved, rgb),
    ];
    nine(&mut t, &modes);
    // An untextured quad: the texel reads as white, so TINT is CONST0, and
    // this one CONST0's alpha.
    t.push(Transaction::write(reg::TEX0_CFG, Tex0Cfg::default().bits()));
    t.push(cc(tint_const_alpha));
    t.extend(quad(250, 300, 140, 50, 500, VARIED));
    t.push(frame(true));

    // 2. The alpha tests; a cut-out in front of a later, farther triangle.
    t.push(cc(CcMode::MODULATE));
    t.push(ramp);
    let white = [[255; 4]; 4];
    for (k, test) in [
        AlphaTest::Always,
        AlphaTest::Less,
        AlphaTest::GEqual,
        AlphaTest::NotEqual,
    ]
    .into_iter()
    .enumerate()
    {
        t.push(alpha(test, 0x88));
        let x = 20 + i16::try_from(k).unwrap_or(0) * 150;
        t.extend(quad(x, 20, 130, 120, 1000, white));
    }
    t.push(alpha(AlphaTest::GEqual, 128));
    t.extend(quad(100, 170, 300, 170, 1000, white));
    t.push(alpha(AlphaTest::Always, 0));
    t.push(Transaction::write(reg::TEX0_CFG, Tex0Cfg::default().bits()));
    t.extend(quad(60, 190, 560, 130, 5000, [[255, 0, 0, 255]; 4]));
    t.push(frame(true));

    // 3. The nine settings' alphas, as cut-outs.
    t.push(alpha(AlphaTest::GEqual, 96));
    nine(&mut t, &modes);
    t.push(frame(true));
    t
}
