#!/usr/bin/env python3
"""Adversarial integrity tests for the frozen T4 engineering regression pod."""

from __future__ import annotations

import json
import shutil
import tempfile
import unittest
from pathlib import Path

from tools.verify_t4_regression_pod import (
    EXPECTED_FIXTURE_SHA256,
    EXPECTED_PRODUCT_COMMIT,
    EXPECTED_PRODUCT_TREE,
    EXPECTED_SOURCE_GIT_BLOB_SHA1,
    EXPECTED_SOURCE_SHA256,
    RegressionPodValidationError,
    load_regression_pod,
    validate_regression_pod,
)


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "assets/t3_9/integration_decks.json"
FIXTURE = ROOT / "assets/ai/pods/regression-v1.json"
REPORT = ROOT / "reports/gates/T4-CARDS/regression-v1-manifest.json"


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def copy_environment(directory: Path) -> Path:
    """Build a temporary repository containing only validator inputs."""

    for relative in (
        "assets/t3_9/integration_decks.json",
        "assets/ai/pods/regression-v1.json",
        "reports/gates/T4-CARDS/regression-v1-manifest.json",
        "metrics/card_semantics_100.json",
    ):
        destination = directory / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)

    source = load_json(SOURCE)
    source_root = directory / "target/translated-cards"
    for deck in source["decks"]:
        paths = [deck["commander"]] + [card["path"] for card in deck["cards"]]
        for relative in paths:
            destination = source_root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "target/translated-cards" / relative, destination)
    return directory


class RegressionPodManifestTests(unittest.TestCase):
    def assert_failure_codes(self, callback, *expected: str) -> None:
        with self.assertRaises(RegressionPodValidationError) as context:
            callback()
        failures = set(context.exception.failures)
        self.assertTrue(set(expected) <= failures, sorted(failures))

    def validate_copy(self, directory: Path) -> dict:
        return validate_regression_pod(
            directory,
            expected_product_commit=EXPECTED_PRODUCT_COMMIT,
            expected_product_tree=EXPECTED_PRODUCT_TREE,
        )

    def test_validator_accepts_exact_frozen_pod(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = self.validate_copy(copy_environment(Path(temporary)))

        self.assertEqual(result["status"], "validated_fail_closed")
        self.assertTrue(result["consumable"])
        self.assertEqual(result["fixture_sha256"], EXPECTED_FIXTURE_SHA256)
        self.assertEqual(result["source_sha256"], EXPECTED_SOURCE_SHA256)
        self.assertEqual(result["source_git_blob_sha1"], EXPECTED_SOURCE_GIT_BLOB_SHA1)

    def test_loader_validates_before_returning_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            fixture = load_regression_pod(
                directory,
                expected_product_commit=EXPECTED_PRODUCT_COMMIT,
                expected_product_tree=EXPECTED_PRODUCT_TREE,
            )

        self.assertEqual(fixture["fixture_version"], "v1")
        self.assertEqual(fixture["taxonomy"]["total_deck_slots"], 400)

    def test_source_tampering_rejects_hash_blob_and_deck_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            source_path = directory / "assets/t3_9/integration_decks.json"
            source = load_json(source_path)
            source["decks"][0]["cards"][0]["count"] += 1
            write_json(source_path, source)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "source_sha256_mismatch",
                "source_git_blob_sha1_mismatch",
                "deck_snapshot_mismatch",
            )

    def test_fixture_tampering_rejects_fixture_hash_and_snapshot_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            fixture_path = directory / "assets/ai/pods/regression-v1.json"
            fixture = load_json(fixture_path)
            fixture["decks"][0]["cards"][0]["count"] += 1
            write_json(fixture_path, fixture)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "fixture_sha256_mismatch",
                "manifest_fixture_sha256_mismatch",
                "deck_snapshot_mismatch",
            )

    def test_unsafe_source_path_is_rejected_before_consumption(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            fixture_path = directory / "assets/ai/pods/regression-v1.json"
            fixture = load_json(fixture_path)
            fixture["source"]["manifest_path"] = "../outside.json"
            write_json(fixture_path, fixture)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "source_manifest_unsafe_path",
            )

    def test_unsafe_card_path_is_rejected_even_when_source_is_tampered(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            source_path = directory / "assets/t3_9/integration_decks.json"
            source = load_json(source_path)
            source["decks"][0]["cards"][0]["path"] = "../outside.frs"
            write_json(source_path, source)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "card_0_0_unsafe_path",
                "source_sha256_mismatch",
            )

    def test_missing_source_path_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            fixture_path = directory / "assets/ai/pods/regression-v1.json"
            fixture = load_json(fixture_path)
            fixture["source"]["manifest_path"] = "assets/t3_9/missing.json"
            write_json(fixture_path, fixture)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "source_manifest_missing",
            )

    def test_explicit_product_commit_and_tree_are_checked(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))

            self.assert_failure_codes(
                lambda: validate_regression_pod(
                    directory,
                    expected_product_commit="0" * 40,
                    expected_product_tree=EXPECTED_PRODUCT_TREE,
                ),
                "expected_product_commit_mismatch",
                "fixture_product_commit_mismatch",
                "manifest_product_commit_mismatch",
            )

    def test_manifest_product_and_fixture_bindings_cannot_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            report_path = directory / "reports/gates/T4-CARDS/regression-v1-manifest.json"
            report = load_json(report_path)
            report["product_binding"]["tree"] = "0" * 40
            report["fixture_binding"]["version"] = "v2"
            write_json(report_path, report)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "manifest_product_tree_mismatch",
                "manifest_fixture_version_mismatch",
            )

    def test_manifest_checks_are_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = copy_environment(Path(temporary))
            report_path = directory / "reports/gates/T4-CARDS/regression-v1-manifest.json"
            report = load_json(report_path)
            report["checks"]["source_sha256_matches"] = False
            write_json(report_path, report)

            self.assert_failure_codes(
                lambda: self.validate_copy(directory),
                "manifest_check_false:source_sha256_matches",
            )


if __name__ == "__main__":
    unittest.main()
