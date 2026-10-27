# pico-gs2 task runner. Each recipe is a script in scripts/.

# List recipes.
default:
    @just --list

# Generate Verilog + Rust simulator crates from hw/ into build/sinter/.
gen:
    scripts/gen.sh

# Generate, then build the Rust workspace in sim/.
build:
    scripts/build.sh

# Remove all build output.
clean:
    scripts/clean.sh

# Build, then run every check and test.
check: build
    scripts/check.sh

# Verilator lint of board/ plus the generated Verilog.
lint: gen
    scripts/lint.sh

# Synthesise, place and route into build/fpga/top.bit (fails on timing).
bitstream: gen
    scripts/bitstream.sh

# Load build/fpga/top.bit onto a connected ICEpi Zero.
flash:
    scripts/flash.sh

# Everything CI runs, in order.
ci:
    scripts/ci.sh
