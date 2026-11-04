#!/usr/bin/env bash
# Generate SystemVerilog and the Rust simulator crates from the Sinter design.
set -euo pipefail
cd "$(dirname "$0")/.."

# The setup engine's micro-ROMs and seed table, which hw/setup.snt loads
# with include_hex!. models/setup is its own Cargo workspace, so this
# doesn't need the crates sinter generates below.
echo "==> setup-gen build/gen"
cargo run --quiet --release --manifest-path models/setup/Cargo.toml --bin setup-gen -- build/gen

echo "==> sinter build hw/gpu.snt"
rm -rf build/sinter
sinter build hw/gpu.snt --out build/sinter

# The setup engine on its own, for its cycle-exact tests (sim/tb) until it
# joins the GPU.
echo "==> sinter build hw/setup_tb.snt"
rm -rf build/sinter-setup
sinter build hw/setup_tb.snt --out build/sinter-setup
