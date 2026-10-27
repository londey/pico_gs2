#!/usr/bin/env bash
# Everything CI runs, in order.
set -euo pipefail
cd "$(dirname "$0")/.."

scripts/build.sh
scripts/check.sh
scripts/lint.sh
scripts/bitstream.sh
