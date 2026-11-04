// pico-gs2 architecture slides. Build with `just slides`, which writes
// build/slides/architecture.pdf. Package versions are pinned here and in
// .devcontainer/Dockerfile (which prefetches them); keep the two in step.

#import "@preview/touying:0.8.0": *
#import themes.metropolis: *
#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge

#show: metropolis-theme.with(
  aspect-ratio: "16-9",
  footer: self => self.info.title,
  config-info(
    title: [pico-gs2: architecture and status],
    subtitle: [A PS1/N64-era GPU for the ICEpi Zero, written in Sinter],
    date: [November 2026],
  ),
)

#set text(size: 18pt)

// Status colours: built and running on the board; built and verified but
// not yet in the GPU; a Rust model; planned.
#let built-colour = rgb("#2e7d32")
#let model-colour = rgb("#1565c0")
#let planned-colour = luma(130)

// A block that is built in the design.
#let built(pos, body, ..args) = node(
  pos, body,
  stroke: 1.2pt + built-colour,
  fill: built-colour.lighten(88%),
  corner-radius: 3pt,
  shape: rect,
  ..args,
)

// A block that is built in Sinter and verified, but not yet in the GPU.
#let standalone(pos, body, ..args) = node(
  pos, body,
  stroke: (paint: built-colour, thickness: 1.2pt, dash: "dotted"),
  fill: built-colour.lighten(94%),
  corner-radius: 3pt,
  shape: rect,
  ..args,
)

// A block that exists as a Rust model, not yet in the design.
#let modelled(pos, body, ..args) = node(
  pos, body,
  stroke: 1.2pt + model-colour,
  fill: model-colour.lighten(88%),
  corner-radius: 3pt,
  shape: rect,
  ..args,
)

// A block that is planned.
#let planned(pos, body, ..args) = node(
  pos, body,
  stroke: (paint: planned-colour, thickness: 1.2pt, dash: "dashed"),
  fill: luma(246),
  corner-radius: 3pt,
  shape: rect,
  ..args,
)

// An external chip or connector.
#let chip(pos, body, ..args) = node(
  pos, body,
  stroke: 1.2pt + black,
  fill: luma(225),
  shape: rect,
  ..args,
)

// The colour key, for diagrams.
#let key = text(size: 13pt)[
  #box(fill: built-colour.lighten(88%), stroke: 1pt + built-colour, inset: 3pt)[built]
  #h(0.5em)
  #box(fill: built-colour.lighten(94%), stroke: (paint: built-colour, dash: "dotted"), inset: 3pt)[built, not yet in the GPU]
  #h(0.5em)
  #box(fill: model-colour.lighten(88%), stroke: 1pt + model-colour, inset: 3pt)[Rust model]
  #h(0.5em)
  #box(fill: luma(246), stroke: (paint: planned-colour, dash: "dashed"), inset: 3pt)[planned]
  #h(0.5em)
  #box(fill: luma(225), stroke: 1pt, inset: 3pt)[off-chip]
]

#let done = text(fill: built-colour, weight: "bold")[done]
#let next = text(fill: model-colour, weight: "bold")[next]
#let later = text(fill: planned-colour, weight: "bold")[planned]

#title-slide()

= Overview

== What pico-gs2 is

- A from-scratch rewrite of *pico-gs*: a fixed-function 3D GPU of the
  PS1/N64 era, on a small FPGA board.
- *Board:* ICEpi Zero v1.3. Lattice ECP5-25K (LFE5U-25F-6BG256C), 32 MB SDRAM
  (Winbond W9825G6KH), DVI out on mini-HDMI, an RP2350 microcontroller as the
  host.
- *Language:* Sinter. One source compiles to SystemVerilog for the FPGA *and*
  a cycle-accurate Rust simulator, the design's digital twin. There is no
  separate hand-written model.
- *Today:* 1280×720 DVI from a tiled 640×360 framebuffer in SDRAM, verified
  on the board. The triangle setup engine is built in Sinter and verified
  cycle for cycle against its Rust model; it joins the GPU, with traversal,
  for the first triangles on screen next.

#v(0.5em)
#key

== Board and clocks

