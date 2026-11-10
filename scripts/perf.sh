#!/usr/bin/env bash
# Performance figures (docs/design/host-interface.md §15), measured on the
# twin: sim/tb/examples/perf.rs, built in release mode, prints Markdown
# tables of each scene's draw time and where the cycles went. Run via
# `just perf`, which builds first. `--check` fails if a scene is slower than
# its figure in sim/tb/perf-baseline.txt (check.sh passes it); `--record`
# writes the figures there.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> perf (sim/tb/examples/perf.rs)"
cargo build --quiet --release --manifest-path sim/Cargo.toml -p tb --example perf 2>/dev/null
sim/target/release/examples/perf "$@"
