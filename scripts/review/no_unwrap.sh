#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS no_unwrap.sh self-test"
  exit 0
fi

engine_dirs=()
for dir in crates/forge-core crates/forge-ai; do
  [[ -d "$dir" ]] && engine_dirs+=("$dir")
done

if [[ "${#engine_dirs[@]}" -eq 0 ]]; then
  echo "SKIP: no forge-core or forge-ai crate directories found"
  exit 0
fi

tmp="$(mktemp "${TMPDIR:-/tmp}/forge_no_unwrap.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

for dir in "${engine_dirs[@]}"; do
  while IFS= read -r -d '' file; do
    grep -nE '\.(unwrap|expect)[[:space:]]*\(' "$file" | sed "s|^|$file:|" >>"$tmp" || true
  done < <(
    find "$dir" -type f -name '*.rs' \
      ! -path '*/tests/*' \
      ! -path '*/benches/*' \
      ! -path '*/examples/*' \
      ! -name '*_test.rs' \
      ! -name 'tests.rs' \
      -print0
  )
done

if [[ -s "$tmp" ]]; then
  echo "ERROR: unwrap()/expect() found in non-test engine code paths:" >&2
  cat "$tmp" >&2
  exit 1
fi

echo "PASS no_unwrap.sh"
