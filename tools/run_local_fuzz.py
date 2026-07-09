#!/usr/bin/env python3
"""Run bounded sanitizer fuzz workers using local CPU and isolated build state."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
import shutil
import subprocess
import sys
from pathlib import Path


TARGETS = [
    "fuzz_apply",
    "fuzz_characteristics",
    "fuzz_scenarioparse",
    "fuzz_carddsl",
    "fuzz_carddb",
]


def source_hash(root: Path) -> str:
    digest = hashlib.sha256()
    paths = [root / "fuzz/Cargo.toml", *sorted((root / "fuzz/fuzz_targets").glob("*.rs"))]
    for crate in ("forge-cardc", "forge-carddef", "forge-cards", "forge-core", "forge-testkit"):
        paths.append(root / f"crates/{crate}/Cargo.toml")
        paths.extend(sorted((root / f"crates/{crate}/src").glob("*.rs")))
    paths.extend(sorted((root / "cards/cp_dsl/definitions").glob("*.frs")))
    paths.extend(sorted((root / "cards/integration/layers").glob("*.frs")))
    for path in paths:
        digest.update(str(path.relative_to(root)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def worker_budget(root: Path, requested: int | None) -> int:
    if requested is not None:
        if requested < 1:
            raise ValueError("workers must be positive")
        cpu_workers = requested
    elif os.environ.get("FORGE_FUZZ_WORKERS"):
        cpu_workers = int(os.environ["FORGE_FUZZ_WORKERS"])
    else:
        result = subprocess.run(
            [str(root / "scripts/local_workers.sh")],
            cwd=root,
            check=True,
            text=True,
            capture_output=True,
        )
        cpu_workers = int(result.stdout.strip())
    free = shutil.disk_usage(root).free
    reserve = 20 * 1024**3
    disk_workers = max(1, (max(0, free - reserve)) // (600 * 1024**2))
    return max(1, min(cpu_workers, int(disk_workers), 8))


def shared_corpora(root: Path, target: str) -> list[Path]:
    if target == "fuzz_carddsl":
        return [
            root / "cards/cp_dsl/definitions",
            root / "cards/integration/layers",
        ]
    if target == "fuzz_carddb":
        path = root / "target/local-fuzz/shared/fuzz_carddb"
        path.mkdir(parents=True, exist_ok=True)
        seed = path / "versioned-header"
        seed.write_bytes(b"FORGECDB" + (1).to_bytes(4, "little"))
        return [path]
    path = root / "fuzz/corpus" / target
    path.mkdir(parents=True, exist_ok=True)
    return [path]


def run_worker(
    root: Path,
    worker: int,
    target: str,
    seconds: int,
    sanitizer: str,
) -> dict[str, object]:
    base = root / "target/local-fuzz" / f"worker-{worker:02d}-{target}"
    corpus = base / "corpus"
    artifacts = base / "artifacts"
    build = base / "target"
    logs = root / "target/local-fuzz/logs"
    for path in (corpus, artifacts, build, logs):
        path.mkdir(parents=True, exist_ok=True)
    command = [
        "cargo",
        "+nightly-2026-07-05",
        "fuzz",
        "run",
        "--sanitizer",
        sanitizer,
        target,
        str(corpus),
        *(str(path) for path in shared_corpora(root, target)),
        "--",
        f"-max_total_time={seconds}",
        "-timeout=10",
        "-rss_limit_mb=4096",
        "-max_len=65536",
        "-print_final_stats=1",
        f"-artifact_prefix={artifacts}/",
    ]
    environment = os.environ.copy()
    environment.update(
        {
            "CARGO_BUILD_JOBS": "1",
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(build),
        }
    )
    try:
        result = subprocess.run(
            command,
            cwd=root,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=seconds + 300,
        )
        output = result.stdout
        status = "passed" if result.returncode == 0 else "failed"
        reason = "clean" if result.returncode == 0 else "fuzzer returned nonzero"
    except subprocess.TimeoutExpired as error:
        output = error.stdout or ""
        status = "failed"
        reason = "controller timeout"
    log_path = logs / f"worker-{worker:02d}-{target}.log"
    log_path.write_text(output)
    if status != "passed":
        tail = "\n".join(output.splitlines()[-30:])
        print(f"FUZZ FAILURE worker={worker} target={target}\n{tail}", file=sys.stderr)
    return {
        "worker": worker,
        "target": target,
        "seconds": seconds,
        "sanitizer": sanitizer,
        "status": status,
        "reason": reason,
        "log": str(log_path.relative_to(root)),
        "artifact_directory": str(artifacts.relative_to(root)),
        "target_directory": str(build.relative_to(root)),
    }


def evaluate(report: dict[str, object], minimum_worker_seconds: int) -> tuple[bool, str]:
    rows = report.get("workers", [])
    if not isinstance(rows, list):
        return False, "worker records are missing"
    target_names = {row.get("target") for row in rows if isinstance(row, dict)}
    total = sum(int(row.get("seconds", 0)) for row in rows if isinstance(row, dict))
    all_passed = all(row.get("status") == "passed" for row in rows if isinstance(row, dict))
    required = set(TARGETS)
    passed = all_passed and required.issubset(target_names) and total >= minimum_worker_seconds
    return passed, f"workers={len(rows)} worker_seconds={total} targets={sorted(target_names)}"


def run(
    root: Path,
    seconds: int,
    requested_workers: int | None,
    sanitizer: str,
    minimum_total_worker_seconds: int,
) -> int:
    if seconds < 1:
        raise ValueError("seconds must be positive")
    workers = worker_budget(root, requested_workers)
    if minimum_total_worker_seconds > 0:
        seconds = max(seconds, math.ceil(minimum_total_worker_seconds / workers))
    assignments = [(index, TARGETS[index % len(TARGETS)]) for index in range(workers)]
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [
            executor.submit(run_worker, root, index, target, seconds, sanitizer)
            for index, target in assignments
        ]
        rows = [future.result() for future in futures]
    rows.sort(key=lambda row: int(row["worker"]))
    report: dict[str, object] = {
        "schema_version": 1,
        "source_sha256": source_hash(root),
        "worker_count": workers,
        "seconds_per_worker": seconds,
        "total_worker_seconds": workers * seconds,
        "sanitizer": sanitizer,
        "workers": rows,
    }
    passed, summary = evaluate(report, max(workers * seconds, minimum_total_worker_seconds))
    report["passed"] = passed
    output = root / "metrics/local_fuzz.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Local fuzz: passed={passed} {summary}")
    return 0 if passed else 1


def check(root: Path, minimum_worker_seconds: int) -> int:
    output = root / "metrics/local_fuzz.json"
    if not output.exists():
        print("missing metrics/local_fuzz.json", file=sys.stderr)
        return 1
    report = json.loads(output.read_text())
    if report.get("source_sha256") != source_hash(root):
        print("local fuzz report is stale", file=sys.stderr)
        return 1
    passed, summary = evaluate(report, minimum_worker_seconds)
    print(f"Local fuzz report: passed={passed} {summary}")
    return 0 if passed and report.get("passed") is True else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--workers", type=int)
    parser.add_argument("--sanitizer", default="address")
    parser.add_argument("--minimum-total-worker-seconds", type=int, default=0)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--minimum-worker-seconds", type=int, default=60)
    args = parser.parse_args()
    try:
        if args.check:
            return check(args.root, args.minimum_worker_seconds)
        return run(
            args.root,
            args.seconds,
            args.workers,
            args.sanitizer,
            args.minimum_total_worker_seconds,
        )
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_local_fuzz.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
