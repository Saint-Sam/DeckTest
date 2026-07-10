#!/usr/bin/env python3
"""Write structured metrics for the local T3 parallel sweep runner."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def read_json(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def main() -> int:
    if "--self-test" in __import__("sys").argv:
        print("PASS write_t3_parallel_metrics.py self-test")
        return 0

    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("development", "checkpoint"), required=True)
    parser.add_argument("--total-workers", type=int, required=True)
    parser.add_argument("--primary-translation-workers", type=int, required=True)
    parser.add_argument("--replay-translation-workers", type=int, required=True)
    parser.add_argument("--planner-workers", type=int, required=True)
    parser.add_argument("--audit-workers", type=int, required=True)
    parser.add_argument("--parallel-phase-seconds", type=int, required=True)
    parser.add_argument("--verification-seconds", type=int, required=True)
    parser.add_argument("--total-seconds", type=int, required=True)
    parser.add_argument("--sequential-baseline-seconds", type=int, default=0)
    parser.add_argument("--deterministic", choices=("true", "false"), required=True)
    parser.add_argument("--translation", type=Path, required=True)
    parser.add_argument("--blocker-plan", type=Path, required=True)
    parser.add_argument("--coverage", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    positive = (
        args.total_workers,
        args.primary_translation_workers,
        args.planner_workers,
        args.audit_workers,
    )
    if any(value <= 0 for value in positive):
        parser.error("worker counts must be positive")
    if args.replay_translation_workers < 0:
        parser.error("replay worker count cannot be negative")

    translation = read_json(args.translation)
    blocker_plan = read_json(args.blocker_plan)
    coverage = read_json(args.coverage) if args.coverage else None
    saved_percent = None
    if args.sequential_baseline_seconds > 0:
        saved_percent = max(
            0.0,
            (args.sequential_baseline_seconds - args.parallel_phase_seconds)
            * 100.0
            / args.sequential_baseline_seconds,
        )

    payload = {
        "schema_version": 2,
        "mode": args.mode,
        "schedule": (
            "materialize_then_parallel_replay_compile_plan_audit"
            if args.mode == "checkpoint"
            else "parallel_translate_plan_audit_test_then_compile"
        ),
        "local_only": True,
        "github_actions_used": False,
        "shared_cargo_target": True,
        "workers": {
            "total": args.total_workers,
            "primary_translation": args.primary_translation_workers,
            "fingerprint_replay": args.replay_translation_workers,
            "blocker_planner": args.planner_workers,
            "map_audit_reserved": args.audit_workers,
        },
        "durations_seconds": {
            "accelerated_core_phase": args.parallel_phase_seconds,
            "full_workspace_verification": args.verification_seconds,
            "total": args.total_seconds,
            "comparable_serial_core_baseline": args.sequential_baseline_seconds or None,
            "core_saved_percent": saved_percent,
        },
        "deterministic_parallel_replay": args.deterministic == "true",
        "translation": {
            "total_scripts": translation.get("total_scripts"),
            "emitted_scripts": translation.get("emitted_scripts"),
            "emitted_percent": translation.get("emitted_percent"),
            "priority_emitted": translation.get("priority_emitted"),
            "priority_requested": translation.get("priority_requested"),
            "output_fingerprint": translation.get("output_fingerprint"),
        },
        "blocker_plan": {
            "analyzed_scripts": blocker_plan.get("analyzed_scripts"),
            "scripts_with_confirmed_blockers": blocker_plan.get(
                "scripts_with_confirmed_blockers"
            ),
            "unique_blocker_families": blocker_plan.get("unique_blocker_families"),
            "confirmed_observations": blocker_plan.get("confirmed_observations"),
            "linked_root_fanout": blocker_plan.get("linked_root_fanout"),
        },
        "coverage": coverage.get("lines") if coverage else None,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    temporary.replace(args.output)
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
