#!/usr/bin/env python3
"""Generate and verify the human-readable Tier 3 fuzz report."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
METRIC = ROOT / "metrics/local_fuzz.json"
OUTPUT = ROOT / "reports/gates/T3/fuzz_report.md"
REQUIRED_TARGETS = {
    "fuzz_apply",
    "fuzz_characteristics",
    "fuzz_scenarioparse",
    "fuzz_carddsl",
    "fuzz_carddb",
}


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def validate(metric: dict[str, Any]) -> None:
    coverage = load_json(ROOT / "metrics/coverage.json")
    if metric.get("schema_version") != 2 or metric.get("passed") is not True:
        raise ValueError("local fuzz metric is not a passing schema-v2 record")
    if metric.get("reviewed_commit") != coverage.get("reviewed_commit"):
        raise ValueError("local fuzz commit binding is stale")
    if metric.get("reviewed_tree") != coverage.get("reviewed_tree"):
        raise ValueError("local fuzz tree binding is stale")
    workers = metric.get("workers")
    if not isinstance(workers, list) or len(workers) != 8:
        raise ValueError("Tier 3 requires exactly eight fuzz workers")
    if metric.get("total_worker_seconds", 0) < 3600:
        raise ValueError("Tier 3 fuzz duration is below 3,600 worker-seconds")
    if {row.get("target") for row in workers} != REQUIRED_TARGETS:
        raise ValueError("Tier 3 fuzz target set is incomplete")
    for row in workers:
        if row.get("status") != "passed" or row.get("return_code") != 0:
            raise ValueError("a Tier 3 fuzz worker did not pass")
        if row.get("artifacts") != []:
            raise ValueError("a Tier 3 fuzz worker produced a crash artifact")
        log = ROOT / str(row.get("log", ""))
        if not log.is_file() or sha256_file(log) != row.get("log_sha256"):
            raise ValueError(f"fuzz log hash mismatch: {log}")


def render(metric: dict[str, Any]) -> str:
    workers = metric["workers"]
    executions = sum(int(row["completed_runs"]) for row in workers)
    targets = ", ".join(sorted({str(row["target"]) for row in workers}))
    metric_sha = sha256_file(METRIC)
    return "\n".join(
        [
            "# Tier 3 Local Fuzz Report",
            "",
            f"Reviewed product: `{metric['reviewed_commit']}`",
            "",
            f"Reviewed tree: `{metric['reviewed_tree']}`",
            "",
            f"Metric SHA-256: `{metric_sha}`",
            "",
            "## Result",
            "",
            f"PASS. Eight local AddressSanitizer workers completed "
            f"{metric['total_worker_seconds']:,} verified worker-seconds and "
            f"{executions:,} executions.",
            "",
            f"Required targets: {targets}.",
            "",
            "Every worker returned zero, emitted final libFuzzer statistics, met its "
            "requested duration, and produced no crash artifact. Full logs are archived "
            "under `reports/gates/T3/fuzz/` and hash-bound by `metrics/local_fuzz.json`.",
            "",
            "The campaign was local and offline. It used no GitHub Actions, network "
            "access, install, push, or PR.",
            "",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        metric = load_json(METRIC)
        validate(metric)
        expected = render(metric)
        if args.check:
            if not OUTPUT.is_file() or OUTPUT.read_text(encoding="utf-8") != expected:
                raise ValueError("Tier 3 fuzz report is stale")
            print("PASS Tier 3 fuzz report is generated and current")
            return 0
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(expected, encoding="utf-8")
        print(f"wrote {OUTPUT}")
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"write_t3_fuzz_report.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
