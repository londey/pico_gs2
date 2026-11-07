# 0008: A register host interface

**Status:** Proposed (2026-11-07)

## Context

ADR 0002 put the built-in scene in a ROM of command packets "in the same
format the host link will carry": a header word per strip or fan, then four
32-bit words per vertex, plus clear and end-of-frame commands. That format
was designed to drive the pipeline from a ROM, not as a host protocol: no
render state beyond a cull mode, no reads, no room to grow.

pico-gs, which this project rewrites, had a register interface: 72-bit
transactions (read/write, 7-bit address, 64-bit data) through a command
FIFO, with vertex registers that kick triangles. A lot of thought went into
it, but it also carries features this pipeline doesn't have, and its
three-slot vertex buffer overwrites a fan's first vertex.

## Decision

The host drives the GPU with register writes and reads in pico-gs's 72-bit
transactions, using its registers as inspiration rather than a
specification (docs/design/host-interface.md):

- `ATTR0`: a latched colour (RGBA8888) and S′T′; `ATTR1` reserved for a
  second colour and texture.
- `VERTEX_NOKICK`, `VERTEX_KICK_012`, `VERTEX_KICK_021` and
  `VERTEX_KICK_FAN`, each X, Y, Z and Q in 16 bits. A `NOKICK` after a kick
  starts a primitive whose first vertex fans pivot on.
- `RENDER_MODE`: cull mode, `Z_TEST`, `Z_WRITE`, `COLOR_WRITE`, and the
  untextured program.
- `FRAME`: present, clear colour and clear depth in one register, the
  buffers staying the GPU's (ADR 0003); no `MEM_FILL`.
- `STATUS` and `ID` to read.

The first link is the board's USB UART at 3 Mbaud with credit flow control
and DTR as its enable; SPI from an RP2350 follows with the same
transactions. The built-in scene becomes register writes in ROM.

## Consequences

- A vertex is 18 bytes on the link (two transactions), against 16 for the
  packet format and 27 for pico-gs's; a vertex reusing the last `ATTR0` is 9.
- The host chooses each strip triangle's winding by its kick, so the parser
  no longer tracks strip parity.
- The packet format of ADR 0002 goes; its built-in scene stays, as
  transactions.
- Render-to-texture, memory upload, blending and depth functions other than
  "less than" are left for later registers.