#grid(
  columns: (1.15fr, 1fr),
  gutter: 1em,
  text(size: 14pt, diagram(
    spacing: (16mm, 12mm),
    node-inset: 7pt,
    chip((1, 0), [50 MHz oscillator], name: <osc>),
    planned((0, 1), [RP2350\ host], name: <host>),
    built((1, 1), [ECP5-25K\ PLL · Sinter core\ `board/top.sv`], name: <fpga>),
    chip((2, 1), [SDRAM\ 32 MB], name: <sdram>),
    chip((1, 2), [mini-HDMI (DVI)], name: <hdmi>),
    edge(<osc>, <fpga>, "-|>"),
    edge(<host>, <fpga>, "--|>", [SPI]),
    edge(<fpga>, <sdram>, "<|-|>", [16 bit]),
    edge(<fpga>, <hdmi>, "-|>", [4 TMDS pairs, 750 Mb/s], label-side: left),
  )),
  [
    #set text(size: 15pt)
    #table(
      columns: (auto, auto, 1fr),
      stroke: 0.5pt + luma(180),
      [*Clock*], [*MHz*], [*Drives*],
      [`clk_core`], [75], [the whole Sinter core; also the 720p pixel clock],
      [`clk_sdram`], [75], [the SDRAM: `clk_core` lagging 270°],
      [`clk_eclk`], [375], [ODDRX2F serialisers, 2 bits per edge],
      [`clk_sclk`], [187.5], [serialisers' parallel side (eclk ÷ 2)],
    )
    One EHXPLLL (750 MHz VCO), so all are phase-locked. Reset comes from PLL
    lock alone.
  ],
)

== System block diagram

#align(center, text(size: 13pt, diagram(
  spacing: (9mm, 11mm),
  node-inset: 6pt,
  planned((0, 0), [SPI host\ link], name: <spi>),
  planned((1, 0), [command\ parser], name: <cmd>),
  planned((2, 0), [vertex\ ring], name: <ring>),
  standalone((3, 0), [triangle\ setup], name: <setup>),
  planned((4, 0), [traversal +\ iterators], name: <trav>),
  planned((5, 0), [tile buffer\ colour + Z], name: <tile>),
  planned((0, 1), [test scene\ ROM], name: <rom>),
  built((2, 1), [test-pattern\ fill], name: <fill>),
  planned((4, 1), [SDRAM\ arbiter], name: <arb>),
  built((5, 1), [SDRAM\ controller], name: <ctl>),
  chip((6, 1), [SDRAM], name: <sdram>),
  built((4, 2), [scan-out\ ring], name: <scan>),
  built((3, 2), [video timing +\ TMDS encoders], name: <enc>),
  built((2, 2), [TMDS gearbox\ + ODDRX2F], name: <gear>),
  chip((1, 2), [DVI], name: <dvi>),
  edge(<spi>, <cmd>, "--|>"),
  edge(<rom>, <cmd>, "--|>"),
  edge(<cmd>, <ring>, "--|>"),
  edge(<ring>, <setup>, "--|>"),
  edge(<setup>, <trav>, "--|>"),
  edge(<trav>, <tile>, "--|>"),
  edge(<tile>, <arb>, "<|--|>"),
  edge(<fill>, <arb>, "-|>", bend: -10deg),
  edge(<arb>, <ctl>, "-|>"),
  edge(<ctl>, <sdram>, "<|-|>"),
  edge(<arb>, <scan>, "-|>"),
  edge(<scan>, <enc>, "-|>"),
  edge(<enc>, <gear>, "-|>"),
  edge(<gear>, <dvi>, "-|>"),
)))
#align(center, key)
Everything runs on the one 75 MHz core clock. Only the serialisers and
(later) the SPI link have clocks of their own.

== One source, two artefacts

