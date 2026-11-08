# pico_gs2

A from-scratch rewrite of [pico-gs](https://github.com/londey/pico-gs), a
PS1/N64-era 3D graphics processor for the ICEpi Zero (ECP5-25K), written in
[Sinter](https://github.com/sinter-lang/sinter-lang).

## Development environment

Open the repository in VS Code and choose **Reopen in Container**. The
devcontainer (`.devcontainer/`) provides sinter, the OSS CAD Suite (Yosys,
nextpnr-ecp5, Verilator, openFPGALoader), Rust stable and just.

Clone with submodules (`git clone --recurse-submodules` is not needed; the
container runs `git submodule update --init` on creation).

The setup plan is in [docs/setup.md](docs/setup.md).

## Building

```sh
just check       # generate, build and run every test
just bitstream   # build/fpga/top.bit, failing if timing isn't met
just flash       # load it onto a connected ICEpi Zero
just ci          # everything CI runs
```

`just` on its own lists every recipe. `just slides` builds the architecture
slides (`docs/slides/architecture.typ`) into `build/slides/architecture.pdf`;
CI uploads them as an artifact too.

The devcontainer has no USB access, so flash from the host instead:
`openFPGALoader -b icepi-zero build/fpga/top.bit` (the workspace is shared,
so the bitstream is there too). This loads the FPGA's configuration RAM,
which is lost at power off; add `--write-flash` to keep it.

## On the board

The design fills a 640x360 RGB565 framebuffer in the SDRAM, stored as
tiled 8x8 blocks, with a test pattern and shows it, then draws a built-in
scene (register writes in ROM, as a host would send them), a spinning Gouraud-shaded cube and two triangles crossing in depth
over a dark grey background, depth tested against a 16-bit depth buffer
and triple buffered: a frame drawn is shown from the next frame start. The
picture goes out of the GPDI (mini HDMI) port as 1280x720 at 60 Hz (720p),
each framebuffer pixel shown as 2x2 screen pixels. The cube turns a quarter
turn every 32 frames. The test pattern: red rising left to right, green
rising top to bottom, a blue checkerboard of 32-pixel squares and a
one-pixel white border. The LEDs show:

| LED | Lit when |
|---|---|
| `led[0]` | blinks: the heartbeat, about 0.56 Hz |
| `led[1]` | the SDRAM has been initialised |
| `led[2]` | the framebuffer has been filled |
| `led[3]` | part of the picture wasn't loaded from SDRAM in time (it never should be) |
| `led[4]` | toggles at every frame shown (so glows at 30 Hz) |

### Driving it from a PC

The board's USB port is also a 3 Mbaud serial link to the GPU
([docs/design/host-interface.md](docs/design/host-interface.md)): a host
writes and reads registers with it. With the board plugged in and the
bitstream loaded, build the PC tool in the devcontainer with
`just gs-host`, then on the host:

```
sim/target/release/gs-host --port /dev/ttyUSB0             # stream a live cube for 10 s
sim/target/release/gs-host --port /dev/ttyUSB0 --status    # read ID and STATUS only
sim/target/release/gs-host --port /dev/ttyUSB0 --screenshot cube.png         # stream, then save the last frame
sim/target/release/gs-host --port /dev/ttyUSB0 --status --screenshot rom.png  # save the built-in scene's frame
sim/target/release/gs-host --port /dev/ttyUSB0 --combiner                     # the colour combiner and alpha test
```

While the port is open the GPU draws what the host sends: the tool uploads
a brick texture and its textured cube, filtered bilinear, turns all the
way round, over a dark blue background (`--nearest` samples the texture
nearest, `--untextured` draws the plain cube). When it closes, the
GPU goes back to its built-in scene. The tool reports the frames shown,
how long a frame took to draw and the link's throughput. `--screenshot`
reads the frame on screen back over the link (about 2 s), saves it as a
640×360 PNG, and says whether it matches, pixel for pixel, what the
reference renderer draws: the last frame streamed or, with `--status`, one
of the built-in scene's frames.
