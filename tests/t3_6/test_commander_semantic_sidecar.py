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
                "semantic_case_ready": 34,
                "blocked_semantic_gap": 24,
                "blocked_runtime": 42,
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
        self.assertEqual(ready, 34)

    def test_runtime_smoke_success_does_not_promote_known_semantic_gaps(self) -> None:
        blocked = [
            case
            for case in self.cases["cases"]
            if case["status"] == "blocked_semantic_gap"
        ]
        self.assertEqual(len(blocked), 24)
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

    def test_incremental_report_keeps_checkpoint_open(self) -> None:
        observed = observed_from_cases(self.cases)
        report = SEMANTICS.build_report(self.cases, observed)
        self.assertEqual(report["checkpoint"]["status"], "in_progress")
        self.assertEqual(report["checkpoint"]["semantic_verified"], 34)
        self.assertEqual(report["checkpoint"]["remaining"], 66)
        self.assertEqual(report["measured"]["runtime_smoke_passed"], 58)


if __name__ == "__main__":
    unittest.main()
