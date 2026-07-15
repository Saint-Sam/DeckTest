#!/usr/bin/env python3
"""Validate the frozen T4 engineering regression pod before consumption.

This validator is intentionally independent of the runtime and of the current
worktree HEAD.  A caller may consume the pod only after the source manifest,
fixture bytes, card paths, product binding, and gate manifest all agree.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_VERSION = 1
FIXTURE_RELATIVE = "assets/ai/pods/regression-v1.json"
SOURCE_RELATIVE = "assets/t3_9/integration_decks.json"
REPORT_RELATIVE = "reports/gates/T4-CARDS/regression-v1-manifest.json"
SOURCE_ROOT_RELATIVE = "target/translated-cards"
SEMANTIC_REGISTRY_RELATIVE = "metrics/card_semantics_100.json"

# These constants make the v1 freeze independently checkable.  Comparing only
# values copied into the fixture or report would allow a coordinated edit of
# all of those artifacts to pass as a new v1.
EXPECTED_PRODUCT_COMMIT = "19ef3302c40db3e916d2a60925546d4ebc28608d"
EXPECTED_PRODUCT_TREE = "e79efa91e0146f23f7219367e117db34ce13867a"
EXPECTED_SOURCE_SHA256 = "0ed6260e37d1f62ad3d5463bbe9235730a31860d2c1c69c4b69f0735979c40c1"
EXPECTED_SOURCE_GIT_BLOB_SHA1 = "f6ab74fe7fcc5befdb6f158ea065bf68bf9e9e41"
EXPECTED_FIXTURE_SHA256 = "ca26b30e66a26904eeb8e7237351905d617eb9b7e3a909291ba639e58063e786"

REQUIRED_REPORT_CHECKS = frozenset(
    {
        "commander_slots_4",
        "exact_replay_contract_preserved",
        "fixture_present",
        "fixture_sha256_matches",
        "fixture_snapshot_matches_source",
        "four_decks_preserved",
        "mainboard_slots_396",
        "mainboard_unique_identities_21",
        "nonrepresentative_label_present",
        "realistic_benchmark_excluded",
        "source_git_blob_sha1_matches",
        "source_manifest_present",
        "source_schema_and_roots_match",
        "source_sha256_matches",
        "strength_and_calibration_excluded",
        "fail_closed_validator_bound",
        "diagnostic_gate_blocked",
    }
)

EXPECTED_CONSUMER_CONTRACT = {
    "validator": "tools/verify_t4_regression_pod.py",
    "gate": "scripts/gates/gate_T4_cards.sh",
    "mode": "fail_closed_before_consume",
    "product_binding": "explicit_expected_commit_and_tree_arguments",
}


class RegressionPodValidationError(ValueError):
    """Raised when the regression pod is stale, unsafe, or inconsistent."""

    def __init__(self, failures: list[str]) -> None:
        self.failures = tuple(failures)
        super().__init__("; ".join(failures))


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _git_blob_sha1(data: bytes) -> str:
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def _read_bytes(path: Path, label: str, failures: list[str]) -> bytes | None:
    try:
        return path.read_bytes()
    except OSError:
        failures.append(f"{label}_missing")
        return None


def _read_json(path: Path, label: str, failures: list[str]) -> dict[str, Any] | None:
    data = _read_bytes(path, label, failures)
    if data is None:
        return None
    try:
        value = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        failures.append(f"{label}_invalid_json")
        return None
    if not isinstance(value, dict):
        failures.append(f"{label}_not_object")
        return None
    return value


def _path_text(raw: Any) -> str | None:
    if isinstance(raw, Path):
        raw = raw.as_posix()
    if not isinstance(raw, str) or not raw or "\x00" in raw:
        return None
    return raw


def _unsafe_relative_path(raw: str) -> bool:
    # PureWindowsPath catches drive-letter and UNC paths even on POSIX, while
    # PurePosixPath catches traversal in the repository's native spelling.
    posix = PurePosixPath(raw)
    windows = PureWindowsPath(raw)
    return (
        posix.is_absolute()
        or windows.is_absolute()
        or bool(windows.drive)
        or ".." in posix.parts
        or ".." in windows.parts
    )


def _resolve_path(
    root: Path,
    raw: Any,
    label: str,
    failures: list[str],
    *,
    allow_absolute_argument: bool = False,
    require_file: bool = False,
    require_dir: bool = False,
) -> tuple[Path, str] | None:
    value = _path_text(raw)
    if value is None:
        failures.append(f"{label}_unsafe_path")
        return None
    if not allow_absolute_argument and _unsafe_relative_path(value):
        failures.append(f"{label}_unsafe_path")
        return None
    candidate = Path(value)
    if candidate.is_absolute() and not allow_absolute_argument:
        failures.append(f"{label}_unsafe_path")
        return None
    try:
        resolved = (candidate if candidate.is_absolute() else root / candidate).resolve()
        relative = resolved.relative_to(root).as_posix()
    except (OSError, ValueError):
        failures.append(f"{label}_unsafe_path")
        return None
    if not relative or relative == ".":
        failures.append(f"{label}_unsafe_path")
        return None
    if require_file and not resolved.is_file():
        failures.append(f"{label}_missing")
    if require_dir and not resolved.is_dir():
        failures.append(f"{label}_missing")
    return resolved, relative


def _resolve_child(
    root: Path,
    base: Path,
    raw: Any,
    label: str,
    failures: list[str],
) -> Path | None:
    value = _path_text(raw)
    if value is None or _unsafe_relative_path(value):
        failures.append(f"{label}_unsafe_path")
        return None
    try:
        resolved = (base / value).resolve()
        resolved.relative_to(base)
        resolved.relative_to(root)
    except (OSError, ValueError):
        failures.append(f"{label}_unsafe_path")
        return None
    if not resolved.is_file():
        failures.append(f"{label}_missing")
    return resolved


def _equal(
    actual: Any,
    expected: Any,
    code: str,
    failures: list[str],
) -> bool:
    if actual != expected:
        failures.append(code)
        return False
    return True


def _valid_product_id(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and value == value.lower() and all(
        char in "0123456789abcdef" for char in value
    )


def _taxonomy(decks: Any, failures: list[str]) -> dict[str, Any] | None:
    if not isinstance(decks, list):
        failures.append("decks_not_array")
        return None
    mainboard_slots = 0
    mainboard_paths: set[str] = set()
    commander_paths: set[str] = set()
    for index, deck in enumerate(decks):
        if not isinstance(deck, dict):
            failures.append(f"deck_{index}_not_object")
            continue
        commander = deck.get("commander")
        if not isinstance(commander, str):
            failures.append(f"deck_{index}_commander_invalid")
        else:
            commander_paths.add(commander)
        cards = deck.get("cards")
        if not isinstance(cards, list):
            failures.append(f"deck_{index}_cards_not_array")
            continue
        for card_index, card in enumerate(cards):
            if not isinstance(card, dict):
                failures.append(f"deck_{index}_card_{card_index}_not_object")
                continue
            path = card.get("path")
            count = card.get("count")
            if not isinstance(path, str) or not path:
                failures.append(f"deck_{index}_card_{card_index}_path_invalid")
            else:
                mainboard_paths.add(path)
            if isinstance(count, bool) or not isinstance(count, int) or count < 1:
                failures.append(f"deck_{index}_card_{card_index}_count_invalid")
            else:
                mainboard_slots += count
    return {
        "deck_count": len(decks),
        "mainboard_slots": mainboard_slots,
        "commander_slots": len(decks),
        "total_deck_slots": mainboard_slots + len(decks),
        "mainboard_unique_identity_count": len(mainboard_paths),
        "commander_unique_identity_count": len(commander_paths),
        "distinct_identity_count_including_commanders": len(mainboard_paths | commander_paths),
    }


def _report_taxonomy(fixture_taxonomy: Any) -> dict[str, Any] | None:
    if not isinstance(fixture_taxonomy, dict):
        return None
    keys = (
        "deck_count",
        "mainboard_slots",
        "commander_slots",
        "total_deck_slots",
        "mainboard_unique_identity_count",
        "commander_unique_identity_count",
        "distinct_identity_count_including_commanders",
    )
    if any(key not in fixture_taxonomy for key in keys):
        return None
    return {key: fixture_taxonomy[key] for key in keys} | {
        "role": "deterministic engineering regression, not realistic Commander benchmarking"
    }


def _path_or_none(
    root: Path,
    value: Any,
    label: str,
    failures: list[str],
) -> Path | None:
    resolved = _resolve_path(root, value, label, failures, require_file=True)
    return resolved[0] if resolved is not None else None


def validate_regression_pod(
    root: Path = ROOT,
    *,
    fixture_path: str | Path = FIXTURE_RELATIVE,
    manifest_path: str | Path = REPORT_RELATIVE,
    expected_product_commit: str | None = None,
    expected_product_tree: str | None = None,
) -> dict[str, Any]:
    """Validate v1 and return a machine-readable validation summary.

    The expected product values are deliberately not read from git.  When
    omitted, the fixture's pinned values are used for convenience; evidence
    callers and the local gate should pass both arguments explicitly.
    """

    root = Path(root).resolve()
    failures: list[str] = []
    fixture_result = _resolve_path(
        root,
        fixture_path,
        "fixture",
        failures,
        allow_absolute_argument=True,
        require_file=True,
    )
    report_result = _resolve_path(
        root,
        manifest_path,
        "manifest",
        failures,
        allow_absolute_argument=True,
        require_file=True,
    )
    if fixture_result is None or report_result is None:
        raise RegressionPodValidationError(sorted(set(failures)))
    fixture_file, fixture_relative = fixture_result
    report_file, report_relative = report_result
    _equal(fixture_relative, FIXTURE_RELATIVE, "fixture_path_mismatch", failures)
    _equal(report_relative, REPORT_RELATIVE, "manifest_path_mismatch", failures)

    fixture_bytes = _read_bytes(fixture_file, "fixture", failures)
    report_bytes = _read_bytes(report_file, "manifest", failures)
    fixture = _read_json(fixture_file, "fixture", failures)
    report = _read_json(report_file, "manifest", failures)
    if fixture is None or report is None or fixture_bytes is None or report_bytes is None:
        raise RegressionPodValidationError(sorted(set(failures)))

    fixture_sha256 = _sha256(fixture_bytes)
    _equal(fixture_sha256, EXPECTED_FIXTURE_SHA256, "fixture_sha256_mismatch", failures)
    report_fixture_hint = report.get("fixture_binding")
    if not isinstance(report_fixture_hint, dict):
        report_fixture_hint = {}
    _equal(
        _sha256(fixture_bytes),
        report_fixture_hint.get("sha256"),
        "manifest_fixture_sha256_mismatch",
        failures,
    )

    source_binding = fixture.get("source")
    if not isinstance(source_binding, dict):
        failures.append("source_binding_missing")
        source_binding = {}
    source_result = _resolve_path(
        root,
        source_binding.get("manifest_path"),
        "source_manifest",
        failures,
        require_file=True,
    )
    source_file = source_result[0] if source_result is not None else None
    source = _read_json(source_file, "source_manifest", failures) if source_file else None
    source_bytes = _read_bytes(source_file, "source_manifest", failures) if source_file else None

    expected_commit = (
        expected_product_commit
        if expected_product_commit is not None
        else source_binding.get("product_commit")
    )
    expected_tree = expected_product_tree if expected_product_tree is not None else source_binding.get("product_tree")
    if expected_product_commit is not None and not _valid_product_id(expected_product_commit):
        failures.append("expected_product_commit_invalid")
    if expected_product_tree is not None and not _valid_product_id(expected_product_tree):
        failures.append("expected_product_tree_invalid")
    if expected_product_commit is None and expected_product_tree is not None:
        failures.append("expected_product_commit_missing")
    if expected_product_tree is None and expected_product_commit is not None:
        failures.append("expected_product_tree_missing")
    if not _valid_product_id(expected_commit):
        failures.append("expected_product_commit_invalid")
    if not _valid_product_id(expected_tree):
        failures.append("expected_product_tree_invalid")
    if _valid_product_id(expected_commit):
        _equal(expected_commit, EXPECTED_PRODUCT_COMMIT, "expected_product_commit_mismatch", failures)
    if _valid_product_id(expected_tree):
        _equal(expected_tree, EXPECTED_PRODUCT_TREE, "expected_product_tree_mismatch", failures)

    _equal(fixture.get("schema_version"), SCHEMA_VERSION, "fixture_schema_mismatch", failures)
    _equal(fixture.get("fixture_id"), "t4-engineering-regression-pod", "fixture_id_mismatch", failures)
    _equal(fixture.get("fixture_version"), "v1", "fixture_version_mismatch", failures)
    freeze = fixture.get("freeze")
    _equal(
        freeze.get("status") if isinstance(freeze, dict) else None,
        "immutable",
        "fixture_not_immutable",
        failures,
    )
    classification = fixture.get("classification")
    if not isinstance(classification, dict):
        failures.append("classification_missing")
    else:
        _equal(classification.get("kind"), "engineering_regression", "classification_mismatch", failures)
        for key in (
            "representative_of_normal_commander",
            "benchmark_eligible",
            "ai_strength_calibration_eligible",
            "search_knee_eligible",
            "product_cost_estimation_eligible",
            "promotion_eligible",
        ):
            _equal(classification.get(key), False, f"classification_{key}_mismatch", failures)

    source_root_raw = fixture.get("source_root")
    semantic_registry_raw = fixture.get("semantic_registry")
    source_root = _resolve_path(root, source_root_raw, "source_root", failures, require_dir=True)
    semantic_registry = _resolve_path(
        root,
        semantic_registry_raw,
        "semantic_registry",
        failures,
        require_file=True,
    )
    _equal(source_root_raw, SOURCE_ROOT_RELATIVE, "source_root_binding_mismatch", failures)
    _equal(semantic_registry_raw, SEMANTIC_REGISTRY_RELATIVE, "semantic_registry_binding_mismatch", failures)

    if source is not None:
        _equal(source.get("schema_version"), SCHEMA_VERSION, "source_schema_mismatch", failures)
        _equal(source.get("schema_version"), fixture.get("schema_version"), "source_fixture_schema_mismatch", failures)
        _equal(source.get("source_root"), source_root_raw, "source_root_mismatch", failures)
        _equal(source.get("semantic_registry"), semantic_registry_raw, "semantic_registry_mismatch", failures)
        _equal(source.get("decks"), fixture.get("decks"), "deck_snapshot_mismatch", failures)
    if source_bytes is not None:
        source_sha256 = _sha256(source_bytes)
        source_blob_sha1 = _git_blob_sha1(source_bytes)
        _equal(source_sha256, EXPECTED_SOURCE_SHA256, "source_sha256_mismatch", failures)
        _equal(source_blob_sha1, EXPECTED_SOURCE_GIT_BLOB_SHA1, "source_git_blob_sha1_mismatch", failures)
        _equal(source_sha256, source_binding.get("sha256"), "fixture_source_sha256_mismatch", failures)
        _equal(source_blob_sha1, source_binding.get("git_blob_sha1"), "fixture_source_git_blob_sha1_mismatch", failures)

    source_decks = source.get("decks") if isinstance(source, dict) else None
    taxonomy = _taxonomy(source_decks, failures)
    fixture_taxonomy = fixture.get("taxonomy")
    if taxonomy is not None:
        if not isinstance(fixture_taxonomy, dict):
            failures.append("fixture_taxonomy_missing")
        else:
            for key, value in taxonomy.items():
                _equal(fixture_taxonomy.get(key), value, f"taxonomy_{key}_mismatch", failures)

    if source_root is not None and isinstance(source_decks, list):
        for deck_index, deck in enumerate(source_decks):
            if not isinstance(deck, dict):
                continue
            _resolve_child(root, source_root[0], deck.get("commander"), f"commander_{deck_index}", failures)
            cards = deck.get("cards")
            if not isinstance(cards, list):
                continue
            for card_index, card in enumerate(cards):
                if isinstance(card, dict):
                    _resolve_child(
                        root,
                        source_root[0],
                        card.get("path"),
                        f"card_{deck_index}_{card_index}",
                        failures,
                    )

    _equal(source_binding.get("manifest_path"), SOURCE_RELATIVE, "fixture_source_path_mismatch", failures)
    _equal(source_binding.get("schema_version"), SCHEMA_VERSION, "fixture_source_schema_mismatch", failures)
    _equal(source_binding.get("product_commit"), expected_commit, "fixture_product_commit_mismatch", failures)
    _equal(source_binding.get("product_tree"), expected_tree, "fixture_product_tree_mismatch", failures)
    drift_detection = fixture.get("drift_detection")
    _equal(
        drift_detection.get("mode") if isinstance(drift_detection, dict) else None,
        "fail_closed",
        "drift_mode_mismatch",
        failures,
    )

    report_product = report.get("product_binding")
    if not isinstance(report_product, dict):
        failures.append("manifest_product_binding_missing")
        report_product = {}
    _equal(report_product.get("commit"), expected_commit, "manifest_product_commit_mismatch", failures)
    _equal(report_product.get("tree"), expected_tree, "manifest_product_tree_mismatch", failures)
    report_source = report.get("source_binding")
    if not isinstance(report_source, dict):
        failures.append("manifest_source_binding_missing")
        report_source = {}
    expected_report_source = {
        "path": SOURCE_RELATIVE,
        "schema_version": SCHEMA_VERSION,
        "sha256": EXPECTED_SOURCE_SHA256,
        "git_blob_sha1": EXPECTED_SOURCE_GIT_BLOB_SHA1,
        "source_root": SOURCE_ROOT_RELATIVE,
        "semantic_registry": SEMANTIC_REGISTRY_RELATIVE,
    }
    for key, value in expected_report_source.items():
        _equal(report_source.get(key), value, f"manifest_source_{key}_mismatch", failures)
    report_fixture = report.get("fixture_binding")
    if not isinstance(report_fixture, dict):
        failures.append("manifest_fixture_binding_missing")
        report_fixture = {}
    for key, value in {
        "path": FIXTURE_RELATIVE,
        "version": "v1",
        "sha256": EXPECTED_FIXTURE_SHA256,
    }.items():
        _equal(report_fixture.get(key), value, f"manifest_fixture_{key}_mismatch", failures)

    _equal(report.get("schema_version"), SCHEMA_VERSION, "manifest_schema_mismatch", failures)
    _equal(report.get("pod_id"), fixture.get("fixture_id"), "manifest_fixture_id_mismatch", failures)
    _equal(report.get("pod_version"), fixture.get("fixture_version"), "manifest_fixture_version_mismatch", failures)
    _equal(report.get("status"), "frozen_nonrepresentative", "manifest_status_mismatch", failures)
    _equal(report.get("promotion_eligible"), False, "manifest_promotion_mismatch", failures)
    expected_report_taxonomy = _report_taxonomy(fixture_taxonomy)
    _equal(report.get("taxonomy"), expected_report_taxonomy, "manifest_taxonomy_mismatch", failures)
    _equal(report.get("consumer_contract"), EXPECTED_CONSUMER_CONTRACT, "manifest_consumer_contract_mismatch", failures)

    report_checks = report.get("checks")
    if not isinstance(report_checks, dict):
        failures.append("manifest_checks_missing")
    else:
        missing_checks = REQUIRED_REPORT_CHECKS - set(report_checks)
        failures.extend(f"manifest_check_missing:{key}" for key in sorted(missing_checks))
        failures.extend(
            f"manifest_check_false:{key}"
            for key in sorted(REQUIRED_REPORT_CHECKS)
            if report_checks.get(key) is not True
        )

    if failures:
        raise RegressionPodValidationError(sorted(set(failures)))
    return {
        "status": "validated_fail_closed",
        "fixture_path": fixture_relative,
        "manifest_path": report_relative,
        "source_path": SOURCE_RELATIVE,
        "product_binding": {"commit": expected_commit, "tree": expected_tree},
        "fixture_sha256": fixture_sha256,
        "source_sha256": EXPECTED_SOURCE_SHA256,
        "source_git_blob_sha1": EXPECTED_SOURCE_GIT_BLOB_SHA1,
        "checks": {key: True for key in sorted(REQUIRED_REPORT_CHECKS)},
        "consumable": True,
    }


def load_regression_pod(
    root: Path = ROOT,
    **kwargs: Any,
) -> dict[str, Any]:
    """Validate and then load the fixture, preventing unvalidated use."""

    validate_regression_pod(root, **kwargs)
    fixture_path = kwargs.get("fixture_path", FIXTURE_RELATIVE)
    fixture_result = _resolve_path(Path(root).resolve(), fixture_path, "fixture", [], allow_absolute_argument=True)
    if fixture_result is None:
        raise RegressionPodValidationError(["fixture_missing_after_validation"])
    value = json.loads(fixture_result[0].read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RegressionPodValidationError(["fixture_not_object_after_validation"])
    return value


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--fixture", type=Path, default=Path(FIXTURE_RELATIVE))
    parser.add_argument(
        "--manifest",
        "--report",
        dest="manifest",
        type=Path,
        default=Path(REPORT_RELATIVE),
    )
    parser.add_argument(
        "--product-commit",
        "--expected-product-commit",
        dest="product_commit",
        default=None,
        help="exact runtime product commit; never inferred from git HEAD",
    )
    parser.add_argument(
        "--product-tree",
        "--expected-product-tree",
        dest="product_tree",
        default=None,
        help="exact runtime product tree; never inferred from git HEAD",
    )
    parser.add_argument("--json", action="store_true", help="emit a JSON result")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        result = validate_regression_pod(
            args.root,
            fixture_path=args.fixture,
            manifest_path=args.manifest,
            expected_product_commit=args.product_commit,
            expected_product_tree=args.product_tree,
        )
    except RegressionPodValidationError as exc:
        payload = {"status": "blocked", "consumable": False, "failures": list(exc.failures)}
        if args.json:
            print(json.dumps(payload, sort_keys=True))
        else:
            print("T4 regression pod blocked: " + "; ".join(exc.failures), file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(result, sort_keys=True))
    else:
        print(
            "PASS verify_t4_regression_pod.py "
            f"(fail-closed validated; product={result['product_binding']['commit']})"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
