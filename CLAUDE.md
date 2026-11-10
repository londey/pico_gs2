# pico-gs2

A from-scratch rewrite of [pico-gs](https://github.com/londey/pico-gs), a
PS1/N64-era 3D graphics processor, for the ICEpi Zero v1.3 (ECP5-25K,
CABGA256, driven by an RP2350 over SPI, DVI out). The design is
written in [Sinter](https://github.com/sinter-lang/sinter-lang), which
compiles one source to SystemVerilog **and** a cycle-accurate Rust simulator.
The generated simulator is the digital twin; there is no separate model.

## Layout

```
.devcontainer/          Dockerfile, devcontainer.json (pinned toolchain)
.claude/skills/         coding-standard skills (git submodules)
.github/workflows/      ci.yml: runs `just ci`
board/                  hand-written SystemVerilog top + constraints
  top.sv                PLL, reset, edge clocks, ODDRX2F gearboxes, SDRAM clock and
                        DQ buffers, I/O; instantiates the Sinter core
  tmds_gearbox.sv       TMDS symbols (clk_core) to 4-bit nibbles (clk_sclk)
  tb/                   Verilator testbenches for board/*.sv, run by `just check`
  icepi_zero.lpf        pin constraints for the ports top.sv uses
hw/                     Sinter sources (the design)
  gpu.snt               the root: the #[clock] struct the core module is named after
  sdram.snt             SDRAM controller (#[module])
  video.snt             video timing generator and TMDS encoder (#[module]s)
  setup.snt             triangle setup engine (#[module])
  raster.snt            command parser (register transactions from the scene ROM or
                        the host link) and rasteriser (clear, traversal, depth test,
                        tile buffers)
  link.snt              host link (#[module]): USB UART at 3 Mbaud, transaction FIFO,
                        credits, read replies
  setup_tb.snt          the engine's test root (sim/tb drives it); setup_synth.snt its
                        synthesis root
  *_tb.snt              test-harness roots that expose a block's ports to `sinter sim`
  tests/                directed stimulus: <root>.<case>.csv drives hw/<root>.snt
sim/                    Rust workspace: testbenches (members include models/*)
  tb/                   integration tests driving the generated simulators
host/                   the host side (docs/design/host-interface.md), sim/ members
  proto/                gs-proto: no_std registers, transactions, framing, credits
  gs-host/              PC tool: drives the GPU over the USB UART (`just gs-host`)
models/                 Rust models of external chips
  sdram/                W9825G6KH SDRAM: pin-level, checks datasheet timing
  setup/                setup engine models, microcode generator (setup-gen) and
                        interpreter, the scenes (scene.rs: test card, spinning cube),
                        the reference renderer (render.rs); a Cargo workspace of its
                        own (just gen runs it first)
  reference/            sources the models were written from (datasheet, old C++)
external/icepi-zero     board repo (git submodule): pin sites, examples
scripts/  justfile      task runner; every recipe is a script in scripts/
docs/setup.md           the setup and bring-up plan
docs/design/            design notes: triangle-pipeline.md (the rendering plan and
                        phases), setup-engine-brief.md (the setup engine spec),
                        setup-engine.md (its numerics, as built), host-interface.md
                        (registers and the host link; v1.0, figures v1.1),
                        draw-speed.md (v1.1: faster drawing, same interface; done)
docs/adr/               architecture decision records, NNNN-title.md
docs/slides/            Typst architecture slides (Touying + Fletcher, the theme in
                        theme/pico.typ, Inter and JetBrains Mono); `just slides`
docs/sinter-requirements.md  Sinter gaps pico_gs2 needs closed, with stopgaps
build/                  (git-ignored) generated Verilog, simulator crates, bitstreams
```

**Generated files are never committed**: Verilog, simulator crates and
bitstreams all live in `build/`.

## Commands

| Command | Does |
|---|---|
| `just gen` | `setup-gen` (micro-ROMs into `build/gen/`), then `sinter build` of `hw/gpu.snt` into `build/sinter` and `hw/setup_tb.snt` into `build/sinter-setup` |
| `just build` | `gen`, then `cargo build` in `sim/` |
| `just check` | `build`, then `sinter check`, `just lint`, `just perf --check` (last), the `hw/tests/*.csv` stimulus tests, the `board/tb` testbenches, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, then the stimulus the Rust tests recorded in `build/stimulus/`; every stimulus file runs with `--cosim` (simulator vs Verilator) |
| `just perf` | the performance figures (host-interface §15): `sim/tb/examples/perf.rs`, in release mode, draws a set of scenes on the twin and prints their draw times and SDRAM use; `just perf --record` records them in `sim/tb/perf-baseline.txt`, which `just check` holds the design to (no scene more than 0.5% slower) |
| `just lint` | Verilator `--lint-only -Wall` over `board/*.sv` and the generated `.sv` |
| `just bitstream` | Yosys + nextpnr + ecppack into `build/fpga/top.bit`; fails if timing isn't met, including the clock crossings in `board/top.sv` that nextpnr reports but doesn't check |
| `just setup-report` | place and route the setup engine alone at 75 MHz; resources, Fmax, critical path |
| `just slides` | `typst compile` each `docs/slides/*.typ` into `build/slides/*.pdf` |
| `just gs-host` | build `sim/target/release/gs-host`, the PC tool for the host link (the user runs it on the host) |
| `just flash` | `openFPGALoader -b icepi-zero build/fpga/top.bit` (the user runs this) |
| `just ci` | everything CI runs, in order |
| `just clean` | remove `build/` and `sim/target/` |

`sim/` depends on the generated `build/sinter/gpu-sim` crate, so run
`just gen` (or `just build`) before any cargo command there.

## Sinter

Sinter is new; don't guess at it. The book (user guide + language
reference) is at
`https://github.com/sinter-lang/sinter-lang/tree/v1.0.0-rc.3/docs/book/src`
(start with `tutorial/`, `reference/state.md`, `reference/hierarchy.md`,
`reference/expressions.md`, `reference/clock-domains.md`). The generated
interfaces are in `docs/generated-verilog.md` and `docs/generated-rust.md`,
and `examples/` has worked designs, all in the same repo at that tag.

The rules that most often trip people up:

1. **A design is a state struct plus a `#[clock]` method.** One call is one
   clock cycle: it takes `self` (the current registers) and inputs, and
   returns the next state, optionally with outputs: `-> (Self, Out)`.
   Registers need reset values (`count: u8 = 0`). `#[no_reset]` on a field
   makes its registers power-on only: datapath state is `#[no_reset]`, only
   control state is reset (ADR 0006).
2. **No implicit conversions, not even widening.** `u8 + u8` is a `u9`;
   storing it in a `u8` needs `as u8 wrap` (or `sat`). Comparison operands
   must have the same type. A bare literal takes its type from context.
3. **`&`, `|`, `^` bind more loosely than `==`**, unlike Rust:
   `a == 1 & b == 0` means `(a == 1) & (b == 0)`. `!` only takes a `u1`.
4. **Every field is named in a struct value**, or use struct update:
   `Self { count: n, ..self }`.
5. **`pub` fields of the top struct become `<field>_q` output ports**; an
   extra output `(Self, Out)` becomes `out_*` ports (`<domain>_out_*` with
   several clock domains).
6. **`#[module]` on a struct makes it a separate Verilog module.** It has one
   step method `fn step(self, ..) -> (Self, Out)` with `Out` a struct; its
   ports are the parameters and `out_<field>`. Its parent must call `step`
   exactly once per cycle, outside any `if`/`match`, and can't read its
   state, only `Out`.
7. **Memories (`mem<T, N>`) read with one cycle of latency**: `value()` is
   the word addressed by the previous cycle's `next(raddr, we, waddr,
   wdata)`. `next` is called once per cycle, unconditionally.
8. **Clock domains** are `#[domain(d)]` fields with one
   `#[clock(domain = d, period = p)]` method each. A domain can't read another
   domain's registers; crossings go through `sync`, `pulse`, `mailbox`, or
   `async_fifo`. The module gets one `clk_<d>` port per domain and a single
   `rst_n`, synchronized internally per domain.