#align(center, text(size: 13pt, diagram(
  spacing: (12mm, 12mm),
  node-inset: 6pt,
  built((0, 1), [`hw/*.snt`\ Sinter source], name: <src>),
  built((1, 0), [SystemVerilog], name: <sv>),
  built((1, 2), [Rust simulator\ crates], name: <rs>),
  built((2, 0), [Yosys · nextpnr\ · ecppack\ + `board/top.sv`], name: <pnr>),
  chip((3, 0), [`top.bit`\ on the board], name: <bit>),
  built((2, 2), [cargo tests\ + chip models], name: <tests>),
  built((2, 1), [`sinter sim --cosim`\ Verilator vs simulator], name: <cosim>),
  edge(<src>, <sv>, "-|>", [`sinter build`]),
  edge(<src>, <rs>, "-|>"),
  edge(<sv>, <pnr>, "-|>"),
  edge(<pnr>, <bit>, "-|>"),
  edge(<rs>, <tests>, "-|>"),
  edge(<sv>, <cosim>, "-|>"),
  edge(<rs>, <cosim>, "-|>"),
  edge(<tests>, <cosim>, "-|>", [recorded\ stimulus], label-side: right),
)))

- The simulator *is* the model: tests drive it cycle by cycle against Rust
  models of the SDRAM chip, and the setup engine against its microcode
  interpreter.
- `just check` replays every stimulus file, including the 3.7M cycles the
  frame test records, through Verilator, and fails on any difference.
- Generated Verilog, crates and bitstreams live in `build/`; never committed.

= Built and running

== Video out: 1280×720 at 60 Hz

#align(center, text(size: 13pt, diagram(
  spacing: (26mm, 9mm),
  node-inset: 6pt,
  built((0, 0), [video timing\ (CEA-861)], name: <vt>),
  built((1, 0), [scan-out ring], name: <ring>),
  built((2, 0), [TMDS encoder ×3\ 4-stage pipeline], name: <enc>),
  built((2, 1), [gearbox, 187.5 MHz\ 2 symbols per 5 cycles], name: <gear>),
  built((1, 1), [ODDRX2F ×4, 4:1], name: <oddr>),
  chip((0, 1), [DVI], name: <dvi>),
  edge(<vt>, <ring>, "-|>", [beam]),
  edge(<ring>, <enc>, "-|>", [RGB565]),
  edge(<enc>, <gear>, "-|>", [10 bit · 75 MHz], label-side: left),
  edge(<gear>, <oddr>, "-|>", [4 bit], label-side: left),
  edge(<oddr>, <dvi>, "-|>", [750 Mb/s], label-side: left),
)))

- 640×360 RGB565 framebuffer, each pixel shown as 2×2: a 16:9 picture.
- Pixel clock 75 MHz instead of CEA's 74.25 (one PLL); monitors accept it.
- The serialiser moved from fabric logic (failed at 375 MHz) to the ECP5's
  ODDRX2F gearboxes; the hand-written gearbox crossing is tested at 24 clock
  phase and reset combinations.
- 750 Mb/s per pin is above Lattice's documented GPIO rate, but works.

== SDRAM controller

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    - W9825G6KH: 4 banks × 8192 rows × 512 columns × 16 bits.
    - A *run* is 1–32 words in one row: ACTIVE, a READ or WRITE every cycle,
      PRECHARGE. About 6 cycles of overhead per run.
    - CAS latency 2, burst length 1, the slowest speed grade's timings, so
      any part works.
    - A write mask per word (DQM): blocks can be written without reading
      colour back.
  ],
  [
    - Peak 150 MB/s; scan-out uses about 23%.
    - Tested against a pin-level chip model that checks every datasheet
      rule: timings, refresh rate, power-up.
    - *Fixed in Phase 8:* reads were captured a cycle late since Phase 4.
      Each word read was the next one; the tiled pattern made it visible.
      The testbench had modelled a board delay that doesn't exist.
  ],
)

== Tiled framebuffer

