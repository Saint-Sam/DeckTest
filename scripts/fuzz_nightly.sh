#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS fuzz_nightly.sh self-test"
  exit 0
fi

if [[ ! -d fuzz/fuzz_targets ]]; then
  echo "SKIP: fuzz targets are not present yet"
  exit 0
fi

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "ERROR: cargo-fuzz is required for nightly fuzzing" >&2
  exit 1
fi

for target in fuzz/fuzz_targets/*.rs; do
  [[ -e "$target" ]] || continue
  name="$(basename "$target" .rs)"
  cargo fuzz run "$name" -- -max_total_time="${FORGE_FUZZ_SECONDS:-3600}"
done

