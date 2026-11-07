# Host interface

How a host drives the GPU: a register model, its transactions, and the
links that carry them. Taken from pico-gs's register interface (INT-010,
INT-012 in that repo) as inspiration, not followed exactly: the registers
here are the ones this pipeline can honour, with pico-gs's addresses where a
register means the same thing. ADR [0008](../adr/0008-register-host-interface.md)
records the decision.

Status: proposed for Phase 13.

## 1. Phases

| Phase | Branch | Work | Gate |
|---|---|---|---|
| 13 | `phase-13-host-link` | The register model (§3, §4); the serial link over the board's USB UART (§5); the built-in scene as register writes (§6); a Rust host library and a PC tool, `gs-host` (§7). | Twin tests pass (§8); on the board, the cube plays from ROM with no host, `gs-host` streams a scene and reads `ID` and `STATUS`, and the ROM scene resumes when it exits; link throughput measured |
| 14 | `phase-14-spi-host` | The same transactions over SPI from an RP2350 on the GPIO header, with a CMD_FULL line; Rust firmware (`no_std`, sharing the host library). | The scene streamed from the RP2350 on the board |

## 2. Transactions

A transaction is 72 bits, as in pico-gs:

| Bits | Field |
|---|---|
| 71 | R/W̄: 1 read, 0 write |
| 70:64 | register address |
| 63:0 | data (a write's value; absent from a read) |

Transactions take effect in order. A write goes into the command FIFO and is
carried out when the parser reaches it. A read also goes through the FIFO,
so it is a fence: it is answered once everything before it has been taken
by the parser (not necessarily drawn). Writes to unused addresses are
ignored; reads of them return 0.

## 3. Registers

| Address | Name | Access | Contents |
|---|---|---|---|
| 0x00 | `ATTR0` | W | colour and texture coordinates for the vertices that follow |
| 0x01 | `ATTR1` | — | reserved: a second colour (specular) and a second texture's S′T′ |
| 0x06 | `VERTEX_NOKICK` | W | a vertex; draws nothing |
| 0x07 | `VERTEX_KICK_012` | W | a vertex; draws (two back, one back, this one) |
| 0x08 | `VERTEX_KICK_021` | W | a vertex; draws (two back, this one, one back): the other winding |
| 0x09 | `VERTEX_KICK_FAN` | W | a vertex; draws (the primitive's first, one back, this one) |
| 0x30 | `RENDER_MODE` | R/W | cull mode, depth and colour write enables, program |
| 0x40 | `FRAME` | W | present the frame, and clear |
| 0x7E | `STATUS` | R | frames shown, the last frame's cycles, FIFO space |
| 0x7F | `ID` | R | identification and interface version |

### 0x00 `ATTR0`

| Bits | Field |
|---|---|
| 63:56 | R (UQ8.0) |
| 55:48 | G |
| 47:40 | B |
| 39:32 | A (carried, unused until blending) |
| 31:16 | S′ = S/W (Q-format as the texture unit will define; carried, unused until texturing) |
| 15:0 | T′ = T/W |

Latched: every vertex written takes the `ATTR0` last written. A batch of one
colour writes it once. Reset value 0 (black, S′ = T′ = 0).

### 0x06–0x09 `VERTEX_*`

| Bits | Field |
|---|---|
| 63:48 | X, signed, 1/16 pixel (Q12.4) |
| 47:32 | Y, signed, 1/16 pixel (Q12.4) |
| 31:16 | Z, unsigned, 0 nearest, 0xFFFF farthest |
| 15:0 | Q = 1/W (UQ1.15; carried, unused until texturing) |

Each write adds a vertex: this position with the latched `ATTR0`. These are
today's scene vertex words 0 and 2 (position; Z and Q) in one register,
and `ATTR0` is words 1 and 3.

**Primitives.** A `VERTEX_NOKICK` that follows a kick (or is the first
vertex after reset or a link reset) starts a new primitive; its first
vertex is the one `VERTEX_KICK_FAN` pivots on. A kick draws only once the
primitive has three vertices; before that it just adds the vertex.

- Strip: `NOKICK, NOKICK, KICK_012, KICK_021, KICK_012, …` (the host
  alternates, which keeps one winding).
- Fan: `NOKICK, NOKICK, KICK_FAN, KICK_FAN, …`.
- Separate triangles: `NOKICK, NOKICK, KICK_012` each.

The GPU chooses the setup engine's vertex ring slots (four: a fan's first
vertex keeps one, the rest cycle), as the parser does now; the host never
sees them. Each triangle takes `RENDER_MODE` as it is at its kick.

### 0x30 `RENDER_MODE`

| Bits | Field | Reset |
|---|---|---|
| 8 | `TEXTURED`: 0 runs the untextured setup program (80 cycles, not 99) and ignores S′, T′, Q | 0 |
| 6:5 | `CULL`: 0 none, 1 discard anticlockwise (on screen), 2 discard clockwise, 3 none | 0 |
| 4 | `COLOR_WRITE`: write the colour buffer | 1 |
| 3 | `Z_WRITE`: write the depth buffer where a pixel is drawn | 1 |
| 2 | `Z_TEST`: draw a pixel only if nearer (less than) | 1 |

Bit positions follow pico-gs's where the field exists there. The depth test
is "less than" only; pico-gs's `Z_COMPARE` and blending are left out until
something needs them. `CULL` = 1 is the setup engine's `Back` mode, 2 its
`Front`.

### 0x40 `FRAME`

| Bits | Field |
|---|---|
| 31:16 | clear colour, RGB565 |
| 2 | `CLEAR_DEPTH`: set the depths to 0xFFFF |
| 1 | `CLEAR_COLOUR`: fill with the clear colour |
| 0 | `PRESENT`: the frame being drawn is done |

In order: if `PRESENT`, the frame being drawn is handed to the display
(shown from the next frame start) and drawing moves to the next buffer;
then the clears apply to the buffer now being drawn. So one write,
`PRESENT | CLEAR_COLOUR | CLEAR_DEPTH`, ends a frame and starts the next;
the clears alone clear the current one (the first frame, or a depth-only
clear mid-frame).

The buffers stay the GPU's (ADR 0003, Phase 12): three colour buffers, one
shown, one drawn, the third finished and waiting or spare. A `PRESENT`
waits, holding up the commands behind it, while a finished frame is
already waiting; the host sees this as the FIFO not draining. Buffer
addresses never reach the host. Render-to-texture would add `FB_CONFIG`-style
registers beside `FRAME` later.

### 0x7E `STATUS`

| Bits | Field |
|---|---|
| 49:40 | free FIFO entries |
| 39:16 | cycles the last finished frame took to draw, at 75 MHz |
| 15:0 | frames shown since reset (wraps) |

### 0x7F `ID`

| Bits | Field |
|---|---|
| 63:32 | `0x50475332`, "PGS2" |
| 31:16 | interface version: 1 |
| 15:0 | 0 |

## 4. What happens to the parser

The parser takes transactions instead of packet words. A write to `ATTR0`
or `RENDER_MODE` updates a register; a `VERTEX_*` writes the vertex's
fields into the ring and, for a kick with three vertices, starts a triangle
with the kick's roles; `FRAME` waits as the end-of-frame and clear
commands do now. The setup engine and rasteriser don't change, except that
the rasteriser takes the triangle's `Z_TEST`, `Z_WRITE` and `COLOR_WRITE`
with it.

## 5. The serial link (Phase 13)

The board's FT230X is a USB UART: `usb_rx` into the FPGA, `usb_tx` out, and
its `DTR#` and `RTS#` outputs. Nothing runs from the FPGA to its `CTS#`, so
flow control goes over the data.

- **Line:** 3,000,000 baud, 8N1, the FT230X's fastest: 25 core clocks a bit.
  About 300 KB/s, so 33,000 transactions a second, or 18,500 vertices
  (two transactions each, when every vertex has its own `ATTR0`).
- **Bytes:** a write is 9 bytes, the 72 bits MSB first. A read is 1 byte,
  bit 7 set and the address below.
- **Link enable:** `DTR#`. Opening the port on the PC asserts DTR; closing it
  deasserts it. While DTR is deasserted the receiver is held in reset (no
  partial transaction survives) and the built-in scene plays (§6). When it
  is asserted the receiver starts at a transaction boundary, so DTR is the
  serial link's chip select.
- **Flow control by credits:** the FIFO holds 512 transactions. The host
  starts with 512 credits and spends one per transaction. The GPU sends
  `0x01` each time the parser has taken 64 more, and the host adds 64. The
  host never overruns the FIFO, so nothing is dropped. At 33,000
  transactions a second, 512 in flight covers 15 ms of USB latency, about
  the FT230X's default 16 ms latency timer.
- **Read replies:** `0x02` and the 8 data bytes, MSB first, in order with
  the credit bytes.
- **Synchronisers:** `usb_rx` and `DTR#` pass through two flip-flops in
  `board/top.sv`, outside the Sinter core.

## 6. The built-in scene as register writes

The scene ROM holds transactions instead of packet words: today's
commands become `RENDER_MODE`, `ATTR0`, `VERTEX_*` and `FRAME` writes,
generated by `models/setup` from the same scene description. The parser
takes them from the ROM while DTR is deasserted, and from the FIFO while it
is asserted. It switches only at a `FRAME` with `PRESENT`, so a frame is
never half one source's; switching back to the ROM restarts it from the
beginning, which starts with a clear.

Sinter values are at most 64 bits, so the ROM is two: 64-bit data and an
8-bit address. The cube is about 60 transactions a frame, about 2,000 for
its 32 frames: 8 DP16KD for the data and 1 for the addresses (est.),
against 16 now.

The reference renderer (`render.rs`) runs on the same transactions, so the
frame test checks the parser too.

## 7. The host side

- **`host/proto`**: `no_std` Rust: register and field definitions, encoding
  transactions, the serial framing and credit counting, decoding replies.
  Used by the PC tool, the twin tests, the scene generator, and the RP2350
  firmware in Phase 14.
- **`host/gs-host`**: a PC command-line tool. It opens the serial port,
  reads `ID`, streams a scene (the cube, turned live rather than from 32
  precomputed frames, so it is visibly not the ROM), and prints `STATUS`
  and the link's throughput. It is built in the devcontainer as a static
  Linux binary and run on the host, which has the USB port, as
  openFPGALoader is.

Both are members of the `sim/` workspace.

## 8. Verification

- **Twin, from reset with DTR deasserted:** today's frame test, unchanged in
  what it checks: the ROM scene's frames pixel-exact against the reference.
- **Twin, with a host:** a test drives `usb_rx` bit by bit at 25 cycles a
  bit with bytes from `host/proto`, honouring credits from `usb_tx`; checks
  `ID`, `STATUS`, frames pixel-exact against the reference renderer given
  the same transactions, and that the FIFO never overflows.
- **Link reset:** DTR deasserted in mid-transaction and mid-frame; the ROM
  scene resumes at a frame boundary.
- **Directed stimulus:** the UART receiver and transmitter, through
  `--cosim`, like the other blocks.
- **Recorded stimulus:** the frame tests record `usb_rx` and `DTR#` with
  `sdram_dq`, and `just check` replays them with `--cosim`.
- **Mutation:** break the credits, the kick roles and a write enable on
  purpose and see the tests fail.

## 9. Resources (estimates)

| | DP16KD |
|---|---|
| command FIFO, 512 × 72 bits | 2 to 3 |
| scene ROM, about 2,000 × 72 bits | 9 (16 now) |

The UART, parser registers and credit counter are small. The total should
fall a little from Phase 12's 26 DP16KD.
