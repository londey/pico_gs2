#!/usr/bin/env bash
# Synthesise, place and route the setup engine on its own (hw/setup_synth.snt
# drives it from an LFSR), and report its resources, Fmax and critical path.
# nextpnr fails the run if 75 MHz isn't met. Run via `just setup-report`,
# which generates first (the micro-ROMs come from setup-gen).
set -euo pipefail
cd "$(dirname "$0")/.."

out=build/setup-synth
rm -rf "$out"
mkdir -p "$out"

echo "==> sinter build hw/setup_synth.snt"
sinter build hw/setup_synth.snt --out "$out/sinter" > /dev/null

echo "==> yosys synth_ecp5"
yosys -q -l "$out/yosys.log" -p "
    read_verilog -sv $out/sinter/*.sv
    synth_ecp5 -top SetupSynth -json $out/setup.json
"

echo "==> nextpnr-ecp5 (75 MHz)"
nextpnr-ecp5 --25k --package CABGA256 --freq 75 --lpf-allow-unconstrained \
    --json "$out/setup.json" --report "$out/report.json" \
    --log "$out/nextpnr.log" --quiet
grep -E "(TRELLIS_COMB|TRELLIS_FF|DP16KD|MULT18X18D|TRELLIS_RAMW):" "$out/nextpnr.log" \
    | sed 's/Info: */    /'
awk '/Routing complete/ { f = 1 } f && /Max frequency/' "$out/nextpnr.log" | sed 's/Info: */    /'
# The critical path's two ends.
awk '/Critical path report for clock/ { f = 1 } f && /(Source|Sink)/' "$out/nextpnr.log" \
    | sed -n '1p;$p' | sed -E 's/Info: +//; s/ +/ /g' | cut -c1-120 | sed 's/^/    /'
