#!/usr/bin/env python3
"""Run the focused T4 search and decision-surface mutation campaign locally."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOT = ROOT / "target/t4-mutation/source"
LOG_ROOT = ROOT / "reports/gates/T4/mutations"
METRIC = ROOT / "metrics/ai/t4_mutation.json"
REPORT = ROOT / "reports/gates/T4/mutation_test_report.md"


@dataclass(frozen=True)
class Mutant:
    mutant_id: str
    category: str
    file: str
    old: str
    new: str
    package: str
    test: str


MUTANTS = (
    Mutant(
        "M01_SHARED_DEADLINE_IGNORED_BETWEEN_TREES",
        "shared total-decision deadline",
        "crates/forge-ai/src/search.rs",
        """        if deadline_expired && (worker != 0 || !results.is_empty()) {
            break;
        }
""",
        """        if false && deadline_expired && (worker != 0 || !results.is_empty()) {
            break;
        }
""",
        "forge-ai",
        "total_wall_budget_does_not_start_sequential_determinizations_after_expiry",
    ),
    Mutant(
        "M02_CALLER_CONTEXT_TIME_RESET",
        "caller-side context time consumes deadline",
        "crates/forge-ai/src/search.rs",
        "        let started = config.decision_started.unwrap_or_else(Instant::now);\n",
        "        let started = Instant::now();\n",
        "forge-ai",
        "caller_side_context_time_counts_against_the_total_budget",
    ),
    Mutant(
        "M03_EDGE_VISITS_NOT_BACKPROPAGATED",
        "edge-level visits and values",
        "crates/forge-ai/src/search.rs",
        "        edge.visits = edge.visits.saturating_add(1);\n",
        "        edge.visits = edge.visits.saturating_add(0);\n",
        "forge-ai",
        "state_keys_share_convergent_children_and_report_hits",
    ),
    Mutant(
        "M04_EDGE_VALUES_NOT_BACKPROPAGATED",
        "edge-level visits and values",
        "crates/forge-ai/src/search.rs",
        "        edge.total_value = edge.total_value.saturating_add(i128::from(value));\n",
        "        edge.total_value = edge.total_value.saturating_add(0);\n",
        "forge-ai",
        "root_parallel_visit_sum_selects_the_winning_action_replayably",
    ),
    Mutant(
        "M05_TRANSPOSITION_COLLISION_MERGED_BLINDLY",
        "transposition collision guard",
        "crates/forge-ai/src/search.rs",
        """                    bucket.iter().copied().find(|candidate| {
                        domain.transposition_equivalent(&arena[*candidate].state, &next)
                    })
""",
        """                    bucket.iter().copied().find(|candidate| {
                        let _ = candidate;
                        true
                    })
""",
        "forge-ai",
        "colliding_keys_do_not_merge_non_equivalent_states",
    ),
    Mutant(
        "M06_TRANSPOSITION_CHILD_NOT_REGISTERED",
        "transposition equivalence reuse",
        "crates/forge-ai/src/search.rs",
        """                if let Some(key) = key {
                    transpositions.entry(key).or_default().push(child);
                }
""",
        """                if let Some(_key) = key {
                    // Mutant intentionally omits child registration.
                }
""",
        "forge-ai",
        "state_keys_share_convergent_children_and_report_hits",
    ),
    Mutant(
        "M07_PLAYER_VIEW_LEAKS_HIDDEN_IDENTITIES",
        "hidden-information poisoning",
        "crates/forge-core/src/lib.rs",
        "                if hidden_from_observer && !remembered {\n",
        "                if hidden_from_observer && !remembered && record.card().get() == 0 {\n",
        "forge-ai",
        "hidden_identity_poison_does_not_change_sample",
    ),
    Mutant(
        "M08_ATTACK_PATH_IGNORES_DEFENDER_PREFIX",
        "hierarchical path discrimination",
        "crates/forge-game-runner/src/lib.rs",
        "        state = combat_path_mix(state, attack.defending_player().index() as u64);\n",
        "        state = combat_path_mix(state, active.index() as u64);\n",
        "forge-game-runner",
        "attack_subcontexts_expose_split_defenders_without_a_cartesian_product",
    ),
    Mutant(
        "M09_PARTIAL_TARGET_LEGALITY_FORCED_TRUE",
        "partial target legality",
        "crates/forge-game-runner/src/lib.rs",
        """                .with_source(pending.object)
                .with_announced_targets(
                    pending.target_requirements.clone(),
                    pending.targets.clone(),
                )
                .with_target_legalities(pending.target_legalities.clone())
                .with_object_choices(object_choices)
