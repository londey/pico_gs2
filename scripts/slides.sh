#!/usr/bin/env bash
# Compile the architecture slides (docs/slides/*.typ) to PDFs in
# build/slides/. Run via `just slides`.
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p build/slides
for src in docs/slides/*.typ; do
    out="build/slides/$(basename "$src" .typ).pdf"
    echo "==> typst compile $src"
    typst compile "$src" "$out"
    echo "    wrote $out"
done
