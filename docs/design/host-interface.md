# Host interface

How a host drives the GPU: a register model, the transactions that read and
write it, and the links that carry them. This is the document a program
using pico-gs2 is written against: a game on Linux over the USB serial
port first, and on an RP2350 later.

The registers take pico-gs's (INT-010, INT-014 in that repo) as
inspiration, not as a specification: they are the ones this pipeline
honours, at pico-gs's addresses and bit positions where a register or
field means the same thing. ADR [0008](../adr/0008-register-host-interface.md)
records the register interface and ADR
[0009](../adr/0009-v1-interface.md) its v1.0 scope.

**Status:** interface version 2, the v1.0 draft (Phase 14), approved 2026-11-08.
Each register and field is marked *built* (in the design now) or with the
phase that builds it. Version 1 (Phase 13) is built and works on the board;
§14 lists what version 2 changes. The document is frozen as v1.0 at
Phase 19, with what the phases between actually built.

## 1. Phases

| Phase | Branch | Work | Gate |
|---|---|---|---|
| 13 | `phase-13-host-link` | The register model, the serial link over the board's USB UART, the built-in scene as register writes, `gs-proto` and `gs-host` | Done: twin tests; on the board, the ROM cube with no host, a streamed cube at 60.7 fps, the ROM back when the host goes |
| 14 | `phase-14-interface` | This document and ADR 0009: the v1.0 register map, texturing, the combiner, memory access, the SPI link's design | Approved |
| 15 | `phase-15-memory` | `MEM_ADDR` and `MEM_DATA`, write and read (§9); `STATUS` and `ID` version 2; `gs-host` uploads, reads back and saves the frame on screen | Twin tests; on the board, a frame read back over the link matches the twin's frame pixel for pixel |
| 16 | `phase-16-textures` | `TEX0_CFG` (§6); RGB565 and ARGB4444; the texture cache; perspective-correct nearest sampling; the wrap modes; texture × shade | Pixel-exact against the reference renderer; a textured cube on the board; frame time and cache hit rate measured |
| 17 | `phase-17-combiner` | `CC_MODE`, `CONST0`, `CONST1` (§7); alpha test (§8); `RENDER_MODE` version 2 | Pixel-exact; a scene with cut-out alpha and the combiner's presets on the board |
| 18 | `phase-18-bilinear` | Bilinear filtering, if it fits the ECP5-25K and 75 MHz; if not, `FILTER` = 1 samples nearest and the document says so | Pixel-exact; resources and timing |
| 19 | `phase-19-v1` | Resources, timing and performance figures; this document frozen as v1.0 | Approved |

After v1.0, in an order to be decided: the SPI link (§11) and RP2350
firmware; alpha blending (`ALPHA_BLEND`, `DST_COLOR`); `ATTR1`'s second
colour; meshes kept in SDRAM and drawn from there.

## 2. Layers

```
  game / gs-host                         RP2350 firmware (later)
        │                                        │
  gs-proto: registers, transactions, credits, replies  (no_std Rust)
        │                                        │
  serial link (§10): 3 Mbaud UART        SPI link (§11): later
        │                                        │
        └──────── command FIFO (512 transactions) ──┘
                         │
                 command parser → setup → rasteriser → SDRAM
```

Sections 3 to 9 are the same over either link. A link carries the same
bytes each way: transactions to the GPU, credits and read replies back
(§10). Only how the bytes travel and how the link is enabled differ, so
moving a program from the serial port to SPI changes only the code that
moves bytes.

## 3. Transactions

A transaction is 72 bits, as in pico-gs:

