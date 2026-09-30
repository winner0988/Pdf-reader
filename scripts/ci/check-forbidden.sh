#!/usr/bin/env bash
# Fails if any tracked file contains a forbidden telemetry / network pattern.
# Docs and Markdown are excluded because they legitimately discuss these names, and so are
# the guards themselves (the ban list in deny.toml, the guard scripts' tests), which must
# spell the forbidden names out. A pattern followed by "@only <path>..." is allowed in those
# files and forbidden everywhere else (ADR 0009: the update check's one module).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

patterns_file="scripts/ci/forbidden-patterns.txt"
patterns="$(mktemp)"
trap 'rm -f "$patterns"' EXIT

excludes=(
  ':(exclude)docs/**'
  ':(exclude)*.md'
  ':(exclude)deny.toml'
  ':(exclude)scripts/ci/*.test.mjs'
  ":(exclude)$patterns_file"
)

# Strip comments and blank lines; git grep would treat them as patterns.
rules="$(grep -Ev '^[[:space:]]*(#|$)' "$patterns_file")"
grep -v ' @only ' <<< "$rules" > "$patterns" || true

found=0
if [ -s "$patterns" ] && matches="$(git grep -n -I -i -E -f "$patterns" -- . "${excludes[@]}")"; then
  echo "::error::Forbidden telemetry/network pattern found (see AGENTS.md and ADR 0009):"
  echo "$matches"
  found=1
fi

while IFS= read -r rule; do
  [ -n "$rule" ] || continue
  pattern="${rule%% @only *}"
  read -ra paths <<< "${rule#* @only }"
  allowed=()
  for path in "${paths[@]}"; do
    allowed+=(":(exclude)$path")
  done
  if matches="$(git grep -n -I -i -E -e "$pattern" -- . "${excludes[@]}" "${allowed[@]}")"; then
    echo "::error::'$pattern' is only allowed in ${paths[*]} (see ADR 0009):"
    echo "$matches"
    found=1
  fi
done <<< "$(grep ' @only ' <<< "$rules" || true)"

if [ "$found" -ne 0 ]; then
  exit 1
fi
echo "OK: no forbidden patterns in tracked files."
