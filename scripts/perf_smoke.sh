#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  python3 "$ROOT/tools/perf_diff.py" --self-test
  python3 "$ROOT/tools/criterion_to_perf.py" --self-test
  echo "PASS perf_smoke.sh self-test"
  exit 0
fi

threshold="${FORGE_PERF_THRESHOLD:-0.05}"
target_dir="${CARGO_TARGET_DIR:-target}"
criterion_root="$target_dir/criterion"
stamp="$ROOT/metrics/perf_bench_start.tmp"
if [[ "${FORGE_PERF_RUN_BENCHES:-1}" != "0" ]]; then
  : > "$stamp"
  cargo bench -p forge-core --bench kernel -- --quiet
  python3 "$ROOT/tools/criterion_to_perf.py" \
    --criterion-root "$criterion_root" \
    --since-file "$stamp" \
    --out "$ROOT/metrics/perf_current.json"
fi

python3 "$ROOT/tools/perf_diff.py" \
  --threshold "$threshold" \
  --summary-out "$ROOT/metrics/perf_summary.tmp.json"
