#!/usr/bin/env python3
"""Build and verify linked local cross-target artifacts."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


TARGETS = [
    {
        "target": "wasm32-unknown-unknown",
        "package": "forge-app-wasm",
        "crate_type": "cdylib",
        "extension": ".wasm",
        "magic": b"\0asm",
    },
    {
        "target": "aarch64-linux-android",
        "package": "forge-app-android",
        "crate_type": "staticlib",
        "extension": ".a",
        "magic": b"!<arch>\n",
    },
    {
        "target": "aarch64-apple-ios",
        "package": "forge-app-ios",
        "crate_type": "staticlib",
        "extension": ".a",
        "magic": b"!<arch>\n",
    },
    {
        "target": "x86_64-pc-windows-msvc",
        "package": "forge-app-desktop",
        "crate_type": "staticlib",
        "extension": ".lib",
        "magic": b"!<arch>\n",
    },
]


def source_hash(root: Path) -> str:
    paths = [
        root / name
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "tools/local_platform_metrics.py",
        )
    ]
    paths.extend(sorted((root / "crates").glob("*/Cargo.toml")))
    paths.extend(sorted((root / "crates").glob("*/src/*.rs")))
    digest = hashlib.sha256()
    for path in paths:
        digest.update(str(path.relative_to(root)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def display_path(root: Path, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(root.resolve()))
    except ValueError:
        return str(path.resolve())


def resolve_path(root: Path, value: object) -> Path:
    path = Path(str(value))
    return path if path.is_absolute() else root / path


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


def evidence_directory(root: Path) -> Path:
    configured = os.environ.get("FORGE_CP_DSL_EVIDENCE_DIR")
    if configured:
        return Path(configured) / "platforms"
    return root / "target/local-platforms/evidence/current"


def build_command(spec: dict[str, object]) -> list[str]:
    return [
        "cargo",
        "rustc",
        "--locked",
        "--offline",
        "--release",
        "-p",
        str(spec["package"]),
        "--target",
        str(spec["target"]),
        "--",
        f"--crate-type={spec['crate_type']}",
    ]


def locate_artifact(target_dir: Path, spec: dict[str, object]) -> Path:
    target = str(spec["target"])
    stem = str(spec["package"]).replace("-", "_")
    extension = str(spec["extension"])
    candidates = sorted(
        path
        for path in (target_dir / target / "release/deps").glob(f"*{stem}*{extension}")
        if path.is_file()
    )
    if len(candidates) != 1:
        raise ValueError(
            f"expected one linked {extension} artifact for {target}, found {candidates}"
        )
    artifact = candidates[0]
    expected_magic = bytes(spec["magic"])
    if artifact.stat().st_size == 0 or artifact.read_bytes()[: len(expected_magic)] != expected_magic:
        raise ValueError(f"linked artifact has invalid magic or size: {artifact}")
    return artifact


def build_target(
    root: Path,
    evidence_dir: Path,
    spec: dict[str, object],
) -> dict[str, object]:
    target = str(spec["target"])
    target_dir = root / "target/local-platforms" / target
    if target_dir.exists():
        shutil.rmtree(target_dir)
    target_dir.mkdir(parents=True)
    evidence_dir.mkdir(parents=True, exist_ok=True)
    command = build_command(spec)
    environment = os.environ.copy()
    environment["CARGO_NET_OFFLINE"] = "true"
    environment["CARGO_TARGET_DIR"] = str(target_dir)
    environment["CARGO_BUILD_JOBS"] = str(max(1, (os.cpu_count() or 4) // len(TARGETS)))
    started_at = utc_now()
    started = time.monotonic()
    result = subprocess.run(
        command,
        cwd=root,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    finished_at = utc_now()
    elapsed_seconds = round(time.monotonic() - started, 3)
    log_path = evidence_dir / f"{target}.log"
    log_path.write_text(
        f"started_at={started_at}\n"
        f"finished_at={finished_at}\n"
        f"elapsed_seconds={elapsed_seconds}\n"
        f"target_directory={target_dir}\n"
        f"command={json.dumps(command)}\n"
        f"exit_code={result.returncode}\n"
        "--- output ---\n"
        f"{result.stdout}"
    )
    if result.returncode != 0:
        tail = "\n".join(result.stdout.splitlines()[-80:])
        raise ValueError(f"linked build failed for {target}\n{tail}")
    artifact = locate_artifact(target_dir, spec)
    description = command_output(["file", "-b", str(artifact)], root)
    return {
        "target": target,
        "package": spec["package"],
        "crate_type": spec["crate_type"],
        "command": command,
        "target_dir": display_path(root, target_dir),
        "started_at": started_at,
        "finished_at": finished_at,
        "elapsed_seconds": elapsed_seconds,
        "exit_code": result.returncode,
        "log": display_path(root, log_path),
        "log_sha256": sha256(log_path),
        "artifact": {
            "path": display_path(root, artifact),
            "sha256": sha256(artifact),
            "size_bytes": artifact.stat().st_size,
            "magic_hex": bytes(spec["magic"]).hex(),
            "file_description": description,
        },
        "passed": True,
    }


def validate(root: Path, report: dict[str, object]) -> tuple[bool, str]:
    if report.get("schema_version") != 2:
        return False, "platform report schema is unsupported"
    current_commit = command_output(["git", "rev-parse", "HEAD"], root)
    current_tree = command_output(["git", "rev-parse", "HEAD^{tree}"], root)
    if report.get("reviewed_commit") != current_commit:
        return False, "platform report is not bound to the current commit"
    if report.get("reviewed_tree") != current_tree:
        return False, "platform report is not bound to the current tree"
    if report.get("source_sha256") != source_hash(root):
        return False, "platform report is stale for current source"
    rows = report.get("targets")
    if not isinstance(rows, list) or len(rows) != len(TARGETS):
        return False, "platform report does not contain four target records"
    expected = {str(spec["target"]): spec for spec in TARGETS}
    observed: set[str] = set()
    for row in rows:
        if not isinstance(row, dict):
            return False, "platform record is not an object"
        target = str(row.get("target", ""))
        spec = expected.get(target)
        if spec is None:
            return False, f"unexpected platform target {target}"
        if target in observed:
            return False, f"duplicate platform target {target}"
        observed.add(target)
        if row.get("package") != spec["package"] or row.get("crate_type") != spec["crate_type"]:
            return False, f"platform package metadata mismatch for {target}"
        if row.get("command") != build_command(spec):
            return False, f"platform command mismatch for {target}"
        if row.get("exit_code") != 0 or row.get("passed") is not True:
            return False, f"platform build did not pass for {target}"
        log_path = resolve_path(root, row.get("log", ""))
        if not log_path.is_file() or row.get("log_sha256") != sha256(log_path):
            return False, f"platform log is absent or changed for {target}"
        for marker in ("command=", "exit_code=0", "--- output ---"):
            if marker not in log_path.read_text():
                return False, f"platform log lacks {marker} for {target}"
        command_line = next(
            (
                line.removeprefix("command=")
                for line in log_path.read_text().splitlines()
                if line.startswith("command=")
            ),
            None,
        )
        if command_line is None or json.loads(command_line) != row.get("command"):
            return False, f"platform log command mismatch for {target}"
        artifact = row.get("artifact")
        if not isinstance(artifact, dict):
            return False, f"platform artifact record is absent for {target}"
        artifact_path = resolve_path(root, artifact.get("path", ""))
        if not artifact_path.is_file():
            return False, f"linked platform artifact is absent for {target}"
        if artifact.get("sha256") != sha256(artifact_path):
            return False, f"linked platform artifact hash changed for {target}"
        expected_magic = bytes(spec["magic"])
        if artifact.get("magic_hex") != expected_magic.hex():
            return False, f"linked platform artifact magic record changed for {target}"
        if artifact_path.read_bytes()[: len(expected_magic)] != expected_magic:
            return False, f"linked platform artifact magic changed for {target}"
        if artifact_path.stat().st_size <= 0 or int(artifact.get("size_bytes", 0)) != artifact_path.stat().st_size:
            return False, f"linked platform artifact size changed for {target}"
        if not artifact.get("file_description"):
            return False, f"linked platform artifact description is absent for {target}"
    if observed != set(expected):
        return False, "platform report does not cover the exact target set"
    if int(report.get("linked_artifact_count", 0)) != len(TARGETS):
        return False, "platform linked artifact count is incorrect"
    return True, "verified four linked artifacts, logs, and hashes"


def execute(root: Path) -> dict[str, object]:
    evidence_dir = evidence_directory(root)
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(TARGETS)) as executor:
        futures = {
            str(spec["target"]): executor.submit(build_target, root, evidence_dir, spec)
            for spec in TARGETS
        }
        rows = [futures[str(spec["target"])].result() for spec in TARGETS]
    return {
        "schema_version": 2,
        "reviewed_commit": command_output(["git", "rev-parse", "HEAD"], root),
        "reviewed_tree": command_output(["git", "rev-parse", "HEAD^{tree}"], root),
        "source_sha256": source_hash(root),
        "rustc": command_output(["rustc", "--version"], root),
        "evidence_directory": display_path(root, evidence_dir),
        "targets": rows,
        "linked_artifact_count": len(rows),
        "native_tests_and_benches_separate": True,
        "passed": all(row["passed"] for row in rows),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    output = args.root / "metrics/local_platforms.json"
    try:
        if args.check and args.validate_only:
            raise ValueError("--check and --validate-only are mutually exclusive")
        if args.check or args.validate_only:
            if not output.is_file():
                raise ValueError("local platform report is absent")
            report = json.loads(output.read_text())
            if not isinstance(report, dict):
                raise ValueError("local platform report is not an object")
            passed, reason = validate(args.root, report)
            if not passed or report.get("passed") is not True:
                raise ValueError(reason)
            print(f"Local platform report: {reason}")
            return 0
        report = execute(args.root)
        passed, reason = validate(args.root, report)
        report["passed"] = passed
        output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"Local platform report: {reason}")
        return 0 if passed else 1
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"local_platform_metrics.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
