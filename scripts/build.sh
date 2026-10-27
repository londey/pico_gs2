#!/usr/bin/env bash
# Generate SystemVerilog and Rust simulator crates from the Sinter design,
# then build the Rust workspace in sim/ (which depends on the generated crates).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -f hw/gpu.snt ]]; then
    echo "==> sinter build hw/gpu.snt"
    sinter build hw/gpu.snt --out build/sinter
else
    echo "==> skip sinter build: hw/gpu.snt not found"
fi

if [[ -f sim/Cargo.toml ]]; then
    echo "==> cargo build (sim/)"
    cargo build --manifest-path sim/Cargo.toml --all-targets
else
    echo "==> skip cargo build: sim/Cargo.toml not found"
fi
