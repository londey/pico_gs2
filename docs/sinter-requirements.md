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
| R11 | `sinter sim --cosim` on long stimulus in less memory | `just check` replays each Rust test's recorded stimulus (up to 16 million cycles, for the host test) against the Verilog. rc.3's compiled simulator made it fast (34 s for that run), but a run still holds about 0.7 KB a cycle (11 GB for the host run) | `check.sh` runs the replays largest first, as many at once as memory allows (one at a time on 16 GB); tests keep idle link time out of their recordings |
| R12 | Concurrent `sinter sim` runs of one design sharing a `--sim-dir` | `check.sh` runs the directed tests of one root (two `video_timing_tb` cases) in parallel. rc.3 writes the stimulus to a fixed `<sim-dir>/<Module>/stimulus.bin`, so two runs at once overwrite each other's input: one case ran the other's stimulus and failed its expectations and the cosim | Each run gets its own `--sim-dir build/sim/<case>` (a release build of a few seconds each) |

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
| R11a | Faster `sinter sim` on long stimulus | v1.0.0-rc.3 compiles the simulator: the GPU at about 2.3 million cycles a second against rc.2's 26,000, and `just check` in under 3 minutes against 45 |
| R8 | ECP5 LPF from the pin map | v1.0.0-rc.2 `sinter build --constraints F.lpf` (not used: `board/top.sv` is the top level, so its LPF stays hand-written) |
