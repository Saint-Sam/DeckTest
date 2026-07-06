#!/usr/bin/env python3
"""Write Forge metrics JSON files atomically."""

from __future__ import annotations

import argparse
import json
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def parse_value(raw: str) -> Any:
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        pass
    try:
        if "." in raw:
            return float(raw)
        return int(raw)
    except ValueError:
        return raw


def set_dotted(target: dict[str, Any], key: str, value: Any) -> None:
    parts = [part for part in key.split(".") if part]
    if not parts:
        raise ValueError("metric key cannot be empty")
    node = target
    for part in parts[:-1]:
        existing = node.setdefault(part, {})
        if not isinstance(existing, dict):
            raise ValueError(f"cannot set {key}: {part} is already non-object")
        node = existing
    node[parts[-1]] = value


def load_existing(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def atomic_write(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "w",
        encoding="utf-8",
        dir=str(path.parent),
        prefix=f".{path.name}.",
        suffix=".tmp",
        delete=False,
    ) as handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
        handle.write("\n")
        tmp_name = handle.name
    Path(tmp_name).replace(path)


def run(out: Path, metrics: list[str], json_metrics: list[str], stamp: bool) -> int:
    payload = load_existing(out)
    for item in metrics:
        if "=" not in item:
            raise ValueError(f"--metric must be KEY=VALUE, got {item!r}")
        key, raw = item.split("=", 1)
        set_dotted(payload, key, parse_value(raw))
    for item in json_metrics:
        if "=" not in item:
            raise ValueError(f"--set-json must be KEY=JSON, got {item!r}")
        key, raw = item.split("=", 1)
        set_dotted(payload, key, json.loads(raw))
    if stamp:
        payload["updated_at"] = datetime.now(timezone.utc).isoformat()
    atomic_write(out, payload)
    print(f"WROTE {out}")
    return 0


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="forge_metrics_write_") as temp:
        out = Path(temp) / "metrics.json"
        run(out, ["coverage.lines=82.5", "counts.tests=3", "status=green"], ['flags={"t0":true}'], True)
        data = json.loads(out.read_text(encoding="utf-8"))
        assert data["coverage"]["lines"] == 82.5
        assert data["counts"]["tests"] == 3
        assert data["status"] == "green"
        assert data["flags"]["t0"] is True
        assert "updated_at" in data
    print("PASS metrics_write.py self-test")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("metrics/metrics.json"))
    parser.add_argument("--metric", action="append", default=[], help="Set dotted KEY=VALUE")
    parser.add_argument("--set-json", action="append", default=[], help="Set dotted KEY=JSON")
    parser.add_argument("--timestamp", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    return run(args.out, args.metric, args.set_json, args.timestamp)


if __name__ == "__main__":
    raise SystemExit(main())