| Bits | Field |
|---|---|
| 71 | R/W̄: 1 read, 0 write |
| 70:64 | register address |
| 63:0 | data (a write's value; absent from a read) |

Transactions take effect in order. A write goes into the command FIFO and is
carried out when the parser reaches it. A read also goes through the FIFO,
so it is a fence: it is answered once everything before it has been taken
by the parser. A read of `MEM_DATA` (§9) also waits for drawing to finish,
so it sees every pixel drawn before it. Writes to unused addresses and
read-only registers are ignored; reads of unused addresses and write-only
registers return 0.

**Render state goes with each triangle.** A triangle is drawn with
`RENDER_MODE`, `TEX0_CFG`, `CC_MODE`, `CONST0` and `CONST1` as they were at
its kick, however long it waits in the pipeline. A write to `TEX0_CFG` or
`MEM_DATA` waits until the triangles before it are drawn (the texture
cache is emptied then), so group triangles by texture: each change costs
the pipeline's drain, about the time of one large triangle.

## 4. Registers

| Address | Name | Access | Contents | Status |
|---|---|---|---|---|
| 0x00 | `ATTR0` | W | colour and texture coordinates for the vertices that follow | built |
| 0x01 | `ATTR1` | — | reserved: a second colour (specular) | after v1.0 |
| 0x06 | `VERTEX_NOKICK` | W | a vertex; draws nothing | built |
| 0x07 | `VERTEX_KICK_012` | W | a vertex; draws (two back, one back, this one) | built |
| 0x08 | `VERTEX_KICK_021` | W | a vertex; draws (two back, this one, one back): the other winding | built |
| 0x09 | `VERTEX_KICK_FAN` | W | a vertex; draws (the primitive's first, one back, this one) | built |
| 0x10 | `TEX0_CFG` | R/W | the texture: enable, format, size, filter, wrap, address | 16 |
| 0x18 | `CC_MODE` | R/W | the colour combiner's inputs | 17 |
| 0x19 | `CONST0` | R/W | a constant colour for the combiner | 17 |
| 0x1B | `CONST1` | R/W | a second constant colour | 17 |
| 0x30 | `RENDER_MODE` | R/W | cull mode, depth test and writes, alpha test | built; version 2 in 17 |
| 0x40 | `FRAME` | W | present the frame, and clear | built |
| 0x70 | `MEM_ADDR` | R/W | the memory address `MEM_DATA` reads and writes | 15 |
| 0x71 | `MEM_DATA` | R/W | 64 bits of GPU memory, then the next | 15 |
| 0x7E | `STATUS` | R | frames shown, the last frame's cycles, FIFO space, the buffer on screen, link errors | built; version 2 in 15 |
| 0x7F | `ID` | R | identification and interface version | built |

pico-gs's `FB_CONFIG`, `FB_DISPLAY`, `MEM_FILL`, `PALETTEn`, `Z_RANGE` and
`PERF_TIMESTAMP` have no counterpart: the GPU owns its buffers (§9.2),
`FRAME` clears, and `STATUS` times frames.

## 5. Vertices and primitives (built)

### 0x00 `ATTR0`

| Bits | Field |
|---|---|
| 63:56 | R (UQ8.0) |
| 55:48 | G |
| 47:40 | B |
| 39:32 | A |
| 31:16 | S′ = S·Q, signed Q4.12 |
| 15:0 | T′ = T·Q, signed Q4.12 |

Latched: every vertex written takes the `ATTR0` last written. A batch of one
colour writes it once. Reset value 0 (black, transparent, S′ = T′ = 0).

R, G, B and A are interpolated across the triangle as the combiner's
`SHADE0` (§7). S, T are texture coordinates in units of the texture's size
(0 to 1 spans it once), so the same coordinates suit any texture size.
S′ and T′ are them multiplied by the vertex's Q; the GPU interpolates S′,
T′ and Q linearly on the screen and divides per pixel, which is
perspective-correct (§6.3). A vertex's S′ must stay within ±8: with Q near
1, a triangle can span up to 8 repeats of a texture; more needs smaller
triangles.

### 0x06–0x09 `VERTEX_*`

| Bits | Field |
|---|---|
| 63:48 | X, signed, 1/16 pixel (Q12.4) |
| 47:32 | Y, signed, 1/16 pixel (Q12.4) |
| 31:16 | Z, unsigned, 0 nearest, 0xFFFF farthest |
| 15:0 | Q ∝ 1/W, UQ1.15 (0 to just under 2) |

Each write adds a vertex: this position with the latched `ATTR0`. The
framebuffer is 640 × 360 pixels; (0, 0) is the top left corner of the top
left pixel, whose centre is (0.5, 0.5). Triangles may reach outside it
(X and Y from −2048 to 2047.9375) and are clipped to it; the host clips to
the near plane, so Q > 0 at every vertex of a textured triangle.

Q only needs to be proportional to 1/W within a triangle, so the host may
scale it per triangle or per draw; scaling the largest Q to about 1 keeps
the most precision. S′ and T′ are scaled with it, being S·Q and T·Q.

**Primitives.** A `VERTEX_NOKICK` that follows a kick (or is the first
vertex after reset or a link reset) starts a new primitive; its first
vertex is the one `VERTEX_KICK_FAN` pivots on. A kick draws only once the
primitive has three vertices; before that it just adds the vertex.

- Strip: `NOKICK, NOKICK, KICK_012, KICK_021, KICK_012, …` (the host
  alternates, which keeps one winding).
- Fan: `NOKICK, NOKICK, KICK_FAN, KICK_FAN, …`.
- Separate triangles: `NOKICK, NOKICK, KICK_012` each.

The GPU chooses the setup engine's vertex ring slots; the host never sees
them. A primitive's first vertex goes in slot 0. If its third vertex is a
`VERTEX_KICK_FAN` the primitive is a fan, slot 0 is kept, and later
vertices cycle through slots 1 to 3; otherwise it is a strip and later
vertices cycle through all four. So a fan's kicks should all be
`VERTEX_KICK_FAN`, and a strip's never: a `KICK_FAN` in a strip pivots on
whatever slot 0 then holds. A new primitive waits for the ring to be free,
since its first vertex reuses slot 0.

**Winding and culling.** A triangle is clockwise or anticlockwise as it
appears on the screen (Y down), in the order its kick names the vertices.
`RENDER_MODE.CULL` discards one winding (§8).

## 6. Texturing (Phase 16)

One texture unit, `TEX0`. A triangle is textured if `TEX0_CFG.ENABLE` was
set at its kick; then the setup engine runs its full program (99 cycles a
triangle, against 80 untextured) and the combiner's `TEX0` input is the
texture's colour at each pixel. Untextured, `TEX0` is opaque white, so the
reset combiner (texture × shade) draws the shade alone.

### 0x10 `TEX0_CFG`

| Bits | Field | Reset |
|---|---|---|
| 47:32 | `BASE_ADDR`: the texture's address in GPU memory, in units of 512 bytes (256 words) | 0 |
| 19:18 | `V_WRAP`: wrap mode for T (below) | 0 |
| 17:16 | `U_WRAP`: wrap mode for S | 0 |
| 15:12 | `HEIGHT_LOG2`: height is 2^n texels, n from 2 to 10 | 0 |
| 11:8 | `WIDTH_LOG2`: width is 2^n texels, n from 2 to 10 | 0 |
| 7:4 | `FORMAT`: 0 RGB565, 1 ARGB4444; others reserved | 0 |
| 3:2 | `FILTER`: 0 nearest, 1 bilinear (Phase 18; nearest until then); others reserved | 0 |
| 0 | `ENABLE`: texture the triangles that follow | 0 |

Wrap modes, as pico-gs: 0 `REPEAT`, 1 `CLAMP_TO_EDGE`, 2 `MIRROR` (every
other repeat reflected), 3 reserved (as `REPEAT`). The other bits are
reserved: write 0, read 0. Field positions are pico-gs's. A `WIDTH_LOG2`
or `HEIGHT_LOG2` below 2 is taken as 2, above 10 as 10.

A write waits for the triangles before it to be drawn, then empties the
texture cache, so it also makes a texture just uploaded to the same
address visible. Upload with `MEM_DATA`, which does the same.

### 6.1 Texel formats

A texel is one 16-bit word:

| Format | Bits 15 to 0 | To 8 bits per channel |
|---|---|---|
| 0 `RGB565` | R[15:11] G[10:5] B[4:0] | each channel's bits repeated to fill 8 (R 0x1F → 0xFF); A = 255 |
| 1 `ARGB4444` | A[15:12] R[11:8] G[7:4] B[3:0] | each × 17 (0xF → 0xFF) |

The cache holds texels as stored and widens them as they leave it, so both
formats cost the same cache space and memory bandwidth.

### 6.2 Layout in memory

A texture is tiled in 4 × 4 blocks, as in pico-gs (INT-014): each block is
16 consecutive words holding its texels in raster order, and blocks follow
in raster order across the texture. Texel (u, v), counted from the top left
texel, is at word

```
BASE_ADDR × 256 + ((v / 4) × (width / 4) + u / 4) × 16 + (v mod 4) × 4 + u mod 4
```

so a block is one 16-word SDRAM run, the cache's line. A 256 × 256 texture
is 128 KB, 1024 × 1024 is 2 MB. `gs-proto` tiles an image for upload.

### 6.3 Sampling

At each pixel's centre the GPU has S′, T′ and Q, each interpolated linearly
from the vertices, and computes

```
s = S′ / Q,   t = T′ / Q            (the texture coordinates, perspective-correct)
U = s × width,   V = t × height     (in texels, with 4 fraction bits)
```

then wraps U and V: `REPEAT` takes them modulo the size, `CLAMP_TO_EDGE`
clamps them to the texture, `MIRROR` reflects every other repeat.

- **Nearest:** the texel ⌊U⌋, ⌊V⌋ after wrapping.
- **Bilinear (Phase 18):** the four texels around (U − ½, V − ½), each wrapped,
  weighted by the fraction bits.

The divide is a reciprocal of Q (a table and a Newton–Raphson step) and two
multiplies. Its exact rounding is the reference renderer's
(`models/setup/src/render.rs`), which the hardware matches bit for bit.
Phase 16 measures its error against an exact divide, by how far a
triangle's farthest vertex is against its nearest, and records it here.

There is no mipmapping: a texture seen from far away shimmers; smaller
textures for far objects are the host's choice.

## 7. The colour combiner (Phase 17)

After texturing, each pixel's colour comes from one combiner pass, as the
N64's one-cycle mode, for R, G and B and separately for A:

```
result = (A − B) × C + D
```

with A, B, C and D each chosen from the inputs below, every channel 8 bits
(0 to 255 standing for 0 to 1).

### 0x18 `CC_MODE`

| Bits | Field | Sources |
|---|---|---|
| 63:32 | pass 1: write 0 | 0 passes pass 0's result through (after v1.0, a second pass) |
| 31:28 | `ALPHA_D` | as `RGB_A` |
| 27:24 | `ALPHA_C` | as `RGB_A` |
| 23:20 | `ALPHA_B` | as `RGB_A` |
| 19:16 | `ALPHA_A` | as `RGB_A` |
| 15:12 | `RGB_D` | as `RGB_A` |
| 11:8 | `RGB_C` | 1 `TEX0`, 3 `SHADE0`, 4 `CONST0`, 5 `CONST1`, 6 `ONE`, 7 `ZERO`, 8 `TEX0_ALPHA`, 10 `SHADE0_ALPHA`, 11 `CONST0_ALPHA` |
| 7:4 | `RGB_B` | as `RGB_A` |
| 3:0 | `RGB_A` | 1 `TEX0`, 3 `SHADE0`, 4 `CONST0`, 5 `CONST1`, 6 `ONE`, 7 `ZERO` |

The encodings are pico-gs's. The codes not listed read as 0; pico-gs uses
them for its second pass's `COMBINED` (0), a second texture (2), `SHADE1`
(8 in A, B, D; 13 and 14 in `RGB_C`, from `ATTR1`) and `DST_COLOR` (9,
for blending), which may follow v1.0 at the same codes. The `_ALPHA`
sources broadcast that input's alpha to R, G and B.

Reset value `0x7371_7371`: `(TEX0 − ZERO) × SHADE0 + ZERO` for colour and
alpha, the texture modulated by the shade. Other common settings:

| Effect | RGB: A, B, C, D | Alpha: A, B, C, D | `CC_MODE` |
|---|---|---|---|
| modulate (reset) | `TEX0`, `ZERO`, `SHADE0`, `ZERO` | the same | `0x7371_7371` |
| decal: texture only | `ZERO`, `ZERO`, `ZERO`, `TEX0` | the same | `0x1777_1777` |
| shade only | `ZERO`, `ZERO`, `ZERO`, `SHADE0` | the same | `0x3777_3777` |
| tinted: texture × constant | `TEX0`, `ZERO`, `CONST0`, `ZERO` | the same | `0x7471_7471` |
| texture + shade (cheap specular) | `TEX0`, `ZERO`, `ONE`, `SHADE0` | `TEX0`, `ZERO`, `SHADE0`, `ZERO` | `0x7371_3671` |
| fade to `CONST1` by shade alpha | `TEX0`, `CONST1`, `SHADE0_ALPHA`, `CONST1` | `TEX0`, `ZERO`, `ONE`, `ZERO` | `0x7671_5A51` |

**Arithmetic.** For each channel, with A, B, C and D from 0 to 255:

```
c′ = C + (C >> 7)                      (0 to 256, so ONE is exactly 1)
result = clamp(D + (((A − B) × c′ + 128) >> 8), 0, 255)     (>> rounds toward −∞)
```

The colour is then written as RGB565 by keeping each channel's top bits
(R 8 → 5, G 8 → 6, B 8 → 5); a texel read and written unchanged keeps its
value.

### 0x19 `CONST0`, 0x1B `CONST1`

| Bits | Field |
|---|---|
| 31:24 | R |
| 23:16 | G |
| 15:8 | B |
| 7:0 | A |

The same order as `ATTR0`'s colour. Reset value `0xFFFF_FFFF` (opaque
white). (pico-gs calls them `MAT_COLOR0` and `FOG_COLOR`; there is no fog
factor here, so `CONST1` is just a second constant.)

## 8. Render state

### 0x30 `RENDER_MODE`

| Bits | Field | Reset | Status |
|---|---|---|---|
| 26:19 | `ALPHA_REF`: the alpha test's reference, 0 to 255 | 0 | 17 |
| 18:17 | `ALPHA_TEST`: 0 always pass, 1 pass if alpha < `ALPHA_REF`, 2 pass if alpha ≥ `ALPHA_REF`, 3 pass if alpha ≠ `ALPHA_REF` | 0 | 17 |
| 15:13 | `Z_COMPARE`: 0 less than; others reserved (pico-gs: 1 ≤, 6 always, …) | 0 | after v1.0 |
| 9:7 | `ALPHA_BLEND`: 0 off; others reserved (pico-gs: 1 add, 3 blend) | 0 | after v1.0 |
| 6:5 | `CULL`: 0 none, 1 discard anticlockwise (on screen), 2 discard clockwise, 3 none | 0 | built |
| 4 | `COLOR_WRITE`: write the colour buffer | 1 | built |
| 3 | `Z_WRITE`: write the depth buffer where a pixel is drawn | 1 | built |
| 2 | `Z_TEST`: draw a pixel only if its depth is less than the buffer's | 1 | built |

Reset value `0x1C`. Field positions are pico-gs's. Version 1's
`TEXTURED` (bit 8) goes: it sat inside pico-gs's `ALPHA_BLEND`, and
`TEX0_CFG.ENABLE` replaces it (§14).

**A pixel is drawn** if the triangle covers it, the alpha test passes on the
combiner's alpha, and the depth test passes (or `Z_TEST` is off). A drawn
pixel writes its colour if `COLOR_WRITE` and its depth if `Z_WRITE`. A pixel
that fails the alpha test writes nothing, depth included, so cut-out
textures (`ALPHA_TEST` = 2, `ALPHA_REF` = 128, ARGB4444 with alpha 0 or 15)
hide what is behind them only where they are opaque, in any drawing order.

### 0x40 `FRAME` (built)

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
clear mid-frame). Clearing colour and depth takes about 6.7 ms.

