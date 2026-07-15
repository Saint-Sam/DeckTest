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


def decision(
    key: str,
    *,
    path: int | None = None,
    action: str = "a",
    episode: str | None = None,
) -> dict:
    context_id = f"context-{key}-{path}"
    return {
        "kind": "declare_attackers",
        "context_id": context_id,
        "decision_state_key": key,
        "normalized_benchmark_key": f"normalized-{key}",
        "normalized_player_view_hash": "normalized-view",
        "normalized_legal_action_ids": [f"normalized-{action}"],
        "benchmark_normalization_complete": True,
        "path_discriminator": path,
        "player_view_hash": "view",
        "legal_actions": 1,
        "decision_episode_id": episode or f"episode-{key}-{path}-{action}",
        "root_context_id": context_id,
        "parent_context_id": None,
        "path_depth": 0,
        "is_forced": True,
        "is_strategic_root": False,
        "is_terminal_subchoice": True,
        "final_concrete_action_id": action,
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

    def test_accepts_repeated_recorded_signatures_and_distinct_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = self.write_replay(
                root,
                [
                    decision("same", episode="same-1"),
                    decision("same", episode="same-2"),
                    decision("path-1", path=1),
                    decision("path-2", path=2),
                ],
            )
            report = MODULE.build_report([path], "commit", "tree")
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["recorded_key_signature_consistency"], "passed")
            self.assertEqual(report["normalized_key_signature_consistency"], "passed")
            self.assertEqual(
                report["near_state_dedup_audit"],
                "requires_runtime_isomorphism_fixture_artifact",
            )
            self.assertEqual(report["totals"]["path_bound_decisions"], 2)
            self.assertEqual(report["totals"]["unique_state_keys"], 3)
            self.assertEqual(report["totals"]["decision_episodes"], 4)

    def test_accepts_one_linked_strategic_episode(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            first = decision("root", action="root-action", episode="linked")
            first.update(
                {
                    "legal_actions": 2,
                    "is_forced": False,
                    "is_strategic_root": True,
                    "is_terminal_subchoice": False,
                    "final_concrete_action_id": "complete-action",
                }
            )
            second = decision("child", action="child-action", episode="linked")
            second.update(
                {
                    "root_context_id": first["context_id"],
                    "parent_context_id": first["context_id"],
                    "path_depth": 1,
                    "final_concrete_action_id": "complete-action",
                }
            )
            path = self.write_replay(root, [first, second])
            report = MODULE.build_report([path], "commit", "tree")
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["totals"]["decision_episodes"], 1)
            self.assertEqual(report["totals"]["strategic_decision_episodes"], 1)

    def test_accepts_exact_product_runtime_isomorphism_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            replay = self.write_replay(root, [decision("state")])
            fixture = root / "runtime-isomorphism.json"
            fixture.write_text(
                json.dumps(
                    {
                        "status": "passed",
                        "product_commit": "commit",
                        "product_tree": "tree",
                        "checks": {
                            "exact_replay_ids_unchanged": True,
                            "object_allocation_order_isomorphic": True,
                            "equivalent_mana_source_creation_order_isomorphic": True,
                            "ability_registration_order_isomorphic": True,
                            "equivalent_zone_membership_runtime_handles_isomorphic": True,
                            "exact_runtime_handles_remain_distinct": True,
                            "hierarchical_paths_remain_distinct": True,
                            "unequal_semantics_remain_distinct": True,
                            "visible_stack_semantics_remain_distinct": True,
                            "normalization_complete": True,
                        },
                    }
                ),
                encoding="utf-8",
            )
            report = MODULE.build_report([replay], "commit", "tree", fixture)
            self.assertEqual(report["status"], "passed")
            self.assertEqual(
                report["near_state_dedup_audit"], "passed_runtime_isomorphism"
            )

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
