#!/usr/bin/env bash
# Build gs-host, the PC side of the host link (host/gs-host), for the machine
# the board is plugged into: the devcontainer has no USB access, so run the
# binary from the host, as with openFPGALoader. Run via `just gs-host`, which
# generates first (the sim/ workspace needs the generated crates).
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo build --release -p gs-host"
cargo build --quiet --release --manifest-path sim/Cargo.toml -p gs-host
echo "    built sim/target/release/gs-host; on the host, run:"
echo "    sim/target/release/gs-host --port /dev/ttyUSB0"
