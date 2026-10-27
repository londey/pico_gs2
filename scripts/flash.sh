#!/usr/bin/env bash
# Load the bitstream into the FPGA's configuration RAM (lost at power off).
set -euo pipefail
cd "$(dirname "$0")/.."

openFPGALoader -b icepi-zero build/fpga/top.bit
