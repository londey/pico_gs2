#!/usr/bin/env bash
# Type-check and lint the design, run its directed stimulus tests and the
# board-level testbenches, then format check, lint and test the Rust
# workspace. Run via
# `just check`, which builds first.
#
# The slow parts are independent runs (co-simulations, test binaries, the
# gearbox testbench's cases), so each batch runs in parallel, one process a
# run; each run's output goes to build/check-logs/<batch>/, and a failed
# run's is printed.
set -euo pipefail
cd "$(dirname "$0")/.."

# Runs commands read from standard input, one a line, at most `max` at once
# (default: all); waits for them all; prints each failure's command and
# output. Fails if any failed.
parallel() {
    local batch=$1 max=${2:-0}
    local dir="build/check-logs/$batch"
    rm -rf "$dir"
    mkdir -p "$dir"
    local n=0 cmd
    while IFS= read -r cmd; do
        if [ "$max" -gt 0 ]; then
            while [ "$(jobs -rp | wc -l)" -ge "$max" ]; do
                wait -n || true
            done
        fi
        printf '%s\n' "$cmd" > "$dir/$n.cmd"
        ( (eval "$cmd") > "$dir/$n.log" 2>&1; echo $? > "$dir/$n.status" ) &
        n=$((n + 1))
    done
    wait
    local failed=0 k
    for ((k = 0; k < n; k++)); do
        if [ "$(cat "$dir/$k.status")" != 0 ]; then
            echo "FAILED: $(cat "$dir/$k.cmd")" >&2
            cat "$dir/$k.log" >&2
            failed=1
        fi
    done
    echo "    $n runs"
    return $failed
}

# How many `sinter sim --cosim` runs of the recorded stimulus fit in memory
# at once: the longest (16 million cycles) holds about 11 GB.
cosim_slots() {
    local avail
    avail=$(awk '/MemAvailable/ { print $2 }' /proc/meminfo)
    local slots=$((avail / (12 * 1024 * 1024)))
    echo $((slots < 1 ? 1 : slots))
}

# The test binaries of a workspace, as commands that run each from its
# package's directory (as cargo test does), after building them.
test_commands() {
    cargo test "$@" --no-run --message-format=json | python3 -c '
import json, os, shlex, sys
for line in sys.stdin:
    m = json.loads(line)
    exe = m.get("executable")
    if m.get("reason") == "compiler-artifact" and m["profile"]["test"] and exe:
        d = os.path.dirname(m["manifest_path"])
        print("cd " + shlex.quote(d) + " && " + shlex.quote(exe))
'
}

echo "==> sinter check hw/gpu.snt"
sinter check hw/gpu.snt

scripts/lint.sh

# The design a stimulus file, <root>.<case>.csv, drives: hw/<root>.snt.
root_of() {
    local name
    name=$(basename "$1" .csv)
    echo "hw/${name%%.*}.snt"
}

# Runs a stimulus file against its design in the compiled simulator and,
# with --cosim, in the generated Verilog under Verilator, failing on any
# expectation or any difference between the two. Each run builds its own
# simulator, in build/sim/<case>/: sinter sim writes the stimulus to a fixed
# file in its --sim-dir, so runs sharing one would overwrite each other's
# (docs/sinter-requirements.md R12).
cosim() {
    local csv=$1
    local name root
    name=$(basename "$csv" .csv)
    root=$(root_of "$csv")
    echo "==> sinter sim $root --stimulus $csv --cosim"
    RUST_LOG=warn sinter sim "$root" --stimulus "$csv" --quiet \
        --sim-dir "build/sim/$name" --cosim --cosim-dir "build/cosim/$name" > /dev/null
}

# Directed tests: hw/tests/<root>.<case>.csv.
echo "==> directed stimulus, --cosim"
for csv in hw/tests/*.csv; do
    echo "cosim $csv"
done | parallel directed

# Board-level testbenches: the hand-written logic in board/ that the Rust
# twin doesn't cover. tmds_gearbox runs at each clk_sclk phase the board can
# have (and two that it can't), several reset release times, and short and
# long clk_core-to-symbol delays. (Verilator wrongly reports the
# testbench's task and function locals as unused, so that one warning is
# off for the testbench only; board/*.sv itself is linted with all of
# them by lint.sh, above.)
echo "==> verilator board/tb/tmds_gearbox_tb.sv"
tb=build/verilator/tmds_gearbox_tb
verilator --binary --timing -Wall -Wno-UNUSEDSIGNAL --top-module tmds_gearbox_tb \
    -Mdir "$tb" -o tb board/tb/tmds_gearbox_tb.sv board/tmds_gearbox.sv > /dev/null
for offset in 0 1333 2666 4000; do
    for reset in 100000 103333 107777; do
        for delay in 500 2500; do
            echo "$tb/tb +sclk_offset=$offset +reset=$reset +delay=$delay | grep '^PASS'"
        done
    done
done | parallel gearbox

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
cargo test "${setup[@]}" --doc
cargo test "${manifest[@]}" --doc
{
    test_commands "${setup[@]}"
    test_commands "${manifest[@]}"
} > build/check-tests.txt
parallel tests < build/check-tests.txt

# Stimulus the Rust testbenches recorded (build/stimulus/<root>.<case>.csv),
# replayed against the Verilog: the same long runs the twin was checked on.
# Largest first, as many at once as memory allows.
slots=$(cosim_slots)
echo "==> recorded stimulus, --cosim ($slots at once)"
for csv in $(ls -S build/stimulus/*.csv); do
    echo "cosim $csv"
done | parallel stimulus "$slots"

echo "==> check passed"
