#!/usr/bin/env python3
"""Run and verify the scored Tier 3 focused mutation campaign."""

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
RUN_ROOT = ROOT / "target/t3-mutation"
SOURCE_ROOT = RUN_ROOT / "source"
BUILD_ROOT = RUN_ROOT / "build"
LOG_ROOT = ROOT / "reports/gates/T3/mutations"
METRIC = ROOT / "metrics/t3_mutation.json"
LEDGER = ROOT / "reports/gates/T3/test_log.txt"
REPORT = ROOT / "reports/gates/T3/mutation_test_report.md"
GATE_ARTIFACTS = (
    ("parallel_checkpoint", "metrics/t3_parallel_validation.json"),
    ("coverage", "metrics/coverage.json"),
    ("runtime_stage", "metrics/card_runtime_smoke.json"),
    ("semantic_stage", "metrics/card_semantics_100.json"),
    ("pod_stage", "metrics/pod_integration.json"),
    ("pod_campaign", "reports/gates/T3.9/cp-four-player-pod-2026-07-13.json"),
    ("semantic_revalidation", "reports/gates/T3.9/t3-6-semantic-revalidation.json"),
    ("sanitizer_fuzz", "metrics/local_fuzz.json"),
    ("fuzz_report", "reports/gates/T3/fuzz_report.md"),
)


@dataclass(frozen=True)
class Mutant:
    mutant_id: str
    file: str
    old: str
    new: str
    package: str
    test: str


