// Spec-ref: unit_011_texture_sampler.md
// Spec-ref: unit_011.03_index_cache.md
// Spec-ref: unit_011.06_palette_lut.md
//
// Behavioral SDRAM model header for the integration test harness.
//
// This model simulates the W9825G6KH-6 SDRAM (32 MB, 16-bit data bus)
// as a simple flat array of 16-bit words, with helper methods for the
// 4x4 block-tiled address layout defined in INT-011.
//
// The model is a *behavioral* stub: it provides correct read/write
// semantics without modeling SDRAM timing (ACTIVATE, CAS latency, etc.).
// connect_sdram() in harness.cpp wraps this model with the SDRAM command
// decoder and CAS-latency read pipeline that the SDRAM controller drives.
//
// Texture-pipeline burst patterns served by this model (UNIT-011):
//
//   * Index cache fill (UNIT-011.03):
//       8 x 16-bit words = 16 bytes = 16 x 8-bit palette indices
//       (one 4x4 INDEXED8_2X2 index block, INT-014 layout).
//       FSM in texture_sampler.sv: IDLE -> F_REQ -> F_BURST -> F_INSTALL.
//
//   * Palette load (UNIT-011.06):
//       Up to 32 x 16-bit words per sub-burst (port-3 arbiter cap).
//       64 sub-bursts cover one 4096-byte slot (256 entries x
//       4 quadrants x RGBA8888).
//       FSM in texture_palette_lut.sv: IDLE -> ARMING -> BURSTING -> DONE.
//       Sub-bursts are interruptible by index-cache fills (which preempt
//       palette traffic at the 3-way arbiter inside texture_sampler.sv).
//
// References:
//   INT-011 (SDRAM Memory Layout) -- 4x4 block-tiled address layout,
//       memory map, surface base addresses.
//   INT-014 (Texture Memory Layout) -- INDEXED8_2X2 index array layout
//       and palette blob layout.
//   UNIT-011 (Texture Sampler), UNIT-011.03 (Index Cache),
//   UNIT-011.06 (Palette LUT) -- burst lengths and FSM behaviour.

#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <stdexcept>
#include <vector>

/// Texture format codes matching INT-014 TEXn_CFG.FORMAT field encoding.
///
/// UNIT-011 currently supports INDEXED8_2X2 only.  RGB565 is retained for
/// the framebuffer scan-out helpers and for the legacy `fill_texture`
/// uncompressed-upload path used by the (now-obsolete) BC test scripts.
enum class TexFormat : uint8_t {
    INDEXED8_2X2 = 0, ///< 8 bpp indexed (UNIT-011 active path)
    RGB565 = 4,       ///< 16 bpp uncompressed (framebuffer / scan-out)
};

/// UNIT-011 burst lengths per format (in 16-bit words).
///
/// INDEXED8_2X2 is the active texture format: each 4x4 index-block fill
/// reads 8 x 16-bit words = 16 index bytes.
static constexpr uint8_t BURST_LEN_INDEX_BLOCK = 8;

/// Palette load FSM (UNIT-011.06) sub-burst length cap (in 16-bit words).
///
/// The 3-way port-3 arbiter inside texture_sampler.sv limits each
/// palette sub-burst to 32 words; 64 such sub-bursts cover one 4096-byte
/// palette slot.
static constexpr uint8_t BURST_LEN_PALETTE_SUBBURST = 32;

/// Behavioral SDRAM model.
///
/// Provides a flat array of 16-bit words with methods for word-level
/// read/write access and texture upload with INT-011 block-tiled layout.
class SdramModel {
public:
    /// Construct a model with the given number of 16-bit words.
    /// The standard SDRAM is 16M words (32 MB).
    ///
    /// @param num_words  Number of 16-bit words to allocate.
    explicit SdramModel(uint32_t num_words);

    /// Read a 16-bit word at the given word address.
    /// Returns 0 for out-of-range addresses.
    ///
    /// @param word_addr  Word address to read.
    /// @return The 16-bit value at word_addr, or 0 if out of range.
    uint16_t read_word(uint32_t word_addr) const;

    /// Write a 16-bit word at the given word address.
    /// Silently ignores out-of-range addresses.
    ///
    /// @param word_addr  Word address to write.
    /// @param data       16-bit value to write.
    void write_word(uint32_t word_addr, uint16_t data);

    /// Upload raw byte data into SDRAM as 16-bit little-endian words.
    ///
    /// Used by the harness to pre-stage palette blobs and index arrays
    /// directly into SDRAM (bypassing the GPU's MEM_DATA / MEM_FILL path)
    /// for tests that want to skip the firmware-driven upload phase.
    ///
    /// @param base_word_addr  Starting word address in SDRAM.
    /// @param data            Raw bytes to upload.
    void upload_raw(uint32_t base_word_addr, std::span<const uint8_t> data);

