#!/usr/bin/env bash
# Performance figures (docs/design/host-interface.md §15), measured on the
# twin: sim/tb/examples/perf.rs, built in release mode, prints a Markdown
# table of each scene's draw time and where the cycles went. Run via
# `just perf`, which builds first.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> perf (sim/tb/examples/perf.rs)"
cargo run --quiet --release --manifest-path sim/Cargo.toml -p tb --example perf 2>/dev/null
