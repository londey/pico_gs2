#!/usr/bin/env bash
# Runs each time a session attaches to the container.
set -euo pipefail

echo "pico-gs2 dev container ready."
echo "  sinter:    $(sinter --version 2>/dev/null || echo 'not found')"
echo "  Yosys:     $(yosys -V 2>/dev/null | head -1 || echo 'not found')"
echo "  Verilator: $(verilator --version 2>/dev/null | head -1 || echo 'not found')"
echo "  Rust:      $(rustc --version 2>/dev/null || echo 'not found')"
echo "  just:      $(just --version 2>/dev/null || echo 'not found')"
