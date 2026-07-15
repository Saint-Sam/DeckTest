from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "build_t4_card_admission", ROOT / "tools/build_t4_card_admission.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


PRODUCT = "a" * 40
TREE = "b" * 40
STALE_PRODUCT = "c" * 40
STALE_TREE = "d" * 40
CARD_ID = "card-a"
CARD_B_ID = "card-b"


class CardAdmissionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "assets").mkdir()
        (self.root / "metrics").mkdir()
        (self.root / "target").mkdir()
        (self.root / "reports").mkdir()
        (self.root / "assets/card_catalog.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "identities": [
                        {"id": CARD_ID, "name": "Card A", "classification": "UnverifiedPlayable"},
                        {"id": CARD_B_ID, "name": "Card B", "classification": "UnverifiedPlayable"},
                        {"id": "out-of-scope", "name": "Out", "classification": {"OutOfV1": "test"}},
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def manifest(self, identity_id: str = CARD_ID, *, freeze_status: str = "frozen") -> Path:
        path = self.root / "candidate.json"
        path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "manifest_id": "fixture-v1",
                    "pool_kind": "development",
                    "bootstrap": False,
                    "freeze_status": freeze_status,
                    "evidence_paths": {
                        "structural_translation": "metrics/card_maturity.json",
                        "runtime": "metrics/runtime.json",
                        "family": "metrics/family.json",
                        "semantic": "metrics/semantic.json",
                        "ai": "metrics/ai.json",
                        "pod": "metrics/pod.json",
                    },
                    "candidates": [
                        {
                            "identity_id": identity_id,
                            "name": "Card A" if identity_id == CARD_ID else "Unknown",
                            "mechanic_family": "mana_rock",
                            "deck_ids": ["deck-a"],
                        }
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )
        return path

    def evidence(self, stage: str, *, identity_ids: list[str] | None = None, checks: dict | None = None,
                 product: str = PRODUCT, tree: str = TREE, passed: bool = True,
                 extra: dict | None = None, schema_version: int = 1) -> Path:
        path = self.root / "metrics" / f"{stage}.json"
        evidence_artifact = self.root / "reports" / f"{stage}-evidence.txt"
        evidence_artifact.write_text(f"fixture evidence for {stage}\n", encoding="utf-8")
        record = {
            "schema_version": schema_version,
            "stage": stage,
            "passed": passed,
            "product_commit": product,
            "product_tree": tree,
            "identity_ids": identity_ids if identity_ids is not None else [CARD_ID],
            "source": {
                "card_catalog_sha256": MODULE.sha256_file(self.root / "assets/card_catalog.json"),
            },
            "evidence": f"reports/{stage}-evidence.txt",
            "evidence_sha256": MODULE.sha256_file(evidence_artifact),
        }
        if checks is not None:
            record["checks"] = checks
        if extra:
            record.update(extra)
        path.write_text(
            json.dumps(record, sort_keys=True),
            encoding="utf-8",
        )
        return path

    def full_evidence(self, *, fallback: bool = True) -> None:
        self.evidence("card_maturity")
        self.evidence("runtime")
        self.evidence("family")
        self.evidence("semantic")
        self.evidence("ai")
        self.evidence(
            "pod",
            checks={
                "human_choice_coverage": True,
                "ai_choice_coverage": True,
                "benchmark_adapter": True,
                "hidden_information_redacted": True,
                "exact_action_replay": True,
                "no_unsupported_fallback": fallback,
                "no_card_name_branch": True,
                "performance_within_limit": True,
                "rules_unambiguous": True,
            },
        )

    def build(self, manifest: Path | None = None, *, product: str = PRODUCT, tree: str = TREE) -> dict:
        report, _ = MODULE.build_from_paths(
            self.root,
            manifest or self.manifest(),
            product,
            tree,
        )
        return report

    def test_schema_declares_required_machine_readable_contract(self) -> None:
        schema = json.loads((ROOT / "schemas/t4/card_admission.schema.json").read_text(encoding="utf-8"))
        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertEqual(schema["$defs"]["reason_code"]["enum"][:10], list(MODULE.HANDOFF_REASON_CODES))
        required = set(schema["required"])
        self.assertTrue({"product_binding", "input_hashes", "cards", "blocker_families"} <= required)
        self.full_evidence()
        report = self.build()
        self.assertEqual(report["product_binding"], {"commit": PRODUCT, "tree": TREE, "source": report["product_binding"]["source"]})
        self.assertIn("structural_translation", report["cards"][0]["evidence"])
        self.assertEqual(report["cards"][0]["status"], "benchmark_admitted")

    def test_build_is_deterministic_and_sorted(self) -> None:
        self.full_evidence()
        first = self.build()
        second = self.build()
        self.assertEqual(first, second)
        self.assertEqual(first["cards"][0]["identity_id"], CARD_ID)
        self.assertIsNone(first["metadata"]["generated_at"])

    def test_stale_product_binding_blocks_every_stage(self) -> None:
        self.full_evidence()
        report = self.build(product=STALE_PRODUCT, tree=STALE_TREE)
        card = report["cards"][0]
        self.assertEqual(card["status"], "blocked")
        self.assertEqual(card["primary_blocker"]["reason_code"], "STALE_PRODUCT_BINDING")
        self.assertFalse(report["promotion_eligible"])
        self.assertEqual(card["evidence"]["structural_translation"]["reason_code"], "STALE_PRODUCT_BINDING")

    def test_structural_translation_does_not_imply_runtime_readiness(self) -> None:
        self.evidence("card_maturity")
        manifest = self.manifest()
        manifest_value = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_value["evidence_paths"] = {
            "structural_translation": "metrics/card_maturity.json",
            "runtime": "metrics/missing-runtime.json",
            "family": None,
            "semantic": None,
            "ai": None,
            "pod": None,
        }
        manifest.write_text(json.dumps(manifest_value, sort_keys=True), encoding="utf-8")
        report = self.build(manifest)
        card = report["cards"][0]
        self.assertEqual(card["primary_blocker"]["reason_code"], "RUNTIME_UNSUPPORTED")
        self.assertEqual(card["last_verified_status"], "candidate")
        self.assertFalse(card["readiness_checks"]["runtime_execution"])

    def test_missing_family_evidence_is_semantic_blocker(self) -> None:
        self.evidence("card_maturity")
        self.evidence("runtime")
        manifest = self.manifest()
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["evidence_paths"] = {
            "structural_translation": "metrics/card_maturity.json",
            "runtime": "metrics/runtime.json",
            "family": None,
            "semantic": None,
            "ai": None,
            "pod": None,
        }
        manifest.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
        report = self.build(manifest)
        self.assertEqual(report["cards"][0]["primary_blocker"]["reason_code"], "SEMANTIC_EVIDENCE_MISSING")
        self.assertEqual(report["cards"][0]["last_verified_status"], "runtime_ready")

    def test_unknown_identity_and_fallback_behavior_fail_closed(self) -> None:
        unknown = self.build(self.manifest("unknown-id"))
        self.assertEqual(unknown["cards"][0]["primary_blocker"]["reason_code"], "IDENTITY_OUT_OF_SCOPE")
        self.full_evidence(fallback=False)
        fallback = self.build()
        card = fallback["cards"][0]
        self.assertEqual(card["status"], "blocked")
        self.assertIn("BENCHMARK_FIXTURE_MISSING", {item["reason_code"] for item in card["blockers"]})
        self.assertNotEqual(card["status"], "benchmark_admitted")

    def test_realistic_candidate_packet_is_the_default_and_binds_four_decks(self) -> None:
        report, _ = MODULE.build_from_paths(ROOT, None, MODULE.RUNTIME_PRODUCT_COMMIT, MODULE.RUNTIME_PRODUCT_TREE)
        self.assertFalse(report["candidate_manifest"]["bootstrap"])
        self.assertEqual(report["candidate_manifest"]["candidate_count"], 282)
        self.assertEqual(report["candidate_manifest"]["deck_count"], 4)
        self.assertEqual(len(report["candidate_manifest"]["selected_deck_ids"]), 4)
        self.assertEqual(report["campaign_bindings"]["deck_manifest_sha256"], report["input_hashes"]["assets/ai/pods/realistic-pod-v1-candidates.json"])
        self.assertEqual(report["campaign_bindings"]["pilot_intent_sha256"], report["input_hashes"]["assets/ai/pilot_intents/realistic-pod-v1-candidates.json"])
        self.assertEqual(report["product_binding"]["commit"], MODULE.RUNTIME_PRODUCT_COMMIT)
        self.assertEqual(report["product_binding"]["tree"], MODULE.RUNTIME_PRODUCT_TREE)
        self.assertEqual(report["status"], "blocked")

    def test_bootstrap_requires_explicit_diagnostic_mode(self) -> None:
        realistic, _ = MODULE.build_from_paths(ROOT, None, MODULE.RUNTIME_PRODUCT_COMMIT, MODULE.RUNTIME_PRODUCT_TREE)
        bootstrap, _ = MODULE.build_from_paths(ROOT, None, PRODUCT, TREE, bootstrap=True)
        self.assertFalse(realistic["candidate_manifest"]["bootstrap"])
        self.assertTrue(bootstrap["candidate_manifest"]["bootstrap"])
        self.assertEqual(bootstrap["candidate_manifest"]["candidate_count"], 21)

    def test_forged_exact_product_evidence_without_direct_hash_is_blocked(self) -> None:
        self.evidence("card_maturity")
        evidence_path = self.evidence("runtime")
        value = json.loads(evidence_path.read_text(encoding="utf-8"))
        value.pop("evidence")
        value.pop("evidence_sha256")
        evidence_path.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
        manifest = self.manifest()
        manifest_value = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_value["evidence_paths"] = {"structural_translation": "metrics/card_maturity.json", "runtime": "metrics/runtime.json", "family": None, "semantic": None, "ai": None, "pod": None}
        manifest.write_text(json.dumps(manifest_value, sort_keys=True), encoding="utf-8")
        report = self.build(manifest)
        self.assertEqual(report["cards"][0]["primary_blocker"]["reason_code"], "INPUT_INTEGRITY_FAILURE")

    def test_unknown_schema_and_status_are_rejected(self) -> None:
        self.evidence("card_maturity")
        self.evidence("runtime", schema_version=99)
        manifest = self.manifest()
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["evidence_paths"] = {"structural_translation": "metrics/card_maturity.json", "runtime": "metrics/runtime.json", "family": None, "semantic": None, "ai": None, "pod": None}
        manifest.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
        report = self.build(manifest)
        self.assertIn("INPUT_INTEGRITY_FAILURE", {item["reason_code"] for item in report["cards"][0]["blockers"]})

    def test_source_and_replay_hash_tampering_is_rejected(self) -> None:
        self.evidence("card_maturity")
        runtime_path = self.evidence("runtime")
        runtime_value = json.loads(runtime_path.read_text(encoding="utf-8"))
        runtime_value["source"]["card_catalog_sha256"] = "0" * 64
        runtime_path.write_text(json.dumps(runtime_value, sort_keys=True), encoding="utf-8")
        manifest = self.manifest()
        manifest_value = json.loads(manifest.read_text(encoding="utf-8"))
        manifest_value["evidence_paths"] = {"structural_translation": "metrics/card_maturity.json", "runtime": "metrics/runtime.json", "family": None, "semantic": None, "ai": None, "pod": None}
        manifest.write_text(json.dumps(manifest_value, sort_keys=True), encoding="utf-8")
        report = self.build(manifest)
        self.assertEqual(report["cards"][0]["primary_blocker"]["reason_code"], "INPUT_INTEGRITY_FAILURE")

        replay = self.root / "reports/replay.frsreplay"
        replay.write_bytes(b"original replay")
        self.evidence("pod", extra={"action_replays": [{"path": "reports/replay.frsreplay", "sha256": MODULE.sha256_file(replay)}]})
        replay.write_bytes(b"tampered replay")
        self.evidence("runtime")
        self.evidence("family")
        self.evidence("semantic")
        self.evidence("ai")
        pod_manifest = self.manifest()
        pod_value = json.loads(pod_manifest.read_text(encoding="utf-8"))
        pod_value["evidence_paths"] = {"structural_translation": "metrics/card_maturity.json", "runtime": "metrics/runtime.json", "family": None, "semantic": None, "ai": None, "pod": "metrics/pod.json"}
        pod_manifest.write_text(json.dumps(pod_value, sort_keys=True), encoding="utf-8")
        report = self.build(pod_manifest)
        self.assertIn("INPUT_INTEGRITY_FAILURE", {item["reason_code"] for item in report["cards"][0]["blockers"]})

    def test_absolute_and_traversal_evidence_paths_are_blocked(self) -> None:
        self.evidence("card_maturity")
        self.evidence("runtime")
        for mutated_path in (str(self.root / "metrics/runtime.json"), "../metrics/runtime.json"):
            manifest = self.manifest()
            value = json.loads(manifest.read_text(encoding="utf-8"))
            value["evidence_paths"] = {"structural_translation": "metrics/card_maturity.json", "runtime": mutated_path, "family": None, "semantic": None, "ai": None, "pod": None}
            manifest.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
            report = self.build(manifest)
            self.assertEqual(report["cards"][0]["primary_blocker"]["reason_code"], "INPUT_INTEGRITY_FAILURE")

    def test_per_card_checks_do_not_leak_between_identities(self) -> None:
        manifest = self.manifest()
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["candidates"] = [
            {"identity_id": CARD_ID, "name": "Card A", "mechanic_family": "mana_rock", "deck_ids": ["deck-a"]},
            {"identity_id": CARD_B_ID, "name": "Card B", "mechanic_family": "mana_rock", "deck_ids": ["deck-a"]},
        ]
        manifest.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
        for stage in ("card_maturity", "runtime", "family", "semantic", "ai"):
            self.evidence(stage, identity_ids=[CARD_ID, CARD_B_ID])
        checks = {
            "human_choice_coverage": True,
            "ai_choice_coverage": True,
            "benchmark_adapter": True,
            "hidden_information_redacted": True,
            "exact_action_replay": True,
            "no_unsupported_fallback": True,
            "no_card_name_branch": True,
            "performance_within_limit": True,
            "rules_unambiguous": True,
        }
        self.evidence(
            "pod",
            identity_ids=[CARD_ID, CARD_B_ID],
            extra={
                "identity_evidence": [
                    {"identity_id": CARD_ID, "passed": True, "checks": checks},
                    {"identity_id": CARD_B_ID, "passed": True, "checks": {key: False for key in checks}},
                ]
            },
        )
        report = self.build(manifest)
        by_id = {card["identity_id"]: card for card in report["cards"]}
        self.assertEqual(by_id[CARD_ID]["status"], "benchmark_admitted")
        self.assertEqual(by_id[CARD_B_ID]["status"], "blocked")

    def test_unique_blocker_arithmetic_retains_stages(self) -> None:
        candidate = {"identity_id": CARD_ID, "mechanic_family": "family", "deck_ids": ["deck-a"]}
        blocker = lambda stage: MODULE._blocker("SEMANTIC_EVIDENCE_MISSING", stage, stage, candidate)
        card = {"identity_id": CARD_ID, "blockers": [blocker("family"), blocker("semantic")]}
        family = MODULE._aggregate_blockers([card])[0]
        self.assertEqual(family["affected_card_count"], 1)
        self.assertEqual(family["affected_identity_ids"], [CARD_ID])
        self.assertEqual(family["stages"], ["family", "semantic"])

    def test_cli_check_is_deterministic_and_schema_contract_is_present(self) -> None:
        metric = self.root / "metric.json"
        gate = self.root / "gate.json"
        command = [sys.executable, str(ROOT / "tools/build_t4_card_admission.py"), "--output", str(metric), "--gate-output", str(gate)]
        subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)
        checked = subprocess.run(command + ["--check"], cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(checked.returncode, 0, checked.stderr)
        schema = json.loads((ROOT / "schemas/t4/card_admission.schema.json").read_text(encoding="utf-8"))
        for output_path in (metric, gate):
            output = json.loads(output_path.read_text(encoding="utf-8"))
            self.assertTrue(set(schema["required"]) <= set(output))
            self.assertEqual(output["schema_version"], 1)
            self.assertEqual(output["product_binding"]["commit"], MODULE.RUNTIME_PRODUCT_COMMIT)
            self.assertNotIn("/Users/", output_path.read_text(encoding="utf-8"))
            for card in output["cards"]:
                self.assertTrue(set(schema["$defs"]["card"]["required"]) <= set(card))
                self.assertIn(card["status"], {"blocked", "benchmark_admitted"})
                self.assertEqual(set(card["evidence"]), {"structural_translation", "runtime", "family", "semantic", "ai", "pod"})
            for family in output["blocker_families"]:
                self.assertEqual(family["affected_card_count"], len(set(family["affected_identity_ids"])))
        gate_value = json.loads(gate.read_text(encoding="utf-8"))
        self.assertEqual(gate_value["gate"]["gate_id"], "T4-CARDS/ADMISSION")
        self.assertFalse(gate_value["gate"]["cp_ai_realistic_pod_passed"])


if __name__ == "__main__":
    unittest.main()