There are three colour buffers (ADR 0003): one shown, one drawn, the third
finished and waiting or spare. A `PRESENT` waits, holding up the commands
behind it, while a finished frame is already waiting; the host sees this
as the FIFO not draining. So the host never runs more than a frame ahead of
the display, and a frame is never torn.

### 0x7E `STATUS`

| Bits | Field | Status |
|---|---|---|
| 53:52 | link errors, sticky until the link resets: bit 52 a transaction arrived with the FIFO full, bit 53 a malformed byte | 15 |
| 51:50 | the colour buffer on screen, 0 to 2 (§9.2) | 15 |
| 49:40 | free FIFO entries | built |
| 39:16 | cycles the last finished frame took to draw, at 75 MHz | built |
| 15:0 | frames shown since reset (wraps) | built |

### 0x7F `ID`

| Bits | Field |
|---|---|
| 63:32 | `0x50475332`, "PGS2" |
| 31:16 | interface version: 1 now; 2 from Phase 15 |
| 15:0 | 0 |

## 9. GPU memory (Phase 15)

### 0x70 `MEM_ADDR`, 0x71 `MEM_DATA`

`MEM_ADDR` bits 21:0 are an address in 64-bit units ("dwords") of the
32 MiB of SDRAM; the other bits are reserved. A write of `MEM_DATA` stores
its 64 bits at `MEM_ADDR`; a read of `MEM_DATA` returns the 64 bits there.
Either then adds 1 to `MEM_ADDR`, so a run of them moves a block of memory.
A dword holds four 16-bit words, the lowest address in bits 15:0.

