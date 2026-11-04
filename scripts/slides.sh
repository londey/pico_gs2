#!/usr/bin/env bash
# Compile the architecture slides (docs/slides/*.typ) to PDFs in
# build/slides/. Run via `just slides`.
set -euo pipefail
cd "$(dirname "$0")/.."

# The slides' theme (docs/slides/theme/pico.typ) sets them in Inter and
# JetBrains Mono, which the devcontainer installs. Typst falls back to a
# serif font without them, with only a warning, so check first.
fonts="$(typst fonts)"
for family in "Inter" "Inter Display" "JetBrains Mono"; do
    if ! grep -qx "$family" <<< "$fonts"; then
        echo "error: font '$family' not found; rebuild the devcontainer" \
            "(.devcontainer/Dockerfile installs it)" >&2
        exit 1
    fi
done

mkdir -p build/slides
for src in docs/slides/*.typ; do
    out="build/slides/$(basename "$src" .typ).pdf"
    echo "==> typst compile $src"
    typst compile "$src" "$out"
    echo "    wrote $out"
done
