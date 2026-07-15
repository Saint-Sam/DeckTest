#!/usr/bin/env python3
"""Build exact-product T4 game-length diagnostics from AI replays."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from datetime import date
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def nearest_rank(values: list[int], quantile: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[max(0, math.ceil(quantile * len(ordered)) - 1)]


def ratio_ppm(numerator: int, denominator: int) -> int:
    return 0 if denominator <= 0 else min(4_294_967_295, numerator * 1_000_000 // denominator)


def require_integer(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"{label} must be a nonnegative integer")
    return value


def summarize_run(path: Path, replay: dict[str, Any]) -> dict[str, Any]:
    expected = replay.get("expected")
    if not isinstance(expected, dict):
        raise ValueError(f"{path} has no expected summary")
    progress = expected.get("progress")
    if not isinstance(progress, dict):
        raise ValueError(f"{path} has no current progress diagnostics")
    rounds = progress.get("rounds")
    if not isinstance(rounds, list) or not rounds:
        raise ValueError(f"{path} has no per-round progress records")
    required = (
        "state_observations",
        "repeated_full_state_hashes",
        "repeated_decision_state_keys",
        "no_progress_rounds",
        "maximum_consecutive_no_progress_rounds",
    )
    for field in required:
        require_integer(progress.get(field), f"{path}:{field}")
    round_totals = {
        field: sum(require_integer(round_record.get(field), f"{path}:round:{field}") for round_record in rounds)
        for field in (
            "table_damage_to_players",
            "life_total_movement",
            "casts",
            "meaningful_actions",
            "pass_only_priority_cycles",
            "active_players_with_progress",
            "eliminations",
        )
    }
    turns = require_integer(expected.get("turns"), f"{path}:turns")
    decisions = replay.get("decisions")
    if not isinstance(decisions, list):
        raise ValueError(f"{path} decisions must be an array")
    state_observations = int(progress["state_observations"])
    repeated_hashes = int(progress["repeated_full_state_hashes"])
    repeated_keys = int(progress["repeated_decision_state_keys"])
    elimination_turns = [
        require_integer(item.get("turn"), f"{path}:elimination:turn")
        for item in progress.get("eliminations", [])
        if isinstance(item, dict)
    ]
    damage_per_turn_milli = round_totals["table_damage_to_players"] * 1000 // max(1, turns)
    casts_per_turn_milli = round_totals["casts"] * 1000 // max(1, turns)
    return {
        "policy": replay.get("policy_kind"),
        "path": str(path.resolve().relative_to(ROOT)) if path.resolve().is_relative_to(ROOT) else str(path),
        "sha256": sha256(path),
        "seed": replay.get("seed"),
        "policy_seed": replay.get("policy_seed"),
        "turns": turns,
        "max_turns": require_integer(replay.get("max_turns"), f"{path}:max_turns"),
        "termination_reason": progress.get("termination_reason"),
        "turn_cap_reached": progress.get("turn_cap_reached"),
        "state_observations": state_observations,
        "repeated_full_state_hashes": repeated_hashes,
        "repeated_full_state_hash_rate_ppm": ratio_ppm(repeated_hashes, state_observations),
        "decision_state_key_observations": len(decisions),
        "repeated_decision_state_keys": repeated_keys,
        "repeated_decision_state_key_rate_ppm": ratio_ppm(repeated_keys, len(decisions)),
        "rounds": len(rounds),
        "no_progress_rounds": int(progress["no_progress_rounds"]),
        "maximum_consecutive_no_progress_rounds": int(
            progress["maximum_consecutive_no_progress_rounds"]
        ),
        **round_totals,
        "damage_per_turn_milli": damage_per_turn_milli,
        "casts_per_turn_milli": casts_per_turn_milli,
        "first_elimination_turn": min(elimination_turns) if elimination_turns else None,
        "last_elimination_turn": max(elimination_turns) if elimination_turns else None,
        "diagnostic_flags": {
            "full_state_loop_observed": repeated_hashes > 0,
            "decision_state_repetition_observed": repeated_keys > 0,
            "consecutive_no_progress_observed": int(
                progress["maximum_consecutive_no_progress_rounds"]
            )
            > 0,
            "low_damage_throughput_below_two_per_turn": damage_per_turn_milli < 2_000,
            "low_cast_throughput_below_one_per_four_turns": casts_per_turn_milli < 250,
        },
    }


def build_report(paths: list[Path], product_commit: str, product_tree: str) -> dict[str, Any]:
    if len(product_commit) != 40 or len(product_tree) != 40:
        raise ValueError("product commit and tree must be full 40-character hashes")
    runs = [summarize_run(path, load(path)) for path in paths]
    turns = [run["turns"] for run in runs]
    state_observations = sum(run["state_observations"] for run in runs)
    repeated_hashes = sum(run["repeated_full_state_hashes"] for run in runs)
    key_observations = sum(run["decision_state_key_observations"] for run in runs)
    repeated_keys = sum(run["repeated_decision_state_keys"] for run in runs)
    caps = sum(bool(run["turn_cap_reached"]) for run in runs)
    aggregate = {
        "games": len(runs),
        "turn_p50": nearest_rank(turns, 0.50),
        "turn_p95": nearest_rank(turns, 0.95),
        "turn_p99": nearest_rank(turns, 0.99),
        "maximum_turns": max(turns, default=0),
        "turn_cap_games": caps,
        "turn_cap_rate_ppm": ratio_ppm(caps, len(runs)),
        "state_observations": state_observations,
        "repeated_full_state_hashes": repeated_hashes,
        "repeated_full_state_hash_rate_ppm": ratio_ppm(repeated_hashes, state_observations),
        "decision_state_key_observations": key_observations,
        "repeated_decision_state_keys": repeated_keys,
        "repeated_decision_state_key_rate_ppm": ratio_ppm(repeated_keys, key_observations),
        "no_progress_rounds": sum(run["no_progress_rounds"] for run in runs),
        "maximum_consecutive_no_progress_rounds": max(
            (run["maximum_consecutive_no_progress_rounds"] for run in runs),
            default=0,
        ),
        "table_damage_to_players": sum(run["table_damage_to_players"] for run in runs),
        "life_total_movement": sum(run["life_total_movement"] for run in runs),
        "casts": sum(run["casts"] for run in runs),
        "meaningful_actions": sum(run["meaningful_actions"] for run in runs),
        "pass_only_priority_cycles": sum(run["pass_only_priority_cycles"] for run in runs),
    }
    return {
        "schema_version": 1,
        "generated_at": date.today().isoformat(),
        "status": "diagnostic_complete",
        "artifact_classification": "diagnostic_not_promotion_eligible",
        "product_commit": product_commit,
        "product_tree": product_tree,
        "definitions": {
            "table_round": "four successive game turns",
            "no_progress": "no player damage, life movement, cast, meaningful action, or elimination",
            "throughput_flags": "provisional diagnostic thresholds, not promotion thresholds",
        },
        "runs": runs,
        "aggregate": aggregate,
        "attribution": {
            "state_loop_supported": repeated_hashes > 0,
            "no_progress_stall_supported": aggregate["no_progress_rounds"] > 0,
            "low_damage_throughput_observed": any(
                run["diagnostic_flags"]["low_damage_throughput_below_two_per_turn"]
                for run in runs
            ),
            "low_cast_throughput_observed": any(
                run["diagnostic_flags"]["low_cast_throughput_below_one_per_four_turns"]
                for run in runs
            ),
        },
        "promotion_eligible": False,
        "remaining_evidence": [
            "multiple seeds for each policy",
            "at least three materially different archetype pods",
            "guardrail and evaluation-weight ablations",
            "development, validation, and sealed strength tracks",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("replays", nargs="+", type=Path)
    parser.add_argument("--product-commit", required=True)
    parser.add_argument("--product-tree", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    report = build_report(args.replays, args.product_commit, args.product_tree)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.check:
        if not args.output.exists() or args.output.read_text(encoding="utf-8") != rendered:
            raise SystemExit(f"stale T4 long-game diagnostics: {args.output}")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
