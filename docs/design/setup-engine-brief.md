# pico_gs2 Design Brief: Microcoded Triangle Setup Engine and Raster Front End, in Sinter

**Audience:** Claude Code, working on **pico_gs2** with Nicholas.
**Project:** pico_gs2 is a **clean-slate** design and implementation of an FPGA graphics synthesizer, written in **Sinter** (`sinter-lang`, `.snt`). Sinter is Nicholas's Rust/Rhai-inspired RTL language and compiles to a Rust cycle-accurate simulator and to portable Verilog.
**Status:** Proposal for discussion. Nothing here is decided until Nicholas confirms it in Phase 0.
**Scope:** The triangle setup engine is specified in full. The stages around it (host geometry, traversal, iterators, perspective divide) are covered in enough detail to define its interfaces.

---

## 0. How to use this document

1. **Start with Phase 0 (§10): readiness check and discussion. Write no RTL until Nicholas approves the plan.** Phase 0 covers three things: what sinter-lang can do *today*, what this design needs from it (§6), and the open decisions (§11).
2. **pico_gs2 starts from nothing.** The earlier pico-gs (SystemVerilog) is reference material only: board constraints, the host link, and lessons learned. Reuse nothing from it unless Nicholas says so.
3. **pico_gs2 drives sinter-lang's development.** If the design needs a language feature that doesn't exist or doesn't work, **don't quietly work around it in hand-written Verilog.** Record it as a sinter-lang requirement with the pico_gs2 use case, propose a stopgap (for example an extern Verilog module), and agree that stopgap with Nicholas.
4. Numbers marked **(est.)** are planning estimates. Replace them with measured values as soon as the generator, simulation or synthesis produces them.
5. Use pico_gs2's documentation and requirements conventions once they are set up in Phase 0 (spec files, ADRs, and a SysML v2 model if the project uses one).
6. For ECP5 primitives, resource budgeting and checking the Yosys/Verilator compatibility of the Verilog that Sinter emits, use the `ecp5-sv-yosys-verilator` skill if it is available.

**Fixed-point notation in this brief:** `sI.F` means signed, with I integer bits *excluding* the sign bit and F fraction bits, so 1 + I + F bits in total. `uI.F` is unsigned, I + F bits. For example s11.4 is 16 bits. When writing Sinter types, map these to Sinter's fixed-point notation (e.g. `i6.10`) **according to its sign-bit convention**. Check that convention in Phase 0; do not assume it matches this brief.

---

## 1. Context and constraints

| Item | Value |
|---|---|
| FPGA (assumed, same as pico-gs; **confirm**) | Lattice ECP5 LFE5U-25F: 24,288 LUT4, 28 × MULT18X18D, 56 × 18 Kbit EBR (DP16KD / PDPW16KD), distributed RAM (TRELLIS_DPR16X4) |
| Host (assumed; **confirm**) | RP2350 (Cortex-M33 with single-precision FPU) |
| HDL | **Sinter** → Rust cycle-accurate simulator + portable Verilog |
| Back end | Sinter Verilog → Yosys `synth_ecp5` → nextpnr-ecp5; Verilator on the emitted Verilog |
| Built-in oracle | The Sinter-generated simulator and Verilator runs of the emitted Verilog are compared against each other in continuous integration (CI) |
| Lesson from pico-gs | Routing congestion came from high-fanout reset nets. **Datapath registers, register files and RAMs get no reset.** Only control state (sequencer counter, valid bits, buffer ownership) is reset. |
| Build rule | The DSP blocks (28 multipliers) are the scarce resource; LUTs and EBR are relatively plentiful. Keep LUT use under about 50–60% so nextpnr can close timing comfortably. |

---

## 2. Background: what we take from the Nintendo 64

The N64 split graphics into two processors. The **RSP** was a programmable 8-lane, 16-bit vector unit running microcode. It did the 4×4 transforms, lighting, clipping, perspective divide and **all of triangle setup**. The **RDP** was a fixed-function rasterizer. It received finished edge equations and per-attribute gradients and only had to add them up.

Key N64 facts that influence this design:

- **Matrices:** 4×4, s15.16 fixed point, stored as 16 integer halves followed by 16 fraction halves to suit the 16-bit vector lanes. Modelview and projection were combined into one MP matrix, and each vertex was transformed once, when loaded. Nintendo advised doing matrix concatenation in floating point on the CPU for precision.
- **Perspective divide:** per vertex on the RSP, using a reciprocal from a lookup table refined by a Newton–Raphson step. `gSPPerspNormalize` scaled w into range before the divide.
- **Triangle command:** edges as YH/YM/YL (s11.2) with XH/XM/XL (s11.16) and slopes DxHDy/DxMDy/DxLDy (s13.16). Each attribute had a start value plus gradients d/dx, d/de (along the major edge) and d/dy, mostly in s15.16. A shaded, textured, Z-buffered triangle took up to 176 bytes of command.
- **Perspective correction:** the RSP sent S·W′, T·W′ and W′ (proportional to 1/w), which are linear in screen space. The RDP stepped them like any other attribute and divided per pixel, using a small piecewise-linear reciprocal table.
- **Z:** 15.3 fixed point in the blender, stored in memory as a 14-bit floating-point value plus a 4-bit ΔZ.

