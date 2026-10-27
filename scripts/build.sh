#!/usr/bin/env bash
# Generate the design, then build the Rust workspace in sim/ (which depends
# on the generated simulator crates).
set -euo pipefail
cd "$(dirname "$0")/.."

scripts/gen.sh

echo "==> cargo build (sim/)"
cargo build --manifest-path sim/Cargo.toml --all-targets
