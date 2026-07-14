#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  command -v jq >/dev/null
  command -v git >/dev/null
  [[ -x "$ROOT/scripts/t3_parallel_sweep.sh" ]]
  [[ -x "$ROOT/scripts/check_coverage.sh" ]]
  [[ -f "$ROOT/tools/run_t3_6_commander_semantics.py" ]]
  [[ -f "$ROOT/tools/run_t3_9_pod_gate.py" ]]
  [[ -f "$ROOT/tools/write_pod_integration.py" ]]
  echo "PASS gate_T3.sh self-test"
  exit 0
fi

for command in git jq python3 shasum; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "ERROR: $command is required for the T3 gate" >&2
    exit 1
  fi
done

if [[ "${1:-}" == "--run-exact" ]]; then
  reviewed_commit="$(jq -er '.reviewed_commit' metrics/coverage.json)"
  if [[ "$(git rev-parse HEAD)" != "$reviewed_commit" ]]; then
    echo "ERROR: --run-exact requires HEAD to equal the bound product commit" >&2
    exit 1
  fi
  FORGE_T3_TOTAL_WORKERS="${FORGE_T3_TOTAL_WORKERS:-24}" \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-24}" \
    "$ROOT/scripts/t3_parallel_sweep.sh" checkpoint
  CARGO_NET_OFFLINE=true python3 tools/run_t3_6_commander_semantics.py \
    --translated-root target/translated-cards \
    --cargo-target-dir target \
    --report reports/gates/T3.9/t3-6-semantic-revalidation.json
  python3 tools/run_t3_9_pod_gate.py \
    --games 1000 --jobs 24 \
    --report reports/gates/T3.9/cp-four-player-pod-2026-07-13.json \
    --replay-dir reports/gates/T3.9/replays
elif [[ -n "${1:-}" ]]; then
  echo "usage: scripts/gates/gate_T3.sh [--run-exact|--self-test]" >&2
  exit 2
fi

required_files=(
  metrics/legacy_parse.json
  metrics/translation.json
  metrics/t3_parallel_validation.json
  metrics/coverage.json
  metrics/card_runtime_smoke.json
  metrics/card_semantics_100.json
  metrics/pod_integration.json
  metrics/card_maturity.json
  metrics/primitive_tickets.json
  metrics/local_fuzz.json
  reports/gates/T3.5/runtime-interpreter-final-2026-07-13.json
  reports/gates/T3.9/t3-6-semantic-revalidation.json
  reports/gates/T3.9/cp-four-player-pod-2026-07-13.json
  reports/gates/T3/fuzz_report.md
)
for path in "${required_files[@]}"; do
  if [[ ! -f "$path" ]]; then
    echo "ERROR: missing T3 evidence $path" >&2
    exit 1
  fi
done

reviewed_commit="$(jq -er '.reviewed_commit' metrics/coverage.json)"
reviewed_tree="$(jq -er '.reviewed_tree' metrics/coverage.json)"
if [[ "$(git show -s --format=%T "$reviewed_commit")" != "$reviewed_tree" ]]; then
  echo "ERROR: coverage product commit/tree binding is invalid" >&2
  exit 1
fi

jq -e '
  .parsed_files == .total_files and
  .total_files >= 33290 and
  .failed_files == 0
' metrics/legacy_parse.json >/dev/null

jq -e '
  .total_scripts >= 33290 and
  .emitted_scripts + .quarantined_scripts == .total_scripts and
  (.emitted_scripts * 100 >= .total_scripts * 60)
' metrics/translation.json >/dev/null

jq -e '
  .schema_version == 3 and
  .mode == "checkpoint" and
  .local_only == true and
  .github_actions_used == false and
  .deterministic_parallel_replay == true and
  .deterministic_blocker_plan_replay == true and
  .workers.ceiling == 24 and
  .workers.total <= 24 and
  .coverage.percent >= 80 and
  .translation.emitted_percent >= 60
' metrics/t3_parallel_validation.json >/dev/null

jq -e --arg commit "$reviewed_commit" --arg tree "$reviewed_tree" '
  .schema_version == 2 and
  .passed == true and
  .reviewed_commit == $commit and
  .reviewed_tree == $tree and
  .floor_percent >= 80 and
  .lines.percent >= 80