**What we keep:** add-only stepping of attributes (plane equations), iterating S/w, T/w and 1/w then dividing per pixel, and a compressed depth format.

**What we change:**
- Transforms happen in float on the host, using ordinary 4×4 (or 3×4 plus a sparse projection) matrices.
- Triangle setup moves into the FPGA, so the host sends plain vertices (about 28 bytes per triangle in strips) instead of large gradient packets.
- Traversal uses edge functions over blocks instead of N64-style span walking.

### Why setup is time-multiplexed

| At ~75 MHz (est.) | Fully parallel logic | RV32IM soft core | **Time-multiplexed datapath (chosen)** |
|---|---|---|---|
| Clocks per triangle | 1–4, pipelined | ~400–800 | **~80** with 1 multiplier slot |
| Triangles per second | Far above raster rate | ~0.1–0.2M | ~0.9–1.0M |
| 18×18 multipliers | Too many: doesn't fit unshared | 2–4 | 2 (1 slot) / 4 (2 slots) |
| LUT4 | ~4–6K | ~2–4K plus code RAM | ~1–1.5K |
| Changing the attribute set | Rewrite logic | Recompile | Regenerate from the op list |

Setup only has to finish within the time the *previous* triangle takes to rasterize. Time-sharing one explicitly instantiated multiplier is enough for that, whereas a CPU's per-instruction overhead is not. This fits Sinter's philosophy directly: resource sharing is explicit, never automatic.

---

## 3. Target architecture

```
Host (RP2350)                       ECP5-25K (all Sinter)
─────────────                       ────────────────────────────────────────────────────────────────
float 4×4 transform  ── link ──▶  Command FIFO ─▶ SETUP ENGINE ─▶ Traversal ─▶ Attribute ─▶ Perspective ─▶ Texture ─▶ Combine/
lighting, clipping       (CDC)    (vertex ring)   (time-multiplexed, (edge fns,  iterators    divide          cache      blend/Z
viewport, Q scaling                               triangle rate)     blocks)    (adders)     (per pixel)                 ─▶ memory
strip/fan packets
```

| Stage | Rate | Responsibility | Natural Sinter form |
|---|---|---|---|
| Host | Per vertex | Float transform, lighting, near-plane clipping (required, so that Q > 0), guard-band clipping only when outside the s11.4 range, viewport mapping to s11.4 with rounding, attribute conversion, Q scaling per draw call, packet building | (host firmware, Rust) |
| Command FIFO | Per vertex | Parses packets, loads vertices into the vertex ring. The host-link clock → core clock crossing is explicit. | Clock-domain-typed CDC primitive |
| **Setup engine** | Per triangle | Area, cull/flip, bounding box, edge equations and start values, 1/area, gradients and start values for every attribute | `multiplex fn` or ROM-driven sequencer (§5, D1) with an explicit shared multiplier |
| Traversal | Per pixel / block | Edge-function tests over blocks, top-left fill rule, pixel coverage | State machine + `pipeline fn` |
| Iterators | Per pixel | start + i·dx + j·dy using adders only | `pipeline fn` |
| Perspective divide | Per pixel | r ≈ 1/Q; S = S′·r, T = T′·r | `pipeline fn`, Static(n) latency |
| Later stages | Per pixel | Later phases | — |

**Host Q scaling:** choose one scale per draw call, from the near plane, so that Q ≤ 1 (the same idea as `gSPPerspNormalize`). Do **not** rescale per triangle, because vertices shared within a strip must have the same value for every triangle that uses them.

---

## 4. Setup engine: functional specification (language-independent)

### 4.1 Inputs

- **Vertex ring:** 4 physical vertex slots. A strip or fan loads 1 new vertex per triangle, which overwrites the oldest slot not used by the current triangle.
- **Role map:** a per-triangle register maps the roles v0, v1 and v2 to physical slots (3 × 2 bits), latched at triangle start. The setup program addresses vertices **by role**, never by slot. A *flip* swaps roles 1 and 2.
- **Vertex fields** (10 used, 16 addressable): `x, y, R, G, B, A, Z, S′, T′, Q`, each stored in its native input format (§4.3).
- **Per-triangle state:** cull mode (NONE / BACK / FRONT) and a program selector (full, untextured, flat and so on).

