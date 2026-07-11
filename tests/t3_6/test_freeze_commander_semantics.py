#!/usr/bin/env python3
"""Focused tests for the bounded T3.6 candidate freeze."""

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TOOL_PATH = ROOT / "tools/freeze_t3_6_commander_semantics.py"
SPEC = importlib.util.spec_from_file_location("t3_6_freeze", TOOL_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {TOOL_PATH}")
FREEZE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = FREEZE
SPEC.loader.exec_module(FREEZE)


class FreezeCommanderSemanticsTests(unittest.TestCase):
    def test_repository_replay_is_deterministic_and_complete(self) -> None:
        first = FREEZE.build_manifest(ROOT)
        second = FREEZE.build_manifest(ROOT)

        self.assertEqual(FREEZE.render_json(first), FREEZE.render_json(second))
        self.assertEqual(first["summary"]["selected_count"], 100)
        self.assertEqual(
            len(first["selected"]) + len(first["exclusions"]),
            first["summary"]["priority_identity_count"],
        )
        self.assertEqual(sum(item["quota"] for item in first["strata"]), 100)
        self.assertEqual(len({item["oracle_id"] for item in first["selected"]}), 100)
        self.assertTrue(
            all(
                item["reason_code"] in FREEZE.EXCLUSION_REASONS
                for item in first["exclusions"]
            )
        )

    def test_payload_hash_rejects_tampering(self) -> None:
        manifest = FREEZE.build_manifest(ROOT)
        self.assertTrue(FREEZE.verify_payload_hash(manifest))

        tampered = copy.deepcopy(manifest)
        tampered["selected"][0]["oracle_id"] = "tampered"
        self.assertFalse(FREEZE.verify_payload_hash(tampered))

    def test_zone_fields_are_order_independent_and_nonland_is_not_land(self) -> None:
        bounce = (
            "A:SP$ ChangeZone | ValidTgts$ Permanent.nonLand+YouDontCtrl | "
            "Origin$ Battlefield | Destination$ Hand"
        )
        forest_search = (
            "A:SP$ ChangeZone | Origin$ Library | Destination$ Battlefield | "
            "ChangeType$ Forest"
        )

        self.assertTrue(FREEZE.targeted_interaction(bounce))
        self.assertFalse(FREEZE.land_ramp(bounce))
        self.assertTrue(FREEZE.land_ramp(forest_search))
        self.assertFalse(FREEZE.library_or_graveyard_access(forest_search))

    def test_priority_parser_rejects_casefolded_duplicates(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate priority name"):
            FREEZE.parse_priority("# TIER 0 - universal\nSol Ring|sol ring\n")


if __name__ == "__main__":
    unittest.main()
