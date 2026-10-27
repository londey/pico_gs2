#!/usr/bin/env bash
# Remove generated Verilog, simulator crates, bitstreams and cargo output.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> rm -rf build/ sim/target/"
rm -rf build sim/target
