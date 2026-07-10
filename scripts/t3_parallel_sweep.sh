#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -f "$ROOT/tools/write_t3_parallel_metrics.py" ]]
  python3 "$ROOT/tools/write_t3_parallel_metrics.py" --self-test
  echo "PASS t3_parallel_sweep.sh self-test"
  exit 0
fi

mode="${1:-development}"
if [[ "$mode" != "development" && "$mode" != "checkpoint" ]]; then
  echo "usage: scripts/t3_parallel_sweep.sh [development|checkpoint|--self-test]" >&2
  exit 2
fi

for command in cargo cmp jq python3; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "ERROR: $command is required for the T3 parallel sweep" >&2
    exit 1
  fi
done

total_workers="${FORGE_T3_TOTAL_WORKERS:-}"
if [[ -z "$total_workers" ]] && command -v sysctl >/dev/null 2>&1; then
  total_workers="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
fi
if ! [[ "$total_workers" =~ ^[1-9][0-9]*$ ]] && command -v getconf >/dev/null 2>&1; then
  total_workers="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
fi
if ! [[ "$total_workers" =~ ^[1-9][0-9]*$ ]]; then
  total_workers=8
fi
if ((total_workers < 6)); then
  echo "ERROR: T3 parallel sweeps require at least 6 workers" >&2
  exit 1
fi

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$total_workers}"
export CARGO_NET_OFFLINE=true

run_dir="target/t3-parallel"
mkdir -p "$run_dir"
active_pids=()

terminate_jobs() {
  local pid
  for pid in "${active_pids[@]:-}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done
}
trap terminate_jobs EXIT INT TERM

wait_checked() {
  local pid="$1"
  local label="$2"
  local log="$3"
  local status=0
  set +e
  wait "$pid"
  status=$?
  set -e
  if ((status != 0)); then
    echo "ERROR: $label failed; log follows" >&2
    cat "$log" >&2
    return "$status"
  fi
}

compare_translation_reports() {
  local primary="$1"
  local replay="$2"
  local primary_normalized="$run_dir/translation-primary-normalized.json"
  local replay_normalized="$run_dir/translation-replay-normalized.json"
  jq -S 'del(.jobs)' "$primary" >"$primary_normalized"
  jq -S 'del(.jobs)' "$replay" >"$replay_normalized"
  if ! cmp -s "$primary_normalized" "$replay_normalized"; then
    echo "ERROR: deterministic translation reports differ" >&2
    return 1
  fi
}

total_start="$SECONDS"
cargo build --locked --offline --quiet \
  -p forge-porttools -p forge-cardc -p forge-cards

porttools="target/debug/forge-porttools"
cardc="target/debug/forge-cardc"
cards="target/debug/forge-cards"
root="vendor/legacy-forge/forge-gui/res/cardsfolder"
catalog="assets/card_catalog.json"
priority="assets/coverage_priority.txt"
primary_output="target/translated-cards"
primary_metrics="metrics/translation.json"
primary_quarantine="metrics/translation_quarantine.json"
primary_priority="metrics/priority_coverage.json"
blocker_metrics="metrics/blocker_plan.json"
blocker_details="$run_dir/blocker-cards.json"
api_metrics="metrics/api_coverage.json"
api_quarantine="metrics/api_quarantine.json"
database="$run_dir/translated-carddb.bin"

deterministic=false
verification_seconds=0
parallel_start="$SECONDS"

if [[ "$mode" == "development" ]]; then
  primary_translation_workers=$((total_workers * 2 / 3))
  replay_translation_workers=0
  audit_workers=2
  planner_workers=$((total_workers - primary_translation_workers - audit_workers))
  if ((planner_workers < 1)); then
    planner_workers=1
  fi

  "$porttools" translate --all --jobs "$primary_translation_workers" \
    --root "$root" --catalog "$catalog" --output "$primary_output" \
    --metrics "$primary_metrics" --quarantine "$primary_quarantine" \
    --priority "$priority" --priority-metrics "$primary_priority" \
    >"$run_dir/translate-primary.log" 2>&1 &
  translate_pid=$!
  active_pids+=("$translate_pid")

  "$porttools" legacy blocker-plan --jobs "$planner_workers" \
    --root "$root" --priority "$priority" --output "$blocker_metrics" \
    --details "$blocker_details" --batch-size 5 --batch-count 6 \
    >"$run_dir/blocker-plan.log" 2>&1 &
  planner_pid=$!
  active_pids+=("$planner_pid")

  "$porttools" legacy map-audit --root "$root" \
    --metrics "$api_metrics" --quarantine "$api_quarantine" \
    >"$run_dir/map-audit.log" 2>&1 &
  audit_pid=$!
  active_pids+=("$audit_pid")

  cargo test -p forge-porttools --quiet \
    >"$run_dir/focused-tests.log" 2>&1 &
  focused_test_pid=$!
  active_pids+=("$focused_test_pid")

  wait_checked "$translate_pid" "translation sweep" "$run_dir/translate-primary.log"
  wait_checked "$planner_pid" "blocker planner" "$run_dir/blocker-plan.log"
  wait_checked "$audit_pid" "mapping audit" "$run_dir/map-audit.log"
  wait_checked "$focused_test_pid" "focused porttools tests" "$run_dir/focused-tests.log"