#grid(
  columns: (auto, 1fr),
  gutter: 1.5em,
  {
    // One 8x8 block: words 0-31 (top half) and 32-63 (bottom half).
    let cell(i) = {
      let top = i < 32
      box(
        width: 9mm, height: 7mm,
        fill: if top { luma(232) } else { luma(205) },
        stroke: 0.4pt + luma(120),
        align(center + horizon, text(size: 9pt)[#i]),
      )
    }
    stack(
      spacing: 4pt,
      grid(columns: 8, gutter: 0pt, ..range(64).map(cell)),
      text(size: 12pt)[one 8×8 block, 64 words in raster order:\ light = run 1 (words 0–31), dark = run 2],
    )
  },
  [
    - 80 × 45 blocks; block $(b_x, b_y)$ is the 64 words from
      $(80 b_y + b_x) dot 64$.
    - A block never crosses a 512-word SDRAM row.
    - Each half (8×4) is *one 32-word run*: rendering moves a block in 2
      runs, not 8.
    - Pixel $(x, y)$ is word `x[9:3] @ y[2:0] @ x[2:0]` of its block row, plus
      $5120 dot y\[8:3\]$.

    #set text(size: 14pt)
    #table(
      columns: (auto, 1fr, auto),
      stroke: 0.5pt + luma(180),
      [*Bank*], [*Contents*], [*Status*],
      [0], [colour buffer], done,
      [1, 2], [colour buffers (triple buffering)], later,
      [3], [Z buffer, 16-bit], later,
    )
  ],
)

== Scan-out: the half-block ring

#align(center, text(size: 13pt, diagram(
  spacing: (12mm, 10mm),
  node-inset: 6pt,
  chip((0, 0), [SDRAM\ tiled buffer], name: <sd>),
  built((1, 0), [loader\ 80 runs × 32 words\ per half-block row], name: <ld>),
  built((2, 1), [ring: 192 half-block slots\ 4096 + 2048 words, 6 DP16KD], name: <ring>),
  built((3, 0), [display\ word = `slot @ py @ px`], name: <disp>),
  edge(<sd>, <ld>, "-|>"),
  edge(<ld>, <ring>, "-|>", [fills while slots are free], label-side: right),
  edge(<ring>, <disp>, "-|>", [pixels], label-side: right),
  edge(<disp>, <ld>, "-|>", [frees 80 slots per 4 lines], label-side: left),
)))

- A half-block row is 4 framebuffer lines (8 screen lines); two fit with 32
  slots to spare, so loads run ahead of the display.
- Each pixel is read from SDRAM once: about 23% of the bus.
- The ring restarts at the end of each frame's active area, so a buffer swap
  at vertical blank never mixes frames.
- Two memories because Sinter memories must be a power of two deep (8192
  words would take 8 block RAMs).

= The rendering pipeline

== Planned: from vertices to pixels

#align(center, text(size: 13pt, diagram(
  spacing: (11mm, 14mm),
  node-inset: 6pt,
  planned((0, 0), [packets\ (ROM, then SPI)], name: <pk>),
  planned((1, 0), [vertex ring\ 4 slots], name: <vr>),
  standalone((2, 0), [setup\ 99 cycles / triangle], name: <su>),
  planned((3, 0), [traversal\ 8×8 blocks, 1 px/clk], name: <tr>),
  planned((4, 0), [tile buffer\ colour + Z], name: <tb>),
  chip((4, 1), [SDRAM], name: <sd>),
  edge(<pk>, <vr>, "--|>"),
  edge(<vr>, <su>, "--|>"),
  edge(<su>, <tr>, "--|>"),
  edge(<tr>, <tb>, "--|>"),
  edge(<tb>, <sd>, "<|--|>", [Z read; colour + Z masked write], label-side: right),
)))

- *Traversal:* skip blocks wholly outside an edge; step three edge functions
  and R, G, B, Z with adders; top-left fill rule; Z less-than test.
- *Per block:* read Z (2 runs), write colour and Z with masks (2 + 2 runs):
  about 230 SDRAM cycles, 3.6 per pixel (est.).
- *Buffers:* triple-buffered colour, swapped at vertical blank.
- *Scene:* a ROM of host-style packets (a spinning cube) until the SPI link.