Both wait until the triangles before them are drawn, and a write empties the
texture cache, so reads see every pixel drawn before them and textures see
every upload before them. As in pico-gs, but without its prefetch: a read
here is answered after the memory is read, which the FIFO ordering
already allows for.

Throughput over the serial link: four texels per 9-byte write, so a
64 × 64 texture (8 KB) uploads in about 30 ms and a 256 × 256 one (128 KB)
in about 0.5 s. Reads come back at about the same rate.

### 9.2 The memory map

The SDRAM is 4 banks of 8 MB. A word address is bank (bits 23:22), row
(21:9) and column (8:0); a dword address is the word address ÷ 4, so its
bank is bits 21:20.

| Dwords | Bank | Holds |
|---|---|---|
| 0x000000 – 0x00FFFF | 0 | colour buffer 0 (the GPU's) |
| 0x010000 – 0x0FFFFF | 0 | the host's: 7.5 MB |
| 0x100000 – 0x10FFFF | 1 | colour buffer 1 (the GPU's) |
| 0x110000 – 0x1FFFFF | 1 | the host's: 7.5 MB |
| 0x200000 – 0x20FFFF | 2 | colour buffer 2 (the GPU's) |
| 0x210000 – 0x2FFFFF | 2 | the host's: 7.5 MB |
| 0x300000 – 0x30FFFF | 3 | the depth buffer (the GPU's) |
| 0x310000 – 0x3FFFFF | 3 | the host's: 7.5 MB |

Textures go in the host's 30 MB, anywhere `BASE_ADDR` can point (512-byte
alignment). The GPU's regions can be read at any time; writing them while
the GPU draws or shows them gives an undefined picture, not a fault.

**Reading a frame back:** `STATUS` bits 51:50 name the buffer on screen,
which stays there until the next `PRESENT` is shown. Each buffer is
640 × 360 RGB565, tiled in 8 × 8 blocks: pixel (x, y) is word

```
y[8:3] × 5120 + x[9:3] × 64 + y[2:0] × 8 + x[2:0]
```

of its buffer, which starts at dword `buffer × 0x100000`. `gs-proto` untiles
it. The 230,400 pixels are 57,600 reads, about 2 s over the serial link.

## 10. The serial link (built)

The board's USB UART is an FT231X (the v1.3 PCB; its pin file calls it an
FTDI): its TXD reaches the FPGA as `usb_rx` (ball K16), the FPGA drives its
RXD as `usb_tx` (K15), and its `DTR#` reaches the FPGA (L15). Its `CTS#`,
`DSR#`, `DCD#` and `RI#` are the FPGA's JTAG lines (the board is programmed
through this chip), so there is no hardware flow control from the FPGA,
and flow control goes over the data.

- **Line:** 3,000,000 baud, 8N1, the FT231X's fastest: 25 core clocks a bit.
  About 300 KB/s: 33,000 transactions a second, about 550 a frame at
  60 fps. A vertex is one transaction, two when it has its own `ATTR0`, so
  a strip costs about two transactions a triangle and a frame can carry
  about 250 triangles at 60 fps, 500 at 30.
- **Host to GPU:** a write is 9 bytes, the 72 bits MSB first. A read is 1
  byte, bit 7 set and the address below.
- **GPU to host:** a credit is `0x01`; a read's reply is `0x02` and the 8
  data bytes, MSB first. Credits may come before or after a reply, never
  inside it. One reply waits at a time; a read behind it waits.
- **Link enable:** `DTR#`. Opening the port on the PC asserts DTR; closing
  it deasserts it. While DTR is deasserted the receiver is held in reset
  (no partial transaction survives) and the built-in scene plays (§12).
  When it is asserted the receiver starts at a transaction boundary, so DTR
  is the serial link's chip select.
- **Flow control by credits:** the FIFO holds 512 transactions. The host
  starts with 512 credits and spends one per transaction. The GPU sends a
  credit each time the parser has taken 64 more, and the host adds 64. The
  host never overruns the FIFO, so nothing is dropped. At 33,000
  transactions a second, 512 in flight covers 15 ms of USB latency, about
  the chip's default 16 ms latency timer (`gs-host` sets it to 1 ms if it
  can).
