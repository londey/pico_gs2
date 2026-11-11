# pico_gs2

A PS1/N64-era 3D graphics processor for the ICEpi Zero v1.3 (Lattice
ECP5-25K, 32 MB SDRAM, HDMI out), a from-scratch rewrite of
[pico-gs](https://github.com/londey/pico-gs). It is written in
[Sinter](https://github.com/sinter-lang/sinter-lang), which compiles one
source to SystemVerilog for the FPGA and to a cycle-accurate Rust
simulator: the simulator is the design's digital twin, and every test runs
against it.

**Status: v1.1**, released with its bitstream, the host interface's Rust
crate and the API docs on the
[releases page](https://github.com/londey/pico_gs2/releases). A host drives
it over the board's USB serial port today; an RP2350 over SPI is designed
but not built.

## What it does

- **Triangles**: strips and fans, Gouraud shaded, perspective
  correct, depth tested against a 16-bit depth buffer, either winding
  optionally culled.
- **Textures** in SDRAM: RGB565 or ARGB4444, up to 1024 × 1024, nearest or
  bilinear, repeated, clamped or mirrored, through a texture cache.
- **A colour combiner**, (A − B) × C + D over the texture, the shade and two
  constants as the N64's one-cycle mode, and an **alpha test**.
- **A 640 × 360 RGB565 framebuffer**, triple buffered, shown as 720p60 over
  HDMI (each pixel 2 × 2).
- **A register interface** (pico-gs's registers, where they mean the same
  thing) over a 3 Mbaud serial link with credit flow control; GPU memory
  read and written over the same link, for texture uploads and screenshots.
- **A built-in scene**, a spinning cube, while no host is connected.

At 75 MHz, a clear takes 0.05 ms, a screen covered by a bilinear texture
9.1 ms, and a small triangle about 260 cycles (3.5 µs). A 60 fps frame has
room for about 1.8 screens of drawing; with small triangles the serial link
runs out first, at about 500 untextured triangles a frame in strips.
[host-interface.md](docs/design/host-interface.md) §15 has the figures and
what follows for a program.

## How it works

```
 USB UART ─► host link ─► command FIFO ─► parser ─► setup engine ─► rasteriser ─┐
 (3 Mbaud)   (credits,    (512 trans-     (registers,  (microcoded,  (8×8 blocks:  │
              replies)     actions)        scene ROM)   99 cycles a   front half,   │
                                                        triangle)     back half,    │
                                                                      writer)       │
                                    SDRAM controller ◄──────────────────────────────┘
                                          │  ▲ texels
                          scan-out ◄──────┘
                              │
                      TMDS encoder ─► board/top.sv: serialisers ─► HDMI
```

- **The setup engine** turns three vertices into edge functions and
  attribute gradients with a small microcoded datapath
  ([setup-engine.md](docs/design/setup-engine.md), ADR 0001).
- **The rasteriser** works on the framebuffer's 8 × 8 tiles in three
  stages, each on a different block at once: the front half reads the
  block's depths and steps its pixels, the back half samples, filters,
  combines and alpha tests, and the writer writes the colours and depths
  back. A clear only marks blocks cleared, so their old contents are never
  read ([draw-speed.md](docs/design/draw-speed.md)).
- **The SDRAM** (W9825G6KH) is shared by scan-out, which comes first, and
  the rasteriser's depth reads, texel fetches and writes, in runs of 32
  words.
- **`board/top.sv`**, hand-written, holds what Sinter can't express: the
  PLL, reset, the DDR output primitives for HDMI and the SDRAM clock, and
  the SDRAM's bidirectional data pins.

The GPU runs at 75 MHz, the 720p pixel clock, and places and routes at
85.2 MHz in 78% of the ECP5-25K's logic, 68% of its block RAM and 43% of
its multipliers. The [architecture slides](docs/slides/architecture.typ)
(`just slides`) draw the pipeline, what overlaps and where it can stall.

## Using it from a program

[docs/design/host-interface.md](docs/design/host-interface.md) is the
document a program is written against: the transactions, the registers,
the memory map, the serial link and the performance figures.
[`host/proto`](host/proto) (`gs-proto`) is the same interface as a
`no_std`, dependency-free Rust crate: registers and fields, encoding
transactions, framing, credits, replies, tiling textures. Each release
has it as a standalone crate and its API docs.

A cargo git dependency on this repository doesn't work: cargo checks out
every submodule, and the board repository's nested submodules have no
URLs. Unpack the release's `gs-proto-<version>.tar.gz` into the program's
repository, or add this repository as a submodule (without
`--recursive`) and depend on the crate by path:

```toml
gs-proto = { path = "pico_gs2/host/proto" }
```

[`host/gs-host`](host/gs-host) is a working example: it uploads a texture,
streams a cube and reads the frame back.

## Development environment

Open the repository in VS Code and choose **Reopen in Container**. The
devcontainer (`.devcontainer/`) provides sinter, the OSS CAD Suite (Yosys,
nextpnr-ecp5, Verilator, openFPGALoader), Rust stable, typst and just, and
runs `git submodule update --init` on creation.

## Building

```sh
just check       # generate, build, lint and run every test (simulator and Verilator)
just perf        # draw times of a set of scenes, on the twin
just bitstream   # build/fpga/top.bit, failing if timing isn't met
just slides      # build/slides/architecture.pdf
just gs-host     # the PC tool, sim/target/release/gs-host
just ci          # everything CI runs
just release     # gather a release's files into build/release/<tag>/
```

`just` on its own lists every recipe. Generated files (Verilog, simulator
crates, bitstreams) go in `build/` and are never committed.

CI runs `just ci` on every push and pull request. Pushing a `v*` tag also
publishes a GitHub release with the bitstream, `gs-proto`, the docs and
`gs-host`, its notes from `docs/releases/<tag>.md`.

## On the board

The devcontainer has no USB access, so flash from the host:
`openFPGALoader -b icepi-zero build/fpga/top.bit` (the workspace is shared,
so the bitstream is there too), or a release's `pico_gs2-<tag>.bit`. This
loads the FPGA's configuration RAM, which is lost at power off; add
`--write-flash` to keep it.

At power-up the GPU fills the framebuffer with a test pattern and shows it:
red rising left to right, green rising top to bottom, a blue checkerboard
of 32-pixel squares and a one-pixel white border. Then it draws its
built-in scene (register writes in ROM, as a host would send them): a
spinning Gouraud-shaded cube and two triangles crossing in depth, over a
dark grey background. The cube turns a quarter turn every 32 frames. The
LEDs show:

| LED | Lit when |
|---|---|
| `led[0]` | blinks: the heartbeat, about 0.56 Hz |
| `led[1]` | the SDRAM has been initialised |
| `led[2]` | the framebuffer has been filled |
| `led[3]` | part of the picture wasn't loaded from SDRAM in time (it never should be) |
| `led[4]` | toggles at every frame shown |

### Driving it from a PC

The board's USB port is also the 3 Mbaud serial link to the GPU. With the
bitstream loaded, build `gs-host` in the devcontainer with `just gs-host`
(or take it from a release), then on the host:

```
gs-host --port /dev/ttyUSB0                                # stream a live textured cube for 10 s
gs-host --port /dev/ttyUSB0 --status                       # read ID, round-trip SCRATCH, read STATUS
gs-host --port /dev/ttyUSB0 --screenshot cube.png          # stream, then save the last frame
gs-host --port /dev/ttyUSB0 --status --screenshot rom.png  # save the built-in scene's frame
gs-host --port /dev/ttyUSB0 --combiner                     # the colour combiner and alpha test
```

While the port is open the GPU draws what the host sends: the tool uploads
a brick texture and its textured cube, filtered bilinear, turns all the
way round, over a dark blue background (`--nearest` samples the texture
nearest, `--untextured` draws the plain cube). When it closes, the GPU
goes back to its built-in scene. Every run first writes the `SCRATCH`
register with a few patterns and reads each back, which tests the link
both ways. The tool reports the frames shown, how long a frame took to
draw and the link's throughput. `--screenshot` reads the frame on screen
back over the link (about 2 s), saves it as a 640 × 360 PNG, and says
whether it matches, pixel for pixel, what the reference renderer draws.

## Layout

| Path | Holds |
|---|---|
| `hw/` | the design, in Sinter: `gpu.snt` is the root |
| `board/` | the hand-written SystemVerilog top level, constraints and its testbenches |
| `sim/` | the Rust workspace: integration tests driving the twin, `just perf` |
| `host/` | `gs-proto` (the interface crate) and `gs-host` (the PC tool) |
| `models/` | Rust models: the SDRAM chip, the setup engine and its microcode generator, the scenes and the reference renderer |
| `docs/` | design notes, decision records (`adr/`), slides, release notes |
| `scripts/`, `justfile` | every recipe, one script each |

## Documentation

- [docs/design/host-interface.md](docs/design/host-interface.md): the host
  interface, v1.0 (interface version 2), with v1.1's figures.
- [docs/design/triangle-pipeline.md](docs/design/triangle-pipeline.md):
  the rendering plan and how the pipeline was built up, phase by phase.
- [docs/design/draw-speed.md](docs/design/draw-speed.md): v1.1, faster
  drawing at the same interface.
- [docs/design/setup-engine.md](docs/design/setup-engine.md): the setup
  engine's numerics.
- [docs/adr/](docs/adr/): the decisions, and why.
- [docs/setup.md](docs/setup.md): the toolchain and board bring-up.
- [docs/sinter-requirements.md](docs/sinter-requirements.md): Sinter gaps
  the design works around.

## What's next

Not yet built, in an order to be decided: the SPI link and RP2350
firmware, alpha blending, the second vertex colour (`ATTR1`), and meshes
kept in SDRAM. Drawing is now limited by the SDRAM's bandwidth; the ideas
for more speed are in [draw-speed.md](docs/design/draw-speed.md).
