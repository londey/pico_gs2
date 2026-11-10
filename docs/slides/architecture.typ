// pico-gs2 architecture slides. Build with `just slides`, which writes
// build/slides/architecture.pdf. Package versions are pinned here and in
// .devcontainer/Dockerfile (which prefetches them); keep the two in step.

#import "@preview/touying:0.8.0": *
#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge
#import "theme/pico.typ": *

#show: pico-theme.with(
  config-info(
    title: [pico-gs2: architecture and status],
    short-title: [pico-gs2],
    subtitle: [A PS1/N64-era GPU for the ICEpi Zero, written in Sinter],
    date: [November 2026],
  ),
)

// Status colours: built and running on the board; a Rust model; planned.
#let built-colour = rgb("#059669")
#let model-colour = rgb("#2563eb")
#let planned-colour = rgb("#94a3b8")

// A block that is built in the design.
#let built(pos, body, ..args) = node(
  pos, body,
  stroke: 1pt + built-colour,
  fill: rgb("#ecfdf5"),
  corner-radius: 6pt,
  shape: rect,
  ..args,
)

// A block that exists as a Rust model, not yet in the design.
#let modelled(pos, body, ..args) = node(
  pos, body,
  stroke: 1pt + model-colour,
  fill: rgb("#eff6ff"),
  corner-radius: 6pt,
  shape: rect,
  ..args,
)

// A block that is planned.
#let planned(pos, body, ..args) = node(
  pos, body,
  stroke: (paint: planned-colour, thickness: 1pt, dash: "dashed"),
  fill: white,
  corner-radius: 6pt,
  shape: rect,
  ..args,
)

// An external chip or connector.
#let chip(pos, body, ..args) = node(
  pos, body,
  stroke: 1pt + rgb("#475569"),
  fill: rgb("#f1f5f9"),
  corner-radius: 2pt,
  shape: rect,
  ..args,
)

// The colour key, for diagrams.
#let key = text(size: 12pt)[
  #box(fill: rgb("#ecfdf5"), stroke: 1pt + built-colour, radius: 4pt, inset: 4pt)[built]
  #h(0.5em)
  #box(fill: rgb("#eff6ff"), stroke: 1pt + model-colour, radius: 4pt, inset: 4pt)[Rust model]
  #h(0.5em)
  #box(stroke: (paint: planned-colour, dash: "dashed"), radius: 4pt, inset: 4pt)[planned]
  #h(0.5em)
  #box(fill: rgb("#f1f5f9"), stroke: 1pt + rgb("#475569"), radius: 2pt, inset: 4pt)[off-chip]
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
  (Winbond W9825G6KH), DVI out on mini-HDMI, an FT231X USB UART to a PC.
- *Language:* Sinter. One source compiles to SystemVerilog for the FPGA *and*
  a cycle-accurate Rust simulator, the design's digital twin. There is no
  separate hand-written model.
- *v1.0:* 1280×720 DVI from a tiled, triple-buffered 640×360 framebuffer
  in SDRAM. Gouraud-shaded triangles, depth-tested against a 16-bit Z
  buffer; perspective-correct textures (RGB565, ARGB4444), nearest or
  bilinear, through an on-chip cache; an N64-style colour combiner and an
  alpha test. A PC drives it with register writes over USB serial, uploads
  textures and reads frames back; with no PC it plays a built-in scene.
  Pixel-exact against a reference renderer in simulation, and on the board.

#v(0.5em)
#key

== Board and clocks

