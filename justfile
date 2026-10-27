# pico-gs2 task runner. Each recipe is a script in scripts/.

# List recipes.
default:
    @just --list

# Generate Verilog + Rust simulator crates from hw/, then build sim/.
build:
    scripts/build.sh

# Remove all build output.
clean:
    scripts/clean.sh

# Build, then run every check and test.
check: build
    scripts/check.sh
