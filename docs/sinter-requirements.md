# Sinter requirements from pico_gs2

Features pico_gs2 needs that Sinter lacks or only partly has, each with the
use case and the stopgap in use. Brief references (F*n*) are to
[design/setup-engine-brief.md](design/setup-engine-brief.md) §6.

## Open

| # | Requirement | pico_gs2 use case | Stopgap |
|---|---|---|---|
| R6 | Const generics or module parameters (F9) | `N_MUL`, `N_ALU`, latencies, block size | rc.2's `-D` overrides; the generator owns the parameters |
| R7 | Faster `async_fifo` write side | The host link's SPI clock to core clock crossing | Not needed yet; the display already avoids it |
| R10 | Memories of any depth, not only powers of two (a convenience, low priority) | The 192-slot (6,144-word) scan-out ring; the setup engine's 37-slot output buffer. Yosys already maps a 6,144 × 16 memory onto exactly 6 DP16KD plus about 77 LUTs of read select, the same as the workaround, so this is about simpler source, not hardware. Sinter would need a rule for addresses past the end (as arrays have: the last entry, or undefined, or proved in range) | Two memories, 4,096 + 2,048 words, with a read select |
| R9 | Reject `#[no_reset]` on a field that never holds a register (a field of an `Out` or request struct) | rc.2 accepted it silently on `SdramPins` and `TmdsOut`, so the attribute missed the registers it was meant for | Review: every `#[no_reset]` is on a state field |

## Not needed

| # | Requirement | Why |
|---|---|---|
| R5 | Pipelined results across `multiplex fn` phases, and early exit (F5, F6) | Option B was dropped (ADR 0001): the setup engine is microcoded |

## Closed

| # | Requirement | Closed by |
|---|---|---|
| R1 | Per-register reset opt-out (F8) | v1.0.0-rc.2 `#[no_reset]` (ADR 0006) |
| R2 | Co-simulation of the generated Verilog | v1.0.0-rc.2 `sinter sim --cosim` (ADR 0005) |
| R3 | Memory placement hint (F3) | v1.0.0-rc.2 `#[storage(regs \| lutram \| bram)]` on `mem` and `rom` |
| R4 | Verilog parameters on `#[verilog]` bindings (F4) | v1.0.0-rc.2 `#[params(..)]` |
| R8 | ECP5 LPF from the pin map | v1.0.0-rc.2 `sinter build --constraints F.lpf` (not used: `board/top.sv` is the top level, so its LPF stays hand-written) |
