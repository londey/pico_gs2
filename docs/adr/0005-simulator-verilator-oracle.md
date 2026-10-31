# 0005: Simulator vs Verilator oracle

**Status:** Proposed (2026-10-31)

## Context

The Sinter-generated simulator is the digital twin, and every test runs
against it. Nothing checks that the generated Verilog behaves the same:
Sinter rc.1 has no co-simulation, though `sinter build` writes a Verilator
harness (`<Module>_tb.cpp`) that reads one `u64` per input per cycle and
writes one per output per cycle.

## Decision

A harness in `sim/` builds each generated module's `<Module>_tb.cpp` with
Verilator, drives it and the Sinter simulator with the same stimulus
(directed and random), and compares every output on every cycle. It runs in
`just check`. Co-simulation built into Sinter is recorded as a sinter-lang
requirement.

## Consequences

- Sinter compiler bugs show up as mismatches in CI, not on the board.
- `just check` gets slower by a Verilator build per module.
- The harness is retired if Sinter gains co-simulation.
