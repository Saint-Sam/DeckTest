#!/usr/bin/env python3
"""Focused tests for exact T4 long-game diagnostics."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "summarize_t4_long_games", ROOT / "tools/summarize_t4_long_games.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def replay(policy: str, turns: int, repeated_hashes: int = 0) -> dict:
    return {
        "policy_kind": policy,
        "seed": 7,
        "policy_seed": 11,
        "max_turns": 500,
        "decisions": [{"decision_state_key": "a"}, {"decision_state_key": "a"}],
        "expected": {
            "turns": turns,
            "progress": {
                "termination_reason": "winner",
                "turn_cap_reached": False,
                "state_observations": 10,
                "repeated_full_state_hashes": repeated_hashes,
                "repeated_decision_state_keys": 1,
                "no_progress_rounds": 0,
                "maximum_consecutive_no_progress_rounds": 0,
                "eliminations": [{"seat": 1, "turn": turns}],
                "rounds": [
                    {
                        "table_damage_to_players": 4,
                        "life_total_movement": 4,
                        "casts": 1,
                        "meaningful_actions": 5,
                        "pass_only_priority_cycles": 8,
                        "active_players_with_progress": 4,
                        "eliminations": 1,
                    }
                ],
            },
        },
    }


class LongGameDiagnosticsTests(unittest.TestCase):
    def test_report_aggregates_exact_progress_and_rates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            paths = []
            for index, payload in enumerate((replay("random", 40), replay("heuristic", 80, 2))):
                path = Path(temporary) / f"run-{index}.json"
                path.write_text(json.dumps(payload), encoding="utf-8")
                paths.append(path)
            report = MODULE.build_report(paths, "a" * 40, "b" * 40)
        self.assertEqual(report["aggregate"]["turn_p50"], 40)
        self.assertEqual(report["aggregate"]["turn_p95"], 80)
        self.assertEqual(report["aggregate"]["repeated_full_state_hash_rate_ppm"], 100_000)
        self.assertEqual(report["aggregate"]["repeated_decision_state_key_rate_ppm"], 500_000)
        self.assertFalse(report["promotion_eligible"])

    def test_missing_progress_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "stale.json"
            path.write_text(json.dumps({"expected": {}, "decisions": []}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "progress diagnostics"):
                MODULE.summarize_run(path, MODULE.load(path))


if __name__ == "__main__":
    unittest.main()