#grid(
  columns: (1.15fr, 1fr),
  gutter: 1em,
  text(size: 14pt, diagram(
    spacing: (22mm, 12mm),
    node-inset: 7pt,
    chip((1, 0), [50 MHz oscillator], name: <osc>),
    chip((0, 1), [PC\ FT231X], name: <host>),
    planned((0, 2), [RP2350\ (later)], name: <rp>),
    built((1, 1), [ECP5-25K\ PLL · Sinter core\ `board/top.sv`], name: <fpga>),
    chip((2, 1), [SDRAM\ 32 MB], name: <sdram>),
    chip((1, 2), [mini-HDMI (DVI)], name: <hdmi>),
    edge(<osc>, <fpga>, "-|>"),
    edge(<host>, <fpga>, "<|-|>", [USB\ serial]),
    edge(<rp>, <fpga>, "--|>", [SPI], label-side: right),
    edge(<fpga>, <sdram>, "<|-|>", [16 bit]),
    edge(<fpga>, <hdmi>, "-|>", [4 TMDS pairs, 750 Mb/s], label-side: left),
  )),
  [
    #set text(size: 15pt)
    #table(
      columns: (auto, auto, 1fr),
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
  built((0, 0), [USB serial\ link + FIFO], name: <spi>),
  planned((0, -1), [SPI link\ (later)], name: <spi2>),
  built((1, 0), [command\ parser], name: <cmd>),
  built((2, 0), [vertex\ ring], name: <ring>),
  built((3, 0), [triangle\ setup], name: <setup>),
  built((4, 0), [traversal +\ iterators], name: <trav>),
  built((5, 0), [tile buffers\ colour + Z], name: <tile>),
  built((4, -1), [texture pipeline\ + cache], name: <tex>),
  built((5, -1), [combiner +\ alpha test], name: <comb>),
  built((0, 1), [scene ROM\ (transactions)], name: <rom>),
  built((2, 1), [test-pattern\ fill], name: <fill>),
  built((4, 1), [SDRAM\ arbiter], name: <arb>),
  built((5, 1), [SDRAM\ controller], name: <ctl>),
  chip((6, 1), [SDRAM], name: <sdram>),
  built((4, 2), [scan-out\ ring], name: <scan>),
  built((3, 2), [video timing +\ TMDS encoders], name: <enc>),
  built((2, 2), [TMDS gearbox\ + ODDRX2F], name: <gear>),
  chip((1, 2), [DVI], name: <dvi>),
  edge(<spi>, <cmd>, "-|>"),
  edge(<spi2>, <cmd>, "--|>"),
  edge(<rom>, <cmd>, "-|>"),
  edge(<cmd>, <ring>, "-|>"),
  edge(<ring>, <setup>, "-|>"),
  edge(<setup>, <trav>, "-|>"),
  edge(<trav>, <tex>, "-|>"),
  edge(<tex>, <comb>, "-|>"),
  edge(<comb>, <tile>, "-|>"),
  edge(<arb>, <tex>, "-|>", bend: 40deg),
  edge(<tile>, <arb>, "<|-|>"),
  edge(<fill>, <arb>, "-|>", bend: -10deg),
  edge(<arb>, <ctl>, "-|>"),
  edge(<ctl>, <sdram>, "<|-|>"),
  edge(<arb>, <scan>, "-|>"),
  edge(<scan>, <enc>, "-|>"),
  edge(<enc>, <gear>, "-|>"),
  edge(<gear>, <dvi>, "-|>"),
)))
#align(center, key)
Everything runs on the one 75 MHz core clock, the UART included. Only the
serialisers and (later) the SPI link have clocks of their own.

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
- `just check` replays every stimulus file, including the 55M cycles the
  twin tests record, through Verilator, and fails on any difference: about
  2 minutes, since Sinter rc.3 compiles its simulator.
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
        fill: if top { rgb("#eef2ff") } else { rgb("#c7d2fe") },
        stroke: 0.5pt + white,
        align(center + horizon, text(size: 9pt, fill: rgb("#3730a3"))[#i]),
      )
    }
    stack(
      spacing: 4pt,
      grid(columns: 8, gutter: 0pt, ..range(64).map(cell)),
      text(size: 12pt, fill: rgb("#64748b"))[one 8×8 block, 64 words in raster order:\ light = run 1 (words 0–31), dark = run 2],
    )
  },
  [
    - 80 × 45 blocks; block (b#sub[x], b#sub[y]) is the 64 words from
      (80 b#sub[y] + b#sub[x]) × 64.
    - A block never crosses a 512-word SDRAM row.
    - Each half (8×4) is *one 32-word run*: rendering moves a block in 2
      runs, not 8.
    - Pixel (x, y) is word `x[9:3] @ y[2:0] @ x[2:0]` of its block row, plus
      5120 × `y[8:3]`.

    #set text(size: 14pt)
    #table(
      columns: (auto, 1fr, auto),
      [*Bank*], [*Contents*], [*Status*],
      [0, 1, 2], [colour buffers (triple buffering)], done,
      [3], [Z buffer, 16-bit], done,
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

== From vertices to pixels

#align(center, text(size: 13pt, diagram(
  spacing: (11mm, 18mm),
  node-inset: 6pt,
  built((0, 0), [register writes\ (ROM or host)], name: <pk>),
  built((1, 0), [vertex ring\ 4 slots], name: <vr>),
  built((2, 0), [setup\ 99 cycles / triangle], name: <su>),
  built((3, 0), [traversal\ 8×8 blocks, 1 px/clk], name: <tr>),
  built((4, 0), [texels, combiner\ alpha test], name: <tx>),
  built((5, 0), [tile buffers\ colour + Z], name: <tb>),
  chip((4.5, 1), [SDRAM], name: <sd>),
  edge(<pk>, <vr>, "-|>"),
  edge(<vr>, <su>, "-|>"),
  edge(<su>, <tr>, "-|>"),
  edge(<tr>, <tx>, "-|>"),
  edge(<tx>, <tb>, "-|>"),
  edge(<sd>, <tx>, "-|>", [texels], label-side: left),
  edge(<tb>, <sd>, "<|-|>", [Z read;\ colour, Z written], label-side: left),
)))

