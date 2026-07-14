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
                "equipment": SEMANTICS.expected_equipment_probe(case),
                "sacrifice_counter": SEMANTICS.expected_sacrifice_counter_probe(case),
                "temporary_protection": (
                    SEMANTICS.expected_temporary_protection_probe(case)
                ),
                "commander_alternate_cost": (
                    SEMANTICS.expected_commander_alternate_cost_probe(case)
                ),
                "flashback_looting": SEMANTICS.expected_flashback_looting_probe(case),
                "split_second": SEMANTICS.expected_split_second_probe(case),
                "overload": SEMANTICS.expected_overload_probe(case),
                "evoke": SEMANTICS.expected_evoke_probe(case),
                "boros_charm": SEMANTICS.expected_boros_charm_probe(case),
                "reconnaissance_mission": (
                    SEMANTICS.expected_reconnaissance_mission_probe(case)
                ),
                "smothering_tithe": SEMANTICS.expected_smothering_tithe_probe(case),
                "purphoros": SEMANTICS.expected_purphoros_probe(case),
                "bala_ged_modal_dfc": (
                    SEMANTICS.expected_bala_ged_modal_dfc_probe(case)
                ),
                "noncreature_counter": (
                    SEMANTICS.expected_noncreature_counter_probe(case)
                ),
                "temporary_creature_protection": (
                    SEMANTICS.expected_temporary_creature_protection_probe(case)
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
                "semantic_case_ready": 79,
                "blocked_semantic_gap": 21,
                "blocked_runtime": 0,
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

    def test_equipment_probe_requires_live_protection_and_reattachment(self) -> None:
        observed = observed_from_cases(self.cases)
        equipment_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Swiftfoot Boots"
        )
        observed[equipment_index]["semantic_probe"]["equipment"]["first_attachment"][
            "opponent_targetable"
        ] = True
        with self.assertRaisesRegex(ValueError, "equipment semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_equipment_attack_probe_requires_basic_land_move_and_tap(self) -> None:
        observed = observed_from_cases(self.cases)
        sword_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Sword of the Animist"
        )
        observed[sword_index]["semantic_probe"]["equipment"][
            "attached_attack_trigger"
        ]["basic_land_tapped"] = False
        with self.assertRaisesRegex(ValueError, "equipment semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_sacrifice_counter_probe_requires_real_cost_and_source_counter(self) -> None:
        observed = observed_from_cases(self.cases)
        feeder_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Carrion Feeder"
        )
        observed[feeder_index]["semantic_probe"]["sacrifice_counter"][
            "source_bound_counter_action"
        ] = False
        with self.assertRaisesRegex(ValueError, "sacrifice-counter semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_temporary_protection_probe_requires_cleanup_expiration(self) -> None:
        observed = observed_from_cases(self.cases)
        intervention_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Heroic Intervention"
        )
        observed[intervention_index]["semantic_probe"]["temporary_protection"][
            "restrictions_removed_at_cleanup"
        ] = False
        with self.assertRaisesRegex(
            ValueError, "temporary-protection semantic probe changed"
        ):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_commander_alternate_cost_requires_a_controlled_battlefield_commander(
        self,
    ) -> None:
        observed = observed_from_cases(self.cases)
        guardianship_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Fierce Guardianship"
        )
        observed[guardianship_index]["semantic_probe"]["commander_alternate_cost"][
            "available_without_controlled_battlefield_commander"
        ] = True
        with self.assertRaisesRegex(ValueError, "commander alternate-cost probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_fierce_guardianship_rejects_creature_stack_targets(self) -> None:
        observed = observed_from_cases(self.cases)
        guardianship_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Fierce Guardianship"
        )
        observed[guardianship_index]["semantic_probe"]["noncreature_counter"][
            "creature_stack_target_rejected"
        ] = False
        with self.assertRaisesRegex(ValueError, "noncreature-counter semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_flawless_maneuver_requires_cleanup_expiration(self) -> None:
        observed = observed_from_cases(self.cases)
        maneuver_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Flawless Maneuver"
        )
        observed[maneuver_index]["semantic_probe"]["temporary_creature_protection"][
            "restrictions_removed_at_cleanup"
        ] = False
        with self.assertRaisesRegex(
            ValueError, "temporary creature-protection probe changed"
        ):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_faithless_looting_requires_flashback_exile_resolution(self) -> None:
        observed = observed_from_cases(self.cases)
        looting_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Faithless Looting"
        )
        observed[looting_index]["semantic_probe"]["flashback_looting"][
            "source_exiled_on_resolution"
        ] = False
        with self.assertRaisesRegex(ValueError, "flashback-looting semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_faithless_looting_choices_fail_before_mutation(self) -> None:
        observed = observed_from_cases(self.cases)
        looting_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Faithless Looting"
        )
        observed[looting_index]["semantic_probe"]["flashback_looting"][
            "duplicate_choice_rejected_before_mutation"
        ] = False
        with self.assertRaisesRegex(ValueError, "flashback-looting semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_krosan_grip_split_second_blocks_non_mana_responses(self) -> None:
        observed = observed_from_cases(self.cases)
        grip_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Krosan Grip"
        )
        observed[grip_index]["semantic_probe"]["split_second"][
            "responder_non_mana_ability_rejected_before_mutation"
        ] = False
        with self.assertRaisesRegex(ValueError, "split-second semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_krosan_grip_split_second_allows_mana_and_then_expires(self) -> None:
        observed = observed_from_cases(self.cases)
        grip_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Krosan Grip"
        )
        observed[grip_index]["semantic_probe"]["split_second"][
            "ordinary_cast_available_after_resolution"
        ] = False
        with self.assertRaisesRegex(ValueError, "split-second semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_cyclonic_rift_ordinary_mode_requires_one_target(self) -> None:
        observed = observed_from_cases(self.cases)
        rift_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Cyclonic Rift"
        )
        observed[rift_index]["semantic_probe"]["overload"]["ordinary"][
            "cast_without_target_rejected_before_mutation"
        ] = False
        with self.assertRaisesRegex(ValueError, "overload semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_cyclonic_rift_overload_moves_only_opponent_nonlands(self) -> None:
        observed = observed_from_cases(self.cases)
        rift_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Cyclonic Rift"
        )
        observed[rift_index]["semantic_probe"]["overload"]["overload"][
            "friendly_nonland_unchanged"
        ] = False
        with self.assertRaisesRegex(ValueError, "overload semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_mulldrifter_normal_cast_keeps_the_source(self) -> None:
        observed = observed_from_cases(self.cases)
        mulldrifter_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Mulldrifter"
        )
        observed[mulldrifter_index]["semantic_probe"]["evoke"]["normal"][
            "source_remained_battlefield_after_draw"
        ] = False
        with self.assertRaisesRegex(ValueError, "evoke semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_mulldrifter_evoke_draws_before_source_sacrifice(self) -> None:
        observed = observed_from_cases(self.cases)
        mulldrifter_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Mulldrifter"
        )
        observed[mulldrifter_index]["semantic_probe"]["evoke"]["evoke"][
            "draw_then_sacrificed"
        ] = False
        with self.assertRaisesRegex(ValueError, "evoke semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_boros_charm_requires_one_valid_announced_mode(self) -> None:
        observed = observed_from_cases(self.cases)
        charm_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Boros Charm"
        )
        observed[charm_index]["semantic_probe"]["boros_charm"]["contract"][
            "no_mode_rejected_before_mutation"
        ] = False
        with self.assertRaisesRegex(ValueError, "Boros Charm semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_boros_charm_damage_mode_covers_player_and_planeswalker(self) -> None:
        observed = observed_from_cases(self.cases)
        charm_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Boros Charm"
        )
        observed[charm_index]["semantic_probe"]["boros_charm"]["damage"][
            "planeswalker_loyalty_after_damage"
        ] = 7
        with self.assertRaisesRegex(ValueError, "Boros Charm semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_boros_charm_indestructible_covers_controlled_permanents(self) -> None:
        observed = observed_from_cases(self.cases)
        charm_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Boros Charm"
        )
        observed[charm_index]["semantic_probe"]["boros_charm"]["indestructible"][
            "protected_artifact_survived_destroy"
        ] = False
        with self.assertRaisesRegex(ValueError, "Boros Charm semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_boros_charm_double_strike_targets_any_creature_and_expires(self) -> None:
        observed = observed_from_cases(self.cases)
        charm_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Boros Charm"
        )
        observed[charm_index]["semantic_probe"]["boros_charm"]["cleanup"][
            "double_strike_expired"
        ] = False
        with self.assertRaisesRegex(ValueError, "Boros Charm semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_reconnaissance_mission_cycling_is_paid_discard_then_draw(self) -> None:
        observed = observed_from_cases(self.cases)
        mission_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Reconnaissance Mission"
        )
        observed[mission_index]["semantic_probe"]["reconnaissance_mission"][
            "cycling"
        ]["payment_consumed"] = False
        with self.assertRaisesRegex(
            ValueError, "Reconnaissance Mission semantic probe changed"
        ):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_reconnaissance_mission_queues_only_the_typed_combat_trigger(self) -> None:
        observed = observed_from_cases(self.cases)
        mission_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Reconnaissance Mission"
        )
        observed[mission_index]["semantic_probe"]["reconnaissance_mission"][
            "combat_trigger"
        ]["pending_trigger_exact"] = False
        with self.assertRaisesRegex(
            ValueError, "Reconnaissance Mission semantic probe changed"
        ):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_reconnaissance_mission_optional_draw_supports_decline_and_accept(self) -> None:
        observed = observed_from_cases(self.cases)
        mission_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Reconnaissance Mission"
        )
        observed[mission_index]["semantic_probe"]["reconnaissance_mission"][
            "optional_draw"
        ]["decline_emits_no_actions_or_draw"] = False
        with self.assertRaisesRegex(
            ValueError, "Reconnaissance Mission semantic probe changed"
        ):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_smothering_tithe_queues_one_trigger_per_opponent_card_drawn(self) -> None:
        observed = observed_from_cases(self.cases)
        tithe_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Smothering Tithe"
        )
        observed[tithe_index]["semantic_probe"]["smothering_tithe"]["event_boundary"][
            "one_trigger_per_opponent_card_drawn"
        ] = False
        with self.assertRaisesRegex(ValueError, "Smothering Tithe semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_smothering_tithe_ignores_controller_and_failed_draws(self) -> None:
        observed = observed_from_cases(self.cases)
        tithe_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Smothering Tithe"
        )
        boundary = observed[tithe_index]["semantic_probe"]["smothering_tithe"][
            "event_boundary"
        ]
        boundary["controller_draw_queued_no_trigger"] = False
        boundary["empty_library_queued_no_trigger"] = False
        with self.assertRaisesRegex(ValueError, "Smothering Tithe semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_smothering_tithe_decline_creates_exactly_one_treasure(self) -> None:
        observed = observed_from_cases(self.cases)
        tithe_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Smothering Tithe"
        )
        observed[tithe_index]["semantic_probe"]["smothering_tithe"]["decline"][
            "exactly_one_treasure_created"
        ] = False
        with self.assertRaisesRegex(ValueError, "Smothering Tithe semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_smothering_tithe_exact_payment_consumes_payer_mana_and_suppresses_token(
        self,
    ) -> None:
        observed = observed_from_cases(self.cases)
        tithe_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Smothering Tithe"
        )
        payment = observed[tithe_index]["semantic_probe"]["smothering_tithe"]["pay"]
        payment["payer_mana_consumed"] = False
        payment["treasure_suppressed"] = False
        with self.assertRaisesRegex(ValueError, "Smothering Tithe semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_purphoros_devotion_toggles_creature_type_at_one_five_one(self) -> None:
        observed = observed_from_cases(self.cases)
        purphoros_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Purphoros, God of the Forge"
        )
        observed[purphoros_index]["semantic_probe"]["purphoros"]["devotion"][
            "source_creature_at_five"
        ] = False
        with self.assertRaisesRegex(ValueError, "Purphoros semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_purphoros_trigger_excludes_self_opponents_and_noncreatures(self) -> None:
        observed = observed_from_cases(self.cases)
        purphoros_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Purphoros, God of the Forge"
        )
        trigger = observed[purphoros_index]["semantic_probe"]["purphoros"][
            "creature_enter_trigger"
        ]
        trigger["self_entry_excluded"] = False
        trigger["opponent_creature_excluded"] = False
        with self.assertRaisesRegex(ValueError, "Purphoros semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_purphoros_deals_untargeted_damage_to_each_opponent(self) -> None:
        observed = observed_from_cases(self.cases)
        purphoros_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Purphoros, God of the Forge"
        )
        damage = observed[purphoros_index]["semantic_probe"]["purphoros"][
            "opponent_damage"
        ]
        damage["untargeted_contract"] = False
        damage["second_opponent_life"] = 20
        with self.assertRaisesRegex(ValueError, "Purphoros semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_purphoros_team_pump_is_paid_scoped_and_expires(self) -> None:
        observed = observed_from_cases(self.cases)
        purphoros_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["card_name"] == "Purphoros, God of the Forge"
        )
        pump = observed[purphoros_index]["semantic_probe"]["purphoros"]["team_pump"]
        pump["payment_consumed"] = False
        pump["pump_expired_at_cleanup"] = False
        with self.assertRaisesRegex(ValueError, "Purphoros semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_bala_ged_front_face_requires_owned_graveyard_target(self) -> None:
        observed = observed_from_cases(self.cases)
        bala_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["scenario_id"] == "T3.6-086"
        )
        front = observed[bala_index]["semantic_probe"]["bala_ged_modal_dfc"][
            "front_face"
        ]
        front["wrong_zone_rejected_before_mutation"] = False
        front["opponent_card_rejected_before_mutation"] = False
        with self.assertRaisesRegex(ValueError, "modal DFC semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_bala_ged_back_face_enters_tapped_and_produces_green(self) -> None:
        observed = observed_from_cases(self.cases)
        bala_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["scenario_id"] == "T3.6-086"
        )
        back = observed[bala_index]["semantic_probe"]["bala_ged_modal_dfc"][
            "back_face"
        ]
        back["entered_battlefield_tapped"] = False
        back["added_exactly_green"] = False
        with self.assertRaisesRegex(ValueError, "modal DFC semantic probe changed"):
            SEMANTICS.verify_observed(self.cases, observed)

    def test_bala_ged_faces_remain_isolated(self) -> None:
        observed = observed_from_cases(self.cases)
        bala_index = next(
            index
            for index, case in enumerate(self.cases["cases"])
            if case["scenario_id"] == "T3.6-086"
        )
        isolation = observed[bala_index]["semantic_probe"]["bala_ged_modal_dfc"][
            "face_isolation"
        ]
        isolation["back_rejects_front_target_before_mutation"] = False
        isolation["front_not_playable_as_land"] = False
        with self.assertRaisesRegex(ValueError, "modal DFC semantic probe changed"):
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
