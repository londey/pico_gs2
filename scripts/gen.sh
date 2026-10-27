#!/usr/bin/env bash
# Generate SystemVerilog and the Rust simulator crates from the Sinter design.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> sinter build hw/gpu.snt"
rm -rf build/sinter
sinter build hw/gpu.snt --out build/sinter
