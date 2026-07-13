#!/usr/bin/env python3
"""Focused tests for the T3.6 card-specific semantic sidecar."""

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TOOL_PATH = ROOT / "tools/run_t3_6_commander_semantics.py"
SPEC = importlib.util.spec_from_file_location("t3_6_semantics", TOOL_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {TOOL_PATH}")
SEMANTICS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = SEMANTICS
SPEC.loader.exec_module(SEMANTICS)


def observed_from_cases(cases: dict) -> list[dict]:
    observed = []
    for case in cases["cases"]:
        expected = case["expected_runtime"]
        entry = {
            "path": case["translated_path"],
            "oracle_id": case["oracle_id"],
            "card_name": case["card_name"],
        }
        if expected["disposition"] == "passed":
            entry.update(expected)
            mana = []
            for expectation in SEMANTICS.expected_mana_abilities(case):
                mana.append(
                    {
                        **expectation,
                        "replayed_outputs": expectation["legal_outputs"],
                        "all_outputs_replayed": True,
                        "condition_rejected_below_threshold": (
                            True
                            if expectation["minimum_matching_permanents"] is not None
                            else None
                        ),
                    }
                )
            entry["semantic_probe"] = {
                "base_subtypes": SEMANTICS.expected_base_subtypes(case) or [],
                "mana_abilities": mana,
                "token_mana_abilities": [],
                "token_subtypes": [],
                "no_maximum_hand_size": (
                    {
                        "setup_succeeded": True,
                        "registered": True,
                        "active_for_controller": True,
                        "opponent_unaffected": True,
                        "moved_source_to_graveyard": True,
                        "expired_off_battlefield": True,
                    }
                    if any(
                        atom["op"] == "modify_player_rules"
                        for atom in case.get("semantic_atoms", [])
                    )
                    else None
                ),
            }
        else:
            entry.update(
                {
                    "disposition": expected["disposition"],
                    "code": expected["code"],
                    "detail": expected["detail"],
                }
            )
        observed.append(entry)
    return observed


class CommanderSemanticSidecarTests(unittest.TestCase):
    def setUp(self) -> None:
        self.candidates = SEMANTICS.load_json(SEMANTICS.CANDIDATES_PATH)
        self.cases = SEMANTICS.load_json(SEMANTICS.CASES_PATH)

    def test_manifest_is_hash_bound_and_accounts_for_all_candidates(self) -> None:
        self.assertTrue(SEMANTICS.verify_payload_hash(self.candidates))
        self.assertTrue(SEMANTICS.verify_payload_hash(self.cases))
        self.assertEqual(
            self.cases["candidate_payload_sha256"],
            self.candidates["payload_sha256"],
        )
        self.assertEqual(len(self.cases["cases"]), 100)
        self.assertEqual(
            [case["oracle_id"] for case in self.cases["cases"]],
            [item["oracle_id"] for item in self.candidates["selected"]],
        )
        self.assertEqual(
            self.cases["summary"],
            {
                "candidate_count": 100,
                "semantic_case_ready": 64,
                "blocked_semantic_gap": 20,
                "blocked_runtime": 16,
            },
        )

    def test_ready_cases_have_closed_typed_atoms_matching_runtime_capabilities(self) -> None:
        ready = 0
        for case in self.cases["cases"]:
            SEMANTICS.validate_expected_runtime(case)
            if case["status"] != "semantic_case_ready":
                continue
            ready += 1
            self.assertTrue(case["expected_behavior"])
            derived = []
            for atom in case["semantic_atoms"]:
                self.assertIn(atom["op"], SEMANTICS.ATOM_CAPABILITIES)
                self.assertGreater(len(atom), 1)
                derived.append(SEMANTICS.ATOM_CAPABILITIES[atom["op"]])
            self.assertEqual(derived, case["expected_runtime"]["capabilities"])
        self.assertEqual(ready, self.cases["summary"]["semantic_case_ready"])

    def test_runtime_smoke_success_does_not_promote_known_semantic_gaps(self) -> None:
        blocked = [
            case
            for case in self.cases["cases"]
            if case["status"] == "blocked_semantic_gap"
        ]
        self.assertEqual(len(blocked), self.cases["summary"]["blocked_semantic_gap"])
        self.assertTrue(
            all(case["expected_runtime"]["disposition"] == "passed" for case in blocked)
        )
        self.assertTrue(
            all(
                blocker["code"] in SEMANTICS.SEMANTIC_BLOCKER_CODES
                for case in blocked
                for blocker in case["blockers"]
            )
        )

    def test_exact_runtime_projection_rejects_a_changed_card_outcome(self) -> None:
        observed = observed_from_cases(self.cases)
        SEMANTICS.verify_observed(self.cases, observed)
        tampered = copy.deepcopy(observed)
        ready_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["status"] == "semantic_case_ready"
        )
        tampered[ready_index]["final_hash"] = "1"
        with self.assertRaisesRegex(ValueError, "runtime outcome changed"):
            SEMANTICS.verify_observed(self.cases, tampered)

    def test_card_specific_mana_probe_requires_every_legal_output(self) -> None:
        observed = observed_from_cases(self.cases)
        mana_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["status"] == "semantic_case_ready"
            and SEMANTICS.expected_mana_abilities(case)
        )
        observed[mana_index]["semantic_probe"]["mana_abilities"][0][
            "all_outputs_replayed"
        ] = False
        with self.assertRaisesRegex(ValueError, "mana ability 0 replay failed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_card_specific_subtype_probe_rejects_missing_printed_subtypes(self) -> None:
        observed = observed_from_cases(self.cases)
        subtype_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["status"] == "semantic_case_ready"
            and SEMANTICS.expected_base_subtypes(case)
        )
        observed[subtype_index]["semantic_probe"]["base_subtypes"] = []
        with self.assertRaisesRegex(ValueError, "printed subtype state changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_conditional_mana_probe_requires_fail_closed_threshold(self) -> None:
        observed = observed_from_cases(self.cases)
        condition_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if any(
                expectation["minimum_matching_permanents"] is not None
                for expectation in SEMANTICS.expected_mana_abilities(case)
            )
        )
        observed[condition_index]["semantic_probe"]["mana_abilities"][0][
            "condition_rejected_below_threshold"
        ] = False
        with self.assertRaisesRegex(ValueError, "condition did not fail closed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_source_bound_player_rule_probe_requires_expiration(self) -> None:
        observed = observed_from_cases(self.cases)
        rule_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if any(
                atom["op"] == "modify_player_rules"
                for atom in case.get("semantic_atoms", [])
            )
        )
        observed[rule_index]["semantic_probe"]["no_maximum_hand_size"][
            "expired_off_battlefield"
        ] = False
        with self.assertRaisesRegex(ValueError, "did not remain source-bound"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_incremental_report_keeps_checkpoint_open(self) -> None:
        observed = observed_from_cases(self.cases)
        report = SEMANTICS.build_report(self.cases, observed)
        self.assertEqual(report["checkpoint"]["status"], "in_progress")
        verified = self.cases["summary"]["semantic_case_ready"]
        self.assertEqual(report["checkpoint"]["semantic_verified"], verified)
        self.assertEqual(report["checkpoint"]["remaining"], 100 - verified)
        self.assertEqual(
            report["measured"]["runtime_smoke_passed"],
            verified + self.cases["summary"]["blocked_semantic_gap"],
        )


if __name__ == "__main__":
    unittest.main()
