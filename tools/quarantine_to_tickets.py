#!/usr/bin/env python3
"""Convert NEEDS_NEW_PRIMITIVE quarantine records into deterministic T2 tickets."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = ROOT / "metrics/translation_quarantine.json"
DEFAULT_OUTPUT = ROOT / "metrics/primitive_tickets.json"
REASON_CODE = "NEEDS_NEW_PRIMITIVE"


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_quarantine(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read quarantine evidence {path}: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("translation quarantine must be a schema-v1 object")
    files = value.get("files")
    if not isinstance(files, list):
        raise ValueError("translation quarantine has no files array")
    if value.get("total_quarantined") != len(files):
        raise ValueError("translation quarantine total does not match files array")
    return value


def ticket_id(message: str) -> str:
    payload = f"{REASON_CODE}\0{message}".encode("utf-8")
    return f"T2-PRIMITIVE-{hashlib.sha256(payload).hexdigest()[:12]}"


def build_ticket_queue(value: dict[str, Any], source_path: Path) -> dict[str, Any]:
    grouped: dict[str, list[dict[str, object]]] = defaultdict(list)
    seen_paths: set[str] = set()
    for index, entry in enumerate(value["files"]):
        if not isinstance(entry, dict):
            raise ValueError(f"quarantine file entry {index} is not an object")
        path = entry.get("path")
        code = entry.get("code")
        message = entry.get("message")
        line = entry.get("line")
        if not isinstance(path, str) or not path:
            raise ValueError(f"quarantine file entry {index} has no path")
        if path in seen_paths:
            raise ValueError(f"translation quarantine repeats path {path}")
        seen_paths.add(path)
        if not isinstance(code, str) or not code:
            raise ValueError(f"quarantine file entry {index} has no code")
        if not isinstance(message, str) or not message:
            raise ValueError(f"quarantine file entry {index} has no message")
        if not isinstance(line, int) or line < 1:
            raise ValueError(f"quarantine file entry {index} has invalid line")
        if code == REASON_CODE:
            grouped[message].append({"path": path, "line": line})

    tickets = []
    for message, locations in grouped.items():
        ordered = sorted(locations, key=lambda item: (str(item["path"]), int(item["line"])))
        tickets.append(
            {
                "id": ticket_id(message),
                "route": "T2",
                "status": "open",
                "reason_code": REASON_CODE,
                "title": message,
                "affected_scripts": len(ordered),
                "locations": ordered,
                "acceptance": [
                    "Specify one card-neutral typed primitive and its rules semantics.",
                    "Add focused kernel and card-runtime tests, including rejection cases.",
                    "Keep the mapper fail-closed until exact lowering and replay evidence pass.",
                ],
            }
        )
    tickets.sort(key=lambda item: str(item["id"]))
    reported_count = value.get("reason_counts", {}).get(REASON_CODE, 0)
    affected_scripts = sum(int(ticket["affected_scripts"]) for ticket in tickets)
    if reported_count != affected_scripts:
        raise ValueError(
            f"translation quarantine reports {reported_count} {REASON_CODE} records, "
            f"but files contain {affected_scripts}"
        )
    try:
        stable_source_path = source_path.resolve().relative_to(ROOT.resolve()).as_posix()
    except ValueError:
        stable_source_path = source_path.as_posix()
    return {
        "schema_version": 1,
        "kind": "t3_8_t2_primitive_ticket_queue",
        "route": "T2",
        "source": {
            "path": stable_source_path,
            "sha256": sha256_file(source_path),
            "source_revision": value.get("source_revision"),
            "total_quarantined": value["total_quarantined"],
        },
        "reason_code": REASON_CODE,
        "ticket_count": len(tickets),
        "affected_scripts": affected_scripts,
        "tickets": tickets,
    }


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        value = load_quarantine(args.input)
        rendered = json_bytes(build_ticket_queue(value, args.input))
    except ValueError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    if args.check:
        if not args.output.is_file() or args.output.read_bytes() != rendered:
            print(f"ERROR: stale primitive ticket queue: {args.output}", file=sys.stderr)
            return 1
        print(f"PASS primitive ticket queue current: {args.output}")
        return 0
    write_atomic(args.output, rendered)
    print(
        f"wrote {args.output}: "
        f"tickets={json.loads(rendered)['ticket_count']} "
        f"affected_scripts={json.loads(rendered)['affected_scripts']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
