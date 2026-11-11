#!/usr/bin/env bash
# Fetches pinned public PDF corpora and verifies their contents against sha256 manifests.
# Usage: fetch.sh [--write-manifest] <destination_dir> [name...]
# Rationale: Issue #439 (B5-02). Untrusted corpus files are downloaded outside the repo.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CORPORA_TSV="$REPO_ROOT/tests/corpus-sweep/corpora.tsv"
MANIFEST_DIR="$REPO_ROOT/tests/corpus-sweep"

write_manifest=0
positional=()

for arg in "$@"; do
  case "$arg" in
    --write-manifest)
      write_manifest=1
      ;;
    -h|--help)
      echo "Usage: fetch.sh [--write-manifest] <destination_dir> [name...]"
      exit 0
      ;;
    *)
      positional+=("$arg")
      ;;
  esac
done

if [ "${#positional[@]}" -lt 1 ]; then
  echo "Usage: fetch.sh [--write-manifest] <destination_dir> [name...]" >&2
  exit 1
fi

dest_dir="${positional[0]}"
requested_names=("${positional[@]:1}")

if [ ! -f "$CORPORA_TSV" ]; then
  echo "Error: corpora definition file not found: $CORPORA_TSV" >&2
  exit 1
fi

# Load corpora from corpora.tsv
declare -A corpus_repo
declare -A corpus_commit
declare -A corpus_scope
declare -A corpus_license
all_names=()

while IFS=$'\t' read -r name repo commit scope license || [ -n "$name" ]; do
  # Skip comments and header lines
  [[ "$name" =~ ^[[:space:]]*# ]] && continue
  [ -z "$name" ] && continue
  if [ "$name" = "名字" ] || [ "$name" = "name" ]; then
    continue
  fi
  all_names+=("$name")
  corpus_repo["$name"]="$repo"
  corpus_commit["$name"]="$commit"
  corpus_scope["$name"]="$scope"
  corpus_license["$name"]="$license"
done < "$CORPORA_TSV"

targets=()
if [ "${#requested_names[@]}" -eq 0 ]; then
  targets=("${all_names[@]}")
else
  for rname in "${requested_names[@]}"; do
    if [ -z "${corpus_repo["$rname"]+x}" ]; then
      echo "Error: unknown corpus: $rname" >&2
      exit 1
    fi
    targets+=("$rname")
  done
fi

mkdir -p "$dest_dir"

for name in "${targets[@]}"; do
  repo="${corpus_repo["$name"]}"
  commit="${corpus_commit["$name"]}"
  scope="${corpus_scope["$name"]}"
  target_dir="$dest_dir/$name"
  manifest_file="$MANIFEST_DIR/${name}.sha256"

  echo "=== Corpus: $name ($repo @ $commit) ==="
  tarball="$(mktemp -t "corpus-${name}-XXXXXX.tar.gz")"
  # Clean up tarball on exit or error
  # shellcheck disable=SC2064
  trap "rm -f '$tarball'" EXIT

  url="https://codeload.github.com/${repo}/tar.gz/${commit}"
  echo "Downloading $url ..."
  curl --fail --location --proto '=https' --max-filesize 524288000 -o "$tarball" "$url"

  echo "Extracting to $target_dir ..."
  mkdir -p "$target_dir"

  # Extract only regular PDF files in scope
  if [ "$scope" = "test/pdfs/*.pdf" ] || [ "$scope" = "test/pdfs" ]; then
    tar -xzf "$tarball" -C "$target_dir" --strip-components=1 --wildcards '*/test/pdfs/*.[pP][dD][fF]'
  else
    tar -xzf "$tarball" -C "$target_dir" --strip-components=1 --wildcards --no-anchored '*.[pP][dD][fF]'
  fi

  # Remove any symbolic links to ensure only regular files are retained
  find "$target_dir" -type l -exec rm -f {} +

  rm -f "$tarball"
  trap - EXIT

  if [ "$write_manifest" -eq 1 ]; then
    echo "Writing manifest $manifest_file ..."
    (
      cd "$target_dir"
      find . -type f \( -name '*.pdf' -o -name '*.Pdf' -o -name '*.PDF' \) | \
        sed 's|^\./||' | \
        sort | \
        xargs -d '\n' sha256sum | \
        sed 's/ \*/  /'
    ) > "$manifest_file"
    echo "Wrote $(wc -l < "$manifest_file") files to $manifest_file"
  else
    if [ ! -f "$manifest_file" ]; then
      echo "Error: manifest file does not exist: $manifest_file (run with --write-manifest first)" >&2
      exit 1
    fi

    echo "Verifying $name against $manifest_file ..."
    (
      cd "$target_dir"
      # 1. Verify all manifest entries match
      if ! sha256sum --check --status "$manifest_file"; then
        echo "Error: sha256 mismatch or missing files in $name:" >&2
        sha256sum --check "$manifest_file" | grep -v ': OK$' >&2
        exit 1
      fi

      # 2. Verify no extra PDF files exist
      actual_list="$(mktemp)"
      manifest_list="$(mktemp)"
      # shellcheck disable=SC2064
      trap "rm -f '$actual_list' '$manifest_list'" EXIT

      find . -type f \( -name '*.pdf' -o -name '*.Pdf' -o -name '*.PDF' \) | sed 's|^\./||' | sort > "$actual_list"
      sed 's/^[0-9a-fA-F]* [ *]//' "$manifest_file" | sort > "$manifest_list"

      extra_files="$(comm -23 "$actual_list" "$manifest_list")"
      rm -f "$actual_list" "$manifest_list"
      trap - EXIT

      if [ -n "$extra_files" ]; then
        echo "Error: extra PDF files found in $target_dir not in manifest:" >&2
        echo "$extra_files" >&2
        exit 1
      fi
    )
    echo "Verified $name: all files match manifest, no extra files."
  fi
done

echo "Fetch and verification complete."
