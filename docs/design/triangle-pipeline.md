# Triangle rendering: plan and design note

**Status:** Phase 6 (the brief's Phase 0). For approval; no functional RTL yet.
**Source:** [setup-engine-brief.md](setup-engine-brief.md) (the brief), the
interview of 2026-10-31, and spike measurements made against Sinter
v1.0.0-rc.1. Decisions are recorded as ADRs in [../adr/](../adr/).

The brief specifies the setup engine in full; this note doesn't repeat it.
It records what was decided, what Sinter can do today, the interfaces
between the new blocks, and the phases.

## 1. Decisions

| Brief | Decision | ADR |
|---|---|---|
| D1 | Microcoded setup (Option A): Rust generator and scheduler, 64-bit micro-ROM, Sinter datapath. Option B (`multiplex fn`) is not planned. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D2 | One MUL slot and one ALU slot. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D3 | LUT RAM for the vertex ring, register banks and output buffers. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D5 | Setup computes all 8 attributes (R, G, B, A, Z, S′, T′, Q). Traversal uses R, G, B and Z for now. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D8 | BACK culls area < 0 in y-down screen space. Strips alternate winding through the role-map flip. | [0001](../adr/0001-microcoded-setup-engine.md) |
| D9 | 8×8 blocks at 1 pixel per clock through an on-chip tile buffer. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| D10 | Immediate mode. Revisit after the bandwidth is measured on the board. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| D11 | Round half up everywhere (Sinter's `round` does this). | [0001](../adr/0001-microcoded-setup-engine.md) |
| D12 | One clock: setup, traversal, SDRAM and scan-out all on the 75 MHz core clock. The host link crossing comes with the host link. | — |
| D13 | Generated files are produced at build time only, never committed. | [0004](../adr/0004-generated-files-at-build-time.md) |
| — | Triangles come from a built-in test scene in ROM until the SPI host link exists. | [0002](../adr/0002-built-in-test-scene.md) |
| — | 16-bit Z buffer, triple-buffered colour. | [0003](../adr/0003-pixel-path-and-buffers.md) |
| — | A Sinter simulator vs Verilator oracle, built in this repo. | [0005](../adr/0005-simulator-verilator-oracle.md) |
| F8 | Datapath registers are reset for now; the reset net is watched. | [0006](../adr/0006-datapath-reset-stopgap.md) |
| — | Code uses Sinter's fixed-point notation; the brief's is translated. | [0007](../adr/0007-fixed-point-notation.md) |

## 2. Hardware (brief §1, confirmed)

- FPGA: **LFE5U-25F-6BG256C** (from the board schematics in
  `external/icepi-zero`), speed grade 6, nextpnr's default.
- SDRAM: W9825G6KH, 4 banks × 8192 rows × 512 columns × 16 bits (32 MB).
- Host: RP2350, not yet connected (ADR 0002).
- Core clock: 75 MHz, also the 720p pixel clock.

## 3. Sinter readiness (brief §6)

Measured with a spike design (a pipelined 36×18 multiply, a 32 × i36
register bank with 4 read ports, a 1024 × u18 seed ROM from
`include_hex!`, a 54-bit runtime arithmetic shift), synthesised alone with
`synth_ecp5` and placed with nextpnr at 75 MHz.

| # | Feature | Status | Notes |
|---|---|---|---|
| F1 | Fixed point | Available | `iI.F` / `uI.F`, with `I` **including** the sign bit (ADR 0007). Casts: `wrap`, `trunc`, `sat`, `round` (halves up). |
| F2 | Runtime shift | Available | `x >> amount`, arithmetic for signed `x`; amount unsigned. |
| F3 | Ported memories, ROM init | Partial | `mem<T, N, R>`: R read ports, one write port, 1-cycle read. `rom<T, N> = include_hex!(..)`. No LUT-RAM / block-RAM hint: the spike's 4-port bank went to LUT RAM (TRELLIS_DPR16X4), the seed ROM to one DP16KD. |
| F4 | Primitives | Partial | `*` on a 36×18 product infers **2 MULT18X18D**, with the combine in carry-chain logic (no ALU54B). `#[verilog]` bindings can't set Verilog parameters, so a primitive needs a hand-written wrapper module. Inference is enough for now. |
| F5 | Pipelined unit | Available | `pipeline fn` with fixed latency, registers without reset. The spike's 3-stage multiply ran at **107 MHz** once accumulate and shift were in stages of their own. |
| F6 | Shared resource check | Partial | `multiplex fn` shares one `*` per phase but completes it inside the phase; not needed for Option A. |
| F7 | Clock domains, CDC | Available | `async_fifo` reaches only about 85 MHz; not needed until the host link. |
| F8 | Reset opt-out | **Missing** | Every register needs a reset value, and the generated Verilog resets each one asynchronously (`negedge rst_n`). ADR 0006. |
| F9 | Const generics | Missing | Constants only. `N_MUL`, `N_ALU` and the latencies are generator parameters, so Sinter needs constants, not generics. |
| F10 | `Option` | Available | |
| F11 | Rust build-step interop | Available | Data through `include_hex!`; constants shared by a checked hand-written `.snt` (§6.3). |
| F12 | Rust simulator API | Available | Already used by `sim/tb`. |
| — | Co-simulation | Partial | `sinter build` writes `<Module>_tb.cpp`, a Verilator harness that reads one `u64` per input per cycle and writes one per output, but nothing runs it against the simulator. ADR 0005. |

Gaps, with their pico_gs2 use cases, are listed in
[../sinter-requirements.md](../sinter-requirements.md).

## 4. Block diagram

```
ROM scene ─▶ command parser ─▶ vertex ring ─▶ SETUP ─▶ output buffers ─▶ TRAVERSAL ─▶ tile buffer ─┐
(later: SPI)  (packets)        (4 slots)      (µcode)   (2 × 64 slots)    (8×8 blocks,  (colour + Z, │
                                                                         iterators)    8×8)        │
                                                                                                   ▼
      DVI ◀─ encoders ◀─ line buffer ◀──────────── SDRAM arbiter ◀── clears, tile reads/writes ─────┘
                                                  (scan-out first)
```

Everything runs on the core clock. The existing scan-out keeps its line
buffer and gains a buffer select (front buffer). The SDRAM controller is
unchanged; an arbiter in front of it gives scan-out priority.

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

32-bit words, from the scene ROM now and the SPI link later:

| Word | Meaning |
|---|---|
| `0x1` header: `kind`, `cull`, `program` | Start a strip (`kind` 0) or fan (`kind` 1); `cull` NONE/BACK/FRONT; `program` full/untextured/flat. Followed by vertices. |
| vertex, 4 words | `x @ y` (two `i12.4`), `R @ G @ B @ A`, `Z @ Q`, `S′ @ T′` |
| `0x2` end of primitive | |
| `0x3` clear | Clear the back buffer's colour (to a given RGB565) and Z (to 0xFFFF). |
| `0x4` end of frame | Hand the back buffer to the display (§6.4). |

The first three vertices of a strip or fan make a triangle; each later
vertex makes one more, overwriting the ring slot the brief §4.1 names.

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

### 6.4 SDRAM memory map and buffers

| Bank | Contents |
|---|---|
| 0, 1, 2 | Colour buffers: 640×360 RGB565, word `y * 640 + x`, from row 0 |
| 3 | Z buffer: 640×360 `u16`, same layout |

One buffer per bank keeps addressing trivial and leaves room to interleave
banks later. Triple buffering: one buffer is displayed, at most one is
complete and waiting, and the renderer draws into the third. At each
vertical blank the display takes the waiting buffer, if there is one; the
renderer never waits for vertical blank, only for a free buffer.

### 6.5 Tile buffer and traversal

For each 8×8 block the bounding box touches:

1. Test the block's corners against the three edges; skip the block if it's
   wholly outside one.
2. Read the block's 64 Z values from SDRAM (8 runs of 8 words).
3. Step the edge functions and R, G, B, Z one pixel per clock; on coverage
   (top-left rule) and Z less-than, write colour and Z into the tile buffer.
4. Write back the covered rows' colour and Z (8 runs each, with per-word
   DQM masks for uncovered pixels).

The SDRAM controller opens and closes a row per run, so an 8-word run costs
about 13 cycles: a block is roughly 300 cycles of SDRAM time (est.), about
5 cycles per pixel. Scan-out takes about 18% of the bus. The renderer
overlapping step 3 of one block with steps 2 and 4 of others, bank
interleaving and longer runs are optimisations for later, once frames are
measured.

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
| 6 | `phase-6-raster-plan` | This note, the ADRs, the Sinter requirements list, the brief committed | Plan approved |
| 7 | `phase-7-oracle` | Sinter vs Verilator oracle for the current `Gpu` and its modules, in `just check`. Find out what Yosys and nextpnr do with the async reset and whether the ECP5's GSR can carry it. | The oracle passes on the current design; reset findings reported |
| 8 | `phase-8-setup-model` | Generator crate (brief §7): op-list DSL with formats and ranges, register allocator, list scheduler, micro-ROM and seed emitters, bit-exact interpreter, f64 reference, reference traversal. All brief §8 reference-level tests: exhaustive reciprocal, random-triangle error, edge cases, watertightness, overflow, schedule determinism. | Tests pass; error and schedule reports reviewed |
| 9 | `phase-9-setup-engine` | The setup engine in Sinter: datapath, sequencer, vertex ring, output buffers. Bit- and cycle-exact against the interpreter on the corpus; oracle; synthesis report. | Exact on the corpus; resources and Fmax against brief §9 |
| 10 | `phase-10-first-triangles` | Command parser and a static ROM scene; traversal, iterators and tile buffer; SDRAM arbiter; Gouraud colour into one framebuffer, no Z. | Frames pixel-exact against the reference renderer; triangles on the board |
| 11 | `phase-11-depth-and-buffers` | Z buffer, clear, triple buffering, the spinning cube with overlapping triangles (ADR 0002). | Pixel-exact frames; spinning cube on the board; frame rate measured |

Perspective-correct texturing (brief phase 5), the Option B experiment
(phase 6) and tiling (phase 7) aren't in this plan. The SPI host link is a
phase of its own, to be scheduled.

## 9. Risks

- **SDRAM bandwidth.** The 8-word runs waste about 40% of the bus. Measured
  in phase 11 before anything is optimised.
- **Reset fanout.** Every register is on `rst_n` until Sinter can opt out
  (ADR 0006).
- **Scene ROM size.** Precomputed cube frames take block RAM: about 120 words
  per frame, so 32 frames is about 4K words, or 8 DP16KD (est.). The
  rotation is about the vertical axis, so a quarter turn loops seamlessly.
- **LUT RAM ports.** Four read ports on each 32 × 36 bank cost 4 copies;
  the spike used 64 DPR16X4 for one bank. Two banks plus the vertex ring and
  output buffers may reach 1,000 LUTs or more.
