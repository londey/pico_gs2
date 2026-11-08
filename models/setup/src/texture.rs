//! Texture sampling, bit for bit as the rasteriser does it
//! (`docs/design/host-interface.md` §6): the perspective divide, the wrap
//! modes, the texel's address in the 4 × 4-block tiled layout and its
//! cache line, the texel formats, and the colour combiner (§7).
//!
//! Each pixel has the S′, T′ and Q planes' iterator values (36-bit counts,
//! 16 fraction bits over each field's native units: Q4.12 for S′ and T′,
//! UQ1.15 for Q). With `q` the Q value's low 32 bits (`Q · 2^31`):
//!
//! ```text
//! lz = leading zeros of q;  n = q << lz;  x = n >> 15     (UQ1.16, 1 <= x < 2)
//! r0 = SEED[n[30:21]]                                     (1/x to 11 bits, UQ0.17)
//! c  = (2^34 - x·r0) >> 17;  r = min((r0·c) >> 16, 2^17 - 1)   (one Newton–Raphson step)
//! sp = S′ bits 31:7, signed                               (S′ · 2^21)
//! U  = (sp · r) · 2^(lz + log2 width - 34)                (s · width, 4 fraction bits)
//! ```
//!
//! `U` is shifted with the product first widened by 7 bits and shifted
//! right (arithmetically, so rounding down), then saturated to 20 bits,
//! ±32,768 texels.

use gs_proto::{CcMode, CcSource, Tex0Cfg, TexFormat, Wrap};

use crate::reciprocal::SEED_TABLE;

/// The largest `r`, so that it fits 17 bits.
const R_MAX: u64 = (1 << 17) - 1;

/// The texture coordinate's range after the shift: 20 bits, signed.
const U_MIN: i64 = -(1 << 19);

/// The largest texture coordinate.
const U_MAX: i64 = (1 << 19) - 1;

/// `1 / Q` for a pixel: `r` (UQ0.17, as `2^17 / x`) and `lz`.
///
/// # Arguments
///
/// * `q_count` - The Q plane's iterator count at the pixel.
///
/// # Returns
///
/// `(r, lz)`, with `1 / Q ≈ r · 2^(lz - 17)`, Q being the count over 2^31.
pub fn reciprocal(q_count: i64) -> (u64, u32) {
    // The low 32 bits, as the hardware takes them; 0 (outside the
    // triangle) is taken as 1.
    let q = u32::try_from(q_count & 0xFFFF_FFFF).unwrap_or(0).max(1);
    let lz = q.leading_zeros();
    let n = u64::from(q << lz);
    let x = n >> 15;
    let index = usize::try_from(n >> 21 & 0x3FF).expect("10 bits");
    let r0 = u64::try_from(SEED_TABLE[index]).expect("positive");
    let c = ((1_u64 << 34) - x * r0) >> 17;
    let r = ((r0 * c) >> 16).min(R_MAX);
    (r, lz)
}

/// A texture coordinate in texels with 4 fraction bits: `s · 2^log2 · 16`.
///
/// # Arguments
///
/// * `s_count` - The S′ (or T′) plane's iterator count at the pixel.
/// * `recip` - The pixel's [`reciprocal`] of Q.
/// * `log2` - log₂ of the texture's width (or height), 2 to 10.
///
/// # Returns
///
/// The coordinate, saturated to ±2^19.
pub fn coordinate(s_count: i64, recip: (u64, u32), log2: u8) -> i64 {
    let (r, lz) = recip;
    // Bits 31:7 of the count, sign-extended from bit 24.
    let sp = ((s_count >> 7 & 0x1FF_FFFF) ^ 0x100_0000) - 0x100_0000;
    let product = sp * i64::try_from(r).expect("17 bits");
    // The exponent lz + log2 - 34 is -32 to 7: widen by 7, shift right by
    // 0 to 39.
    let right = 7 - (i64::from(lz) + i64::from(log2) - 34);
    ((product << 7) >> right).clamp(U_MIN, U_MAX)
}

/// A texel coordinate brought into the texture by a wrap mode.
///
/// # Arguments
///
/// * `u` - The coordinate, 4 fraction bits.
/// * `log2` - log₂ of the texture's size along it.
/// * `wrap` - The wrap mode.
///
/// # Returns
///
/// The texel's column (or row), `0` to `2^log2 - 1`.
pub fn wrap(u: i64, log2: u8, wrap: Wrap) -> u32 {
    let ui = u >> 4;
    let size = 1_i64 << log2;
    let w = match wrap {
        Wrap::Repeat => ui & (size - 1),
        Wrap::ClampToEdge => ui.clamp(0, size - 1),
        Wrap::Mirror if ui & size != 0 => !ui & (size - 1),
        Wrap::Mirror => ui & (size - 1),
    };
    u32::try_from(w).expect("in the texture")
}

/// Where a texel is: its word address, and its place in the texture cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexelAddr {
    /// The word address in GPU memory (24 bits, wrapping).
    pub word: u32,

    /// The cache tag: the 4 × 4 block's word address over 16 (20 bits).
    pub tag: u32,

    /// The cache line: the block's row and column, 4 bits each.
    pub line: u32,

    /// The texel within the block: row and column, 2 bits each.
    pub offset: u32,
}

