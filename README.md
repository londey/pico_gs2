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
