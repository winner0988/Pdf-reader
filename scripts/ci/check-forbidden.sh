#!/usr/bin/env bash
# Fails if any tracked file contains a forbidden telemetry / network pattern.
# Docs and Markdown are excluded because they legitimately discuss these names, and so are
# the guards themselves (the ban list in deny.toml, the guard scripts' tests), which must
# spell the forbidden names out.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

patterns_file="scripts/ci/forbidden-patterns.txt"
patterns="$(mktemp)"
trap 'rm -f "$patterns"' EXIT

# Strip comments and blank lines; git grep would treat them as patterns.
grep -Ev '^[[:space:]]*(#|$)' "$patterns_file" > "$patterns"

if matches="$(git grep -n -I -i -E -f "$patterns" -- . \
    ':(exclude)docs/**' \
    ':(exclude)*.md' \
    ':(exclude)deny.toml' \
    ':(exclude)scripts/ci/*.test.mjs' \
    ":(exclude)$patterns_file")"; then
  echo "::error::Forbidden telemetry/network pattern found (see AGENTS.md and ADR 0009):"
  echo "$matches"
  exit 1
fi

echo "OK: no forbidden patterns in tracked files."