### 4.2 Mathematics (authoritative)

All positions are screen-space s11.4. Define:

```
dx1 = x1 − x0    dy1 = y1 − y0
dx2 = x2 − x0    dy2 = y2 − y0
area = dx1·dy2 − dx2·dy1                         (twice the signed area; exact integer)
```

**Cull and flip.** If `area == 0`, discard the triangle. If the cull mode rejects this sign, discard it. Otherwise, if `area < 0`, swap roles 1 and 2 and restart, so that all later math sees `area > 0`. The winding convention ("BACK" means area < 0 under y-down screen coordinates) is defined once in the reference model and tested.

**Edge functions** for edges 0→1, 1→2 and 2→0, with edge i→j:

```
E_ij(p) = (x_j − x_i)(p_y − y_i) − (y_j − y_i)(p_x − x_i)
A_ij = ∂E/∂x = y_i − y_j        B_ij = ∂E/∂y = x_j − x_i
```

With `area > 0`, a point is inside the triangle when all three E ≥ 0 (after the fill-rule bias). Concretely:

| Edge | A | B |
|---|---|---|
| 0→1 | −dy1 | dx1 |
| 1→2 | y1 − y2 | x2 − x1 |
| 2→0 | dy2 | −dx2 |

**Start point.** `xs, ys` = the bounding-box minimum, snapped down to the traversal block grid. The pixel centre is `xc = xs + ½`, `yc = ys + ½` (in s11.4, ½ is the constant 8).
Edge start values: `Es_ij = A_ij·(xc − x_i) + B_ij·(yc − y_i)` (exact integer).

The **top-left fill-rule bias** is applied in traversal, which already holds A and B; it is not done in setup.

**Reciprocal of area:**

```
area = m · 2^e,  m ∈ [1, 2) as u1.17           (leading-zero count and normalise)
x0   = SEED[m[16:7]]                            (1024 × 18-bit table, approximates 1/m)
r    = x0 · (2 − m·x0)                          (one Newton–Raphson step, about 17 good bits)
1/area = r · 2^−e
```

**Gradient factors** (block floating point: 18-bit mantissas sharing the implicit exponent e):

```
k1 = dy2 · r     k2 = dy1 · r     k3 = dx1 · r     k4 = dx2 · r
```

**Start-point weights** (the barycentric coordinates of the start point). These remove any dependency of a start value on its gradients:

```
cx = xc − x0     cy = yc − y0
w1 = k1·cx − k4·cy
w2 = k3·cy − k2·cx
```

**Per attribute a**, for each of R, G, B, A, Z, S′, T′ and Q:

```
d1 = a1 − a0      d2 = a2 − a0
∂a/∂x = d1·k1 − d2·k2
∂a/∂y = d2·k3 − d1·k4
a_start = a0 + d1·w1 + d2·w2       (the value at the pixel centre xc, yc)
```

**Multiply count:** area 2 + edge starts 6 + reciprocal 2 + k 4 + w 4 = 18 per triangle, plus 6 per attribute × 8 = 48. **Total 66 multiplier issues.**

### 4.3 Number formats (baseline; tune using reference-model error reports)

| Quantity | Format | Notes |
|---|---|---|
| x, y | s11.4 (16 bits) | ±2048 px guard band; 4 sub-pixel bits |
| dx, dy, A, B, cx, cy, (x − x_i) | s12.4 / s13.4 (≤ 18 bits) | Fits the 18-bit multiplier operand |
| area, Es | s24.8, ≤ 34 bits | **Exact.** Shared edges are evaluated identically, so there are no cracks |
| m, x0, r | u1.17 / u0.18 | |
| k1–k4 | 18-bit signed mantissa, exponent e implicit | Goes on the multiplier's 18-bit b-port |
| w1, w2 | ≤ 36 bits, exponent e implicit | Can be large for slivers, so it goes on the **36-bit a-port** |
| Vertex attributes | Native: R, G, B, A u8.0; Z u16.0 (or the chosen depth format); S′, T′, Q per the texture unit (D6) | Deltas d1, d2 must fit in **18 bits** when used for start values (b-port). See D7. |
| Iterator outputs (start, dx, dy) | **Baseline: s15.16 (32 bits) for every attribute**, like the N64 | Narrow colour later, using the rule below |
| Gradient outputs | Saturate on overflow | Extreme slivers can produce huge gradients; the reference model defines the exact behaviour |

**Fractional-bit rule for iterators:** `frac_bits ≥ output_frac + ⌈log2(max_steps)⌉ + 1`, where max_steps ≈ bounding-box width + height. At 640×480 that is about 12 bits for 8-bit colour and about 15 for Z. **Never narrow Q**: S′/Q amplifies any error in Q.

