#!/usr/bin/env python3
"""Write a portable, source-bound summary of LLVM coverage output."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_hash(root: Path) -> str:
    paths = {
        root / "Cargo.toml",
        root / "Cargo.lock",
        root / "rust-toolchain.toml",
        root / "scripts/check_coverage.sh",
        root / "tools/coverage_summary.py",
    }
    for base in (root / "crates", root / "tests"):
        paths.update(base.rglob("*.rs"))
        paths.update(base.rglob("*.ron"))
        paths.update(base.rglob("Cargo.toml"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        if not path.is_file():
            continue
        digest.update(str(path.relative_to(root)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def git_value(root: Path, value: str) -> str:
    if not (root / ".git").exists():
        environment_key = {
            "HEAD": "FORGE_ARCHIVE_SOURCE_COMMIT",
            "HEAD^{tree}": "FORGE_ARCHIVE_SOURCE_TREE",
        }.get(value)
        archived_value = os.environ.get(environment_key or "")
        if archived_value:
            return archived_value
        raise ValueError(f"archive coverage provenance is missing for {value}")
    result = subprocess.run(
        ["git", "rev-parse", value],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode,
            ["git", "rev-parse", value],
            output=result.stdout,
            stderr=result.stderr,
        )
    return result.stdout.strip()


def build_report(root: Path, raw_path: Path, floor: int) -> dict[str, object]:
    raw = json.loads(raw_path.read_text())
    totals = raw["data"][0]["totals"]
    lines = totals["lines"]
    count = int(lines["count"])
    covered = int(lines["covered"])
    percent = float(lines["percent"])
    return {
        "schema_version": 2,
        "passed": percent >= floor,
        "reviewed_commit": git_value(root, "HEAD"),
        "reviewed_tree": git_value(root, "HEAD^{tree}"),
        "source_sha256": source_hash(root),
        "raw_report_sha256": sha256(raw_path),
        "raw_report_path": str(raw_path.resolve().relative_to(root.resolve())),
        "floor_percent": floor,
        "lines": {
            "count": count,
            "covered": covered,
            "not_covered": count - covered,
            "percent": percent,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--floor", type=int)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = args.output or args.root / "metrics/coverage.json"
    try:
        existing = None
        if args.check:
            if not output.is_file():
                raise ValueError("coverage summary is absent")
            existing = json.loads(output.read_text())
            if not isinstance(existing, dict):
                raise ValueError("coverage summary is not an object")
            raw_value = existing.get("raw_report_path")
            floor = int(existing.get("floor_percent", 0))
        else:
            if args.raw is None or args.floor is None:
                raise ValueError("--raw and --floor are required unless --check is used")
            raw_value = args.raw
            floor = args.floor
        raw_candidate = Path(str(raw_value))
        raw_path = raw_candidate if raw_candidate.is_absolute() else args.root / raw_candidate
        report = build_report(args.root, raw_path, floor)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.check:
            if existing != report:
                raise ValueError("coverage summary or raw report is stale")
        else:
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered)
        lines = report["lines"]
        print(
            f"PASS coverage summary lines={lines['covered']}/{lines['count']} "
            f"percent={lines['percent']:.4f}% floor={floor}%"
        )
        return 0 if report["passed"] else 1
    except (
        OSError,
        KeyError,
        IndexError,
        TypeError,
        ValueError,
        subprocess.SubprocessError,
        json.JSONDecodeError,
    ) as error:
        print(f"coverage_summary.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
