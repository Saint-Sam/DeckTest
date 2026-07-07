#!/usr/bin/env python3
"""Export Criterion estimates into Forge's perf_current.json format."""

from __future__ import annotations

import argparse
import json
import math
import sys
import tempfile
from pathlib import Path
from typing import Any


DEFAULT_CRITERION_ROOT = Path("target/criterion")
DEFAULT_OUT = Path("metrics/perf_current.json")


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def point_estimate(path: Path) -> float:
    payload = load_json(path)
    try:
        value = float(payload["mean"]["point_estimate"])
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"missing mean.point_estimate in {path}") from error
    if not math.isfinite(value):
        raise ValueError(f"non-finite mean.point_estimate in {path}")
    return value


def metric_name(root: Path, estimates_path: Path) -> str:
    relative = estimates_path.relative_to(root)
    parts = relative.parts
    if len(parts) < 3 or parts[-2:] != ("new", "estimates.json"):
        raise ValueError(f"not a Criterion estimates path: {estimates_path}")
    return "/".join(parts[:-2])


def collect(root: Path, since_file: Path | None = None) -> dict[str, float]:
    since = since_file.stat().st_mtime if since_file is not None else None
    metrics: dict[str, float] = {}
    for path in sorted(root.rglob("estimates.json")):
        if path.parent.name != "new":
            continue
        if since is not None and path.stat().st_mtime < since:
            continue
        name = metric_name(root, path)
        metrics[name] = point_estimate(path)
    return metrics


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp.replace(path)


def run(root: Path, out: Path, since_file: Path | None) -> int:
    if not root.exists():
        print(f"ERROR: Criterion output missing: {root}", file=sys.stderr)
        return 2
    if since_file is not None and not since_file.exists():
        print(f"ERROR: since-file missing: {since_file}", file=sys.stderr)
        return 2
    try:
        metrics = collect(root, since_file)
    except ValueError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    if not metrics:
        print(f"ERROR: no Criterion estimates found under {root}", file=sys.stderr)
        return 2
    write_json(out, metrics)
    print(f"PASS criterion_to_perf.py wrote {len(metrics)} metric(s) to {out}")
    return 0


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="forge_criterion_to_perf_") as temp:
        root = Path(temp)
        estimates = root / "target" / "criterion" / "kernel" / "clone" / "new" / "estimates.json"
        estimates.parent.mkdir(parents=True)
        estimates.write_text(
            json.dumps({"mean": {"point_estimate": 123.0}}),
            encoding="utf-8",
        )
        out = root / "metrics" / "perf_current.json"
        status = run(root / "target" / "criterion", out, None)
        if status != 0:
            return status
        payload = load_json(out)
        if payload != {"kernel/clone": 123.0}:
            print("ERROR: criterion_to_perf.py self-test mismatch", file=sys.stderr)
            return 1
    print("PASS criterion_to_perf.py self-test")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--criterion-root", type=Path, default=DEFAULT_CRITERION_ROOT)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--since-file", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return run(args.criterion_root, args.out, args.since_file)


if __name__ == "__main__":
    raise SystemExit(main())