- *Traversal:* skip blocks wholly outside an edge; step three edge functions
  and colour, depth, S′, T′, Q with adders; top-left fill rule.
- *Per block:* read its 64 depths (two runs, none if cleared); the front
  half (64 cycles); the back half (64, plus misses); a pixel is drawn if
  covered, nearer and passing the alpha test; then one masked 32-word run
  per half block with a drawn pixel, for colour and for Z. Front half,
  back half and writer each work on a different block (next slides).
- *Per frame:* a clear (flags only: 3,603 cycles), the triangles, a
  present. The built-in cube frame takes about 0.2 M cycles (2.8 ms; 1.25
  M at v1.0).

== Depth and triple buffering

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  text(size: 13pt, diagram(
    spacing: (14mm, 14mm),
    node-inset: 7pt,
    built((0, 0), [drawn], name: <dr>),
    built((1, 1), [waiting], name: <wt>),
    built((2, 0), [shown], name: <sh>),
    built((1, -1), [spare], name: <sp>),
    edge(<dr>, <wt>, "-|>", [present], label-side: right),
    edge(<wt>, <sh>, "-|>", [frame start], label-side: right),
    edge(<sh>, <sp>, "-|>", [frame start], label-side: right),
    edge(<sp>, <dr>, "-|>", [present], label-side: right),
  )),
  [
    #set text(size: 15pt)
    - Three colour buffers (banks 0–2) take turns: one shown, one drawn, one
      finished and waiting or spare.
    - A finished frame is shown from the next frame start, so a frame is
      never torn. If one is already waiting, the parser holds at the
      present.
    - *Z:* 16 bits in bank 3, the iterator rounded and clamped; less-than
      test, cleared to 0xFFFF.
    - *Cube:* 32 frames a quarter turn, back faces culled, looping
      seamlessly; two triangles crossing in depth test the Z buffer.
  ],
)

== The fragment pipeline (v1.1)

// A stall point: where a stage can wait, keyed in the list below.
#let stall-colour = rgb("#dc2626")
#let stall(n) = box(
  fill: stall-colour,
  radius: 50%,
  inset: (x: 3.5pt, y: 2pt),
  text(fill: white, weight: "bold", size: 9pt, n),
)

