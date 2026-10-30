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
# The final (post-route) report: everything after "Routing complete".
final=$(awk '/Routing complete/ { f = 1 } f && /Max (frequency|delay)/' "$out/nextpnr.log" | sed 's/^Info: */    /')
echo "$final"

# nextpnr reports paths between clocks but doesn't check them. The board's
# crossings rely on them being short (board/top.sv): clk_core to clk_pixel
# edges can be 10 ns apart, clk_pixel to clk_tmds 8 ns. Allow 2 ns for skew.
check_crossing() {
    local from=$1 to=$2 budget=$3
    local delay
    delay=$(echo "$final" | awk -v f="$from" -v t="$to" '
        $0 ~ "posedge .*" f " *->" && $0 ~ "-> posedge .*" t " *:" { gsub(/ ns/, ""); print $NF }')
    if [[ -z "$delay" ]]; then
        echo "error: no $from -> $to path in the timing report" >&2
        exit 1
    fi
    if awk -v d="$delay" -v b="$budget" 'BEGIN { exit !(d > b) }'; then
        echo "error: $from -> $to path is $delay ns, over its $budget ns budget" >&2
        exit 1
    fi
}
check_crossing clk_core clk_pixel 8
check_crossing clk_pixel clk_tmds 6

echo "==> ecppack"
ecppack --compress "$out/top.config" "$out/top.bit"
echo "    wrote $out/top.bit"
