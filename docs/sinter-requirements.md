# Sinter requirements from pico_gs2

Features pico_gs2 needs that Sinter lacks or only partly has, each with the
use case and the stopgap in use. Brief references (F*n*) are to
[design/setup-engine-brief.md](design/setup-engine-brief.md) §6.

## Open

| # | Requirement | pico_gs2 use case | Stopgap |
|---|---|---|---|
| R5 | Pipelined results across `multiplex fn` phases, and early exit (F5, F6) | Option B of the setup engine (brief §5) | Option A, the ROM-driven sequencer (ADR 0001) |
| R6 | Const generics or module parameters (F9) | `N_MUL`, `N_ALU`, latencies, block size | rc.2's `-D` overrides; the generator owns the parameters |
| R7 | Faster `async_fifo` write side | The host link's SPI clock to core clock crossing | Not needed yet; the display already avoids it |

## Closed

| # | Requirement | Closed by |
|---|---|---|
| R1 | Per-register reset opt-out (F8) | v1.0.0-rc.2 `#[no_reset]` (ADR 0006) |
| R2 | Co-simulation of the generated Verilog | v1.0.0-rc.2 `sinter sim --cosim` (ADR 0005) |
| R3 | Memory placement hint (F3) | v1.0.0-rc.2 `#[storage(regs \| lutram \| bram)]` on `mem` and `rom` |
| R4 | Verilog parameters on `#[verilog]` bindings (F4) | v1.0.0-rc.2 `#[params(..)]` |
| R8 | ECP5 LPF from the pin map | v1.0.0-rc.2 `sinter build --constraints F.lpf` (not used: `board/top.sv` is the top level, so its LPF stays hand-written) |
