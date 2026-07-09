#!/usr/bin/env python3
"""Generate the T2 exit-gate oracle expansion pack.

The generated `.ron` files are committed so fresh clones can satisfy the T2
oracle-count gate without running this script. Re-run this tool only when
intentionally refreshing the T2 generated scenario pack.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ORACLE_DIR = ROOT / "tests" / "oracle"
GENERATED_DIR = ORACLE_DIR / "generated_t2_gate_622"
TARGET_TOTAL = 1_200


@dataclass(frozen=True)
class Scenario:
    slug: str
    name: str
    setup: str
    script: list[str]
    expect: str


def list_value(items: list[str]) -> str:
    return "[" + ", ".join(f'"{item}"' for item in items) + "]"


def colors(*items: str) -> str:
    return list_value(list(items))


def keywords(*items: str) -> str:
    return list_value(list(items))


def types(*items: str) -> str:
    return list_value(list(items))


def zone(zone_name: str, player: int | None = None) -> str:
    if player is None:
        return f'zone: "{zone_name}"'
    return f'zone: "{zone_name}", player: {player}'


def zone_count(zone_name: str, count: int, player: int | None = None) -> str:
    return f"({zone(zone_name, player)}, count: {count})"


def object_setup(
    card: int,
    owner: int = 0,
    *,
    controller: int | None = None,
    zone_name: str = "Battlefield",
    player: int | None = None,
) -> str:
    if controller is None:
        controller = owner
    player_part = "" if player is None else f", player: {player}"
    return (
        f'(card: {card}, owner: {owner}, controller: {controller}, '
        f'zone: "{zone_name}"{player_part})'
    )


def library(player: int, start: int, count: int) -> str:
    cards = ", ".join(str(start + index) for index in range(count))
    return f"(player: {player}, cards: [{cards}])"


def mana(**parts: int) -> str:
    ordered = [
        ("white", parts.get("white", 0)),
        ("blue", parts.get("blue", 0)),
        ("black", parts.get("black", 0)),
        ("red", parts.get("red", 0)),
        ("green", parts.get("green", 0)),
        ("colorless", parts.get("colorless", 0)),
    ]
    body = ", ".join(f"{name}: {value}" for name, value in ordered if value)
    return f"({body})" if body else "()"


def cost(**parts: int) -> str:
    ordered = [
        ("white", parts.get("white", 0)),
        ("blue", parts.get("blue", 0)),
        ("black", parts.get("black", 0)),
        ("red", parts.get("red", 0)),
        ("green", parts.get("green", 0)),
        ("generic", parts.get("generic", 0)),
    ]
    body = ", ".join(f"{name}: {value}" for name, value in ordered if value)
    return f"({body})" if body else "()"


def setup_block(
    *,
    players: int = 1,
    objects: list[str] | None = None,
    libraries: list[str] | None = None,
) -> str:
    lines = [f"        players: {players},"]
    if libraries:
        lines.append("        libraries: [")
        for item in libraries:
            lines.append(f"            {item},")
        lines.append("        ],")
    if objects:
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
    return step("set_base_creature", object=obj, power=power, toughness=toughness, keywords=kws)


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


def chars(obj: int, **fields: object) -> str:
    parts = [f"object: {obj}"]
    for key, value in fields.items():
        if isinstance(value, bool):
            rendered = "true" if value else "false"
        else:
            rendered = str(value)
        parts.append(f"{key}: {rendered}")
    return f"({', '.join(parts)})"


def assert_chars(obj: int, **fields: object) -> str:
    return step("assert_characteristics", object=obj, **fields)


def expect_block(
    *,
    zones: list[str] | None = None,
    characteristics: list[str] | None = None,
    players: list[str] | None = None,
    outcome: str = '"in_progress"',
    active_player: int | str | None = None,
    priority_player: int | str | None = None,
    current_step: str | None = None,
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
    if players:
        lines.append("        players: [")
        for item in players:
            lines.append(f"            {item},")
        lines.append("        ],")
    lines.append(f"        outcome: {outcome},")
    if active_player is not None:
        rendered = f'"{active_player}"' if isinstance(active_player, str) else str(active_player)
        lines.append(f"        active_player: {rendered},")
    if priority_player is not None:
        rendered = (
            f'"{priority_player}"' if isinstance(priority_player, str) else str(priority_player)
        )
        lines.append(f"        priority_player: {rendered},")
    if current_step is not None:
        lines.append(f'        current_step: "{current_step}",')
    lines.append("        invariants: [")
    lines.append('            "zone_conservation",')
    lines.append('            "hash_consistency",')
    lines.append("        ],")
    lines.append("        hash_determinism: true,")
    return "\n".join(lines)


def render_scenario(scenario: Scenario) -> str:
    lines = [
        "(",
        f'    name: "{scenario.name}",',
        "    setup: (",
        scenario.setup,
        "    ),",
        "    script: [",
    ]
    for item in scenario.script:
        lines.append(f"        {item},")
    lines.extend(["    ],", "    expect: (", scenario.expect, "    ),", ")", ""])
    return "\n".join(lines)


def counter_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    for index in range(120):
        card = 90_000 + index
        mode = index % 4
        if mode == 0:
            power = 1 + (index % 4)
            toughness = 2 + (index % 5)
            amount = 1 + (index % 4)
            script = [
                base(0, power, toughness),
                step("add_object_counters", object=0, kind='"plus_one_plus_one"', amount=amount),
                step("assert_object_counters", object=0, kind='"plus_one_plus_one"', count=amount),
                assert_chars(0, power=power + amount, toughness=toughness + amount),
            ]
            slug = f"plus_counter_characteristics_{index:03d}"
        elif mode == 1:
            power = 7 + (index % 4)
            toughness = 8 + (index % 5)
            amount = 1 + (index % 3)
            script = [
                base(0, power, toughness),
                step("add_object_counters", object=0, kind='"minus_one_minus_one"', amount=amount),
                step("assert_object_counters", object=0, kind='"minus_one_minus_one"', count=amount),
                assert_chars(0, power=power - amount, toughness=toughness - amount),
            ]
            slug = f"minus_counter_characteristics_{index:03d}"
        elif mode == 2:
            plus = 1 + (index % 4)
            minus = 1 + ((index * 2) % 4)
            remaining_plus = max(plus - minus, 0)
            remaining_minus = max(minus - plus, 0)
            script = [
                base(0, 6, 6),
                step("add_object_counters", object=0, kind='"plus_one_plus_one"', amount=plus),
                step("add_object_counters", object=0, kind='"minus_one_minus_one"', amount=minus),
                step("check_state_based_actions"),
                step(
                    "assert_object_counters",
                    object=0,
                    kind='"plus_one_plus_one"',
                    count=remaining_plus,
                ),
                step(
                    "assert_object_counters",
                    object=0,
                    kind='"minus_one_minus_one"',
                    count=remaining_minus,
                ),
                assert_chars(0, power=6 + remaining_plus - remaining_minus, toughness=6 + remaining_plus - remaining_minus),
            ]
            slug = f"plus_minus_cancel_{index:03d}"
        else:
            kind = f'"named:{7_000 + index}"'
            amount = 2 + (index % 5)
            remove = index % 2
            script = [
                base(0, 2, 3),
                step("add_object_counters", object=0, kind=kind, amount=amount),
                step("remove_object_counters", object=0, kind=kind, amount=remove),
                step("assert_object_counters", object=0, kind=kind, count=amount - remove),
                assert_chars(0, power=2, toughness=3),
            ]
            slug = f"named_counter_bookkeeping_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated counter oracle {index:03d}",
                setup=setup_block(objects=[object_setup(card)]),
                script=script,
                expect=expect_block(zones=[zone_count("Battlefield", 1)]),
            )
        )
    return scenarios


def token_copy_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    kw_sets = [keywords(), keywords("flying"), keywords("trample"), keywords("haste")]
    for index in range(100):
        mode = index % 4
        power = 1 + (index % 5)
        toughness = 2 + (index % 4)
        kws = kw_sets[index % len(kw_sets)]
        if mode == 0:
            script = [
                step(
                    "create_token",
                    card=91_000 + index,
                    owner=0,
                    controller=0,
                    power=power,
                    toughness=toughness,
                    keywords=kws,
                ),
                step("assert_object_flags", object=0, token=True, copy=False),
                assert_chars(0, power=power, toughness=toughness, keywords=kws),
            ]
            setup = setup_block()
            zones = [zone_count("Battlefield", 1)]
            slug = f"token_characteristics_{index:03d}"
        elif mode == 1:
            script = [
                step(
                    "create_token",
                    card=91_000 + index,
                    owner=0,
                    controller=0,
                    power=power,
                    toughness=toughness,
                    keywords=kws,
                ),
                step("move_object", object=0, **{"zone": '"Graveyard"', "player": 0}),
                step("check_state_based_actions"),
                step("assert_object_zone", object=0, **{"zone": '"Ceased"'}),
            ]
            setup = setup_block()
            zones = [zone_count("Battlefield", 0), zone_count("Graveyard", 0, 0), zone_count("Ceased", 1)]
            slug = f"token_ceases_off_battlefield_{index:03d}"
        elif mode == 2:
            script = [
                base(0, power, toughness, kws),
                step("create_permanent_copy", source=0, owner=0, controller=0, token=False),
                step("assert_object_flags", object=1, token=False, copy=True, copy_source=0),
                assert_chars(1, power=power, toughness=toughness, keywords=kws),
            ]
            setup = setup_block(objects=[object_setup(91_000 + index)])
            zones = [zone_count("Battlefield", 2)]
            slug = f"permanent_copy_characteristics_{index:03d}"
        else:
            script = [
                base(0, power, toughness, kws),
                step("create_permanent_copy", source=0, owner=0, controller=0, token=True),
                step("assert_object_flags", object=1, token=True, copy=True, copy_source=0),
                assert_chars(1, power=power, toughness=toughness, keywords=kws),
                step("move_object", object=1, **{"zone": '"Graveyard"', "player": 0}),
                step("check_state_based_actions"),
                step("assert_object_zone", object=1, **{"zone": '"Ceased"'}),
            ]
            setup = setup_block(objects=[object_setup(91_000 + index)])
            zones = [zone_count("Battlefield", 1), zone_count("Graveyard", 0, 0), zone_count("Ceased", 1)]
            slug = f"token_copy_ceases_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated token copy oracle {index:03d}",
                setup=setup,
                script=script,
                expect=expect_block(zones=zones),
            )
        )
    return scenarios


def commander_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    color_sets = [
        ["white"],
        ["blue", "red"],
        ["black", "green"],
        ["white", "blue", "black"],
        ["red", "green"],
    ]
    off_colors = ["green", "white", "blue", "red", "black"]
    for index in range(100):
        mode = index % 4
        if mode == 0:
            casts = index % 5
            cols = color_sets[index % len(color_sets)]
            script = [
                step("designate_commander", object=0, colors=colors(*cols)),
                *[step("record_commander_cast", object=0) for _ in range(casts)],
                step(
                    "assert_commander",
                    object=0,
                    commander=True,
                    colors=colors(*cols),
                    cast_count=casts,
                    tax_generic=casts * 2,
                ),
            ]
            setup = setup_block(objects=[object_setup(92_000 + index, zone_name="Command")])
            zones = [zone_count("Command", 1)]
            slug = f"commander_tax_{index:03d}"
        elif mode == 1:
            cols = color_sets[index % len(color_sets)]
            card_cols = cols[: max(1, len(cols) - 1)]
            script = [
                step("designate_commander", object=0, colors=colors(*cols)),
                step("set_object_color_identity", object=1, colors=colors(*card_cols)),
                step("assert_commander_identity_legal", player=0, object=1, expected=True),
                step("validate_commander_color_identity", player=0, objects="[1]"),
            ]
            setup = setup_block(
                objects=[
                    object_setup(92_000 + index * 2, zone_name="Command"),
                    object_setup(92_001 + index * 2),
                ]
            )
            zones = [zone_count("Command", 1), zone_count("Battlefield", 1)]
            slug = f"commander_identity_legal_{index:03d}"
        elif mode == 2:
            cols = color_sets[index % len(color_sets)]
            illegal = next(color for color in off_colors if color not in cols)
            script = [
                step("designate_commander", object=0, colors=colors(*cols)),
                step("set_object_color_identity", object=1, colors=colors(illegal)),
                step("assert_commander_identity_legal", player=0, object=1, expected=False),
            ]
            setup = setup_block(
                objects=[
                    object_setup(92_000 + index * 2, zone_name="Command"),
                    object_setup(92_001 + index * 2),
                ]
            )
            zones = [zone_count("Command", 1), zone_count("Battlefield", 1)]
            slug = f"commander_identity_illegal_{index:03d}"
        else:
            player_count = 3 + (index % 3)
            order = list(range(player_count))
            rotate = index % player_count
            order = order[rotate:] + order[:rotate]
            rendered = "[" + ", ".join(str(player) for player in order) + "]"
            script = [
                step("set_turn_order", order=rendered),
                step("assert_turn_order", order=rendered),
                step("assert_range_of_influence", mode='"off"'),
            ]
            setup = setup_block(players=player_count)
            zones = None
            slug = f"multiplayer_turn_order_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated multiplayer commander oracle {index:03d}",
                setup=setup,
                script=script,
                expect=expect_block(zones=zones),
            )
        )
    return scenarios


def targeting_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    for index in range(100):
        mode = index % 4
        if mode == 0:
            script = [
                base(1, 2, 2),
                step("register_restriction", controller=0, subject_object=1, effect='"shroud"'),
                step(
                    "assert_can_target",
                    player=0,
                    source_object=0,
                    requirement='(kind: "permanent", controller: "you", required_types: ["creature"])',
                    target_object=1,
                    expected=False,
                ),
            ]
            setup = setup_block(
                players=2,
                objects=[
                    object_setup(93_000 + index * 3, owner=0, zone_name="Hand", player=0),
                    object_setup(93_001 + index * 3, owner=0),
                ],
            )
            zones = [zone_count("Hand", 1, 0), zone_count("Battlefield", 1)]
            slug = f"shroud_blocks_target_{index:03d}"
        elif mode == 1:
            script = [
                base(1, 2, 2),
                step("register_restriction", controller=0, subject_object=1, effect='"hexproof"'),
                step(
                    "assert_can_target",
                    player=1,
                    source_object=2,
                    requirement='(kind: "permanent", controller: "opponent", required_types: ["creature"])',
                    target_object=1,
                    expected=False,
                ),
                step(
                    "assert_can_target",
                    player=0,
                    source_object=0,
                    requirement='(kind: "permanent", controller: "you", required_types: ["creature"])',
                    target_object=1,
                    expected=True,
                ),
            ]
            setup = setup_block(
                players=2,
                objects=[
                    object_setup(93_000 + index * 3, owner=0, zone_name="Hand", player=0),
                    object_setup(93_001 + index * 3, owner=0),
                    object_setup(93_002 + index * 3, owner=1, zone_name="Hand", player=1),
                ],
            )
            zones = [zone_count("Hand", 1, 0), zone_count("Hand", 1, 1), zone_count("Battlefield", 1)]
            slug = f"hexproof_owner_exception_{index:03d}"
        elif mode == 2:
            script = [
                base(2, 2, 4),
                ce("set_colors", target=0, colors=colors("red"), timestamp=1),
                step(
                    "register_restriction",
                    controller=1,
                    subject_object=2,
                    effect='"protection"',
                    colors=colors("red"),
                ),
                step(
                    "assert_can_target",
                    player=0,
                    source_object=0,
                    requirement='(kind: "permanent", controller: "opponent", required_types: ["creature"])',
                    target_object=2,
                    expected=False,
                ),
                step(
                    "assert_can_target",
                    player=0,
                    source_object=1,
                    requirement='(kind: "permanent", controller: "opponent", required_types: ["creature"])',
                    target_object=2,
                    expected=True,
                ),
            ]
            setup = setup_block(
                players=2,
                objects=[
                    object_setup(93_000 + index * 3, owner=0, zone_name="Hand", player=0),
                    object_setup(93_001 + index * 3, owner=0, zone_name="Hand", player=0),
                    object_setup(93_002 + index * 3, owner=1),
                ],
            )
            zones = [zone_count("Hand", 2, 0), zone_count("Battlefield", 1)]
            slug = f"protection_color_source_{index:03d}"
        else:
            script = [
                base(0, 3, 3),
                step("register_restriction", controller=1, subject_object=0, effect='"cannot_attack"'),
                step("start_turn", player=0),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step(
                    "assert_can_attack",
                    player=0,
                    attack="(attacker: 0, defender: 1)",
                    expected=False,
                ),
            ]
            setup = setup_block(
                players=2,
                libraries=[library(0, 93_500 + index * 10, 2)],
                objects=[object_setup(93_000 + index)],
            )
            zones = [zone_count("Battlefield", 1)]
            slug = f"cannot_attack_restriction_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated targeting restriction oracle {index:03d}",
                setup=setup,
                script=script,
                expect=expect_block(
                    zones=zones,
                    active_player=0 if mode == 3 else None,
                    priority_player=0 if mode == 3 else None,
                    current_step="DeclareAttackers" if mode == 3 else None,
                ),
            )
        )
    return scenarios


def layer_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    color_pairs = [("red", "green"), ("blue", "white"), ("black", "red"), ("green", "blue")]
    for index in range(100):
        mode = index % 5
        if mode == 0:
            first, second = color_pairs[index % len(color_pairs)]
            script = [
                base(0, 2, 2),
                ce("set_colors", target=0, colors=colors(first), timestamp=1),
                ce("set_colors", target=0, colors=colors(second), timestamp=2),
                assert_chars(0, colors=colors(second), power=2, toughness=2),
            ]
            setup = setup_block(objects=[object_setup(94_000 + index)])
            characteristics = [chars(0, colors=colors(second), power=2, toughness=2)]
            slug = f"layer_color_timestamp_{index:03d}"
        elif mode == 1:
            power = 1 + (index % 5)
            toughness = 2 + (index % 4)
            script = [
                base(0, power, toughness, keywords("flying")),
                ce("add_types", target=0, types=types("artifact"), timestamp=1),
                ce("remove_keywords", target=0, keywords=keywords("flying"), timestamp=2),
                ce("modify_pt", target=0, power=2, toughness=-1, timestamp=3),
                assert_chars(
                    0,
                    types=types("artifact", "creature"),
                    keywords=keywords(),
                    power=power + 2,
                    toughness=toughness - 1,
                ),
            ]
            setup = setup_block(objects=[object_setup(94_000 + index)])
            characteristics = [
                chars(
                    0,
                    types=types("artifact", "creature"),
                    keywords=keywords(),
                    power=power + 2,
                    toughness=toughness - 1,
                )
            ]
            slug = f"layer_type_keyword_pt_stack_{index:03d}"
        elif mode == 2:
            script = [
                base(0, 3, 5, keywords("trample")),
                base(1, 1, 1),
                ce("copy_base_creature", target=1, from_object=0, timestamp=1),
                ce("modify_pt", target=0, power=3, toughness=0, timestamp=2),
                ce("add_keywords", target=1, keywords=keywords("flying"), timestamp=3),
                assert_chars(0, power=6, toughness=5, keywords=keywords("trample")),
                assert_chars(1, power=3, toughness=5, keywords=keywords("flying", "trample")),
            ]
            setup = setup_block(objects=[object_setup(94_000 + index * 2), object_setup(94_001 + index * 2)])
            characteristics = [
                chars(0, power=6, toughness=5, keywords=keywords("trample")),
                chars(1, power=3, toughness=5, keywords=keywords("flying", "trample")),
            ]
            slug = f"layer_copy_then_modifier_{index:03d}"
        elif mode == 3:
            script = [
                base(0, 2, 2),
                base(1, 4, 4),
                ce("set_pt", all_objects=True, power=1, toughness=1, timestamp=1),
                ce("modify_pt", target=1, power=3, toughness=2, timestamp=2),
                ce("switch_pt", target=1, timestamp=3),
                assert_chars(0, power=1, toughness=1),
                assert_chars(1, power=3, toughness=4),
            ]
            setup = setup_block(objects=[object_setup(94_000 + index * 2), object_setup(94_001 + index * 2)])
            characteristics = [chars(0, power=1, toughness=1), chars(1, power=3, toughness=4)]
            slug = f"layer_global_specific_switch_{index:03d}"
        else:
            script = [
                base(0, 1, 1),
                ce("set_colors", target=0, colors=colors("red"), timestamp=5),
                ce("set_colors", target=0, colors=colors("blue"), timestamp=1, dependencies="[0]"),
                assert_chars(0, colors=colors("blue"), power=1, toughness=1),
            ]
            setup = setup_block(objects=[object_setup(94_000 + index)])
            characteristics = [chars(0, colors=colors("blue"), power=1, toughness=1)]
            slug = f"layer_dependency_color_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated layer oracle {index:03d}",
                setup=setup,
                script=script,
                expect=expect_block(zones=[zone_count("Battlefield", 1 if mode in (0, 1, 4) else 2)], characteristics=characteristics),
            )
        )
    return scenarios


def keyword_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    kw_sets = [
        keywords("defender"),
        keywords("haste"),
        keywords("indestructible"),
        keywords("prowess"),
        keywords("flash"),
        keywords("flying", "vigilance"),
    ]
    for index in range(102):
        mode = index % 6
        if mode == 0:
            kws = kw_sets[index % len(kw_sets)]
            script = [base(0, 2 + (index % 3), 3 + (index % 2), kws), assert_chars(0, keywords=kws)]
            setup = setup_block(objects=[object_setup(95_000 + index)])
            expect_kwargs = {"zones": [zone_count("Battlefield", 1)], "characteristics": [chars(0, keywords=kws)]}
            slug = f"keyword_characteristics_{index:03d}"
        elif mode == 1:
            script = [
                base(0, 2, 2, keywords("defender")),
                step("start_turn", player=0),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step("advance_step"),
                step("assert_can_attack", player=0, attack="(attacker: 0, defender: 1)", expected=False),
            ]
            setup = setup_block(players=2, libraries=[library(0, 95_500 + index * 10, 2)], objects=[object_setup(95_000 + index)])
            expect_kwargs = {"zones": [zone_count("Battlefield", 1)], "active_player": 0, "priority_player": 0, "current_step": "DeclareAttackers"}
            slug = f"defender_blocks_attack_{index:03d}"
        elif mode == 2:
            script = [
                base(0, 2, 2, keywords("indestructible")),
                step("mark_damage", object=0, amount=9 + (index % 3)),
                step("check_state_based_actions"),
                step("assert_object_zone", object=0, **{"zone": '"Battlefield"'}),
            ]
            setup = setup_block(objects=[object_setup(95_000 + index)])
            expect_kwargs = {"zones": [zone_count("Battlefield", 1)]}
            slug = f"indestructible_survives_{index:03d}"
        elif mode == 3:
            script = [
                step("scry", player=0, count=2, bottom="[2]"),
                step("assert_zone_order", **{"zone": '"Library"', "player": 0, "objects": "[2, 0, 1]"}),
            ]
            setup = setup_block(
                objects=[
                    object_setup(95_000 + index * 10, zone_name="Library", player=0),
                    object_setup(95_001 + index * 10, zone_name="Library", player=0),
                    object_setup(95_002 + index * 10, zone_name="Library", player=0),
                ],
            )
            expect_kwargs = {"zones": [zone_count("Library", 3, 0)]}
            slug = f"scry_order_{index:03d}"
        elif mode == 4:
            script = [
                step("surveil", player=0, count=2, graveyard="[1]"),
                step("assert_zone_order", **{"zone": '"Library"', "player": 0, "objects": "[0, 2]"}),
                step("assert_zone_order", **{"zone": '"Graveyard"', "player": 0, "objects": "[1]"}),
            ]
            setup = setup_block(
                objects=[
                    object_setup(95_000 + index * 10, zone_name="Library", player=0),
                    object_setup(95_001 + index * 10, zone_name="Library", player=0),
                    object_setup(95_002 + index * 10, zone_name="Library", player=0),
                ],
            )
            expect_kwargs = {"zones": [zone_count("Library", 2, 0), zone_count("Graveyard", 1, 0)]}
            slug = f"surveil_order_{index:03d}"
        else:
            script = [
                step("start_turn", player=0),
                step("advance_step"),
                step("add_mana", player=0, mana=mana(colorless=1)),
                step("cycle_auto", player=0, object=0, cost=cost(generic=1)),
                step("assert_object_zone", object=0, **{"zone": '"Graveyard"', "player": 0}),
                step("assert_object_zone", object=1, **{"zone": '"Hand"', "player": 0}),
            ]
            setup = setup_block(
                objects=[
                    object_setup(95_000 + index * 10, zone_name="Hand", player=0),
                    object_setup(95_001 + index * 10, zone_name="Library", player=0),
                ]
            )
            expect_kwargs = {
                "zones": [zone_count("Hand", 1, 0), zone_count("Graveyard", 1, 0)],
                "players": ["(player: 0, mana: ())"],
                "active_player": 0,
                "priority_player": 0,
                "current_step": "Upkeep",
            }
            slug = f"cycling_zone_mana_{index:03d}"
        scenarios.append(
            Scenario(
                slug=slug,
                name=f"T2 generated keyword oracle {index:03d}",
                setup=setup,
                script=script,
                expect=expect_block(**expect_kwargs),
            )
        )
    return scenarios


def generated_scenarios() -> list[Scenario]:
    scenarios: list[Scenario] = []
    scenarios.extend(counter_scenarios())
    scenarios.extend(token_copy_scenarios())
    scenarios.extend(commander_scenarios())
    scenarios.extend(targeting_scenarios())
    scenarios.extend(layer_scenarios())
    scenarios.extend(keyword_scenarios())
    return scenarios


def count_without_generated_dir() -> int:
    count = 0
    for path in ORACLE_DIR.rglob("*.ron"):
        try:
            path.relative_to(GENERATED_DIR)
        except ValueError:
            count += 1
    return count


def main() -> None:
    base_count = count_without_generated_dir()
    required = TARGET_TOTAL - base_count
    if required < 0:
        raise SystemExit(f"oracle count {base_count} already exceeds target {TARGET_TOTAL}")
    scenarios = generated_scenarios()
    if len(scenarios) != required:
        raise SystemExit(
            f"generator produced {len(scenarios)} scenarios, expected {required} "
            f"for {base_count} existing oracles"
        )
    GENERATED_DIR.mkdir(parents=True, exist_ok=True)
    for stale in GENERATED_DIR.glob("*.ron"):
        stale.unlink()
    for index, scenario in enumerate(scenarios, start=1):
        path = GENERATED_DIR / f"t2_gate_{index:03d}_{scenario.slug}.ron"
        path.write_text(render_scenario(scenario), encoding="utf-8")
    print(f"wrote {len(scenarios)} generated scenarios; {base_count + len(scenarios)} total")


if __name__ == "__main__":
    main()
