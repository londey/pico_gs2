# 0005: Simulator vs Verilator oracle

**Status:** Accepted (2026-11-04)

## Context

The Sinter-generated simulator is the digital twin, and every test runs
against it. Something has to check that the generated Verilog behaves the
same. Sinter v1.0.0-rc.2 added `sinter sim --cosim`, which runs the emitted
Verilog under Verilator over the same stimulus as the simulator and fails on
the first output that differs.

## Decision

`just check` runs every directed stimulus test with `--cosim`. The Rust
testbenches that drive the whole core (the frame test, later the renderer
tests) record their input stimulus as CSV, and `just check` replays it with
`--cosim` too, so the Verilog is checked on the same long, realistic runs as
the twin, not only on random inputs.

## Consequences

- Sinter compiler bugs show up as mismatches in CI, not on the board.
- `just check` gets slower by a Verilator build per root and the replays.
