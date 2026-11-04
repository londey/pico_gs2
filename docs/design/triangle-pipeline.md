# Triangle rendering: plan and design note

**Status:** Phase 6 (the brief's Phase 0). Approved 2026-11-04, with tiled
8×8 blocks (§6.4) and Sinter v1.0.0-rc.2. No functional RTL.
**Source:** [setup-engine-brief.md](setup-engine-brief.md) (the brief), the
interview of 2026-10-31, and spike measurements made against Sinter
v1.0.0-rc.1, rechecked on v1.0.0-rc.2. Decisions are recorded as ADRs in
[../adr/](../adr/).

The brief specifies the setup engine in full; this note doesn't repeat it.
It records what was decided, what Sinter can do today, the interfaces
between the new blocks, and the phases.

## 1. Decisions

| Brief | Decision | ADR |
|---|---|---|
| D1 | Microcoded setup (Option A): Rust generator and scheduler, 64-bit micro-ROM, Sinter datapath. Option B (`multiplex fn`) is dropped. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D2 | One MUL slot and one ALU slot. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D3 | LUT RAM for the vertex ring, register banks and output buffers. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D5 | Setup computes all 8 attributes (R, G, B, A, Z, S′, T′, Q). Traversal uses R, G, B and Z for now. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D8 | BACK culls area < 0 in y-down screen space. Strips alternate winding through the role-map flip. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D9 | 8×8 blocks at 1 pixel per clock through an on-chip tile buffer. Colour and Z buffers are tiled, so a block is two 32-word SDRAM runs in one row. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| D10 | Immediate mode. Revisit after the bandwidth is measured on the board. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| D11 | Round half up everywhere (Sinter's `round` does this). | [0001](../adr/0001-microcoded-setup-engine.md) |
| D12 | One clock: setup, traversal, SDRAM and scan-out all on the 75 MHz core clock. The host link crossing comes with the host link. | — |
| D13 | Generated files are produced at build time only, never committed. | [0004](../adr/0004-generated-files-at-build-time.md) |
| — | Triangles come from a built-in test scene in ROM until the SPI host link exists. | [0002](../adr/0002-built-in-test-scene.md) |
| — | 16-bit Z buffer, triple-buffered colour. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| — | A Sinter simulator vs Verilator oracle: `sinter sim --cosim` on every stimulus test, including stimulus recorded from the Rust testbenches. | [0005](../adr/0005-simulator-verilator-oracle.md) |
| F8 | Datapath registers, register banks and buffers are `#[no_reset]`; only control state is reset. | [0006](../adr/0006-datapath-reset-stopgap.md) |
| — | Code uses Sinter's fixed-point notation; the brief's is translated. | [0007](../adr/0007-fixed-point-notation.md) |

## 2. Hardware (brief §1, confirmed)

- FPGA: **LFE5U-25F-6BG256C** (from the board schematics in
  `external/icepi-zero`), speed grade 6, nextpnr's default.
- SDRAM: W9825G6KH, 4 banks × 8192 rows × 512 columns × 16 bits (32 MB).
- Host: RP2350, not yet connected (ADR 0002).
- Core clock: 75 MHz, also the 720p pixel clock.

## 3. Sinter readiness (brief §6)

Measured on v1.0.0-rc.1 with a spike design (a pipelined 36×18 multiply, a
32 × i36 register bank with 4 read ports, a 1024 × u18 seed ROM from
`include_hex!`, a 54-bit runtime arithmetic shift), synthesised alone with
`synth_ecp5` and placed with nextpnr at 75 MHz. v1.0.0-rc.2 added the
features marked *rc.2*; `--cosim` was checked on the current design.

| # | Feature | Status | Notes |
|---|---|---|---|
| F1 | Fixed point | Available | `iI.F` / `uI.F`, with `I` **including** the sign bit (ADR 0007). Casts: `wrap`, `trunc`, `sat`, `round` (halves up). |
| F2 | Runtime shift | Available | `x >> amount`, arithmetic for signed `x`; amount unsigned. |
| F3 | Ported memories, ROM init | Available | `mem<T, N, R>`: R read ports, one write port, 1-cycle read. `rom<T, N> = include_hex!(..)`. *rc.2:* `#[storage(regs \| lutram \| bram)]` (LUT RAM up to 256 words). Without a hint the spike's 4-port bank went to LUT RAM, the seed ROM to one DP16KD. |
| F4 | Primitives | Available | `*` on a 36×18 product infers **2 MULT18X18D**, with the combine in carry-chain logic (no ALU54B). *rc.2:* `#[params(..)]` sets Verilog parameters on a `#[verilog]` binding, if a primitive is ever needed. |
| F5 | Pipelined unit | Available | `pipeline fn` with fixed latency. The spike's 3-stage multiply ran at **107 MHz** once accumulate and shift were in stages of their own. |
| F6 | Shared resource check | Not needed | `multiplex fn` shares one `*` per phase but completes it inside the phase; Option B is dropped (ADR 0001). |
| F7 | Clock domains, CDC | Available | `async_fifo` reaches only about 85 MHz; not needed until the host link. |
| F8 | Reset opt-out | Available | *rc.2:* `#[no_reset]` on a field gives its registers (and a memory's read registers) a power-on value only. ADR 0006. |
| F9 | Const generics | Partial | *rc.2:* `-D NAME=VALUE` overrides a `pub const` at build time. No generics; the generator owns `N_MUL`, `N_ALU` and the latencies. |
| F10 | `Option` | Available | |
| F11 | Rust build-step interop | Available | Data through `include_hex!`; constants shared by a checked hand-written `.snt` (§6.3). |
| F12 | Rust simulator API | Available | Already used by `sim/tb`. |
| — | Co-simulation | Available | *rc.2:* `sinter sim --cosim` runs the Verilog under Verilator over the same stimulus and fails on the first difference. It passes on the current `Gpu` (20,000 random cycles) and the three directed tests. ADR 0005. |

What's still missing is listed in
[../sinter-requirements.md](../sinter-requirements.md).

## 4. Block diagram

```
ROM scene ─▶ command parser ─▶ vertex ring ─▶ SETUP ─▶ output buffers ─▶ TRAVERSAL ─▶ tile buffer ─┐
(later: SPI)  (packets)        (4 slots)      (µcode)   (2 × 64 slots)    (8×8 blocks,  (colour + Z, │
                                                                         iterators)    8×8)        │
                                                                                                   ▼
      DVI ◀─ encoders ◀─ half-block ring ◀───────── SDRAM arbiter ◀── clears, block reads/writes ───┘
                         (192 × 8×4)               (scan-out first)
```

Everything runs on the core clock. Scan-out's line buffer becomes a
ring of half blocks (§6.4) and gains a buffer select (the front buffer).
The SDRAM controller keeps its runs of up to 32 words and gains a write mask
per word;
an arbiter in front of it gives scan-out priority.

## 5. Number formats (brief §4.3 in Sinter notation)

| Quantity | Brief | Sinter | Bits |
|---|---|---|---|
| x, y | s11.4 | `i12.4` | 16 |
| dx, dy, A, B, cx, cy | s12.4 / s13.4 | `i13.4` / `i14.4` | ≤ 18 |
| area, Es | s24.8 | `i25.8` | 33 |
| m, x0, r | u1.17 / u0.18 | `u1.17` / `u0.18` | 18 |
| k1–k4 | 18-bit mantissa | `i18`, exponent E implicit | 18 |
| w1, w2 | ≤ 36 bits | `i36`, exponent E implicit | 36 |
| R, G, B, A | u8.0 | `u8` | 8 |
| Z | u16.0 | `u16` | 16 |
| S′, T′ | (D6) | `i16`, provisional | 16 |
| Q | (D6) | `u0.16`, provisional | 16 |
| Iterator start, dx, dy | s15.16 | `i16.16` | 32 |

S′, T′ and Q are placeholders until the texture unit is designed (D6); their
deltas fit the 18-bit operand, so D7 doesn't arise yet. The generator's
range analysis is authoritative and may widen any of these.

## 6. Interfaces

These are defaults, refined in the phase that builds each block.

### 6.1 Command packets

32-bit words, from the scene ROM now and the SPI link later. As built in
Phase 11 (`models/setup/src/scene.rs`, `hw/raster.snt`), a command is a
header word, then its vertices:

| Header bits | Field |
|---|---|
| 31:28 | opcode: 1 strip, 2 fan, 3 clear, 4 end of frame, 15 end of scene (loop) |
| 27:26 | cull mode: 0 none, 1 back, 2 front |
| 25 | 1 for the untextured program |
| 15:0 | vertices that follow (3 or more); for a clear, the RGB565 colour |

A vertex is four words: `x @ y` (two `Q12.4` counts), `R @ G @ B @ A`,
`Z @ Q`, `S′ @ T′`. A strip's vertex `i` makes triangle `(i-2, i-1, i)`,
with roles 1 and 2 swapped on odd triangles so that a strip winds one way;
a fan's makes `(0, i-1, i)`. Strips cycle through ring slots 0–3, fans keep
slot 0 for their first vertex and cycle 1–3, so the next vertex always loads
into a slot the running triangle isn't using. A new primitive starts again
from slot 0, so the parser waits for the ring to be free first.

A clear (Phase 12) sets the frame being drawn to its colour and its depths
to the farthest, 0xFFFF; it waits for the last frame's triangles to finish.
An end of frame waits for the frame's last blocks to be written and for
the display to have taken the frame before, then hands the frame over.

### 6.2 Setup output record

The brief's 37 slots (E01, E12, E20, A01..B20, xs, ys, xmax, ymax, and
start / dx / dy for 8 attributes) in a double-buffered output buffer. Traversal
reads the slots it uses (edges, box, R, G, B, Z) by fixed address.

### 6.3 Shared constants

The micro-word layout, opcodes and address map live in hand-written
`hw/setup/isa.snt`. The generator holds the same values, and a cargo test
checks that the `.snt` file matches them, so neither can drift without CI
failing. The micro-ROM and seed table are generated into `build/gen/` and
loaded by `include_hex!`.

### 6.4 SDRAM memory map, tiling and buffers

| Bank | Contents |
|---|---|
| 0, 1, 2 | Colour buffers: 640×360 RGB565, tiled |
| 3 | Z buffer: 640×360 `u16`, tiled |

**Tiling.** Each buffer is 80 × 45 blocks of 8×8 pixels. Block `(bx, by)`
is the 64 words from `(by * 80 + bx) * 64`, its pixels in raster order, so
pixel `(x, y)` is at word `x[9:3] @ y[2:0] @ x[2:0]` within its block row,
plus `y[8:3] * 5120`. A block never crosses an SDRAM row (512 words). Its
top half (lines 0–3) is words 0–31 and its bottom half words 32–63, so a
**half block** (8×4) is one 32-word run, and a whole block is two runs in the
same row.

**Scan-out** reads half blocks into a ring of 192 half-block slots (6,144
words, 6 DP16KD). A *half-block row* is 4 framebuffer lines (8 screen
lines), 80 half blocks, read as 80 runs of 32 words at a stride of 64. The
loader fills slots in order as long as fewer than 192 are in use; the
display frees a half-block row's 80 slots when it has shown that row's last
line. Two rows fit with 32 slots to spare, so loads can run ahead of the
display by more than a row. Slot `s` holds its half block's pixels at words
`s @ py @ px`, so a screen pixel's word is a concatenation once the row's
first slot is known. Loading a half-block row takes about 3,000 cycles
(est.) out of the 13,200 the display spends on it: about 23% of the bus, as
for today's line loads. The ring restarts at slot 0 at the end of each
frame's active area, so a buffer swap at vertical blank (triple buffering)
never mixes two buffers in the ring.

**Triple buffering.** One buffer is displayed, at most one is complete and
waiting, and the renderer draws into the third. At each vertical blank the
display takes the waiting buffer, if there is one; the renderer never waits
for vertical blank, only for a free buffer.

### 6.5 Tile buffer and traversal

For each 8×8 block the bounding box touches:

1. Test the block's corners against the three edges; skip the block if it's
   wholly outside one.
2. Read the block's Z: two runs of 32 words.
3. Step the edge functions and R, G, B, Z one pixel per clock; on coverage
   (top-left rule) and Z less-than, write colour and Z into the tile buffer
   and set the pixel's mask bit.
4. Write back colour and Z: two runs of 32 words each, with the per-word
   mask (DQM) leaving uncovered and Z-rejected pixels as they were, so the
   colour buffer is never read.

That's about 230 cycles of SDRAM time per block (est.), about 3.6 per pixel,
against about 310 for 8-word runs. Raising the run limit to 64 would save
about 20 cycles a block; it can wait for measurements. Overlapping step 3 of one block with the
SDRAM traffic of others, and skipping the Z read for a block the clear
hasn't touched since, are optimisations for later, once frames are
measured.

**SDRAM controller change:** a write mask per word, taken with the word on
`wtake` and driven on DQM. Runs stay at up to 32 words, so the refresh
timing doesn't change.

**As built in Phase 11** (no Z yet, so no Z read): the rasteriser copies 22
slots (edges, box, R, G, B planes) out of the engine's committed output
buffer, then releases it, so the next triangle's setup overlaps this one's
traversal. Per block: TEST (each edge's largest value over the block's
corners, from a per-triangle corner offset), DECIDE a cycle later, 64 pixel
cycles into the tile buffer, then one masked 32-word run per half block
with a covered pixel. Colour is each plane's iterator rounded to 5/6/5 bits
and clamped. The arbiter registers the request it puts in front of the
controller (fill, then scan-out first, then the rasteriser). The test card
(48 triangles, 47 kept, about 94,500 pixels) draws in 579,360 cycles from
reset, under half a frame after the fill.

**As built in Phase 12:** a 16-bit depth buffer in bank 3 and three colour
buffers in banks 0 to 2, at the same tiled addresses. The rasteriser also
copies the Z plane (25 slots). A clear writes every half block's colour and
then its depth, one unmasked 32-word run each (14,400 runs). Per block not
rejected, ZREAD reads its 64 depths (two runs) into a second tile buffer;
each pixel is drawn if covered and its depth (the Z iterator over 2^16,
rounded and clamped) is less than the buffer's, and its depth goes into the
tile with its colour; then the colour runs and the depth runs go out, each
half block with a drawn pixel, masked. Buffers: one shown, one drawn, the
third finished-and-waiting or spare. At an end of frame the drawn buffer
becomes the waiting one and the spare the drawn one; at the next frame
start the waiting one is shown and the shown one becomes the spare. The
scene is the cube (32 frames a quarter turn, back faces culled) and two
triangles crossing in depth, over a dark grey clear. A frame takes
1,103,542 cycles to draw (68 per second at 75 MHz, so the display shows
every frame at 60 Hz); the clear is about half of it. The GPU places and
routes at 77.1 MHz in 9,512 logic cells, 3,391 FFs, 26 DP16KD (16 of them
the 8,192-word scene ROM) and 2 MULT18X18D.

## 7. Verification

The brief's §8 layers, with this repo's tools:

| Layer | How |
|---|---|
| f64 reference vs bit-exact interpreter | cargo tests in the generator crate |
| interpreter vs Sinter simulator | cargo tests in `sim/tb`, bit- and cycle-exact |
| Sinter simulator vs Verilator | the oracle (ADR 0005), in `just check` |
| synthesis | `just bitstream`, plus a resource report per phase |
| frames | the existing frame test, extended: decoded frames compared pixel-exactly against the reference renderer |

## 8. Phases

Each is its own branch, stacked on the previous one, ending at a gate where
I stop and report.

| Phase | Branch | Work | Gate |
|---|---|---|---|
| 6 | `phase-6-raster-plan` | This note, the ADRs, the Sinter requirements list, the brief committed | Plan approved (done) |
| 7 | `phase-7-oracle` | Sinter v1.0.0-rc.2. `--cosim` on every directed test and on stimulus recorded from the Rust frame test, in `just check`. `#[no_reset]` on the current datapath registers; reset fanout before and after. | The oracle passes in `just check`; reset fanout reported |
| 8 | `phase-8-tiled-framebuffer` | SDRAM controller: per-word write mask. Tiled colour buffer; the test pattern written block by block; scan-out through the 192-slot half-block ring. | Twin frame test pixel-exact; the test pattern on the board, as now |
| 9 | `phase-9-setup-model` | Generator crate (brief §7): op-list DSL with formats and ranges, register allocator, list scheduler, micro-ROM and seed emitters, bit-exact interpreter, f64 reference, reference traversal. All brief §8 reference-level tests: exhaustive reciprocal, random-triangle error, edge cases, watertightness, overflow, schedule determinism. | Tests pass; error and schedule reports reviewed |
| 10 | `phase-10-setup-engine` | The setup engine in Sinter: datapath, sequencer, vertex ring, output buffers. Bit- and cycle-exact against the interpreter on the corpus; oracle; synthesis report. | Exact on the corpus; resources and Fmax against brief §9 |
| 11 | `phase-11-first-triangles` | Command parser and a static ROM scene; traversal, iterators and tile buffer; SDRAM arbiter; Gouraud colour into one framebuffer, no Z. | Frames pixel-exact against the reference renderer; triangles on the board |
| 12 | `phase-12-depth-and-buffers` | Z buffer, clear, triple buffering, the spinning cube with overlapping triangles (ADR 0002). | Pixel-exact frames; spinning cube on the board; frame rate measured |

Perspective-correct texturing (brief phase 5) and tiling into on-chip
tiles (brief phase 7) aren't in this plan. The Option B experiment (brief
phase 6) is dropped (ADR 0001). The
SPI host link is a phase of its own, to be scheduled.

## 9. Risks

- **SDRAM bandwidth.** Per-run overhead is about 6 cycles in 38 with 32-word
  runs. Clearing colour and Z is about 7,200 runs, roughly 0.5M cycles
  (6.7 ms) a frame. Measured in phase 12 before anything is optimised.
- **Block RAM.** The half-block ring takes 6 DP16KD, against 2 for the
  line buffer; with the scene ROM (8) and setup (3) that's about 17 of 56
  before textures.
- **Scene ROM size.** Precomputed cube frames take block RAM: about 120 words
  per frame, so 32 frames is about 4K words, or 8 DP16KD (est.). The
  rotation is about the vertical axis, so a quarter turn loops seamlessly.
- **LUT RAM ports.** Four read ports on each 32 × 36 bank cost 4 copies;
  the spike used 64 DPR16X4 for one bank. Two banks plus the vertex ring and
  output buffers may reach 1,000 LUTs or more.
