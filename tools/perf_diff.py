#!/usr/bin/env python3
"""Compare benchmark metrics against a baseline and fail on regressions."""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import math
import sys
import tempfile
from pathlib import Path
from typing import Any


DEFAULT_BASELINE = Path("metrics/perf_baseline.json")
DEFAULT_CURRENT = Path("metrics/perf_current.json")


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def scalar(value: Any) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)) and math.isfinite(float(value)):
        return float(value)
    if isinstance(value, dict):
        for key in ("mean", "median", "value", "time", "score", "point_estimate"):
            if key in value:
                found = scalar(value[key])
                if found is not None:
                    return found
        if "estimate" in value:
            found = scalar(value["estimate"])
            if found is not None:
                return found
    return None


def collect_metrics(obj: Any, prefix: str = "") -> dict[str, float]:
    metrics: dict[str, float] = {}

    direct = scalar(obj)
    if direct is not None and prefix:
        metrics[prefix] = direct
        return metrics

    if isinstance(obj, dict):
        name = obj.get("name") or obj.get("id") or obj.get("benchmark")
        if isinstance(name, str):
            value = scalar(obj)
            if value is not None:
                metrics[name] = value
        for key, value in obj.items():
            if key in {"name", "id", "benchmark"}:
                continue
            child_prefix = f"{prefix}.{key}" if prefix else str(key)
            child_value = scalar(value)
            if child_value is not None:
                metrics[child_prefix] = child_value
            else:
                metrics.update(collect_metrics(value, child_prefix))
    elif isinstance(obj, list):
        for index, value in enumerate(obj):
            if isinstance(value, dict):
                name = value.get("name") or value.get("id") or value.get("benchmark")
                child_prefix = str(name) if isinstance(name, str) else f"{prefix}[{index}]"
            else:
                child_prefix = f"{prefix}[{index}]"
            metrics.update(collect_metrics(value, child_prefix))

    return metrics


def compare(
    baseline: dict[str, float],
    current: dict[str, float],
    threshold: float,
) -> tuple[list[dict[str, float]], list[str]]:
    common = sorted(set(baseline) & set(current))
    regressions: list[dict[str, float]] = []
    for name in common:
        old = baseline[name]
        new = current[name]
        if old == 0:
            regressed = new > threshold
            ratio = math.inf if new > 0 else 0.0
        else:
            ratio = (new - old) / abs(old)
            regressed = ratio > threshold
        if regressed:
            regressions.append(
                {
                    "name": name,
                    "baseline": old,
                    "current": new,
                    "regression_fraction": ratio,
                }
            )
    return regressions, common


def write_summary(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp.replace(path)


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="forge_perf_diff_") as temp:
        root = Path(temp)
        baseline = root / "baseline.json"
        current_pass = root / "current_pass.json"
        current_fail = root / "current_fail.json"
        baseline.write_text(json.dumps({"bench_a": 100.0, "bench_b": {"mean": 20.0}}), encoding="utf-8")
        current_pass.write_text(json.dumps({"bench_a": 104.0, "bench_b": {"mean": 20.5}}), encoding="utf-8")
        current_fail.write_text(json.dumps({"bench_a": 106.0, "bench_b": {"mean": 20.5}}), encoding="utf-8")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            ok = run(baseline, current_pass, 0.05, None)
            bad = run(baseline, current_fail, 0.05, None)
        if ok != 0 or bad == 0:
            print("ERROR: perf_diff.py self-test failed", file=sys.stderr)
            return 1
    print("PASS perf_diff.py self-test")
    return 0


def run(baseline_path: Path, current_path: Path, threshold: float, summary_out: Path | None) -> int:
    if not baseline_path.exists():
        print(f"SKIP: perf baseline missing: {baseline_path}")
        return 0
    if not current_path.exists():
        print(f"SKIP: perf current metrics missing: {current_path}")
        return 0

    baseline = collect_metrics(load_json(baseline_path))
    current = collect_metrics(load_json(current_path))
    if not baseline:
        print(f"ERROR: no numeric metrics found in baseline: {baseline_path}", file=sys.stderr)
        return 2
    if not current:
        print(f"ERROR: no numeric metrics found in current metrics: {current_path}", file=sys.stderr)
        return 2

    regressions, common = compare(baseline, current, threshold)
    missing = sorted(set(baseline) - set(current))
    added = sorted(set(current) - set(baseline))
    payload = {
        "threshold": threshold,
        "compared_count": len(common),
        "missing_from_current": missing,
        "added_in_current": added,
        "regressions": regressions,
    }
    if summary_out is not None:
        write_summary(summary_out, payload)

    if not common:
        print("ERROR: no overlapping perf metric names between baseline and current", file=sys.stderr)
        return 2

    if regressions:
        print(f"ERROR: {len(regressions)} perf regression(s) exceed {threshold:.1%}", file=sys.stderr)
        for item in regressions:
            print(
                f"{item['name']}: {item['baseline']} -> {item['current']} "
                f"({item['regression_fraction']:.2%})",
                file=sys.stderr,
            )
        return 1

    print(f"PASS perf_diff.py ({len(common)} metric(s), threshold {threshold:.1%})")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--current", type=Path, default=DEFAULT_CURRENT)
    parser.add_argument("--threshold", type=float, default=0.05)
    parser.add_argument("--summary-out", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if args.threshold < 0:
        print("ERROR: threshold must be non-negative", file=sys.stderr)
        return 2
    return run(args.baseline, args.current, args.threshold, args.summary_out)


if __name__ == "__main__":
    raise SystemExit(main())
