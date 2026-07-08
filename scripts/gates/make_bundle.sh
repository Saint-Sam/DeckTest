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
evidence_dir="reports/gates/${gate_id}"
mkdir -p "$bundle"

cp PLAN_STATE.json "$bundle/PLAN_STATE.json"
if [[ -f "${evidence_dir}/SIGNOFF.md" ]]; then
  cp "${evidence_dir}/SIGNOFF.md" "$bundle/SIGNOFF.md"
fi

if [[ -f "${evidence_dir}/metrics_snapshot.json" ]]; then
  cp "${evidence_dir}/metrics_snapshot.json" "$bundle/metrics_snapshot.json"
elif [[ -f "metrics/legacy_inventory.json" ]]; then
  cp "metrics/legacy_inventory.json" "$bundle/metrics_snapshot.json"
elif [[ -f "metrics/metrics.json" ]]; then
  cp "metrics/metrics.json" "$bundle/metrics_snapshot.json"
else
  printf '{\n  "status": "no metrics yet"\n}\n' > "$bundle/metrics_snapshot.json"
fi

for artifact in test_log.txt tests_added.txt fuzz_report.md quarantine_report.md divergences.md; do
  if [[ -f "${evidence_dir}/${artifact}" ]]; then
    cp "${evidence_dir}/${artifact}" "$bundle/${artifact}"
  fi
done

if find "$evidence_dir" -maxdepth 1 -type f -name '*.log' -print -quit 2>/dev/null | grep -q .; then
  mkdir -p "$bundle/logs"
  while IFS= read -r -d '' log_file; do
    cp "$log_file" "$bundle/logs/$(basename "$log_file")"
  done < <(find "$evidence_dir" -maxdepth 1 -type f -name '*.log' -print0 | sort -z)
fi

if find "$evidence_dir" -maxdepth 1 -type f -name '*.csv' -print -quit 2>/dev/null | grep -q .; then
  mkdir -p "$bundle/evidence"
  while IFS= read -r -d '' csv_file; do
    cp "$csv_file" "$bundle/evidence/$(basename "$csv_file")"
  done < <(find "$evidence_dir" -maxdepth 1 -type f -name '*.csv' -print0 | sort -z)
fi

if [[ -d "${evidence_dir}/replays" ]]; then
  mkdir -p "$bundle/replays"
  cp -R "${evidence_dir}/replays/." "$bundle/replays/"
fi

if [[ -d "${evidence_dir}/reviewer_oracles" ]]; then
  mkdir -p "$bundle/reviewer_oracles"
  cp -R "${evidence_dir}/reviewer_oracles/." "$bundle/reviewer_oracles/"
fi

if [[ -f "metrics/coverage.json" ]]; then
  cp "metrics/coverage.json" "$bundle/coverage.json"
fi

if [[ -f "metrics/clone_surface.json" ]]; then
  cp "metrics/clone_surface.json" "$bundle/clone_surface.json"
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

if find "$evidence_dir" -maxdepth 1 -type f -name '*.md' ! -name 'SIGNOFF.md' -print -quit 2>/dev/null | grep -q .; then
  mkdir -p "$bundle/evidence"
  while IFS= read -r -d '' evidence_file; do
    cp "$evidence_file" "$bundle/evidence/$(basename "$evidence_file")"
  done < <(find "$evidence_dir" -maxdepth 1 -type f -name '*.md' ! -name 'SIGNOFF.md' -print0 | sort -z)
fi

echo "WROTE $bundle"
