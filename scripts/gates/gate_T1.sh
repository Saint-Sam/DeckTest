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

python3 - <<'PY' || failures=$((failures + 1))
from pathlib import Path

paths = sorted(Path("tests/oracle").rglob("*.ron"))
combat = []
for path in paths:
    text = path.read_text(encoding="utf-8")
    if (
        'action: "declare_attackers"' in text
        and 'action: "declare_blockers"' in text
        and 'action: "assign_combat_damage"' in text
    ):
        combat.append((path, text))

if len(combat) < 60:
    raise SystemExit(
        f"ERROR: T1 gate requires >=60 combat oracle scenarios; found {len(combat)}"
    )

corpus = "\n".join(text for _, text in combat)
missing = []
for feature in [
    '"first_strike"',
    '"double_strike"',
    '"trample"',
    '"deathtouch"',
    '"lifelink"',
    '"flying"',
    '"reach"',
    '"menace"',
    '"vigilance"',
]:
    if feature not in corpus:
        missing.append(feature.strip('"'))
if not any("double-block ordering" in text for _, text in combat):
    missing.append("double-block ordering")
if not any('"trample"' in text and '"deathtouch"' in text for _, text in combat):
    missing.append("trample+deathtouch")
if missing:
    raise SystemExit(
        "ERROR: T1 combat oracle surface missing " + ", ".join(sorted(missing))
    )

print(
    "PASS combat oracle surface: "
    f"{len(combat)} scenario(s) cover T1.6 combat feature requirements"
)
PY

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
