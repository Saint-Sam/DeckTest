#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS no_card_names.sh self-test"
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

names="$(mktemp "${TMPDIR:-/tmp}/forge_card_names.XXXXXX")"
matches="$(mktemp "${TMPDIR:-/tmp}/forge_card_matches.XXXXXX")"
trap 'rm -f "$names" "$matches"' EXIT

if [[ -n "${CARD_NAMES_FILE:-}" ]]; then
  if [[ ! -f "$CARD_NAMES_FILE" ]]; then
    echo "ERROR: CARD_NAMES_FILE does not exist: $CARD_NAMES_FILE" >&2
    exit 2
  fi
  awk 'NF && length($0) >= 4 { print }' "$CARD_NAMES_FILE" | sort -u >"$names"
elif [[ -d cards ]]; then
  {
    find cards -type f \( -name '*.json' -o -name '*.ron' -o -name '*.toml' -o -name '*.yaml' -o -name '*.yml' -o -name '*.txt' \) -print |
      sed 's|.*/||; s|\.[^.]*$||; s|[_-]| |g'
    grep -RhoE 'name[[:space:]]*[:=][[:space:]]*"[^"]+"' cards 2>/dev/null |
      sed -E 's/.*"([^"]+)".*/\1/' || true
  } |
    awk 'NF && length($0) >= 4 { print }' |
    sort -u >"$names"
fi

if [[ ! -s "$names" ]]; then
  echo "SKIP: no card-name source found; set CARD_NAMES_FILE or add cards/"
  exit 0
fi

grep -RInF -f "$names" "${engine_dirs[@]}" >"$matches" || true
if [[ -s "$matches" ]]; then
  echo "ERROR: card-specific names found in engine crates:" >&2
  cat "$matches" >&2
  exit 1
fi

echo "PASS no_card_names.sh"
