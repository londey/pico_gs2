# 0009: The v1.0 interface

**Status:** Accepted (2026-11-08)

## Context

Phase 13 put the GPU behind a register interface (ADR 0008) on the board's
USB serial port, and it works. The next goal is a v1.0 interface that a
separate game project can be written against: on Linux over the serial
port first, on an RP2350 later. The order of work is features first, with
optimisation only where something doesn't fit the ECP5-25K or 75 MHz.

The RP2350 and its SPI link were next in the plan, but the transactions
don't depend on the link, so SPI can wait without the interface changing.
The link's bandwidth, about 250 triangles a frame at 60 fps over 3 Mbaud,
limits scenes until SPI or meshes kept in GPU memory lift it.

## Decision

v1.0 (docs/design/host-interface.md, interface version 2) has:

- **Links:** the serial link as built; SPI designed and documented, with
  the same bytes each way, but built after v1.0.
- **Textures:** one texture unit (`TEX0_CFG`), RGB565 and ARGB4444,
  power-of-two sizes from 4 to 1024, tiled in 4 × 4 blocks as in pico-gs,
  perspective-correct, `REPEAT`, `CLAMP_TO_EDGE` and `MIRROR`. Nearest
  sampling; bilinear if it fits. No palettes, no compression, no mipmaps.
- **A one-pass colour combiner**, `(A − B) × C + D` for colour and alpha
  as the N64's one-cycle mode, with pico-gs's `CC_MODE` encodings and two
  constant colours. Its second pass is reserved.
- **Alpha test**, not alpha blending: blending needs the framebuffer read
  back, and is left for after v1.0, at reserved fields and codes.
- **Memory access:** `MEM_ADDR` and `MEM_DATA`, write and read, for texture
  uploads and reading frames back; the GPU keeps the first 2 MB of each bank
  for its buffers, and the host has the other 30 MB.
- Register addresses and field positions are pico-gs's where the meaning
  is the same; `RENDER_MODE.TEXTURED` moves to `TEX0_CFG.ENABLE`, since it
  sat inside pico-gs's `ALPHA_BLEND`.

## Consequences

- A game written against `gs-proto` moves from the serial port to SPI by
  changing only the code that moves bytes.
- Texture changes and uploads drain the pipeline; games group triangles by
  texture.
- Frames read back over the link let the board be compared with the twin
  pixel for pixel, for the first time.
- After v1.0, at reserved places: alpha blending and other depth tests,
  `ATTR1`'s second colour, a second combiner pass, SPI, meshes in GPU memory.
