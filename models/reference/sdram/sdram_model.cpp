// Spec-ref: unit_011_texture_sampler.md
// Spec-ref: unit_011.03_index_cache.md
// Spec-ref: unit_011.06_palette_lut.md
//
// Behavioral SDRAM model implementation.
//
// This is a minimal, byte-array-backed SDRAM model that provides correct
// word-level read/write semantics.  Cycle-level command timing
// (ACTIVATE / READ / WRITE / PRECHARGE, CAS latency, refresh, bank
// state) is layered on by connect_sdram() in harness.cpp.
//
// Texture-pipeline burst patterns served by this model:
//
//   * UNIT-011.03 index cache fills:
//       8 x 16-bit words per fill (16 bytes = one 4x4 INDEXED8_2X2
//       index block in INT-014 layout).  Triggered when a sampler
//       misses in its on-chip half-resolution index cache and the
//       texture_sampler.sv fill FSM steps F_REQ -> F_BURST -> F_INSTALL.
//
//   * UNIT-011.06 palette load sub-bursts:
//       Up to 32 x 16-bit words per sub-burst (port-3 arbiter cap).
//       64 sub-bursts cover one 4096-byte palette slot (256 entries x
//       4 quadrants x RGBA8888).  The texture_palette_lut.sv load FSM
//       (IDLE -> ARMING -> BURSTING -> DONE) is preempted by index
//       fills at the 3-way arbiter inside texture_sampler.sv; this
//       model does not need to know about preemption since it only
//       responds to whatever READ commands the SDRAM controller drives.
//
// preload_palette_blob() / preload_index_array() are convenience
// wrappers used by tests that want to skip the firmware-driven
// MEM_DATA / MEM_FILL upload phase and stage data directly into SDRAM.
// Production tests upload through the GPU's mem_dma path so the
// behavioral model receives real WRITE commands from the controller.
//
// References:
//   INT-011 (SDRAM Memory Layout)
//   INT-014 (Texture Memory Layout)
//   UNIT-011 (Texture Sampler)
//   UNIT-011.03 (Index Cache)
//   UNIT-011.06 (Palette LUT)

#include "sdram_model.hpp"

#include <algorithm>
#include <cstdio>
#include <cstring>

namespace {
/// Bytes per palette slot (UNIT-011.06): 256 entries x 4 quadrants x 4
/// bytes per RGBA8888 channel = 4096 bytes.  Matches the
/// `texture_palette_lut.sv` load FSM transfer length.
constexpr std::size_t PALETTE_SLOT_BYTES = 4096;
}

SdramModel::SdramModel(uint32_t num_words) : mem_(num_words, 0) {}

uint16_t SdramModel::read_word(uint32_t word_addr) const {
    if (word_addr >= mem_.size()) {
        return 0;
    }
    return mem_[word_addr];
}

void SdramModel::write_word(uint32_t word_addr, uint16_t data) {
    if (word_addr >= mem_.size()) {
        return;
    }
    mem_[word_addr] = data;
}

void SdramModel::upload_raw(uint32_t base_word_addr, std::span<const uint8_t> data) {
    // Write raw bytes as 16-bit little-endian words.
    size_t num_words = data.size() / 2;
    for (size_t i = 0; i < num_words; i++) {
        uint16_t word =
            static_cast<uint16_t>(data[i * 2]) | (static_cast<uint16_t>(data[i * 2 + 1]) << 8);
        write_word(base_word_addr + static_cast<uint32_t>(i), word);
    }
}

void SdramModel::preload_palette_blob(uint32_t palette_base_word,
                                      std::span<const uint8_t> blob) {
    if (blob.size() != PALETTE_SLOT_BYTES) {
        throw std::invalid_argument(
            "preload_palette_blob: blob must be exactly 4096 bytes");
    }
    upload_raw(palette_base_word, blob);
}

void SdramModel::preload_index_array(uint32_t index_base_word,
                                     std::span<const uint8_t> indices) {
    upload_raw(index_base_word, indices);
}

void SdramModel::burst_read(uint32_t start_word_addr, std::span<uint16_t> buffer) const {
    for (uint32_t i = 0; i < buffer.size(); i++) {
        buffer[i] = read_word(start_word_addr + i);
    }
}

void SdramModel::burst_write(uint32_t start_word_addr, std::span<const uint16_t> buffer) {
    for (uint32_t i = 0; i < buffer.size(); i++) {
        write_word(start_word_addr + i, buffer[i]);
    }
}

