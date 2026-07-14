#!/usr/bin/env python3
"""Run the optimized local T3.9 pod gate with product and resource binding."""

from __future__ import annotations

import argparse
import json
import os
import platform
import resource
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def disk_snapshot() -> dict[str, int]:
    usage = shutil.disk_usage(ROOT)
    return {"total_bytes": usage.total, "used_bytes": usage.used, "free_bytes": usage.free}


def run(command: list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True, env={**os.environ, "CARGO_NET_OFFLINE": "true"})


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
    tracked_dirty = git("status", "--porcelain", "--untracked-files=no")
    if tracked_dirty and not args.allow_dirty:
        raise SystemExit("exact pod gate requires a tracked-clean product worktree")
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
    before_usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    run(
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
    elapsed = time.monotonic() - started
    after_usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    after_disk = disk_snapshot()
    replay_paths = sorted(replay_dir.glob("pod-seed-*.frsreplay"))
    if len(replay_paths) != 10:
        raise SystemExit(f"expected 10 action replays, found {len(replay_paths)}")
    for replay_path in replay_paths:
        run([str(ROOT / "target/release/forge-cli"), "replay", str(replay_path)])

    with report.open(encoding="utf-8") as handle:
        payload: dict[str, Any] = json.load(handle)
    payload.setdefault("results", {})["cli_action_replays_verified"] = len(replay_paths)
    max_rss_raw = int(after_usage.ru_maxrss)
    max_rss_bytes = max_rss_raw if platform.system() == "Darwin" else max_rss_raw * 1024
    payload["product_binding"] = {
        "commit": commit,
        "tree": tree,
        "tracked_clean_at_start": not bool(tracked_dirty),
    }
    payload["resources"] = {
        "platform": platform.platform(),
        "logical_cpu_count": os.cpu_count(),
        "worker_ceiling": 24,
        "workers_used": args.jobs,
        "wall_seconds": elapsed,
        "child_user_cpu_seconds": after_usage.ru_utime - before_usage.ru_utime,
        "child_system_cpu_seconds": after_usage.ru_stime - before_usage.ru_stime,
        "child_max_rss_bytes": max_rss_bytes,
        "disk_before": before_disk,
        "disk_after": after_disk,
        "disk_free_headroom_bytes": after_disk["free_bytes"],
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
    print(f"PASS: exact local pod gate bound to {commit} ({elapsed:.2f}s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