/// The address of texel `(u, v)` of a texture, tiled in 4 × 4 blocks.
///
/// # Arguments
///
/// * `u` - Its column.
/// * `v` - Its row.
/// * `cfg` - The texture.
///
/// # Returns
///
/// Its address.
pub fn texel_addr(u: u32, v: u32, cfg: &Tex0Cfg) -> TexelAddr {
    let (bx, by) = (u >> 2, v >> 2);
    let block = by << (cfg.width_log2 - 2) | bx;
    let tag = (u32::from(cfg.base) << 4).wrapping_add(block) & 0xF_FFFF;
    let offset = (v & 3) << 2 | (u & 3);
    TexelAddr {
        word: tag << 4 | offset,
        tag,
        line: (by & 0xF) << 4 | (bx & 0xF),
        offset,
    }
}

/// A texel's colour, 8 bits a channel: R, G, B, A.
///
/// # Arguments
///
/// * `texel` - The stored word.
/// * `format` - Its format.
///
/// # Returns
///
/// The channels: each field's bits repeated to fill 8.
pub fn decode(texel: u16, format: TexFormat) -> [u8; 4] {
    let bits = |shift: u32, width: u32| u32::from(texel) >> shift & ((1 << width) - 1);
    let widen = |v: u32, width: u32| {
        let w = match width {
            4 => v * 17,
            5 => v << 3 | v >> 2,
            _ => v << 2 | v >> 4,
        };
        u8::try_from(w).expect("8 bits")
    };
    match format {
        TexFormat::Rgb565 => [
            widen(bits(11, 5), 5),
            widen(bits(5, 6), 6),
            widen(bits(0, 5), 5),
            255,
        ],
        TexFormat::Argb4444 => [
            widen(bits(8, 4), 4),
            widen(bits(4, 4), 4),
            widen(bits(0, 4), 4),
            widen(bits(12, 4), 4),
        ],
    }
}

/// A colour channel's iterator count as 8 bits: over 2^16, rounded half
/// up, clamped.
///
/// # Arguments
///
/// * `v` - The iterator count.
///
/// # Returns
///
/// The channel, 0 to 255.
pub fn shade8(v: i64) -> u8 {
    u8::try_from(((v + (1 << 15)) >> 16).clamp(0, 255)).expect("in range")
}

/// The combiner's `(A − B) × C + D` with `B` and `D` zero: `A × C`, `C`
/// stretched to 0 to 256 so that 255 is exactly 1 (host-interface.md §7).
///
/// # Arguments
///
/// * `a` - A channel of A.
/// * `c` - The same channel of C.
///
/// # Returns
///
/// The product, rounded.
pub fn modulate(a: u8, c: u8) -> u8 {
    let c = u32::from(c) + u32::from(c >> 7);
    u8::try_from((u32::from(a) * c + 128) >> 8).expect("at most 255")
}

/// The combiner's inputs at a pixel, each R, G, B, A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcInputs {
    /// The texel (opaque white for an untextured triangle).
    pub tex: [u8; 4],

    /// The shade: the interpolated vertex colour.
    pub shade: [u8; 4],

    /// `CONST0`.
    pub const0: [u8; 4],

    /// `CONST1`.
    pub const1: [u8; 4],
}

/// The colour combiner (host-interface.md §7): for each channel,
/// `clamp(D + (((A − B) × C′ + 128) >> 8), 0, 255)`, `C′` being `C`
/// stretched to 0 to 256 and `>>` rounding down.
///
/// # Arguments
///
/// * `cc` - `CC_MODE`.
/// * `inputs` - The inputs.
///
/// # Returns
///
/// R, G, B and A.
pub fn combine(cc: &CcMode, inputs: &CcInputs) -> [u8; 4] {
    let pick = |s: CcSource, ch: usize| -> i32 {
        i32::from(match s {
            CcSource::Tex0 => inputs.tex[ch],
            CcSource::Shade0 => inputs.shade[ch],
            CcSource::Const0 => inputs.const0[ch],
            CcSource::Const1 => inputs.const1[ch],
            CcSource::One => 255,
            CcSource::Zero | CcSource::Reserved(_) => 0,
            CcSource::Tex0Alpha => inputs.tex[3],
            CcSource::Shade0Alpha => inputs.shade[3],
            CcSource::Const0Alpha => inputs.const0[3],
        })
    };
    [0, 1, 2, 3].map(|ch| {
        let abcd = if ch < 3 { cc.rgb } else { cc.alpha };
        let [a, b, c, d] = abcd.map(|s| pick(s, ch));
        let c = c + (c >> 7);
        let v = d + (((a - b) * c + 128) >> 8);
        u8::try_from(v.clamp(0, 255)).expect("clamped")
    })
}