    /// Pre-load an INDEXED8_2X2 palette blob into SDRAM.
    ///
    /// Convenience wrapper around upload_raw() with the per-INT-014
    /// palette layout asserted (4096 bytes = 256 entries x 4 quadrants x
    /// RGBA8888).  After this call the on-chip palette LUT can be
    /// populated by issuing a PALETTEn write with LOAD_TRIGGER=1 — the
    /// load FSM will burst the same bytes back through arbiter port 3.
    ///
    /// @param palette_base_word  Word address (= byte_addr / 2) where the
    ///                           4096-byte palette blob is to live.  This
    ///                           must equal `PALETTEn.BASE_ADDR * 256`.
    /// @param blob               Palette payload; must be exactly 4096
    ///                           bytes long.
    /// @throws std::invalid_argument if blob is not 4096 bytes.
    void preload_palette_blob(uint32_t palette_base_word,
                              std::span<const uint8_t> blob);

    /// Pre-load an INDEXED8_2X2 index array into SDRAM.
    ///
    /// The bytes are written verbatim — the caller is responsible for
    /// providing the data in 4x4 block-tiled order (INT-014 §3).  Used
    /// for tests that want to skip the MEM_DATA upload phase.
    ///
    /// @param index_base_word  Word address (= byte_addr / 2) of the
    ///                         index array base, matching TEX_CFG.BASE.
    /// @param indices          Index payload (8-bit per index, tiled).
    void preload_index_array(uint32_t index_base_word,
                             std::span<const uint8_t> indices);

    /// Fill an RGB565 texture region with pixel data, converting from
    /// linear row-major pixel order to INT-011 4x4 block-tiled layout.
    ///
    /// Retained for the framebuffer scan-out helper and legacy callers.
    /// INDEXED8_2X2 textures use preload_index_array() / GPU MEM_DATA
    /// uploads instead.
    ///
    /// @param base_word_addr  SDRAM word address for the texture base.
    /// @param fmt             Texture format (must be RGB565).
    /// @param pixel_data      Linear row-major RGB565 pixel data.
    /// @param width_log2      Log2 of texture width in pixels (e.g. 4 for 16px).
    /// @throws std::invalid_argument if fmt is not RGB565.
    void fill_texture(
        uint32_t base_word_addr,
        TexFormat fmt,
        std::span<const uint8_t> pixel_data,
        uint32_t width_log2
    );

    /// Burst read a sequence of consecutive 16-bit words from the model.
    ///
    /// Reads sequential 16-bit words starting at start_word_addr into the
    /// caller-supplied buffer.  This models the SDRAM controller's
    /// sequential burst read used by:
    ///   * UNIT-011.03 index cache fills (8-word bursts)
    ///   * UNIT-011.06 palette load sub-bursts (up to 32 words each)
    ///   * UNIT-006 framebuffer reads
    ///
    /// Out-of-range addresses read as 0.
    ///
    /// @param start_word_addr  Starting 16-bit word address in SDRAM.
    /// @param buffer           Output buffer for read data.
    void burst_read(uint32_t start_word_addr, std::span<uint16_t> buffer) const;

    /// Burst write a sequence of consecutive 16-bit words into the model.
    ///
    /// Writes sequential 16-bit words starting at start_word_addr from the
    /// caller-supplied buffer. This models the SDRAM controller's sequential
    /// burst write, used by the framebuffer write-back path.
    ///
    /// Out-of-range addresses are silently ignored.
    ///
    /// @param start_word_addr  Starting 16-bit word address in SDRAM.
    /// @param buffer           Input buffer of data to write.
    void burst_write(uint32_t start_word_addr, std::span<const uint16_t> buffer);

    /// Read back a framebuffer region from SDRAM using flat linear
    /// addressing matching the rasterizer's WRITE_PIXEL formula.
    ///
    /// The rasterizer (UNIT-005) writes pixels at byte addresses:
    ///   fb_addr = fb_base + y * 1280 + x * 2
    /// where 1280 is the hardcoded byte stride (640-pixel row pitch, each
    /// pixel is 16-bit RGB565).  The SDRAM model's word-address space maps
    /// 1:1 to byte addresses for even addresses, so:
    ///   word_addr = base_word + y * 1280 + x * 2
    ///
    /// The width_log2 parameter determines how many columns per row are
    /// read (the image width), which may be narrower than the 640-pixel
    /// stride.
    ///
    /// @param base_word   Framebuffer base address in SDRAM model word units.
    /// @param width_log2  log2(surface width); e.g. 9 for 512-wide.
    /// @param height      Surface height in pixels to read back.
    /// @return  Vector of (1 << width_log2) * height uint16_t RGB565 pixels
    ///          in row-major order.
    std::vector<uint16_t>
    read_framebuffer(uint32_t base_word, int width_log2, int height) const;

    /// Return the total number of 16-bit words in the model.
    uint32_t size() const {
        return static_cast<uint32_t>(mem_.size());
    }

private:
    std::vector<uint16_t> mem_;
};