**Sinter fit:** Sinter's width-inferred fixed-point arithmetic and explicit narrowing casts (`sat`, `wrap`, `trunc`, `round`) express this table directly. Each multiplier result's narrowing becomes a named cast instead of a hand-computed shift. The one exception is the runtime **block-float exponent e**, which needs a runtime-variable arithmetic shift (§6, F2). The op-list generator (§7) still checks every value's range and proves that nothing overflows.

### 4.4 Micro-architecture

- **Two issue slots per clock: MUL and ALU.** The schedule is static: no interlocks and no stalls within a program. Stalls happen only at program boundaries, while waiting for a valid input triangle and a free output buffer.
- **MUL slot:** **one explicitly instantiated** signed 36 (a-port) × 18 (b-port) → 54-bit multiplier, with a static latency `MUL_LAT` (expect 4–5). After it come an accumulator for MAC/MSUB chains, a shifter (constant amount, plus e when selected), optional rounding and optional saturation to 36 bits.
  - Implementation: 2 × MULT18X18D (`a_hi` signed × b, `a_lo` unsigned × b), combined with an 18-bit offset. **Check** whether the Verilog Sinter emits lets Yosys pack this into DSP blocks (with ALU54B for the combine), or whether explicit primitive instantiation is needed (F4).
  - **Accumulator chain rule:** MUL starts a sum, and MAC/MSUB add to or subtract from the sum of the *immediately preceding* MUL-slot issue. Only the last op in a chain writes a destination. Chains are issued back to back.
  - The MUL slot reads **temporary registers only**. Every multiplier operand is a derived value.
- **ALU slot:** 36-bit, latency `ALU_LAT` (expect 1). It reads vertex fields, constants and temporaries. Ops: MOV, ADD, SUB, NEG, MIN, MAX, SNAP (round down to the block grid), LZC (normalise and write E), RCPLD (seed table lookup), TST (set N/Z flags), ADDV (add a vertex field after a per-field shift that converts native format to iterator format).
- **Address spaces** (7-bit; read space and write space differ):

| Code | Read (source) | Write (destination) |
|---|---|---|
| 0x00–0x2F | Vertex by role: `role<<4 | field` (ALU only) | Output slots 0x00–0x2F |
| 0x30–0x3F | Constants: ZERO, ONE, TWO (u1.17), HALF (s11.4 = 8), … (ALU only) | Output slots 0x30–0x3F |
| 0x40–0x5F | Temporaries, ALU bank | Temporaries, ALU bank: **written by ALU only** |
| 0x60–0x7F | Temporaries, MUL bank | Temporaries, MUL bank: **written by MUL only** |

- **Temporary register file:** 64 × 36 bits in two banks, **each with exactly one writer**. This follows Sinter's single-driver rule naturally: each bank is a ported memory whose writer is fixed by its type. Each bank needs 4 read ports (2 per slot), which means LUT RAM replicated ×4 (TRELLIS_DPR16X4), about 400–600 LUTs (est.).
- **Vertex ring:** 4 slots × 16 fields × 36 bits, read only by the ALU slot (2 read ports), written by the FIFO loader through its own port. Either LUT RAM or 2 × PDPW16KD (EBR adds a read-latency cycle). The ring discipline guarantees the loader never writes a slot named in the current role map.
- **Output buffer:** 64 slots × 36 bits, double-buffered, banked by writer. The MUL bank holds Es and the gradients; the ALU bank holds A, B, the bounding box and start values.
- **Exponent register E** (written by LZC), **flags N, Z** (written by TST), **reciprocal seed ROM** (1024 × 18 bits, one DP16KD, initialised from generated data).
- **Sequencing:** see §5. At a check point after `area` is known: if Z → abort; if the cull mode rejects → abort; if N and the triangle is kept → flip the role map and restart (wastes about 8 clocks); otherwise continue. At the end, commit the output buffer to traversal and release the vertex slot.
- **Handshake:** a program starts when a triangle is valid in the ring **and** an output buffer is free. Use Sinter's `Option`/valid types for these handshakes.
- **Reset:** only the sequencer state, valid bits, buffer ownership and the role map are reset. Nothing in the datapath is reset (F8).

### 4.5 Reference program (unscheduled; the generator is authoritative)

Destinations named `out.*` are output slots. Everything else is a temporary allocated by the generator.