/// An 8-bit colour as RGB565, keeping each channel's top bits.
///
/// # Arguments
///
/// * `rgb` - The channels.
///
/// # Returns
///
/// The colour.
pub fn rgb565(rgb: [u8; 3]) -> u16 {
    u16::from(rgb[0] >> 3) << 11 | u16::from(rgb[1] >> 2) << 5 | u16::from(rgb[2] >> 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reciprocal is within 2^-15 of `1 / x` over every seed interval.
    #[test]
    fn reciprocal_is_accurate() {
        let mut worst: f64 = 0.0;
        for k in 0..(1 << 16) {
            let q = (1_u64 << 31) + (k << 15) + 12_345;
            let (r, lz) = reciprocal(i64::try_from(q).expect("fits"));
            assert_eq!(lz, 0);
            #[allow(clippy::cast_precision_loss)]
            let exact = (1u64 << 48) as f64 / q as f64;
            #[allow(clippy::cast_precision_loss)]
            let err = (r as f64 - exact).abs() / exact;
            worst = worst.max(err);
        }
        assert!(worst < 1.0 / f64::from(1 << 15), "worst {worst}");
    }

    /// `s` = S′ / Q, times the size: an S′ of 0.5 at Q = 1 is half the
    /// texture; at Q = 0.25, two repeats.
    #[test]
    fn coordinates_divide_by_q() {
        // Q = 1.0 (UQ1.15 32768) and 0.25; S′ = 0.5 (Q4.12 2048); counts
        // have 16 fraction bits.
        for (q, want) in [(32768_i64, 32 * 16), (8192, 128 * 16)] {
            let u = coordinate(2048 << 16, reciprocal(q << 16), 6);
            assert!((u - want).abs() <= 1, "q {q}: {u} not {want}");
        }
        // Negative S′.
        let u = coordinate(-2048 << 16, reciprocal(32768 << 16), 6);
        assert!((u + 32 * 16).abs() <= 1, "{u}");
    }

    /// The wrap modes at and beyond the edges of an 8-texel texture.
    #[test]
    fn wrap_modes() {
        let at = |t: i64, w| wrap(t * 16 + 3, 3, w);
        assert_eq!(
            [-1, 0, 7, 8, 17].map(|t| at(t, Wrap::Repeat)),
            [7, 0, 7, 0, 1]
        );
        assert_eq!(
            [-1, 0, 7, 8, 17].map(|t| at(t, Wrap::ClampToEdge)),
            [0, 0, 7, 7, 7]
        );
        assert_eq!(
            [-1, 0, 7, 8, 9, 15, 16].map(|t| at(t, Wrap::Mirror)),
            [0, 0, 7, 7, 6, 0, 0]
        );
    }

    /// Blocks of 4 × 4 in raster order across the texture; the cache line
    /// from the block's row and column.
    #[test]
    fn texels_are_tiled() {
        let cfg = Tex0Cfg {
            width_log2: 5,
            height_log2: 5,
            base: 2,
            ..Tex0Cfg::default()
        };
        let a = texel_addr(5, 6, &cfg);
        // Block (1, 1) of 8 across: block 9; texel (1, 2) in it.
        assert_eq!(a.word, 512 + 9 * 16 + 2 * 4 + 1);
        assert_eq!((a.tag, a.line, a.offset), (32 + 9, 0x11, 9));
    }

    /// The combiner: modulate is [`modulate`]; decal and shade pass an
    /// input through; `(A − B)` can go negative, rounding down, and the sum
    /// is clamped.
    #[test]
    fn combiner() {
        let inputs = CcInputs {
            tex: [200, 100, 50, 255],
            shade: [128, 255, 0, 64],
            const0: [10, 20, 30, 40],
            const1: [255, 0, 255, 0],
        };
        let want = [0, 1, 2, 3].map(|c| modulate(inputs.tex[c], inputs.shade[c]));
        assert_eq!(combine(&CcMode::MODULATE, &inputs), want);
        assert_eq!(combine(&CcMode::DECAL, &inputs), inputs.tex);
        assert_eq!(combine(&CcMode::SHADE, &inputs), inputs.shade);
        // Fade: (tex - const1) x shade alpha + const1. Red: 255 +
        // floor((-55 x 64 + 128) / 256) = 255 - 14.
        let f = combine(&CcMode::FADE, &inputs);
        assert_eq!(f, [241, 25, 204, 255]);
        // Texture + shade saturates.
        assert_eq!(combine(&CcMode::ADD_SHADE, &inputs)[1], 255);
    }

    /// Texel formats widen each field; modulating by white keeps a value.
    #[test]
    fn formats_and_modulate() {
        assert_eq!(decode(0xF800, TexFormat::Rgb565), [255, 0, 0, 255]);
        assert_eq!(decode(0x07E0, TexFormat::Rgb565), [0, 255, 0, 255]);
        assert_eq!(decode(0x8F40, TexFormat::Argb4444), [255, 68, 0, 136]);
        for v in 0..=255 {
            assert_eq!(modulate(v, 255), v);
            assert_eq!(modulate(v, 0), 0);
        }
        assert_eq!(rgb565([255, 255, 255]), 0xFFFF);
        assert_eq!(rgb565([0x84, 0x82, 0x08]), 0x8401);
    }
}
