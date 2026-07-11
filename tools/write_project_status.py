#!/usr/bin/env python3
"""Generate the current project status metric from repository sources."""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load_json(path: Path) -> dict[str, object]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"expected object in {path}")
    return value


def generate() -> dict[str, object]:
    state = load_json(ROOT / "PLAN_STATE.json")
    legacy = load_json(ROOT / "metrics" / "legacy_inventory.json")
    scryfall = load_json(ROOT / "metrics" / "scryfall_cache_summary.json")
    catalog = load_json(ROOT / "metrics" / "card_catalog.json")
    corpus = load_json(ROOT / "metrics" / "cp_dsl_corpus.json")
    mutation = load_json(ROOT / "metrics" / "cp_dsl_mutation.json")
    local_fuzz = load_json(ROOT / "metrics" / "local_fuzz.json")
    local_platforms = load_json(ROOT / "metrics" / "local_platforms.json")
    oracle_semantics = load_json(ROOT / "metrics" / "oracle_semantics.json")
    translation = load_json(ROOT / "metrics" / "translation.json")
    priority = load_json(ROOT / "metrics" / "priority_coverage.json")
    blocker_plan = load_json(ROOT / "metrics" / "blocker_plan.json")
    api_coverage = load_json(ROOT / "metrics" / "api_coverage.json")
    parallel_validation = load_json(ROOT / "metrics" / "t3_parallel_validation.json")
    card_maturity = load_json(ROOT / "metrics" / "card_maturity.json")
    tasks = state.get("tasks", {})
    if not isinstance(tasks, dict):
        raise ValueError("PLAN_STATE tasks must be an object")
    task_statuses: dict[str, int] = {}
    for task in tasks.values():
        if not isinstance(task, dict):
            continue
        status = str(task.get("status", "unknown"))
        task_statuses[status] = task_statuses.get(status, 0) + 1

    source = catalog.get("source", {})
    if not isinstance(source, dict):
        raise ValueError("card catalog source must be an object")
    oracle_measured = oracle_semantics.get("measured", {})
    if not isinstance(oracle_measured, dict):
        raise ValueError("oracle semantic measurements must be an object")
    validation_durations = parallel_validation.get("durations_seconds", {})
    if not isinstance(validation_durations, dict):
        raise ValueError("T3 validation durations must be an object")
    maturity = card_maturity.get("implementation_maturity", {})
    if not isinstance(maturity, dict):
        raise ValueError("card maturity implementation_maturity must be an object")
    maturity_counts = maturity.get("cumulative_counts", {})
    if not isinstance(maturity_counts, dict):
        raise ValueError("card maturity cumulative_counts must be an object")
    scope = card_maturity.get("scope", {})
    if not isinstance(scope, dict):
        raise ValueError("card maturity scope must be an object")
    scope_counts = scope.get("counts", {})
    if not isinstance(scope_counts, dict):
        raise ValueError("card maturity scope counts must be an object")
    hosted_workflows = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
    return {
        "schema_version": 4,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "generator": "tools/write_project_status.py",
        "verification_mode": "local_only",
        "hosted_workflow_count": len(hosted_workflows),
        "tier": state.get("tier"),
        "gates_passed": state.get("gates_passed", []),
        "task_count": len(tasks),
        "task_status_counts": dict(sorted(task_statuses.items())),
        "scenario_evidence_passed": oracle_semantics.get("passed"),
        "scenario_file_count": oracle_measured.get("raw_scenarios"),
        "structural_scenario_family_count": oracle_measured.get("structural_families"),
        "observed_semantic_atom_combinations": oracle_measured.get(
            "rule_interactions"
        ),
        "hand_authored_scenario_count": oracle_measured.get(
            "hand_authored_scenarios"
        ),
        "distinct_scenario_commands": oracle_measured.get("distinct_actions"),
        "distinct_scenario_operations": oracle_measured.get("distinct_operations"),
        "legacy_script_count": legacy.get("total_scripts"),
        "compiler_valid_translated_definitions": translation.get("emitted_scripts"),
        "structurally_tested_legacy_ability_uses": api_coverage.get("mapped_uses"),
        "total_legacy_ability_uses": api_coverage.get("legacy_uses"),
        "compiler_valid_translated_legacy_definitions_percent": translation.get(
            "emitted_percent"
        ),
        "owner_priority_compiler_valid_translated_definitions": priority.get(
            "emitted"
        ),
        "owner_priority_card_names_requested": priority.get("total_requested"),
        "blocker_families_confirmed": blocker_plan.get("unique_blocker_families"),
        "blocker_observations_confirmed": blocker_plan.get("confirmed_observations"),
        "t3_translation_deterministic_replay_passed": parallel_validation.get(
            "deterministic_parallel_replay"
        ),
        "t3_blocker_plan_deterministic_replay_passed": parallel_validation.get(
            "deterministic_blocker_plan_replay"
        ),
        "t3_accelerated_core_seconds": validation_durations.get(
            "accelerated_core_phase"
        ),
        "t3_serial_core_baseline_seconds": validation_durations.get(
            "comparable_serial_core_baseline"
        ),
        "t3_core_validation_saved_percent": validation_durations.get(
            "core_saved_percent"
        ),
        "catalog_total_records": catalog.get("source_records"),
        "catalog_english_printings": catalog.get("imported_english_printings"),
        "catalog_classified_identities": catalog.get("classified_identities"),
        "catalog_dangling_references": catalog.get("dangling_printing_references"),
        "catalog_unique_english_oracle_ids": catalog.get("source_unique_english_oracle_ids"),
        "catalog_unique_english_names": scryfall.get("unique_english_names"),
        "in_v1_scope_oracle_identities": scope_counts.get("in_v1_scope"),
        "compiler_valid_oracle_identities": maturity_counts.get("compiler_valid"),
        "runtime_smoke_passed_oracle_identities": maturity_counts.get(
            "runtime_smoke_passed"
        ),
        "semantic_verified_oracle_identities": maturity_counts.get(
            "semantic_verified"
        ),
        "pod_integration_verified_oracle_identities": maturity_counts.get(
            "pod_integration_verified"
        ),
        "ai_supported_oracle_identities": maturity_counts.get("ai_supported"),
        "product_eligible_oracle_identities": maturity_counts.get(
            "product_eligible"
        ),
        "catalog_source_updated_at": source.get("source_updated_at"),
        "catalog_source_sha256": source.get("source_sha256"),
        "catalog_source_path": source.get("source_path"),
        "cp_dsl_language_stress_definition_count": corpus.get(
            "reviewed_card_count"
        ),
        "cp_dsl_language_stress_primary_strata": corpus.get(
            "distinct_primary_strata"
        ),
        "cp_dsl_distinct_operations": corpus.get("distinct_operations"),
        "cp_dsl_mutation_score_percent": mutation.get("mutation_score_percent"),
        "cp_dsl_mutation_survivors": mutation.get("survived"),
        "local_fuzz_passed": local_fuzz.get("passed"),
        "local_fuzz_worker_seconds": local_fuzz.get("total_worker_seconds"),
        "cross_compile_artifacts_all_passed": local_platforms.get("passed"),
        "cross_compile_artifacts_passed": len(local_platforms.get("targets", [])),
        "deprecated_aliases": {
            "cp_dsl_primary_strata": corpus.get("distinct_primary_strata"),
            "cp_dsl_reviewed_cards": corpus.get("reviewed_card_count"),
            "legacy_translation_percent": translation.get("emitted_percent"),
            "legacy_translated_scripts": translation.get("emitted_scripts"),
            "local_platform_target_count": len(local_platforms.get("targets", [])),
            "local_platforms_passed": local_platforms.get("passed"),
            "oracle_distinct_action_count": oracle_measured.get("distinct_actions"),
            "oracle_distinct_operation_count": oracle_measured.get(
                "distinct_operations"
            ),
            "oracle_file_count": oracle_measured.get("raw_scenarios"),
            "oracle_hand_authored_count": oracle_measured.get(
                "hand_authored_scenarios"
            ),
            "oracle_metrics_passed": oracle_semantics.get("passed"),
            "oracle_rule_interaction_count": oracle_measured.get(
                "rule_interactions"
            ),
            "oracle_structural_family_count": oracle_measured.get(
                "structural_families"
            ),
            "priority_cards_emitted": priority.get("emitted"),
            "priority_cards_requested": priority.get("total_requested"),
            "t3_deterministic_replay_passed": parallel_validation.get(
                "deterministic_parallel_replay"
            ),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "metrics" / "project_status.json",
    )
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    rendered = json.dumps(generate(), indent=2, sort_keys=True) + "\n"
    if args.check:
        existing = args.output.read_text(encoding="utf-8")
        existing_value = json.loads(existing)
        rendered_value = json.loads(rendered)
        existing_value.pop("generated_at", None)
        rendered_value.pop("generated_at", None)
        if existing_value != rendered_value:
            raise SystemExit(f"stale project status: {args.output}")
        print(f"PASS project status current: {args.output}")
        return
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
