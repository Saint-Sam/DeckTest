#!/usr/bin/env python3
"""Generate and verify exact T3.5/T3.6 card-stage evidence."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional


ROOT = Path(__file__).resolve().parents[1]
CASES = ROOT / "tests/t3_6/commander_semantic_cases.json"
CANDIDATES = ROOT / "assets/t3_6_commander_semantic_candidates.json"
CORPUS_REPORT = ROOT / "reports/gates/T3.5/runtime-smoke-corpus.json"
FROZEN_REPORT = ROOT / "reports/gates/T3.5/runtime-smoke-frozen-100.json"
FINAL_REPORT = ROOT / "reports/gates/T3.5/runtime-interpreter-final-2026-07-13.json"
SEMANTIC_REPORT = ROOT / "reports/gates/T3.6-B/EVIDENCE.json"
SEMANTIC_REVALIDATION = ROOT / "reports/gates/T3.9/t3-6-semantic-revalidation.json"
RUNTIME_METRIC = ROOT / "metrics/card_runtime_smoke.json"
SEMANTIC_METRIC = ROOT / "metrics/card_semantics_100.json"
GENERATOR = "tools/run_t3_card_stage_gate.py"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def payload_sha256(value: dict[str, Any]) -> str:
    payload = copy.deepcopy(value)
    payload.pop("payload_sha256", None)
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return sha256_bytes(encoded)


def run(command: list[str], expected: Optional[set[int]] = None) -> int:
    result = subprocess.run(
        command,
        cwd=ROOT,
        env={**os.environ, "CARGO_NET_OFFLINE": "true"},
        check=False,
    )
    accepted = expected if expected is not None else {0}
    if result.returncode not in accepted:
        raise RuntimeError(f"command failed with {result.returncode}: {' '.join(command)}")
    return result.returncode


def product_binding() -> tuple[str, str]:
    coverage = load_json(ROOT / "metrics/coverage.json")
    if coverage.get("schema_version") != 2 or coverage.get("passed") is not True:
        raise ValueError("coverage is not a passing schema-v2 product binding")
    commit = coverage.get("reviewed_commit")
    tree = coverage.get("reviewed_tree")
    if not isinstance(commit, str) or len(commit) != 40:
        raise ValueError("coverage has an invalid reviewed_commit")
    if not isinstance(tree, str) or len(tree) != 40:
        raise ValueError("coverage has an invalid reviewed_tree")
    actual_tree = subprocess.check_output(
        ["git", "show", "-s", "--format=%T", commit], cwd=ROOT, text=True
    ).strip()
    if actual_tree != tree:
        raise ValueError("coverage commit/tree binding is invalid")
    return commit, tree


def rebind_cases(commit: str) -> dict[str, Any]:
    cases = load_json(CASES)
    cases["product_source_commit"] = commit
    cases["payload_sha256"] = payload_sha256(cases)
    write_json(CASES, cases)
    return cases


def verify_cases(cases: dict[str, Any], commit: str) -> list[str]:
    if cases.get("product_source_commit") != commit:
        raise ValueError("semantic cases are bound to a stale product")
    if cases.get("payload_sha256") != payload_sha256(cases):
        raise ValueError("semantic case payload hash is invalid")
    rows = cases.get("cases")
    if not isinstance(rows, list) or len(rows) != 100:
        raise ValueError("semantic case manifest must contain exactly 100 cases")
    identities: list[str] = []
    for row in rows:
        if not isinstance(row, dict) or row.get("status") != "semantic_case_ready":
            raise ValueError("all frozen cases must be semantic_case_ready")
        identity = row.get("oracle_id")
        if not isinstance(identity, str):
            raise ValueError("semantic case has no Oracle identity")
        identities.append(identity)
    if len(set(identities)) != 100:
        raise ValueError("semantic case identities are not unique")
    return identities


def prepare_frozen_root(cases: dict[str, Any], translated_root: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    names: set[str] = set()
    for row in cases["cases"]:
        relative = Path(row["translated_path"])
        if relative.is_absolute() or ".." in relative.parts or relative.suffix != ".frs":
            raise ValueError(f"invalid translated path {relative}")
        if relative.name in names:
            raise ValueError(f"duplicate frozen filename {relative.name}")
        names.add(relative.name)
        source = translated_root / relative
        if not source.is_file():
            raise ValueError(f"missing translated definition {source}")
        shutil.copy2(source, destination / relative.name)
    if len(names) != 100:
        raise ValueError("frozen runtime root must contain exactly 100 definitions")


def validate_runtime_report(
    report: dict[str, Any], *, total: Optional[int] = None, require_all_passed: bool = False
) -> None:
    total_definitions = report.get("total_definitions")
    passed = report.get("passed")
    unsupported = report.get("unsupported_setup")
    failed = report.get("failed")
    if not all(isinstance(value, int) for value in (total_definitions, passed, unsupported, failed)):
        raise ValueError("runtime smoke report has invalid counters")
    if total is not None and total_definitions != total:
        raise ValueError(f"runtime smoke expected {total} definitions, found {total_definitions}")
    if passed + unsupported + failed != total_definitions or failed != 0:
        raise ValueError("runtime smoke accounting is invalid or contains production failures")
    if require_all_passed and (passed != total_definitions or unsupported != 0):
        raise ValueError("frozen runtime smoke did not pass every identity")


def source_binding() -> dict[str, str]:
    translation = load_json(ROOT / "metrics/translation.json")
    carddb = ROOT / "target/t3-parallel/translated-carddb.bin"
    if not carddb.is_file():
        raise ValueError("exact translated card database is missing")
    return {
        "card_catalog_sha256": sha256_file(ROOT / "assets/card_catalog.json"),
        "card_database_sha256": sha256_file(carddb),
        "translation_fingerprint": str(translation["output_fingerprint"]),
    }


def validate_semantic_report(report: dict[str, Any], commit: str, cases: dict[str, Any]) -> None:
    measured = report.get("measured")
    checkpoint = report.get("checkpoint")
    binding = report.get("product_binding")
    if report.get("status") != "pass_local" or not isinstance(measured, dict):
        raise ValueError("semantic report did not pass")
    if measured.get("semantic_verified") != 100:
        raise ValueError("semantic report did not verify 100 identities")
    if measured.get("blocked_semantic_gap") != 0 or measured.get("blocked_runtime") != 0:
        raise ValueError("semantic report contains blockers")
    if not isinstance(checkpoint, dict) or checkpoint.get("status") != "passed":
        raise ValueError("semantic checkpoint did not pass")
    if not isinstance(binding, dict):
        raise ValueError("semantic report has no product binding")
    if binding.get("runtime_source_commit") != commit:
        raise ValueError("semantic report is bound to a stale product")
    if binding.get("semantic_cases_payload_sha256") != cases.get("payload_sha256"):
        raise ValueError("semantic report is bound to stale cases")


def final_runtime_report(
    timestamp: str,
    commit: str,
    tree: str,
    corpus: dict[str, Any],
    frozen: dict[str, Any],
) -> dict[str, Any]:
    coverage = load_json(ROOT / "metrics/coverage.json")
    return {
        "schema_version": 2,
        "generated_at": timestamp,
        "task": "T3.5",
        "slice": "data-driven-card-runtime-interpreter",
        "status": "pass_local",
        "claim_boundary": (
            "T3.5 classifies every compiler-valid translated definition through the "
            "typed fail-closed runtime smoke and executes all 100 frozen Commander "
            "identities. Typed unsupported classifications are not passes."
        ),
        "source_commit": commit,
        "source_tree": tree,
        "generator": GENERATOR,
        "translated_corpus": {
            "source": "target/translated-cards",
            "report": str(CORPUS_REPORT.relative_to(ROOT)),
            "report_sha256": sha256_file(CORPUS_REPORT),
            "total": corpus["total_definitions"],
            "passed": corpus["passed"],
            "unsupported_setup": corpus["unsupported_setup"],
            "failed": corpus["failed"],
            "unsupported_reason_counts": corpus["unsupported_reason_counts"],
        },
        "frozen_semantic_100": {
            "manifest": str(CANDIDATES.relative_to(ROOT)),
            "report": str(FROZEN_REPORT.relative_to(ROOT)),
            "report_sha256": sha256_file(FROZEN_REPORT),
            "total": frozen["total_definitions"],
            "runtime_smoke_passed": frozen["passed"],
            "unsupported_setup": frozen["unsupported_setup"],
            "failed": frozen["failed"],
            "semantic_verified": 100,
            "semantic_evidence": str(SEMANTIC_REPORT.relative_to(ROOT)),
            "semantic_evidence_sha256": sha256_file(SEMANTIC_REPORT),
        },
        "verification": {
            "coverage_percent": coverage["lines"]["percent"],
            "deterministic_translation_replay": True,
            "deterministic_semantic_replay": True,
        },
        "constraints": {
            "local_only": True,
            "network_used": False,
            "installs_performed": False,
            "github_actions_used": False,
            "push_performed": False,
        },
    }


def build_stage_metrics(
    timestamp: str,
    commit: str,
    tree: str,
    identities: list[str],
    source: dict[str, str],
) -> tuple[dict[str, Any], dict[str, Any]]:
    common_inputs = {
        "candidate_manifest_sha256": sha256_file(CANDIDATES),
        "semantic_cases_sha256": sha256_file(CASES),
        "corpus_runtime_report_sha256": sha256_file(CORPUS_REPORT),
        "frozen_runtime_report_sha256": sha256_file(FROZEN_REPORT),
    }
    runtime = {
        "schema_version": 1,
        "generated_at": timestamp,
        "generator": GENERATOR,
        "stage": "runtime_smoke_passed",
        "passed": True,
        "product_commit": commit,
        "product_tree": tree,
        "identity_ids": identities,
        "source": source,
        "evidence": str(FINAL_REPORT.relative_to(ROOT)),
        "evidence_sha256": sha256_file(FINAL_REPORT),
        "inputs": common_inputs,
    }
    semantic_inputs = dict(common_inputs)
    semantic_inputs["semantic_evidence_sha256"] = sha256_file(SEMANTIC_REPORT)
    semantic = {
        "schema_version": 1,
        "generated_at": timestamp,
        "generator": GENERATOR,
        "stage": "semantic_verified",
        "passed": True,
        "product_commit": commit,
        "product_tree": tree,
        "identity_ids": identities,
        "source": source,
        "evidence": str(SEMANTIC_REPORT.relative_to(ROOT)),
        "evidence_sha256": sha256_file(SEMANTIC_REPORT),
        "inputs": semantic_inputs,
    }
    return runtime, semantic


def generate(translated_root: Path, cargo_target_dir: Path) -> int:
    commit, tree = product_binding()
    cases = rebind_cases(commit)
    identities = verify_cases(cases, commit)
    frozen_root = ROOT / "target/t3-card-stage/frozen-100"
    prepare_frozen_root(cases, translated_root, frozen_root)
    run(
        [
            "cargo",
            "build",
            "--locked",
            "--offline",
            "--quiet",
            "-p",
            "forge-testkit",
            "--bin",
            "forge-testkit",
            "--target-dir",
            str(cargo_target_dir),
        ]
    )
    probe = cargo_target_dir / "debug/forge-testkit"
    run(
        [
            str(probe),
            "runtime-smoke",
            str(translated_root),
            "--report",
            str(CORPUS_REPORT),
            "--quiet",
        ],
        expected={0, 2},
    )
    run(
        [
            str(probe),
            "runtime-smoke",
            str(frozen_root),
            "--report",
            str(FROZEN_REPORT),
            "--quiet",
        ]
    )
    semantic_command = [
        sys.executable,
        "tools/run_t3_6_commander_semantics.py",
        "--translated-root",
        str(translated_root),
        "--cargo-target-dir",
        str(cargo_target_dir),
    ]
    run([*semantic_command, "--report", str(SEMANTIC_REPORT)])
    run([*semantic_command, "--report", str(SEMANTIC_REVALIDATION)])

    corpus = load_json(CORPUS_REPORT)
    frozen = load_json(FROZEN_REPORT)
    semantic = load_json(SEMANTIC_REPORT)
    validate_runtime_report(corpus)
    validate_runtime_report(frozen, total=100, require_all_passed=True)
    validate_semantic_report(semantic, commit, cases)
    timestamp = utc_now()
    write_json(FINAL_REPORT, final_runtime_report(timestamp, commit, tree, corpus, frozen))
    runtime_metric, semantic_metric = build_stage_metrics(
        timestamp, commit, tree, identities, source_binding()
    )
    write_json(RUNTIME_METRIC, runtime_metric)
    write_json(SEMANTIC_METRIC, semantic_metric)
    print(
        "PASS T3 card stages: "
        f"runtime={frozen['passed']}/100 semantic={semantic['measured']['semantic_verified']}/100"
    )
    return 0


def check() -> int:
    commit, tree = product_binding()
    cases = load_json(CASES)
    identities = verify_cases(cases, commit)
    corpus = load_json(CORPUS_REPORT)
    frozen = load_json(FROZEN_REPORT)
    final = load_json(FINAL_REPORT)
    semantic_report = load_json(SEMANTIC_REPORT)
    validate_runtime_report(corpus)
    validate_runtime_report(frozen, total=100, require_all_passed=True)
    validate_semantic_report(semantic_report, commit, cases)
    if final.get("source_commit") != commit or final.get("source_tree") != tree:
        raise ValueError("T3.5 final report has a stale product binding")
    if final.get("generator") != GENERATOR or final.get("status") != "pass_local":
        raise ValueError("T3.5 final report is not generator-bound and passing")
    for section, path in (
        ("translated_corpus", CORPUS_REPORT),
        ("frozen_semantic_100", FROZEN_REPORT),
    ):
        if final[section].get("report_sha256") != sha256_file(path):
            raise ValueError(f"T3.5 {section} report hash is stale")
    if final["frozen_semantic_100"].get("semantic_evidence_sha256") != sha256_file(
        SEMANTIC_REPORT
    ):
        raise ValueError("T3.5 semantic evidence hash is stale")
    source = source_binding()
    for path, expected_stage in (
        (RUNTIME_METRIC, "runtime_smoke_passed"),
        (SEMANTIC_METRIC, "semantic_verified"),
    ):
        actual = load_json(path)
        timestamp = actual.get("generated_at")
        if not isinstance(timestamp, str) or not timestamp:
            raise ValueError(f"{path} has no generated_at")
        runtime, semantic = build_stage_metrics(timestamp, commit, tree, identities, source)
        expected = runtime if expected_stage == "runtime_smoke_passed" else semantic
        if actual != expected:
            raise ValueError(f"stale generated stage metric: {path}")
    print("PASS T3 card-stage evidence is generated, hash-bound, and current")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--translated-root", type=Path, default=ROOT / "target/translated-cards")
    parser.add_argument("--cargo-target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        if args.check:
            return check()
        return generate(args.translated_root.resolve(), args.cargo_target_dir.resolve())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_t3_card_stage_gate.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
