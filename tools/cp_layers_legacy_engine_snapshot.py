#!/usr/bin/env python3
"""Run the local legacy Forge engine on the CP-LAYERS 100-card subset."""

from __future__ import annotations

import csv
import json
import subprocess
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SUBSET_CSV = ROOT / "reports" / "gates" / "CP-LAYERS" / "legacy-100-layered-subset-2026-07-07.csv"
RUNNER = ROOT / "tools" / "run_legacy_layer_snapshot.sh"
OUT_DIR = ROOT / "reports" / "gates" / "CP-LAYERS"
REPORT = OUT_DIR / "legacy-engine-snapshot-2026-07-07.md"
METRICS = ROOT / "metrics" / "cp_layers_legacy_engine_snapshot.json"
JSONL = ROOT / "metrics" / "cp_layers_legacy_engine_snapshot.jsonl"


def read_subset() -> list[dict[str, str]]:
    with SUBSET_CSV.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def parse_json_lines(output: str) -> tuple[list[dict[str, object]], int]:
    results: list[dict[str, object]] = []
    ignored = 0
    for line in output.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            if line:
                ignored += 1
            continue
        results.append(json.loads(line))
    return results, ignored


def run_snapshot(names: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(RUNNER), *names],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=300,
        check=False,
    )


def card_summary(result: dict[str, object]) -> str:
    battlefield = result.get("battlefield")
    if not isinstance(battlefield, list):
        return ""
    pieces = []
    for card in battlefield:
        if not isinstance(card, dict):
            continue
        name = str(card.get("name", ""))
        types = str(card.get("types", ""))
        colors = str(card.get("colors", ""))
        power = card.get("power", "")
        toughness = card.get("toughness", "")
        keywords = card.get("keywords", [])
        kw = ", ".join(str(item) for item in keywords) if isinstance(keywords, list) else ""
        pieces.append(f"{name} [{types}; {colors}; {power}/{toughness}; {kw}]")
    return " / ".join(pieces)


def write_outputs(
    subset: list[dict[str, str]],
    results: list[dict[str, object]],
    ignored_stdout_lines: int,
    returncode: int,
    stderr: str,
) -> None:
    by_name = {str(result.get("scenario", "")): result for result in results}
    status_counts = Counter(str(result.get("status", "missing")) for result in results)
    missing = [row["name"] for row in subset if row["name"] not in by_name]
    errors = [
        result
        for result in results
        if result.get("status") != "ok"
    ]

    METRICS.parent.mkdir(parents=True, exist_ok=True)
    JSONL.write_text(
        "".join(json.dumps(result, sort_keys=True) + "\n" for result in results),
        encoding="utf-8",
    )
    METRICS.write_text(
        json.dumps(
            {
                "subset_count": len(subset),
                "snapshot_count": len(results),
                "status_counts": dict(sorted(status_counts.items())),
                "missing_count": len(missing),
                "missing": missing,
                "returncode": returncode,
                "ignored_stdout_lines": ignored_stdout_lines,
                "stderr_nonempty": bool(stderr.strip()),
                "result_jsonl": str(JSONL.relative_to(ROOT)),
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    verdict = "PASS" if returncode == 0 and not missing and not errors and len(results) == len(subset) else "PARTIAL"
    lines = [
        "# CP-LAYERS Legacy Engine Snapshot",
        "",
        "Date: 2026-07-07",
        "",
        "Mode: local-only execution of the vendored legacy Forge Java engine against the selected CP-LAYERS 100-card layered subset.",
        "",
        f"Result: {verdict}",
        "",
        "This evidence upgrades the earlier local-search finding: the legacy Java engine is now runnable through repo-local Corretto 17 and Maven artifacts. The remaining true differential blocker is on the Forge 2.0 side: it still has no real legacy card-script importer/card compiler for executing these same 100 cards in the new engine.",
        "",
        "## Harness",
        "",
        f"- Runner: `{RUNNER.relative_to(ROOT)}`",
        "- Fixture per card: `Runeclaw Bear` under opponent control, `Memnite` under controller control, then the selected source card.",
        "- Attachments: Auras attach to the opponent creature when legal; Equipment/Fortifications attach to the controller artifact when legal.",
        "- Snapshot fields: controller, type line, colors, net power/toughness, and keyword originals after `checkStaticAbilities(false)`.",
        "- Network: none during this run.",
        "",
        "## Counts",
        "",
        "| Metric | Count |",
        "| --- | ---: |",
        f"| Selected subset cards | {len(subset)} |",
        f"| Legacy snapshots emitted | {len(results)} |",
        f"| OK snapshots | {status_counts.get('ok', 0)} |",
        f"| Error snapshots | {len(errors)} |",
        f"| Missing selected names | {len(missing)} |",
        f"| Java process return code | {returncode} |",
        "",
        "## Artifacts",
        "",
        f"- Machine-readable summary: `{METRICS.relative_to(ROOT)}`",
        f"- Legacy snapshot JSONL: `{JSONL.relative_to(ROOT)}`",
        "",
        "## Sample Snapshots",
        "",
        "| ID | Scenario | Snapshot summary |",
        "| --- | --- | --- |",
    ]
    for row in subset[:20]:
        result = by_name.get(row["name"])
        summary = card_summary(result) if result is not None else "MISSING"
        lines.append(f"| {row['id']} | {row['name']} | {summary} |")

    if errors or missing:
        lines.extend(["", "## Errors", "", "| Scenario | Error |", "| --- | --- |"])
        for result in errors:
            lines.append(f"| {result.get('scenario', '')} | {result.get('error', result.get('status', 'unknown'))} |")
        for name in missing:
            lines.append(f"| {name} | missing JSON snapshot |")

    lines.extend(
        [
            "",
            "## Gate Consequence",
            "",
            "The legacy side of the 100-card differential is now executable and recorded. CP-LAYERS still cannot honestly be marked PASS for the true engine-vs-engine clause until Forge 2.0 can translate/import these selected card scripts into executable new-engine layer definitions, or the owner explicitly changes the checkpoint requirement.",
            "",
        ]
    )
    REPORT.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    subset = read_subset()
    names = [row["name"] for row in subset]
    completed = run_snapshot(names)
    results, ignored_stdout_lines = parse_json_lines(completed.stdout)
    write_outputs(subset, results, ignored_stdout_lines, completed.returncode, completed.stderr)
    if completed.returncode != 0:
        print(completed.stderr)
    print(f"wrote {REPORT.relative_to(ROOT)}")
    return 0 if completed.returncode == 0 else completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