- **Synchronisers:** `usb_rx` and `DTR#` pass through two flip-flops in
  `board/top.sv`, outside the Sinter core.

## 11. The SPI link (after v1.0; design only)

For a microcontroller such as the RP2350 on the GPIO header. The bytes are
the serial link's; SPI only carries them faster.

- **Signals:** `SCK`, `MOSI`, `MISO` and `CS#` from the host, SPI mode 0,
  MSB first; and `ATTN`, high while the GPU has bytes to send (credits or a
  reply). Pins on the GPIO header, to be chosen with the board's pin file.
- **Clock:** up to 15 MHz (est.), sampled by the 75 MHz core clock at five
  samples a bit, so the link needs no clock domain of its own (Sinter's
  `async_fifo` reaches only about 85 MHz). That is about 1.9 MB/s, six
  times the serial link: about 1,500 triangles a frame at 60 fps.
- **Frames:** `CS#` low starts a frame and high ends it, and a frame
  carries whole transactions. Its first byte says what follows: `0x00`,
  transactions; `0x01`, nothing (a poll). A transaction cut short by `CS#`
  rising is dropped, as by DTR on the serial link.
- **MISO:** throughout a frame, the GPU shifts out the bytes it owes the
  host, the serial link's credits and replies, and `0x00` when it owes
  nothing (no message starts with `0x00`). A host with nothing to send
  polls when `ATTN` is high.
