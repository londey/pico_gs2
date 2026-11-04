# 0001: Microcoded triangle setup engine

**Status:** Accepted (2026-11-04)

## Context

Triangle setup (area, culling, edge equations, 1/area, per-attribute
gradients and start values) takes 66 multiplies per triangle. Fully
parallel logic doesn't fit; a soft CPU is ten times too slow. The brief
([../design/setup-engine-brief.md](../design/setup-engine-brief.md)) proposes
a time-multiplexed datapath with a statically scheduled program, built
either as a ROM-driven sequencer (Option A) or as a Sinter `multiplex fn`
(Option B). Sinter's `multiplex fn` completes each multiply inside its phase,
so it can't express the pipelined multiplier Option B needs.

## Decision

- **Option A:** a Rust generator schedules the brief's op list and emits a
  64-bit micro-ROM and the reciprocal seed table, loaded with `include_hex!`.
  The Sinter datapath has one pipelined 36×18 multiplier (MUL slot) and one
  36-bit ALU (ALU slot).
- **Slots:** `N_MUL = 1`, `N_ALU = 1`, as generator parameters.
- **Storage:** LUT RAM for the vertex ring, the two temporary banks and the
  double-buffered output buffer.
- **Attributes:** the full program computes all 8 (R, G, B, A, Z, S′, T′, Q);
  variants drop attributes. Traversal uses R, G, B and Z at first.
- **Culling:** BACK culls area < 0 in y-down screen space; strips alternate
  winding by flipping roles 1 and 2.
- **Rounding:** round half up wherever the program rounds.
- **Multiplier:** a 3-stage `pipeline fn` (operands registered, multiply,
  product registered), then accumulate and shift as a fourth stage.
- **Models:** a typed bit-exact Rust version of the setup math, using the
  `qfixed` crate's `Q`/`UQ` types (the same Q convention as Sinter), and a
  cycle-level interpreter of the scheduled microcode with the pipeline
  registers explicit; the interpreter must match the typed version bit for
  bit, and the Sinter engine must match the interpreter cycle for cycle.

## Consequences

- About 80 clocks per triangle, 8 for a culled one (est.).
- Changing the program means re-running the generator, not editing Sinter.
- Micro-word fields are plain bits to Sinter: formats are checked by the
  generator's range analysis, not by Sinter's types.
- `multiplex fn` (Option B) is not used and not planned (2026-11-04). It
  completes each multiply within its phase, so the multiplier can't be
  pipelined; its latency is fixed, so there's no early exit for culled
  triangles and no program chosen at run time; and every temporary carried
  across a `yield` becomes a flip-flop selected by phase, where the
  microcoded engine keeps temporaries in two small LUT-RAM banks addressed
  by the micro-word.