#align(center, text(size: 11pt, diagram(
  spacing: (7mm, 12mm),
  node-inset: 5pt,
  built((0, 0), [setup\ 99 cycles\ a triangle #stall("1")], name: <su>),
  built((1, 0), [*front half* #stall("2") #stall("3")\
    TEST, Z read, then\ 1 fragment / clock:\ edges, planes,\ depth test;\ texel addresses\ (13 stages)], name: <fh>),
  built((2, 0), [shades, texel\ addresses: 2 banks\ depths: 3 banks\ drawn mask], name: <bf>),
  built((3, 0), [*back half* #stall("4") #stall("5")\
    1 fragment / clock:\ cache lookup (3),\ filter (4),\ combiner (4),\ alpha test], name: <bh>),
  built((4, 0), [colour tile\ 2 banks], name: <ct>),
  built((5, 0), [*writer*\ ≤ 4 masked\ 32-word runs], name: <wr>),
  built((3, 1), [SDRAM arbiter #stall("6")\ scan-out first], name: <arb>),
  chip((5, 1), [SDRAM], name: <sd>),
  edge(<su>, <fh>, "-|>"),
  edge(<fh>, <bf>, "-|>"),
  edge(<bf>, <bh>, "-|>"),
  edge(<bh>, <ct>, "-|>"),
  edge(<ct>, <wr>, "-|>"),
  edge(<fh>, <arb>, "<|-", [Z read], label-side: right),
  edge(<bh>, <arb>, "<|-", [texels], label-side: left),
  edge(<wr>, <arb>, "-|>", [writes], label-side: right),
  edge(<arb>, <sd>, "<|-|>"),
)))

#text(size: 11.5pt, grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    - #stall("1") The front half waits for the next triangle's setup.
    - #stall("2") TEST waits while the back half or the writer still holds
      the same block (a later triangle over it).
    - #stall("3") HANDOFF waits for the back half to be free; a block with
      no pixel drawn goes nowhere.
  ],
  [
    - #stall("4") A cache miss: a 16-word fetch, then that pixel again.
      Only the back half stops.
    - #stall("5") HANDOFF waits for the writer to be free.
    - #stall("6") Every run waits for scan-out and its bank. A Z read and a
      texel fetch are never in flight at once.
  ],
))

== What overlaps: three blocks in flight

// A block timeline: lanes of stages, a box per block a stage works on,
// widths in cycles (the goal scene's figures, depth cleared, so no Z read).
#let tl-scale = 0.0235cm
#let tl-height = 0.62cm
#let block-fills = (rgb("#c7d2fe"), rgb("#bbf7d0"), rgb("#fde68a"), rgb("#fbcfe8"), rgb("#bae6fd"))
#let tl-box(start, len, label, fill) = place(dx: start * tl-scale, box(
  width: len * tl-scale,
  height: tl-height,
  fill: fill,
  stroke: 0.6pt + white,
  radius: 2pt,
  align(center + horizon, text(size: 8.5pt, label)),
))
#let tl-wait(start, len) = place(dx: start * tl-scale, box(
  width: len * tl-scale,
  height: tl-height,
  stroke: (paint: stall-colour, thickness: 0.6pt, dash: "dotted"),
  radius: 2pt,
  align(center + horizon, text(size: 8pt, fill: stall-colour, [wait])),
))
#let lane(name, items) = (
  align(right + horizon, text(size: 10pt, name)),
  box(width: 900 * tl-scale, height: tl-height, items.join()),
)
#let b(k, start, len, label: none) = tl-box(start, len, if label == none [#k] else [#k #label], block-fills.at(calc.rem(k, 5)))