- **Credits** work as on the serial link; a host may instead stop when
  `STATUS` shows the FIFO nearly full, but credits cost it no reads.
- **Link enable:** the link is connected from its first frame. The built-in
  scene hands over as for DTR (§12); a host that goes sends a frame with
  first byte `0x02` to hand back.

`gs-proto` gets a `Link` trait (send bytes, receive bytes) with the serial
port and SPI as two implementations; everything above it is shared.

## 12. The built-in scene (built)

The scene ROM holds transactions: the cube's `RENDER_MODE`, `ATTR0`,
`VERTEX_*` and `FRAME` writes, generated by `models/setup` from the same
scene description as the reference renderer's. The parser takes them from
the ROM while no host is connected, and from the FIFO while one is. It
switches only at a `FRAME` with `PRESENT`, so a frame is never half one
source's; switching back to the ROM restarts it from the beginning, which
starts with a clear.

Sinter values are at most 64 bits, so the ROM is two of 2,048 entries:
64-bit data and an 8-bit address byte, where 0xFF (which would be a read of
`ID`) marks the end. The cube is 1,825 transactions, 57 a frame.

A host connecting inherits whatever state the ROM left (`RENDER_MODE`,
and from Phase 16 the texture and combiner registers); it should write the
state it relies on.

## 13. The host side

