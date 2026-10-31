# Sinter requirements from pico_gs2

Features pico_gs2 needs that Sinter v1.0.0-rc.1 lacks or only partly has,
each with the use case and the stopgap in use. Brief references (F*n*) are
to [design/setup-engine-brief.md](design/setup-engine-brief.md) §6.

| # | Requirement | pico_gs2 use case | Stopgap |
|---|---|---|---|
| R1 | Per-register reset opt-out, or reset carried on the ECP5 GSR (F8) | Setup datapath, iterators, tile buffer: only control state needs reset; pico-gs saw routing congestion from reset fanout | Datapath registers reset; fanout measured each phase (ADR 0006) |
| R2 | Co-simulation of the generated Verilog against the simulator | Catching compiler bugs on this design's whole test corpus (brief §8) | Harness in `sim/` around the emitted `<Module>_tb.cpp` (ADR 0005) |
| R3 | Memory placement hint: LUT RAM or block RAM (F3) | Vertex ring and register banks in LUT RAM; tile buffer and ROMs in block RAM | Rely on Yosys's choice; check the resource report |
| R4 | Verilog parameters on `#[verilog]` bindings (F4) | MULT18X18D / ALU54B / DP16KD with chosen register stages and modes | Inference from `*` and `mem`; a hand-written wrapper module if needed |
| R5 | Pipelined results across `multiplex fn` phases, and early exit (F5, F6) | Option B of the setup engine (brief §5) | Option A, the ROM-driven sequencer (ADR 0001) |
| R6 | Const generics or module parameters (F9) | `N_MUL`, `N_ALU`, latencies, block size | Constants; the generator owns the parameters |
| R7 | Faster `async_fifo` write side | The host link's SPI clock to core clock crossing | Not needed yet; the display already avoids it |
| R8 | ECP5 LPF from `sinter build --pins` | Board constraints | Hand-written `board/icepi_zero.lpf` |
