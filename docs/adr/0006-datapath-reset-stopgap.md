# 0006: Reset datapath registers for now

**Status:** Proposed (2026-10-31)

## Context

pico-gs found that high-fanout reset nets caused routing congestion, so the
brief resets only control state. Sinter needs a reset value on every
register, and its Verilog resets each one asynchronously (`negedge rst_n`).
Only `pipeline fn` registers and memory contents have no reset. The ECP5 has
a global set/reset network (GSR) that might carry the reset without using
general routing, possibly needing a primitive or attributes in the generated
Verilog.

## Decision

Datapath registers get reset values for now. Phase 7 finds out what Yosys and
nextpnr do with Sinter's reset (local set/reset per slice, or GSR) and
whether GSR is usable. Each later phase reports the reset net's fanout and
any timing or congestion it causes. Per-register reset opt-out, or a GSR
option in Sinter's code generation, is recorded as a sinter-lang requirement,
to be built if the reports show it matters.

## Consequences

- The design works on today's Sinter.
- Reset fanout grows with the datapath; it's measured, not assumed.