MUTANTS = (
    Mutant(
        "M01_CHANGEZONE_DROPS_ORIGIN_GUARD",
        "crates/forge-porttools/src/mapper.rs",
        """    } else if closed_origin && origin != \"All\" {
        call(
            Operation::MoveZoneFrom,
""",
        """    } else if closed_origin && origin != \"All\" {
        call(
            Operation::MoveZone,
""",
        "forge-porttools",
        "source_bound_closed_zone_moves_retain_their_origin_guard",
    ),
    Mutant(
        "M02_SMOTHERING_TITHE_COST_TWO_TO_ONE",
        "crates/forge-cards/src/runtime.rs",
        '      effect: unless_paid(create_token("c_a_treasure_sac", 1, you()), controller_of(triggered()), mana_cost("{2}"))\n',
        '      effect: unless_paid(create_token("c_a_treasure_sac", 1, you()), controller_of(triggered()), mana_cost("{1}"))\n',
        "forge-cards",
        "opponent_draw_unless_paid_binds_both_exact_branches",
    ),
    Mutant(
        "M03_CAMPAIGN_SEED_COLLAPSES_TO_CONSTANT",
        "crates/forge-game-runner/src/lib.rs",
        """fn campaign_seed(base: u64, index: usize) -> u64 {
    let mut value = base ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}
""",
        """fn campaign_seed(base: u64, _index: usize) -> u64 {
    base
}
""",
        "forge-game-runner",
        "campaign_seed_schedule_is_deterministic_and_disperse",
    ),
    Mutant(
        "M04_IDENTITY_LEDGER_DROPS_EFFECT_ACTIONS",
        "crates/forge-game-runner/src/lib.rs",
        "        self.effect_actions = self.effect_actions.saturating_add(other.effect_actions);\n",
        "        self.effect_actions = self.effect_actions.saturating_add(0);\n",
        "forge-game-runner",
        "identity_exercise_aggregation_preserves_every_counter",
    ),
    Mutant(
        "M05_RON_NESTING_GUARD_OFF_BY_ONE",
        "crates/forge-testkit/src/lib.rs",
        "        if self.nesting_depth >= MAX_RON_NESTING_DEPTH {\n",
        "        if self.nesting_depth > MAX_RON_NESTING_DEPTH {\n",
        "forge-testkit",
        "ron_parser_rejects_adversarial_nesting_without_stack_overflow",
    ),
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def product_binding() -> tuple[str, str]:
    coverage = load_json(ROOT / "metrics/coverage.json")
    commit = coverage.get("reviewed_commit")
    tree = coverage.get("reviewed_tree")
    if coverage.get("passed") is not True or not isinstance(commit, str) or not isinstance(tree, str):
        raise ValueError("coverage does not provide a passing product binding")
    actual = subprocess.check_output(
        ["git", "show", "-s", "--format=%T", commit], cwd=ROOT, text=True
    ).strip()
    if actual != tree:
        raise ValueError("coverage commit/tree binding is invalid")
    return commit, tree


def mutation_spec_sha256() -> str:
    rows = [asdict(mutant) for mutant in MUTANTS]
    payload = json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()
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


def run_test(command: list[str], log_path: Path) -> tuple[int, str]:
    result = subprocess.run(
        command,
        cwd=SOURCE_ROOT,
        env={
            **os.environ,
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(BUILD_ROOT),
        },
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    log_path.parent.mkdir(parents=True, exist_ok=True)
    header = "$ " + " ".join(command) + f"\nexit_code={result.returncode}\n\n"
    log_path.write_text(header + result.stdout, encoding="utf-8")
    return result.returncode, result.stdout


def apply_mutant(mutant: Mutant) -> bytes:
    path = SOURCE_ROOT / mutant.file
    original = path.read_bytes()
    source = original.decode("utf-8")
    occurrences = source.count(mutant.old)
    if occurrences != 1:
        raise ValueError(
            f"{mutant.mutant_id} expected one replacement site, found {occurrences}"
        )
    path.write_text(source.replace(mutant.old, mutant.new, 1), encoding="utf-8")
    return original


def render_ledger(metric: dict[str, Any]) -> str:
    ledger_lines = [
        "Tier 3 generated test and mutation ledger",
        f"product_commit={metric['reviewed_commit']}",
        f"product_tree={metric['reviewed_tree']}",
        "",
        "Baseline focused tests:",
    ]
    for row in metric["baseline"]:
        ledger_lines.append(
            f"PASS {row['test']} exit={row['return_code']} log={row['log']} sha256={row['log_sha256']}"
        )
    ledger_lines.append("")
    ledger_lines.append("Mutants:")
    for row in metric["mutants"]:
        ledger_lines.append(
            f"KILLED {row['mutant_id']} by={row['test']} exit={row['return_code']} "
            f"log={row['log']} sha256={row['log_sha256']}"
        )
    ledger_lines.extend(
        [
            "",
            f"mutation_denominator={metric['mutants_total']}",
            f"mutants_killed={metric['mutants_killed']}",
            f"surviving_mutants={','.join(metric['surviving_mutants']) or 'none'}",
            f"mutation_score_percent={metric['mutation_score_percent']}",
        ]
    )
    ledger_lines.extend(["", "Exact gate artifacts:"])
    for row in metric["gate_artifacts"]:
        ledger_lines.append(
            f"PASS {row['label']} path={row['path']} bytes={row['bytes']} "
            f"sha256={row['sha256']}"
        )
    return "\n".join(ledger_lines) + "\n"


def render_report(metric: dict[str, Any]) -> str:
    report_lines = [
        "# Tier 3 Mutation Gate",
        "",
        f"Reviewed product: `{metric['reviewed_commit']}`",
        "",
        f"The generated campaign ran {metric['mutants_total']} focused baseline tests "
        f"and {metric['mutants_total']} declared mutants. Every baseline passed and "
        "every mutant was killed.",
        "",
        f"- Mutation denominator: {metric['mutants_total']}",
        f"- Killed: {metric['mutants_killed']}",
        f"- Survived: {len(metric['surviving_mutants'])}",
        f"- Score: {metric['mutation_score_percent']}%",
        "",
        "The machine-readable denominator, survivor inventory, commands, return codes, "
        "and log hashes are in `metrics/t3_mutation.json`. The gate rejects a missing "
        "log, changed hash, baseline failure, survivor, or score below 100%.",
    ]
    return "\n".join(report_lines) + "\n"


def write_human_artifacts(metric: dict[str, Any]) -> None:
    LEDGER.parent.mkdir(parents=True, exist_ok=True)
    LEDGER.write_text(render_ledger(metric), encoding="utf-8")
    REPORT.write_text(render_report(metric), encoding="utf-8")


def generate(jobs: int) -> int:
    if not 1 <= jobs <= 24:
        raise ValueError("jobs must be in 1..=24")
    commit, tree = product_binding()
    materialize_product(commit)
    LOG_ROOT.mkdir(parents=True, exist_ok=True)
    for old_log in LOG_ROOT.glob("generated-*.log"):
        old_log.unlink()

    baseline: list[dict[str, Any]] = []
    for mutant in MUTANTS:
        command = command_for(mutant, jobs)
        log = LOG_ROOT / f"generated-baseline-{mutant.mutant_id.lower()}.log"
        return_code, _ = run_test(command, log)
        if return_code != 0:
            raise RuntimeError(f"baseline failed for {mutant.mutant_id}")
        baseline.append(
            {
                "test": mutant.test,
                "command": command,
                "return_code": return_code,
                "log": str(log.relative_to(ROOT)),
                "log_sha256": sha256_file(log),
            }
        )

    rows: list[dict[str, Any]] = []
    survivors: list[str] = []
    for mutant in MUTANTS:
        path = SOURCE_ROOT / mutant.file
        original = apply_mutant(mutant)
        try:
            command = command_for(mutant, jobs)
            log = LOG_ROOT / f"generated-mutant-{mutant.mutant_id.lower()}.log"
            return_code, output = run_test(command, log)
        finally:
            path.write_bytes(original)
        killed = return_code != 0 and ("FAILED" in output or "failed" in output.lower())
        if not killed:
            survivors.append(mutant.mutant_id)
        rows.append(
            {
                "mutant_id": mutant.mutant_id,
                "file": mutant.file,
                "test": mutant.test,
                "command": command,
                "return_code": return_code,
                "status": "killed" if killed else "survived",
                "log": str(log.relative_to(ROOT)),
                "log_sha256": sha256_file(log),
            }
        )

    killed_count = len(MUTANTS) - len(survivors)
    score = killed_count * 100.0 / len(MUTANTS)
    gate_artifacts = []
    for label, relative in GATE_ARTIFACTS:
        path = ROOT / relative
        if not path.is_file():
            raise RuntimeError(f"missing exact gate artifact {path}")
        gate_artifacts.append(
            {
                "label": label,
                "path": relative,
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
        )
    metric: dict[str, Any] = {
        "schema_version": 1,
        "generated_at": utc_now(),
        "generator": "tools/run_t3_mutation_gate.py",
        "reviewed_commit": commit,
        "reviewed_tree": tree,
        "mutation_spec_sha256": mutation_spec_sha256(),
        "baseline": baseline,
        "mutants": rows,
        "mutants_total": len(MUTANTS),
        "mutants_killed": killed_count,
        "surviving_mutants": survivors,
        "mutation_score_percent": score,
        "minimum_score_percent": 100.0,
        "gate_artifacts": gate_artifacts,
        "constraints": {
            "local_only": True,
            "network_used": False,
            "github_actions_used": False,
            "installs_performed": False,
            "push_performed": False,
            "workers_used": jobs,
        },
    }
    write_human_artifacts(metric)
    metric["test_ledger"] = str(LEDGER.relative_to(ROOT))
    metric["test_ledger_sha256"] = sha256_file(LEDGER)
    write_json(METRIC, metric)
    if survivors:
        raise RuntimeError(f"surviving mutants: {', '.join(survivors)}")
    print(f"PASS T3 mutation gate: killed={killed_count}/{len(MUTANTS)} score={score:.1f}%")
    return 0


def check() -> int:
    commit, tree = product_binding()
    metric = load_json(METRIC)
    if metric.get("schema_version") != 1 or metric.get("generator") != "tools/run_t3_mutation_gate.py":
        raise ValueError("mutation metric has invalid schema or generator")
    if metric.get("reviewed_commit") != commit or metric.get("reviewed_tree") != tree:
        raise ValueError("mutation metric is bound to a stale product")
    if metric.get("mutation_spec_sha256") != mutation_spec_sha256():
        raise ValueError("mutation specification hash is stale")
    if metric.get("mutants_total") != len(MUTANTS):
        raise ValueError("mutation denominator is stale")
    if metric.get("mutants_killed") != len(MUTANTS):
        raise ValueError("not every declared mutant was killed")
    if metric.get("surviving_mutants") != [] or metric.get("mutation_score_percent") != 100.0:
        raise ValueError("mutation gate has survivors or a score below 100%")
    baseline = metric.get("baseline")
    mutants = metric.get("mutants")
    if not isinstance(baseline, list) or len(baseline) != len(MUTANTS):
        raise ValueError("mutation baseline ledger is incomplete")
    if not isinstance(mutants, list) or len(mutants) != len(MUTANTS):
        raise ValueError("mutation result ledger is incomplete")
    for row in baseline:
        if row.get("return_code") != 0:
            raise ValueError("a baseline focused test did not pass")
    for row in mutants:
        if row.get("status") != "killed" or row.get("return_code") == 0:
            raise ValueError("a declared mutant survived")
    for row in [*baseline, *mutants]:
        log = ROOT / row["log"]
        if not log.is_file() or sha256_file(log) != row.get("log_sha256"):
            raise ValueError(f"mutation log hash mismatch: {log}")
    artifacts = metric.get("gate_artifacts")
    if not isinstance(artifacts, list) or len(artifacts) != len(GATE_ARTIFACTS):
        raise ValueError("exact gate artifact ledger is incomplete")
    if [(row.get("label"), row.get("path")) for row in artifacts] != list(GATE_ARTIFACTS):
        raise ValueError("exact gate artifact ledger is stale")
    for row in artifacts:
        path = ROOT / row["path"]
        if (
            not path.is_file()
            or path.stat().st_size != row.get("bytes")
            or sha256_file(path) != row.get("sha256")
        ):
            raise ValueError(f"exact gate artifact hash mismatch: {path}")
    ledger = ROOT / str(metric.get("test_ledger", ""))
    if not ledger.is_file() or sha256_file(ledger) != metric.get("test_ledger_sha256"):
        raise ValueError("generated test ledger is missing or stale")
    if ledger.read_text(encoding="utf-8") != render_ledger(metric):
        raise ValueError("generated test ledger content is stale")
    if not REPORT.is_file() or REPORT.read_text(encoding="utf-8") != render_report(metric):
        raise ValueError("generated mutation report is missing or stale")
    print(
        "PASS T3 mutation evidence: "
        f"denominator={len(MUTANTS)} killed={len(MUTANTS)} survivors=0 score=100%"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        return check() if args.check else generate(args.jobs)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_t3_mutation_gate.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
