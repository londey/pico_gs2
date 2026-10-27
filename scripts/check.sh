#!/usr/bin/env bash
# Type-check the design, then format check, lint and test the Rust workspace.
# Run via `just check`, which builds first.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -f hw/gpu.snt ]]; then
    echo "==> sinter check hw/gpu.snt"
    sinter check hw/gpu.snt
else
    echo "==> skip sinter check: hw/gpu.snt not found"
fi

if [[ -f sim/Cargo.toml ]]; then
    manifest=(--manifest-path sim/Cargo.toml)
    echo "==> cargo fmt --check"
    cargo fmt "${manifest[@]}" --all --check
    echo "==> cargo clippy"
    cargo clippy "${manifest[@]}" --all-targets -- -D warnings
    echo "==> cargo test"
    cargo test "${manifest[@]}"
else
    echo "==> skip cargo checks: sim/Cargo.toml not found"
fi

echo "==> check passed"
