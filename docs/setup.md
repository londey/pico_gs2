# Set up pico-gs v2: a Sinter rewrite of the pico-gs GPU

You are setting up a **new, empty repository** for a from-scratch rewrite of
[pico-gs](https://github.com/londey/pico-gs), a PS1/N64-era 3D graphics
processor on an ECP5-25K FPGA (ICEpi Zero v1.3 board, CABGA256 package,
driven by an RP2350 over SPI, 640x480 DVI out).

The old design is hand-written SystemVerilog plus hand-written Rust "digital
twin" models. The new one is written in **Sinter**
([sinter-lang](https://github.com/sinter-lang/sinter-lang)), an RTL language
that compiles one source to SystemVerilog **and** a cycle-accurate Rust
simulator. The generated simulator *is* the digital twin, so there is no
separate model to keep in sync.

This document covers the environment and a first hardware bring-up. Work
through the phases in order, verify each before starting the next, commit
at the end of each phase, and **stop where it says to stop**. Ask the user
whenever a decision isn't covered here.

---

## What carries over from the old repository

Only the **models of chips outside the FPGA**, plus board facts:

| From old pico-gs | To new repo | Why |
|---|---|---|
| `rtl/tb/sdram_model.{cpp,hpp}`, `integration/sim/sdram_model_sim.{cpp,hpp}`, `integration/sim/test_sdram_model.cpp` | `models/reference/sdram/` | Reference for a Rust W9825G6KH model (a later milestone) |
| `external/w9825g6kh_a04.pdf` | `models/reference/sdram/` | SDRAM datasheet |
| `constraints/icepi_zero.lpf` | read only, for pin facts | SPI, GPIO, SDRAM, TMDS pin sites |

Don't copy any old RTL, twin crates, specs, scripts, or build files. You may
*read* the old repo for facts (pin sites, PLL settings, the register map),
but write everything new. Clone it outside the new repo to read it:
`git clone https://github.com/londey/pico-gs /tmp/pico-gs-old`.

---

## Sinter essentials

Sinter is new and you won't know it. Read these before writing any `.snt`:

- The book (user guide + language reference), on GitHub:
  `https://github.com/sinter-lang/sinter-lang/tree/v1.0.0-rc.1/docs/book/src`.
  Start with `tutorial/` and `reference/state.md`, `reference/hierarchy.md`,
  `reference/expressions.md`, `reference/clock-domains.md`.
- The generated interfaces: `docs/generated-verilog.md` and
  `docs/generated-rust.md` in the same repo.
- Examples: `examples/` in the same repo (`fn_hier.snt`, `fn_two_clocks.snt`,
  `fn_async_fifo.snt`, `rv32/`).

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
   extra output `(Self, Out)` becomes `out_*` ports.
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
   Sinter. Those live in a hand-written SystemVerilog top level (below).
10. **Unknown attributes and unfinished features are compile errors**, not
    silently ignored. Read the error; it usually says what to write instead.

`sinter check <file>` type-checks. `sinter sim <file> --stimulus t.csv
--vcd t.vcd` simulates with directed inputs and checks `expect.` columns.
`sinter build <file> --out <dir>` writes one `.sv` per module plus a Rust
simulator crate per module (`<module>-sim/`).

---

## Target layout

```
.devcontainer/          Dockerfile, devcontainer.json
.claude/skills/         coding-standard skills (git submodules, HTTPS URLs)
.github/workflows/      ci.yml
board/                  hand-written SystemVerilog top + constraints
  top.sv                PLL, reset, TMDS serializers, I/O; instantiates the Sinter core
  icepi_zero.lpf        pin constraints for the ports top.sv uses
hw/                     Sinter sources (the design)
  gpu.snt               the root: the #[clock] struct the core module is named after
sim/                    Rust workspace: testbenches and chip models
  tb/                   integration tests driving the generated simulator
models/                 Rust models of external chips (later), reference/ sources
external/icepi-zero     board repo (git submodule, HTTPS)
golden/                 approved reference images
scripts/  justfile      task runner (every recipe is a script)
CLAUDE.md  README.md
build/                  (git-ignored) generated Verilog, simulator crates, bitstreams
```

Generated files (Verilog, simulator crates, bitstreams) are never committed.

---

## Phase 1: devcontainer and toolchain

Write `.devcontainer/Dockerfile` (Ubuntu 24.04) and `devcontainer.json`
with:

- **oss-cad-suite 2026-01-27** (Yosys, nextpnr-ecp5, ecppack, ecppll,
  Verilator, openFPGALoader), from
  `https://github.com/YosysHQ/oss-cad-suite-build/releases/download/2026-01-27/oss-cad-suite-linux-x64-20260127.tgz`
  into `/opt/oss-cad-suite`, on `PATH`. This is the version Sinter's own CI
  tests against.
- **Rust stable** via rustup, with `rustfmt` and `clippy`.
- **just 1.54.0**.
- **sinter v1.0.0-rc.1**, the pinned static binary, checksum-verified:

  ```dockerfile
  ARG SINTER_VERSION=v1.0.0-rc.1
  RUN base="https://github.com/sinter-lang/sinter-lang/releases/download/${SINTER_VERSION}" \
      && asset="sinter-${SINTER_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
      && curl -fsSLO "${base}/${asset}" && curl -fsSLO "${base}/${asset}.sha256" \
      && sha256sum -c "${asset}.sha256" \
      && tar -xzf "${asset}" -C /usr/local/bin sinter \
      && rm "${asset}" "${asset}.sha256" \
      && sinter --version
  ```

- A non-root `vscode` user, as in Sinter's own devcontainer.
- USB passthrough isn't needed for building; flashing the board
  (`openFPGALoader -b icepi-zero`) is something the user runs.

Add the skill submodules with **HTTPS** URLs (SSH URLs break clones without a
key): `https://github.com/londey/claude-skill-rust.git`,
`https://github.com/londey/claude-skill-verilog.git`,
`https://github.com/londey/ecp5-sv-yosys-verilator-skill.git`. Add the board
repo as `external/icepi-zero` from `https://github.com/cheyao/icepi-zero.git`.

**Verify:** the image builds; `sinter --version` prints `1.0.0-rc.1`;
`yosys -V`, `nextpnr-ecp5 --version`, `verilator --version`, `just --version`
all run. If the sinter download 404s, the release tag hasn't been pushed yet:
stop and tell the user.

## Phase 2: project skeleton, task runner, CI

- `justfile` with one script per recipe in `scripts/` (strict bash), at
  least:
  - `just gen`: `sinter build hw/gpu.snt --out build/sinter` (Verilog + Rust
    simulator crates).
  - `just check`: `sinter check hw/gpu.snt`, `cargo fmt --check`,
    `cargo clippy --all-targets -- -D warnings`, `cargo test`.
  - `just lint`: Verilator `--lint-only -Wall` over `board/top.sv` plus
    every generated `.sv` (no suppression pragmas in hand-written code).
  - `just bitstream`: Yosys `synth_ecp5 -top top` over the same files,
    `nextpnr-ecp5 --25k --package CABGA256 --lpf board/icepi_zero.lpf`,
    `ecppack --compress`, into `build/fpga/`. nextpnr must meet timing; fail
    the recipe if it doesn't.
  - `just flash`: `openFPGALoader -b icepi-zero build/fpga/top.bit` (for the
    user to run).
  - `just ci`: everything CI runs, in order.
- `sim/` is a Cargo workspace. Its testbench crate depends on the generated
  crate by path (`gpu-sim = { path = "../../build/sinter/gpu-sim" }`), so
  `just gen` runs before any cargo command; make the recipes depend on it.
- `.github/workflows/ci.yml` installs the same pinned tools (cache
  oss-cad-suite) and runs `just ci`. Bitstream generation belongs in CI too,
  so a timing failure is caught.
- `CLAUDE.md` for the new repo: the layout above, the "Sinter essentials"
  list, where the book is, that generated files are never committed, that
  `board/top.sv` follows the claude-skill-verilog rules, and that Rust follows
  claude-skill-rust.
- `.gitignore`: `build/`, `target/`.

**Verify:** `just check` passes on a placeholder `hw/gpu.snt` (any minimal
design, such as a counter with a `pub` register).

## Phase 3: board bring-up

The first real design proves the whole flow on hardware: a heartbeat on the
LEDs, and a DVI test pattern.

### Clocks

The board has a 50 MHz oscillator (site `M1`). In `board/top.sv`, an
`EHXPLLL` (generate its parameters with `ecppll`) makes:

| Clock | Frequency | Use |
|---|---|---|
| `clk_core` | 100 MHz | the GPU core (Sinter domain `core`) |
| `clk_pixel` | 25 MHz | video timing (Sinter domain `pixel`) |
| `clk_tmds` | 125 MHz | TMDS serializers (5x pixel, with `ODDRX1F` DDR outputs) |

Serialize with `ODDRX1F` DDR output cells: two bits per 125 MHz clock gives
10 bits per pixel clock. The board repo's `gateware/dvi` example (and the old
pico-gs) instead shifts one bit per edge of a 250 MHz clock with no DDR
cells, which is harder to close timing on. Still read that example for its
PLL setup and TMDS pin usage. If DDR output proves troublesome, fall back to
the 250 MHz approach and tell the user.

### The Sinter core (`hw/gpu.snt`)

One struct with two domains, so the clock-crossing machinery is in place
from the start:

- `#[clock(domain = core, period = 1)]` and
  `#[clock(domain = pixel, period = 4)]` (the periods are relative: 100 MHz
  is 4x 25 MHz).
- **core:** a heartbeat counter driving the LEDs as a `pub` register (about
  1 Hz on one LED).
- **pixel:** a 640x480@60 video timing generator (800x525 total, H sync
  656–751, V sync 490–491, both active low) and a test pattern (colour bars,
  for example), then a TMDS 8b/10b encoder per channel with DC balance
  (running disparity), producing three 10-bit symbols per pixel. Make the
  timing generator and the TMDS encoder `#[module]`s.
- A `sync<core, pixel>` crossing carrying one bit from core to pixel (for
  example, invert the pattern every few seconds) to exercise a crossing.

`board/top.sv` instantiates the core, serializes the three 10-bit symbols
plus the TMDS clock channel, drives the LEDs, and makes `rst_n` from the
PLL lock and the reset button (site `C4`, active low). (On the board,
`C4` read low whatever either button did and held the core in reset, so
`rst_n` now comes from the PLL lock alone.) Take pin sites from
`external/icepi-zero/gateware/v1.3/icepi-zero-v1_3.lpf` (LEDs `led[0..]`,
clock) and the old `constraints/icepi_zero.lpf` (TMDS on the GPDI
connector). Note the old LPF's comment on the pseudo-differential TMDS
outputs.

### Tests

- **Directed:** a `sinter sim` CSV test of the timing generator's sync
  pulses at the frame boundaries, run by `just check`.
- **Twin:** a Rust integration test in `sim/tb` that steps the generated
  simulator for one full frame, decodes the TMDS symbols back to pixels (a
  small Rust TMDS decoder: the "monitor" model of the display), checks the
  sync timing, writes a PNG to `build/`, and compares it with
  `golden/test_pattern.png`. Create the golden from the first correct run,
  and ask the user to look at it before committing it.
- Unit tests of the TMDS encoder against the DVI spec's encoding algorithm,
  including running disparity staying bounded.

**Verify:** `just ci` passes: check, tests, lint, and a bitstream that
meets timing at 100/25/125 MHz.

**Stop here.** Tell the user the bitstream is ready for `just flash`, and
what they should see: a heartbeat LED and the test pattern on a DVI monitor.
Don't start on the GPU itself until they confirm it works on the board.

---

## After bring-up (not part of this task)

Listed so the setup doesn't paint itself into a corner:

1. **SDRAM**: port the C++ W9825G6KH model in `models/reference/sdram/` to a
   Rust crate `models/sdram` with its own tests against the datasheet
   timings. The SDRAM data bus is bidirectional: Sinter has no `inout`, so
   the core exposes `dq_out`, `dq_oe`, and `dq_in`, and `board/top.sv` owns
   the tristate buffers. A testbench connects the model to the generated
   simulator with `eval()` / `step()` (see `docs/generated-rust.md`, "Models
   of external chips").
2. **SPI host interface** and register file, matching the RP2350 protocol.
3. The rendering pipeline, block by block, each a `#[module]` with its own
   generated simulator crate for unit tests.

## Known gaps in Sinter v1.0.0-rc.1

Report these to the user rather than working around them silently:

- `sinter build --pins` emits iCE40 PCF constraints, not ECP5 LPF, so use the
  hand-written `board/top.sv` and LPF instead.
- There is no `sinter` subcommand that co-simulates the generated Verilog
  against the simulator (Sinter's own test suite does this). Verification
  here is the Rust twin tests plus Verilator lint and Yosys/nextpnr.
- `--faults` doesn't work in designs that use `#[module]`.
- No `inout` ports, no PLL/DDR primitives from Sinter, values at most 64
  bits.
