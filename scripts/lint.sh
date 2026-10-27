#!/usr/bin/env bash
# Verilator lint of the board top level plus every generated module.
# Run via `just lint`, which generates first.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> verilator --lint-only -Wall"
verilator --lint-only -Wall --top-module top board/*.sv build/sinter/*.sv
