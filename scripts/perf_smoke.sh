#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  python3 "$ROOT/tools/perf_diff.py" --self-test
  echo "PASS perf_smoke.sh self-test"
  exit 0
fi

threshold="${FORGE_PERF_THRESHOLD:-0.05}"
python3 "$ROOT/tools/perf_diff.py" --threshold "$threshold"
