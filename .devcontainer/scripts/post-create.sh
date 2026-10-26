#!/usr/bin/env bash
# Runs once when the container is created.
set -euo pipefail

echo "==> pico-gs2 post-create"

# The named volume at ~/.cargo/registry can be root-owned if it predates the
# Dockerfile's pre-created tree; reclaim it before cargo touches it.
sudo chown -R "$(id -u):$(id -g)" "${CARGO_HOME:-$HOME/.cargo}/registry" || true

# Skills and the board repo are submodules with HTTPS URLs (no key needed).
# Not recursive: icepi-zero has a broken nested submodule (a KiCad library we
# don't use), and we only need its gateware/ files.
echo "==> Initialising submodules"
git submodule update --init

echo "==> Tool versions"
sinter --version
rustc --version
yosys -V
nextpnr-ecp5 --version 2>&1 | head -1
verilator --version
just --version

echo "==> post-create complete"