9. **Values are at most 64 bits.** There is no division, no `inout` port, and
   no way to instantiate a PLL or DDR primitive *with parameters* from
   Sinter. Those live in `board/top.sv`.
10. **Unknown attributes and unfinished features are compile errors**, not
    silently ignored. Read the error; it usually says what to write instead.

v1.0.0-rc.2 added `#[no_reset]`, `#[storage(regs|lutram|bram)]` on memories,
`-D NAME=VALUE` constant overrides, `#[params(..)]` on `#[verilog]` bindings,
ECP5 LPF output (unused: `board/top.sv` is the top level) and
`sinter sim --cosim`, which `just check` runs on every stimulus file (ADR
0005). v1.0.0-rc.3 (pinned) compiles `sinter sim`: it builds the design's
Rust simulator with cargo in `--sim-dir` and runs that, about 90× faster than
rc.2's interpreter on the GPU (`--interpret` keeps the interpreter); `--quiet`
drops the per-cycle table. Runs sharing a `--sim-dir` overwrite each other's
stimulus there, so give concurrent runs their own (`check.sh` does). Known
gaps: `--faults` doesn't work with
`#[module]`; a `#[module]` in a child source module (`video::TmdsEncoder`)
gets a simulator crate and type named `video__…`, which rustc warns about
(harmless, in the generated crate); `async_fifo`'s write side recomputes its
fill level from the gray-coded read pointer every cycle, which only reaches
about 85 MHz on the ECP5, so the design avoids it: the display runs on the
core clock, which is also the pixel clock (75 MHz for 720p). `wait` and other
SystemVerilog keywords can't name registers, and a `fifo` capacity must be a
literal.

A `#[module]` whose output depends combinationally on its inputs can make
Verilator report a false combinational loop (UNOPTFLAT) when the parent
computes those inputs from that output in its single `always_comb`; register
such outputs (as `SdramController` does `accepted`).

The colour and depth buffers carry cleared flags (`docs/design/draw-speed.md`
§3.1): a cleared block's SDRAM words are stale, so a test reads a buffer as
the GPU does, through `Bench::buffer_word` or `Bench::buffer`, never the
SDRAM model directly; and reads the shown buffer while it is shown
(`Bench::snapshot`), since the GPU may draw into it right after.

The generated Verilog must stay lint-clean under `-Wall`. `just lint` passes
`--unused-regexp '*debug*'`: module outputs named `debug_*` exist only for
tests (the setup engine's write ports) and may go unused in the GPU. Verilator flags
unused bits of named wires, so don't bind a wide intermediate with `let` and
narrow it later: cast in the same expression, or build shifted values with
`@` and slices instead of `<<`.

## Coding standards

- `board/*.sv` follows the **claude-skill-verilog** skill (`always_ff` holds
  only simple assignments, logic in `always_comb`, every signal commented,
  lint-clean under `-Wall` with no suppression pragmas) and the
  **ecp5-sv-yosys-verilator** skill for primitives (`ifdef SYNTHESIS` real
  primitive, else a behavioural model for Verilator).
- Rust in `sim/` and `models/` follows the **claude-skill-rust** skill.
- The generated Verilog and Rust are Sinter's; don't edit or lint-fix them.
