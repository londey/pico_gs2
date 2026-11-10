# v1.1: drawing speed

The plan for v1.1: the same interface as v1.0 (interface version 2,
[host-interface.md](host-interface.md)), drawn several times faster. No
register, field or behaviour a host can see changes, except how long
things take. ADR [0010](../adr/0010-v1-1-draw-speed.md) records the
decision.

**Status:** Phase 20, the plan, for approval (2026-11-10).

## 1. Goal

**A whole screen at 60 fps:** a frame that clears colour and depth and
draws a screen-filling quad with a 64 × 64 texture, bilinear, finishes
within one display frame, 1,237,500 cycles (16.5 ms). Today it takes
2,460,609 (32.8 ms, host-interface §15). `just perf` measures it, and a
regression check in `just check` keeps it.

Every phase keeps every v1.0 test pixel-exact, the SDRAM model's datasheet
checks clean, the display never short of pixels, and the bitstream at 75
MHz or better.

## 2. Where the time goes today

From `just perf` (host-interface §15), per 8 × 8 block a triangle touches,
one step after another:

| Step | Cycles | What it is |
|---|---|---|
| depth read | ~90 | two 32-word SDRAM runs |
| pixel pass | 64 | edge functions, colour, depth, S′T′Q; depth test |
| texel pass | ~75 | cache lookups and filter, combiner, alpha test (64 + 11, plus misses) |
| writes | ~225 | colour and depth, each two masked 32-word runs |
| **block** | **~460** | |

And per frame, the clear: 14,400 runs of 32 words, 681,114 cycles
(9.1 ms), more than half of a 60 fps frame.

Three things make it slow:

1. **The clear writes every word of the colour and depth buffers.**
2. **The SDRAM controller does one run at a time**
   ([sdram.snt](../../hw/sdram.snt)): ACTIVE, tRCD, the words,
   tWR, PRECHARGE, tRP, then the next request, about 8 to 10 cycles of
   overhead a run, plus the time scan-out takes (about 23% of the bus).
   Colour (banks 0 to 2) and depth (bank 3) are always in different banks,
   so the next run's row could be opened while this one moves words.
3. **The steps don't overlap.** The SDRAM idles during the pixel and texel
   passes, and the pixel and texel logic idles during the reads and
   writes.

## 3. The approach

### 3.1 Fast clear (Phase 22)

A *cleared* flag for every half block of each colour buffer and of the
depth buffer: 7,200 half blocks × 4 buffers, 28,800 bits, in block RAM
(2 DP16KD). A `FRAME` clear sets the drawn buffer's flags (and depth's)
and records its clear colour, a few hundred cycles in place of 681,000.
Then everything that reads a buffer honours the flags:

- **Depth read:** a cleared half block reads as 0xFFFF without touching
  SDRAM. Most blocks are touched first after a clear, so most depth
  reads go away.
- **Writes:** a cleared half block is written whole, unmasked, its undrawn
  pixels with the clear colour (or 0xFFFF), and its flag cleared. This
  costs the same cycles as a masked run.
- **Scan-out:** a cleared half block fills its ring slot with the buffer's
  clear colour without an SDRAM read, so a sparse frame frees most of
  scan-out's 23% for drawing.
- **`MEM_DATA` reads** of a buffer's words return the clear colour (or
  0xFFFF) where the flag is set, so a host reading a frame back sees
  exactly what v1.0 shows. (`MEM_DATA` writes into the GPU's buffers stay
  undefined, as in v1.0 §9.2; they clear no flag.)

The flags are per half block, not per block, because the SDRAM runs, the
write masks and scan-out's ring all work in half blocks.

### 3.2 SDRAM throughput (Phase 23)

- **Overlapped runs:** while a run moves its words, the controller can
  accept the next request; if it is in another bank, it issues that bank's
  ACTIVE during the current run and its first READ or WRITE right after
  the last word, closing rows with auto-precharge (READA, WRITEA). A run
  in the same bank waits as today. Every datasheet rule stays checked by
  the pin-level SDRAM model.
- **Whole-block runs:** runs of up to 64 words, so a block with both
  halves drawn is one run, not two.
- **Ordering the rasteriser's runs** so that consecutive runs alternate
  between a colour bank and the depth bank, which the overlap needs.

### 3.3 Overlapped blocks (Phases 24 and 25)

- **Phase 24, two sets of tile buffers:** the colour, depth and shade
  tiles and the texel-address tiles are doubled (the 64-entry LUT RAMs
  double in LUTs; `atile` and `btile` double in depth inside the block RAMs
  they already use). Block N's writes go out while block N + 1's depth
  read and pixel pass run. A block then costs about the larger of its
  SDRAM traffic and its computing, not their sum.
- **Phase 25, the two passes overlapped:** block N + 1's pixel pass runs
  while block N's texel pass runs, so computing costs about one pass a
  block, not two.

