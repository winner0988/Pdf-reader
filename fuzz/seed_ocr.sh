#!/usr/bin/env bash
# Generates picture seeds for the ocr fuzz target (B2-10, #486).
set -euo pipefail
dir="${1:-fuzz/corpus/ocr}"
mkdir -p "$dir"
node "$(dirname "$0")/seed_ocr.mjs" "$dir"
