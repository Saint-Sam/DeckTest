#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

require_json() {
  local file="$1"
  [[ -f "$file" ]] || {
    echo "ERROR: missing required T4 evidence: $file" >&2
    exit 1
  }
  jq empty "$file"
}

check_contracts() {
  scripts/verify_lfs_hydration.sh \
    reports/gates/T4.3/ai-baseline.frsreplay \
    reports/gates/T4.3/random-legal-baseline.frsreplay \
    reports/gates/T4.3/search-baseline.frsreplay
  require_json assets/ai/decision_surface.json
  require_json assets/ai/benchmark_splits.json
  require_json metrics/ai/decision_benchmark.json
  require_json metrics/ai/decision_state_audit.json
  require_json metrics/ai/game_length_diagnostics.json
  require_json metrics/ai/latency_cost.json
  require_json metrics/ai/arena_results.json
  require_json metrics/ai/search_budget_knee.json
  require_json metrics/learning/trace_integrity.json
  require_json reports/gates/CP-AI-BENCH/PREFLIGHT.json
  require_json schemas/learning/v1/game_manifest.schema.json
  require_json schemas/learning/v1/decision_record.schema.json
  require_json schemas/learning/v1/post_game_review.schema.json

  jq -e '
    (.gate_status == "blocked_incomplete_adapters" or .gate_status == "complete") and
    ([.families[].kind] | length == 16) and
    ([.families[].kind] | unique | length == 16)
  ' assets/ai/decision_surface.json >/dev/null
  jq -e '
    (.promotion_eligible | type == "boolean") and
    (.recorded_key_signature_consistency | type == "string") and
    (.near_state_dedup_audit | type == "string") and
    (.replay_family_leakage_audit | type == "string")
  ' metrics/ai/decision_benchmark.json >/dev/null
  jq -s -e '
    .[0].status == "passed" and
    .[0].recorded_key_signature_consistency == "passed" and
    .[0].near_state_dedup_audit == "not_run_runtime_isomorphism" and
    .[0].totals.decision_episodes > 0 and
    .[0].totals.strategic_decision_episodes > 0 and
    .[0].totals.forced_prompt_records > 0 and
    .[1].recorded_key_signature_consistency == "passed" and
    .[1].near_state_dedup_audit == "not_run_runtime_isomorphism" and
    all(.[1].runs[];
      .decision_episode_accounting.episode_linkage_complete == true and
      .decision_episode_accounting.raw_prompt_records == .decisions and
      .decision_episode_accounting.decision_episodes > 0 and
      .decision_episode_accounting.strategic_decision_episodes > 0 and
      .progress_diagnostics.termination_reason == "winner" and
      .progress_diagnostics.turn_cap_reached == false and
      (.progress_diagnostics.rounds | length) > 0
    ) and
    .[0].product_commit == .[1].product_commit and
    .[0].product_tree == .[1].product_tree
  ' metrics/ai/decision_state_audit.json metrics/ai/decision_benchmark.json >/dev/null
  local product_commit product_tree
  product_commit="$(jq -r '.product_commit' metrics/ai/decision_benchmark.json)"
  product_tree="$(jq -r '.product_tree' metrics/ai/decision_benchmark.json)"
  python3 tools/audit_t4_decision_keys.py \
    --check \
    --product-commit "$product_commit" \
    --product-tree "$product_tree" \
    --output metrics/ai/decision_state_audit.json \
    reports/gates/T4.3/ai-baseline.frsreplay \
    reports/gates/T4.3/random-legal-baseline.frsreplay \
    reports/gates/T4.3/search-baseline.frsreplay
  jq -e --arg commit "$product_commit" --arg tree "$product_tree" '
    .status == "diagnostic_complete" and
    .promotion_eligible == false and
    .product_commit == $commit and
    .product_tree == $tree and
    .aggregate.games == 3 and
    .aggregate.turn_cap_games == 0
  ' metrics/ai/game_length_diagnostics.json >/dev/null
  python3 tools/summarize_t4_long_games.py \
    --check \
    --product-commit "$product_commit" \
    --product-tree "$product_tree" \
    --output metrics/ai/game_length_diagnostics.json \
    reports/gates/T4.3/ai-baseline.frsreplay \
    reports/gates/T4.3/random-legal-baseline.frsreplay \
    reports/gates/T4.3/search-baseline.frsreplay
  jq -e '
    (.selected_budget_ms == null or .selected_budget_ms >= 0) and
    .cost_table_is_authoritative == false
  ' metrics/ai/search_budget_knee.json >/dev/null
}

run_local_preflight() {
  command -v jq >/dev/null
  check_contracts
  cargo fmt --all -- --check
  cargo clippy \
    -p forge-core -p forge-ai -p forge-game-runner -p forge-arena -p forge-testkit -p forge-cli \
    --all-targets --all-features --locked --offline -- -D warnings
  cargo test \
    -p forge-core -p forge-ai -p forge-game-runner -p forge-arena -p forge-testkit -p forge-cli \
    --locked --offline --no-fail-fast
  target/release/forge-cli replay reports/gates/T4.3/ai-baseline.frsreplay
  target/release/forge-cli replay reports/gates/T4.3/random-legal-baseline.frsreplay
  target/release/forge-cli replay reports/gates/T4.3/search-baseline.frsreplay
  cargo check -p forge-ai --target wasm32-unknown-unknown --locked --offline
  cargo check -p forge-ai --target aarch64-linux-android --locked --offline
  echo "PASS gate_T4.sh --preflight (diagnostics only; no promotion claim)"
}

case "${1:-}" in
  --self-test)
    command -v jq >/dev/null
    check_contracts
    echo "PASS gate_T4.sh self-test"
    exit 0
    ;;
  --preflight)
    run_local_preflight
    exit 0
    ;;
  "")
    run_local_preflight
    ;;
  *)
    echo "usage: scripts/gates/gate_T4.sh [--self-test|--preflight]" >&2
    exit 2
    ;;
esac

jq -e '.gate_status == "complete" and all(.families[]; .human == "complete" and .ai == "complete" and .benchmark == "complete")' \
  assets/ai/decision_surface.json >/dev/null || {
  echo "ERROR: T4 promotion blocked: canonical human/AI/benchmark adapters are incomplete" >&2
  exit 1
}
jq -e '.status == "passed" and .promotion_eligible == true' \
  reports/gates/CP-AI-BENCH/PREFLIGHT.json >/dev/null || {
  echo "ERROR: T4 promotion blocked: CP-AI-BENCH has not passed" >&2
  exit 1
}
jq -e '.promotion_eligible == true and .development_campaign == "passed" and .validation_campaign == "passed" and .sealed_campaign == "passed"' \
  metrics/ai/arena_results.json >/dev/null || {
  echo "ERROR: T4 promotion blocked: paired arena splits are incomplete" >&2
  exit 1
}
jq -e '.selected_budget_ms != null' metrics/ai/search_budget_knee.json >/dev/null || {
  echo "ERROR: T4 promotion blocked: no accepted search knee exists" >&2
  exit 1
}
jq -e '.cross_target_compile_evidence.reference_device_latency_measured == true' \
  metrics/ai/latency_cost.json >/dev/null || {
  echo "ERROR: T4 promotion blocked: reference-device latency is missing" >&2
  exit 1
}

echo "PASS gate_T4.sh"
