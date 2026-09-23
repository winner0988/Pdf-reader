#!/usr/bin/env bash
# Creates repo labels and one GitHub issue per work card in a backlog folder.
# Idempotent: labels are upserted, cards whose title already exists as an issue are skipped.
#
# Usage: bash scripts/backlog-to-issues.sh [--dry-run] [backlog-dir]
# Requires: GitHub CLI (gh) logged in, unless --dry-run.
set -euo pipefail

dry_run=false
if [ "${1:-}" = "--dry-run" ]; then
  dry_run=true
  shift
fi

cd "$(git rev-parse --show-toplevel)"
dir="${1:-docs/backlog/mvp-batch-1}"

# name|color|description
labels=(
  "task|1d76db|工作卡"
  "bug|d73a4a|缺陷"
  "decision|5319e7|需要負責人拍板，結果寫成 ADR"
  "mvp|0e8a16|MVP 範圍"
  "ux|c5def5|畫面與互動設計"
  "qa|fbca04|測試與品質"
  "security|b60205|安全相關"
  "dependencies|0366d6|依賴更新"
  "needs-security-review|b60205|合併前需負責人親自做安全審查"
  "blocked|000000|被其他工作或決策卡住"
  "area:build|ededed|專案骨架與建置"
  "area:app|ededed|Tauri 主行程"
  "area:worker|ededed|pdf_worker 與 MuPDF"
  "area:ipc|ededed|IPC 合約"
  "area:ui|ededed|前端介面"
  "area:ci|ededed|CI 與自動化"
  "area:security|ededed|安全機制"
  "agent:owner|f9d0c4|負責人"
  "agent:core|f9d0c4|核心 agent"
  "agent:frontend|f9d0c4|前端 agent"
  "agent:qa|f9d0c4|QA／安全 agent"
  "agent:release|f9d0c4|Review／Release agent"
)

run() {
  if $dry_run; then
    printf '[dry-run]'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

existing=""
if ! $dry_run; then
  command -v gh >/dev/null || { echo "GitHub CLI (gh) is not installed: https://cli.github.com/" >&2; exit 1; }
  gh auth status >/dev/null
  existing="$(gh issue list --state all --limit 1000 --json title --jq '.[].title')"
fi

for spec in "${labels[@]}"; do
  IFS='|' read -r name color desc <<< "$spec"
  run gh label create "$name" --color "$color" --description "$desc" --force
done

body="$(mktemp)"
trap 'rm -f "$body"' EXIT

shopt -s nullglob
for card in "$dir"/*.md; do
  [ "$(basename "$card")" = "README.md" ] && continue

  title="$(sed -n 's/^title: *"\(.*\)" *$/\1/p' "$card" | head -n 1)"
  card_labels="$(sed -n 's/^labels: *//p' "$card" | head -n 1)"
  if [ -z "$title" ] || [ -z "$card_labels" ]; then
    echo "skip (missing front matter): $card" >&2
    continue
  fi

  if [ -n "$existing" ] && grep -qxF "$title" <<< "$existing"; then
    echo "skip (already exists): $title"
    continue
  fi

  # Body = everything after the closing '---' of the front matter.
  awk 'n >= 2 { print; next } /^---[[:space:]]*$/ { n++ }' "$card" > "$body"
  run gh issue create --title "$title" --label "$card_labels" --body-file "$body"
done