std::vector<uint16_t>
SdramModel::read_framebuffer(uint32_t base_word, int width_log2, int height) const {
    int width = 1 << width_log2;

    std::vector<uint16_t> fb(static_cast<size_t>(width) * height);

    // Block-tiled readback matching the pixel pipeline's (UNIT-006) tiled
    // address formula.
    //
    // After pixel pipeline integration, the framebuffer uses 4x4 block-tiled
    // addressing (INT-011).  The pixel pipeline computes byte addresses:
    //   fb_base    = fb_color_base << 9   (base * 512 bytes)
    //   block_x    = pixel_x >> 2
    //   block_y    = pixel_y >> 2
    //   local_x    = pixel_x & 3
    //   local_y    = pixel_y & 3
    //   blocks_log2 = max(width_log2 - 2, 0)
    //   block_idx  = (block_y << blocks_log2) | block_x
    //   block_off  = block_idx << 5   (32 bytes per 4x4 RGB565 block)
    //   pixel_off  = (local_y * 4 + local_x) * 2
    //   byte_addr  = fb_base + block_off + pixel_off
    //
    // The SDRAM controller decomposes byte addresses into bank/row/col
    // with col = {addr[8:1], 1'b0}, dropping bit 0.  connect_sdram()
    // reconstructs word_addr = (bank << 23) | (row << 9) | col, which
    // for even byte addresses equals the byte address itself.
    //
    // base_word is in byte units (fb_color_base << 9).
    int blocks_log2 = (width_log2 >= 2) ? (width_log2 - 2) : 0;

    for (int py = 0; py < height; ++py) {
        for (int px = 0; px < width; ++px) {
            uint32_t block_x = static_cast<uint32_t>(px) >> 2;
            uint32_t block_y = static_cast<uint32_t>(py) >> 2;
            uint32_t local_x = static_cast<uint32_t>(px) & 3;
            uint32_t local_y = static_cast<uint32_t>(py) & 3;
            uint32_t block_idx = (block_y << blocks_log2) | block_x;
            uint32_t block_off = block_idx << 5;  // 32 bytes per block
            uint32_t pixel_off = (local_y * 4 + local_x) * 2;
            uint32_t byte_addr = base_word + block_off + pixel_off;

            fb[py * width + px] = read_word(byte_addr);
        }
    }

    return fb;
}

void SdramModel::fill_texture(
    uint32_t base_word_addr, TexFormat fmt, std::span<const uint8_t> pixel_data, uint32_t width_log2
) {
    if (fmt != TexFormat::RGB565) {
        throw std::invalid_argument(
            "fill_texture: only RGB565 is supported (INDEXED8_2X2 textures "
            "use preload_index_array() / preload_palette_blob() or the "
            "MEM_DATA/MEM_FILL upload path)"
        );
    }

    // RGB565 uncompressed: input data is in linear row-major pixel order
    // and must be rearranged into 4x4 block-tiled layout per INT-011.
    //
    // INT-011 block-tiled address calculation:
    //   block_x   = pixel_x >> 2
    //   block_y   = pixel_y >> 2
    //   local_x   = pixel_x & 3
    //   local_y   = pixel_y & 3
    //   block_idx = (block_y << (WIDTH_LOG2 - 2)) | block_x
    //   word_addr = base_word + block_idx * 16 + (local_y * 4 + local_x)

    uint32_t width = 1u << width_log2;
    // RGB565: 2 bytes per pixel, one 16-bit SDRAM word per pixel.
    uint32_t height = static_cast<uint32_t>(pixel_data.size() / 2) / width;
    // Raw loop retained: reinterpret_cast span access with block-tiled
    // address arithmetic does not map cleanly to a standard algorithm.
    const auto* src = reinterpret_cast<const uint16_t*>(pixel_data.data());

    for (uint32_t y = 0; y < height; y++) {
        for (uint32_t x = 0; x < width; x++) {
            uint32_t block_x = x >> 2;
            uint32_t block_y = y >> 2;
            uint32_t local_x = x & 3;
            uint32_t local_y = y & 3;
            uint32_t block_idx = (block_y << (width_log2 - 2)) | block_x;
            uint32_t word_addr = base_word_addr + block_idx * 16 + (local_y * 4 + local_x);

            // Source pixel in linear row-major order (little-endian u16).
            uint16_t pixel = src[y * width + x];
            write_word(word_addr, pixel);
        }
    }
}
