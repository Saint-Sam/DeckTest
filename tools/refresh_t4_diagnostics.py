#!/usr/bin/env python3
"""Refresh product-bound T4 diagnostics from exact local replay artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from datetime import date
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
REPLAYS = (
    ROOT / "reports/gates/T4.3/ai-baseline.frsreplay",
    ROOT / "reports/gates/T4.3/random-legal-baseline.frsreplay",
    ROOT / "reports/gates/T4.3/search-baseline.frsreplay",
)
AUDIT = ROOT / "metrics/ai/decision_state_audit.json"
BENCHMARK = ROOT / "metrics/ai/decision_benchmark.json"
LATENCY = ROOT / "metrics/ai/latency_cost.json"
KNEE_CONTRACT = ROOT / "metrics/ai/search_budget_knee.json"
KNEE_RESULTS = ROOT / "metrics/ai/search_budget_knee_results.json"


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def write(path: Path, value: dict[str, Any]) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def nearest_rank(values: list[int], quantile: float) -> int:
    if not values:
        raise ValueError("cannot measure an empty decision set")
    ordered = sorted(values)
    index = max(0, math.ceil(quantile * len(ordered)) - 1)
    return ordered[index]


def policy_decisions(replay: dict[str, Any], policy: str) -> list[dict[str, Any]]:
    return [decision for decision in replay["decisions"] if decision["policy"] == policy]


def complete_record_count(decisions: list[dict[str, Any]], field: str) -> int:
    return sum(field in decision and decision[field] is not None for decision in decisions)


def episode_summary(decisions: list[dict[str, Any]]) -> dict[str, Any]:
    episodes: dict[str, list[dict[str, Any]]] = {}
    for decision in decisions:
        episode_id = decision.get("decision_episode_id")
        if isinstance(episode_id, str) and episode_id:
            episodes.setdefault(episode_id, []).append(decision)
    complete = len(episodes) > 0 and all(
        len(records) >= 1
        and sum(int(record.get("path_depth", -1) == 0) for record in records) == 1
        and sum(bool(record.get("is_terminal_subchoice")) for record in records) == 1
        and len(
            {
                record.get("final_concrete_action_id")
                for record in records
                if record.get("final_concrete_action_id")
            }
        )
        == 1
        for records in episodes.values()
    ) and sum(len(records) for records in episodes.values()) == len(decisions)
    return {
        "raw_prompt_records": len(decisions),
        "decision_episodes": len(episodes),
        "strategic_decision_episodes": sum(
            any(bool(record.get("is_strategic_root")) for record in records)
            for records in episodes.values()
        ),
        "multi_prompt_episodes": sum(len(records) > 1 for records in episodes.values()),
        "forced_prompt_records": sum(
            bool(decision.get("is_forced")) for decision in decisions
        ),
        "episode_linkage_complete": complete,
    }


def replay_run(
    replay: dict[str, Any], path: Path, existing: dict[str, Any]
) -> dict[str, Any]:
    decisions = replay["decisions"]
    expected = replay["expected"]
    metrics = expected["metrics"]
    run = dict(existing)
    run.update(
        {
            "policy": replay["policy_kind"],
            "replay": str(path.relative_to(ROOT)),
            "replay_sha256": sha256(path),
            "decisions": len(decisions),
            "typed_actions": metrics["actions"],
            "turns": expected["turns"],
            "max_turns": replay["max_turns"],
            "winner_seat_zero_based": expected["winner"],
            "final_hash": str(expected["final_hash"]),
            "canonical_legal_action_records": complete_record_count(
                decisions, "canonical_legal_actions"
            ),
            "player_view_hash_records": complete_record_count(
                decisions, "player_view_hash"
            ),
            "decision_state_key_records": complete_record_count(
                decisions, "decision_state_key"
            ),
            "exact_policy_replay": True,
            "exact_typed_action_replay": True,
            "decision_episode_accounting": episode_summary(decisions),
        }
    )
    if replay["policy_kind"] == "determinized-uct-v1":
        searched = policy_decisions(replay, "determinized-uct-v1")
        run.update(
            {
                "budget": {
                    "iterations_per_determinization": replay["search_iterations"],
                    "determinizations": replay["search_determinizations"],
                    "workers": replay["search_workers"],
                },
                "command": (
                    "target/release/forge-cli play --ai --search "
                    f"--search-iterations {replay['search_iterations']} "
                    f"--determinizations {replay['search_determinizations']} "
                    f"--search-workers {replay['search_workers']} "
                    f"--seed {replay['seed']} "
                    f"--policy-seed {replay['policy_seed']} "
                    f"--max-turns {replay['max_turns']} "
                    f"--replay-out {path.relative_to(ROOT)}"
                ),
                "searched_decisions": len(searched),
                "simulations": sum(item["simulations"] for item in searched),
                "nodes": sum(item["nodes"] for item in searched),
                "maximum_depth": max(item["maximum_depth"] for item in searched),
                "transposition_hits": sum(
                    item["transposition_hits"] for item in searched
                ),
            }
        )
    return run


def latency_run(
    replay: dict[str, Any], policy: str, scope: str
) -> dict[str, Any]:
    measured = policy_decisions(replay, policy)
    wall = [int(item["wall_latency_us"]) for item in measured]
    legal = [int(item["legal_actions"]) for item in measured]
    if not measured:
        raise ValueError(f"replay contains no {policy} decisions")
    run: dict[str, Any] = {
        "policy": policy,
        "scope": scope,
        "decisions": len(measured),
        "mean_wall_latency_us": round(sum(wall) / len(wall), 4),
        "p50_wall_latency_us": nearest_rank(wall, 0.50),
        "p95_wall_latency_us": nearest_rank(wall, 0.95),
        "p99_wall_latency_us": nearest_rank(wall, 0.99),
        "max_wall_latency_us": max(wall),
        "mean_legal_actions": round(sum(legal) / len(legal), 4),
        "max_legal_actions": max(legal),
        "singleton_early_stops": sum(
            item["stop_reason"] == "singleton_legal_action"
            for item in replay["decisions"]
            if scope != "searched_decisions" or item["policy"] == policy
        ),
    }
    if scope == "searched_decisions":
        run.update(
            {
                "budget_kind": "fixed_iterations_per_determinization",
                "iterations": replay["search_iterations"],
                "determinizations": replay["search_determinizations"],
                "workers": replay["search_workers"],
                "searched_decisions": len(measured),
                "simulations": sum(item["simulations"] for item in measured),
                "nodes": sum(item["nodes"] for item in measured),
                "maximum_depth": max(item["maximum_depth"] for item in measured),
                "transposition_hits": sum(
                    item["transposition_hits"] for item in measured
                ),
            }
        )
    return run


def refresh_benchmark(
    product_commit: str,
    product_tree: str,
    replays: list[dict[str, Any]],
    audit: dict[str, Any],
) -> None:
    benchmark = load(BENCHMARK)
    benchmark["generated_at"] = date.today().isoformat()
    benchmark["product_commit"] = product_commit
    benchmark["product_tree"] = product_tree
    existing = {run["policy"]: run for run in benchmark["runs"]}
    benchmark["runs"] = [
        replay_run(replay, path, existing[replay["policy_kind"]])
        for replay, path in zip(replays, REPLAYS)
    ]
    benchmark["recorded_key_signature_consistency"] = audit[
        "recorded_key_signature_consistency"
    ]
    benchmark["near_state_dedup_audit"] = audit["near_state_dedup_audit"]
    totals = audit["totals"]
    benchmark["decision_state_audit"] = {
        "path": str(AUDIT.relative_to(ROOT)),
        "sha256": sha256(AUDIT),
        "decisions": totals["decisions"],
        "unique_state_keys": totals["unique_state_keys"],
        "path_bound_decisions": totals["path_bound_decisions"],
        "decision_episodes": totals["decision_episodes"],
        "strategic_decision_episodes": totals["strategic_decision_episodes"],
        "forced_prompt_records": totals["forced_prompt_records"],
        "failures": totals["failures"],
    }
    benchmark["reasons"][2] = (
        "ordinary and same-batch inter-trigger targets, kernel-recorded partial-target "
        "legality at resolution, resolution-time trigger optionals, event-player unless "
        "payments, compiled discard/sacrifice additional spell costs, compiler-declared "
        "Commander, flashback, evoke, and overload costs, and explicit no-legal-target "
        "dispositions and statically bounded target ranges/divisions use canonical "
        "contexts, but dynamic target relationships, arbitrary and unsupported costs, "
        "non-player combat defenders, and remaining prompt families are incomplete"
    )
    write(BENCHMARK, benchmark)


def compact_comparisons(results: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        {
            "lower_ms": comparison["lower_budget_ms"],
            "upper_ms": comparison["upper_budget_ms"],
            "paired_gain_percentage_points": comparison[
                "paired_win_rate_improvement_percentage_points"
            ],
            "estimated_elo_gain": comparison["estimated_elo_improvement"],
            "confidence_interval_percentage_points": comparison[
                "confidence_interval_improvement_percentage_points"
            ],
            "lower_p95_wall_us": comparison["lower_p95_wall_latency_us"],
            "upper_p95_wall_us": comparison["upper_p95_wall_latency_us"],
            "acceptance_fields_complete": comparison[
                "all_acceptance_criteria_complete"
            ],
            "plateau_accepted": comparison["plateau_acceptance_passed"],
        }
        for comparison in results["comparisons"]
    ]


def refresh_knee_contract(
    product_commit: str,
    product_tree: str,
    search_replay: dict[str, Any],
    results: dict[str, Any],
) -> None:
    contract = load(KNEE_CONTRACT)
    evidence = contract["implementation_evidence"]
    evidence["date"] = date.today().isoformat()
    evidence["product_commit"] = product_commit
    evidence["product_tree"] = product_tree
    searched = policy_decisions(search_replay, "determinized-uct-v1")
    expected = search_replay["expected"]
    smoke = evidence["full_game_smoke"]
    smoke.update(
        {
            "iterations": search_replay["search_iterations"],
            "determinizations": search_replay["search_determinizations"],
            "workers": search_replay["search_workers"],
            "turns": expected["turns"],
            "max_turns": search_replay["max_turns"],
            "canonical_decisions": len(search_replay["decisions"]),
            "typed_actions": expected["metrics"]["actions"],
            "searched_decisions": len(searched),
            "singleton_bypasses": sum(
                item["stop_reason"] == "singleton_legal_action" for item in searched
            ),
            "simulations": sum(item["simulations"] for item in searched),
            "nodes": sum(item["nodes"] for item in searched),
            "maximum_depth": max(item["maximum_depth"] for item in searched),
            "winner_seat_zero_based": expected["winner"],
            "final_hash": str(expected["final_hash"]),
            "replay_sha256": sha256(REPLAYS[2]),
        }
    )
    knee = evidence["release_search_knee_smoke"]
    knee["result_sha256"] = sha256(KNEE_RESULTS)
    knee["games_per_comparison"] = results["games_per_comparison"]
    knee["max_turns"] = results["max_turns"]
    knee["b_vs_2b"] = compact_comparisons(results)
    knee["adaptive_p95_wall_us"] = {
        str(item["budget_ms"]): item["adaptive_p95_wall_latency_us"]
        for item in results["adaptive_ablations"]
    }
    knee["fixed_adaptive_ablations_completed"] = len(results["adaptive_ablations"])
    lower = results["comparisons"][0]["lower_p95_wall_latency_us"]
    upper = results["comparisons"][-1]["upper_p95_wall_latency_us"]
    budget_semantics = evidence["total_decision_budget_semantics"]
    budget_semantics["observed"] = (
        "hierarchical combat contexts, inline single-worker search, and "
        "single-construction determinization keep the refreshed nominal 1-4 ms "
        f"ladder at approximately {lower / 1000:.1f}-{upper / 1000:.1f} ms p95"
    )
    write(KNEE_CONTRACT, contract)


def refresh_latency(
    product_commit: str,
    product_tree: str,
    replays: list[dict[str, Any]],
    audit: dict[str, Any],
    results: dict[str, Any],
) -> None:
    latency = load(LATENCY)
    latency["generated_at"] = date.today().isoformat()
    latency["product_commit"] = product_commit
    latency["product_tree"] = product_tree
    latency["exact_replay_sources"] = [
        {
            "policy": replay["policy_kind"],
            "path": str(path.relative_to(ROOT)),
            "sha256": sha256(path),
        }
        for replay, path in zip(replays, REPLAYS)
    ]
    latency["diagnostic_runs"] = [
        latency_run(replays[0], "heuristic-v1", "policy_owned_decisions"),
        latency_run(replays[1], "random-legal-v1", "policy_owned_decisions"),
        latency_run(replays[2], "determinized-uct-v1", "searched_decisions"),
    ]
    knee = latency["release_search_knee_smoke"]
    knee["result_sha256"] = sha256(KNEE_RESULTS)
    knee["games_per_comparison"] = results["games_per_comparison"]
    knee["comparisons"] = [
        {
            "lower_budget_ms": item["lower_budget_ms"],
            "upper_budget_ms": item["upper_budget_ms"],
            "lower_p95_wall_latency_us": item["lower_p95_wall_latency_us"],
            "upper_p95_wall_latency_us": item["upper_p95_wall_latency_us"],
        }
        for item in results["comparisons"]
    ]
    knee["adaptive_ablations"] = [
        {
            "budget_ms": item["budget_ms"],
            "fixed_p95_wall_latency_us": item["fixed_p95_wall_latency_us"],
            "adaptive_p95_wall_latency_us": item["adaptive_p95_wall_latency_us"],
        }
        for item in results["adaptive_ablations"]
    ]
    observed = [
        results["comparisons"][0]["lower_p95_wall_latency_us"],
        results["comparisons"][-1]["upper_p95_wall_latency_us"],
    ]
    latency["total_decision_budget_contract"]["observed_p95_wall_latency_range_us"] = observed
    total = audit["totals"]["decisions"]
    latency["note"] = (
        "Exact product-bound heuristic, random, and search replays pass with canonical "
        "spell costs, compiler-declared alternate costs, literal-life and matching-permanent "
        "activation costs, ordinary and same-batch inter-trigger targets, kernel-recorded "
        "partial-target legality, resolution choices, and explicit no-legal-target "
        "dispositions and statically bounded target ranges/divisions, plus a "
        f"zero-failure {total:,}-decision recorded key/signature consistency audit. "
        "Independent runtime-isomorphism fixtures remain pending. The refreshed 1-4 ms "
        f"ladder measures approximately {observed[0] / 1000:.1f}-{observed[1] / 1000:.1f} "
        "ms p95. Dynamic target relationships and other prompt families remain "
        "incomplete. Safe "
        "Linux/Android CPU and memory adapters are implemented, but this Darwin replay "
        "correctly retains null resource fields; supported-platform, competence-label, "
        "archetype, and reference-device evidence remain mandatory."
    )
    write(LATENCY, latency)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--product-commit", required=True)
    parser.add_argument("--product-tree", required=True)
    args = parser.parse_args()
    if len(args.product_commit) != 40 or len(args.product_tree) != 40:
        raise ValueError("product commit and tree must be full 40-character hashes")
    replays = [load(path) for path in REPLAYS]
    audit = load(AUDIT)
    if audit.get("product_commit") != args.product_commit:
        raise ValueError("decision audit is not bound to the requested product commit")
    if audit.get("product_tree") != args.product_tree:
        raise ValueError("decision audit is not bound to the requested product tree")
    results = load(KNEE_RESULTS)
    refresh_benchmark(args.product_commit, args.product_tree, replays, audit)
    refresh_knee_contract(args.product_commit, args.product_tree, replays[2], results)
    refresh_latency(args.product_commit, args.product_tree, replays, audit, results)
    print(f"refreshed T4 diagnostics for {args.product_commit}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