```
; ---- geometry -----------------------------------------------------------
dx1 = SUB v1.x, v0.x          dy1 = SUB v1.y, v0.y
dx2 = SUB v2.x, v0.x          dy2 = SUB v2.y, v0.y
area = MUL dx1, dy2 ; MSUB dx2, dy1                 ; exact
TST area ; CHECK                                    ; early exit about 8 clocks in

; ---- bounding box and start point --------------------------------------
xmin = MIN v0.x, v1.x ; xmin = MIN xmin, v2.x       (same pattern for ymin)
out.xmax = MAX(MAX v0.x, v1.x), v2.x                (same pattern for out.ymax)
xs = SNAP xmin ; ys = SNAP ymin ; out.xs = MOV xs ; out.ys = MOV ys
xc = ADD xs, HALF ; yc = ADD ys, HALF
cx0 = SUB xc, v0.x ; cy0 = SUB yc, v0.y             ; cx0/cy0 are also cx/cy for w
cx1 = SUB xc, v1.x ; cy1 = SUB yc, v1.y
cx2 = SUB xc, v2.x ; cy2 = SUB yc, v2.y

; ---- edges --------------------------------------------------------------
out.A01 = NEG dy1       out.B01 = MOV dx1
a12 = SUB v1.y, v2.y    b12 = SUB v2.x, v1.x
out.A12 = MOV a12       out.B12 = MOV b12
out.A20 = MOV dy2       out.B20 = NEG dx2
out.E01 = MUL cy0, dx1 ; MSUB cx0, dy1              ; −dy1·cx0 + dx1·cy0
out.E12 = MUL cx1, a12 ; MAC  cy1, b12
out.E20 = MUL cx2, dy2 ; MSUB cy2, dx2

; ---- reciprocal ----------------------------------------------------------
m  = LZC area                                       ; also sets E
x0 = RCPLD m
t  = MUL m, x0                                      ; → u1.17
t2 = SUB TWO, t
r  = MUL x0, t2
k1 = MUL dy2, r     k2 = MUL dy1, r                 ; 18-bit mantissas, exponent implicit
k3 = MUL dx1, r     k4 = MUL dx2, r
w1 = MUL cx0, k1 ; MSUB cy0, k4
w2 = MUL cy0, k3 ; MSUB cx0, k2

; ---- per attribute: a ∈ {R, G, B, A, Z, S′, T′, Q} ---------------------
d1 = SUB v1.a, v0.a          d2 = SUB v2.a, v0.a
out.a_dx = MUL d1, k1 ; MSUB d2, k2        [shift by e, round, saturate]
out.a_dy = MUL d2, k3 ; MSUB d1, k4        [shift by e, round, saturate]
p        = MUL w1, d1 ; MAC  w2, d2        [shift by e, round]   ; w on a-port, d on b-port
out.a_start = ADDV p, v0.a

END
```

**Output record (37 slots):** E01, E12, E20, A01, B01, A12, B12, A20, B20, xs, ys, xmax, ymax, plus start / dx / dy × 8 attributes.
**Program variants** come from the same op list with attributes removed: untextured (no S′, T′, Q), flat (no colour gradients), and so on.

### 4.6 Scheduling and performance model

- **Critical path (1 MUL slot, MUL_LAT = 4, est.):**
  deltas (c0–1) → area (issued c1–2, ready about c6) → TST/CHECK (c6–7) → LZC (c7) → RCPLD (c8–9) → t (c10, ready c14) → t2 (c14) → r (c15, ready c19) → k1–k4 (issued c19–22).
  The 6 edge-start products fit into the multiplier's idle cycles before c19. After that, 52 k-dependent issues follow at one per clock.
  **About 80 clocks per triangle; a culled triangle takes about 8.**
- **Slot pressure:** about 66 MUL issues and about 60 ALU ops. With one MUL slot, the MUL slot is the bottleneck. With two MUL slots, the ALU becomes the bottleneck at about 55–62 clocks; getting to about 45 also needs a second ALU slot. Keep `N_MUL` and `N_ALU` as parameters, but **start with 1 + 1**.
- **Break-even:** at 1 pixel per clock, triangles averaging about 80 pixels or more hide setup completely. Measure the triangle-size distribution from real test scenes before widening the engine.
- The generator must report: schedule length per program, slot utilisation, the critical path, and peak temporary usage per bank.

---

## 5. Expressing the setup engine in Sinter

Both options below are generated from the **same Rust op list and scheduler** (§7). The choice between them is open decision **D1**.

### Option A: ROM-driven sequencer (baseline)

The Sinter design contains the datapath: the shared multiplier instance, the two register banks, the vertex ring, the output buffer, LZC, the seed ROM, and a counter-based sequencer reading a **64-bit micro-word ROM** initialised from generated data.

