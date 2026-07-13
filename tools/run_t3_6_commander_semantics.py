#!/usr/bin/env python3
"""Run the fail-closed T3.6 Commander semantic sidecar locally."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path, PurePosixPath
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
CANDIDATES_PATH = ROOT / "assets/t3_6_commander_semantic_candidates.json"
CASES_PATH = ROOT / "tests/t3_6/commander_semantic_cases.json"
PROBE_SOURCE = ROOT / "tests/t3_6/runtime_probe.rs"
PROBE_NAME = "forge-t3-6-runtime-probe"

ATOM_CAPABILITIES = {
    "play_land": "land_play",
    "resolve_permanent": "permanent_spell",
    "activate_mana": "mana_ability",
    "activate_ability": "activated_ability",
    "gain_life": "gain_life",
    "lose_life": "lose_life",
    "draw_cards": "draw_cards",
    "scry": "scry",
    "shuffle_library": "shuffle_library",
    "destroy_permanent": "destroy_permanent",
    "exile_object": "exile_object",
    "counter_stack_entry": "counter_stack_entry",
    "move_zone": "move_zone",
    "create_token": "create_token",
    "search_library": "search_library",
    "tap_object": "tap_object",
    "discard_cards": "discard_cards",
    "modify_characteristics": "modify_characteristics",
    "modify_player_rules": "modify_player_rules",
    "reduce_spell_cost": "reduce_spell_cost",
    "attach_object": "attach_object",
    "targeting_restriction": "targeting_restriction",
    "indestructible": "indestructible",
    "add_counters": "add_counters",
    "combat_restriction": "combat_restriction",
    "alternate_cost": "alternate_cost",
    "flashback": "alternate_cost",
    "overload_cost": "alternate_cost",
    "evoke_cost": "alternate_cost",
    "overload": "overload",
    "sacrifice_permanent": "sacrifice_permanent",
    "split_second": "split_second",
}
SEMANTIC_BLOCKER_CODES = {
    "COPY_TRIGGER_EVENT_MISSING",
    "GENERAL_SUBTYPE_STATE_MISSING",
    "MANA_CHOICE_PATHS_NOT_CARD_SPECIFICALLY_REPLAYED",
    "REGENERATION_PROHIBITION_MISSING",
    "REVEAL_KNOWLEDGE_EVENT_MISSING",
    "TOKEN_SUBTYPE_STATE_MISSING",
}
RUNTIME_FIELDS = (
    "disposition",
    "capabilities",
    "effect_actions",
    "production_actions",
    "final_life_totals",
    "destination",
    "final_hash",
)


def json_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain one JSON object")
    return value


def verify_payload_hash(document: dict[str, Any]) -> bool:
    declared = document.get("payload_sha256")
    payload = copy.deepcopy(document)
    payload.pop("payload_sha256", None)
    return isinstance(declared, str) and declared == sha256_bytes(json_bytes(payload))


def translated_relative(selected: dict[str, Any]) -> str:
    source = PurePosixPath(selected["legacy_source_path"])
    if source.is_absolute() or ".." in source.parts or source.suffix != ".txt":
        raise ValueError(f"invalid frozen source path {source}")
    return source.with_suffix(".frs").as_posix()


def translated_oracle(source: str) -> str:
    for line in source.splitlines():
        stripped = line.strip()
        if stripped.startswith("oracle: "):
            value = json.loads(stripped.removeprefix("oracle: "))
            if isinstance(value, str):
                return value
    raise ValueError("translated definition has no string oracle field")


def validate_expected_runtime(case: dict[str, Any]) -> None:
    expected = case.get("expected_runtime")
    if not isinstance(expected, dict):
        raise ValueError(f"{case['scenario_id']}: expected_runtime must be an object")
    status = case["status"]
    if status in {"semantic_case_ready", "blocked_semantic_gap"}:
        if expected.get("disposition") != "passed":
            raise ValueError(f"{case['scenario_id']}: runtime-ready case must expect a pass")
        capabilities = expected.get("capabilities")
        if not isinstance(capabilities, list) or not all(
            isinstance(value, str) for value in capabilities
        ):
            raise ValueError(f"{case['scenario_id']}: invalid capability list")
        if not isinstance(expected.get("effect_actions"), int):
            raise ValueError(f"{case['scenario_id']}: invalid effect action count")
        if not isinstance(expected.get("production_actions"), int) or expected["production_actions"] <= 0:
            raise ValueError(f"{case['scenario_id']}: invalid production action count")
        life = expected.get("final_life_totals")
        if not isinstance(life, list) or len(life) != 2 or not all(isinstance(v, int) for v in life):
            raise ValueError(f"{case['scenario_id']}: invalid final life totals")
        if expected.get("destination") not in {
            "battlefield",
            "exile",
            "owner_graveyard",
        }:
            raise ValueError(f"{case['scenario_id']}: invalid card lifecycle destination")
        final_hash = expected.get("final_hash")
        if not isinstance(final_hash, str) or not final_hash.isdigit() or int(final_hash) == 0:
            raise ValueError(f"{case['scenario_id']}: invalid deterministic hash")
    elif status == "blocked_runtime":
        if expected.get("disposition") != "unsupported_setup":
            raise ValueError(f"{case['scenario_id']}: runtime blocker must remain unsupported")
        if not isinstance(expected.get("code"), str) or not expected["code"].startswith("unsupported_"):
            raise ValueError(f"{case['scenario_id']}: invalid runtime reason code")
        detail = expected.get("detail")
        if not isinstance(detail, str) or not detail:
            raise ValueError(f"{case['scenario_id']}: runtime blocker needs detail")
        if expected.get("detail_sha256") != sha256_bytes(detail.encode()):
            raise ValueError(f"{case['scenario_id']}: runtime detail hash mismatch")


def validate_manifest(translated_root: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    candidates = load_json(CANDIDATES_PATH)
    cases = load_json(CASES_PATH)
    if not verify_payload_hash(candidates):
        raise ValueError("candidate manifest payload hash is invalid")
    if not verify_payload_hash(cases):
        raise ValueError("semantic case manifest payload hash is invalid")
    if cases.get("candidate_payload_sha256") != candidates.get("payload_sha256"):
        raise ValueError("semantic cases are not bound to the current frozen candidates")
    selected = candidates.get("selected")
    records = cases.get("cases")
    if not isinstance(selected, list) or len(selected) != 100:
        raise ValueError("candidate freeze must contain exactly 100 selected identities")
    if not isinstance(records, list) or len(records) != 100:
        raise ValueError("semantic manifest must account for exactly 100 identities")

    status_counts: Counter[str] = Counter()
    seen_ids: set[str] = set()
    for selected_item, case in zip(selected, records):
        if not isinstance(case, dict):
            raise ValueError("semantic case entry must be an object")
        rank = selected_item["freeze_rank"]
        expected_id = f"T3.6-{rank:03d}"
        if case.get("scenario_id") != expected_id:
            raise ValueError(f"freeze rank {rank}: expected scenario id {expected_id}")
        for case_key, selected_key in (
            ("freeze_rank", "freeze_rank"),
            ("oracle_id", "oracle_id"),
            ("card_name", "requested_name"),
            ("stratum", "stratum"),
            ("legacy_source_path", "legacy_source_path"),
            ("legacy_source_sha256", "legacy_source_sha256"),
        ):
            if case.get(case_key) != selected_item.get(selected_key):
                raise ValueError(f"{expected_id}: frozen field {case_key} mismatch")
        oracle_id = case["oracle_id"]
        if oracle_id in seen_ids:
            raise ValueError(f"{expected_id}: duplicate Oracle identity")
        seen_ids.add(oracle_id)

        relative = translated_relative(selected_item)
        if case.get("translated_path") != relative:
            raise ValueError(f"{expected_id}: translated path mismatch")
        source_path = translated_root / relative
        source = source_path.read_text(encoding="utf-8")
        if case.get("translated_source_sha256") != sha256_file(source_path):
            raise ValueError(f"{expected_id}: translated source hash mismatch")
        if case.get("oracle_text") != translated_oracle(source):
            raise ValueError(f"{expected_id}: retained Oracle text mismatch")

        status = case.get("status")
        if status not in {"semantic_case_ready", "blocked_semantic_gap", "blocked_runtime"}:
            raise ValueError(f"{expected_id}: invalid status {status}")
        status_counts[status] += 1
        validate_expected_runtime(case)
        if status == "semantic_case_ready":
            behavior = case.get("expected_behavior")
            atoms = case.get("semantic_atoms")
            if not isinstance(behavior, str) or not behavior:
                raise ValueError(f"{expected_id}: missing card-specific expected behavior")
            if not isinstance(atoms, list) or not atoms:
                raise ValueError(f"{expected_id}: missing semantic atoms")
            derived = []
            for semantic_atom in atoms:
                if not isinstance(semantic_atom, dict) or set(semantic_atom).issubset({"op"}):
                    raise ValueError(f"{expected_id}: semantic atom needs typed arguments")
                operation = semantic_atom.get("op")
                if operation not in ATOM_CAPABILITIES:
                    raise ValueError(f"{expected_id}: unknown semantic atom {operation}")
                derived.append(ATOM_CAPABILITIES[operation])
            if derived != case["expected_runtime"]["capabilities"]:
                raise ValueError(f"{expected_id}: semantic atoms disagree with runtime capabilities")
        else:
            blockers = case.get("blockers")
            if not isinstance(blockers, list) or not blockers:
                raise ValueError(f"{expected_id}: blocked case needs reason-coded blockers")
            for blocker in blockers:
                if not isinstance(blocker, dict) or not isinstance(blocker.get("detail"), str):
                    raise ValueError(f"{expected_id}: invalid blocker record")
                code = blocker.get("code")
                if status == "blocked_semantic_gap" and code not in SEMANTIC_BLOCKER_CODES:
                    raise ValueError(f"{expected_id}: unknown semantic blocker {code}")
                if status == "blocked_runtime" and code != f"T3_5_{case['expected_runtime']['code'].upper()}":
                    raise ValueError(f"{expected_id}: runtime blocker code is not stable")

    declared_counts = cases.get("summary")
    expected_counts = {
        "candidate_count": 100,
        "semantic_case_ready": status_counts["semantic_case_ready"],
        "blocked_semantic_gap": status_counts["blocked_semantic_gap"],
        "blocked_runtime": status_counts["blocked_runtime"],
    }
    if declared_counts != expected_counts:
        raise ValueError(f"semantic summary mismatch: {declared_counts} != {expected_counts}")
    return candidates, cases


def build_probe(cargo_target_dir: Path) -> Path:
    cargo_target_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="forge-t3-6-probe-") as temp:
        manifest = Path(temp) / "Cargo.toml"
        manifest.write_text(
            "\n".join(
                [
                    "[package]",
                    f'name = "{PROBE_NAME}"',
                    'version = "0.0.0"',
                    'edition = "2021"',
                    "publish = false",
                    "",
                    "[dependencies]",
                    f"forge-cardc = {{ path = {json.dumps(str(ROOT / 'crates/forge-cardc'))} }}",
                    f"forge-cards = {{ path = {json.dumps(str(ROOT / 'crates/forge-cards'))} }}",
                    f"forge-core = {{ path = {json.dumps(str(ROOT / 'crates/forge-core'))} }}",
                    f"forge-testkit = {{ path = {json.dumps(str(ROOT / 'crates/forge-testkit'))} }}",
                    'serde_json = "=1.0.150"',
                    "",
                    "[[bin]]",
                    f'name = "{PROBE_NAME}"',
                    f"path = {json.dumps(str(PROBE_SOURCE))}",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        command = [
            os.environ.get("CARGO", "cargo"),
            "build",
            "--offline",
            "--quiet",
            "--manifest-path",
            str(manifest),
            "--target-dir",
            str(cargo_target_dir),
        ]
        result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)
        if result.returncode != 0:
            raise RuntimeError(f"probe build failed\n{result.stdout}\n{result.stderr}")
    suffix = ".exe" if os.name == "nt" else ""
    probe = cargo_target_dir / "debug" / f"{PROBE_NAME}{suffix}"
    if not probe.is_file():
        raise RuntimeError(f"probe build did not create {probe}")
    return probe


def run_probe(probe: Path, translated_root: Path, cases: dict[str, Any]) -> list[dict[str, Any]]:
    paths = [translated_root / case["translated_path"] for case in cases["cases"]]
    result = subprocess.run(
        [str(probe), *(str(path) for path in paths)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    entries = [json.loads(line) for line in result.stdout.splitlines() if line]
    if len(entries) != 100:
        raise RuntimeError(
            f"runtime probe returned {len(entries)} entries, expected 100; stderr={result.stderr.strip()}"
        )
    if result.returncode != 0:
        failures = [entry for entry in entries if entry.get("disposition") == "failed"]
        raise RuntimeError(f"runtime probe reported production failures: {failures}")
    for entry, case in zip(entries, cases["cases"]):
        entry["path"] = case["translated_path"]
    return entries


def verify_observed(cases: dict[str, Any], observed: list[dict[str, Any]]) -> None:
    for case, actual in zip(cases["cases"], observed):
        scenario_id = case["scenario_id"]
        if actual.get("oracle_id") != case["oracle_id"]:
            raise ValueError(f"{scenario_id}: probe Oracle identity mismatch")
        if actual.get("path") != case["translated_path"]:
            raise ValueError(f"{scenario_id}: probe path mismatch")
        expected = case["expected_runtime"]
        if expected["disposition"] == "passed":
            actual_projection = {field: actual.get(field) for field in RUNTIME_FIELDS}
            if actual_projection != expected:
                raise ValueError(
                    f"{scenario_id}: runtime outcome changed\nexpected={expected}\nactual={actual_projection}"
                )
        else:
            actual_projection = {
                "disposition": actual.get("disposition"),
                "code": actual.get("code"),
                "detail": actual.get("detail"),
                "detail_sha256": sha256_bytes(str(actual.get("detail", "")).encode()),
            }
            if actual_projection != expected:
                raise ValueError(
                    f"{scenario_id}: runtime blocker changed\nexpected={expected}\nactual={actual_projection}"
                )
        verify_semantic_probe(case, actual)


def expected_mana_abilities(case: dict[str, Any]) -> list[dict[str, Any]]:
    expected: list[dict[str, Any]] = []
    for atom in case.get("semantic_atoms", []):
        if atom.get("op") != "activate_mana":
            continue
        groups = atom.get("abilities")
        if groups is None:
            groups = [atom]
        if not isinstance(groups, list):
            raise ValueError(f"{case['scenario_id']}: mana abilities must be a list")
        for group in groups:
            outputs = group.get("legal_outputs") if isinstance(group, dict) else None
            damage = group.get("damage_to_controller", 0) if isinstance(group, dict) else None
            if (
                not isinstance(outputs, list)
                or not outputs
                or not all(isinstance(value, str) for value in outputs)
                or not isinstance(damage, int)
                or damage < 0
            ):
                raise ValueError(
                    f"{case['scenario_id']}: invalid card-specific mana replay expectation"
                )
            expected.append(
                {
                    "legal_outputs": outputs,
                    "damage_to_controller": damage,
                    "minimum_matching_permanents": group.get(
                        "minimum_matching_permanents"
                    ),
                }
            )
    return expected


def expected_base_subtypes(case: dict[str, Any]) -> list[str] | None:
    for atom in case.get("semantic_atoms", []):
        if atom.get("op") not in {"play_land", "resolve_permanent"}:
            continue
        subtypes = atom.get("subtypes")
        if subtypes is None:
            continue
        if not isinstance(subtypes, list) or not all(
            isinstance(value, str) and value for value in subtypes
        ):
            raise ValueError(f"{case['scenario_id']}: invalid subtype expectation")
        return sorted(value.casefold() for value in subtypes)
    return None


def expected_equipment_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    equip_atoms = [
        atom
        for atom in atoms
        if atom.get("op") == "activate_ability" and atom.get("ability") == "equip"
    ]
    if not equip_atoms:
        return None
    if len(equip_atoms) != 1:
        raise ValueError(f"{case['scenario_id']}: equipment case needs exactly one equip atom")
    equip = equip_atoms[0]
    generic_cost = equip.get("generic_mana_cost")
    if (
        not isinstance(generic_cost, int)
        or generic_cost < 0
        or equip.get("timing") != "sorcery"
        or equip.get("target") != "creature_you_control"
    ):
        raise ValueError(f"{case['scenario_id']}: invalid equip activation expectation")

    modifiers = [
        atom
        for atom in atoms
        if atom.get("op") == "modify_characteristics"
        and atom.get("subject") == "equipped_creature"
    ]
    attachments = [atom for atom in atoms if atom.get("op") == "attach_object"]
    restrictions = [atom for atom in atoms if atom.get("op") == "targeting_restriction"]
    if len(modifiers) != 1 or attachments != [
        {"attachment": "source", "op": "attach_object", "target": "chosen_creature"}
    ]:
        raise ValueError(f"{case['scenario_id']}: incomplete equipment attachment expectation")
    if len(restrictions) > 1:
        raise ValueError(f"{case['scenario_id']}: equipment has multiple protection expectations")
    modifier = modifiers[0]
    power_delta = modifier.get("power_delta", 0)
    toughness_delta = modifier.get("toughness_delta", 0)
    granted = modifier.get("grant_keywords", [])
    if (
        not isinstance(power_delta, int)
        or not isinstance(toughness_delta, int)
        or not isinstance(granted, list)
        or not all(isinstance(value, str) for value in granted)
        or modifier.get("duration") != "while_source_on_battlefield"
    ):
        raise ValueError(f"{case['scenario_id']}: invalid attached characteristic expectation")
    restriction = None
    if restrictions:
        restriction_atom = restrictions[0]
        restriction = restriction_atom.get("restriction")
        if restriction not in {"hexproof", "shroud"} or restriction_atom != {
            "duration": "while_source_on_battlefield",
            "op": "targeting_restriction",
            "restriction": restriction,
            "subject": "equipped_creature",
        }:
            raise ValueError(f"{case['scenario_id']}: invalid equipment protection expectation")

    controller_targetable = restriction != "shroud"
    opponent_targetable = restriction is None
    attached_snapshot = {
        "power": 2 + power_delta,
        "toughness": 2 + toughness_delta,
        "haste": "haste" in granted,
        "controller_targetable": controller_targetable,
        "opponent_targetable": opponent_targetable,
    }
    base_snapshot = {
        "power": 2,
        "toughness": 2,
        "haste": False,
        "controller_targetable": True,
        "opponent_targetable": True,
    }

    attack_searches = [
        atom
        for atom in atoms
        if atom.get("op") == "search_library"
        and atom.get("trigger") == "equipped_creature_attacks"
    ]
    attack_trigger = None
    if attack_searches:
        if attack_searches != [
            {
                "filter": "basic_land",
                "maximum": 1,
                "op": "search_library",
                "optional": True,
                "player": "source_controller",
                "trigger": "equipped_creature_attacks",
            }
        ]:
            raise ValueError(f"{case['scenario_id']}: invalid attached attack search expectation")
        required_followups = [
            {
                "chosen": "search_result",
                "destination": "battlefield",
                "op": "move_zone",
            },
            {"chosen": "search_result", "op": "tap_object"},
            {"op": "shuffle_library", "player": "source_controller"},
        ]
        for expected in required_followups:
            if expected not in atoms:
                raise ValueError(
                    f"{case['scenario_id']}: attached attack search is missing {expected['op']}"
                )
        attack_trigger = {
            "count": 1,
            "source_bound_condition": True,
            "registered": True,
            "choice_slots": 1,
            "optional_choices": 1,
            "nonbasic_choice_rejected": True,
            "basic_choice_bound": True,
            "bound_action_count": 3,
            "move_action_present": True,
            "tap_action_present": True,
            "shuffle_action_present": True,
            "all_actions_applied": True,
            "basic_land_moved_to_battlefield": True,
            "basic_land_tapped": True,
            "nonbasic_land_remained_in_library": True,
        }

    return {
        "setup_succeeded": True,
        "equip_ability_count": 1,
        "generic_mana_cost": generic_cost,
        "colored_mana_cost": 0,
        "exact_payment_total": generic_cost,
        "timing": "sorcery",
        "target_slots": 1,
        "optional_choices": 0,
        "static_registration_count": 1 + len(restrictions),
        "static_registered": True,
        "controlled_creature_target_bound": True,
        "opponent_creature_target_rejected": True,
        "noncreature_target_rejected": True,
        "source_bound_attach_actions": True,
        "payments_consumed": True,
        "attached_to_first": True,
        "first_attachment": attached_snapshot,
        "attached_to_second": True,
        "first_after_reattachment": base_snapshot,
        "second_after_reattachment": attached_snapshot,
        "attached_attack_trigger": attack_trigger,
        "source_moved_to_graveyard": True,
        "second_after_expiration": base_snapshot,
    }


def expected_sacrifice_counter_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    activations = [
        atom
        for atom in atoms
        if atom.get("op") == "activate_ability"
        and atom.get("ability") == "sacrifice_for_counter"
    ]
    if not activations:
        return None
    if len(activations) != 1:
        raise ValueError(f"{case['scenario_id']}: expected one sacrifice activation")
    activation = activations[0]
    if activation != {
        "ability": "sacrifice_for_counter",
        "cost": {
            "count": 1,
            "kind": "sacrifice",
            "predicate": "creature_you_control",
        },
        "op": "activate_ability",
        "timing": "instant",
    }:
        raise ValueError(f"{case['scenario_id']}: invalid sacrifice activation expectation")
    restrictions = [atom for atom in atoms if atom.get("op") == "combat_restriction"]
    counters = [atom for atom in atoms if atom.get("op") == "add_counters"]
    if restrictions != [
        {
            "duration": "while_source_on_battlefield",
            "op": "combat_restriction",
            "restriction": "cannot_block",
            "subject": "source",
        }
    ] or counters != [
        {
            "amount": 1,
            "counter": "+1/+1",
            "object": "source",
            "op": "add_counters",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: incomplete Carrion Feeder expectation")
    permanent = next(
        (atom for atom in atoms if atom.get("op") == "resolve_permanent"), None
    )
    if not isinstance(permanent, dict):
        raise ValueError(f"{case['scenario_id']}: source characteristics are missing")
    power = permanent.get("power")
    toughness = permanent.get("toughness")
    if not isinstance(power, int) or not isinstance(toughness, int):
        raise ValueError(f"{case['scenario_id']}: invalid source characteristics")
    return {
        "setup_succeeded": True,
        "ability_count": 1,
        "generic_mana_cost": 0,
        "colored_mana_cost": 0,
        "timing": "instant",
        "target_slots": 0,
        "object_choice_slots": 0,
        "optional_choices": 0,
        "sacrifice_count": 1,
        "sacrifice_requires_creature": True,
        "sacrifice_requires_controller": True,
        "fodder_matches": True,
        "opponent_creature_rejected": True,
        "noncreature_rejected": True,
        "fodder_sacrificed": True,
        "source_remained_battlefield": True,
        "source_bound_counter_action": True,
        "counter_action_applied": True,
        "plus_one_counters": 1,
        "power_after_counter": power + 1,
        "toughness_after_counter": toughness + 1,
        "combat_setup_succeeded": True,
        "could_block_before_restriction": True,
        "static_registration_count": 1,
        "restriction_registered": True,
        "source_bound_cannot_block_definition": True,
        "can_block_after_restriction": False,
    }


def expected_temporary_protection_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    targeting = [atom for atom in atoms if atom.get("op") == "targeting_restriction"]
    indestructible = [atom for atom in atoms if atom.get("op") == "indestructible"]
    if not targeting or not indestructible:
        return None
    if targeting != [
        {
            "duration": "until_end_of_turn",
            "op": "targeting_restriction",
            "restriction": "hexproof",
            "subject": "permanents_you_control",
        }
    ] or indestructible != [
        {
            "duration": "until_end_of_turn",
            "op": "indestructible",
            "subject": "permanents_you_control",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid temporary protection expectation")
    return {
        "setup_succeeded": True,
        "bound_action_count": 4,
        "bound_actions_are_restrictions": True,
        "all_actions_applied": True,
        "restriction_count": 4,
        "all_until_end_of_turn": True,
        "controlled_creature_has_hexproof": True,
        "controlled_artifact_has_hexproof": True,
        "controlled_creature_has_indestructible": True,
        "controlled_artifact_has_indestructible": True,
        "opponent_creature_unprotected": True,
        "opponent_artifact_unprotected": True,
        "opponent_cannot_target_controlled_creature": True,
        "opponent_cannot_target_controlled_artifact": True,
        "controller_can_target_controlled_creature": True,
        "controller_can_target_controlled_artifact": True,
        "protected_creature_survived_destroy": True,
        "protected_artifact_survived_destroy": True,
        "protected_creature_survived_lethal_damage": True,
        "cleanup_reached": True,
        "expired_restriction_count": 4,
        "restrictions_removed_at_cleanup": True,
        "opponent_can_target_creature_after_cleanup": True,
        "opponent_can_target_artifact_after_cleanup": True,
        "artifact_destroyed_after_cleanup": True,
        "creature_died_to_lethal_damage_after_cleanup": True,
    }


def expected_commander_alternate_cost_probe(
    case: dict[str, Any],
) -> dict[str, Any] | None:
    alternate_costs = [
        atom for atom in case.get("semantic_atoms", []) if atom.get("op") == "alternate_cost"
    ]
    if not alternate_costs:
        return None
    if alternate_costs != [
        {
            "condition": "controller_controls_commander",
            "cost": "{0}",
            "op": "alternate_cost",
            "spell": "this_spell",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid commander alternate-cost expectation")
    return {
        "setup_succeeded": True,
        "alternate_cost_count": 1,
        "condition_is_controller_controls_commander": True,
        "printed_generic_mana": 2,
        "printed_colored_mana": 1,
        "alternate_generic_mana": 0,
        "alternate_colored_mana": 0,
        "exact_payment_total": 0,
        "zero_payment_plan_available": True,
        "available_without_controlled_battlefield_commander": False,
        "opponent_commander_does_not_enable": True,
        "undesignated_controlled_creature_does_not_enable": True,
        "available_with_controlled_battlefield_commander": True,
        "unavailable_in_command_zone": True,
        "available_after_return_to_battlefield": True,
    }


def expected_noncreature_counter_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    if not any(
        atom.get("op") == "alternate_cost" for atom in case.get("semantic_atoms", [])
    ):
        return None
    counters = [
        atom
        for atom in case.get("semantic_atoms", [])
        if atom.get("op") == "counter_stack_entry"
    ]
    if not counters:
        return None
    if counters != [{"op": "counter_stack_entry", "target": "noncreature_spell"}]:
        raise ValueError(f"{case['scenario_id']}: invalid noncreature-counter expectation")
    return {
        "setup_succeeded": True,
        "target_slots": 1,
        "requirement_is_stack_entry": True,
        "creature_stack_target_rejected": True,
        "noncreature_stack_target_accepted": True,
        "creature_binding_rejected": True,
        "bound_action_count": 1,
        "source_bound_counter_action": True,
        "counter_action_applied": True,
        "noncreature_countered_to_owner_graveyard": True,
        "creature_remained_on_stack": True,
    }


def expected_temporary_creature_protection_probe(
    case: dict[str, Any],
) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    if not any(atom.get("op") == "alternate_cost" for atom in atoms):
        return None
    indestructible = [atom for atom in atoms if atom.get("op") == "indestructible"]
    if not indestructible:
        return None
    if indestructible != [
        {
            "duration": "until_end_of_turn",
            "op": "indestructible",
            "subject": "creatures_you_control",
        }
    ]:
        raise ValueError(
            f"{case['scenario_id']}: invalid temporary creature-protection expectation"
        )
    return {
        "setup_succeeded": True,
        "bound_action_count": 2,
        "bound_actions_are_restrictions": True,
        "all_actions_applied": True,
        "restriction_count": 2,
        "destroy_creature_protected": True,
        "lethal_creature_protected": True,
        "controlled_noncreature_unprotected": True,
        "opponent_creature_unprotected": True,
        "protected_creature_survived_destroy": True,
        "protected_creature_survived_lethal_damage": True,
        "controlled_noncreature_destroyed_while_effect_active": True,
        "cleanup_reached": True,
        "expired_restriction_count": 2,
        "restrictions_removed_at_cleanup": True,
        "creature_destroyed_after_cleanup": True,
        "creature_died_to_lethal_damage_after_cleanup": True,
    }


def expected_flashback_looting_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    flashback = [atom for atom in atoms if atom.get("op") == "flashback"]
    if not flashback:
        return None
    draws = [atom for atom in atoms if atom.get("op") == "draw_cards"]
    discards = [atom for atom in atoms if atom.get("op") == "discard_cards"]
    if flashback != [
        {
            "cost": "{2}{R}",
            "destination": "exile",
            "op": "flashback",
            "source_zone": "graveyard",
        }
    ] or draws != [
        {"count": 2, "op": "draw_cards", "player": "controller"}
    ] or discards != [
        {
            "count": 2,
            "mode": "explicit_choice",
            "op": "discard_cards",
            "player": "controller",
            "zone": "hand",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid flashback-looting expectation")
    return {
        "setup_succeeded": True,
        "alternate_cost_count": 1,
        "condition_is_source_in_controller_graveyard": True,
        "printed_generic_mana": 0,
        "printed_red_mana": 1,
        "flashback_generic_mana": 2,
        "flashback_red_mana": 1,
        "flashback_exact_payment_total": 3,
        "unavailable_from_hand": True,
        "available_from_graveyard": True,
        "cast_window_ready": True,
        "source_on_stack": True,
        "stack_entry_marked_flashback": True,
        "flashback_cost_consumed": True,
        "stack_resolved": True,
        "source_exiled_on_resolution": True,
        "resolution_recorded": True,
        "choice_slot_count": 1,
        "choice_player_is_controller": True,
        "choice_zone_is_hand": True,
        "choice_minimum": 2,
        "choice_maximum": 2,
        "undersized_choice_rejected_before_mutation": True,
        "duplicate_choice_rejected_before_mutation": True,
        "out_of_zone_choice_rejected_before_mutation": True,
        "bound_action_count": 3,
        "draw_action_exact": True,
        "discard_actions_exact": True,
        "exactly_two_cards_drawn": True,
        "exactly_two_explicit_choices_discarded": True,
        "retained_card_remained_in_hand": True,
        "out_of_zone_card_unchanged": True,
    }


def expected_split_second_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    split_second = [atom for atom in atoms if atom.get("op") == "split_second"]
    if not split_second:
        return None
    destroy = [atom for atom in atoms if atom.get("op") == "destroy_permanent"]
    if split_second != [
        {
            "allows": ["activate_mana_abilities"],
            "forbids": ["cast_spells", "activate_non_mana_abilities"],
            "op": "split_second",
            "while": "source_on_stack",
        }
    ] or destroy != [
        {"op": "destroy_permanent", "target": "artifact|enchantment"}
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid split-second expectation")
    return {
        "setup_succeeded": True,
        "split_second_compiled": True,
        "target_slot_count": 1,
        "artifact_target_accepted": True,
        "enchantment_target_accepted": True,
        "creature_target_rejected": True,
        "enchantment_binding_accepted": True,
        "creature_binding_rejected": True,
        "bound_action_count": 1,
        "destroy_action_exact": True,
        "printed_generic_mana": 2,
        "printed_green_mana": 1,
        "cast_payment_total": 3,
        "source_on_stack": True,
        "stack_entry_marked_split_second": True,
        "cast_cost_consumed": True,
        "priority_passed_to_responder": True,
        "responder_spell_rejected_before_mutation": True,
        "responder_non_mana_ability_rejected_before_mutation": True,
        "responder_mana_ability_allowed": True,
        "responder_green_mana_added": True,
        "mana_source_tapped": True,
        "stack_resolved": True,
        "source_moved_to_owner_graveyard": True,
        "resolution_recorded_split_second": True,
        "destroy_action_applied": True,
        "artifact_destroyed_to_owner_graveyard": True,
        "ordinary_cast_available_after_resolution": True,
    }


def expected_overload_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    costs = [atom for atom in atoms if atom.get("op") == "overload_cost"]
    if not costs:
        return None
    overload = [atom for atom in atoms if atom.get("op") == "overload"]
    moves = [atom for atom in atoms if atom.get("op") == "move_zone"]
    if costs != [
        {
            "condition": "source_in_controller_hand",
            "cost": "{6}{U}",
            "kind": "overload",
            "op": "overload_cost",
        }
    ] or overload != [
        {
            "op": "overload",
            "replace": "target",
            "selector": "nonland_permanents_opponents_control",
            "with": "each",
        }
    ] or moves != [
        {
            "from": "battlefield",
            "mode": "ordinary_target_or_overload_each",
            "op": "move_zone",
            "to": "owner_hand",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid overload expectation")
    return {
        "setup_succeeded": True,
        "contract": {
            "overload_compiled": True,
            "alternate_cost_count": 1,
            "alternate_kind_is_overload": True,
            "condition_is_source_in_controller_hand": True,
            "printed_generic_mana": 1,
            "printed_blue_mana": 1,
            "overload_generic_mana": 6,
            "overload_blue_mana": 1,
            "overload_exact_payment_total": 7,
            "available_in_hand": True,
            "unavailable_outside_hand": True,
            "available_after_return_to_hand": True,
            "ordinary_target_slot_count": 1,
            "overload_target_slot_count": 0,
            "opponent_nonland_target_accepted": True,
            "controller_nonland_target_rejected": True,
            "opponent_land_target_rejected": True,
        },
        "ordinary": {
            "binding_without_target_rejected": True,
            "friendly_binding_rejected": True,
            "land_binding_rejected": True,
            "bound_action_count": 1,
            "action_exact": True,
            "cast_without_target_rejected_before_mutation": True,
            "cast_payment_total": 2,
            "source_on_stack": True,
            "stack_target_exact": True,
            "cost_consumed": True,
            "stack_resolved": True,
            "source_moved_to_graveyard": True,
            "action_applied": True,
            "target_returned_to_owner_hand": True,
            "other_opponent_nonlands_unchanged": True,
            "friendly_and_land_unchanged": True,
        },
        "overload": {
            "available_in_hand": True,
            "cast_payment_total": 7,
            "cast_without_targets_succeeded": True,
            "stack_has_no_targets": True,
            "cost_consumed": True,
            "stack_resolved": True,
            "source_moved_to_graveyard": True,
            "explicit_target_rejected": True,
            "bound_action_count": 3,
            "actions_exact": True,
            "actions_applied": True,
            "each_opponent_nonland_returned": True,
            "stolen_permanent_returned_to_owner": True,
            "friendly_nonland_unchanged": True,
            "opponent_land_unchanged": True,
        },
    }


def expected_evoke_probe(case: dict[str, Any]) -> dict[str, Any] | None:
    atoms = case.get("semantic_atoms", [])
    costs = [atom for atom in atoms if atom.get("op") == "evoke_cost"]
    if not costs:
        return None
    permanents = [atom for atom in atoms if atom.get("op") == "resolve_permanent"]
    draws = [atom for atom in atoms if atom.get("op") == "draw_cards"]
    sacrifices = [atom for atom in atoms if atom.get("op") == "sacrifice_permanent"]
    if permanents != [
        {
            "normal_destination": "battlefield",
            "op": "resolve_permanent",
            "subtypes": ["Elemental"],
            "type_line": "Creature - Elemental",
        }
    ] or costs != [
        {
            "condition": "source_in_controller_hand",
            "cost": "{2}{U}",
            "kind": "evoke",
            "op": "evoke_cost",
        }
    ] or draws != [
        {
            "count": 2,
            "op": "draw_cards",
            "player": "controller",
            "trigger": "source_enters",
        }
    ] or sacrifices != [
        {
            "destination": "owner_graveyard",
            "op": "sacrifice_permanent",
            "subject": "source",
            "trigger": "evoke_source_enters",
        }
    ]:
        raise ValueError(f"{case['scenario_id']}: invalid evoke expectation")
    return {
        "setup_succeeded": True,
        "contract": {
            "alternate_cost_count": 1,
            "alternate_kind_is_evoke": True,
            "condition_is_source_in_controller_hand": True,
            "trigger_count": 2,
            "draw_trigger_is_unconditional": True,
            "sacrifice_trigger_requires_evoke": True,
            "both_triggers_are_source_enters": True,
            "printed_generic_mana": 4,
            "printed_blue_mana": 1,
            "printed_payment_total": 5,
            "evoke_generic_mana": 2,
            "evoke_blue_mana": 1,
            "evoke_exact_payment_total": 3,
            "evoke_available_in_hand": True,
            "normal_applicable_trigger_count": 1,
            "evoke_applicable_trigger_count": 2,
        },
        "normal": {
            "cast_window_ready": True,
            "cast_payment_total": 5,
            "source_on_stack": True,
            "cost_consumed": True,
            "stack_resolved": True,
            "source_entered_battlefield": True,
            "draw_bound_action_count": 1,
            "draw_action_exact": True,
            "draw_action_applied": True,
            "exactly_two_drawn": True,
            "source_remained_battlefield_after_draw": True,
            "evoke_trigger_excluded": True,
        },
        "evoke": {
            "cast_window_ready": True,
            "cast_payment_total": 3,
            "source_on_stack": True,
            "cost_consumed": True,
            "stack_resolved": True,
            "source_entered_before_triggers": True,
            "draw_bound_action_count": 1,
            "sacrifice_bound_action_count": 1,
            "draw_action_exact": True,
            "sacrifice_action_exact": True,
            "missing_source_rejected_without_mutation": True,
            "draw_action_applied": True,
            "exactly_two_drawn": True,
            "source_remained_battlefield_after_draw": True,
            "sacrifice_action_applied": True,
            "source_moved_to_owner_graveyard": True,
            "draw_then_sacrificed": True,
        },
    }


def verify_semantic_probe(case: dict[str, Any], actual: dict[str, Any]) -> None:
    if case.get("status") != "semantic_case_ready":
        return
    probe = actual.get("semantic_probe")
    if not isinstance(probe, dict):
        raise ValueError(f"{case['scenario_id']}: semantic probe is missing")

    expected_mana = expected_mana_abilities(case)
    if expected_mana:
        observed_mana = probe.get("mana_abilities")
        if not isinstance(observed_mana, list) or len(observed_mana) != len(expected_mana):
            raise ValueError(f"{case['scenario_id']}: mana ability count changed")
        for index, (expected, observed) in enumerate(zip(expected_mana, observed_mana)):
            projection = {
                "legal_outputs": observed.get("legal_outputs") if isinstance(observed, dict) else None,
                "damage_to_controller": (
                    observed.get("damage_to_controller") if isinstance(observed, dict) else None
                ),
                "minimum_matching_permanents": (
                    observed.get("minimum_matching_permanents")
                    if isinstance(observed, dict)
                    else None
                ),
            }
            if projection != expected or observed.get("replayed_outputs") != expected["legal_outputs"]:
                raise ValueError(
                    f"{case['scenario_id']}: mana ability {index} did not replay every legal output"
                )
            if observed.get("all_outputs_replayed") is not True:
                raise ValueError(
                    f"{case['scenario_id']}: mana ability {index} replay failed"
                )
            if (
                expected["minimum_matching_permanents"] is not None
                and observed.get("condition_rejected_below_threshold") is not True
            ):
                raise ValueError(
                    f"{case['scenario_id']}: mana ability {index} condition did not fail closed"
                )

    expected_subtypes = expected_base_subtypes(case)
    if expected_subtypes is not None:
        observed_subtypes = probe.get("base_subtypes")
        if not isinstance(observed_subtypes, list) or sorted(observed_subtypes) != expected_subtypes:
            raise ValueError(
                f"{case['scenario_id']}: printed subtype state changed; "
                f"expected={expected_subtypes}, actual={observed_subtypes}"
            )

    player_rule_atoms = [
        atom
        for atom in case.get("semantic_atoms", [])
        if atom.get("op") == "modify_player_rules"
    ]
    if player_rule_atoms:
        if player_rule_atoms != [
            {
                "op": "modify_player_rules",
                "player": "source_controller",
                "rule": "no_maximum_hand_size",
                "duration": "while_source_on_battlefield",
            }
        ]:
            raise ValueError(f"{case['scenario_id']}: invalid player-rule expectation")
        rule_probe = probe.get("no_maximum_hand_size")
        required = {
            "setup_succeeded": True,
            "registered": True,
            "active_for_controller": True,
            "opponent_unaffected": True,
            "moved_source_to_graveyard": True,
            "expired_off_battlefield": True,
        }
        if rule_probe != required:
            raise ValueError(
                f"{case['scenario_id']}: no-maximum-hand-size rule did not remain source-bound"
            )

    equipment = expected_equipment_probe(case)
    if equipment is not None and probe.get("equipment") != equipment:
        raise ValueError(f"{case['scenario_id']}: equipment semantic probe changed")

    sacrifice_counter = expected_sacrifice_counter_probe(case)
    if sacrifice_counter is not None and probe.get("sacrifice_counter") != sacrifice_counter:
        raise ValueError(f"{case['scenario_id']}: sacrifice-counter semantic probe changed")

    temporary_protection = expected_temporary_protection_probe(case)
    if (
        temporary_protection is not None
        and probe.get("temporary_protection") != temporary_protection
    ):
        raise ValueError(f"{case['scenario_id']}: temporary-protection semantic probe changed")

    commander_alternate_cost = expected_commander_alternate_cost_probe(case)
    if (
        commander_alternate_cost is not None
        and probe.get("commander_alternate_cost") != commander_alternate_cost
    ):
        raise ValueError(f"{case['scenario_id']}: commander alternate-cost probe changed")

    noncreature_counter = expected_noncreature_counter_probe(case)
    if (
        noncreature_counter is not None
        and probe.get("noncreature_counter") != noncreature_counter
    ):
        raise ValueError(f"{case['scenario_id']}: noncreature-counter semantic probe changed")

    temporary_creature_protection = expected_temporary_creature_protection_probe(case)
    if (
        temporary_creature_protection is not None
        and probe.get("temporary_creature_protection")
        != temporary_creature_protection
    ):
        raise ValueError(
            f"{case['scenario_id']}: temporary creature-protection probe changed"
        )

    flashback_looting = expected_flashback_looting_probe(case)
    if (
        flashback_looting is not None
        and probe.get("flashback_looting") != flashback_looting
    ):
        raise ValueError(f"{case['scenario_id']}: flashback-looting semantic probe changed")

    split_second = expected_split_second_probe(case)
    if split_second is not None and probe.get("split_second") != split_second:
        raise ValueError(f"{case['scenario_id']}: split-second semantic probe changed")

    overload = expected_overload_probe(case)
    if overload is not None and probe.get("overload") != overload:
        raise ValueError(f"{case['scenario_id']}: overload semantic probe changed")

    evoke = expected_evoke_probe(case)
    if evoke is not None and probe.get("evoke") != evoke:
        raise ValueError(f"{case['scenario_id']}: evoke semantic probe changed")


def aggregate_translated_hash(cases: dict[str, Any]) -> str:
    payload = [
        [case["translated_path"], case["translated_source_sha256"]]
        for case in cases["cases"]
    ]
    return sha256_bytes(json_bytes(payload))


def build_report(cases: dict[str, Any], observed: list[dict[str, Any]]) -> dict[str, Any]:
    verified = [
        {
            "scenario_id": case["scenario_id"],
            "freeze_rank": case["freeze_rank"],
            "oracle_id": case["oracle_id"],
            "card_name": case["card_name"],
            "stratum": case["stratum"],
            "final_hash": case["expected_runtime"]["final_hash"],
        }
        for case in cases["cases"]
        if case["status"] == "semantic_case_ready"
    ]
    semantic_blocked = [
        {
            "scenario_id": case["scenario_id"],
            "card_name": case["card_name"],
            "reason_codes": [blocker["code"] for blocker in case["blockers"]],
        }
        for case in cases["cases"]
        if case["status"] == "blocked_semantic_gap"
    ]
    runtime_blocked = [
        {
            "scenario_id": case["scenario_id"],
            "card_name": case["card_name"],
            "reason_code": case["expected_runtime"]["code"],
        }
        for case in cases["cases"]
        if case["status"] == "blocked_runtime"
    ]
    semantic_reason_counts = Counter(
        blocker["code"]
        for case in cases["cases"]
        if case["status"] == "blocked_semantic_gap"
        for blocker in case["blockers"]
    )
    runtime_reason_counts = Counter(item["reason_code"] for item in runtime_blocked)
    runtime_passed = sum(
        case["expected_runtime"]["disposition"] == "passed" for case in cases["cases"]
    )
    return {
        "schema_version": 2,
        "generated_at": "2026-07-13",
        "task": "T3.6-B",
        "status": "pass_incremental_semantic_slice",
        "verification_mode": "local_only",
        "claim_boundary": (
            f"{len(verified)} identities have one card-specific expected production path and exact "
            f"deterministic replay. The other {100 - len(verified)} remain reason-coded and are not "
            "semantic_verified; CP-CARD-SEMANTICS-100 remains open."
        ),
        "checkpoint": {
            "id": "CP-CARD-SEMANTICS-100",
            "status": "in_progress",
            "required": 100,
            "semantic_verified": len(verified),
            "remaining": 100 - len(verified),
        },
        "product_binding": {
            "runtime_source_commit": cases["product_source_commit"],
            "candidate_payload_sha256": cases["candidate_payload_sha256"],
            "semantic_cases_payload_sha256": cases["payload_sha256"],
            "translated_definitions_aggregate_sha256": aggregate_translated_hash(cases),
            "observed_replay_sha256": sha256_bytes(json_bytes(observed)),
        },
        "artifacts": {
            "semantic_cases": {
                "path": str(CASES_PATH.relative_to(ROOT)),
                "sha256": sha256_file(CASES_PATH),
            },
            "runtime_probe": {
                "path": str(PROBE_SOURCE.relative_to(ROOT)),
                "sha256": sha256_file(PROBE_SOURCE),
            },
            "runner": {
                "path": str(Path(__file__).resolve().relative_to(ROOT)),
                "sha256": sha256_file(Path(__file__).resolve()),
            },
        },
        "measured": {
            "frozen_candidates": 100,
            "runtime_smoke_passed": runtime_passed,
            "semantic_verified": len(verified),
            "blocked_semantic_gap": len(semantic_blocked),
            "blocked_runtime": len(runtime_blocked),
            "production_failures": 0,
            "semantic_blocker_reason_counts": dict(sorted(semantic_reason_counts.items())),
            "runtime_blocker_reason_counts": dict(sorted(runtime_reason_counts.items())),
        },
        "semantic_verified_identities": verified,
        "blocked_semantic_gap": semantic_blocked,
        "blocked_runtime": runtime_blocked,
        "deterministic_replay": {
            "runs": 2,
            "exact_report_match": True,
            "final_hashes_nonzero": all(int(item["final_hash"]) > 0 for item in verified),
        },
        "verification": [
            {
                "command": (
                    "CARGO_NET_OFFLINE=true python3 tools/run_t3_6_commander_semantics.py "
                    "--translated-root target/translated-cards --cargo-target-dir target "
                    "--report reports/gates/T3.6-B/EVIDENCE.json"
                ),
                "result": (
                    f"PASS; two exact production replays, {len(verified)} semantic outcomes matched, "
                    f"{len(semantic_blocked)} semantic gaps and {len(runtime_blocked)} runtime "
                    "blockers remained fail-closed"
                ),
            }
        ],
        "constraints": {
            "network_used": False,
            "installs_performed": False,
            "github_actions_used": False,
            "push_performed": False,
            "runtime_source_files_edited_by_sidecar": False,
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--translated-root",
        type=Path,
        default=ROOT / "target/translated-cards",
        help="directory containing generated .frs definitions",
    )
    parser.add_argument("--probe", type=Path, help="use an existing runtime probe binary")
    parser.add_argument(
        "--cargo-target-dir",
        type=Path,
        default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")),
        help="shared Cargo target directory used to build the probe",
    )
    parser.add_argument("--report", type=Path, help="write the exact T3.6-B evidence JSON")
    parser.add_argument(
        "--validate-only",
        action="store_true",
        help="validate frozen cases and source bindings without executing the runtime",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        _, cases = validate_manifest(args.translated_root.resolve())
        if args.validate_only:
            print(
                "T3.6 semantic cases valid: "
                f"ready={cases['summary']['semantic_case_ready']} "
                f"semantic_blocked={cases['summary']['blocked_semantic_gap']} "
                f"runtime_blocked={cases['summary']['blocked_runtime']}"
            )
            return 0
        probe = args.probe.resolve() if args.probe else build_probe(args.cargo_target_dir.resolve())
        first = run_probe(probe, args.translated_root.resolve(), cases)
        second = run_probe(probe, args.translated_root.resolve(), cases)
        if first != second:
            raise ValueError("two exact production replays produced different reports")
        verify_observed(cases, first)
        report = build_report(cases, first)
        if args.report:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(
            "T3.6 semantic replay PASS: "
            f"verified={report['measured']['semantic_verified']}/100 "
            f"semantic_blocked={report['measured']['blocked_semantic_gap']} "
            f"runtime_blocked={report['measured']['blocked_runtime']}"
        )
        return 0
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"T3.6 semantic replay FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
