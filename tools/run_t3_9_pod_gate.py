#!/usr/bin/env python3
"""Run the optimized local T3.9 pod gate with product and resource binding."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional


ROOT = Path(__file__).resolve().parents[1]
MAX_POD_RSS_BYTES = 2 * 1024 * 1024 * 1024
MIN_DISK_FREE_BYTES = 5 * 1024 * 1024 * 1024
MAX_POD_WALL_SECONDS = 300.0
RESOURCE_MARKER = "__FORGE_POD_RESOURCE__"
MEASURE_HELPER = r"""
import json
import os
import platform
import resource
import subprocess
import sys
import time

before = resource.getrusage(resource.RUSAGE_CHILDREN)
started = time.monotonic()
result = subprocess.run(sys.argv[1:], check=False)
elapsed = time.monotonic() - started
after = resource.getrusage(resource.RUSAGE_CHILDREN)
max_rss = int(after.ru_maxrss)
if platform.system() != "Darwin":
    max_rss *= 1024
print("__FORGE_POD_RESOURCE__" + json.dumps({
    "wall_seconds": elapsed,
    "child_user_cpu_seconds": after.ru_utime - before.ru_utime,
    "child_system_cpu_seconds": after.ru_stime - before.ru_stime,
    "child_max_rss_bytes": max_rss,
}))
raise SystemExit(result.returncode)
"""


def git(*args: str) -> str:
    # Preserve porcelain status' leading columns while still dropping line endings.
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).rstrip()


def disk_snapshot() -> dict[str, int]:
    usage = shutil.disk_usage(ROOT)
    return {"total_bytes": usage.total, "used_bytes": usage.used, "free_bytes": usage.free}


def run(command: list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True, env={**os.environ, "CARGO_NET_OFFLINE": "true"})


def run_measured(command: list[str]) -> dict[str, Any]:
    result = subprocess.run(
        [sys.executable, "-c", MEASURE_HELPER, *command],
        cwd=ROOT,
        env={**os.environ, "CARGO_NET_OFFLINE": "true"},
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    measurement: Optional[dict[str, Any]] = None
    for line in result.stdout.splitlines():
        if line.startswith(RESOURCE_MARKER):
            value = json.loads(line.removeprefix(RESOURCE_MARKER))
            if not isinstance(value, dict):
                raise RuntimeError("pod resource helper returned an invalid record")
            measurement = value
        else:
            print(line)
    if result.stderr:
        print(result.stderr, file=sys.stderr, end="")
    if result.returncode != 0:
        raise subprocess.CalledProcessError(result.returncode, command)
    if measurement is None:
        raise RuntimeError("pod resource helper returned no measurement")
    return measurement


def evidence_only_path(path: str) -> bool:
    return (
        path.startswith("metrics/")
        or path.startswith("reports/")
        or path in {"PLAN_STATE.json", "STATUS.md", "tests/t3_6/commander_semantic_cases.json"}
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", default="assets/t3_9/integration_decks.json")
    parser.add_argument(
        "--report", default="reports/gates/T3.9/cp-four-player-pod-2026-07-13.json"
    )
    parser.add_argument("--replay-dir", default="reports/gates/T3.9/replays")
    parser.add_argument("--games", type=int, default=1000)
    parser.add_argument("--jobs", type=int, default=24)
    parser.add_argument("--max-turns", type=int, default=160)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args()

    if not 1 <= args.jobs <= 24:
        raise SystemExit("--jobs must be in 1..=24")
    status_rows = git(
        "status", "--porcelain", "--untracked-files=all", "--ignore-submodules=all"
    ).splitlines()
    changed_paths = [row[3:].split(" -> ")[-1] for row in status_rows if len(row) > 3]
    product_dirty = [path for path in changed_paths if not evidence_only_path(path)]
    if product_dirty and not args.allow_dirty:
        raise SystemExit("exact pod gate requires clean product sources")
    commit = git("rev-parse", "HEAD")
    tree = git("show", "-s", "--format=%T", commit)
    if not args.skip_build:
        run(
            [
                "cargo",
                "build",
                "--locked",
                "--offline",
                "--release",
                "--jobs",
                str(args.jobs),
                "-p",
                "forge-testkit",
                "--bin",
                "forge-t3-9-four-player-pod",
            ]
        )
        run(
            [
                "cargo",
                "build",
                "--locked",
                "--offline",
                "--release",
                "--jobs",
                str(args.jobs),
                "-p",
                "forge-cli",
                "--bin",
                "forge-cli",
            ]
        )

    report = (ROOT / args.report).resolve()
    replay_dir = (ROOT / args.replay_dir).resolve()
    report.parent.mkdir(parents=True, exist_ok=True)
    replay_dir.mkdir(parents=True, exist_ok=True)
    before_disk = disk_snapshot()
    measured = run_measured(
        [
            str(ROOT / "target/release/forge-t3-9-four-player-pod"),
            "--manifest",
            args.manifest,
            "--games",
            str(args.games),
            "--jobs",
            str(args.jobs),
            "--max-turns",
            str(args.max_turns),
            "--output",
            args.report,
            "--replay-dir",
            args.replay_dir,
        ]
    )
    after_disk = disk_snapshot()
    replay_paths = sorted(replay_dir.glob("pod-seed-*.frsreplay"))
    if len(replay_paths) != 10:
        raise SystemExit(f"expected 10 action replays, found {len(replay_paths)}")
    for replay_path in replay_paths:
        run([str(ROOT / "target/release/forge-cli"), "replay", str(replay_path)])

    with report.open(encoding="utf-8") as handle:
        payload: dict[str, Any] = json.load(handle)
    payload.setdefault("results", {})["cli_action_replays_verified"] = len(replay_paths)
    wall_seconds = float(measured["wall_seconds"])
    max_rss_bytes = int(measured["child_max_rss_bytes"])
    if wall_seconds > MAX_POD_WALL_SECONDS:
        raise SystemExit(
            f"pod wall time {wall_seconds:.2f}s exceeds {MAX_POD_WALL_SECONDS:.0f}s"
        )
    if max_rss_bytes > MAX_POD_RSS_BYTES:
        raise SystemExit(
            f"pod max RSS {max_rss_bytes} exceeds {MAX_POD_RSS_BYTES} bytes"
        )
    if after_disk["free_bytes"] < MIN_DISK_FREE_BYTES:
        raise SystemExit(
            f"disk headroom {after_disk['free_bytes']} is below {MIN_DISK_FREE_BYTES} bytes"
        )
    payload["product_binding"] = {
        "commit": commit,
        "tree": tree,
        "tracked_clean_at_start": not bool(product_dirty),
        "ignored_generated_evidence_changes": sorted(
            path for path in changed_paths if evidence_only_path(path)
        ),
    }
    payload["resources"] = {
        "measurement_scope": "pod_process_only",
        "platform": platform.platform(),
        "logical_cpu_count": os.cpu_count(),
        "worker_ceiling": 24,
        "workers_used": args.jobs,
        "wall_seconds": wall_seconds,
        "maximum_wall_seconds": MAX_POD_WALL_SECONDS,
        "child_user_cpu_seconds": measured["child_user_cpu_seconds"],
        "child_system_cpu_seconds": measured["child_system_cpu_seconds"],
        "child_max_rss_bytes": max_rss_bytes,
        "maximum_rss_bytes": MAX_POD_RSS_BYTES,
        "disk_before": before_disk,
        "disk_after": after_disk,
        "disk_free_headroom_bytes": after_disk["free_bytes"],
        "minimum_disk_free_headroom_bytes": MIN_DISK_FREE_BYTES,
    }
    report.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    run(
        [
            "python3",
            "tools/write_pod_integration.py",
            "--report",
            args.report,
            "--manifest",
            args.manifest,
        ]
    )
    print(f"PASS: exact local pod gate bound to {commit} ({wall_seconds:.2f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
