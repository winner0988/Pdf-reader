#!/usr/bin/env bash
# Checks docs/adr/: numbering is unique and contiguous, required sections exist,
# and every ADR is listed in docs/adr/README.md.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)/docs/adr"

fail=0
expected=1
for f in [0-9][0-9][0-9][0-9]-*.md; do
  [ -e "$f" ] || continue
  num=$((10#${f:0:4}))

  if [ "$num" -ne "$expected" ]; then
    echo "::error file=docs/adr/$f::ADR number $num is out of sequence (expected $expected)."
    fail=1
  fi
  expected=$((num + 1))

  for section in '## 狀態' '## 背景' '## 決定' '## 後果'; do
    if ! grep -q "^$section" "$f"; then
      echo "::error file=docs/adr/$f::Missing section: $section"
      fail=1
    fi
  done

  if ! grep -qF "]($f)" README.md; then
    echo "::error file=docs/adr/README.md::$f is not listed in the ADR index."
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK: $((expected - 1)) ADRs, numbering and index consistent."
