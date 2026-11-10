# 0010: v1.1, drawing speed at the v1.0 interface

**Status:** Accepted (2026-11-10)

## Context

v1.0 (ADR 0009) draws about 460 cycles for each 8 × 8 block a triangle
touches, and its clear takes 9.1 ms of a 16.5 ms frame: a 60 fps frame has
room for about a third of the screen (host-interface §15). Games need more
before they need new features. The time goes on SDRAM traffic done one
run at a time, a clear that writes every word, and the steps of a block
done one after another.

## Decision

v1.1 makes drawing faster without changing the interface: no register,
field or visible behaviour changes, and `ID` still reads interface
version 2. In order (docs/design/draw-speed.md):

- **Fast clear:** a cleared flag per 8 × 8 block of each of the four
  buffers (three colour, one depth), one bit at `bank @ block` in a block
  RAM. A clear sets flags; depth reads, writes, scan-out and `MEM_DATA`
  reads honour them. All three colour buffers have flags because with
  triple buffering the shown and waiting buffers hold untouched blocks too.
- **SDRAM throughput:** the controller overlaps a run in one bank with the
  next run's row opening in another, closes rows with auto-precharge, and
  takes runs of up to 64 words.
- **Overlapped blocks:** two sets of tile buffers, so one block's writes
  overlap the next block's reads and pixels; then the pixel and texel
  passes of consecutive blocks overlap.

The goal is a whole screen, cleared and textured, at 60 fps.

## Consequences

- Everything a host wrote against v1.0 runs unchanged, faster.
- The reference renderer and the pixel-exact tests are unchanged; the
  changes are in when pixels are written, so every existing test keeps
  checking them.
- More logic and two more block RAMs; timing headroom has to be won back
  first. The built-in scene may shrink if room runs short.
- The SDRAM controller gets more complex; the pin-level SDRAM model's
  datasheet checks are what keep it honest.

## Outcome (Phase 26)

The goal is met: the goal scene draws in 685,775 cycles (9.1 ms) against
2,460,609 (32.8 ms) at v1.0, and every v1.0 test stays pixel-exact, on the
twin and on the board. As built, against the decision above:

- **Fast clear** as decided: a clear is 3,603 cycles.
- **SDRAM:** auto-precharge, and the next run starting the cycle after
  the last one's final access in any ready bank. Opening a row during
  another bank's run is impossible at burst length 1 (every cycle of a run
  carries a command); the controller takes runs of up to 64 words, but
  nothing issues them yet (a few percent).
- **Overlapped blocks:** three stages on three blocks at once, the front
  half, the back half and the writer, with banked tile buffers between
  them.
- **Cost:** about 1,100 cells, 330 flip-flops and 4 block RAMs; 85.2 MHz.
  The built-in scene didn't need to shrink.
- **Now SDRAM-bound:** about 190 cycles a fully drawn block, the 128
  words it writes and scan-out's fifth of the bus.
