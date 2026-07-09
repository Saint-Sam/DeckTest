#!/usr/bin/env python3
"""Record or validate exact-source local cross-target checks."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


TARGETS = [
    "wasm32-unknown-unknown",
    "aarch64-linux-android",
    "aarch64-apple-ios",
    "x86_64-pc-windows-msvc",
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


def target_record(root: Path, target: str) -> dict[str, object]:
    target_dir = root / "target/local-platforms" / target
    command = [
        "cargo",
        "check",
        "--locked",
        "--offline",
        "--workspace",
        "--all-features",
        "--target",
        target,
    ]
    return {
        "target": target,
        "command": " ".join(command),
        "target_dir": str(target_dir.relative_to(root)),
        "exit_code": 0,
        "passed": True,
    }


def check_target(root: Path, target: str) -> dict[str, object]:
    record = target_record(root, target)
    target_dir = root / str(record["target_dir"])
    command = str(record["command"]).split()
    environment = os.environ.copy()
    environment["CARGO_NET_OFFLINE"] = "true"
    environment["CARGO_TARGET_DIR"] = str(target_dir)
    environment["CARGO_BUILD_JOBS"] = str(max(1, (os.cpu_count() or 4) // len(TARGETS)))
    result = subprocess.run(
        command,
        cwd=root,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if result.returncode != 0:
        raise ValueError(
            f"cross-target check failed for {target}: {' '.join(command)}\n{result.stdout}"
        )
    return record


def report(root: Path, execute: bool) -> dict[str, object]:
    rustc = subprocess.run(
        ["rustc", "--version"],
        cwd=root,
        check=True,
        text=True,
        capture_output=True,
    ).stdout.strip()
    if execute:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(TARGETS)) as executor:
            futures = {target: executor.submit(check_target, root, target) for target in TARGETS}
            checks = [futures[target].result() for target in TARGETS]
    else:
        checks = [target_record(root, target) for target in TARGETS]
    return {
        "schema_version": 1,
        "source_sha256": source_hash(root),
        "rustc": rustc,
        "targets": checks,
        "native_tests_and_benches_separate": True,
        "passed": all(check["passed"] for check in checks),
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
        rendered = json.dumps(report(args.root, execute=not args.validate_only), indent=2) + "\n"
        if args.check or args.validate_only:
            if not output.exists() or output.read_text() != rendered:
                raise ValueError("local platform report is stale")
        else:
            output.write_text(rendered)
        action = "validated" if args.validate_only else "executed"
        print(f"Local platform report: {action} 4/4 targets successfully")
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"local_platform_metrics.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
