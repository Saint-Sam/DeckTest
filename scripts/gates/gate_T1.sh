#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS gate_T1.sh self-test"
  exit 0
fi

failures=0

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "ERROR: missing required T1 gate artifact: $path" >&2
    failures=$((failures + 1))
  fi
}

oracle_count="$(find tests/oracle -type f -name '*.ron' 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$oracle_count" -lt 250 ]]; then
  echo "ERROR: T1 gate requires >=250 oracle scenarios; found $oracle_count" >&2
  failures=$((failures + 1))
fi

"$ROOT/scripts/vl.sh"
"$ROOT/scripts/run_oracle.sh" --all
"$ROOT/scripts/perf_smoke.sh"
cargo run -p forge-arena -- --smoke 10000 --random

demo_replay="target/gates/T1/replays/demo-seed-11.frsreplay"
cargo run -p forge-cli -- play --demo --seed 11 --replay-out "$demo_replay"
cargo run -p forge-cli -- roundtrip "$demo_replay"

require_file "reports/fuzz/T1.12-2026-07-07-run.log"
require_file "reports/fuzz/T1.12-2026-07-07.md"
require_file "metrics/perf_baseline.json"
require_file "metrics/perf_current.json"

clone_ns="$(python3 - <<'PY'
import json
from pathlib import Path
payload = json.loads(Path("metrics/perf_current.json").read_text())
print(payload.get("kernel_clone_200_card_state_x64", "nan"))
PY
)"
python3 - "$clone_ns" <<'PY' || failures=$((failures + 1))
import math
import sys

value = float(sys.argv[1])
per_clone = value / 64.0
if not math.isfinite(per_clone) or per_clone > 200.0:
    print(
        f"ERROR: T1 gate requires clone <=200 ns per 200-card state; "
        f"observed {per_clone:.3f} ns",
        file=sys.stderr,
    )
    raise SystemExit(1)
print(f"PASS clone budget: {per_clone:.3f} ns per 200-card state")
PY

if [[ "$failures" -ne 0 ]]; then
  echo "FAIL gate_T1.sh ($failures issue(s))" >&2
  exit 1
fi

echo "PASS gate_T1.sh"