#text(size: 10pt, grid(
  columns: (3.4cm, 1fr),
  row-gutter: 5pt,
  column-gutter: 6pt,
  align(right, text(fill: muted)[cycles]),
  box(width: 900 * tl-scale, height: 1em, for c in (0, 150, 300, 450, 600, 750, 900) {
    place(dx: c * tl-scale - 1em, box(width: 2em, align(center, text(fill: muted, [#c]))))
  }),
  [*v1.0*], [],
  ..lane([one after another], (
    b(0, 0, 90, label: [Z]), b(0, 90, 64, label: [pixels]), b(0, 154, 75, label: [texels]), b(0, 229, 225, label: [writes]),
    b(1, 454, 90, label: [Z]), b(1, 544, 64, label: [pixels]), b(1, 608, 75, label: [texels]), b(1, 683, 217, label: [writes]),
  )),
  [*Phase 23*], [],
  ..lane([both halves], (
    b(0, 0, 152), b(1, 152, 152), tl-wait(304, 33), b(2, 337, 152), tl-wait(489, 33), b(3, 522, 152), tl-wait(674, 33), b(4, 707, 152),
  )),
  ..lane([writer], (
    b(0, 152, 185), b(1, 337, 185), b(2, 522, 185), b(3, 707, 185),
  )),
  [*Phase 25*], [],
  ..lane([front half], (
    b(0, 0, 80), b(1, 80, 80), b(2, 160, 80), tl-wait(240, 105), b(3, 345, 80), tl-wait(425, 110), b(4, 535, 80), tl-wait(615, 110), b(5, 725, 80),
  )),
  ..lane([back half], (
    b(1, 160, 75), tl-wait(235, 110), b(2, 345, 75), tl-wait(420, 115), b(3, 535, 75), tl-wait(610, 115), b(4, 725, 75),
  ) + (b(0, 80, 75),)),
  ..lane([writer], (
    b(0, 155, 190), b(1, 345, 190), b(2, 535, 190), b(3, 725, 175),
  )),
))

#text(size: 12.5pt)[
  - Within each half, one fragment a clock; between the stages, whole
    blocks, through the banked buffers: a stage hands a block on as soon as
    the next stage is free, and doesn't wait for anything to drain.
  - The writer now sets the pace: 64 colours and 64 depths a block, 128
    words, while scan-out takes a fifth of the SDRAM. The goal scene keeps
    it busy 98% of the time; Phase 23 had nearly reached that limit, so
    overlapping the halves gained 6% there, 12% on the cube (depth reads).
]

= The host interface

== Registers, not packets

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 13pt)
    A *transaction* is 72 bits: read/write, a 7-bit address, 64 bits of data.
    #table(
      columns: (auto, auto, 1fr),
      [*Addr*], [*Register*], [*Holds*],
      [0x00], [`ATTR0`], [RGBA, S′T′ (latched)],
      [0x06–09], [`VERTEX_*`], [X, Y, Z, Q; kicks draw],
      [0x10], [`TEX0_CFG`], [the texture],
      [0x18–1B], [`CC_MODE` …], [combiner, 2 colours],
      [0x30], [`RENDER_MODE`], [cull, Z, alpha test],
      [0x40], [`FRAME`], [present, clears],
      [0x70–71], [`MEM_*`], [GPU memory access],
      [0x7D], [`SCRATCH`], [reads back as written],
      [0x7E], [`STATUS`], [frames, cycles, FIFO],
      [0x7F], [`ID`], [`0x5047533200020000`],
    )
  ],
  [
    #set text(size: 15pt)
    - A vertex is one write; a triangle strip costs about one vertex and one
      colour per triangle.
    - One kick register per winding, so strips and fans need no flags.
    - `FRAME` presents, then clears: one write ends a frame and starts the
      next.
    - Inspired by pico-gs's registers, not copied: only what this pipeline
      can honour (ADR 0008, `docs/design/host-interface.md`).
    - The built-in cube is the same transactions, in a 2,048-entry ROM.
  ],
)

== The serial link

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  text(size: 13pt, diagram(
    spacing: (16mm, 12mm),
    node-inset: 6pt,
    chip((0, 0), [`gs-host` (Rust, PC)], name: <pc>),
    chip((0, 1), [FT231X USB UART], name: <ft>),
    built((0, 2), [UART + assembler\ 25 clocks a bit], name: <rx>),
    built((0, 3), [FIFO\ 512 transactions], name: <fifo>),
    built((1, 3), [command\ parser], name: <cmd>),
    built((1, 2), [scene ROM], name: <rom>),
    edge(<pc>, <ft>, "<|-|>", [USB], label-side: left),
    edge(<ft>, <rx>, "<|-|>", [3 Mbaud each way], label-side: left),
    edge(<rx>, <fifo>, "-|>"),
    edge(<fifo>, <cmd>, "-|>"),
    edge(<rom>, <cmd>, "-|>"),
  )),
  [
    #set text(size: 15pt)
    - A write is 9 bytes, a read 1; a read's reply is a tag and 8 bytes. Reads
      go through the FIFO, so they fence the writes before them.
    - *Credits:* the host starts with 512 and gets one byte back per 64
      taken; there is no flow-control wire.
    - *DTR\#* connects the host: the GPU leaves its ROM scene at the next
      present, and goes back to it when the port closes.
    - *On the board:* `SCRATCH` round-trips six patterns; a live textured
      cube at 60.2 fps, 3,976 transactions/s, 12% of the line; a frame read
      back in 1.75 s. Credits, replies and the hand-over are twin-tested.
  ],
)