""",
        """                .with_source(pending.object)
                .with_announced_targets(
                    pending.target_requirements.clone(),
                    pending.targets.clone(),
                )
                .with_target_legalities(vec![true; pending.target_legalities.len()])
                .with_object_choices(object_choices)
""",
        "forge-game-runner",
        "partially_illegal_targets_skip_only_their_bound_effects",
    ),
    Mutant(
        "M10_SAME_BATCH_TRIGGER_TARGETS_NOT_STAGED",
        "same-batch trigger target staging",
        "crates/forge-game-runner/src/lib.rs",
        """        if requirement.kind() == TargetKind::StackEntry
            && requirement.predicate() == TargetPredicate::Any
        {
""",
        """        if false
            && requirement.kind() == TargetKind::StackEntry
            && requirement.predicate() == TargetPredicate::Any
        {
""",
        "forge-game-runner",
        "same_batch_trigger_targeting_uses_the_staged_stack_for_human_and_ai",
    ),
    Mutant(
        "M11_TRIGGER_ORDER_IGNORES_SELECTED_IDS",
        "trigger ordering",
        "crates/forge-game-runner/src/lib.rs",
        "                    (!used[index] && instance.trigger() == *trigger).then_some(index)\n",
        "                    (!used[index]).then_some(index)\n",
        "forge-game-runner",
        "simultaneous_trigger_order_uses_shared_human_and_ai_contexts",
    ),
    Mutant(
        "M12_NO_LEGAL_TARGET_TRIGGER_FORCED_ON_STACK",
        "no-legal-target trigger disposition",
        "crates/forge-game-runner/src/lib.rs",
        "                bindings.push(TriggerStackBinding::no_legal_targets(trigger, requirements));\n",
        "                bindings.push(TriggerStackBinding::new(trigger));\n",
        "forge-game-runner",
        "required_trigger_without_legal_targets_is_removed_without_prompting",
    ),
    Mutant(
        "M13_ADDITIONAL_DISCARD_COST_DROPPED",
        "additional costs",
        "crates/forge-game-runner/src/lib.rs",
        """                    Ok(SpellAdditionalCostPayment::DiscardCards {
                        objects: objects.clone(),
                    })
""",
        """                    Ok(SpellAdditionalCostPayment::DiscardCards {
                        objects: Vec::new(),
                    })
""",
        "forge-game-runner",
        "additional_spell_costs_use_canonical_human_ai_search_and_replay_paths",
    ),
    Mutant(
        "M14_ALTERNATE_COST_IDENTITY_DROPPED",
        "alternate costs",
        "crates/forge-game-runner/src/lib.rs",
        """        if let Some(alternate) = alternate {
            decisions = decisions.with_alternate_cost(runtime_alternate_to_core(alternate));
        }
""",
        """        if let Some(_alternate) = alternate {
            // Mutant intentionally drops the selected alternate-cost identity.
        }
""",
        "forge-game-runner",
        "commander_alternate_cost_uses_the_shared_canonical_cast_hierarchy",
    ),
    Mutant(
        "M15_SEARCH_STOPS_BEFORE_OPPONENT_PRIORITY",
        "search through opponent priority and resolution",
        "crates/forge-game-runner/src/lib.rs",
        """        if transitioned {
            self.advance_after_transition(next, decision_count)
        } else {
""",
        """        if transitioned {
            Ok(self.finish_state(next, decision_count))
        } else {
""",
        "forge-game-runner",
        "main_search_crosses_opponent_priority_and_resolves_the_response_stack",
    ),
)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def product_binding() -> tuple[str, str, dict[str, Any]]:
    coverage = load_json(ROOT / "metrics/coverage.json")
    commit = coverage.get("reviewed_commit")
    tree = coverage.get("reviewed_tree")
    if (
        coverage.get("passed") is not True
        or coverage.get("schema_version") != 3
        or not isinstance(commit, str)
        or not isinstance(tree, str)
    ):
        raise ValueError("current T4 coverage does not provide a passing exact product")
    actual = subprocess.check_output(
        ["git", "show", "-s", "--format=%T", commit], cwd=ROOT, text=True
    ).strip()
    if actual != tree:
        raise ValueError("coverage commit/tree binding is invalid")
    return commit, tree, coverage


def mutation_spec_sha256() -> str:
    payload = json.dumps(
        [asdict(mutant) for mutant in MUTANTS],
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    return sha256_bytes(payload)


def materialize_product(commit: str) -> None:
    if SOURCE_ROOT.exists():
        shutil.rmtree(SOURCE_ROOT)
    SOURCE_ROOT.mkdir(parents=True)
    archive = subprocess.check_output(
        ["git", "archive", "--format=tar", commit], cwd=ROOT
    )
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as bundle:
        destination = SOURCE_ROOT.resolve()
        for member in bundle.getmembers():
            extracted = (SOURCE_ROOT / member.name).resolve()
            if extracted != destination and destination not in extracted.parents:
                raise ValueError(f"unsafe archive member {member.name}")
        bundle.extractall(SOURCE_ROOT)


def command_for(mutant: Mutant, jobs: int) -> list[str]:
    return [
        "cargo",
        "test",
        "-p",
        mutant.package,
        "--locked",
        "--offline",
        "--jobs",
        str(jobs),
        "--lib",
        mutant.test,
    ]


def run_test(command: list[str], log: Path) -> tuple[int, str]:
    result = subprocess.run(
        command,
        cwd=SOURCE_ROOT,
        env={
            **os.environ,
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(ROOT / "target"),
            "TMPDIR": os.environ.get("TMPDIR", "/private/tmp"),
        },
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text(
        "$ " + " ".join(command) + f"\nexit_code={result.returncode}\n\n" + result.stdout,
        encoding="utf-8",
    )
    return result.returncode, result.stdout


def passing_single_test(return_code: int, output: str) -> bool:
    return return_code == 0 and "running 1 test" in output and "1 passed" in output


def killed_by_test(mutant: Mutant, return_code: int, output: str) -> bool:
    return (
        return_code != 0
        and "running 1 test" in output
        and f"{mutant.test} ... FAILED" in output
    )


def apply_mutant(mutant: Mutant) -> bytes:
    path = SOURCE_ROOT / mutant.file
    original = path.read_bytes()
    source = original.decode("utf-8")
    count = source.count(mutant.old)
    if count != 1:
        raise ValueError(f"{mutant.mutant_id} expected one anchor, found {count}")
    path.write_text(source.replace(mutant.old, mutant.new, 1), encoding="utf-8")
    return original


def validate_spec(root: Path) -> None:
    identifiers = [mutant.mutant_id for mutant in MUTANTS]
    if len(set(identifiers)) != len(identifiers):
        raise ValueError("T4 mutation identifiers are not unique")
    for mutant in MUTANTS:
        path = root / mutant.file
        count = path.read_text(encoding="utf-8").count(mutant.old)
        if count != 1:
            raise ValueError(f"{mutant.mutant_id} expected one anchor, found {count}")


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def render_report(metric: dict[str, Any]) -> str:
    lines = [
        "# T4 Critical Mutation Gate",
        "",
        f"Exact product: `{metric['reviewed_commit']}` / `{metric['reviewed_tree']}`",
        "",
        f"All {metric['mutants_total']} declared critical mutants were killed by focused tests.",
        "",
        f"- Score: {metric['mutation_score_percent']}%",
        f"- Survivors: {len(metric['surviving_mutants'])}",
        f"- Shared Cargo target: `{metric['constraints']['cargo_target_dir']}`",
        "",
        "| Mutant | Category | Killing test |",
        "|---|---|---|",
    ]
    for row in metric["mutants"]:
        lines.append(f"| {row['mutant_id']} | {row['category']} | `{row['test']}` |")
    return "\n".join(lines) + "\n"


def generate(jobs: int) -> int:
    if not 1 <= jobs <= 24:
        raise ValueError("jobs must be in 1..=24")
    commit, tree, coverage = product_binding()
    materialize_product(commit)
    LOG_ROOT.mkdir(parents=True, exist_ok=True)
    for log in LOG_ROOT.glob("generated-*.log"):
        log.unlink()

    baseline_by_test: dict[tuple[str, str], dict[str, Any]] = {}
    for mutant in MUTANTS:
        key = (mutant.package, mutant.test)
        if key in baseline_by_test:
            continue
        command = command_for(mutant, jobs)
        log = LOG_ROOT / f"generated-baseline-{mutant.mutant_id.lower()}.log"
        return_code, output = run_test(command, log)
        if not passing_single_test(return_code, output):
            raise RuntimeError(f"focused baseline failed for {mutant.mutant_id}")
        baseline_by_test[key] = {
            "package": mutant.package,
            "test": mutant.test,
            "command": command,
            "return_code": return_code,
            "log": str(log.relative_to(ROOT)),
            "log_sha256": sha256_file(log),
        }

    rows = []
    survivors = []
    for mutant in MUTANTS:
        path = SOURCE_ROOT / mutant.file
        original = apply_mutant(mutant)
        try:
            command = command_for(mutant, jobs)
            log = LOG_ROOT / f"generated-mutant-{mutant.mutant_id.lower()}.log"
            return_code, output = run_test(command, log)
        finally:
            path.write_bytes(original)
        killed = killed_by_test(mutant, return_code, output)
        if not killed:
            survivors.append(mutant.mutant_id)
        rows.append(
            {
                "mutant_id": mutant.mutant_id,
                "category": mutant.category,
                "file": mutant.file,
                "test": mutant.test,
                "command": command,
                "return_code": return_code,
                "status": "killed" if killed else "survived",
                "kill_kind": "focused_test_failure" if killed else None,
                "log": str(log.relative_to(ROOT)),
                "log_sha256": sha256_file(log),
            }
        )

    killed_count = len(MUTANTS) - len(survivors)
    metric: dict[str, Any] = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "generator": "tools/run_t4_mutation_gate.py",
        "reviewed_commit": commit,
        "reviewed_tree": tree,
        "coverage_report_sha256": sha256_file(ROOT / "metrics/coverage.json"),
        "coverage_lines_percent": coverage["lines"]["percent"],
        "changed_lines_percent": coverage["changed_lines"]["percent"],
        "mutation_spec_sha256": mutation_spec_sha256(),
        "baseline": list(baseline_by_test.values()),
        "mutants": rows,
        "mutants_total": len(MUTANTS),
        "mutants_killed": killed_count,
        "surviving_mutants": survivors,
        "mutation_score_percent": killed_count * 100.0 / len(MUTANTS),
        "minimum_score_percent": 100.0,
        "constraints": {
            "local_only": True,
            "network_used": False,
            "github_actions_used": False,
            "installs_performed": False,
            "push_performed": False,
            "workers_used": jobs,
            "cargo_target_dir": "target",
            "separate_cargo_cache_created": False,
        },
    }
    write_json(METRIC, metric)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(render_report(metric), encoding="utf-8")
    if survivors:
        raise RuntimeError(f"surviving mutants: {', '.join(survivors)}")
    print(f"PASS T4 mutation gate: killed={killed_count}/{len(MUTANTS)} score=100%")
    return 0


def check() -> int:
    commit, tree, _coverage = product_binding()
    metric = load_json(METRIC)
    if metric.get("schema_version") != 1 or metric.get("generator") != "tools/run_t4_mutation_gate.py":
        raise ValueError("T4 mutation metric has an invalid schema or generator")
    if metric.get("reviewed_commit") != commit or metric.get("reviewed_tree") != tree:
        raise ValueError("T4 mutation metric is bound to a stale product")
    if metric.get("coverage_report_sha256") != sha256_file(ROOT / "metrics/coverage.json"):
        raise ValueError("T4 mutation metric is bound to stale coverage")
    if metric.get("mutation_spec_sha256") != mutation_spec_sha256():
        raise ValueError("T4 mutation specification is stale")
    if (
        metric.get("mutants_total") != len(MUTANTS)
        or metric.get("mutants_killed") != len(MUTANTS)
        or metric.get("surviving_mutants") != []
        or metric.get("mutation_score_percent") != 100.0
    ):
        raise ValueError("T4 mutation campaign has a stale denominator or survivors")
    rows = metric.get("mutants")
    if not isinstance(rows, list) or len(rows) != len(MUTANTS):
        raise ValueError("T4 mutation result ledger is incomplete")
    for row in [*metric.get("baseline", []), *rows]:
        log = ROOT / str(row.get("log", ""))
        if not log.is_file() or sha256_file(log) != row.get("log_sha256"):
            raise ValueError(f"T4 mutation log is missing or stale: {log}")
    if any(row.get("status") != "killed" for row in rows):
        raise ValueError("T4 mutation ledger contains a survivor")
    if not REPORT.is_file() or REPORT.read_text(encoding="utf-8") != render_report(metric):
        raise ValueError("T4 mutation report is missing or stale")
    print(f"PASS T4 mutation evidence: killed={len(MUTANTS)}/{len(MUTANTS)} survivors=0")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=24)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        if args.self_test:
            validate_spec(ROOT)
            print(f"PASS T4 mutation specification: mutants={len(MUTANTS)}")
            return 0
        return check() if args.check else generate(args.jobs)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_t4_mutation_gate.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