| Bits | Field | Width | Meaning |
|---|---|---|---|
| 63:61 | `mop` | 3 | NOP, MUL, MAC, MSUB, reserved |
| 60:54 / 53:47 / 46:40 | `ma` / `mb` / `mdst` | 7 each | MUL sources and destination |
| 39:34 | `shamt` | 6 | Constant right shift |
| 33 / 32 / 31 | `fl_e` / `fl_rnd` / `fl_sat` | 1 each | Also shift by E / round / saturate |
| 30:26 | `aop` | 5 | ALU opcode (§4.4) |
| 25:19 / 18:12 / 11:5 | `aa` / `ab` / `adst` | 7 each | ALU sources and destination |
| 4:2 | `seq` | 3 | NEXT, CHECK, END, reserved |
| 1:0 | — | 2 | Reserved |

- Needs only core language features: ported memories, an explicit shared instance, fixed-point, memory initialised from a file.
- Control costs nothing in LUTs (2 EBR). The program can change without regenerating Sinter source, and possibly without re-running place-and-route (EBR init patching).
- Weakness: the micro-word fields are untyped bits. Fixed-point formats are only checked by the generator, not by the Sinter type system.

### Option B: `multiplex fn` with a constant selector (Sinter-native)

The generator emits Sinter source: a `multiplex fn` whose arms are the scheduled cycles. Each arm calls the shared multiplier at most once and the ALU at most once. The selector is a compile-time-bound counter, so latency is `Static(n)` for each program variant, with the CHECK early exit adding a runtime branch on the transition graph.

- **What this gains from Sinter:**
  - The compiler structurally verifies that the multiplier is used by ≤ 1 arm per cycle, and inserts the operand muxes itself.
  - Fixed-point formats and narrowing casts are type-checked in every arm.
  - Unused ALU and MUL operations fold away per arm.
  - The latency contract is visible in the type.
- **Requirement:** a result issued to the pipelined multiplier in one arm must be consumable `MUL_LAT` arms later. Sinter must express or check this (Filament-style timeline / latency types, F5). If it can't yet, Option B waits for it.
- **Cost:** control becomes decoded logic, roughly 64 field functions of an ~80-state counter, about 250–400 LUTs (est.), per program variant.

**Recommendation:** build **Option A first**, using only stable language features, so the engine works early. Once Sinter supports cross-arm pipelined results, generate **Option B from the same op list** and compare LUTs, Fmax and diagnostics. The comparison is cheap because both come from one generator, and Option B is a good showcase test case for sinter-lang.

