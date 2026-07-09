#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

mode="${1:-task}"
if [[ "$mode" == "--self-test" ]]; then
  [[ -x tools/run_local_fuzz.py ]]
  rustup run nightly-2026-07-05 cargo fuzz --version | grep -q 'cargo-fuzz 0.13.2'
  echo "PASS fuzz_local_parallel.sh self-test"
  exit 0
fi

minimum_total_args=()
evidence_args=()
if [[ -n "${FORGE_CP_DSL_EVIDENCE_DIR:-}" ]]; then
  evidence_args=(--evidence-dir "${FORGE_CP_DSL_EVIDENCE_DIR}/fuzz")
fi
case "$mode" in
  smoke) seconds="${FORGE_FUZZ_SECONDS:-10}" ;;
  task) seconds="${FORGE_FUZZ_SECONDS:-30}" ;;
  gate)
    seconds="${FORGE_FUZZ_SECONDS:-300}"
    minimum_total_args=(--minimum-total-worker-seconds 2400)
    ;;
  check)
    exec python3 tools/run_local_fuzz.py --check \
      --minimum-worker-seconds "${FORGE_FUZZ_MINIMUM_WORKER_SECONDS:-60}"
    ;;
  *)
    echo "usage: scripts/fuzz_local_parallel.sh [smoke|task|gate|check]" >&2
    exit 2
    ;;
esac

exec python3 tools/run_local_fuzz.py --seconds "$seconds" \
  "${minimum_total_args[@]}" "${evidence_args[@]}"
