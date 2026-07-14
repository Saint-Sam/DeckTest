#!/usr/bin/env python3
"""Generate the T3.9 pod-integration maturity metric from exact evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REPORT = ROOT / "reports/gates/T3.9/cp-four-player-pod-2026-07-13.json"
DEFAULT_MANIFEST = ROOT / "assets/t3_9/integration_decks.json"
DEFAULT_OUTPUT = ROOT / "metrics/pod_integration.json"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git(*args: str) -> str:
    return subprocess.check_output(
        ["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL
    ).strip()


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def build_metric(report_path: Path, manifest_path: Path) -> dict[str, Any]:
    report = load_json(report_path)
    if report.get("schema_version") != 2 or report.get("status") != "passed":
        raise ValueError("pod report must be a passing schema-version 2 artifact")
    binding = report.get("product_binding")
    if not isinstance(binding, dict):
        raise ValueError("pod report is missing exact product_binding")
    commit = binding.get("commit")
    tree = binding.get("tree")
    if not isinstance(commit, str) or not isinstance(tree, str):
        raise ValueError("pod report product binding is malformed")
    if git("show", "-s", "--format=%T", commit) != tree:
        raise ValueError("pod report product commit/tree binding is invalid")

    configuration = report.get("configuration", {})
    requirements = configuration.get("semantic_identity_requirements")
    if not isinstance(requirements, dict) or len(requirements) != 21:
        raise ValueError("pod report must bind exactly 21 semantic mainboard identities")
    exercise = report.get("results", {}).get("identity_exercise")
    if not isinstance(exercise, dict):
        raise ValueError("pod report is missing the observed identity ledger")
    for identity, is_land in requirements.items():
        observed = exercise.get(identity)
        if not isinstance(observed, dict):
            raise ValueError(f"identity {identity} is absent from the observed ledger")
        if is_land:
            passed = observed.get("land_plays", 0) > 0
        else:
            passed = observed.get("casts", 0) > 0 and observed.get("resolutions", 0) > 0
        if not passed:
            raise ValueError(f"identity {identity} did not satisfy its runtime exercise")

    replay_dir = ROOT / str(configuration.get("replay_directory", ""))
    replay_paths = sorted(replay_dir.glob("pod-seed-*.frsreplay"))
    if len(replay_paths) != 10:
        raise ValueError(f"expected 10 retained pod replays, found {len(replay_paths)}")

    source = load_json(ROOT / "metrics/card_semantics_100.json").get("source", {})
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "generator": "tools/write_pod_integration.py",
        "stage": "pod_integration_verified",
        "passed": True,
        "product_commit": commit,
        "product_tree": tree,
        "identity_ids": sorted(requirements),
        "identity_exercise": {identity: exercise[identity] for identity in sorted(requirements)},
        "evidence": str(report_path.relative_to(ROOT)),
        "evidence_sha256": sha256(report_path),
        "manifest": str(manifest_path.relative_to(ROOT)),
        "manifest_sha256": sha256(manifest_path),
        "action_replays": [
            {
                "path": str(path.relative_to(ROOT)),
                "sha256": sha256(path),
            }
            for path in replay_paths
        ],
        "source": source,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    report = args.report.resolve()
    manifest = args.manifest.resolve()
    output = args.output.resolve()
    metric = build_metric(report, manifest)
    if args.check:
        current = load_json(output)
        metric.pop("generated_at", None)
        current.pop("generated_at", None)
        if current != metric:
            raise SystemExit(f"{output} is stale")
        return 0
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(metric, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
