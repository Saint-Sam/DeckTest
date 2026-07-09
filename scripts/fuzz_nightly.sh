#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

mode="${1:-nightly}"

if [[ "$mode" == "--self-test" ]]; then
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

targets=()
for target in fuzz/fuzz_targets/*.rs; do
  [[ -e "$target" ]] || continue
  targets+=("$target")
done

if [[ "${#targets[@]}" -eq 0 ]]; then
  echo "SKIP: no fuzz targets are present"
  exit 0
fi

if [[ "$mode" == "--t1-gate" ]]; then
  seconds="${FORGE_FUZZ_SECONDS:-$(((6 * 60 * 60 + ${#targets[@]} - 1) / ${#targets[@]}))}"
elif [[ "$mode" == "--t2-gate" ]]; then
  seconds="${FORGE_FUZZ_SECONDS:-$(((12 * 60 * 60 + ${#targets[@]} - 1) / ${#targets[@]}))}"
else
  seconds="${FORGE_FUZZ_SECONDS:-3600}"
fi

sanitizer="${FORGE_FUZZ_SANITIZER:-address}"
if [[ -n "${FORGE_FUZZ_TOOLCHAIN:-}" ]]; then
  toolchain="$FORGE_FUZZ_TOOLCHAIN"
elif [[ "$sanitizer" == "none" ]]; then
  toolchain=""
else
  toolchain="nightly-2026-07-05"
fi

cargo_cmd=(cargo)
if [[ -n "$toolchain" ]]; then
  cargo_cmd+=("+$toolchain")
fi

echo "Running ${#targets[@]} fuzz target(s), ${seconds}s per target, sanitizer=${sanitizer}, mode=${mode}, toolchain=${toolchain:-default}"

for target in "${targets[@]}"; do
  name="$(basename "$target" .rs)"
  "${cargo_cmd[@]}" fuzz run --sanitizer "$sanitizer" "$name" -- -max_total_time="$seconds"
done
