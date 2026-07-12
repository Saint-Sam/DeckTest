#!/usr/bin/env python3
"""Build a local-only T3.3 progress report from committed evidence."""

from __future__ import annotations

import csv
import json
import math
import statistics
import subprocess
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.dates as mdates
import matplotlib.pyplot as plt


ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
EVIDENCE_PREFIX = "reports/gates/T3.3/"


@dataclass(frozen=True)
class Point:
    index: int
    timestamp: datetime
    evidence_file: str
    evidence_commit: str
    mapper_commit: str
    batch: str
    complete_scripts: int
    total_scripts: int
    complete_percent: float
    mapped_uses: int
    total_uses: int
    mapped_percent: float
    delta_scripts: int
    delta_uses: int
    priority_complete: int


def git(*args: str) -> str:
    return subprocess.check_output(
        ["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL
    ).strip()


def committed_points() -> list[Point]:
    commits = git("log", "--reverse", "--format=%H", "--", EVIDENCE_PREFIX).splitlines()
    rows: list[Point] = []
    seen: set[str] = set()
    for commit in commits:
        changed = git(
            "diff-tree", "--no-commit-id", "--name-only", "-r", commit, "--", EVIDENCE_PREFIX
        ).splitlines()
        for path in changed:
            if not path.endswith(".json") or path in seen:
                continue
            try:
                payload = json.loads(git("show", f"{commit}:{path}"))
            except (subprocess.CalledProcessError, json.JSONDecodeError):
                continue
            after = payload.get("coverage_delta", {}).get("after", {})
            if after.get("complete_scripts") is None or after.get("mapped_ability_uses") is None:
                continue
            timestamp_text = payload.get("generated_at") or git("show", "-s", "--format=%cI", commit)
            timestamp = datetime.fromisoformat(timestamp_text.replace("Z", "+00:00"))
            total_scripts = int(after.get("total_scripts", 33290))
            total_uses = int(after.get("total_ability_uses", 43649))
            complete = int(after["complete_scripts"])
            mapped = int(after["mapped_ability_uses"])
            gain = payload.get("coverage_delta", {}).get("gain", {})
            rows.append(
                Point(
                    index=0,
                    timestamp=timestamp,
                    evidence_file=path,
                    evidence_commit=commit[:7],
                    mapper_commit=str(payload.get("product_binding", {}).get("mapper_commit", ""))[:7],
                    batch=str(payload.get("batch", Path(path).stem)),
                    complete_scripts=complete,
                    total_scripts=total_scripts,
                    complete_percent=float(after.get("complete_script_percent", 100 * complete / total_scripts)),
                    mapped_uses=mapped,
                    total_uses=total_uses,
                    mapped_percent=float(after.get("mapped_ability_percent", 100 * mapped / total_uses)),
                    delta_scripts=int(gain.get("complete_scripts", 0)),
                    delta_uses=int(gain.get("mapped_ability_uses", 0)),
                    priority_complete=int(after.get("owner_priority_complete", 0)),
                )
            )
            seen.add(path)
    # Generated timestamps are the checkpoint clock. Keep reviewed downward
    # corrections visible; removing them would bias throughput upward.
    rows.sort(key=lambda row: (row.timestamp, row.evidence_commit))
    return [Point(index=i + 1, **{k: v for k, v in row.__dict__.items() if k != "index"}) for i, row in enumerate(rows)]


def hours(delta_seconds: float) -> float:
    return delta_seconds / 3600.0


def percentile(values: list[float], p: float) -> float:
    if not values:
        return math.nan
    ordered = sorted(values)
    position = (len(ordered) - 1) * p
    lo = math.floor(position)
    hi = math.ceil(position)
    if lo == hi:
        return ordered[lo]
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (position - lo)


def projections(points: list[Point]) -> dict[str, float]:
    first, latest = points[0], points[-1]
    elapsed = max(hours((latest.timestamp - first.timestamp).total_seconds()), 0.01)
    historical_rate = (latest.complete_scripts - first.complete_scripts) / elapsed
    recent = points[-6:]
    recent_elapsed = max(hours((recent[-1].timestamp - recent[0].timestamp).total_seconds()), 0.01)
    recent_rate = (recent[-1].complete_scripts - recent[0].complete_scripts) / recent_elapsed
    interval_rates = [
        (right.complete_scripts - left.complete_scripts)
        / max(hours((right.timestamp - left.timestamp).total_seconds()), 0.01)
        for left, right in zip(recent, recent[1:])
    ]
    low_rate = max(historical_rate, 1.0)
    high_rate = max(recent_rate, low_rate, 1.0)
    return {
        "historical_rate": historical_rate,
        "recent_rate": recent_rate,
        "recent_rate_p25": percentile(interval_rates, 0.25),
        "recent_rate_p75": percentile(interval_rates, 0.75),
        "to_60_fast_hours": max(0, 0.60 * latest.total_scripts - latest.complete_scripts) / high_rate,
        "to_60_slow_hours": max(0, 0.60 * latest.total_scripts - latest.complete_scripts) / low_rate,
        "to_100_fast_hours": max(0, latest.total_scripts - latest.complete_scripts) / high_rate,
        "to_100_slow_hours": max(0, latest.total_scripts - latest.complete_scripts) / low_rate,
    }


def write_csv(points: list[Point]) -> None:
    fields = list(Point.__dataclass_fields__)
    with (OUT / "checkpoint_series.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for point in points:
            row = point.__dict__.copy()
            row["timestamp"] = point.timestamp.isoformat()
            writer.writerow(row)


def style_axis(ax, title: str, ylabel: str) -> None:
    ax.set_title(title, fontsize=13, fontweight="bold")
    ax.set_ylabel(ylabel)
    ax.grid(True, alpha=0.25)
    ax.spines[["top", "right"]].set_visible(False)


def plot_coverage(points: list[Point]) -> None:
    times = [p.timestamp for p in points]
    scripts = [p.complete_percent for p in points]
    uses = [p.mapped_percent for p in points]
    fig, ax = plt.subplots(figsize=(12, 6.5))
    ax.plot(times, scripts, marker="o", ms=4, lw=2.2, label="Complete scripts")
    ax.plot(times, uses, marker="s", ms=4, lw=2.2, label="Mapped ability uses")
    ax.axhline(60, color="#b23a48", ls="--", lw=1.5, label="T3.3 floor (60%)")
    style_axis(ax, "T3.3 structural coverage at committed checkpoints", "Coverage (%)")
    ax.set_ylim(20, 65)
    ax.xaxis.set_major_formatter(mdates.DateFormatter("%b %d\n%H:%M", tz=timezone.utc))
    ax.legend(frameon=False, ncol=3, loc="upper left")
    fig.tight_layout()
    fig.savefig(OUT / "coverage_over_time.png", dpi=180)
    plt.close(fig)


def plot_deltas(points: list[Point]) -> None:
    labels = [str(p.index) for p in points]
    script_delta = [p.delta_scripts for p in points]
    use_delta = [p.delta_uses for p in points]
    fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)
    axes[0].bar(labels, script_delta, color="#2a6f97")
    style_axis(axes[0], "Complete-script gain per committed evidence checkpoint", "Scripts gained")
    axes[1].bar(labels, use_delta, color="#d97706")
    style_axis(axes[1], "Mapped-use gain per committed evidence checkpoint", "Uses gained")
    axes[1].set_xlabel("Checkpoint sequence (see checkpoint_series.csv)")
    step = max(1, len(labels) // 14)
    for tick in axes[1].xaxis.get_major_ticks():
        tick.label1.set_visible(False)
    for i in range(0, len(labels), step):
        axes[1].text(i, -0.13, labels[i], transform=axes[1].get_xaxis_transform(), ha="center")
    fig.tight_layout()
    fig.savefig(OUT / "checkpoint_throughput.png", dpi=180)
    plt.close(fig)


def plot_projection(points: list[Point], projection: dict[str, float]) -> None:
    latest = points[-1]
    remaining = latest.total_scripts - latest.complete_scripts
    targets = ["60% floor", "100% mechanical"]
    fast = [projection["to_60_fast_hours"], projection["to_100_fast_hours"]]
    slow = [projection["to_60_slow_hours"], projection["to_100_slow_hours"]]
    x = range(len(targets))
    fig, ax = plt.subplots(figsize=(9, 6))
    width = 0.35
    ax.bar([i - width / 2 for i in x], fast, width, label="Faster observed checkpoint cadence", color="#2a9d8f")
    ax.bar([i + width / 2 for i in x], slow, width, label="Whole-history wall-clock rate", color="#6c757d")
    for i, value in enumerate(fast):
        ax.text(i - width / 2, value, f"{value:.1f}h", ha="center", va="bottom")
    for i, value in enumerate(slow):
        ax.text(i + width / 2, value, f"{value:.1f}h", ha="center", va="bottom")
    ax.set_xticks(list(x), targets)
    style_axis(ax, "Linear projection from the latest committed checkpoint", "Estimated elapsed hours")
    ax.legend(frameon=False)
    ax.text(
        0.01,
        -0.18,
        f"Latest: {latest.complete_scripts:,}/{latest.total_scripts:,}; {remaining:,} remain. "
        "100% excludes expected long-tail slowdown and semantic redesign time.",
        transform=ax.transAxes,
        fontsize=9,
    )
    fig.tight_layout()
    fig.savefig(OUT / "coverage_projection.png", dpi=180, bbox_inches="tight")
    plt.close(fig)


def write_report(points: list[Point], projection: dict[str, float]) -> None:
    first, latest = points[0], points[-1]
    gained_scripts = latest.complete_scripts - first.complete_scripts
    gained_uses = latest.mapped_uses - first.mapped_uses
    corrections = sum(
        right.complete_scripts < left.complete_scripts or right.mapped_uses < left.mapped_uses
        for left, right in zip(points, points[1:])
    )
    text = f"""# Forge 2.0 T3.3 Script Coverage Progress

Generated from committed local `reports/gates/T3.3` evidence and local git history only. No live metrics, network access, installs, GitHub Actions, or pushes were used.

## Snapshot

- Latest committed checkpoint: **{latest.complete_scripts:,}/{latest.total_scripts:,} complete scripts ({latest.complete_percent:.2f}%)**.
- Mapped ability uses: **{latest.mapped_uses:,}/{latest.total_uses:,} ({latest.mapped_percent:.2f}%)**.
- Owner-priority scripts complete: **{latest.priority_complete:,}/365**.
- Across this evidence series: **+{gained_scripts:,} complete scripts** and **+{gained_uses:,} mapped uses** over {len(points)} committed checkpoints, including **{corrections} reviewed downward correction(s)**.
- T3.3 60% floor remaining: **{max(0, math.ceil(0.60 * latest.total_scripts) - latest.complete_scripts):,} scripts**.
- Mechanical 100% remaining: **{latest.total_scripts - latest.complete_scripts:,} scripts**.

## Projection

The recent six-checkpoint net cadence is **{projection['recent_rate']:.0f} complete scripts/hour**; the whole-series wall-clock rate is **{projection['historical_rate']:.0f}/hour**. These rates use elapsed time between evidence timestamps, not measured hands-on work time. Linear projection gives:

| Target | Faster observed checkpoint cadence | Whole-history wall-clock rate |
|---|---:|---:|
| 60% T3.3 floor | {projection['to_60_fast_hours']:.1f} h | {projection['to_60_slow_hours']:.1f} h |
| 100% mechanical coverage | {projection['to_100_fast_hours']:.1f} h | {projection['to_100_slow_hours']:.1f} h |

These are trend scenarios, not delivery promises. Neither rate measures active engineering time. The faster rate assumes compatible high-yield families remain available and work continues in dense batches. The historical rate includes inactive gaps and small experimental batches. The 100% estimate is especially optimistic because the tail shifts toward linked abilities, open selectors, unsupported values, replacement effects, and rules requiring new semantic design; actual time can be several times the linear estimate, and some items may appropriately remain quarantined until later runtime work.

## Independent Review Corrections

Ampere's review found two P1 fail-open paths and one P2 test gap in an earlier mapper batch. The source-zone issue now lowers closed source moves through `MoveZoneFrom(source, origin, destination)`, retaining the legacy no-op guard when the source is not in the declared origin. `RestrictValid` now checks each complete branch against an exact closed vocabulary, rejecting approved-prefix extensions such as `Spell.Runtime.Arbitrary`. Focused regressions now cover dynamic restricted-mana amounts, both dynamic Dig counts, and subtype removal across Continuous, Animate, and AnimateAll. These findings temporarily invalidated the older false-positive checkpoint; the current exact checkpoint was regenerated after remediation.

Downward corrections are retained in the CSV and plots. Projections use net changes rather than deleting regressions or ignoring nonpositive intervals.

## How Coverage Is Being Built

1. **Structural typed lowering.** Legacy Forge script forms are parsed into explicit typed CardDef operations, selectors, conditions, costs, events, and values. Unsupported or ambiguous forms remain quarantined instead of being guessed.
2. **Blocker histograms.** Development sweeps regenerate exact blocker families and counts. This exposes concentration in unsupported parameters, values, and unmapped APIs.
3. **Singleton and high-yield ranking.** Candidate families are ranked by scripts that would become fully translatable if that family were closed, not merely by raw occurrence count.
4. **Fanout and priority weighting.** Linked sub-abilities are weighted by how many parent triggers depend on them, while the Owner priority list raises strategically important cards.
5. **Compatible-family batching.** Several families sharing a representation or parser path are implemented together before validation, reducing repetitive compile and sweep overhead.
6. **24-worker shared-cache local sweeps.** Development and exact checkpoint sweeps use up to 24 local workers and one Cargo cache. This avoids duplicate caches and GitHub Actions consumption.
7. **Exact deterministic checkpoints.** Source is committed before the checkpoint. Workspace tests, clippy with warnings denied, compiler round trips, database validation, oracle/nightmare/smoke suites, coverage floors, parallel replay, and blocker-plan replay bind evidence to an exact product commit and tree.

## Reading The Evidence

- `coverage_over_time.png` shows complete-script and mapped-use percentages at committed checkpoints.
- `checkpoint_throughput.png` shows the gain associated with each evidence checkpoint; the sequence maps to `checkpoint_series.csv`.
- `coverage_projection.png` compares faster observed-checkpoint and whole-history linear scenarios.
- `checkpoint_series.csv` is the machine-readable series, including evidence and mapper commits.

## Important Boundary

This is **structural mapping evidence**, not proof that every translated card behaves correctly in a complete game runtime. T3.5/T3.6 semantic, differential, oracle, and card-specific verification remain necessary. A rising mapped-use percentage means fewer scripts are rejected by the typed importer; it does not by itself establish visual correctness, gameplay completeness, or zero bugs.
"""
    (OUT / "REPORT.md").write_text(text, encoding="utf-8")


def verify_pngs() -> None:
    from PIL import Image, ImageStat

    for path in OUT.glob("*.png"):
        image = Image.open(path).convert("RGB")
        stat = ImageStat.Stat(image)
        if image.width < 400 or image.height < 300 or sum(stat.var) < 10:
            raise RuntimeError(f"plot appears blank or undersized: {path}")


def main() -> None:
    points = committed_points()
    if len(points) < 2:
        raise RuntimeError("not enough committed T3.3 coverage evidence")
    projection = projections(points)
    write_csv(points)
    plot_coverage(points)
    plot_deltas(points)
    plot_projection(points, projection)
    write_report(points, projection)
    verify_pngs()
    print(f"created {len(points)}-checkpoint report in {OUT}")


if __name__ == "__main__":
    main()
