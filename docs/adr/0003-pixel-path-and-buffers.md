# 0003: Tiled 8×8 blocks, 16-bit Z, triple buffering

**Status:** Accepted (2026-11-04)

## Context

The SDRAM controller opens and closes a row for every run of words, so the
cost of a run is its length plus about 6 cycles. Single-pixel writes would
pay that per pixel; an 8×8 block in a line-ordered framebuffer is 8 runs of
8 words. Scan-out already takes about 23% of the bus. The test scene needs
depth to be order-independent, and a low or uneven frame rate shouldn't
stall rendering.

## Decision

- **Tiling:** colour and Z buffers are stored as 80 × 45 blocks of 8×8
  pixels, each block 64 contiguous words in raster order, block `(bx, by)`
  at word `(by * 80 + bx) * 64`. A block never crosses an SDRAM row, so it
  is read or written as one run of 64 words.
- **Pixel path:** traversal works on 8×8 blocks at one pixel per clock in an
  on-chip tile buffer (colour and Z). Per block: one Z read, then one colour
  write and one Z write, with a per-word write mask (DQM) for uncovered or
  Z-rejected pixels. Immediate mode, no binning.
- **Scan-out:** loads a block row (8 lines, 5,120 contiguous words) at a
  time into a double-buffered block-row buffer in block RAM.
- **Depth:** a 16-bit Z buffer, cleared to 0xFFFF each frame, less-than test.
- **Buffers:** three colour buffers in banks 0 to 2 and the Z buffer in
  bank 3. One is displayed, at most one is complete and waiting, and the
  renderer draws into the third. The display swaps to the waiting buffer at
  vertical blank.
- **Arbitration:** scan-out loads have priority over the renderer.
- **Controller:** runs of up to 64 words, with a write mask per word.

## Consequences

- About 210 SDRAM cycles per block (est.), about 3.3 per pixel, against
  about 310 with 8-word runs.
- Scan-out drops to about 21% of the bus, but its buffer grows from 2 to
  about 10 DP16KD.
- Anything reading the framebuffer (tests, a future readback) must undo the
  tiling.
- 32 MB holds the four buffers (1.8 MB) many times over; the rest is free for
  textures later.
- Binning into on-chip tiles (brief D10) can be revisited once bandwidth is
  measured on the board.
