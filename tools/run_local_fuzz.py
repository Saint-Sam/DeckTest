#!/usr/bin/env python3
"""Run bounded sanitizer fuzz workers using local CPU and isolated build state."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


TARGETS = [
    "fuzz_apply",
    "fuzz_characteristics",
    "fuzz_scenarioparse",
    "fuzz_carddsl",
    "fuzz_carddb",
]

DONE_PATTERN = re.compile(r"Done (?P<runs>\d+) runs in (?P<seconds>\d+) second\(s\)")
STAT_PATTERN = re.compile(r"^stat::(?P<name>[a-z_]+):\s+(?P<value>\d+)\s*$", re.MULTILINE)


def source_hash(root: Path) -> str:
    digest = hashlib.sha256()
    paths = [
        root / "fuzz/Cargo.toml",
        root / "tools/run_local_fuzz.py",
        root / "scripts/fuzz_local_parallel.sh",
        root / "rust-toolchain.toml",
        root / "Cargo.toml",
        root / "Cargo.lock",
        *sorted((root / "fuzz/fuzz_targets").glob("*.rs")),
    ]
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


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def display_path(root: Path, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(root.resolve()))
    except ValueError:
        return str(path.resolve())


def resolve_report_path(root: Path, value: object) -> Path:
    path = Path(str(value))
    return path if path.is_absolute() else root / path


def command_output(command: list[str], root: Path) -> str:
    result = subprocess.run(
        command,
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode, command, output=result.stdout, stderr=result.stderr
        )
    return result.stdout.strip()


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
    run_root: Path,
    evidence_dir: Path,
    worker: int,
    target: str,
    seconds: int,
    sanitizer: str,
) -> dict[str, object]:
    base = run_root / f"worker-{worker:02d}-{target}"
    corpus = base / "corpus"
    artifacts = base / "artifacts"
    build = base / "target"
    for path in (corpus, artifacts, build, evidence_dir):
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
    started_at = utc_now()
    started = time.monotonic()
    timed_out = False
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
        return_code: int | None = result.returncode
    except subprocess.TimeoutExpired as error:
        result = None
        timed_out = True
        output_value = error.stdout or ""
        output = output_value.decode() if isinstance(output_value, bytes) else output_value
        return_code = None
    finished_at = utc_now()
    elapsed_seconds = round(time.monotonic() - started, 3)
    done_match = DONE_PATTERN.search(output)
    statistics = {
        match.group("name"): int(match.group("value"))
        for match in STAT_PATTERN.finditer(output)
    }
    verified_runtime_seconds = int(done_match.group("seconds")) if done_match else 0
    completed_runs = int(done_match.group("runs")) if done_match else 0
    final_units = int(statistics.get("number_of_executed_units", 0))
    final_stats_present = (
        done_match is not None
        and final_units == completed_runs
        and int(statistics.get("average_exec_per_sec", 0)) > 0
        and "peak_rss_mb" in statistics
    )
    if result is None:
        status = "failed"
        reason = "controller timeout"
    elif result.returncode != 0:
        status = "failed"
        reason = "fuzzer returned nonzero"
    elif not final_stats_present:
        status = "failed"
        reason = "missing or inconsistent final libFuzzer statistics"
    elif verified_runtime_seconds < seconds:
        status = "failed"
        reason = "verified fuzzer runtime is shorter than requested"
    else:
        status = "passed"
        reason = "clean run with verified final statistics"
    log_path = evidence_dir / f"worker-{worker:02d}-{target}.log"
    header = (
        f"started_at={started_at}\n"
        f"finished_at={finished_at}\n"
        f"elapsed_seconds={elapsed_seconds}\n"
        f"target_directory={build}\n"
        f"artifact_directory={artifacts}\n"
        f"command={json.dumps(command)}\n"
        f"return_code={return_code}\n"
        "--- output ---\n"
    )
    log_path.write_text(header + output)
    if status != "passed":
        tail = "\n".join(output.splitlines()[-30:])
        print(f"FUZZ FAILURE worker={worker} target={target}\n{tail}", file=sys.stderr)
    artifact_rows = [
        {
            "path": display_path(root, path),
            "sha256": file_hash(path),
            "size_bytes": path.stat().st_size,
        }
        for path in sorted(artifacts.rglob("*"))
        if path.is_file()
    ]
    return {
        "worker": worker,
        "target": target,
        "requested_seconds": seconds,
        "verified_runtime_seconds": verified_runtime_seconds,
        "completed_runs": completed_runs,
        "final_statistics": statistics,
        "sanitizer": sanitizer,
        "status": status,
        "reason": reason,
        "started_at": started_at,
        "finished_at": finished_at,
        "elapsed_seconds": elapsed_seconds,
        "return_code": return_code,
        "timed_out": timed_out,
        "command": command,
        "log": display_path(root, log_path),
        "log_sha256": file_hash(log_path),
        "artifacts": artifact_rows,
        "artifact_directory": display_path(root, artifacts),
        "target_directory": display_path(root, build),
    }


def evaluate(report: dict[str, object], minimum_worker_seconds: int) -> tuple[bool, str]:
    rows = report.get("workers", [])
    if not isinstance(rows, list):
        return False, "worker records are missing"
    target_names = {row.get("target") for row in rows if isinstance(row, dict)}
    total = sum(
        int(row.get("verified_runtime_seconds", 0))
        for row in rows
        if isinstance(row, dict)
    )
    all_passed = len(rows) > 0 and all(
        isinstance(row, dict)
        and row.get("status") == "passed"
        and int(row.get("verified_runtime_seconds", 0))
        >= int(row.get("requested_seconds", 0))
        and int(row.get("completed_runs", 0)) > 0
        for row in rows
    )
    required = set(TARGETS)
    passed = all_passed and required.issubset(target_names) and total >= minimum_worker_seconds
    return passed, f"workers={len(rows)} verified_worker_seconds={total} targets={sorted(target_names)}"


def validate_evidence(root: Path, report: dict[str, object]) -> tuple[bool, str]:
    rows = report.get("workers")
    if not isinstance(rows, list) or not rows:
        return False, "worker evidence is missing"
    for row in rows:
        if not isinstance(row, dict):
            return False, "worker evidence record is not an object"
        path = resolve_report_path(root, row.get("log", ""))
        if not path.is_file():
            return False, f"missing full fuzz log: {path}"
        if row.get("log_sha256") != file_hash(path):
            return False, f"fuzz log hash mismatch: {path}"
        text = path.read_text()
        for marker in (
            "started_at=",
            "finished_at=",
            "target_directory=",
            "command=",
            "--- output ---",
            "stat::number_of_executed_units:",
            "stat::average_exec_per_sec:",
            "stat::peak_rss_mb:",
        ):
            if marker not in text:
                return False, f"fuzz log lacks {marker}: {path}"
        done_match = DONE_PATTERN.search(text)
        statistics = {
            match.group("name"): int(match.group("value"))
            for match in STAT_PATTERN.finditer(text)
        }
        if done_match is None:
            return False, f"fuzz completion line is absent: {path}"
        if int(done_match.group("seconds")) != int(row.get("verified_runtime_seconds", 0)):
            return False, f"fuzz runtime mismatch: {path}"
        if int(done_match.group("runs")) != int(row.get("completed_runs", 0)):
            return False, f"fuzz execution count mismatch: {path}"
        if statistics != row.get("final_statistics"):
            return False, f"fuzz final-statistics mismatch: {path}"
        for artifact in row.get("artifacts", []):
            if not isinstance(artifact, dict):
                return False, f"invalid artifact record for {path}"
            artifact_path = resolve_report_path(root, artifact.get("path", ""))
            if not artifact_path.is_file() or artifact.get("sha256") != file_hash(artifact_path):
                return False, f"fuzz artifact hash mismatch: {artifact_path}"
    return True, f"verified {len(rows)} full fuzz logs and final-stat blocks"


def run(
    root: Path,
    seconds: int,
    requested_workers: int | None,
    sanitizer: str,
    minimum_total_worker_seconds: int,
    evidence_dir: Path,
) -> int:
    if seconds < 1:
        raise ValueError("seconds must be positive")
    workers = worker_budget(root, requested_workers)
    if minimum_total_worker_seconds > 0:
        seconds = max(seconds, math.ceil(minimum_total_worker_seconds / workers))
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    run_root = root / "target/local-fuzz/runs" / run_id
    evidence_dir.mkdir(parents=True, exist_ok=True)
    for old_log in evidence_dir.glob("worker-*.log"):
        old_log.unlink()
    started_at = utc_now()
    assignments = [(index, TARGETS[index % len(TARGETS)]) for index in range(workers)]
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [
            executor.submit(
                run_worker,
                root,
                run_root,
                evidence_dir,
                index,
                target,
                seconds,
                sanitizer,
            )
            for index, target in assignments
        ]
        rows = [future.result() for future in futures]
    rows.sort(key=lambda row: int(row["worker"]))
    report: dict[str, object] = {
        "schema_version": 2,
        "reviewed_commit": command_output(["git", "rev-parse", "HEAD"], root),
        "reviewed_tree": command_output(["git", "rev-parse", "HEAD^{tree}"], root),
        "source_sha256": source_hash(root),
        "started_at": started_at,
        "finished_at": utc_now(),
        "toolchains": {
            "rustc_nightly": command_output(
                ["rustup", "run", "nightly-2026-07-05", "rustc", "--version"], root
            ),
            "cargo_fuzz": command_output(
                ["rustup", "run", "nightly-2026-07-05", "cargo", "fuzz", "--version"],
                root,
            ),
        },
        "run_directory": display_path(root, run_root),
        "evidence_directory": display_path(root, evidence_dir),
        "worker_count": workers,
        "seconds_per_worker": seconds,
        "requested_worker_seconds": workers * seconds,
        "total_worker_seconds": sum(
            int(row["verified_runtime_seconds"]) for row in rows
        ),
        "sanitizer": sanitizer,
        "workers": rows,
    }
    passed, summary = evaluate(report, minimum_total_worker_seconds or workers * seconds)
    evidence_passed, evidence_summary = validate_evidence(root, report)
    report["evidence_validation"] = evidence_summary
    report["passed"] = passed and evidence_passed
    output = root / "metrics/local_fuzz.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Local fuzz: passed={report['passed']} {summary}; {evidence_summary}")
    return 0 if report["passed"] else 1


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
    evidence_passed, evidence_summary = validate_evidence(root, report)
    print(f"Local fuzz report: passed={passed} {summary}; {evidence_summary}")
    return 0 if passed and evidence_passed and report.get("passed") is True else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--workers", type=int)
    parser.add_argument("--sanitizer", default="address")
    parser.add_argument("--minimum-total-worker-seconds", type=int, default=0)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--minimum-worker-seconds", type=int, default=60)
    args = parser.parse_args()
    configured_evidence = os.environ.get("FORGE_CP_DSL_EVIDENCE_DIR")
    evidence_dir = (
        args.evidence_dir
        or (Path(configured_evidence) / "fuzz" if configured_evidence else None)
        or args.root / "target/local-fuzz/evidence/current"
    )
    try:
        if args.check:
            return check(args.root, args.minimum_worker_seconds)
        return run(
            args.root,
            args.seconds,
            args.workers,
            args.sanitizer,
            args.minimum_total_worker_seconds,
            evidence_dir,
        )
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_local_fuzz.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
