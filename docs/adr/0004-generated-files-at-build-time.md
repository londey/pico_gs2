# 0004: Generated files at build time only

**Status:** Accepted (2026-11-04)

## Context

The setup generator produces the micro-ROM and the reciprocal seed table.
The brief (D13) suggests committing a golden copy for CI to diff against.
The project's rule is that generated files are never committed.

## Decision

The generator runs in `just gen`, before `sinter build`, writing into
`build/gen/`. CI checks the generator's schedule report (cycles per program,
slot use, peak temporaries) against limits in cargo tests, not against a
committed copy. Constants shared by Rust and Sinter live in a hand-written
`.snt` file that a test checks against the generator.

## Consequences

- No generated file to keep in step by hand.
- A schedule change shows up as a test failure on a limit, not as a diff;
  limits are tightened as the schedule improves.
