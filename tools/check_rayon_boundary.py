#!/usr/bin/env python3
"""Fail when Rayon crosses the approved forge-porttools production boundary."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ALLOWED_MANIFEST = Path("crates/forge-porttools/Cargo.toml")
ALLOWED_SOURCE_ROOT = Path("crates/forge-porttools")
RAYON_SOURCE = re.compile(r"(?:\buse\s+rayon\b|\bextern\s+crate\s+rayon\b|\brayon::)")


def cargo_metadata(root: Path, manifest: Path) -> dict:
    command = [
        "cargo",
        "metadata",
        "--no-deps",
        "--offline",
        "--locked",
        "--format-version",
        "1",
        "--manifest-path",
        str(manifest),
    ]
    completed = subprocess.run(
        command, cwd=root, check=False, capture_output=True, text=True
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "cargo metadata failed")
    value = json.loads(completed.stdout)
    if not isinstance(value, dict):
        raise RuntimeError("cargo metadata did not return an object")
    return value


def metadata_documents(root: Path) -> list[dict]:
    documents = [cargo_metadata(root, root / "Cargo.toml")]
    fuzz_manifest = root / "fuzz/Cargo.toml"
    if fuzz_manifest.is_file():
        documents.append(cargo_metadata(root, fuzz_manifest))
    return documents


def rayon_package_violations(metadata: dict, root: Path) -> list[str]:
    violations: list[str] = []
    packages = metadata.get("packages", [])
    if not isinstance(packages, list):
        return ["cargo metadata packages field is not a list"]
    for package in packages:
        if not isinstance(package, dict):
            continue
        manifest_value = package.get("manifest_path")
        if not isinstance(manifest_value, str):
            continue
        manifest = Path(manifest_value).resolve()
        try:
            relative = manifest.relative_to(root)
        except ValueError:
            relative = manifest
        dependencies = package.get("dependencies", [])
        if not isinstance(dependencies, list):
            continue
        for dependency in dependencies:
            if isinstance(dependency, dict) and dependency.get("name") == "rayon":
                if relative != ALLOWED_MANIFEST:
                    kind = dependency.get("kind") or "normal"
                    violations.append(f"{relative}: direct Rayon dependency ({kind})")
    return violations


def rust_sources(root: Path) -> list[Path]:
    crates = root / "crates"
    return sorted(path for path in crates.rglob("*.rs") if path.is_file())


def check_workspace(root: Path) -> list[str]:
    violations: list[str] = []
    try:
        documents = metadata_documents(root)
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        return [f"could not inspect Cargo metadata: {error}"]
    for metadata in documents:
        violations.extend(rayon_package_violations(metadata, root))

    for source in rust_sources(root):
        relative = source.relative_to(root)
        try:
            text = source.read_text(encoding="utf-8")
        except OSError as error:
            violations.append(f"{relative}: cannot read source: {error}")
            continue
        if RAYON_SOURCE.search(text) and not relative.is_relative_to(ALLOWED_SOURCE_ROOT):
            violations.append(f"{relative}: Rayon production import/reference outside porttools")
    return violations


def self_test() -> None:
    root = Path("/workspace")
    allowed = {
        "packages": [
            {
                "manifest_path": "/workspace/crates/forge-porttools/Cargo.toml",
                "dependencies": [{"name": "rayon", "kind": None}],
            }
        ]
    }
    forbidden = {
        "packages": [
            {
                "manifest_path": "/workspace/crates/forge-core/Cargo.toml",
                "dependencies": [{"name": "rayon", "kind": "dev"}],
            }
        ]
    }
    criterion_only = {
        "packages": [
            {
                "manifest_path": "/workspace/crates/forge-core/Cargo.toml",
                "dependencies": [{"name": "criterion", "kind": "dev"}],
            }
        ]
    }
    assert not rayon_package_violations(allowed, root)
    assert rayon_package_violations(forbidden, root)
    assert not rayon_package_violations(criterion_only, root)
    assert RAYON_SOURCE.search("use rayon::prelude::*;")
    assert not RAYON_SOURCE.search("criterion may depend on Rayon transitively")
    print("PASS check_rayon_boundary.py self-test")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0

    root = args.root.resolve()
    violations = check_workspace(root)
    if violations:
        for violation in violations:
            print(f"ERROR: {violation}", file=sys.stderr)
        return 1
    print("PASS Rayon boundary: direct dependency and production references are porttools-only")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
