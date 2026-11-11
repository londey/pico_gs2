#!/usr/bin/env bash
# Gather a release's files into build/release/<tag>/: the bitstream, the
# host interface (gs-proto as a standalone crate, its API docs, the design
# documents), gs-host for x86-64 Linux and the slides. Run via
# `just release [tag]` after `just bitstream` and `just slides`; the tag
# defaults to the one on HEAD. CI runs it for a pushed v* tag and attaches
# the files to a GitHub release, with docs/releases/<tag>.md as its notes.
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:-$(git describe --tags --exact-match 2>/dev/null || true)}"
if [[ -z "$tag" ]]; then
    echo "error: HEAD has no tag; pass one: just release v1.1.0" >&2
    exit 1
fi
if [[ ! -f "docs/releases/$tag.md" ]]; then
    echo "error: no release notes at docs/releases/$tag.md" >&2
    exit 1
fi
for f in build/fpga/top.bit build/slides/architecture.pdf; do
    if [[ ! -f "$f" ]]; then
        echo "error: $f is missing; run just bitstream and just slides first" >&2
        exit 1
    fi
done

out="build/release/$tag"
rm -rf "$out"
mkdir -p "$out"
manifest=sim/Cargo.toml

echo "==> bitstream"
cp build/fpga/top.bit "$out/pico_gs2-$tag.bit"

echo "==> gs-proto, as a crate a game can vendor"
version=$(cargo pkgid --quiet --manifest-path "$manifest" -p gs-proto | sed 's/.*[@#]//')
cargo package --quiet --manifest-path "$manifest" -p gs-proto --no-verify --allow-dirty
cp "sim/target/package/gs-proto-$version.crate" "$out/gs-proto-$version.tar.gz"

echo "==> API docs: gs-proto's rustdoc and the design documents"
rm -rf sim/target/doc
RUSTDOCFLAGS="-D warnings" cargo doc --quiet --manifest-path "$manifest" -p gs-proto --no-deps
docs="$out/pico_gs2-$tag-docs"
mkdir -p "$docs"
cp -r sim/target/doc "$docs/rustdoc"
cp -r docs/design docs/adr "$docs/"
cp "build/slides/architecture.pdf" "$docs/"
cat > "$docs/index.html" <<EOF
<!doctype html>
<meta charset="utf-8">
<title>pico_gs2 $tag</title>
<h1>pico_gs2 $tag</h1>
<ul>
  <li><a href="design/host-interface.md">The host interface</a>: registers,
      transactions, the serial link, performance (what a program is written against)</li>
  <li><a href="rustdoc/gs_proto/index.html">gs-proto</a>: the interface as a
      no_std Rust crate</li>
  <li><a href="architecture.pdf">Architecture slides</a></li>
  <li><a href="design/">Design notes</a> and <a href="adr/">decision records</a></li>
</ul>
EOF
tar -czf "$docs.tar.gz" -C "$out" "$(basename "$docs")"
rm -rf "$docs"
cp docs/design/host-interface.md "$out/host-interface.md"
cp build/slides/architecture.pdf "$out/pico_gs2-$tag-architecture.pdf"

echo "==> gs-host"
cargo build --quiet --release --manifest-path "$manifest" -p gs-host
cp sim/target/release/gs-host "$out/gs-host-$tag-x86_64-linux"

(cd "$out" && sha256sum -- * > SHA256SUMS)
echo "    wrote $out:"
ls -1 "$out" | sed 's/^/      /'
