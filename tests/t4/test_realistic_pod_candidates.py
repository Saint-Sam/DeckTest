#!/usr/bin/env python3
"""Fail-closed tests for the T4 Realistic Pod v1 candidate packet."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import unittest
from collections import Counter, defaultdict
from pathlib import Path, PurePosixPath, PureWindowsPath


ROOT = Path(__file__).resolve().parents[2]
CANDIDATE_PATH = ROOT / "assets/ai/pods/realistic-pod-v1-candidates.json"
PILOT_PATH = ROOT / "assets/ai/pilot_intents/realistic-pod-v1-candidates.json"
INVENTORY_PATH = ROOT / "reports/gates/T4-CARDS/realistic-pod-v1-inventory.json"
BLOCKERS_PATH = ROOT / "reports/gates/T4-CARDS/realistic-pod-v1-blockers.json"
SCHEMA_PATH = ROOT / "schemas/t4/realistic_pod_candidates.schema.json"

SPEC = importlib.util.spec_from_file_location(
    "build_t4_card_admission", ROOT / "tools/build_t4_card_admission.py"
)
assert SPEC is not None and SPEC.loader is not None
ADMISSION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADMISSION)


def load(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(value, dict)
    return value


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class RealisticPodCandidateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.candidate = load(CANDIDATE_PATH)
        cls.pilot = load(PILOT_PATH)
        cls.inventory = load(INVENTORY_PATH)
        cls.blockers = load(BLOCKERS_PATH)
        cls.catalog, _, _ = ADMISSION.load_catalog(ROOT)

    def normalize(self, candidate: dict | None = None, pilot: dict | None = None) -> dict:
        return ADMISSION.realistic_manifest(
            ROOT,
            candidate if candidate is not None else copy.deepcopy(self.candidate),
            "assets/ai/pods/realistic-pod-v1-candidates.json",
            pilot if pilot is not None else copy.deepcopy(self.pilot),
            self.catalog,
            ADMISSION.RUNTIME_PRODUCT_COMMIT,
            ADMISSION.RUNTIME_PRODUCT_TREE,
            sha256(PILOT_PATH),
        )

    def test_schema_and_candidate_only_boundary(self) -> None:
        schema = load(SCHEMA_PATH)
        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertEqual(self.candidate["schema_version"], 1)
        self.assertEqual(self.candidate["status"], "candidate_blocked_pending_admission")
        self.assertEqual(self.candidate["freeze_status"], "not_frozen")
        self.assertFalse(self.candidate["claim_boundary"]["promotion_eligible"])
        self.assertFalse(self.candidate["claim_boundary"]["cp_ai_realistic_pod_passed"])
        self.assertEqual(
            self.candidate["product_binding"],
            {"commit": ADMISSION.RUNTIME_PRODUCT_COMMIT, "tree": ADMISSION.RUNTIME_PRODUCT_TREE},
        )

    def test_four_exact_legal_commander_decks(self) -> None:
        decks = self.candidate["candidate_decks"]
        self.assertEqual(len(decks), 4)
        self.assertEqual({deck["selection_rank"] for deck in decks}, {1, 2, 3, 4})
        for deck in decks:
            deck_colors = set(deck["color_identity"])
            names: set[str] = set()
            slots = 0
            lands = 0
            for card in deck["mainboard"]:
                self.assertNotEqual(card["name"], deck["commander"])
                self.assertNotIn(card["name"], names)
                names.add(card["name"])
                self.assertLessEqual(set(card["color_identity"]), deck_colors)
                self.assertEqual(card["basic_land"], "Basic Land" in card["type_line"])
                if card["count"] != 1:
                    self.assertTrue(card["basic_land"])
                self.assertEqual(self.catalog[card["identity_id"]]["name"], card["name"])
                slots += card["count"]
                if "Land" in card["type_line"]:
                    lands += card["count"]
            self.assertEqual(slots, 99)
            self.assertEqual(deck["mainboard_slots"], 99)
            self.assertEqual(deck["total_slots"], 100)
            self.assertEqual(deck["land_slots"], lands)
            self.assertEqual(deck["nonland_slots"], 99 - lands)

        zaxara = next(deck for deck in decks if deck["deck_id"] == "zaxara-ramp-value")
        self.assertEqual(zaxara["color_identity"], ["B", "G", "U"])
        self.assertTrue(
            {
                "Blue Sun's Zenith",
                "Exsanguinate",
                "Genesis Hydra",
                "Hydroid Krasis",
                "Torment of Hailfire",
                "Villainous Wealth",
                "Walking Ballista",
            }
            <= {card["name"] for card in zaxara["mainboard"]}
        )

    def test_exact_identity_membership_and_slot_accounting(self) -> None:
        memberships: dict[str, set[str]] = defaultdict(set)
        slot_counts: Counter[str] = Counter()
        for deck in self.candidate["candidate_decks"]:
            memberships[deck["commander"]].add(deck["deck_id"])
            slot_counts[deck["commander"]] += 1
            for card in deck["mainboard"]:
                memberships[card["name"]].add(deck["deck_id"])
                slot_counts[card["name"]] += card["count"]

        universe = self.candidate["selected_pod"]["candidate_identity_universe"]
        self.assertEqual(len(universe), 282)
        self.assertGreaterEqual(len(universe), 250)
        self.assertLessEqual(len(universe), 350)
        self.assertEqual(set(universe), set(memberships))
        records = {record["name"]: record for record in self.candidate["candidates"]}
        self.assertEqual(set(records), set(universe))
        for name, record in records.items():
            self.assertEqual(record["deck_ids"], sorted(memberships[name]))
            self.assertEqual(record["slot_count"], slot_counts[name])
            self.assertEqual(self.catalog[record["identity_id"]]["name"], name)
            self.assertEqual(record["candidate_status"], "blocked")

    def test_pilot_intents_are_exact_and_on_deck(self) -> None:
        decks = {deck["deck_id"]: deck for deck in self.candidate["candidate_decks"]}
        intents = {intent["deck_id"]: intent for intent in self.pilot["intents"]}
        self.assertEqual(set(intents), set(decks))
        self.assertEqual(self.pilot["provenance"]["inventory_sha256"], sha256(CANDIDATE_PATH))
        for deck_id, intent in intents.items():
            deck = decks[deck_id]
            deck_names = {deck["commander"]} | {card["name"] for card in deck["mainboard"]}
            self.assertEqual(intent["commander"], deck["commander"])
            self.assertEqual(intent["color_identity"], deck["color_identity"])
            self.assertLessEqual(set(intent["tutor_priorities"]), deck_names)
            self.assertLessEqual(set(intent["signature_cards"]), deck_names)
            self.assertIn("Fail closed", intent["unsupported_line_policy"])

    def test_inventory_and_blocker_reports_bind_exact_inputs(self) -> None:
        for report in (self.inventory, self.blockers):
            self.assertEqual(report["status"], "blocked_candidate")
            self.assertFalse(report["promotion_eligible"])
            self.assertEqual(
                report["input_bindings"]["candidate_manifest"]["sha256"],
                sha256(CANDIDATE_PATH),
            )
            self.assertEqual(
                report["input_bindings"]["pilot_intents"]["sha256"],
                sha256(PILOT_PATH),
            )
            self.assertEqual(report["summary"]["unique_identity_count"], 282)
        for family in self.blockers["blocker_families"]:
            self.assertEqual(
                family["affected_card_count"],
                len(set(family["affected_identity_ids"])),
            )
            self.assertEqual(
                family["affected_deck_count"],
                len(set(family["affected_deck_ids"])),
            )

    def test_no_private_absolute_or_traversal_paths(self) -> None:
        def visit(value, key: str = "") -> None:
            if isinstance(value, dict):
                for child_key, child in value.items():
                    visit(child, child_key)
            elif isinstance(value, list):
                for child in value:
                    visit(child, key)
            elif isinstance(value, str):
                self.assertNotIn("/Users/", value)
                if "path" in key.lower():
                    self.assertFalse(PurePosixPath(value).is_absolute())
                    self.assertFalse(PureWindowsPath(value).is_absolute())
                    self.assertNotIn("..", PurePosixPath(value).parts)

        for artifact in (self.candidate, self.pilot, self.inventory, self.blockers):
            visit(artifact)

    def test_admission_normalization_accepts_exact_candidate_packet(self) -> None:
        normalized = self.normalize()
        self.assertEqual(normalized["manifest_id"], "realistic-pod-v1-candidates")
        self.assertEqual(normalized["candidate_deck_count"], 4)
        self.assertEqual(normalized["candidate_identity_universe_count"], 282)
        self.assertEqual(len(normalized["candidates"]), 282)
        self.assertTrue(all(candidate["deck_ids"] for candidate in normalized["candidates"]))

    def test_off_color_card_and_nonbasic_duplicate_fail_closed(self) -> None:
        off_color = copy.deepcopy(self.candidate)
        krenko = next(deck for deck in off_color["candidate_decks"] if deck["deck_id"] == "krenko-goblin-pressure")
        krenko["mainboard"][0]["color_identity"] = ["G"]
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "off-color"):
            self.normalize(off_color)

        duplicate = copy.deepcopy(self.candidate)
        deck = duplicate["candidate_decks"][0]
        card = next(item for item in deck["mainboard"] if not item["basic_land"])
        card["count"] = 2
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "repeats nonbasic"):
            self.normalize(duplicate)

    def test_membership_and_pilot_drift_fail_closed(self) -> None:
        membership = copy.deepcopy(self.candidate)
        membership["candidates"][0]["deck_ids"] = ["made-up-deck"]
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "deck membership mismatch"):
            self.normalize(membership)

        pilot = copy.deepcopy(self.pilot)
        pilot["intents"][0]["signature_cards"].append("Forest")
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "references cards outside"):
            self.normalize(pilot=pilot)

    def test_product_hash_and_path_drift_fail_closed(self) -> None:
        stale = copy.deepcopy(self.candidate)
        stale["product_binding"]["commit"] = "0" * 40
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "stale product"):
            self.normalize(stale)

        unsafe = copy.deepcopy(self.candidate)
        unsafe["provenance"]["normalization_source"]["repository_path_if_present"] = "../outside.json"
        with self.assertRaisesRegex(ADMISSION.AdmissionError, "unsafe"):
            self.normalize(unsafe)


if __name__ == "__main__":
    unittest.main()
