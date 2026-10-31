# 0003: 8×8 tile buffer, 16-bit Z, triple buffering

**Status:** Proposed (2026-10-31)

## Context

The SDRAM controller opens and closes a row for every run of 1 to 32 words,
so single-pixel writes cost an activate and a precharge each. Scan-out
already takes about 18% of the bus. The test scene needs depth to be
order-independent, and a low or uneven frame rate shouldn't stall rendering.

## Decision

- **Pixel path:** traversal works on 8×8 blocks at one pixel per clock in an
  on-chip tile buffer (colour and Z). Each block's Z is read as 8 runs of 8
  words, and its colour and Z are written back the same way, with DQM masks
  for uncovered pixels. Immediate mode, no binning.
- **Depth:** a 16-bit Z buffer, cleared to 0xFFFF each frame, less-than test.
- **Buffers:** three colour buffers in banks 0 to 2 and the Z buffer in
  bank 3, each 640×360 words with lines packed. One buffer is displayed, at
  most one is complete and waiting, and the renderer draws into the third.
  The display swaps to the waiting buffer at vertical blank.
- **Arbitration:** scan-out line loads have priority over the renderer.

## Consequences

- About 300 SDRAM cycles per 8×8 block (est.), about 5 per pixel, before any
  overlap of traversal with SDRAM traffic.
- 32 MB holds the four buffers (1.8 MB) many times over; the rest is free for
  textures later.
- Binning into on-chip tiles (brief D10) can be revisited once bandwidth is
  measured on the board.
