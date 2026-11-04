# 0007: Sinter fixed-point notation

**Status:** Accepted (2026-11-04)

## Context

The brief writes `sI.F` with `I` excluding the sign bit (s11.4 is 16 bits).
Sinter writes `iI.F` with `I` including it (`i12.4` is 16 bits).

## Decision

Code, comments and design notes use Sinter's notation: `iI.F` and `uI.F`,
with `I` counting the sign bit. The brief stays as written;
[../design/triangle-pipeline.md](../design/triangle-pipeline.md) §5 translates
its formats.

## Consequences

One notation in everything new, matching the types in the source.
