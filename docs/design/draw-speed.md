# v1.1: drawing speed

The plan for v1.1: the same interface as v1.0 (interface version 2,
[host-interface.md](host-interface.md)), drawn several times faster. No
register, field or behaviour a host can see changes, except how long
things take. ADR [0010](../adr/0010-v1-1-draw-speed.md) records the
decision.

**Status:** approved 2026-11-10 (Phase 20); Phases 21 and 22 done, and
23 in simulation (§7): **the goal is met** (746,840 cycles, 10 ms). Phases 23 and 24 swapped after Phase 22 (§7): the SDRAM
work counts in full only once blocks overlap.

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

A *cleared* flag for every 8 × 8 block of every buffer, addressed by the
block's SDRAM bank and block number: `bank @ block`, 2 + 12 bits (3,600
blocks a buffer), 16,384 × 1 bit, exactly one DP16KD (two if scan-out and
the rasteriser need a read port each). Banks 0 to 2 are the colour
buffers and bank 3 the depth buffer, so every reader looks its flag up
from the SDRAM address it already has, with no mapping of which buffer is
shown, waiting or drawn. A `FRAME` clear sets the drawn buffer's flags
(and depth's), 3,600 cycles each, and records that buffer's clear colour
(one for each colour buffer). Then everything that reads a buffer honours
the flags:

- **Depth read:** a cleared block reads as 0xFFFF without touching SDRAM.
  Most blocks are touched first after a clear, so most depth reads go
  away.
- **Writes:** the first write to a cleared block writes all of it,
  unmasked, its undrawn pixels with the clear colour (or 0xFFFF), and
  clears its flag. A first touch that draws in only one half writes the
  other half too, one extra 32-word run (none once Phase 24 writes a
  block as one 64-word run).
- **Scan-out:** a cleared block's half blocks fill their ring slots with
  the buffer's clear colour without an SDRAM read, so a sparse frame
  frees most of scan-out's 23% for drawing.
- **`MEM_DATA` reads** of a buffer's words return the clear colour (or
  0xFFFF) where the flag is set, so a host reading a frame back sees
  exactly what v1.0 shows. (`MEM_DATA` writes into the GPU's buffers stay
  undefined, as in v1.0 §9.2; they clear no flag.)

Every colour buffer needs its flags, not only the one being drawn: with
triple buffering the buffer on screen and the one waiting were cleared
and partly drawn too, and scan-out and `MEM_DATA` must see their
untouched blocks as the clear colour. Flags for the drawn buffer alone
would need a pass at each present writing the clear colour into its
untouched blocks, which for a sparse frame gives back most of the colour
clear (up to 4.5 ms).

### 3.2 SDRAM throughput (Phase 24)

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

### 3.3 Overlapped blocks (Phases 23 and 25)

