# 0006: Only control state is reset

**Status:** Accepted (2026-11-04)

## Context

pico-gs found that high-fanout reset nets caused routing congestion, so the
brief resets only control state: sequencer counters, valid bits, buffer
ownership. Sinter v1.0.0-rc.1 reset every register asynchronously; v1.0.0-rc.2
added `#[no_reset]`, which gives a field's registers a power-on value only.

## Decision

Datapath registers, register banks, buffers and memories' read registers
are `#[no_reset]`. Control state keeps its reset. Each phase that adds logic
reports the reset net's fanout. The ECP5's global set/reset (GSR) isn't
used; it can be revisited if the reports show the remaining reset net
matters.

## Consequences

- A `#[no_reset]` register keeps its value through a reset, so a design must
  not depend on it being cleared: control state decides when it is valid.
- The stimulus tests can assert `rst_n` mid-run to check that.