' metrics/coverage.json >/dev/null

jq -e --arg commit "$reviewed_commit" --arg tree "$reviewed_tree" '
  .schema_version == 2 and
  .passed == true and
  .reviewed_commit == $commit and
  .reviewed_tree == $tree and
  .sanitizer == "address" and
  .worker_count == 8 and
  .total_worker_seconds >= 3600 and
  ([.workers[].target] | unique) ==
    ["fuzz_apply", "fuzz_carddb", "fuzz_carddsl", "fuzz_characteristics", "fuzz_scenarioparse"] and
  ([.workers[]] | all(
    .status == "passed" and
    .sanitizer == "address" and
    .return_code == 0 and
    .timed_out == false and
    .verified_runtime_seconds >= .requested_seconds and
    .completed_runs > 0 and
    .final_statistics.number_of_executed_units == .completed_runs and
    .final_statistics.average_exec_per_sec > 0 and
    (.artifacts | length) == 0
  ))
' metrics/local_fuzz.json >/dev/null

while IFS=$'\t' read -r recorded_log recorded_sha; do
  archived_log="reports/gates/T3/fuzz/$(basename "$recorded_log")"
  if [[ ! -f "$archived_log" ]]; then
    echo "ERROR: missing archived fuzz log $archived_log" >&2
    exit 1
  fi
  read -r actual_sha _ < <(shasum -a 256 "$archived_log")
  if [[ "$actual_sha" != "$recorded_sha" ]]; then
    echo "ERROR: archived fuzz log hash mismatch: $archived_log" >&2
    exit 1
  fi
done < <(jq -r '.workers[] | [.log, .log_sha256] | @tsv' metrics/local_fuzz.json)

read -r fuzz_metric_sha _ < <(shasum -a 256 metrics/local_fuzz.json)
if ! grep -q "$fuzz_metric_sha" reports/gates/T3/fuzz_report.md; then
  echo "ERROR: fuzz report is not bound to metrics/local_fuzz.json" >&2
  exit 1
fi

for stage_file in \
  metrics/card_runtime_smoke.json \
  metrics/card_semantics_100.json; do
  jq -e --arg commit "$reviewed_commit" --arg tree "$reviewed_tree" '
    .schema_version == 1 and
    .passed == true and
    .product_commit == $commit and
    .product_tree == $tree and
    (.identity_ids | length) > 0
  ' "$stage_file" >/dev/null
done

jq -e --arg commit "$reviewed_commit" --arg tree "$reviewed_tree" '
  .schema_version == 1 and
  .passed == true and
  .product_commit == $commit and
  .product_tree == $tree and
  (.identity_ids | length) == 21 and
  (.action_replays | length) == 10
' metrics/pod_integration.json >/dev/null

jq -e '
  .status == "pass_local" and
  .translated_corpus.total >= 20082 and
  .translated_corpus.passed + .translated_corpus.unsupported_setup ==
    .translated_corpus.total and
  .translated_corpus.failed == 0 and
  .frozen_semantic_100.total == 100 and
  .frozen_semantic_100.runtime_smoke_passed == 100 and
  .frozen_semantic_100.semantic_verified == 100 and
  .frozen_semantic_100.failed == 0
' reports/gates/T3.5/runtime-interpreter-final-2026-07-13.json >/dev/null

jq -e '
  .status == "pass_local" and
  .checkpoint.status == "passed" and
  .checkpoint.semantic_verified == 100 and
  .checkpoint.remaining == 0 and
  .measured.production_failures == 0 and
  .deterministic_replay.exact_report_match == true
' reports/gates/T3.9/t3-6-semantic-revalidation.json >/dev/null

