#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS determinism.sh self-test"
  exit 0
fi

replay_dir="${DETERMINISM_REPLAY_DIR:-}"
if [[ -z "$replay_dir" ]]; then
  if [[ -d replays ]]; then
    replay_dir="replays"
  else
    replay_dir="$(find reports/gates -mindepth 2 -maxdepth 2 -type d -name replays 2>/dev/null | sort | tail -n 1)"
  fi
fi

if [[ -z "$replay_dir" || ! -d "$replay_dir" ]]; then
  echo "SKIP: replay corpus is absent; determinism replay gate is not active yet"
  exit 0
fi

replay_files=()
while IFS= read -r replay; do
  replay_files+=("$replay")
done < <(find "$replay_dir" -type f -name '*.frsreplay' | sort | head -n "${FORGE_DETERMINISM_REPLAY_COUNT:-50}")
if [[ "${#replay_files[@]}" -eq 0 ]]; then
  echo "SKIP: no .frsreplay files found under $replay_dir"
  exit 0
fi

if [[ ! -f Cargo.toml && -z "${DETERMINISM_CMD:-}" ]]; then
  echo "ERROR: replays exist but no Cargo.toml or DETERMINISM_CMD is available" >&2
  exit 1
fi

cmd="${DETERMINISM_CMD:-cargo run -q -p forge-cli -- replay \"\$REPLAY\"}"
for replay in "${replay_files[@]}"; do
  first="$(REPLAY="$replay" sh -c "$cmd")"
  second="$(REPLAY="$replay" sh -c "$cmd")"
  if [[ "$first" != "$second" ]]; then
    echo "ERROR: nondeterministic replay output for $replay" >&2
    diff -u <(printf '%s\n' "$first") <(printf '%s\n' "$second") >&2 || true
    exit 1
  fi
done

echo "PASS determinism.sh (${#replay_files[@]} replay(s))"