- **`host/proto` (`gs-proto`)**: `no_std` Rust: register and field
  definitions, encoding transactions, the link framing and credit counting,
  decoding replies; from Phase 15, tiling textures and untiling frames.
  Used by the PC tool, the twin tests, the scene generator, and later the
  RP2350 firmware. A game project depends on it.
- **`host/gs-host`**: a PC command-line tool. It opens the serial port,
  reads `ID`, streams a live cube, and prints `STATUS` and the link's
  throughput; `--status` reads `ID` and `STATUS` only. From Phase 15,
  `--screenshot <file>` saves the frame on screen as a PNG.
  `just gs-host` builds it in the devcontainer (it needs only the C
  library) to run on the host, which has the USB port:
  `sim/target/release/gs-host --port /dev/ttyUSB0`.

## 14. Changes from interface version 1

Version 1 is Phase 13, as built. Version 2:

- `RENDER_MODE.TEXTURED` (bit 8) goes; `TEX0_CFG.ENABLE` replaces it.
- `RENDER_MODE` gains `ALPHA_TEST` and `ALPHA_REF`, and reserves
  `ALPHA_BLEND` and `Z_COMPARE` at pico-gs's positions.
- New: `TEX0_CFG`, `CC_MODE`, `CONST0`, `CONST1`, `MEM_ADDR`, `MEM_DATA`.
- `STATUS` gains the buffer on screen and the link's error flags.
- S′ and T′ are signed Q4.12 and Q is UQ1.15 (version 1 left them
  provisional and unused).