== Triangle setup: what it computes

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    *Edges* (exact integers, so shared edges never crack):
    $ E_(i j)(p) = A_(i j)(p_x - x_i) + B_(i j)(p_y - y_i) $
    $ A_(i j) = y_i - y_j, quad B_(i j) = x_j - x_i $

    *Area:* $2 A = d x_1 d y_2 - d x_2 d y_1$: zero → discard; wrong winding →
    cull; negative → swap vertices 1 and 2.

    *Reciprocal:* $2A = m dot 2^(E-8)$, a 1024-entry seed, one Newton–Raphson
    step: 22 correct bits.
  ],
  [
    *Gradients*, per attribute ($d_1, d_2$ = deltas from vertex 0):
    $ k = r dot d quad "(block floating point)" $
    $ (partial a) / (partial x) = (d_1 k_1 - d_2 k_2) dot 2^(8-E) $

    *Start value* at the bounding box's snapped corner:
    $ a_s = a_0 + (partial a) / (partial x) c_x + (partial a) / (partial y) c_y quad (mod 2^36) $
    The corner can be far outside a thin triangle; traversal's adders wrap
    the same way, so the values inside come out right.
  ],
)

== Setup engine: a microcoded datapath

#align(center, text(size: 12pt, diagram(
  spacing: (9mm, 8mm),
  node-inset: 5pt,
  standalone((0, 0), [micro-ROMs\ MUL 38 b · ALU 33 b], name: <rom>),
  standalone((0, 1), [sequencer\ NEXT · CHECK · END], name: <seq>),
  standalone((1, 0), [ALU slot\ ADD SUB MIN MAX SNAP\ LZC RCPLD TST ADDV], name: <alu>),
  standalone((1, 1), [MUL slot\ 36 × 18 → acc 56 →\ shift (≪8, ≫ c + E)\ round · saturate], name: <mul>),
  standalone((2, 0), [ALU bank\ 32 × 36], name: <ab>),
  standalone((2, 1), [MUL bank\ 32 × 36], name: <mb>),
  standalone((3, 0.5), [output buffer\ 37 slots], name: <out>),
  standalone((1, -1), [vertex ring\ (by role) · consts\ · seed ROM], name: <vtx>),
  edge(<rom>, <alu>, "-|>"),
  edge(<rom>, <mul>, "-|>"),
  edge(<seq>, <rom>, "-|>"),
  edge(<vtx>, <alu>, "-|>"),
  edge(<alu>, <ab>, "-|>"),
  edge(<mul>, <mb>, "-|>"),
  edge(<ab>, <mul>, "-|>", bend: 25deg),
  edge(<mb>, <alu>, "-|>", bend: 25deg),
  edge(<ab>, <out>, "-|>"),
  edge(<mb>, <out>, "-|>"),
)))

- One MUL and one ALU issue per cycle, statically scheduled by a Rust
  generator. Each bank has one writer. Latencies: ALU 3, seed and LZC 4,
  MUL 6, all set by timing closure at 75 MHz.
- `hw/setup.snt`; its ROMs come from the generator at build time. Not
  Sinter's `multiplex fn`: that can't pipeline the multiplier or exit early
  for culled triangles.

== Setup: results

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto, auto),
      stroke: 0.5pt + luma(180),
      [*Schedule*], [*Now*], [*Brief*],
      [full program], [99 cycles], [85],
      [untextured], [80 cycles], [—],
      [rejected triangle], [16 cycles], [10],
      [multiplies / triangle], [62], [66],
      [engine Fmax, alone], [82.5 MHz], [75],
      [engine logic cells], [3,841], [1,500],
    )
    Timing closure cost 5 cycles. A 5-cycle multiplier saves 4 more but
    reaches only 53 MHz.
  ],
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto),
      stroke: 0.5pt + luma(180),
      [*Accuracy* (inside the triangle)], [*Worst error*],
      [ordinary triangles, colour], [0.011 LSB],
      [ordinary, 16-bit attributes], [0.018 LSB],
      [slivers, 16-bit attributes], [0.5 LSB],
      [reciprocal], [22 bits],
    )
    The interpreter matches the bit-exact model on 18,000 random triangles,
    and the Sinter engine matches the interpreter, cycle for cycle, on
    1,500; random meshes are covered watertight.
  ],
)

= Verification and status

== Verification layers

