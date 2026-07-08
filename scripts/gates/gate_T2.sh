#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -x "$ROOT/scripts/vl.sh" ]]
  [[ -x "$ROOT/scripts/run_oracle.sh" ]]
  [[ -x "$ROOT/scripts/run_nightmare_suite.sh" ]]
  [[ -x "$ROOT/scripts/fuzz_nightly.sh" ]]
  echo "PASS gate_T2.sh self-test"
  exit 0
fi

failures=0

oracle_count="$(find tests/oracle -type f -name '*.ron' 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$oracle_count" -lt 1200 ]]; then
  echo "ERROR: T2 gate requires >=1200 oracle scenarios; found $oracle_count" >&2
  failures=$((failures + 1))
fi

"$ROOT/scripts/vl.sh"
"$ROOT/scripts/run_oracle.sh" --all
"$ROOT/scripts/run_nightmare_suite.sh" "${FORGE_T2_NIGHTMARE_GAMES:-1000}" "${FORGE_T2_NIGHTMARE_MAX_TURNS:-6}"

if [[ "${FORGE_T2_RUN_FUZZ:-0}" == "1" ]]; then
  "$ROOT/scripts/fuzz_nightly.sh" --t2-gate
else
  echo "ERROR: T2 gate requires a 12-hour sanitizer fuzz run; rerun with FORGE_T2_RUN_FUZZ=1 or attach equivalent current evidence" >&2
  failures=$((failures + 1))
fi

if [[ "$failures" -ne 0 ]]; then
  echo "FAIL gate_T2.sh ($failures issue(s))" >&2
  exit 1
fi

echo "PASS gate_T2.sh"