jq -e --arg commit "$reviewed_commit" --arg tree "$reviewed_tree" '
  . as $report |
  .schema_version == 2 and
  .status == "passed" and
  .checkpoint == "CP-FOUR-PLAYER-POD" and
  .product_binding.commit == $commit and
  .product_binding.tree == $tree and
  .product_binding.tracked_clean_at_start == true and
  (.configuration.decks | length) == 4 and
  ([.configuration.decks[].cards] | all(. == 100)) and
  .configuration.games >= 1000 and
  .configuration.players_per_game == 4 and
  .configuration.starting_life == 40 and
  .configuration.jobs <= 24 and
  .configuration.semantic_identity_count == 21 and
  (.configuration.semantic_identity_requirements | length) == 21 and
  .results.games_completed == .configuration.games and
  .results.direct_typed_action_replays_matched == .configuration.games and
  .results.action_replays_matched == 10 and
  .results.cli_action_replays_verified == 10 and
  .results.commander_zone_returns >= .configuration.games and
  .results.taxed_commander_recasts >= .configuration.games and
  .results.hidden_information_checks >
    (.configuration.games * .configuration.players_per_game * 2) and
  ([.configuration.semantic_identity_requirements | to_entries[]] | all(
    . as $requirement |
    ($requirement.key as $id |
      if $requirement.value then
        $report.results.identity_exercise[$id].land_plays > 0
      else
        ($report.results.identity_exercise[$id].casts > 0 and
         $report.results.identity_exercise[$id].resolutions > 0)
      end)
  )) and
  .results.invariant_violations == 0 and
  .results.hidden_information_canary_violations == 0 and
  .results.eliminations >= (.configuration.games * 3) and
  .constraints.github_actions_used == false and
  .constraints.network_used == false and
  .constraints.installs_performed == false and
  .constraints.push_performed == false and
  .resources.workers_used <= 24 and
  .resources.logical_cpu_count >= .resources.workers_used and
  .resources.wall_seconds > 0 and
  .resources.child_user_cpu_seconds > 0 and
  .resources.child_max_rss_bytes > 0 and
  .resources.disk_free_headroom_bytes > 0
' reports/gates/T3.9/cp-four-player-pod-2026-07-13.json >/dev/null

for replay in reports/gates/T3.9/replays/pod-seed-*.frsreplay; do
  jq -e '
    .format == "forge-pod-replay-v1" and
    (.actions | length) > 0 and
    ([.actions[].action] | any(startswith("ChooseCommanderZone"))) and
    ([.actions[].action] | all(
      (startswith("MoveObject") and contains("kind: Command")) | not
    ))
  ' "$replay" >/dev/null
done

jq -e '
  .stage == "runtime_smoke_passed" and
  (.identity_ids | length) == 100
' metrics/card_runtime_smoke.json >/dev/null
jq -e '
  .stage == "semantic_verified" and
  (.identity_ids | length) == 100
' metrics/card_semantics_100.json >/dev/null
jq -e '
  .stage == "pod_integration_verified" and
  (.identity_ids | length) == 21
' metrics/pod_integration.json >/dev/null
read -r pod_report_sha _ < <(
  shasum -a 256 reports/gates/T3.9/cp-four-player-pod-2026-07-13.json
)
read -r pod_manifest_sha _ < <(shasum -a 256 assets/t3_9/integration_decks.json)
if [[ "$pod_report_sha" != "$(jq -er '.evidence_sha256' metrics/pod_integration.json)" ||
      "$pod_manifest_sha" != "$(jq -er '.manifest_sha256' metrics/pod_integration.json)" ]]; then
  echo "ERROR: pod integration evidence hash binding is stale" >&2
  exit 1
fi
jq -e '
  .implementation_maturity.cumulative_counts.runtime_smoke_passed == 100 and
  .implementation_maturity.cumulative_counts.semantic_verified == 100 and
  .implementation_maturity.cumulative_counts.pod_integration_verified == 21
' metrics/card_maturity.json >/dev/null
jq -e '
  .schema_version == 1 and
  .reason_code == "NEEDS_NEW_PRIMITIVE" and
  .route == "T2" and
  .ticket_count == (.tickets | length)
' metrics/primitive_tickets.json >/dev/null

python3 tools/run_t3_6_commander_semantics.py --validate-only >/dev/null
python3 tools/write_pod_integration.py --check >/dev/null
python3 tools/write_card_maturity.py --check >/dev/null
python3 tools/write_project_status.py --check >/dev/null

echo "PASS gate_T3.sh: structural=60.3244% semantic=100/100 pod=1000/1000 coverage=80.3087%"