== Texturing

#grid(
  columns: (1fr, 1fr),
  gutter: 1em,
  stack(
    spacing: 6pt,
    image("images/texture-floor.png", height: 37%),
    image("images/texture-cube.png", height: 37%),
  ),
  [
    #set text(size: 14pt)
    - *Per pixel:* S′, T′ and Q stepped with colour and depth; a 13-stage
      pipeline: 1/Q (seed + Newton–Raphson), S′ and T′ × it, shift to
      texels, two columns and rows, wrap (repeat, clamp, mirror), tags.
    - *Cache:* 256 lines of one 4×4 block, direct-mapped by block row and
      column (any 64×64 window fits), tagged by address; a miss is one
      16-word run. Emptied only after `MEM_DATA` writes.
    - *Bilinear:* the cache's texels in four banks by column and row
      parity, its tags in four by block parity, so a pixel's four texels
      and up to four blocks are read and tested at once; blended by shifts
      and adds. Free in time; nearest misses only on its one block.
    - *Divide error:* within 1/8 texel down to Q = 1/256.
    - *Cube:* every block fetched once, nearest or bilinear, in the same
      cycles. On the board, bilinear, read back pixel-exact.
    - Left: the twin's frames, each pixel-exact against the reference.
  ],
)

== Colour combiner and alpha test

#grid(
  columns: (1fr, 1fr),
  gutter: 1em,
  stack(
    spacing: 6pt,
    image("images/combiner-modes.png", height: 37%),
    image("images/combiner-alpha.png", height: 37%),
  ),
  [
    #set text(size: 14pt)
    - *One N64-style cycle:* $(A - B) times C + D$, one for colour and one
      for alpha, from texel, shade, `CONST0`, `CONST1`, one and zero;
      colour's C can take any source's alpha. Presets: modulate, decal,
      shade, tint, texture + shade, fade.
    - *Every triangle* goes through it; untextured, the texel reads white.
    - *Alpha test:* always, <, ≥ or ≠ `ALPHA_REF`; a failing pixel writes
      no colour and no depth, so cut-outs show what's behind.
    - *Cost:* 4 more pipeline stages, 1,349 logic cells, one multiplier.
    - *Checked* by nine mutations; on the board the demo's last frame read
      back pixel-exact.
    - Left: the nine combiner settings; the four alpha tests and a cut-out.
  ],
)

== In simulation

#align(center, image("images/cube-sim.png", height: 80%))
#align(center, text(size: 14pt)[The built-in scene's cube frame 0 as the frame test decodes it
  from the simulated DVI output: pixel-exact against the reference renderer, as are the three
  frames after it.])

== On the board

#grid(
  columns: (1fr, 1fr),
  gutter: 1em,
  [
    #align(center, image("images/board-phase11.jpg", height: 62%))
    #align(center, text(size: 13pt)[Phase 11: the test card over the test pattern, on
      a monitor.])
  ],
  [
    #align(center, image("images/board-cube-bilinear.png", height: 62%))
    #align(center, text(size: 13pt)[Phase 18: the bilinear cube `gs-host` streams, read back
      from the board's SDRAM over the link: pixel for pixel the reference renderer's.])
  ],
)

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
  built((0, 0), [micro-ROMs\ MUL 38 b · ALU 33 b], name: <rom>),
  built((0, 1), [sequencer\ NEXT · CHECK · END], name: <seq>),
  built((1, 0), [ALU slot\ ADD SUB MIN MAX SNAP\ LZC RCPLD TST ADDV], name: <alu>),
  built((1, 1), [MUL slot\ 36 × 18 → acc 56 →\ shift (≪8, ≫ c + E)\ round · saturate], name: <mul>),
  built((2, 0), [ALU bank\ 32 × 36], name: <ab>),
  built((2, 1), [MUL bank\ 32 × 36], name: <mb>),
  built((3, 0.5), [output buffer\ 37 slots], name: <out>),
  built((1, -1), [vertex ring\ (by role) · consts\ · seed ROM], name: <vtx>),
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
  built((2, 1), [Sinter simulator\ of the engine], name: <s>),
  built((1, 1), [Verilator on\ the Verilog], name: <v>),
  built((0, 1), [the board], name: <b>),
  edge(<f64>, <q>, "<|-|>", [error bounds]),
  edge(<q>, <i>, "<|-|>", [bit for bit]),
  edge(<i>, <s>, "<|-|>", [cycle for cycle], label-side: left),
  edge(<s>, <v>, "<|-|>", [`--cosim`]),
  edge(<v>, <b>, "<|-|>", [bitstream]),
)))

