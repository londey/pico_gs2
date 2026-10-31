# 0002: Built-in test scene before the host link

**Status:** Proposed (2026-10-31)

## Context

The RP2350 isn't connected to the FPGA yet: there's no SPI link and no host
firmware. Triangle rendering needs a source of vertices.

## Decision

Triangles come from a ROM of command packets in the FPGA, in the same format
the host link will carry, through the same command parser. A Rust program
plays the host: it transforms the scene in `f32` as the RP2350 would, maps
it to screen space, and writes the packets as hex for `include_hex!`.

- Phase 10: a static scene.
- Phase 11: a Gouraud cube rotating about the vertical axis over a quarter
  turn of precomputed frames (which loops seamlessly), a pair of
  interpenetrating triangles for Z, and a fan and grid patch for
  watertightness.

## Consequences

- The scene costs block RAM (about 8 DP16KD for 32 frames, est.), freed when
  the host link replaces it.
- The SPI link becomes a phase of its own, and the packet format is settled
  before it.
