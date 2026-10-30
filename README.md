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

`just` on its own lists every recipe.

The devcontainer has no USB access, so flash from the host instead:
`openFPGALoader -b icepi-zero build/fpga/top.bit` (the workspace is shared,
so the bitstream is there too). This loads the FPGA's configuration RAM,
which is lost at power off; add `--write-flash` to keep it.

## On the board

The design fills a 640x480 RGB565 framebuffer in the SDRAM with a test
pattern and scans it out to the GPDI (mini HDMI) port at 640x480, 60 Hz:
red rising left to right, green rising top to bottom, a blue checkerboard
of 32-pixel squares and a one-pixel white border. The LEDs show:

| LED | Lit when |
|---|---|
| `led[0]` | blinks: the heartbeat, about 0.75 Hz |
| `led[1]` | the SDRAM has been initialised |
| `led[2]` | the framebuffer has been filled |
| `led[3]` | the scan-out ran out of pixels (it never should) |
