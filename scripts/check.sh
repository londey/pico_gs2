#!/usr/bin/env bash
# Type-check the design, run its directed stimulus tests and the board-level
# testbenches, then format check, lint and test the Rust workspace. Run via
# `just check`, which builds first.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> sinter check hw/gpu.snt"
sinter check hw/gpu.snt

# Runs a stimulus file, <root>.<case>.csv, against hw/<root>.snt in the
# simulator and, with --cosim, in the generated Verilog under Verilator,
# failing on any expectation or any difference between the two.
cosim() {
    local csv=$1
    local name root
    name=$(basename "$csv" .csv)
    root="hw/${name%%.*}.snt"
    echo "==> sinter sim $root --stimulus $csv --cosim"
    RUST_LOG=warn sinter sim "$root" --stimulus "$csv" \
        --cosim --cosim-dir "build/cosim/$name" > /dev/null
}

# Directed tests: hw/tests/<root>.<case>.csv.
for csv in hw/tests/*.csv; do
    cosim "$csv"
done

# Board-level testbenches: the hand-written logic in board/ that the Rust
# twin doesn't cover. tmds_gearbox runs at each clk_sclk phase the board can
# have (and two that it can't), several reset release times, and short and
# long clk_core-to-symbol delays. (Verilator wrongly reports the
# testbench's task and function locals as unused, so that one warning is
# off for the testbench only; board/*.sv itself is linted with all of
# them by `just lint`.)
echo "==> verilator board/tb/tmds_gearbox_tb.sv"
tb=build/verilator/tmds_gearbox_tb
verilator --binary --timing -Wall -Wno-UNUSEDSIGNAL --top-module tmds_gearbox_tb \
    -Mdir "$tb" -o tb board/tb/tmds_gearbox_tb.sv board/tmds_gearbox.sv > /dev/null
for offset in 0 1333 2666 4000; do
    for reset in 100000 103333 107777; do
        for delay in 500 2500; do
            "$tb/tb" +sclk_offset=$offset +reset=$reset +delay=$delay | grep -q '^PASS' || {
                echo "tmds_gearbox_tb failed: +sclk_offset=$offset +reset=$reset +delay=$delay" >&2
                exit 1
            }
        done
    done
done

manifest=(--manifest-path sim/Cargo.toml)
# The setup models are a workspace of their own (models/setup): see gen.sh.
setup=(--manifest-path models/setup/Cargo.toml)

echo "==> cargo fmt --check"
# From inside sim/: with --manifest-path, cargo fmt can't find the targets of
# members outside the workspace directory (models/).
(cd sim && cargo fmt --check)
(cd models/setup && cargo fmt --check)
echo "==> cargo clippy"
cargo clippy "${manifest[@]}" --all-targets -- -D warnings
cargo clippy "${setup[@]}" --all-targets -- -D warnings
echo "==> cargo test"
rm -rf build/stimulus
cargo test "${setup[@]}"
cargo test "${manifest[@]}"

# Stimulus the Rust testbenches recorded (build/stimulus/<root>.<case>.csv),
# replayed against the Verilog: the same long runs the twin was checked on.
for csv in build/stimulus/*.csv; do
    cosim "$csv"
done

echo "==> check passed"
