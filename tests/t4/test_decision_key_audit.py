from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "audit_t4_decision_keys", ROOT / "tools/audit_t4_decision_keys.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def decision(key: str, *, path: int | None = None, action: str = "a") -> dict:
    return {
        "kind": "declare_attackers",
        "context_id": f"context-{key}-{path}",
        "decision_state_key": key,
        "path_discriminator": path,
        "player_view_hash": "view",
        "canonical_legal_actions": [
            {
                "action_id": action,
                "descriptor_schema_version": 1,
                "descriptor": {"kind": "assign_attacker", "defender": None},
            }
        ],
    }


class DecisionKeyAuditTests(unittest.TestCase):
    def write_replay(self, root: Path, decisions: list[dict]) -> Path:
        path = root / "replay.frsreplay"
        path.write_text(
            json.dumps(
                {
                    "policy_kind": "heuristic-v1",
                    "seed": 7,
                    "decisions": decisions,
                }
            ),
            encoding="utf-8",
        )
        return path

    def test_accepts_repeated_isomorphic_states_and_distinct_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = self.write_replay(
                root,
                [
                    decision("same"),
                    decision("same"),
                    decision("path-1", path=1),
                    decision("path-2", path=2),
                ],
            )
            report = MODULE.build_report([path], "commit", "tree")
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["totals"]["path_bound_decisions"], 2)
            self.assertEqual(report["totals"]["unique_state_keys"], 3)

    def test_rejects_one_key_for_different_semantic_states(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = self.write_replay(
                root,
                [decision("collision", action="a"), decision("collision", action="b")],
            )
            report = MODULE.build_report([path], "commit", "tree")
            self.assertEqual(report["status"], "failed")
            self.assertIn(
                "STATE_KEY_COLLISION",
                {failure["code"] for failure in report["failures"]},
            )

    def test_rejects_different_keys_for_one_semantic_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = self.write_replay(root, [decision("left"), decision("right")])
            report = MODULE.build_report([path], "commit", "tree")
            self.assertEqual(report["status"], "failed")
            self.assertIn(
                "ISOMORPHIC_STATE_KEY_ALIAS",
                {failure["code"] for failure in report["failures"]},
            )


if __name__ == "__main__":
    unittest.main()
