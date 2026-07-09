#!/usr/bin/env python3
"""Create and verify an exact-commit local CP-DSL evidence packet."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


REQUIRED_COMMAND_LOGS = [
    "00-preflight.log",
    "01-fmt-workspace.log",
    "02-fmt-fuzz.log",
    "03-clippy.log",
    "04-tests.log",
    "05-deny.log",
    "06-platforms.log",
    "07-fuzz.log",
    "08-mutation.log",
    "09-card-regression.log",
    "10-platform-validate.log",
    "11-oracle-semantics.log",
    "12-cp-dsl-metrics.log",
    "13-bootstrap.log",
    "14-archive-bootstrap.log",
    "15-local-verify.log",
]

REQUIRED_ARTIFACTS = [
    "assets/carddb.bin",
    "assets/carddb.index.json",
    "assets/layer_scenarios.carddb.bin",
    "assets/layer_scenarios.carddb.index.json",
    "cards/cp_dsl/malformed/manifest.json",
    "metrics/card_catalog.json",
    "metrics/cp_dsl_corpus.json",
    "metrics/cp_dsl_mutation.json",
    "metrics/cp_dsl_verification.json",
    "metrics/coverage.json",
    "metrics/local_fuzz.json",
    "metrics/local_platforms.json",
    "metrics/oracle_semantics.json",
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def command_output(command: list[str], root: Path) -> str:
    result = subprocess.run(
        command,
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode, command, output=result.stdout, stderr=result.stderr
        )
    return result.stdout.strip()


def file_record(root: Path, path: Path) -> dict[str, object]:
    if not path.is_file():
        raise ValueError(f"required evidence file is absent: {path}")
    return {
        "path": str(path.resolve().relative_to(root.resolve())),
        "sha256": sha256(path),
        "size_bytes": path.stat().st_size,
    }


def resolve_path(root: Path, value: object) -> Path:
    path = Path(str(value))
    return path if path.is_absolute() else root / path


def validated_reports(root: Path) -> dict[str, dict[str, object]]:
    reports = {
        "mutation": load_json(root / "metrics/cp_dsl_mutation.json"),
        "fuzz": load_json(root / "metrics/local_fuzz.json"),
        "platforms": load_json(root / "metrics/local_platforms.json"),
        "oracles": load_json(root / "metrics/oracle_semantics.json"),
        "verification": load_json(root / "metrics/cp_dsl_verification.json"),
        "malformed": load_json(root / "cards/cp_dsl/malformed/manifest.json"),
        "coverage": load_json(root / "metrics/coverage.json"),
    }
    if reports["mutation"].get("passed") is not True:
        raise ValueError("mutation report did not pass")
    control = reports["mutation"].get("control")
    if not isinstance(control, dict) or control.get("status") != "passed":
        raise ValueError("mutation report lacks a passing unmutated control")
    if reports["fuzz"].get("passed") is not True:
        raise ValueError("fuzz report did not pass")
    if int(reports["fuzz"].get("total_worker_seconds", 0)) < 2400:
        raise ValueError("fuzz report lacks 2,400 verified worker-seconds")
    if reports["platforms"].get("passed") is not True:
        raise ValueError("platform report did not pass")
    if len(reports["platforms"].get("targets", [])) != 4:
        raise ValueError("platform report does not contain four executed targets")
    if reports["oracles"].get("passed") is not True:
        raise ValueError("semantic oracle report did not pass")
    if reports["verification"].get("passed") is not True:
        raise ValueError("CP-DSL verification report did not pass")
    if reports["coverage"].get("passed") is not True:
        raise ValueError("coverage report did not pass")
    if int(reports["malformed"].get("recursive_argument_case_count", 0)) < 50:
        raise ValueError("fewer than 50 recursive-argument diagnostics are recorded")
    if reports["malformed"].get("missing_argument_kinds") != []:
        raise ValueError("recursive-argument diagnostics omit an argument kind")
    return reports


def command_records(root: Path, evidence_dir: Path) -> list[dict[str, object]]:
    commands = evidence_dir / "commands"
    rows = []
    for filename in REQUIRED_COMMAND_LOGS:
        path = commands / filename
        record = file_record(root, path)
        text = path.read_text()
        if filename == "00-preflight.log":
            for marker in ("clean=true", "detached=true", "reviewed_commit="):
                if marker not in text:
                    raise ValueError(f"preflight log lacks {marker}")
        elif "exit_code=0" not in text:
            raise ValueError(f"command log does not record success: {path}")
        rows.append(record)
    return rows


def isolated_targets(reports: dict[str, dict[str, object]]) -> list[str]:
    targets = set()
    control = reports["mutation"].get("control", {})
    if isinstance(control, dict):
        targets.add(str(control.get("target_directory", "")))
    for row in reports["mutation"].get("mutants", []):
        if isinstance(row, dict):
            targets.add(str(row.get("target_directory", "")))
    for row in reports["fuzz"].get("workers", []):
        if isinstance(row, dict):
            targets.add(str(row.get("target_directory", "")))
    for row in reports["platforms"].get("targets", []):
        if isinstance(row, dict):
            targets.add(str(row.get("target_dir", "")))
    targets.update(f"target/card-regression/isolated-{index}" for index in range(1, 4))
    targets.discard("")
    return sorted(targets)


def create(
    root: Path,
    evidence_dir: Path,
    reviewed_commit: str,
    started_at: str,
) -> int:
    current_commit = command_output(["git", "rev-parse", "HEAD"], root)
    if current_commit != reviewed_commit:
        raise ValueError(
            f"reviewed commit changed during the gate: {reviewed_commit} -> {current_commit}"
        )
    reports = validated_reports(root)
    for name in ("mutation", "fuzz"):
        if reports[name].get("reviewed_commit") != reviewed_commit:
            raise ValueError(f"{name} evidence is not bound to {reviewed_commit}")
    if reports["coverage"].get("reviewed_commit") != reviewed_commit:
        raise ValueError("coverage evidence is not bound to the reviewed commit")
    command_rows = command_records(root, evidence_dir)
    artifact_rows = [file_record(root, root / path) for path in REQUIRED_ARTIFACTS]
    workflows = sorted((root / ".github/workflows").glob("*.y*ml"))
    if workflows:
        raise ValueError(f"hosted workflows are active: {workflows}")
    packet: dict[str, object] = {
        "schema_version": 1,
        "passed": True,
        "runner": "local-only",
        "github_actions_used": False,
        "reviewed_commit": reviewed_commit,
        "reviewed_tree": command_output(
            ["git", "rev-parse", f"{reviewed_commit}^{{tree}}"], root
        ),
        "detached_clean_start": True,
        "started_at": started_at,
        "finished_at": utc_now(),
        "toolchains": {
            "rustc": command_output(["rustc", "--version"], root),
            "cargo": command_output(["cargo", "--version"], root),
            "rustup": command_output(["rustup", "--version"], root).splitlines()[0],
            "cargo_deny": command_output(["cargo", "deny", "--version"], root),
            "cargo_llvm_cov": command_output(["cargo", "llvm-cov", "--version"], root),
            "cargo_fuzz": command_output(["cargo", "fuzz", "--version"], root),
        },
        "commands": command_rows,
        "artifacts": artifact_rows,
        "isolated_target_directories": isolated_targets(reports),
        "source_bindings": {
            "mutation": reports["mutation"].get("source_sha256"),
            "fuzz": reports["fuzz"].get("source_sha256"),
            "platforms": reports["platforms"].get("source_sha256"),
            "oracles": reports["oracles"].get("source_sha256"),
            "coverage": reports["coverage"].get("source_sha256"),
            "catalog": reports["verification"].get("provenance"),
        },
        "acceptance": {
            "recursive_argument_diagnostics": reports["malformed"].get(
                "recursive_argument_case_count"
            ),
            "mutation_control": "passed",
            "mutants_killed": reports["mutation"].get("killed"),
            "verified_fuzz_worker_seconds": reports["fuzz"].get(
                "total_worker_seconds"
            ),
            "cross_targets": len(reports["platforms"].get("targets", [])),
            "semantic_oracles": reports["oracles"].get("measured"),
        },
    }
    output = evidence_dir / "packet.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(packet, indent=2) + "\n")
    print(
        f"PASS CP-DSL evidence packet commit={reviewed_commit} "
        f"commands={len(command_rows)} artifacts={len(artifact_rows)}"
    )
    return 0


def check(root: Path, evidence_dir: Path) -> int:
    packet_path = evidence_dir / "packet.json"
    packet = load_json(packet_path)
    if packet.get("passed") is not True or packet.get("github_actions_used") is not False:
        raise ValueError("evidence packet is not a passing local-only packet")
    reviewed_commit = str(packet.get("reviewed_commit", ""))
    if not reviewed_commit:
        raise ValueError("evidence packet has no reviewed commit")
    reviewed_tree = command_output(["git", "rev-parse", f"{reviewed_commit}^{{tree}}"], root)
    if packet.get("reviewed_tree") != reviewed_tree:
        raise ValueError("reviewed tree hash does not match the reviewed commit")
    reports = validated_reports(root)
    for name in ("mutation", "fuzz"):
        if reports[name].get("reviewed_commit") != reviewed_commit:
            raise ValueError(f"{name} evidence is not bound to the reviewed commit")
    if reports["coverage"].get("reviewed_commit") != reviewed_commit:
        raise ValueError("coverage evidence is not bound to the reviewed commit")
    expected_commands = command_records(root, evidence_dir)
    if packet.get("commands") != expected_commands:
        raise ValueError("command-log manifest is stale")
    expected_artifacts = [file_record(root, root / path) for path in REQUIRED_ARTIFACTS]
    if packet.get("artifacts") != expected_artifacts:
        raise ValueError("artifact manifest is stale")
    command_output(["python3", "tools/run_cp_dsl_mutation.py", "--check"], root)
    command_output(
        [
            "python3",
            "tools/run_local_fuzz.py",
            "--check",
            "--minimum-worker-seconds",
            "2400",
        ],
        root,
    )
    if sorted((root / ".github/workflows").glob("*.y*ml")):
        raise ValueError("hosted workflows became active after packet creation")
    print(
        f"PASS CP-DSL evidence packet check commit={reviewed_commit} "
        f"commands={len(expected_commands)} artifacts={len(expected_artifacts)}"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--evidence-dir", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--create", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--reviewed-commit")
    parser.add_argument("--started-at")
    args = parser.parse_args()
    evidence_dir = args.evidence_dir or args.root / "reports/gates/CP-DSL/evidence"
    try:
        if args.create:
            if not args.reviewed_commit or not args.started_at:
                raise ValueError("--create requires --reviewed-commit and --started-at")
            return create(args.root, evidence_dir, args.reviewed_commit, args.started_at)
        return check(args.root, evidence_dir)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"cp_dsl_evidence_packet.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
