#!/usr/bin/env python3
"""Validate and record CP-DSL gate evidence from local artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_checked(command: list[str], root: Path) -> str:
    environment = os.environ.copy()
    environment.setdefault("CARGO_NET_OFFLINE", "true")
    result = subprocess.run(
        command,
        cwd=root,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if result.returncode != 0:
        raise ValueError(f"command failed: {' '.join(command)}\n{result.stdout}")
    return result.stdout


def build_report(root: Path) -> dict[str, object]:
    catalog = load_json(root / "metrics/card_catalog.json")
    corpus = load_json(root / "metrics/cp_dsl_corpus.json")
    manifest = load_json(root / "cards/cp_dsl/manifest.json")
    malformed = load_json(root / "cards/cp_dsl/malformed/manifest.json")
    mutation = load_json(root / "metrics/cp_dsl_mutation.json")
    local_fuzz = load_json(root / "metrics/local_fuzz.json")
    local_platforms = load_json(root / "metrics/local_platforms.json")
    oracle_semantics = load_json(root / "metrics/oracle_semantics.json")

    build_dir = root / "target/card-regression"
    databases = [build_dir / f"carddb-{index}.bin" for index in range(1, 4)]
    indexes = [build_dir / f"carddb-{index}.index.json" for index in range(1, 4)]
    layer_databases = [build_dir / f"layer-scenarios-{index}.bin" for index in range(1, 4)]
    layer_indexes = [
        build_dir / f"layer-scenarios-{index}.index.json" for index in range(1, 4)
    ]
    for path in [*databases, *indexes, *layer_databases, *layer_indexes]:
        if not path.is_file():
            raise ValueError(f"missing deterministic-build artifact: {path}")
    database_hashes = [sha256(path) for path in databases]
    index_hashes = [sha256(path) for path in indexes]
    deterministic = len(set(database_hashes)) == 1 and len(set(index_hashes)) == 1
    layer_database_hashes = [sha256(path) for path in layer_databases]
    layer_index_hashes = [sha256(path) for path in layer_indexes]
    layer_deterministic = (
        len(set(layer_database_hashes)) == 1 and len(set(layer_index_hashes)) == 1
    )

    first_bytes = databases[0].read_bytes()[:12]
    versioned_header = first_bytes[:8] == b"FORGECDB" and first_bytes[8:12] == (1).to_bytes(4, "little")
    index = load_json(indexes[0])
    identity_count = len(index.get("identities", []))
    printing_count = len(index.get("printings", []))
    definition_count = int(manifest.get("card_count", 0))
    malformed_cases = malformed.get("cases", [])
    recursive_argument_cases = [
        case
        for case in malformed_cases
        if isinstance(case, dict) and case.get("category") == "recursive_argument"
    ] if isinstance(malformed_cases, list) else []
    recursive_argument_kinds = {
        str(case.get("argument_kind")) for case in recursive_argument_cases
    }
    required_argument_kinds = {
        str(kind) for kind in malformed.get("required_argument_kinds", [])
    }
    recursive_argument_features = {
        str(feature)
        for case in recursive_argument_cases
        for feature in case.get("features", [])
        if isinstance(case.get("features", []), list)
    }
    recursive_argument_depths = {
        int(case.get("depth", 0)) for case in recursive_argument_cases
    }
    mandatory_strata = corpus.get("mandatory_strata", [])
    manifest_strata = manifest.get("strata", {})
    closed_strata = (
        isinstance(mandatory_strata, list)
        and len(mandatory_strata) == 25
        and len(set(mandatory_strata)) == 25
        and isinstance(manifest_strata, dict)
        and set(manifest_strata) == set(mandatory_strata)
        and corpus.get("missing_mandatory_strata") == []
        and corpus.get("unexpected_strata") == []
        and manifest.get("missing_mandatory_strata") == []
        and manifest.get("unexpected_strata") == []
        and all(int(count) == 4 for count in manifest_strata.values())
    )

    runtime_output = run_checked(
        [str(root / "target/debug/forge-cards"), "validate", str(databases[0])], root
    )
    nightmare_output = run_checked(
        [
            str(root / "target/debug/forge-arena"),
            "--nightmare-suite",
            "--games",
            "10",
            "--max-turns",
            "2",
        ],
        root,
    )
    run_checked(
        ["cargo", "test", "--quiet", "-p", "forge-cardc", "--test", "malformed_corpus"],
        root,
    )
    run_checked(["python3", "tools/local_platform_metrics.py", "--validate-only"], root)
    run_checked(["python3", "tools/oracle_semantic_metrics.py", "--check"], root)
    run_checked(["python3", "tools/run_cp_dsl_mutation.py", "--check"], root)
    run_checked(
        [
            "python3",
            "tools/run_local_fuzz.py",
            "--check",
            "--minimum-worker-seconds",
            "2400",
        ],
        root,
    )

    expected_identities = int(catalog.get("expected_classified_identities", 0))
    checks = {
        "catalog_coverage": catalog.get("imported_english_printings")
        == catalog.get("source_english_records"),
        "identity_classification": int(catalog.get("classified_identities", 0))
        == expected_identities,
        "zero_dangling_references": catalog.get("dangling_printing_references") == 0,
        "one_hundred_roundtrips": definition_count == 100,
        "mandatory_strata_exact": closed_strata,
        "catalog_only_classification_separate": int(catalog.get("catalog_only", 0)) > 0
        and corpus.get("catalog_only_records_verified_separately") is True,
        "expressiveness_threshold": int(corpus.get("distinct_operations", 0)) >= 50,
        "recursive_argument_diagnostics_threshold": len(recursive_argument_cases) >= 50
        and int(malformed.get("recursive_argument_case_count", 0))
        == len(recursive_argument_cases)
        and recursive_argument_kinds == required_argument_kinds
        and malformed.get("missing_argument_kinds") == []
        and recursive_argument_depths.issuperset({1, 2, 3, 4})
        and {
            "bare_symbol",
            "category_correct_wrong_argument",
            "prose",
            "variadic",
        }.issubset(recursive_argument_features),
        "deterministic_database": deterministic,
        "versioned_database_header": versioned_header,
        "runtime_loader": "validated" in runtime_output,
        "compiled_card_driven_nightmare_fixtures": "10 compiled card-driven fixture(s)"
        in nightmare_output
        and layer_deterministic,
        "mutation_threshold": mutation.get("passed") is True
        and float(mutation.get("mutation_score_percent", 0.0)) >= 90.0
        and int(mutation.get("survived", 1)) == 0
        and int(mutation.get("invalid", 1)) == 0,
        "local_sanitizer_fuzz": local_fuzz.get("passed") is True
        and int(local_fuzz.get("total_worker_seconds", 0)) >= 2400,
        "local_cross_target_checks": local_platforms.get("passed") is True
        and len(local_platforms.get("targets", [])) == 4,
        "semantic_oracle_breadth": oracle_semantics.get("passed") is True,
        "database_catalog_counts": identity_count == int(catalog.get("classified_identities", 0))
        and printing_count == int(catalog.get("imported_english_printings", 0)),
    }
    return {
        "schema_version": 1,
        "passed": all(checks.values()),
        "checks": checks,
        "catalog": {
            "source_english_printings": catalog.get("source_english_records"),
            "imported_english_printings": catalog.get("imported_english_printings"),
            "source_oracle_identities": catalog.get("source_unique_english_oracle_ids"),
            "fallback_identities": catalog.get("source_fallback_identities"),
            "classified_identities": catalog.get("classified_identities"),
            "dangling_references": catalog.get("dangling_printing_references"),
        },
        "corpus": {
            "definitions": definition_count,
            "canonical_roundtrips": definition_count,
            "primary_strata": corpus.get("distinct_primary_strata"),
            "mandatory_strata": mandatory_strata,
            "missing_mandatory_strata": corpus.get("missing_mandatory_strata"),
            "unexpected_strata": corpus.get("unexpected_strata"),
            "minimum_cards_per_stratum": corpus.get("minimum_cards_per_stratum"),
            "distinct_operations": corpus.get("distinct_operations"),
            "malformed_positioned_diagnostics": malformed.get("case_count"),
            "recursive_argument_diagnostics": len(recursive_argument_cases),
            "recursive_argument_kinds": sorted(recursive_argument_kinds),
            "card_driven_nightmare_fixtures": 10,
        },
        "database": {
            "schema_version": 1,
            "identity_count": identity_count,
            "printing_count": printing_count,
            "definition_count": definition_count,
            "sha256": database_hashes[0],
            "index_sha256": index_hashes[0],
            "identical_clean_builds": 3 if deterministic else 0,
            "layer_scenario_sha256": layer_database_hashes[0],
            "layer_scenario_index_sha256": layer_index_hashes[0],
            "identical_layer_scenario_builds": 3 if layer_deterministic else 0,
        },
        "mutation": {
            "mutants": mutation.get("mutant_count"),
            "killed": mutation.get("killed"),
            "survived": mutation.get("survived"),
            "invalid": mutation.get("invalid"),
            "score_percent": mutation.get("mutation_score_percent"),
        },
        "local_fuzz": {
            "workers": local_fuzz.get("worker_count"),
            "seconds_per_worker": local_fuzz.get("seconds_per_worker"),
            "total_worker_seconds": local_fuzz.get("total_worker_seconds"),
            "sanitizer": local_fuzz.get("sanitizer"),
        },
        "local_platforms": {
            "targets": local_platforms.get("targets"),
            "rustc": local_platforms.get("rustc"),
        },
        "oracle_semantics": oracle_semantics.get("measured"),
        "provenance": {
            "source_snapshot_sha256": manifest.get("source_snapshot_sha256"),
            "catalog_source_sha256": catalog.get("source", {}).get("source_sha256")
            if isinstance(catalog.get("source"), dict)
            else None,
            "compiler_test_source_sha256": mutation.get("source_sha256"),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = args.root / "metrics/cp_dsl_verification.json"
    try:
        report = build_report(args.root)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.check:
            if not output.exists() or output.read_text() != rendered:
                raise ValueError("CP-DSL verification report is stale")
        else:
            output.write_text(rendered)
        print(
            "CP-DSL verification: "
            f"passed={report['passed']} cards={report['corpus']['definitions']} "
            f"strata={report['corpus']['primary_strata']} "
            f"mutation={report['mutation']['score_percent']}%"
        )
        return 0 if report["passed"] else 1
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"cp_dsl_metrics.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
