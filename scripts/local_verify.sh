#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

mode="${1:-task}"
if [[ "$mode" == "--self-test" ]]; then
  [[ -x "$ROOT/scripts/vl.sh" ]]
  [[ -x "$ROOT/scripts/local_workers.sh" ]]
  [[ -x "$ROOT/tools/local_platform_metrics.py" ]]
  "$ROOT/scripts/local_workers.sh" --self-test
  echo "PASS local_verify.sh self-test"
  exit 0
fi
if [[ "$mode" != "task" && "$mode" != "gate" && "$mode" != "platforms" ]]; then
  echo "usage: scripts/local_verify.sh [task|gate|platforms]" >&2
  exit 2
fi

workers="$("$ROOT/scripts/local_workers.sh")"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$workers}"
export CARGO_NET_OFFLINE=true

echo "Local verification mode=$mode workers=$CARGO_BUILD_JOBS cargo_offline=$CARGO_NET_OFFLINE"

if [[ "$mode" == "task" || "$mode" == "gate" ]]; then
  "$ROOT/scripts/vl.sh"
  "$ROOT/scripts/review/determinism.sh"
  python3 "$ROOT/tools/oracle_semantic_metrics.py" --check
fi

if [[ "$mode" == "gate" ]]; then
  "$ROOT/scripts/run_oracle.sh" --all
  if find cards -type f -name '*.frs' -print -quit 2>/dev/null | grep -q .; then
    "$ROOT/scripts/card_regression.sh"
  fi
fi

if [[ "$mode" == "platforms" || "$mode" == "gate" ]]; then
  python3 tools/local_platform_metrics.py
fi

echo "LOCAL VERIFICATION PASSED mode=$mode workers=$CARGO_BUILD_JOBS"
