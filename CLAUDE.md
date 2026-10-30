# pico-gs2

A from-scratch rewrite of [pico-gs](https://github.com/londey/pico-gs), a
PS1/N64-era 3D graphics processor, for the ICEpi Zero v1.3 (ECP5-25K,
CABGA256, driven by an RP2350 over SPI, 640x480 DVI out). The design is
written in [Sinter](https://github.com/sinter-lang/sinter-lang), which
compiles one source to SystemVerilog **and** a cycle-accurate Rust simulator.
The generated simulator is the digital twin; there is no separate model.

## Layout

```
.devcontainer/          Dockerfile, devcontainer.json (pinned toolchain)
.claude/skills/         coding-standard skills (git submodules)
.github/workflows/      ci.yml: runs `just ci`
board/                  hand-written SystemVerilog top + constraints
  top.sv                PLL, reset, symbol re-timing, serializers, SDRAM clock and
                        DQ buffers, I/O; instantiates the Sinter core
  icepi_zero.lpf        pin constraints for the ports top.sv uses
hw/                     Sinter sources (the design)
  gpu.snt               the root: the #[clock] struct the core module is named after
  sdram.snt             SDRAM controller (#[module])
  video.snt             video timing generator and TMDS encoder (#[module]s)
  *_tb.snt              test-harness roots that expose a block's ports to `sinter sim`
  tests/                directed stimulus: <root>.<case>.csv drives hw/<root>.snt
sim/                    Rust workspace: testbenches (members include models/*)
  tb/                   integration tests driving the generated simulators
models/                 Rust models of external chips
  sdram/                W9825G6KH SDRAM: pin-level, checks datasheet timing
  reference/            sources the models were written from (datasheet, old C++)
external/icepi-zero     board repo (git submodule): pin sites, examples
scripts/  justfile      task runner; every recipe is a script in scripts/
docs/setup.md           the setup and bring-up plan
build/                  (git-ignored) generated Verilog, simulator crates, bitstreams
```

**Generated files are never committed**: Verilog, simulator crates and
bitstreams all live in `build/`.

## Commands

| Command | Does |
|---|---|
| `just gen` | `sinter build hw/gpu.snt --out build/sinter` (Verilog + simulator crates) |
| `just build` | `gen`, then `cargo build` in `sim/` |
| `just check` | `build`, then `sinter check`, the `hw/tests/*.csv` stimulus tests, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` |
| `just lint` | Verilator `--lint-only -Wall` over `board/*.sv` and the generated `.sv` |
| `just bitstream` | Yosys + nextpnr + ecppack into `build/fpga/top.bit`; fails if timing isn't met, including the clock crossings in `board/top.sv` that nextpnr reports but doesn't check |
| `just flash` | `openFPGALoader -b icepi-zero build/fpga/top.bit` (the user runs this) |
| `just ci` | everything CI runs, in order |
| `just clean` | remove `build/` and `sim/target/` |

`sim/` depends on the generated `build/sinter/gpu-sim` crate, so run
`just gen` (or `just build`) before any cargo command there.

## Sinter

Sinter is new; don't guess at it. The book (user guide + language
reference) is at
`https://github.com/sinter-lang/sinter-lang/tree/v1.0.0-rc.1/docs/book/src`
(start with `tutorial/`, `reference/state.md`, `reference/hierarchy.md`,
`reference/expressions.md`, `reference/clock-domains.md`). The generated
interfaces are in `docs/generated-verilog.md` and `docs/generated-rust.md`,
and `examples/` has worked designs, all in the same repo at that tag.

The rules that most often trip people up:

1. **A design is a state struct plus a `#[clock]` method.** One call is one
   clock cycle: it takes `self` (the current registers) and inputs, and
   returns the next state, optionally with outputs: `-> (Self, Out)`.
   Registers need reset values (`count: u8 = 0`).
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

Known gaps in v1.0.0-rc.1: `sinter build --pins` emits iCE40 PCF, not ECP5
LPF (so `board/` is hand-written); there is no co-simulation of the
generated Verilog against the simulator; `--faults` doesn't work with
`#[module]`; a `#[module]` in a child source module (`video::TmdsEncoder`)
gets a simulator crate and type named `video__…`, which rustc warns about
(harmless, in the generated crate); `async_fifo`'s write side recomputes its
fill level from the gray-coded read pointer every cycle, which only reaches
about 85 MHz on the ECP5, so the display runs on the 100 MHz core clock with
a pixel enable and `board/top.sv` re-times the TMDS symbols to the 25 MHz
pixel clock. `wait` and other SystemVerilog keywords can't name registers,
and a `fifo` capacity must be a literal.

The generated Verilog must stay lint-clean under `-Wall`. Verilator flags
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
