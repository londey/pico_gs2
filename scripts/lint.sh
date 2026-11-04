#!/usr/bin/env bash
# Verilator lint of the board top level plus every generated module.
# Run via `just lint`, which generates first.
set -euo pipefail
cd "$(dirname "$0")/.."

# Module outputs named debug_* exist for the tests (the setup engine's write
# ports, say); the GPU leaves them unconnected, so they may go unused.
unused=(--unused-regexp '*debug*')

echo "==> verilator --lint-only -Wall"
verilator --lint-only -Wall "${unused[@]}" --top-module top board/*.sv build/sinter/*.sv

# The setup engine, not yet in the GPU, through its test root.
echo "==> verilator --lint-only -Wall (setup engine)"
verilator --lint-only -Wall "${unused[@]}" --top-module SetupTb build/sinter-setup/*.sv
