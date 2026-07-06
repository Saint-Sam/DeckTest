#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS make_bundle.sh self-test"
  exit 0
fi

gate_id="${1:-}"
if [[ -z "$gate_id" ]]; then
  echo "ERROR: usage: scripts/gates/make_bundle.sh T<k>" >&2
  exit 2
fi

bundle="reports/gates/${gate_id}/bundle"
mkdir -p "$bundle"

cp PLAN_STATE.json "$bundle/PLAN_STATE.json"
if [[ -f "metrics/legacy_inventory.json" ]]; then
  cp "metrics/legacy_inventory.json" "$bundle/metrics_snapshot.json"
elif [[ -f "metrics/metrics.json" ]]; then
  cp "metrics/metrics.json" "$bundle/metrics_snapshot.json"
else
  printf '{\n  "status": "no metrics yet"\n}\n' > "$bundle/metrics_snapshot.json"
fi

{
  echo "# Open Questions"
  echo
  if [[ -f reports/questions/QUEUE.md ]]; then
    cat reports/questions/QUEUE.md
  else
    echo "Question queue not found."
  fi
} > "$bundle/questions_open.md"

{
  echo "# Blockers History"
  echo
  if find reports/blockers -type f -name '*.md' -print -quit 2>/dev/null | grep -q .; then
    find reports/blockers -type f -name '*.md' -print | sort
  else
    echo "No blockers filed."
  fi
} > "$bundle/blockers_history.md"

echo "WROTE $bundle"