- **Phase 23, two sets of tile buffers:** the colour, depth and shade
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
| 20 | `phase-20-draw-plan` | This plan and ADR 0010 | Done: approved 2026-11-10 |
| 21 | `phase-21-groundwork` | Timing headroom back above 80 MHz (registering scan-out's `presenting` path, the critical path at v1.0); `just perf` with SDRAM bus use by requester, and a regression check in `just check` against recorded figures; every twin test asserts the display never ran short; the v1.1 goal scene in `just perf` | Done: 83.5 MHz; the perf check in `just check`; every figure unchanged; on the board |
| 22 | `phase-22-fast-clear` | Cleared flags per block (§3.1): clear, depth reads, writes, scan-out, `MEM_DATA` reads | Done: pixel-exact; `fast_clear.rs`; the clear 3,603 cycles; 14 mutations caught; 83.2 MHz; on the board, three read-backs pixel-exact |
| 23 | `phase-23-overlap-writes` | Two sets of tile buffers: one block's writes overlap the next block's depth read and passes (§3.3) | Done in simulation: pixel-exact; `overlap.rs`; 10 mutations caught; 81.0 MHz; the goal met. On the board: to do |
| 24 | `phase-24-sdram` | Overlapped runs in other banks, auto-precharge, 64-word runs (§3.2) | No SDRAM model violations; pixel-exact; scan-out never short; per-block SDRAM time measured; on the board |
| 25 | `phase-25-overlap-passes` | The pixel pass of one block overlaps the texel pass of the one before (§3.3) | Pixel-exact; mutations; **the goal scene at 60 fps** |
| 26 | `phase-26-v1-1` | Performance figures (host-interface §15 updated), resources, slides; v1.1 | Approved |

**Estimates**, per block a triangle touches (§2's 460 cycles today) and
for the goal scene (one full-screen layer, 3,600 blocks, after a clear):

| After | Clear | Per block, first touch | Per block, again | Goal scene |
|---|---|---|---|---|
| v1.0 | 681,000 | ~460 | ~460 | 2,460,000 (32.8 ms) |
| 22, fast clear (measured) | 3,603 | ~344 (no depth read) | ~440 | 1,279,202 (17.1 ms) |
| 23, writes overlapped | 3,603 | ~190 | ~280 | ~750,000 (10 ms) |
| 24, SDRAM | 3,603 | ~165 | ~240 | ~650,000 |
| 25, passes overlapped | 3,603 | ~150 | ~200 | ~600,000 |

(Rows 23 to 25 re-estimated after Phase 22, from its measurements and the
controller's 5 cycles a run, §7.) So the goal should be met by Phase 23
for one layer; Phases 24 and 25 buy the room for overdraw, more triangles
and textures that miss. If a phase
falls well short of its estimate, I report before going on.

## 5. Verification

- **Correctness:** every existing twin test stays pixel-exact against the
  reference renderer, which doesn't change: v1.1 changes when pixels are
  written, never what. Each phase adds the tests its change needs (Phase
  22: read-backs of cleared buffers; Phase 24: back-to-back runs in each
  bank pairing; Phases 23 and 25: blocks whose later triangle depends on
  the earlier one's depth, so an overlap that reads stale depth fails),
  and deliberate breaks of its new logic, each caught.
- **The SDRAM model** checks every datasheet rule on every cycle of every
  test, which matters most in Phase 24.
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
- **Block RAM.** 34 of 56 in use; the flags need 1 or 2 and the doubled
  address tiles fit in the ones they use. Room to spare.
- **Correctness of overlap.** A block's depth read must see the previous
  block's writes when they are the same block (two triangles in a row
  touching it). The rasteriser walks a triangle's blocks in order, so the
  same block comes back only for the next triangle; the overlap must
  wait for the write when it does, and Phase 23's tests check it.
- **Fast clear and memory access.** `MEM_DATA` reads of the GPU's buffers
  are part of the interface (frames read back); the flags must cover them
  exactly, and Phase 22's test reads back cleared, partly drawn and
  depth-only-cleared buffers.

## 7. Results

### Phase 21: groundwork

- **Timing:** the end of a half-block row and of the frame were decoded
  from the beam's position and fed scan-out's next state and the
  arbiter's request in the same cycle. They are now registers decoded a
  pixel early (the beam's previous column on the same line), which changes
  no cycle's behaviour. The GPU places at 83.5 MHz (76.2 at v1.0), in
  17,939 cells, 8,202 flip-flops, 34 DP16KD and 12 multipliers. The
  critical path is now the texture cache's: an address tile read through
  the fill check to a texel bank's address (6.8 ns logic, 5.2 routing).
- **`just perf`** also reports the SDRAM's use over a frame by requester,
  and prints the v1.1 goal's figure against 60 fps. `just perf --record`
  writes every scene's figure to `sim/tb/perf-baseline.txt`, and `just
  check` runs `perf --check`, which fails if a scene is more than 0.5%
  slower than recorded (checked by lowering one recorded figure), or
  link-bound, or the display ran short.
- **The display never runs short:** the host, memory, texture and
  combiner twin tests now assert it, as the frame test did.
- **Every figure is v1.0's**, to the cycle (host-interface §15).
- **On the board:** `gs-host --status --screenshot` read the built-in
  scene back from the 83.5 MHz bitstream, pixel for pixel its frame 14,
  with `SCRATCH`'s round trips and a frame drawn in 1,252,161 cycles, as
  before.

The SDRAM over a frame, from `just perf`: during the full-screen quad it
is in no run for 28% of the frame (the pixel and texel passes, with the
bus idle), drawing 52% and scan-out 20%; during the clear it is
saturated (74% clearing, 20% scan-out, 6% between runs).

| Scene | scan-out | drawing | no run |
|---|---|---|---|
| clear only | 19.7% | 74.0% | 6.3% |
| full screen, untextured | 19.8% | 52.8% | 27.5% |
| full screen, 64 × 64 bilinear (the goal) | 19.8% | 51.6% | 28.6% |
| 320 triangles of 8 px, untextured | 20.0% | 67.3% | 12.7% |
| the cube, bilinear | 19.7% | 59.4% | 20.9% |

### Phase 22: fast clear

- **As built:** the cleared flags are a word per block with a bit per
  bank (`cflags`, 4,096 × 4 bits, two read ports, so 2 DP16KD), in the
  rasteriser, which is their only writer. A clear is one pass over the
  3,600 blocks, reading each word and writing it back a cycle later with
  the cleared buffers' bits set: 3,603 cycles for colour and depth alike
  (681,114 before). A block's flags are read in TEST; a cleared depth
  skips the depth read and tests against 0xFFFF; a cleared buffer's runs
  are both written, unmasked, the clear value where nothing was drawn;
  its bit goes once both are out, and the word is written back in NEXT.
  Scan-out reads the flags of its next load's block and skips a cleared
  one (its ring slot marked, shown as the buffer's clear colour) when no
  load is waiting or in flight. A `MEM_DATA` read of a cleared block
  reads the clear value.
- **Timing:** the first build placed at 52 MHz: scan-out used the flag
  RAM's output to move its next load, whose address was the RAM's next
  read address, a loop through the RAM in one cycle. Scan-out now reads
  the flags at its current load's address and registers the bit; a load
  waits until the bit is its own, two cycles after the address moved.
  83.2 MHz, in 18,317 cells (75%), 8,263 flip-flops, 36 DP16KD and 12
  multipliers (+378 cells, +61 flip-flops, +2 DP16KD). The critical path
  is a `MEM_DATA` read's substitution, from the flag RAM to the parser's
  read register.
- **Tests:** every twin test stays pixel-exact. `frame.rs` read the shown
  bank from the SDRAM after the run; the SDRAM no longer holds cleared
  blocks, and with a 3,603-cycle clear the GPU starts drawing into that
  buffer within the blanking, so the test now reads the buffer as the
  GPU does (`Bench::buffer_word`, through the flags) at the frame's
  start, while it is on screen. A new test, `fast_clear.rs`, reads back
  through `MEM_DATA` a block drawn in its top half only, blocks drawn
  with colour writes or depth writes off, and an untouched block, of the
  colour and depth buffers, after a full clear and after a depth-only
  clear; then shows three frames with different clear colours, the last
  clearing colour only. 14 deliberate breaks, each caught: the depth
  test ignoring the flag; a cleared run masked; a cleared block written
  in one half; the flag kept after the write; the word not written back;
  the clear stopping early; a colour-only clear clearing depth; every
  clear colour stored as bank 0's; scan-out ignoring the flags, using
  them a cycle early, skipping with a load in flight, or showing the
  drawn buffer's clear colour; a `MEM_DATA` read using bank 0's flag, or
  0xFFFF for bank 2.

| Scene | Phase 21 | Phase 22 |
|---|---|---|
| clear (colour and depth, or depth only) | 681,114 | 3,603 |
| full screen, untextured | 2,407,224 | 1,226,275 |
| **full screen, 64 × 64 bilinear (the goal)** | **2,460,609 (32.8 ms)** | **1,279,202 (17.1 ms)** |
| full screen, 256 × 256 minified | 4,837,631 | 3,640,073 |
| 320 triangles of 8 px | 848,203 | 122,237 |
| 120 triangles of 32 px | 1,240,490 | 389,436 |
| the cube, bilinear | 1,256,638 | 398,407 |

**On the board:** `gs-host --status --screenshot` read back the built-in
scene pixel for pixel (its frame 16), drawn in 391,732 cycles (191.5 fps;
1,252,161 before); the streamed textured cube, 616 frames sent and 615
shown at 60.7 fps, each drawn in about 395,000 cycles, read back pixel
for pixel; so did the combiner demo's last frame.

The goal is 3.4% over a 60 fps frame (1,237,500). The estimate was about
1,350,000; it came in under because the first touch of a block skips its
depth read and, with fewer requests from scan-out for sparse rows, the
rasteriser's runs wait less. In a full-screen frame the SDRAM is now in
no run for 39% of the frame: the passes, which Phases 24 and 25 overlap.

### Phases 23 and 24 swapped

Measured after Phase 22, the SDRAM controller's overhead is 5 cycles a
run (ACTIVE, a wait, the words, write recovery, PRECHARGE, a wait: 37
cycles for a 32-word write, 36 for a read), not the 8 to 10 §2 assumed.
In the goal scene a block costs about 344 cycles: writes 183 (four runs,
148, plus about 35 waiting for scan-out's runs), the texel pass 75, the
pixel pass 64, the drain and the rest about 20. Overlapped runs and
64-word runs would save about 25 cycles a block now (7%); overlapping a
block's writes with the next block's passes would save about 150 (a
block costs about the larger of its SDRAM time and its computing, not
their sum). Once blocks overlap, the SDRAM is the limit, and the
controller's savings count in full; so the overlap comes first.

### Phase 23: overlapped writes

- **As built:** the passes hand a finished block (HANDOFF) to a writer in
  the rasteriser, with its tile bank, drawn pixels, cleared flags, block
  and the triangle's write enables, and go on to the next block; the
  writer writes its runs, then the block's flags. `tile` and `ztile` are
  128 words, two banks of a block (`ztile` with a second read port, for
  the depth test and the writer at once). The passes' runs (depth reads,
  texture blocks) and the writer's share the rasteriser's request, the
  passes' first, a run held in front of the arbiter until accepted. A
  block the writer still has is not tested again until its flags are
  written back: a later triangle's block sees the earlier one's depths
  and flags. 81.0 MHz in 18,862 cells (77%), 8,375 flip-flops, 36
  DP16KD (+545 cells, +112 flip-flops); the critical path is the writer's
  request into the arbiter, mostly routing.
- **Tests:** every twin test stays pixel-exact. Removing the wait for the
  writer passed all six of them (their consecutive triangles never
  depended on a block still being written) but failed `just perf`'s
  small-triangle scene's pixel check; so a new test, `overlap.rs`,
  draws three layers of strips of small triangles (the second, farther,
  hidden by the first's depths), and pairs of triangles over the same
  blocks at alternating depths, some writing depth only or colour only.
  10 deliberate breaks, each caught: no wait for the writer; the wait
  ending before the flags are written back; a handoff while the writer
  is busy; the passes' bank not swapped; the writer reading the passes'
  colour or depth bank; the writer using the passes' drawn pixels or
  write enables; a run not held until accepted; the flags written to the
  passes' block.

| Scene | Phase 22 | Phase 23 |
|---|---|---|
| full screen, untextured | 1,226,275 | 723,086 |
| **full screen, 64 × 64 bilinear (the goal)** | **1,279,202 (17.1 ms)** | **746,840 (10.0 ms)** |
| full screen, 256 × 256 minified | 3,640,073 | 3,255,241 |
| full screen, 256 × 256 magnified | 1,356,226 | 853,030 |
| 320 triangles of 8 px | 122,237 | 92,627 |
| 120 triangles of 32 px | 389,436 | 223,546 |
| the cube, bilinear | 398,407 | 236,596 |

The goal scene now takes 60% of a 60 fps frame. In it the SDRAM is busy
90% of the time (drawing 70%, scan-out 20%), so drawing is now limited by
the SDRAM, which is Phase 24's. Small triangles spend 17 to 19% of
their time waiting in TEST for the writer to finish a block they share;
Phase 25, or a later one, could test such a block's pixels against the
writer's tile instead of waiting.
