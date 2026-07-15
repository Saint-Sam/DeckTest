#!/usr/bin/env python3
"""Write a portable, source-bound summary of LLVM coverage output."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


HUNK_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


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


def parse_lcov(root: Path, path: Path) -> dict[str, dict[int, int]]:
    """Return executable line counts keyed by repository-relative source path."""
    records: dict[str, dict[int, int]] = {}
    current: str | None = None
    resolved_root = root.resolve()
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if raw_line.startswith("SF:"):
            source = Path(raw_line[3:])
            try:
                current = source.resolve().relative_to(resolved_root).as_posix()
            except ValueError:
                current = None
            if current is not None:
                records.setdefault(current, {})
        elif current is not None and raw_line.startswith("DA:"):
            fields = raw_line[3:].split(",", 2)
            if len(fields) < 2:
                raise ValueError(f"malformed LCOV line: {raw_line}")
            line_number = int(fields[0])
            count = int(fields[1])
            records[current][line_number] = max(
                count, records[current].get(line_number, 0)
            )
    return records


def parse_changed_lines(diff: str) -> dict[str, set[int]]:
    """Parse added-side line numbers from a zero-context unified diff."""
    changed: dict[str, set[int]] = {}
    current: str | None = None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            value = line[4:].split("\t", 1)[0]
            current = None if value == "/dev/null" else value.removeprefix("b/")
            if current is not None:
                changed.setdefault(current, set())
            continue
        match = HUNK_RE.match(line)
        if current is None or match is None:
            continue
        start = int(match.group(1))
        count = int(match.group(2) or "1")
        changed[current].update(range(start, start + count))
    return changed


def git_changed_lines(root: Path, base: str, product: str) -> dict[str, set[int]]:
    result = subprocess.run(
        [
            "git",
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            f"{base}...{product}",
            "--",
            "*.rs",
        ],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode,
            result.args,
            output=result.stdout,
            stderr=result.stderr,
        )
    return parse_changed_lines(result.stdout)


def changed_line_report(
    root: Path, lcov_path: Path, base: str, product: str
) -> dict[str, object]:
    coverage = parse_lcov(root, lcov_path)
    changed = git_changed_lines(root, base, product)
    files = []
    changed_total = 0
    executable_total = 0
    covered_total = 0
    for source in sorted(changed):
        added = changed[source]
        executable = added.intersection(coverage.get(source, {}))
        covered = sum(coverage[source][line] > 0 for line in executable)
        changed_total += len(added)
        executable_total += len(executable)
        covered_total += covered
        if executable:
            files.append(
                {
                    "path": source,
                    "changed_lines": len(added),
                    "executable_lines": len(executable),
                    "covered_lines": covered,
                    "percent": covered * 100.0 / len(executable),
                }
            )
    percent = 100.0 if executable_total == 0 else covered_total * 100.0 / executable_total
    return {
        "base_commit": git_value(root, f"{base}^{{commit}}"),
        "base_tree": git_value(root, f"{base}^{{tree}}"),
        "scope": "added executable Rust lines from the pre-T4 product through reviewed_commit",
        "changed_rust_lines": changed_total,
        "executable_lines": executable_total,
        "non_executable_lines": changed_total - executable_total,
        "covered": covered_total,
        "not_covered": executable_total - covered_total,
        "percent": percent,
        "files": files,
    }


def build_report(
    root: Path,
    raw_path: Path,
    lcov_path: Path,
    changed_base: str,
    floor: int,
    reviewed_commit: str | None = None,
    reviewed_tree: str | None = None,
) -> dict[str, object]:
    raw = json.loads(raw_path.read_text())
    totals = raw["data"][0]["totals"]
    lines = totals["lines"]
    count = int(lines["count"])
    covered = int(lines["covered"])
    percent = float(lines["percent"])
    commit = reviewed_commit or git_value(root, "HEAD")
    tree = reviewed_tree or git_value(root, "HEAD^{tree}")
    if (root / ".git").exists() and git_value(root, f"{commit}^{{tree}}") != tree:
        raise ValueError("reviewed coverage commit/tree binding is invalid")
    return {
        "schema_version": 3,
        "passed": percent >= floor,
        "reviewed_commit": commit,
        "reviewed_tree": tree,
        "source_sha256": source_hash(root),
        "raw_report_sha256": sha256(raw_path),
        "raw_report_path": str(raw_path.resolve().relative_to(root.resolve())),
        "lcov_report_sha256": sha256(lcov_path),
        "lcov_report_path": str(lcov_path.resolve().relative_to(root.resolve())),
        "floor_percent": floor,
        "lines": {
            "count": count,
            "covered": covered,
            "not_covered": count - covered,
            "percent": percent,
        },
        "changed_lines": changed_line_report(root, lcov_path, changed_base, commit),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--raw", type=Path)
    parser.add_argument("--lcov", type=Path)
    parser.add_argument("--changed-base")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--floor", type=int)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = args.output or args.root / "metrics/coverage.json"
    try:
        existing = None
        reviewed_commit = None
        reviewed_tree = None
        if args.check:
            if not output.is_file():
                raise ValueError("coverage summary is absent")
            existing = json.loads(output.read_text())
            if not isinstance(existing, dict):
                raise ValueError("coverage summary is not an object")
            raw_value = existing.get("raw_report_path")
            lcov_value = existing.get("lcov_report_path")
            changed = existing.get("changed_lines")
            if not isinstance(changed, dict):
                raise ValueError("coverage summary lacks changed-line evidence")
            changed_base = str(changed.get("base_commit", ""))
            reviewed_commit = str(existing.get("reviewed_commit", ""))
            reviewed_tree = str(existing.get("reviewed_tree", ""))
            floor = int(existing.get("floor_percent", 0))
        else:
            if (
                args.raw is None
                or args.lcov is None
                or args.changed_base is None
                or args.floor is None
            ):
                raise ValueError(
                    "--raw, --lcov, --changed-base, and --floor are required unless --check is used"
                )
            raw_value = args.raw
            lcov_value = args.lcov
            changed_base = args.changed_base
            floor = args.floor
        raw_candidate = Path(str(raw_value))
        raw_path = raw_candidate if raw_candidate.is_absolute() else args.root / raw_candidate
        lcov_candidate = Path(str(lcov_value))
        lcov_path = lcov_candidate if lcov_candidate.is_absolute() else args.root / lcov_candidate
        report = build_report(
            args.root,
            raw_path,
            lcov_path,
            changed_base,
            floor,
            reviewed_commit,
            reviewed_tree,
        )
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
            f"percent={lines['percent']:.4f}% floor={floor}% "
            f"changed={report['changed_lines']['covered']}/"
            f"{report['changed_lines']['executable_lines']}"
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