- Colour reaches RGB565 by keeping each 8-bit channel's top bits, after the
  combiner; version 1 rounded the interpolated colour straight to 5/6/5
  bits, so untextured triangles can differ by one step in a channel.
- `ID` reads interface version 2.

## 15. Verification

As built in Phase 13 (`sim/tb/tests/`):

- **`frame.rs`, from reset with no host:** the ROM scene's frames pixel-exact
  against the reference renderer on the same transactions.
- **`host.rs`, with a host:** the ROM scene; then a host (`tb::Host`, at 25
  cycles a bit, honouring credits) connects, reads `ID`, `STATUS` and
  `RENDER_MODE`, and streams two frames: one with the depth test off, a
  depth-only clear, a triangle that writes only depth and one that writes
  no depth; then a cube frame. Every frame shown is whole and pixel-exact,
  ROM then host; credits come back for every 64 transactions; the FIFO
  drains and never overflows. The host disconnects and the ROM scene
  restarts at its first frame.
- **`host_link.rs`, the link alone:** transactions in order; DTR# high in
  mid-transaction drops it and empties the FIFO; credits and whole replies;
  the overflow and framing flags.
- **Recorded stimulus:** `frame.rs` and `host.rs` record `sdram_dq`,
  `usb_rx` and `usb_dtr_n` every cycle, and `just check` replays both with
  `--cosim`, so the link's Verilog runs the whole host test.
- **Mutation:** ignoring `Z_TEST`, `COLOR_WRITE` or `Z_WRITE`; clearing
  colour on a depth-only clear; credits twice as often; `KICK_021` with the
  old odd-strip roles; a fan pivoting on two back: each fails a test.

Planned, phase by phase: the reference renderer gains each feature first,
and the twin tests compare against it pixel for pixel, as now.

- **15:** a host writes memory and reads it back; reads the frame on screen
  and gets the reference frame; `MEM_DATA` after a triangle sees it drawn.
  On the board, `gs-host --screenshot` of the ROM cube matches the twin's
  frame.
- **16:** textured triangles in both formats, every wrap mode, textures
  smaller and larger than the cache, a texture uploaded between two frames
  (the cache emptied), steep perspective; the setup engine's S′, T′ and Q
  gradients against the interpreter.
- **17:** each combiner preset in §7 and every source; each alpha test; a
  cut-out in front of and behind other triangles.
- **18:** bilinear against the reference, including at wrap edges.

## 16. Resources and timing

As built (Phase 13): the GPU places and routes at 76 to 78 MHz (75 needed;
it varies with placement from build to build) in 10,404 logic cells, 3,948
flip-flops, 21 DP16KD and 2 MULT18X18D, of 24,288, 24,288, 56 and 28. The
command FIFO takes 2 DP16KD and the scene ROM 11.

The setup engine brief's budget (an estimate, ±50%) for what v1.0 adds:

| Block | LUT4 | Multipliers | DP16KD |
|---|---|---|---|
| Perspective divide | 400 | 3 | 1 |
| Texture cache and filter | 2,500 | 6 | 12 |
| Combiner and alpha test | 1,500 | 6 | 0 |
| **v1.0 total, with today's design** | **about 15,000 (60%)** | **about 17 (60%)** | **about 34 (60%)** |

The cache would hold 4 to 8 DP16KD of texels (8 to 16 KB), banked so that
bilinear filtering reads four texels a cycle. Each phase reports the real
figures; the frame time of a textured scene, with its cache misses, is
Phase 16's to measure.

## 17. On the board (Phase 13)

`gs-host` read `ID` (0x5047533200010000) and `STATUS`, then streamed the
live cube for 10 s: 615 frames sent and 615 shown, 60.7 a second (the
display's rate: a `PRESENT` waits while a finished frame is waiting), at
3,520 transactions and 31,700 bytes a second, 11% of the line. Each frame
is about 57 transactions, so the link could carry about 9 times as many
vertices before it limits the frame rate. `STATUS` read a frame drawn in
1,127,178 cycles (66.5 frames a second), as the twin predicts.

(The first board run read about 822,000 cycles: the counter skipped every
cycle the parser waited at a present, including cycles in which the
rasteriser was still drawing. It now skips only waits with the pipeline
idle.)
