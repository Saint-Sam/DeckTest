#!/usr/bin/env python3
"""Generate the CP-LAYERS owner-approved reviewer oracle pack.

The generated `.ron` files are committed. Re-run this tool only when the
approved CP-LAYERS reviewer scenario packet intentionally changes.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ORACLE_DIR = ROOT / "tests" / "oracle" / "reviewer_layers"
EVIDENCE_DIR = ROOT / "reports" / "gates" / "CP-LAYERS" / "reviewer_oracles"
COUNT = 100


@dataclass(frozen=True)
class Scenario:
    index: int
    slug: str
    focus: str
    setup: str
    script: list[str]
    expect: str


def list_value(items: list[str]) -> str:
    return "[" + ", ".join(f'"{item}"' for item in items) + "]"


def types(*items: str) -> str:
    return list_value(list(items))


def colors(*items: str) -> str:
    return list_value(list(items))


def keywords(*items: str) -> str:
    return list_value(list(items))


def object_setup(
    card: int,
    owner: int = 0,
    *,
    controller: int | None = None,
    zone: str = "Battlefield",
    player: int | None = None,
) -> str:
    if controller is None:
        controller = owner
    player_part = "" if player is None else f", player: {player}"
    return (
        f'(card: {card}, owner: {owner}, controller: {controller}, '
        f'zone: "{zone}"{player_part})'
    )


def library(player: int, start: int, count: int = 2) -> str:
    cards = ", ".join(str(start + offset) for offset in range(count))
    return f"(player: {player}, cards: [{cards}])"


def setup(objects: list[str], *, libraries: list[str] | None = None) -> str:
    lines = ["        players: 2,"]
    if libraries:
        lines.append("        libraries: [")
        for item in libraries:
            lines.append(f"            {item},")
        lines.append("        ],")
    lines.append("        objects: [")
    for item in objects:
        lines.append(f"            {item},")
    lines.append("        ],")
    return "\n".join(lines)


def step(action: str, **fields: object) -> str:
    parts = [f'action: "{action}"']
    for key, value in fields.items():
        if isinstance(value, bool):
            rendered = "true" if value else "false"
        else:
            rendered = str(value)
        parts.append(f"{key}: {rendered}")
    return f"({', '.join(parts)})"


def base(obj: int, power: int, toughness: int, kws: str = "[]") -> str:
    return step(
        "set_base_creature",
        object=obj,
        power=power,
        toughness=toughness,
        keywords=kws,
    )


def ce(
    operation: str,
    *,
    controller: int = 0,
    target: int | None = None,
    all_objects: bool = False,
    timestamp: int | None = None,
    **fields: object,
) -> str:
    kwargs: dict[str, object] = {"controller": controller}
    if all_objects:
        kwargs["all_objects"] = True
    else:
        assert target is not None
        kwargs["target_object"] = target
    kwargs["operation"] = f'"{operation}"'
    kwargs.update(fields)
    if timestamp is not None:
        kwargs["timestamp"] = timestamp
    return step("register_continuous_effect", **kwargs)


def assert_chars(obj: int, **fields: object) -> str:
    return step("assert_characteristics", object=obj, **fields)


def chars(obj: int, **fields: object) -> str:
    parts = [f"object: {obj}"]
    for key, value in fields.items():
        if isinstance(value, bool):
            rendered = "true" if value else "false"
        else:
            rendered = str(value)
        parts.append(f"{key}: {rendered}")
    return f"({', '.join(parts)})"


def zone_count(zone: str, count: int, player: int | None = None) -> str:
    player_part = "" if player is None else f", player: {player}"
    return f'(zone: "{zone}"{player_part}, count: {count})'


def expect(
    *,
    characteristics: list[str] | None = None,
    zones: list[str] | None = None,
    include_life: bool = False,
) -> str:
    lines: list[str] = []
    if zones:
        lines.append("        zone_counts: [")
        for item in zones:
            lines.append(f"            {item},")
        lines.append("        ],")
    if characteristics:
        lines.append("        characteristics: [")
        for item in characteristics:
            lines.append(f"            {item},")
        lines.append("        ],")
    lines.append('        outcome: "in_progress",')
    lines.append("        invariants: [")
    lines.append('            "zone_conservation",')
    if include_life:
        lines.append('            "life_poison_sanity",')
    lines.append('            "hash_consistency",')
    lines.append("        ],")
    lines.append("        hash_determinism: true,")
    return "\n".join(lines)


def combat_steps(*, attacker: int = 0, defender: int = 1, blocker: int | None = None) -> list[str]:
    steps = [
        step("start_turn", player=0),
        step("advance_step"),
        step("advance_step"),
        step("advance_step"),
        step("advance_step"),
        step("advance_step"),
        step("declare_attackers", player=0, attacks=f"[(attacker: {attacker}, defender: 1)]"),
    ]
    if blocker is not None:
        steps.extend(
            [
                step("advance_step"),
                step(
                    "declare_blockers",
                    player=1,
                    blocks=f"[(blocker: {blocker}, attacker: {attacker})]",
                ),
            ]
        )
    return steps


def sc(
    index: int,
    slug: str,
    focus: str,
    script: list[str],
    characteristics: list[str] | None = None,
    *,
    objects: int | list[str] = 1,
    zones: list[str] | None = None,
    libraries: list[str] | None = None,
    include_life: bool = False,
) -> Scenario:
    if isinstance(objects, int):
        setup_objects = [
            object_setup(64_000 + index * 10 + offset) for offset in range(objects)
        ]
    else:
        setup_objects = objects
    return Scenario(
        index=index,
        slug=slug,
        focus=focus,
        setup=setup(setup_objects, libraries=libraries),
        script=script,
        expect=expect(characteristics=characteristics, zones=zones, include_life=include_life),
    )


def make_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = [
        sc(1, "copy_ignores_source_modifier", "R001", [base(0, 2, 3, keywords("flying")), base(1, 1, 1), ce("modify_pt", target=0, power=3, toughness=3, timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(0, power=5, toughness=6), chars(1, power=2, toughness=3, keywords=keywords("flying"))], objects=2),
        sc(2, "copy_excludes_gained_keyword", "R002", [base(0, 2, 2), base(1, 1, 1), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(1, power=2, toughness=2, keywords=keywords())], objects=2),
        sc(3, "copy_animated_noncreature_excludes_animation", "R003", [base(1, 1, 1), ce("add_types", target=0, types=types("creature"), timestamp=1), ce("set_pt", target=0, power=4, toughness=4, timestamp=2), ce("copy_base_creature", target=1, from_object=0, timestamp=3)], [chars(0, is_creature=True, power=4, toughness=4), chars(1, is_creature=False, types=types())], objects=2),
        sc(4, "copy_excludes_later_cda_marker", "R004", [base(0, 1, 1), base(1, 3, 3), ce("set_base_pt", target=0, power=5, toughness=5, timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(0, power=5, toughness=5), chars(1, power=1, toughness=1)], objects=2),
        sc(5, "copy_excludes_text_marker", "R005", [base(0, 2, 4), base(1, 1, 1), ce("set_text_marker", target=0, marker=9, timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(0, text_marker=9), chars(1, power=2, toughness=4, text_marker=0)], objects=2),
        sc(6, "copy_excludes_color_effect", "R006", [base(0, 3, 3), base(1, 1, 1), ce("set_colors", target=0, colors=colors("red"), timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(0, colors=colors("red")), chars(1, colors=colors())], objects=2),
        sc(7, "copy_excludes_global_animation", "R007", [base(1, 1, 1), ce("add_types", all_objects=True, types=types("creature"), timestamp=1), ce("set_pt", target=0, power=6, toughness=6, timestamp=2), ce("copy_base_creature", target=1, from_object=0, timestamp=3)], [chars(0, is_creature=True, power=6, toughness=6), chars(1, is_creature=True, power=0, toughness=0)], objects=2),
        sc(8, "copy_then_own_modifier", "R008", [base(0, 2, 2), base(1, 1, 1), ce("copy_base_creature", target=1, from_object=0, timestamp=1), ce("modify_pt", target=1, power=1, toughness=2, timestamp=2)], [chars(0, power=2, toughness=2), chars(1, power=3, toughness=4)], objects=2),
        sc(9, "copy_before_later_type_layer", "R009", [base(0, 4, 1), base(1, 1, 1), ce("set_types", target=1, types=types("land"), timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=99)], [chars(1, is_creature=False, types=types("land"))], objects=2),
        sc(10, "copy_replay_hash", "R010", [base(0, 5, 2, keywords("trample")), base(1, 1, 1), ce("copy_base_creature", target=1, from_object=0, timestamp=1)], [chars(1, power=5, toughness=2, keywords=keywords("trample"))], objects=2),
        sc(11, "control_target_specific", "R011", [base(0, 2, 2), base(1, 2, 2), ce("change_controller", target=0, player=1, timestamp=1)], [chars(0, controller=1), chars(1, controller=0)], objects=2),
        sc(12, "control_global_then_specific", "R012", [base(0, 2, 2), base(1, 2, 2), ce("change_controller", all_objects=True, player=1, timestamp=1), ce("change_controller", target=0, player=0, timestamp=2)], [chars(0, controller=0), chars(1, controller=1)], objects=2),
        sc(13, "control_specific_then_global", "R013", [base(0, 2, 2), base(1, 2, 2), ce("change_controller", target=0, player=1, timestamp=1), ce("change_controller", all_objects=True, player=0, timestamp=2)], [chars(0, controller=0), chars(1, controller=0)], objects=2),
        sc(14, "control_dependency_reorders", "R014", [base(0, 2, 2), ce("change_controller", target=0, player=1, timestamp=5), ce("change_controller", target=0, player=0, timestamp=1, dependencies="[0]")], [chars(0, controller=0)]),
        sc(15, "control_equal_timestamp_id_order", "R015", [base(0, 2, 2), ce("change_controller", target=0, player=1, timestamp=7), ce("change_controller", target=0, player=0, timestamp=7)], [chars(0, controller=0)]),
        sc(16, "control_change_allows_attack", "R016", [base(0, 2, 2), ce("change_controller", target=0, player=0, timestamp=1), *combat_steps(attacker=0)], [chars(0, controller=0)], objects=[object_setup(64_160, owner=1, controller=1), object_setup(64_161, owner=0, zone="Library", player=0)], libraries=[library(0, 74_160)]),
        sc(17, "control_query_interleave", "R017", [base(0, 2, 2), assert_chars(0, controller=0), ce("change_controller", target=0, player=1, timestamp=1), assert_chars(0, controller=1)], [chars(0, controller=1)]),
        sc(18, "control_replay_hash", "R018", [base(0, 1, 3), ce("change_controller", target=0, player=1, timestamp=1), ce("change_controller", target=0, player=0, timestamp=2)], [chars(0, controller=0)]),
        sc(19, "text_simple_replacement", "R019", [ce("set_text_marker", target=0, marker=19, timestamp=1)], [chars(0, text_marker=19)]),
        sc(20, "text_timestamp_later_wins", "R020", [ce("set_text_marker", target=0, marker=1, timestamp=1), ce("set_text_marker", target=0, marker=2, timestamp=2)], [chars(0, text_marker=2)]),
        sc(21, "text_dependency_reorders", "R021", [ce("set_text_marker", target=0, marker=3, timestamp=5), ce("set_text_marker", target=0, marker=4, timestamp=1, dependencies="[0]")], [chars(0, text_marker=4)]),
        sc(22, "text_target_isolation", "R022", [ce("set_text_marker", all_objects=True, marker=1, timestamp=1), ce("set_text_marker", target=1, marker=2, timestamp=2)], [chars(0, text_marker=1), chars(1, text_marker=2)], objects=2),
        sc(23, "text_copy_exclusion", "R023", [base(0, 2, 2), base(1, 1, 1), ce("set_text_marker", target=0, marker=23, timestamp=1), ce("copy_base_creature", target=1, from_object=0, timestamp=2)], [chars(0, text_marker=23), chars(1, text_marker=0)], objects=2),
        sc(24, "text_replay_hash", "R024", [ce("set_text_marker", target=0, marker=7, timestamp=1), ce("set_text_marker", target=0, marker=8, timestamp=2)], [chars(0, text_marker=8)]),
        sc(25, "type_add_creature_to_noncreature", "R025", [ce("add_types", target=0, types=types("creature"), timestamp=1), ce("set_pt", target=0, power=3, toughness=3, timestamp=2)], [chars(0, is_creature=True, power=3, toughness=3, types=types("creature"))]),
        sc(26, "type_set_replaces_artifact", "R026", [ce("add_types", target=0, types=types("artifact"), timestamp=1), ce("set_types", target=0, types=types("creature"), timestamp=2), ce("set_pt", target=0, power=2, toughness=2, timestamp=3)], [chars(0, is_creature=True, types=types("creature"), power=2, toughness=2)]),
        sc(27, "type_add_artifact_to_creature", "R027", [base(0, 2, 2), ce("add_types", target=0, types=types("artifact"), timestamp=1)], [chars(0, types=types("artifact", "creature"))]),
        sc(28, "type_remove_creature", "R028", [base(0, 2, 2), ce("remove_types", target=0, types=types("creature"), timestamp=1)], [chars(0, is_creature=False, types=types())]),
        sc(29, "type_set_land", "R029", [ce("set_types", target=0, types=types("land"), timestamp=1)], [chars(0, types=types("land"))]),
        sc(30, "type_add_planeswalker_marker", "R030", [ce("add_types", target=0, types=types("planeswalker"), timestamp=1)], [chars(0, types=types("planeswalker"))]),
        sc(31, "type_all_objects", "R031", [base(0, 2, 2), ce("add_types", all_objects=True, types=types("artifact"), timestamp=1)], [chars(0, types=types("artifact", "creature")), chars(1, types=types("artifact"))], objects=2),
        sc(32, "type_global_then_specific", "R032", [ce("set_types", all_objects=True, types=types("land"), timestamp=1), ce("set_types", target=0, types=types("creature"), timestamp=2), ce("set_pt", target=0, power=2, toughness=2, timestamp=3)], [chars(0, is_creature=True, types=types("creature")), chars(1, types=types("land"))], objects=2),
        sc(33, "type_specific_then_global", "R033", [ce("set_types", target=0, types=types("creature"), timestamp=1), ce("set_types", all_objects=True, types=types("land"), timestamp=2)], [chars(0, is_creature=False, types=types("land")), chars(1, types=types("land"))], objects=2),
        sc(34, "type_dependency_reorders", "R034", [ce("add_types", target=0, types=types("artifact"), timestamp=5), ce("set_types", target=0, types=types("artifact", "creature"), timestamp=1, dependencies="[0]"), ce("set_pt", target=0, power=4, toughness=4, timestamp=3)], [chars(0, is_creature=True, types=types("artifact", "creature"), power=4, toughness=4)]),
        sc(35, "type_before_ability_cross_layer", "R035", [ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("add_types", target=0, types=types("creature"), timestamp=9), ce("set_pt", target=0, power=1, toughness=1, timestamp=10)], [chars(0, is_creature=True, power=1, toughness=1, keywords=keywords("flying"))]),
        sc(36, "type_removal_blocks_ability", "R036", [base(0, 2, 2), ce("remove_types", target=0, types=types("creature"), timestamp=1), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=2)], [chars(0, is_creature=False)]),
        sc(37, "type_and_pt_animation", "R037", [ce("add_types", target=0, types=types("creature"), timestamp=1), ce("set_pt", target=0, power=5, toughness=7, timestamp=2)], [chars(0, is_creature=True, power=5, toughness=7)]),
        sc(38, "type_replay_hash", "R038", [ce("add_types", target=0, types=types("artifact"), timestamp=1), ce("add_types", target=0, types=types("creature"), timestamp=2), ce("set_pt", target=0, power=3, toughness=3, timestamp=3)], [chars(0, types=types("artifact", "creature"), power=3, toughness=3)]),
        sc(39, "color_set_mono", "R039", [ce("set_colors", target=0, colors=colors("green"), timestamp=1)], [chars(0, colors=colors("green"))]),
        sc(40, "color_set_multicolor", "R040", [ce("set_colors", target=0, colors=colors("white", "blue"), timestamp=1)], [chars(0, colors=colors("white", "blue"))]),
        sc(41, "color_set_colorless", "R041", [ce("set_colors", target=0, colors=colors("black"), timestamp=1), ce("set_colors", target=0, colors=colors(), timestamp=2)], [chars(0, colors=colors())]),
        sc(42, "color_add_vs_set_model", "R042", [ce("set_colors", target=0, colors=colors("red", "green"), timestamp=1), ce("set_colors", target=0, colors=colors("blue"), timestamp=2)], [chars(0, colors=colors("blue"))]),
        sc(43, "color_global_plus_targeted", "R043", [ce("set_colors", all_objects=True, colors=colors("red"), timestamp=1), ce("set_colors", target=1, colors=colors("blue"), timestamp=2)], [chars(0, colors=colors("red")), chars(1, colors=colors("blue"))], objects=2),
        sc(44, "color_dependency_reorders", "R044", [ce("set_colors", target=0, colors=colors("red"), timestamp=5), ce("set_colors", target=0, colors=colors("green"), timestamp=1, dependencies="[0]")], [chars(0, colors=colors("green"))]),
        sc(45, "color_target_update_proxy", "R045", [assert_chars(0, colors=colors()), ce("set_colors", target=0, colors=colors("white"), timestamp=1), assert_chars(0, colors=colors("white"))], [chars(0, colors=colors("white"))]),
        sc(46, "color_replay_hash", "R046", [ce("set_colors", target=0, colors=colors("black"), timestamp=1), ce("set_colors", target=0, colors=colors("green", "blue"), timestamp=2)], [chars(0, colors=colors("blue", "green"))]),
        sc(47, "ability_grant_keyword", "R047", [base(0, 2, 2), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1)], [chars(0, keywords=keywords("flying"))]),
        sc(48, "ability_remove_keyword", "R048", [base(0, 2, 2, keywords("flying")), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1)], [chars(0, keywords=keywords())]),
        sc(49, "ability_add_then_remove", "R049", [base(0, 2, 2), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=2)], [chars(0, keywords=keywords())]),
        sc(50, "ability_remove_then_add", "R050", [base(0, 2, 2, keywords("flying")), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=2)], [chars(0, keywords=keywords("flying"))]),
        sc(51, "ability_remove_all_represented", "R051", [base(0, 2, 2, keywords("flying", "trample", "haste")), ce("remove_keywords", target=0, keywords=keywords("flying", "trample", "haste"), timestamp=1)], [chars(0, keywords=keywords())]),
        sc(52, "ability_humility_class_stack", "R052", [base(0, 4, 4, keywords("trample")), ce("remove_keywords", target=0, keywords=keywords("trample", "flying"), timestamp=1), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=2), ce("set_pt", target=0, power=1, toughness=1, timestamp=3)], [chars(0, power=1, toughness=1, keywords=keywords("flying"))]),
        sc(53, "ability_global_remove", "R053", [base(0, 2, 2, keywords("flying")), base(1, 2, 2, keywords("trample")), ce("remove_keywords", all_objects=True, keywords=keywords("flying", "trample"), timestamp=1)], [chars(0, keywords=keywords()), chars(1, keywords=keywords())], objects=2),
        sc(54, "ability_target_isolation", "R054", [base(0, 2, 2, keywords("flying")), base(1, 2, 2, keywords("flying")), ce("remove_keywords", target=1, keywords=keywords("flying"), timestamp=1)], [chars(0, keywords=keywords("flying")), chars(1, keywords=keywords())], objects=2),
        sc(55, "ability_type_applicability", "R055", [ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("add_types", target=0, types=types("creature"), timestamp=2), ce("set_pt", target=0, power=2, toughness=2, timestamp=3)], [chars(0, is_creature=True, keywords=keywords("flying"))]),
        sc(56, "ability_dependency_reorders", "R056", [base(0, 2, 2), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=5), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1, dependencies="[0]")], [chars(0, keywords=keywords())]),
        sc(57, "ability_equal_timestamp_id_order", "R057", [base(0, 2, 2), ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=7), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=7)], [chars(0, keywords=keywords())]),
        sc(58, "ability_flying_loss_allows_block", "R058", [base(0, 3, 3, keywords("flying")), base(1, 1, 4), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1), *combat_steps(attacker=0, blocker=1)], [chars(0, keywords=keywords())], objects=[object_setup(64_580, owner=0), object_setup(64_581, owner=1), object_setup(64_582, owner=0, zone="Library", player=0)], libraries=[library(0, 74_580)]),
        sc(59, "ability_lifelink_removed_before_damage", "R059", [base(0, 2, 2, keywords("lifelink")), ce("remove_keywords", target=0, keywords=keywords("lifelink"), timestamp=1)], [chars(0, keywords=keywords())]),
        sc(60, "ability_replay_hash", "R060", [base(0, 2, 2, keywords("reach")), ce("add_keywords", target=0, keywords=keywords("vigilance"), timestamp=1), ce("remove_keywords", target=0, keywords=keywords("reach"), timestamp=2)], [chars(0, keywords=keywords("vigilance"))]),
        sc(61, "pt_7a_base_then_modifier", "R061", [base(0, 1, 1), ce("set_base_pt", target=0, power=4, toughness=5, timestamp=1), ce("modify_pt", target=0, power=1, toughness=1, timestamp=2)], [chars(0, power=5, toughness=6)]),
        sc(62, "pt_7a_after_type_animation", "R062", [ce("add_types", target=0, types=types("creature"), timestamp=1), ce("set_base_pt", target=0, power=2, toughness=6, timestamp=2)], [chars(0, is_creature=True, power=2, toughness=6)]),
        sc(63, "pt_7a_copied_base_proxy", "R063", [base(0, 3, 7), base(1, 1, 1), ce("copy_base_creature", target=1, from_object=0, timestamp=1)], [chars(1, power=3, toughness=7)], objects=2),
        sc(64, "pt_7a_with_ability_removal", "R064", [base(0, 2, 2, keywords("flying")), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("set_base_pt", target=0, power=6, toughness=3, timestamp=2)], [chars(0, power=6, toughness=3, keywords=keywords())]),
        sc(65, "pt_7a_multiple_cdas", "R065", [base(0, 1, 1), ce("set_base_pt", target=0, power=2, toughness=2, timestamp=1), ce("set_base_pt", target=0, power=5, toughness=5, timestamp=2)], [chars(0, power=5, toughness=5)]),
        sc(66, "pt_7a_seen_by_later_modifier", "R066", [base(0, 1, 1), ce("set_base_pt", target=0, power=3, toughness=4, timestamp=1), ce("modify_pt", target=0, power=2, toughness=-1, timestamp=2)], [chars(0, power=5, toughness=3)]),
        sc(67, "pt_7a_replay_hash", "R067", [base(0, 1, 1), ce("set_base_pt", target=0, power=7, toughness=2, timestamp=1)], [chars(0, power=7, toughness=2)]),
        sc(68, "pt_7a_zero_toughness_sba", "R068", [base(0, 1, 1), ce("set_base_pt", target=0, power=1, toughness=0, timestamp=1), step("check_state_based_actions")], zones=[zone_count("Battlefield", 0), zone_count("Graveyard", 1, player=0)]),
        sc(69, "pt_7b_set", "R069", [base(0, 1, 1), ce("set_pt", target=0, power=4, toughness=4, timestamp=1)], [chars(0, power=4, toughness=4)]),
        sc(70, "pt_7b_later_set_wins", "R070", [base(0, 1, 1), ce("set_pt", target=0, power=2, toughness=2, timestamp=1), ce("set_pt", target=0, power=5, toughness=5, timestamp=2)], [chars(0, power=5, toughness=5)]),
        sc(71, "pt_7b_global_plus_specific", "R071", [base(0, 1, 1), base(1, 1, 1), ce("set_pt", all_objects=True, power=2, toughness=2, timestamp=1), ce("set_pt", target=0, power=5, toughness=5, timestamp=2)], [chars(0, power=5, toughness=5), chars(1, power=2, toughness=2)], objects=2),
        sc(72, "pt_7b_ability_independence", "R072", [base(0, 4, 4, keywords("flying")), ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("set_pt", target=0, power=1, toughness=1, timestamp=2)], [chars(0, power=1, toughness=1, keywords=keywords())]),
        sc(73, "pt_7b_type_gating", "R073", [base(0, 1, 1), ce("remove_types", target=0, types=types("creature"), timestamp=1), ce("set_pt", target=0, power=9, toughness=9, timestamp=2)], [chars(0, is_creature=False)]),
        sc(74, "pt_7b_dependency_reorders", "R074", [base(0, 1, 1), ce("set_pt", target=0, power=2, toughness=2, timestamp=5), ce("set_pt", target=0, power=6, toughness=6, timestamp=1, dependencies="[0]")], [chars(0, power=6, toughness=6)]),
        sc(75, "pt_7b_zero_toughness_sba", "R075", [base(0, 1, 1), ce("set_pt", target=0, power=1, toughness=0, timestamp=1), step("check_state_based_actions")], zones=[zone_count("Battlefield", 0), zone_count("Graveyard", 1, player=0)]),
        sc(76, "pt_7b_replay_hash", "R076", [base(0, 1, 1), ce("set_pt", target=0, power=8, toughness=1, timestamp=1)], [chars(0, power=8, toughness=1)]),
        sc(77, "pt_7c_modifier", "R077", [base(0, 2, 2), ce("modify_pt", target=0, power=2, toughness=2, timestamp=1)], [chars(0, power=4, toughness=4)]),
        sc(78, "pt_7c_multiple_modifiers", "R078", [base(0, 2, 2), ce("modify_pt", target=0, power=2, toughness=2, timestamp=1), ce("modify_pt", target=0, power=-1, toughness=-1, timestamp=2)], [chars(0, power=3, toughness=3)]),
        sc(79, "pt_7c_counter_like_modifier", "R079", [base(0, 2, 2), ce("set_pt", target=0, power=2, toughness=2, timestamp=1), ce("modify_pt", target=0, power=1, toughness=1, timestamp=2)], [chars(0, power=3, toughness=3)]),
        sc(80, "pt_7c_global_plus_specific", "R080", [base(0, 2, 2), base(1, 2, 2), ce("modify_pt", all_objects=True, power=2, toughness=2, timestamp=1), ce("modify_pt", target=0, power=-1, toughness=-3, timestamp=2)], [chars(0, power=3, toughness=1), chars(1, power=4, toughness=4)], objects=2),
        sc(81, "pt_7c_type_gating", "R081", [base(0, 2, 2), ce("remove_types", target=0, types=types("creature"), timestamp=1), ce("modify_pt", target=0, power=7, toughness=7, timestamp=2)], [chars(0, is_creature=False)]),
        sc(82, "pt_7c_dependency_executes", "R082", [base(0, 1, 1), ce("modify_pt", target=0, power=1, toughness=0, timestamp=5), ce("modify_pt", target=0, power=2, toughness=3, timestamp=1, dependencies="[0]")], [chars(0, power=4, toughness=4)]),
        sc(83, "pt_7c_lethal_sba", "R083", [base(0, 2, 2), ce("modify_pt", target=0, power=0, toughness=-2, timestamp=1), step("check_state_based_actions")], zones=[zone_count("Battlefield", 0), zone_count("Graveyard", 1, player=0)]),
        sc(84, "pt_7c_replay_hash", "R084", [base(0, 3, 3), ce("modify_pt", target=0, power=-2, toughness=5, timestamp=1)], [chars(0, power=1, toughness=8)]),
        sc(85, "pt_7d_switch", "R085", [base(0, 2, 5), ce("switch_pt", target=0, timestamp=1)], [chars(0, power=5, toughness=2)]),
        sc(86, "pt_7d_double_switch", "R086", [base(0, 2, 5), ce("switch_pt", target=0, timestamp=1), ce("switch_pt", target=0, timestamp=2)], [chars(0, power=2, toughness=5)]),
        sc(87, "pt_7d_set_then_switch", "R087", [base(0, 1, 1), ce("set_pt", target=0, power=3, toughness=7, timestamp=1), ce("switch_pt", target=0, timestamp=2)], [chars(0, power=7, toughness=3)]),
        sc(88, "pt_7d_modify_then_switch", "R088", [base(0, 2, 3), ce("modify_pt", target=0, power=1, toughness=-1, timestamp=1), ce("switch_pt", target=0, timestamp=2)], [chars(0, power=2, toughness=3)]),
        sc(89, "pt_7d_lethal_sba", "R089", [base(0, 0, 3), ce("switch_pt", target=0, timestamp=1), step("check_state_based_actions")], zones=[zone_count("Battlefield", 0), zone_count("Graveyard", 1, player=0)]),
        sc(90, "pt_7d_replay_hash", "R090", [base(0, 6, 1), ce("switch_pt", target=0, timestamp=1)], [chars(0, power=1, toughness=6)]),
        sc(91, "dependency_chain_same_layer", "R091", [ce("set_colors", target=0, colors=colors("red"), timestamp=5), ce("set_colors", target=0, colors=colors("blue"), timestamp=3, dependencies="[0]"), ce("set_colors", target=0, colors=colors("green"), timestamp=1, dependencies="[1]")], [chars(0, colors=colors("green"))]),
        sc(92, "dependency_cycle_public_api_guard", "R092", [ce("set_colors", target=0, colors=colors("red"), timestamp=1), ce("set_colors", target=0, colors=colors("blue"), timestamp=1)], [chars(0, colors=colors("blue"))]),
        sc(93, "dependency_nonapplicable_isolated", "R093", [ce("set_colors", target=1, colors=colors("red"), timestamp=5), ce("set_colors", target=0, colors=colors("blue"), timestamp=1, dependencies="[0]")], [chars(0, colors=colors("blue")), chars(1, colors=colors("red"))], objects=2),
        sc(94, "dependency_cross_layer_guard", "R094", [ce("add_keywords", target=0, keywords=keywords("flying"), timestamp=1), ce("add_types", target=0, types=types("creature"), timestamp=2), ce("set_pt", target=0, power=2, toughness=2, timestamp=3)], [chars(0, is_creature=True, keywords=keywords("flying"), power=2, toughness=2)]),
        sc(95, "equal_timestamp_id_order", "R095", [base(0, 1, 1), ce("set_pt", target=0, power=2, toughness=2, timestamp=9), ce("set_pt", target=0, power=4, toughness=4, timestamp=9), ce("set_pt", target=0, power=6, toughness=6, timestamp=9)], [chars(0, power=6, toughness=6)]),
        sc(96, "registration_replay", "R096", [base(0, 1, 1), ce("set_colors", target=0, colors=colors("black"), timestamp=1), ce("add_keywords", target=0, keywords=keywords("haste"), timestamp=2), ce("modify_pt", target=0, power=2, toughness=0, timestamp=3)], [chars(0, colors=colors("black"), keywords=keywords("haste"), power=3, toughness=1)]),
        sc(97, "reverse_tie_registration", "R097", [base(0, 1, 1), ce("set_pt", target=0, power=5, toughness=5, timestamp=7), ce("set_pt", target=0, power=2, toughness=2, timestamp=7)], [chars(0, power=2, toughness=2)]),
        sc(98, "mutation_query_interleave", "R098", [base(0, 1, 1), assert_chars(0, power=1, toughness=1), ce("modify_pt", target=0, power=4, toughness=4, timestamp=1), assert_chars(0, power=5, toughness=5), ce("switch_pt", target=0, timestamp=2), assert_chars(0, power=5, toughness=5)], [chars(0, power=5, toughness=5)]),
        sc(99, "legal_action_update_attack", "R099", [base(0, 2, 2), ce("change_controller", target=0, player=0, timestamp=1), *combat_steps(attacker=0)], [chars(0, controller=0)], objects=[object_setup(64_990, owner=1, controller=1), object_setup(64_991, owner=0, zone="Library", player=0)], libraries=[library(0, 74_990)]),
        sc(100, "full_brutal_stack", "R100", [base(0, 2, 5, keywords("trample")), base(1, 1, 1), ce("copy_base_creature", target=1, from_object=0, timestamp=1), ce("change_controller", target=1, player=1, timestamp=2), ce("set_text_marker", target=1, marker=100, timestamp=3), ce("add_types", target=1, types=types("artifact"), timestamp=4), ce("set_colors", target=1, colors=colors("white", "blue"), timestamp=5), ce("remove_keywords", target=1, keywords=keywords("trample"), timestamp=6), ce("add_keywords", target=1, keywords=keywords("flying"), timestamp=7), ce("set_pt", target=1, power=3, toughness=6, timestamp=8), ce("modify_pt", target=1, power=1, toughness=-2, timestamp=9), ce("switch_pt", target=1, timestamp=10)], [chars(1, controller=1, text_marker=100, types=types("artifact", "creature"), colors=colors("white", "blue"), keywords=keywords("flying"), power=4, toughness=4)], objects=2),
    ]
    assert len(scenarios) == COUNT
    return scenarios


def render(scenario: Scenario) -> str:
    script = "\n".join(f"        {item}," for item in scenario.script)
    return f"""(
    name: "CP-LAYERS reviewer {scenario.index:03d} {scenario.slug}",
    setup: (
{scenario.setup}
    ),
    script: [
{script}
    ],
    expect: (
{scenario.expect}
    ),
)
"""


def write_pack(target: Path, scenarios: list[Scenario]) -> None:
    target.mkdir(parents=True, exist_ok=True)
    for scenario in scenarios:
        path = target / f"cp_layers_reviewer_{scenario.index:03d}_{scenario.slug}.ron"
        path.write_text(render(scenario), encoding="utf-8")
    manifest = target / "MANIFEST.md"
    lines = [
        "# CP-LAYERS Reviewer Oracle Manifest",
        "",
        "Date: 2026-07-07",
        "",
        "Owner-approved 100-scenario CP-LAYERS reviewer oracle pack.",
        "",
        "| ID | Focus | File |",
        "| --- | --- | --- |",
    ]
    for scenario in scenarios:
        filename = f"cp_layers_reviewer_{scenario.index:03d}_{scenario.slug}.ron"
        lines.append(f"| R{scenario.index:03d} | {scenario.focus} | `{filename}` |")
    manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    scenarios = make_scenarios()
    write_pack(ORACLE_DIR, scenarios)
    write_pack(EVIDENCE_DIR, scenarios)
    print(f"WROTE {COUNT} CP-LAYERS reviewer oracles to {ORACLE_DIR}")
    print(f"WROTE evidence mirror to {EVIDENCE_DIR}")


if __name__ == "__main__":
    main()