#align(center, text(size: 13pt, diagram(
  spacing: (32mm, 10mm),
  node-inset: 6pt,
  modelled((0, 0), [f64 reference], name: <f64>),
  modelled((1, 0), [bit-exact model\ (qfixed types)], name: <q>),
  modelled((2, 0), [microcode\ interpreter], name: <i>),
  standalone((2, 1), [Sinter simulator\ of the engine], name: <s>),
  built((1, 1), [Verilator on\ the Verilog], name: <v>),
  built((0, 1), [the board], name: <b>),
  edge(<f64>, <q>, "<|-|>", [error bounds]),
  edge(<q>, <i>, "<|-|>", [bit for bit]),
  edge(<i>, <s>, "<|-|>", [cycle for cycle], label-side: left),
  edge(<s>, <v>, "<|-|>", [`--cosim`]),
  edge(<v>, <b>, "<|-|>", [bitstream]),
)))

- *Video and SDRAM today:* stimulus CSVs; a frame test decoding two 720p
  frames pixel-exactly; the SDRAM chip model's datasheet checks; the gearbox
  testbench; all through `--cosim`.
- *Setup today:* reciprocal over every mantissa, accuracy by triangle shape,
  edge cases, watertightness, interpreter against the bit-exact model, and
  the Sinter engine against the interpreter, its Verilog through `--cosim`.
  The setup engine isn't on the board yet.
- Tests are checked by mutation: break the design on purpose, see them fail.

== Resources and timing today

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto, auto, auto),
      stroke: 0.5pt + luma(180),
      [*Resource*], [*GPU*], [*Setup*], [*Of*],
      [logic cells], [1288], [3841], [24288],
      [flip-flops], [609], [974], [24288],
      [block RAM (DP16KD)], [6], [5], [56],
      [multipliers], [0], [2], [28],
      [PLL], [1], [—], [2],
      [I/O], [49], [—], [197],
    )
    _GPU_: today's bitstream. _Setup_: the engine placed and routed alone; it
    isn't in the GPU yet.
  ],
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto, auto),
      stroke: 0.5pt + luma(180),
      [*Clock*], [*Reaches*], [*Needs*],
      [`clk_core`], [103 MHz], [75],
      [`clk_sclk`], [315 MHz], [187.5],
      [setup engine], [82.5 MHz], [75],
    )
    The brief's whole-design budget (estimate): about 11,000 LUTs (45%), 19
    multipliers, 27 block RAMs. Datapath registers are `#[no_reset]`: only
    191 flip-flops are on the core reset.
  ],
)

== Status by phase

#text(size: 14pt, table(
  columns: (auto, 1fr, auto),
  stroke: 0.5pt + luma(180),
  [*Phase*], [*Work*], [*Status*],
  [2–3], [toolchain, CI; 640×480 colour bars], done,
  [4–5], [SDRAM controller and framebuffer; 720p], done,
  [6], [rendering plan, ADRs], done,
  [7], [Sinter rc.2, co-simulation in CI, `#[no_reset]`], done,
  [8], [tiled framebuffer, half-block ring, write masks; read-capture fix], done,
  [9], [setup engine: Rust models, microcode, scheduler, interpreter], done,
  [10], [setup engine in Sinter, cycle-exact against the interpreter], done,
  [11], [command parser, traversal, tile buffer: first triangles on screen], next,
  [12], [Z buffer, triple buffering, spinning cube, frame rate], later,
  [—], [SPI host link and RP2350 firmware], later,
  [—], [textures and perspective correction], later,
))

== Open issues

- *Setup speed:* 99 cycles against the brief's 85. A 5-cycle multiplier
  misses 75 MHz; next options are overlapping triangles or a second
  multiplier, if traversal shows setup is the bottleneck.
- *Setup size:* 3,841 logic cells against the brief's 1,500, mostly the
  four-read-port register banks (four copies each in LUT RAM).
- *Accuracy on slivers:* 16-bit attributes reach 0.5 LSB; the reciprocal's 22
  bits limit it.
- *No formal overflow proof* for the setup program yet; ports and the
  accumulator are checked at run time over the test corpus.
- *Sinter gaps:* const generics; a slow `async_fifo` (needed for SPI);
  `#[no_reset]` accepted on non-registers; power-of-two memory depths.
- *Off spec but working:* 750 Mb/s DVI pins; a 75 MHz (not 74.25 MHz) pixel
  clock.
