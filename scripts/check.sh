#!/usr/bin/env bash
# Type-check the design, run its directed stimulus tests, then format check,
# lint and test the Rust workspace. Run via `just check`, which builds first.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> sinter check hw/gpu.snt"
sinter check hw/gpu.snt

# Directed tests: hw/tests/<root>.<case>.csv drives hw/<root>.snt.
for csv in hw/tests/*.csv; do
    root="hw/$(basename "$csv" | cut -d. -f1).snt"
    echo "==> sinter sim $root --stimulus $csv"
    RUST_LOG=warn sinter sim "$root" --stimulus "$csv" > /dev/null
done

manifest=(--manifest-path sim/Cargo.toml)
echo "==> cargo fmt --check"
cargo fmt "${manifest[@]}" --check
echo "==> cargo clippy"
cargo clippy "${manifest[@]}" --all-targets -- -D warnings
echo "==> cargo test"
cargo test "${manifest[@]}"

echo "==> check passed"
