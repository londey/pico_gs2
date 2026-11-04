# Setup engine: numerics

**Status:** Phase 9 (`models/setup`): the bit-exact model, the microcode,
its scheduler and the cycle-level interpreter, done; the Sinter engine is
Phase 10.
**Spec:** [setup-engine-brief.md](setup-engine-brief.md) §4, with the
changes below, each found by measuring the model against its `f64`
reference.

Formats are Sinter's and `qfixed`'s Q notation: `Q<I, F>` has `I` integer
bits including the sign (ADR 0007). Multiplier operands are written
36-bit port × 18-bit port.

## Formats

| Value | Format | Bits | Port | Notes |
|---|---|---|---|---|
| x, y | `Q12.4` | 16 | | guard band ±2048 px |
| dx, dy, A, B, cx, cy | `Q13.4` | 17 | 18-bit | |
| 2 × area, edge values | `Q26.8` | 34 | | exact |
| mantissa `m` | `Q2.33` | 35 | 36-bit | **exact**: the brief's `u1.17` drops up to 16 of the area's bits |
| seed `x0` | `Q1.17` | 18 | 18-bit | `0.5 < x0 < 1`, 1024 entries |
| `t = m·x0`, `2 − t`, `r` | `Q2.33` | 35 | 36-bit | |
| `k = r·d` | `Q13.23` | 36 | **36-bit** | the brief's 18-bit `k` leaves a small triangle's factors a few bits |
| attribute delta | `Q17.0` | 17 | 18-bit | native units |
| iterator start, dx, dy | `Q20.16` | 36 | 36-bit | native units, every attribute |

`k` is block floating point: its true value is `k · 2^(8 − E)`, `E` being
the position of the area's leading one (0 to 32).

## The post-shifter

The multiplier's 56-bit accumulator reaches a register through a fixed left
shift of 8, then an arithmetic right shift by a constant, or by a constant
plus `E`, then optional round-half-up and optional saturation to 36 bits
(else wrap). The left shift keeps every shift non-negative for small `E`.

## Start values

The brief computes start values from the start point's barycentric weights
`w1`, `w2`. The start point is the bounding box's corner snapped to the
8×8 grid, which for a thin triangle is far outside it in barycentric terms:
the weights saturate, and the start values with them.

Instead, `start = a0 + dx·cx + dy·cy`, computed modulo 2^36 counts.
Traversal's 36-bit adders wrap the same way, so stepping from the corner
lands on the right values inside the triangle even when the corner's own
value is out of range. Gradients saturate; start values wrap. This drops
`w1`, `w2` and 4 multiplies: **62 per triangle** (brief: 66).

## Other changes

- The bounding box is clipped to the framebuffer in setup: the start point
  to (0, 0), the far corner to the last pixel centre (639.5, 359.5).
- The `untextured` program is the full one without S′, T′ and Q. The `flat`
  variant is not built yet.

## Microcode

`src/isa.rs` defines it; `src/program.rs` is the program as an op list,
`src/schedule.rs` schedules it and allocates registers, `src/interp.rs`
runs the encoded words, `src/emit.rs` and `setup-gen` write the ROMs.

- **Word:** two ROM words at one address: MUL slot (38 bits: op, a, b,
  temporary, output slot, shift, by-E, round, saturate) and ALU slot plus
  sequencer (33 bits: op, a, b, temporary, output slot, NEXT/CHECK/END).
  The brief's single 64-bit word has no room for a result to go to both a
  temporary and an output, which gradients, edge coefficients and the
  bounding box need; without it the ALU would need about 24 more MOVs.
- **Address spaces:** reads: vertex fields by role (0x00–0x2F), constants
  (0x30), ALU bank (0x40, 32 registers), MUL bank (0x60, 32). Each bank has
  one writer. Outputs: 0x00–0x12 written by the MUL slot (edge values,
  gradients), 0x20–0x31 by the ALU slot (A, B, box, starts).
- **Pipeline** (Sinter memories read first: a read in the cycle of a write
  sees the old word):

  | Operation | Operands | Result written | Latency |
  |---|---|---|---|
  | ALU | issue + 1 | end of issue + 1 | 2 |
  | RCPLD | issue + 1 | end of issue + 2 (seed ROM) | 3 |
  | MUL term | issue + 1 | accumulates at issue + 4; shifted (with E) and written at the end of issue + 5 | 6 |

- **Check and restart:** CHECK aborts on zero area or a culled winding, or
  swaps roles 1 and 2 and restarts on a negative area; the restarted pass
  doesn't cull again. Writes still in flight from the first pass land
  before the restarted pass writes or reads those registers.

### Schedule

| Program | Cycles | Critical path | Check | MUL issues | ALU issues | Temporaries (ALU, MUL) |
|---|---|---|---|---|---|---|
| full | 94 | 50 | 13 | 62 (66%) | 54 (57%) | 28, 14 |
| untextured | 74 | 50 | 13 | 44 (59%) | 45 (61%) | 22, 8 |

A rejected triangle takes 14 cycles, a flipped one 14 more than a kept
one. The brief targets 85 and 10. The gap is the chain to the gradient
factors: area, LZC, seed, `t`, `2 − t`, `r`, `k` pass through the
6-cycle multiplier four times, so `k` is ready about cycle 34 while 48 of
the 62 multiplies depend on it. Options, to weigh once Phase 10 has real
timing: a shorter multiplier pipeline (the latencies are generator
constants), or overlapping one triangle's start with the last one's
gradients (more registers).

## Verification

| Test | What it checks |
|---|---|
| `reciprocal` | the reciprocal over every mantissa |
| `accuracy` | fixed point against `f64`, by triangle shape |
| `edge_cases` | zero area, culling × winding, flips, exact edges, box, variants, guard band |
| `microcode` | interpreter = bit-exact model for 18,000 random triangles (2 programs × 3 cull modes), cycle counts as scheduled, encode/decode |
| `watertight` | the reference traversal (`src/traverse.rs`, top-left rule) covers each pixel of random meshes and fans exactly once |

Mutations caught: writes landing a cycle late (interpreter), and a fill
rule without the top-left tie-break (traversal).

Not done yet: a formal range analysis of the op list. Port widths and
accumulator overflow are asserted at run time over the test corpus
instead. And `setup-gen` isn't in `just gen` yet: the `sim` workspace
needs the Sinter crates to exist before any of its members can run, so
Phase 10 has to break that cycle when the engine first loads the ROMs.

## Accuracy

`cargo test -p setup-model -- --nocapture` prints the full report.

| Test | Result |
|---|---|
| Reciprocal, all 2^17 mantissa top bits × 3 low patterns | ≥ 21.99 correct bits (brief: 16) |
| Ordinary triangles (≥ 1 px², every altitude ≥ 1 px), 8 to 400 px, random attributes | worst error inside the triangle: colour 1.1 × 10⁻² LSB, 16-bit attributes 1.8 × 10⁻² LSB; no saturation |
| Slivers (altitude < 1 px) | colour < 1.1 × 10⁻² LSB; 16-bit attributes up to 0.5 LSB; 16-bit gradients saturate on 1–3% of them |
| Tiny triangles (< 1 px²) | colour < 5 × 10⁻³ LSB; 16-bit attributes up to 0.6 LSB; 16-bit gradients saturate on about 5% of them |

The tests' 16-bit attributes are random over their whole range, so a
sliver's Z can change by 65535 across a fraction of a pixel: harsher than
real meshes. The remaining error inside a triangle is the reciprocal's
22 bits times the gradient times the triangle's size.