else
  primary_translation_workers=$total_workers
  replay_translation_workers=$((total_workers / 2))
  planner_workers=$((total_workers / 4))
  audit_workers=2
  secondary_output="$run_dir/translated-cards-secondary"
  secondary_metrics="$run_dir/translation-secondary.json"
  secondary_quarantine="$run_dir/quarantine-secondary.json"
  secondary_priority="$run_dir/priority-secondary.json"

  "$porttools" translate --all --jobs "$primary_translation_workers" \
    --root "$root" --catalog "$catalog" --output "$primary_output" \
    --metrics "$primary_metrics" --quarantine "$primary_quarantine" \
    --priority "$priority" --priority-metrics "$primary_priority" \
    >"$run_dir/translate-primary.log" 2>&1 &
  primary_pid=$!
  active_pids+=("$primary_pid")
  wait_checked "$primary_pid" "primary deterministic sweep" "$run_dir/translate-primary.log"

  "$porttools" translate --all --jobs "$replay_translation_workers" \
    --root "$root" --catalog "$catalog" --output "$secondary_output" \
    --metrics "$secondary_metrics" --quarantine "$secondary_quarantine" \
    --priority "$priority" --priority-metrics "$secondary_priority" \
    --write-output false \
    >"$run_dir/translate-secondary.log" 2>&1 &
  secondary_pid=$!
  active_pids+=("$secondary_pid")

  (
    "$cardc" build "$primary_output" --catalog "$catalog" -o "$database"
    "$cards" validate "$database"
  ) >"$run_dir/compiler.log" 2>&1 &
  compiler_pid=$!
  active_pids+=("$compiler_pid")

  "$porttools" legacy blocker-plan --jobs "$planner_workers" \
    --root "$root" --priority "$priority" --output "$blocker_metrics" \
    --details "$blocker_details" --batch-size 5 --batch-count 6 \
    >"$run_dir/blocker-plan.log" 2>&1 &
  planner_pid=$!
  active_pids+=("$planner_pid")

  "$porttools" legacy map-audit --root "$root" \
    --metrics "$api_metrics" --quarantine "$api_quarantine" \
    >"$run_dir/map-audit.log" 2>&1 &
  audit_pid=$!
  active_pids+=("$audit_pid")

  wait_checked "$secondary_pid" "secondary deterministic sweep" "$run_dir/translate-secondary.log"
  wait_checked "$compiler_pid" "translated-card compiler" "$run_dir/compiler.log"
  wait_checked "$planner_pid" "blocker planner" "$run_dir/blocker-plan.log"
  wait_checked "$audit_pid" "mapping audit" "$run_dir/map-audit.log"
  compare_translation_reports "$primary_metrics" "$secondary_metrics"
  cmp -s "$primary_quarantine" "$secondary_quarantine"
  cmp -s "$primary_priority" "$secondary_priority"
  deterministic=true
fi

if [[ "$mode" == "development" ]]; then
  "$cardc" build "$primary_output" --catalog "$catalog" -o "$database" \
    >"$run_dir/compiler.log" 2>&1
  "$cards" validate "$database" >>"$run_dir/compiler.log" 2>&1
fi

parallel_seconds=$((SECONDS - parallel_start))

if [[ "$mode" == "checkpoint" ]]; then
  verification_start="$SECONDS"
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --quiet
  scripts/check_coverage.sh 80
  verification_seconds=$((SECONDS - verification_start))
fi

total_seconds=$((SECONDS - total_start))
baseline_seconds="${FORGE_T3_SEQUENTIAL_BASELINE_SECONDS:-0}"
metrics_args=(
  --mode "$mode" \
  --total-workers "$total_workers" \
  --primary-translation-workers "$primary_translation_workers" \
  --replay-translation-workers "$replay_translation_workers" \
  --planner-workers "$planner_workers" \
  --audit-workers "$audit_workers" \
  --parallel-phase-seconds "$parallel_seconds" \
  --verification-seconds "$verification_seconds" \
  --total-seconds "$total_seconds" \
  --sequential-baseline-seconds "$baseline_seconds" \
  --deterministic "$deterministic" \
  --translation "$primary_metrics" \
  --blocker-plan "$blocker_metrics" \
  --output metrics/t3_parallel_validation.json
)
if [[ "$mode" == "checkpoint" ]]; then
  metrics_args+=(--coverage metrics/coverage.json)
fi
python3 tools/write_t3_parallel_metrics.py "${metrics_args[@]}"

active_pids=()
trap - EXIT INT TERM
echo "PASS T3 $mode sweep: workers=$total_workers parallel_phase=${parallel_seconds}s total=${total_seconds}s deterministic=$deterministic"