**Illustrative only (not real Sinter syntax; use the language's actual syntax):**

```
// Option B shape
multiplex fn setup(sel: const Counter<80>, mul: &SharedMul<Static<4>>, ...) -> SetupOut? {
    arm 3  => { mul.issue(MUL, dx1, dy2); t.dy1 = v1.y - v0.y; }
    arm 4  => { mul.issue(MSUB, dx2, dy1, dst: area); t.dr1 = v1.r - v0.r; }
    arm 8  => { let (m, e) = lzc(area); check(area)?; }
    ...
    arm 24 => { mul.issue(MUL, d1, k1); }
    arm 25 => { out.r_dx = mul.issue(MSUB, d2, k2) >> e |> round |> sat as i15.16; }
}
```

---

## 6. Sinter feature checklist (verify in Phase 0)

For each item, record whether it is available, partial or missing. Missing items go to sinter-lang as requirements, each with a stopgap agreed with Nicholas.

| # | Feature | Used for | Needed by |
|---|---|---|---|
| F1 | First-class fixed-point types; explicit `sat`/`wrap`/`trunc`/`round`; width-inferred arithmetic; settled sign-bit convention | All formats in §4.3 | A, B |
| F2 | Runtime-variable arithmetic shift on fixed-point / bit-vectors | Block-float exponent e; LZC normalise | A, B |
| F3 | Ported memories with declared port counts; LUT-RAM vs EBR placement hint; initialisation from a file or generated constant | Register banks, vertex ring, output buffer, seed ROM, micro-ROM | A, B |
| F4 | Extern / black-box primitive instantiation with simulation models (MULT18X18D, ALU54B, DP16KD, PDPW16KD, TRELLIS_DPR16X4), **or** emitted Verilog that Yosys reliably infers | DSP and EBR packing | A, B |
| F5 | Pipelined unit with `Static(n)` latency, callable from a sequencer or `multiplex fn`, with results consumed n cycles later | Shared multiplier | A (simple), B (checked) |
| F6 | Explicit shared resource instance plus the ≤ 1-user-per-cycle check | Shared multiplier, register-bank write ports | B |
| F7 | Clock domain as a phantom type, plus a CDC primitive | Host link → core clock | Whole design |
| F8 | Per-register reset opt-out (no reset on datapath state) | Congestion lesson (§1) | Whole design |
| F9 | Const generics / parameters | `N_MUL`, `N_ALU`, latencies, block size, per-field `align()` | A, B |
| F10 | `Option` / valid types for handshakes | Input-valid, buffer-free, output-valid | Whole design |
| F11 | Interop with a Rust build step: generated `.snt` source or memory-init data; constants and enums shared between Rust and Sinter | Op-list generator | A, B |
| F12 | A Rust API for the generated simulator (drive inputs, step, read outputs) usable from cargo tests | Bit- and cycle-exact comparison with the reference model | A, B |

---

## 7. Generator and reference model (Rust)

The op list in §4.5 is the single source of truth. A Rust crate in the pico_gs2 workspace provides:

1. **Op-list DSL** with typed fixed-point formats and declared value ranges, leading to range analysis and an overflow proof.
2. **Register allocator** that respects writer banks and the ALU-only access to vertex fields and constants.
3. **List scheduler** with resource constraints (issue slots, read ports, banks, latencies, back-to-back accumulator chains).
4. **Emitters:**
   - Option A: micro-ROM and seed-table init data, plus a Sinter constants module (opcodes, address map).
   - Option B: generated `multiplex fn` source.
   - Both: matching Rust constants.
5. **Bit-exact interpreter:** executes the scheduled program cycle by cycle. This is the golden model of the setup engine and the setup stage of pico_gs2's system-level reference model.
6. **f64 reference** for setup and rasterization, used to measure numerical error.

---

## 8. Verification plan

**Verification layers:**

| Layer | Compared against | Proves |
|---|---|---|
| f64 reference | Bit-exact interpreter | The chosen widths are accurate enough |
| Bit-exact interpreter | Sinter-generated simulator of the RTL | The RTL implements the micro-architecture: **bit-exact and cycle-exact** |
| Sinter simulator | Verilator on the emitted Verilog | The Sinter compiler is correct (the built-in oracle, run on this design's full test corpus) |
| Emitted Verilog | Yosys + nextpnr | It fits and meets timing |

**Tests:**
- **Reciprocal:** exhaustive over all 2^17 mantissas. Report worst-case relative error; target ≥ 16 correct bits.
- **Random triangles:** maximum and RMS error per output slot versus f64, reported per attribute.
- **Edge cases:** zero area, coincident vertices, slivers 1/16 px high, guard-band-sized triangles, extreme Q ratios, every cull mode × both windings, strip winding alternation.
- **Watertightness:** random fans, grids and meshes with shared edges, rasterized through the reference traversal. **Every pixel must be covered exactly once.** This also fixes the top-left classification.
- **Overflow:** the generator's proofs, plus runtime assertions in the interpreter.
- **Determinism:** the schedule is static, so the cycle count per triangle must match the generator's report exactly.
- **Synthesis:** Yosys + nextpnr on LFE5U-25F at the board's speed grade. Report LUT, FF, DSP, EBR and Fmax, and confirm DSP packing (2 MULT18X18D per MUL slot).

---

## 9. Targets (est.; replace with measured values)

| Metric | Target |
|---|---|
| Setup LUT4 | ≤ 1,500 (Option A) |
| Setup DSP | 2 MULT18X18D (+ ALU54B if used) per MUL slot |
| Setup EBR | ≤ 6 (micro-ROM 2, seed 1, vertex ring 0–2, output buffer 0–2) |
| Fmax | ≥ the core/pixel clock (provisionally 75 MHz) |
| Clocks per triangle | ≤ 85 for the full program; ≤ 10 for a culled triangle |
| Gradient accuracy | Within the output LSB budget of §4.3 for all non-saturated cases |

**Whole-device planning budget (est., ±50%):**

| Block | LUT4 | DSP | EBR |
|---|---|---|---|
| Host link + command FIFO | 600 | 0 | 2 |
| Setup engine (2 MUL slots) | 1,200 | 4 | 4 |
| Block traversal | 900 | 0 | 0 |
| Attribute iterators | 1,400 | 0 | 0 |
| Perspective divide | 400 | 3 | 1 |
| Texture cache + bilinear filter | 2,500 | 6 | 12 |
| Combine, blend, Z | 1,500 | 6 | 0 |
| Tile buffer | 300 | 0 | 4 |
| Memory controller + scanout | 2,200 | 0 | 4 |
| **Total** | **~11,000 (45%)** | **19 (68%)** | **27 (48%)** |

---

## 10. Phased plan

| Phase | Work | Exit gate |
|---|---|---|
| **0. Readiness and discussion** | (a) Survey sinter-lang's current state and fill in the §6 checklist, with a small spike `.snt` for each uncertain feature. (b) Confirm the hardware assumptions (§1). (c) Propose the pico_gs2 repo/workspace layout, documentation conventions and CI (Sinter simulator ⇄ Verilator oracle, Yosys/nextpnr). (d) Answer §11 with recommendations. Write a design note. **No functional RTL.** | Nicholas approves the plan, the decisions and the stopgaps for missing features |
| 1. Generator and reference model | DSL, allocator, scheduler, emitters, interpreter, f64 reference, and every reference-level test in §8 | Tests pass; error and schedule reports reviewed |
| 2. Setup engine in Sinter (Option A) | Datapath plus ROM sequencer; cargo tests against the interpreter | Bit-exact and cycle-exact on the full corpus; the Sinter simulator ⇄ Verilator oracle passes |
| 3. Synthesis | Yosys + nextpnr; DSP packing; timing closure | Meets §9, or deviations are explained and accepted |
| 4. Traversal + iterators | In Sinter, plus end-to-end image comparison against the reference model | Pixel-exact frames on test scenes |
| 5. Perspective divide | Per-pixel reciprocal unit | Bit-exact; texel error report |
| 6. Option B experiment | Generate the `multiplex fn` version; compare it with Option A | Comparison report; Nicholas decides which to keep |
| 7. Later stages / tiling | Separate decisions (D10) | — |

Stop at every gate and report. Don't roll several phases into one change.

---

## 11. Open decisions (recommendations in *italics*)

- **D1. Sequencer form:** ROM-driven (Option A) or `multiplex fn` (Option B). *A first, B as a phase 6 experiment from the same generator.*
- **D2. Slot counts:** *Start with `N_MUL = 1`, `N_ALU = 1`; widen only if measured triangle sizes require it.*
- **D3. Storage:** LUT RAM or EBR for the vertex ring and output buffers. *LUT RAM first.*
- **D4. Missing Sinter features:** for each gap in §6, choose between waiting, an extern Verilog stopgap, or a language change. *Prefer language changes for anything Option A needs.*
- **D5. Attribute set:** Is alpha needed? Z width? Fog, or a second texture coordinate? *Keep 8 attributes; add program variants rather than more hardware.*
- **D6. Texture coordinate formats:** native S′, T′, Q formats and the texel output format. *Settle them with the texture unit design, not inside setup.*
- **D7. Wide operands:** S′, T′ or Q deltas wider than 18 bits need a 36×36 macro (2 issues). *Add it only if the f64 comparison shows texture error.*
- **D8. Winding and culling:** the convention, and how strips alternate winding. *Handle strip alternation with the role-map flip.*
- **D9. Traversal:** block size and pixel rate. *8×8 blocks at 1 pixel per clock first.*
- **D10. Rendering mode:** immediate mode or binned tiles (a 32×32 on-chip tile buffer implies binning on the host or in hardware). *Decide after phase 4, using memory bandwidth measured on the real board.*
- **D11. Rounding:** *Round half up wherever rounding is used, defined once in the reference model.*
- **D12. Clocking:** *Setup in the same clock domain as traversal; the only crossing is at the host link, typed by Sinter's clock-domain phantom.*
- **D13. Generated files:** checked in, or produced at build time. *Produce them in the build; check in a golden copy for CI comparison.*

---

## 12. References

- N64brew wiki, RDP command reference (triangle coefficient formats): https://n64brew.dev/wiki/Reality_Display_Processor/Commands
- Nintendo 64 function reference, mirrored at ultra64.ca:
  - `gSPMatrix`: https://ultra64.ca/files/documentation/online-manuals/functions_reference_manual_2.0i/gsp/gSPMatrix.html
  - `gSPPerspNormalize`: https://ultra64.ca/files/documentation/online-manuals/functions_reference_manual_2.0i/gsp/gSPPerspNormalize.html
  - `gDPSetTexturePersp`: https://ultra64.ca/files/documentation/online-manuals/functions_reference_manual_2.0i/gdp/gDPSetTexturePersp.html
  - `gDPSetPrimDepth`: https://ultra64.ca/files/documentation/online-manuals/functions_reference_manual_2.0i/gdp/gDPSetPrimDepth.html
- N64 Programming Manual, Matrix State: https://ultra64.ca/files/documentation/online-manuals/man/pro-man/pro11/11-03.html
- ParaLLEl-RDP and angrylion-rdp-plus: reference RDP implementations (per-pixel reciprocal, edge stepping).
- J. Pineda, "A Parallel Algorithm for Polygon Rasterization", SIGGRAPH 1988 (edge functions).
- Lattice ECP5 sysDSP Usage Guide and ECP5 Memory Usage Guide (MULT18X18D / ALU54B / DP16KD / PDPW16KD behaviour).
- sinter-lang design document (`DESIGN.md` in the sinter-lang repository): execution model, `pipeline fn` / `multiplex fn`, latency classes, fixed-point types, clock domains, the single-driver rule.