- *Video and SDRAM:* directed stimulus CSVs; the SDRAM chip model's
  datasheet checks; the gearbox testbench.
- *Setup:* reciprocal over every mantissa, accuracy by triangle shape,
  edge cases, watertightness, interpreter against the bit-exact model, and
  the Sinter engine against the interpreter, its Verilog through `--cosim`.
- *The whole core:* twin tests decode the DVI output with a host on the
  simulated UART: the built-in scene, the host taking over, memory access,
  texturing, the combiner. Every frame pixel-exact against a reference
  renderer built from the same models; every run replayed through
  Verilator; `just perf` times a set of scenes.
- Tests are checked by mutation: break the design on purpose, see them fail.

== Resources and timing at v1.0

#grid(
  columns: (1fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto, auto, auto),
      [*Resource*], [*GPU*], [*Setup*], [*Of*],
      [logic cells], [17944], [3841], [24288],
      [flip-flops], [8200], [974], [24288],
      [block RAM (DP16KD)], [34], [5], [56],
      [multipliers], [12], [2], [28],
      [PLL], [1], [—], [2],
      [I/O], [52], [—], [197],
    )
    _GPU_: today's bitstream, everything included; the scene ROM takes 11
    block RAMs, the texture cache 8 and its pixels' addresses 4, the
    link's FIFO 2.
    _Setup_: the engine placed and routed alone.
  ],
  [
    #set text(size: 15pt)
    #table(
      columns: (1fr, auto, auto),
      [*Clock*], [*Reaches*], [*Needs*],
      [`clk_core`], [76.2 MHz], [75],
      [`clk_sclk`], [389 MHz], [187.5],
      [setup engine], [82.5 MHz], [75],
    )
    The brief's v1.0 budget (estimate, ±50%): about 15,000 cells, 17
    multipliers, 34 block RAMs. Datapath registers are `#[no_reset]`: only
    control state is on the core reset (191 flip-flops at Phase 11).
  ],
)

== Performance at v1.0

#grid(
  columns: (1.15fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 13pt)
    #table(
      columns: (1fr, auto, auto),
      [*Scene (twin, `just perf`)*], [*ms*], [*Beyond the clear*],
      [clear colour and depth], [9.1], [],
      [clear depth only], [4.5], [],
      [the screen, untextured], [32.1], [7.5 cycles/pixel],
      [the screen, 64×64 texture], [32.8], [7.7 cycles/pixel],
      [the screen, 256×256 minified], [64.5], [18 cycles/pixel],
      [320 triangles of 8×8 px], [11.3], [522 cycles each],
      [120 triangles of 32×32 px], [16.5], [4,661 cycles each],
      [the cube, untextured], [16.7], [],
      [the cube, nearest or bilinear], [16.8], [],
    )
  ],
  [
    #set text(size: 14pt)
    - A 60 fps frame is 16.5 ms. Drawing costs about *460 cycles per 8×8
      block* a triangle touches: depth read 90, pixels 64, texels and
      combiner 75, colour and depth written 225, one after another.
    - So after a full clear, about 1,200 blocks a frame at 60 fps: a third
      of the screen. Small triangles: about 1,000, more than the link
      carries.
    - Bilinear is free; minifying a large texture more than doubles the
      cost (no mipmaps).
    - After v1.0: overlap each block's writes with the next block's reads,
      and a fast clear.
  ],
)

== Performance at v1.1 (Phase 25)