## 4. Phases

Each is its own branch, stacked on the one before, ending at a gate where
I stop and report. The cycle figures are estimates, to be replaced by
measurements.

| Phase | Branch | Work | Gate |
|---|---|---|---|
| 20 | `phase-20-draw-plan` | This plan and ADR 0010 | Approved |
| 21 | `phase-21-groundwork` | Timing headroom back above 80 MHz (registering scan-out's `presenting` path, the critical path at v1.0); `just perf` with SDRAM bus use by requester, and a regression check in `just check` against recorded figures; every twin test asserts the display never ran short; the v1.1 goal scene in `just perf` | ≥ 80 MHz; the perf check in `just check`; no change in any figure |
| 22 | `phase-22-fast-clear` | Cleared flags (§3.1): clear, depth reads, writes, scan-out, `MEM_DATA` reads | Pixel-exact; a new twin test reading back cleared, partly drawn and depth-only-cleared buffers; the clear under 1,000 cycles; mutations; on the board, read-backs pixel-exact |
| 23 | `phase-23-sdram` | Overlapped runs in other banks, auto-precharge, 64-word runs (§3.2) | No SDRAM model violations; pixel-exact; scan-out never short; per-block SDRAM time measured; on the board |
| 24 | `phase-24-overlap-writes` | Two sets of tile buffers: one block's writes overlap the next block's depth read and pixel pass (§3.3) | Pixel-exact; mutations; resources and timing; on the board |
| 25 | `phase-25-overlap-passes` | The pixel pass of one block overlaps the texel pass of the one before (§3.3) | Pixel-exact; mutations; **the goal scene at 60 fps** |
| 26 | `phase-26-v1-1` | Performance figures (host-interface §15 updated), resources, slides; v1.1 | Approved |

**Estimates**, per block a triangle touches (§2's 460 cycles today) and
for the goal scene (one full-screen layer, 3,600 blocks, after a clear):

| After | Clear | Per block, first touch | Per block, again | Goal scene |
|---|---|---|---|---|
| v1.0 | 681,000 | ~460 | ~460 | 2,460,000 (32.8 ms) |
| 22, fast clear | < 1,000 | ~370 (no depth read) | ~460 | ~1,350,000 (18 ms) |
| 23, SDRAM | < 1,000 | ~280 | ~350 | ~1,000,000 (13.5 ms) |
| 24, writes overlapped | < 1,000 | ~220 | ~250 | ~800,000 |
| 25, passes overlapped | < 1,000 | ~150 to 200 | ~200 | ~600,000 to 700,000 |

So the goal may be met by Phase 23 for one layer; Phases 24 and 25 buy
the room for overdraw, more triangles and textures that miss. If a phase
falls well short of its estimate, I report before going on.

## 5. Verification

- **Correctness:** every existing twin test stays pixel-exact against the
  reference renderer, which doesn't change: v1.1 changes when pixels are
  written, never what. Each phase adds the tests its change needs (Phase
  22: read-backs of cleared buffers; Phase 23: back-to-back runs in each
  bank pairing; Phases 24 and 25: blocks whose later triangle depends on
  the earlier one's depth, so an overlap that reads stale depth fails),
  and deliberate breaks of its new logic, each caught.
- **The SDRAM model** checks every datasheet rule on every cycle of every
  test, which matters most in Phase 23.
- **Scan-out** must never run short: from Phase 21 every twin test asserts
  it, since drawing will compete harder for the bus.
- **Speed:** `just perf`'s figures are checked by `just check` from Phase
  21: a scene slower than its recorded figure fails, and a phase that
  makes one faster records the new figure.
- **Co-simulation:** every recorded run is replayed against the Verilog,
  as now.

## 6. Risks

- **Timing.** The v1.0 bitstream meets 75 MHz at 76.2. Overlapping adds
  muxes in front of the tiles and a busier arbiter. Phase 21 buys
  headroom first; each phase reports Fmax, and a miss is fixed in that
  phase.
- **Logic.** 17,944 of 24,288 cells (74%). Doubled tiles and the
  controller's overlap might take 1,500 to 2,500 cells. If it gets short,
  the built-in scene can shrink (a shorter cube animation frees block RAMs
  and some logic), as agreed for v1.1.
- **Block RAM.** 34 of 56 in use; the flags need 2 and the doubled
  address tiles fit in the ones they use. Room to spare.
- **Correctness of overlap.** A block's depth read must see the previous
  block's writes when they are the same block (two triangles in a row
  touching it). The rasteriser walks a triangle's blocks in order, so the
  same block comes back only for the next triangle; the overlap must
  wait for the write when it does, and Phase 24's tests check it.
- **Fast clear and memory access.** `MEM_DATA` reads of the GPU's buffers
  are part of the interface (frames read back); the flags must cover them
  exactly, and Phase 22's test reads back cleared, partly drawn and
  depth-only-cleared buffers.
