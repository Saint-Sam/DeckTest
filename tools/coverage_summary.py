#!/usr/bin/env python3
"""Write a portable, source-bound summary of LLVM coverage output."""

from __future__ import annotations

import argparse
import hashlib
import json
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--raw", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--floor", type=int, required=True)
    args = parser.parse_args()
    output = args.output or args.root / "metrics/coverage.json"
    raw_path = args.raw if args.raw.is_absolute() else args.root / args.raw
    try:
        raw = json.loads(raw_path.read_text())
        totals = raw["data"][0]["totals"]
        lines = totals["lines"]
        count = int(lines["count"])
        covered = int(lines["covered"])
        percent = float(lines["percent"])
        report = {
            "schema_version": 2,
            "passed": percent >= args.floor,
            "reviewed_commit": git_value(args.root, "HEAD"),
            "reviewed_tree": git_value(args.root, "HEAD^{tree}"),
            "source_sha256": source_hash(args.root),
            "raw_report_sha256": sha256(raw_path),
            "raw_report_path": str(raw_path.resolve().relative_to(args.root.resolve())),
            "floor_percent": args.floor,
            "lines": {
                "count": count,
                "covered": covered,
                "not_covered": count - covered,
                "percent": percent,
            },
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n")
        print(
            f"PASS coverage summary lines={covered}/{count} "
            f"percent={percent:.4f}% floor={args.floor}%"
        )
        return 0 if report["passed"] else 1
    except (OSError, KeyError, IndexError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"coverage_summary.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