#grid(
  columns: (1.15fr, 1fr),
  gutter: 1.2em,
  [
    #set text(size: 13pt)
    #table(
      columns: (1fr, auto, auto),
      [*Scene (twin, `just perf`)*], [*v1.0 ms*], [*now ms*],
      [clear colour and depth], [9.1], [0.05],
      [the screen, untextured], [32.1], [9.1],
      [the screen, 64×64 bilinear (goal)], [32.8], [9.1],
      [the screen, 256×256 minified], [64.5], [42.5],
      [320 triangles of 8×8 px], [11.3], [1.2],
      [120 triangles of 32×32 px], [16.5], [2.8],
      [the cube, bilinear], [16.8], [2.7],
    )
  ],
  [
    #set text(size: 14pt)
    - *Goal met:* the clear and a full-screen bilinear quad in 9.1 ms, a
      60 fps frame being 16.5 ms.
    - Fast clear (flags per block), overlapped writes, auto-precharge, then
      the two halves of the fragment pipeline at once.
    - *Now SDRAM-bound:* the writer's 128 words a block, with scan-out
      taking a fifth of the bus; the writer is busy 98% of the goal scene.
    - Minifying a large texture is still slow: texel fetches (no mipmaps).
  ],
)

== Status by phase

#text(size: 11.5pt, table(
  columns: (auto, 1fr, auto),
  [*Phase*], [*Work*], [*Status*],
  [2–3], [toolchain, CI; 640×480 colour bars], done,
  [4–5], [SDRAM controller and framebuffer; 720p], done,
  [6], [rendering plan, ADRs], done,
  [7], [Sinter rc.2, co-simulation in CI, `#[no_reset]`], done,
  [8], [tiled framebuffer, half-block ring, write masks; read-capture fix], done,
  [9], [setup engine: Rust models, microcode, scheduler, interpreter], done,
  [10], [setup engine in Sinter, cycle-exact against the interpreter], done,
  [11], [command parser, traversal, tile buffer: first triangles on screen], done,
  [12], [Z buffer, clears, triple buffering, spinning cube], done,
  [13], [register interface; USB serial host link; `gs-host`], done,
  [14], [v1.0 interface document: textures, combiner, memory, SPI design], done,
  [15], [memory access: uploads, reading frames back], done,
  [16], [textures: cache, perspective-correct sampling], done,
  [17], [colour combiner and alpha test], done,
  [18], [bilinear filtering: a banked texture cache], done,
  [19], [v1.0 frozen for games: performance figures], done,
  [20–25], [v1.1: faster drawing (fast clear, overlapped writes, SDRAM auto-precharge, both fragment pipeline halves at once)], done,
  [26], [v1.1 frozen: performance figures], next,
  [—], [SPI from an RP2350; alpha blending; meshes kept in SDRAM], later,
))

== Open issues

#set text(size: 13.5pt)

- *Setup speed:* 99 cycles against the brief's 85. A 5-cycle multiplier
  misses 75 MHz. Not the bottleneck at v1.0: a block's SDRAM traffic is.
- *Setup size:* 3,841 logic cells against the brief's 1,500, mostly the
  four-read-port register banks (four copies each in LUT RAM).
- *Accuracy on slivers:* 16-bit attributes reach 0.5 LSB; the reciprocal's 22
  bits limit it.
- *No formal overflow proof* for the setup program yet; ports and the
  accumulator are checked at run time over the test corpus.
- *Link bandwidth:* 3 Mbaud carries about 550 transactions a frame at 60
  fps, roughly 250 triangles; SPI or meshes in SDRAM lift it after v1.0.
- *Drawing speed:* about 190 cycles per 8×8 block (460 at v1.0), bound by
  the SDRAM: 128 words written a block, and scan-out. 64-word runs would
  save a few percent; more needs fewer words or a faster bus.
- *Core timing margin:* 85.2 MHz against 75 at Phase 25 (76.2 at v1.0);
  placement moves it by several MHz from build to build.
- *`just check` time:* about 3 minutes since Sinter rc.3 compiles
  `sinter sim` (45 before); a 16M-cycle cosim still holds 11 GB (R11).
- *Sinter gaps:* const generics; a slow `async_fifo` (needed for SPI);
  `#[no_reset]` accepted on non-registers; power-of-two memory depths;
  concurrent `sinter sim` runs sharing a `--sim-dir` (R12).
- *Off spec but working:* 750 Mb/s DVI pins; a 75 MHz (not 74.25 MHz) pixel
  clock.
