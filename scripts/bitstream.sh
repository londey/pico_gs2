#!/usr/bin/env bash
# Synthesise, place and route, and pack the bitstream into build/fpga/.
# nextpnr fails the run if any clock misses its constraint.
# Run via `just bitstream`, which generates first.
set -euo pipefail
cd "$(dirname "$0")/.."

out=build/fpga
mkdir -p "$out"

echo "==> yosys synth_ecp5"
yosys -q -l "$out/yosys.log" -p "
    read_verilog -sv -D SYNTHESIS board/*.sv build/sinter/*.sv
    synth_ecp5 -top top -json $out/top.json
"

echo "==> nextpnr-ecp5"
nextpnr-ecp5 --25k --package CABGA256 \
    --lpf board/icepi_zero.lpf \
    --json "$out/top.json" \
    --textcfg "$out/top.config" \
    --report "$out/report.json" \
    --log "$out/nextpnr.log" \
    --quiet
grep -E "Max frequency for clock" "$out/nextpnr.log" | sed 's/^Info: */    /' | sort -u

echo "==> ecppack"
ecppack --compress "$out/top.config" "$out/top.bit"
echo "    wrote $out/top.bit"
