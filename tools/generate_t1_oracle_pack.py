#!/usr/bin/env python3
"""Generate the T1 oracle expansion pack.

The generated `.ron` files are committed so fresh clones can run the oracle
gate without running this script. Re-run this tool only when intentionally
refreshing the T1 generated scenario pack.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ORACLE_DIR = ROOT / "tests" / "oracle"
GENERATED_DIR = ORACLE_DIR / "generated_t1_300"
TARGET_TOTAL = 300


@dataclass(frozen=True)
class Scenario:
    slug: str
    name: str
    setup: str
    script: list[str]
    expect: str


def zone(zone_name: str, player: int | None = None) -> str:
    if player is None:
        return f'zone: "{zone_name}"'
    return f'zone: "{zone_name}", player: {player}'


def zone_count(zone_name: str, count: int, player: int | None = None) -> str:
    return f"({zone(zone_name, player)}, count: {count})"


def object_setup(card: int, owner: int, zone_name: str, player: int | None = None) -> str:
    player_part = "" if player is None else f", player: {player}"
    return (
        f'(card: {card}, owner: {owner}, controller: {owner}, '
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
        ("x_count", parts.get("x_count", 0)),
        ("x_value", parts.get("x_value", 0)),
    ]
    body = ", ".join(f"{name}: {value}" for name, value in ordered if value)
    return f"({body})" if body else "()"


def step(action: str, **fields: object) -> str:
    parts = [f'action: "{action}"']
    for key, value in fields.items():
        parts.append(f"{key}: {value}")
    return f"({', '.join(parts)})"


def expect_block(
    *,
    zones: list[str] | None = None,
    players: list[str] | None = None,
    outcome: str = '"in_progress"',
    active_player: str | int | None = None,
    priority_player: str | int | None = None,
    current_step: str | None = None,
) -> str:
    lines = []
    if zones:
        lines.append("        zone_counts: [")
        for item in zones:
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
    lines.extend(
        [
            "        invariants: [",
            '            "zone_conservation",',
            '            "life_poison_sanity",',
            '            "hash_consistency",',
            "        ],",
            "        hash_determinism: true,",
        ]
    )
    return "\n".join(lines)


def setup_block(
    *,
    players: int = 2,
    seed: int | None = None,
    libraries: list[str] | None = None,
    objects: list[str] | None = None,
) -> str:
    lines = []
    if seed is not None:
        lines.append(f"        seed: {seed},")
    lines.append(f"        players: {players},")
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


def creature_step(
    object_index: int,
    power: int,
    toughness: int,
    keywords: str = "[]",
) -> str:
    return step(
        "set_base_creature",
        object=object_index,
        power=power,
        toughness=toughness,
        keywords=keywords,
    )


def combat_setup(index: int, objects: list[str]) -> str:
    return setup_block(
        libraries=[library(0, 50_000 + index * 10, 2)],
        objects=objects,
    )


def enter_declare_attackers(player: int = 0) -> list[str]:
    scripts = [step("start_turn", player=player)]
    scripts.extend(step("advance_step") for _ in range(5))
    return scripts


def declare_attackers(attacker: int, defender: int = 1, player: int = 0) -> str:
    return step(
        "declare_attackers",
        player=player,
        attacks=f"[(attacker: {attacker}, defender: {defender})]",
    )


def declare_blockers(blocks: list[tuple[int, int]], player: int = 1) -> str:
    rendered = ", ".join(
        f"(blocker: {blocker}, attacker: {attacker})" for blocker, attacker in blocks
    )
    return step("declare_blockers", player=player, blocks=f"[{rendered}]")


def damage_to_player(player: int, amount: int) -> str:
    return f"(player: {player}, amount: {amount})"


def damage_to_object(object_index: int, amount: int) -> str:
    return f"(object: {object_index}, amount: {amount})"


def combat_damage_request(source: int, assignments: list[str]) -> str:
    return f"(source: {source}, assignments: [{', '.join(assignments)}])"


def assign_combat_damage(requests: list[str]) -> str:
    return step("assign_combat_damage", assignments=f"[{', '.join(requests)}]")


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
    lines.extend(
        [
            "    ],",
            "    expect: (",
            scenario.expect,
            "    ),",
            ")",
            "",
        ]
    )
    return "\n".join(lines)


def setup_zone_scenarios() -> list[Scenario]:
    scenarios = []
    shared = ["Battlefield", "Exile", "Command", "Stack"]
    owned = ["Library", "Hand", "Graveyard"]
    for index in range(24):
        player_count = 2 + (index % 2)
        objects = []
        counts: dict[tuple[str, int | None], int] = {}
        for offset in range(5):
            if offset % 2 == 0:
                zone_name = shared[(index + offset) % len(shared)]
                player = None
            else:
                zone_name = owned[(index + offset) % len(owned)]
                player = (index + offset) % player_count
            owner = (index + offset) % player_count
            objects.append(object_setup(10_000 + index * 10 + offset, owner, zone_name, player))
            counts[(zone_name, player)] = counts.get((zone_name, player), 0) + 1
        zone_expectations = [
            zone_count(name, count, player) for (name, player), count in sorted(counts.items())
        ]
        scenarios.append(
            Scenario(
                slug=f"setup_zone_{index:03d}",
                name=f"T1 generated setup zone conservation {index:03d}",
                setup=setup_block(players=player_count, objects=objects),
                script=[],
                expect=expect_block(
                    zones=zone_expectations,
                    players=[
                        "(player: 0, life: 20, poison: 0)",
                        "(player: 1, life: 20, poison: 0)",
                    ],
                ),
            )
        )
    return scenarios


def zone_move_scenarios() -> list[Scenario]:
    sources = [
        ("Battlefield", None),
        ("Exile", None),
        ("Command", None),
        ("Library", 0),
        ("Hand", 0),
        ("Graveyard", 0),
    ]
    destinations = [
        ("Graveyard", 0),
        ("Battlefield", None),
        ("Exile", None),
        ("Hand", 1),
        ("Library", 1),
        ("Command", None),
    ]
    scenarios = []
    for index in range(32):
        source = sources[index % len(sources)]
        destination = destinations[(index * 2 + 1) % len(destinations)]
        if source == destination:
            destination = destinations[(index * 2 + 2) % len(destinations)]
        objects = [object_setup(11_000 + index, 0, source[0], source[1])]
        scenarios.append(
            Scenario(
                slug=f"zone_move_{index:03d}",
                name=f"T1 generated object move {index:03d}",
                setup=setup_block(objects=objects),
                script=[step("move_object", object=0, **{"zone": f'"{destination[0]}"'}, **({} if destination[1] is None else {"player": destination[1]}))],
                expect=expect_block(
                    zones=[
                        zone_count(source[0], 0, source[1]),
                        zone_count(destination[0], 1, destination[1]),
                    ],
                ),
            )
        )
    return scenarios


def opening_hand_scenarios() -> list[Scenario]:
    scenarios = []
    for index in range(30):
        extra_a = index % 5
        extra_b = (index * 2) % 5
        seed = 2_000 + index
        scripts = [step("decide_turn_order"), step("draw_opening_hands")]
        if index % 3 == 0:
            scripts.extend(
                [
                    step("keep_opening_hand", player=0, bottom="[]"),
                    step("keep_opening_hand", player=1, bottom="[]"),
                ]
            )
        scenarios.append(
            Scenario(
                slug=f"opening_hand_{index:03d}",
                name=f"T1 generated opening hand counts {index:03d}",
                setup=setup_block(
                    seed=seed,
                    libraries=[
                        library(0, 12_000 + index * 100, 7 + extra_a),
                        library(1, 13_000 + index * 100, 7 + extra_b),
                    ],
                ),
                script=scripts,
                expect=expect_block(
                    zones=[
                        zone_count("Hand", 7, 0),
                        zone_count("Hand", 7, 1),
                        zone_count("Library", extra_a, 0),
                        zone_count("Library", extra_b, 1),
                    ],
                    players=[
                        "(player: 0, life: 20, poison: 0)",
                        "(player: 1, life: 20, poison: 0)",
                    ],
                ),
            )
        )
    return scenarios


def turn_priority_scenarios() -> list[Scenario]:
    scenarios = []
    step_names = [
        "Untap",
        "Upkeep",
        "Draw",
        "PrecombatMain",
        "BeginningOfCombat",
        "DeclareAttackers",
        "EndOfCombat",
        "PostcombatMain",
        "End",
        "Cleanup",
        "Untap",
    ]
    for index in range(30):
        player_count = 2 + (index % 2)
        active = index % player_count
        advances = index % len(step_names)
        scripts = [step("start_turn", player=active)]
        scripts.extend(step("advance_step") for _ in range(advances))
        expected_step = step_names[advances]
        expected_active = (active + 1) % player_count if advances == 10 else active
        priority = "none" if expected_step in ("Untap", "Cleanup") else expected_active
        extra_zones = []
        if advances >= 2:
            extra_zones = [
                zone_count("Hand", 1, active),
                zone_count("Library", 4, active),
            ]
        scenarios.append(
            Scenario(
                slug=f"turn_priority_{index:03d}",
                name=f"T1 generated turn priority window {index:03d}",
                setup=setup_block(
                    players=player_count,
                    libraries=[library(active, 14_000 + index * 10, 5)],
                ),
                script=scripts,
                expect=expect_block(
                    zones=extra_zones,
                    active_player=expected_active,
                    priority_player=priority,
                    current_step=expected_step,
                ),
            )
        )
    return scenarios


def mana_scenarios() -> list[Scenario]:
    scenarios = []
    colors = ["white", "blue", "black", "red", "green"]
    for index in range(24):
        player = index % 2
        color = colors[index % len(colors)]
        scripts = []
        expected = {}
        if index % 4 == 0:
            scripts.append(step("add_mana", player=player, mana=mana(**{color: 2, "colorless": 2})))
            scripts.append(step("pay_mana_auto", player=player, cost=cost(**{color: 1, "generic": 1})))
            expected = {color: 1, "colorless": 1}
        elif index % 4 == 1:
            scripts.append(step("add_mana", player=player, mana=mana(colorless=4)))
            scripts.append(
                step(
                    "pay_mana_auto",
                    player=player,
                    cost=cost(generic=1, x_count=1, x_value=2),
                )
            )
            expected = {"colorless": 1}
        elif index % 4 == 2:
            scripts.append(step("add_mana", player=player, mana=mana(**{color: 3})))
            scripts.append(step("clear_mana", player=player))
            expected = {}
        else:
            scripts.append(step("add_mana", player=player, mana=mana(**{color: 1, "colorless": 1})))
            expected = {color: 1, "colorless": 1}
        scenarios.append(
            Scenario(
                slug=f"mana_{index:03d}",
                name=f"T1 generated mana pool payment {index:03d}",
                setup=setup_block(),
                script=scripts,
                expect=expect_block(
                    players=[f"(player: {player}, mana: {mana(**expected)})"],
                ),
            )
        )
    return scenarios


def life_scenarios() -> list[Scenario]:
    scenarios = []
    for index in range(30):
        mode = index % 4
        if mode == 0:
            player = index % 2
            start = 20 + index
            loss = 1 + (index % 7)
            scripts = [
                step("set_life", player=player, life=start),
                step("lose_life", player=player, amount=loss),
                step("check_state_based_actions"),
            ]
            outcome = '"in_progress"'
            players = [f"(player: {player}, life: {start - loss})"]
        elif mode == 1:
            player = index % 2
            scripts = [
                step("set_life", player=player, life=1),
                step("lose_life", player=player, amount=1 + (index % 3)),
                step("check_state_based_actions"),
            ]
            winner = 1 - player
            outcome = f'(status: "won", player: {winner})'
            players = [f"(player: {player}, life: {0 - (index % 3)})"]
        elif mode == 2:
            scripts = [
                step("lose_life", player=0, amount=20 + (index % 2)),
                step("lose_life", player=1, amount=20 + ((index + 1) % 2)),
                step("check_state_based_actions"),
            ]
            outcome = '"draw"'
            players = ["(player: 0, life: 0)", "(player: 1, life: -1)"]
            if index % 2 == 1:
                players = ["(player: 0, life: -1)", "(player: 1, life: 0)"]
        else:
            player = index % 2
            scripts = [
                step("set_life", player=player, life=1),
                step("lose_life", player=player, amount=1),
                step("gain_life", player=player, amount=5),
                step("check_state_based_actions"),
            ]
            outcome = '"in_progress"'
            players = [f"(player: {player}, life: 5)"]
        scenarios.append(
            Scenario(
                slug=f"life_{index:03d}",
                name=f"T1 generated life SBA {index:03d}",
                setup=setup_block(),
                script=scripts,
                expect=expect_block(players=players, outcome=outcome),
            )
        )
    return scenarios


def poison_scenarios() -> list[Scenario]:
    scenarios = []
    for index in range(22):
        mode = index % 3
        if mode == 0:
            player = index % 2
            amount = index % 10
            scripts = [
                step("add_poison_counters", player=player, amount=amount),
                step("check_state_based_actions"),
            ]
            outcome = '"in_progress"'
            players = [f"(player: {player}, poison: {amount})"]
        elif mode == 1:
            player = index % 2
            amount = 10 + (index % 4)
            scripts = [
                step("add_poison_counters", player=player, amount=amount),
                step("check_state_based_actions"),
            ]
            outcome = f'(status: "won", player: {1 - player})'
            players = [f"(player: {player}, poison: {amount})"]
        else:
            scripts = [
                step("add_poison_counters", player=0, amount=10 + (index % 2)),
                step("add_poison_counters", player=1, amount=10 + ((index + 1) % 2)),
                step("check_state_based_actions"),
            ]
            outcome = '"draw"'
            players = [
                f"(player: 0, poison: {10 + (index % 2)})",
                f"(player: 1, poison: {10 + ((index + 1) % 2)})",
            ]
        scenarios.append(
            Scenario(
                slug=f"poison_{index:03d}",
                name=f"T1 generated poison SBA {index:03d}",
                setup=setup_block(),
                script=scripts,
                expect=expect_block(players=players, outcome=outcome),
            )
        )
    return scenarios


def creature_sba_scenarios() -> list[Scenario]:
    scenarios = []
    keyword_sets = ["[]", '["vigilance"]', '["flying"]', '["reach"]']
    for index in range(30):
        owner = index % 2
        toughness = (index % 5) + 1
        mode = index % 4
        scripts = [
            step(
                "set_base_creature",
                object=0,
                power=(index % 6) + 1,
                toughness=toughness if mode != 2 else 0,
                keywords=keyword_sets[index % len(keyword_sets)],
            )
        ]
        if mode in (0, 1):
            damage = toughness if mode == 0 else toughness - 1
            scripts.append(step("mark_damage", object=0, amount=damage))
        scripts.append(step("check_state_based_actions"))
        dies = mode in (0, 2)
        scenarios.append(
            Scenario(
                slug=f"creature_sba_{index:03d}",
                name=f"T1 generated creature SBA {index:03d}",
                setup=setup_block(
                    objects=[object_setup(15_000 + index, owner, "Battlefield")]
                ),
                script=scripts,
                expect=expect_block(
                    zones=[
                        zone_count("Battlefield", 0 if dies else 1),
                        zone_count("Graveyard", 1 if dies else 0, owner),
                    ],
                ),
            )
        )
    return scenarios


def combat_scenarios() -> list[Scenario]:
    scenarios = []

    for index in range(10):
        power = 2 + (index % 4)
        keywords = '["vigilance"]' if index % 2 else "[]"
        script = [
            creature_step(0, power, 3, keywords),
            *enter_declare_attackers(),
            declare_attackers(0),
            step("advance_step"),
            declare_blockers([]),
            step("advance_step"),
            assign_combat_damage(
                [combat_damage_request(0, [damage_to_player(1, power)])]
            ),
        ]
        scenarios.append(
            Scenario(
                slug=f"combat_unblocked_vigilance_{index:03d}",
                name=f"T1 generated combat unblocked vigilance damage {index:03d}",
                setup=combat_setup(
                    index,
                    [object_setup(17_000 + index, 0, "Battlefield")],
                ),
                script=script,
                expect=expect_block(
                    zones=[zone_count("Battlefield", 1)],
                    players=[f"(player: 1, life: {20 - power})"],
                ),
            )
        )

    for index in range(10):
        if index < 5:
            objects = [
                object_setup(18_000 + index * 2, 0, "Battlefield"),
                object_setup(18_001 + index * 2, 1, "Battlefield"),
            ]
            script = [
                creature_step(0, 3, 3, '["flying"]'),
                creature_step(1, 0, 3, '["reach"]'),
                *enter_declare_attackers(),
                declare_attackers(0),
                step("advance_step"),
                declare_blockers([(1, 0)]),
                step("advance_step"),
                assign_combat_damage(
                    [combat_damage_request(0, [damage_to_object(1, 3)])]
                ),
            ]
            zones = [
                zone_count("Battlefield", 1),
                zone_count("Graveyard", 1, 1),
            ]
        else:
            objects = [
                object_setup(18_000 + index * 3, 0, "Battlefield"),
                object_setup(18_001 + index * 3, 1, "Battlefield"),
                object_setup(18_002 + index * 3, 1, "Battlefield"),
            ]
            script = [
                creature_step(0, 4, 4, '["menace"]'),
                creature_step(1, 0, 2),
                creature_step(2, 0, 2),
                *enter_declare_attackers(),
                declare_attackers(0),
                step("advance_step"),
                declare_blockers([(1, 0), (2, 0)]),
                step("advance_step"),
                assign_combat_damage(
                    [
                        combat_damage_request(
                            0,
                            [damage_to_object(1, 2), damage_to_object(2, 2)],
                        )
                    ]
                ),
            ]
            zones = [
                zone_count("Battlefield", 1),
                zone_count("Graveyard", 2, 1),
            ]
        scenarios.append(
            Scenario(
                slug=f"combat_evasion_block_legality_{index:03d}",
                name=f"T1 generated combat flying reach menace legality {index:03d}",
                setup=combat_setup(20 + index, objects),
                script=script,
                expect=expect_block(zones=zones),
            )
        )

    for index in range(10):
        if index % 2 == 0:
            script = [
                creature_step(0, 2, 2, '["first_strike"]'),
                creature_step(1, 2, 2),
                *enter_declare_attackers(),
                declare_attackers(0),
                step("advance_step"),
                declare_blockers([(1, 0)]),
                step("advance_step"),
                assign_combat_damage(
                    [combat_damage_request(0, [damage_to_object(1, 2)])]
                ),
            ]
            objects = [
                object_setup(19_000 + index * 2, 0, "Battlefield"),
                object_setup(19_001 + index * 2, 1, "Battlefield"),
            ]
            expect = expect_block(
                zones=[
                    zone_count("Battlefield", 1),
                    zone_count("Graveyard", 1, 1),
                ]
            )
        else:
            script = [
                creature_step(0, 2, 2, '["double_strike"]'),
                *enter_declare_attackers(),
                declare_attackers(0),
                step("advance_step"),
                declare_blockers([]),
                step("advance_step"),
                assign_combat_damage(
                    [combat_damage_request(0, [damage_to_player(1, 2)])]
                ),
                step("advance_step"),
                assign_combat_damage(
                    [combat_damage_request(0, [damage_to_player(1, 2)])]
                ),
            ]
            objects = [object_setup(19_000 + index, 0, "Battlefield")]
            expect = expect_block(
                zones=[zone_count("Battlefield", 1)],
                players=["(player: 1, life: 16)"],
            )
        scenarios.append(
            Scenario(
                slug=f"combat_strike_steps_{index:03d}",
                name=f"T1 generated combat first and double strike steps {index:03d}",
                setup=combat_setup(40 + index, objects),
                script=script,
                expect=expect,
            )
        )

    for index in range(10):
        first_toughness = 2
        second_toughness = 3
        script = [
            creature_step(0, first_toughness + second_toughness, 5),
            creature_step(1, 0, first_toughness),
            creature_step(2, 0, second_toughness),
            *enter_declare_attackers(),
            declare_attackers(0),
            step("advance_step"),
            declare_blockers([(1, 0), (2, 0)]),
            step("advance_step"),
            assign_combat_damage(
                [
                    combat_damage_request(
                        0,
                        [
                            damage_to_object(1, first_toughness),
                            damage_to_object(2, second_toughness),
                        ],
                    )
                ]
            ),
        ]
        scenarios.append(
            Scenario(
                slug=f"combat_double_block_order_{index:03d}",
                name=f"T1 generated combat double-block ordering {index:03d}",
                setup=combat_setup(
                    60 + index,
                    [
                        object_setup(20_000 + index * 3, 0, "Battlefield"),
                        object_setup(20_001 + index * 3, 1, "Battlefield"),
                        object_setup(20_002 + index * 3, 1, "Battlefield"),
                    ],
                ),
                script=script,
                expect=expect_block(
                    zones=[
                        zone_count("Battlefield", 1),
                        zone_count("Graveyard", 2, 1),
                    ],
                ),
            )
        )

    for index in range(10):
        script = [
            creature_step(0, 5, 5, '["trample", "deathtouch"]'),
            creature_step(1, 3, 3),
            *enter_declare_attackers(),
            declare_attackers(0),
            step("advance_step"),
            declare_blockers([(1, 0)]),
            step("advance_step"),
            assign_combat_damage(
                [
                    combat_damage_request(
                        0,
                        [damage_to_object(1, 1), damage_to_player(1, 4)],
                    ),
                    combat_damage_request(1, [damage_to_object(0, 3)]),
                ]
            ),
        ]
        scenarios.append(
            Scenario(
                slug=f"combat_trample_deathtouch_{index:03d}",
                name=f"T1 generated combat trample plus deathtouch {index:03d}",
                setup=combat_setup(
                    80 + index,
                    [
                        object_setup(21_000 + index * 2, 0, "Battlefield"),
                        object_setup(21_001 + index * 2, 1, "Battlefield"),
                    ],
                ),
                script=script,
                expect=expect_block(
                    zones=[
                        zone_count("Battlefield", 1),
                        zone_count("Graveyard", 1, 1),
                    ],
                    players=["(player: 1, life: 16)"],
                ),
            )
        )

    for index in range(10):
        power = 2 + (index % 3)
        script = [
            creature_step(0, power, 3, '["lifelink"]'),
            *enter_declare_attackers(),
            declare_attackers(0),
            step("advance_step"),
            declare_blockers([]),
            step("advance_step"),
        ]
        active_life = 20 + power
        if index == 0:
            script.append(step("set_life", player=0, life=0))
            active_life = power
        script.append(
            assign_combat_damage(
                [combat_damage_request(0, [damage_to_player(1, power)])]
            )
        )
        scenarios.append(
            Scenario(
                slug=f"combat_lifelink_unblocked_{index:03d}",
                name=f"T1 generated combat lifelink damage {index:03d}",
                setup=combat_setup(
                    100 + index,
                    [object_setup(22_000 + index, 0, "Battlefield")],
                ),
                script=script,
                expect=expect_block(
                    zones=[zone_count("Battlefield", 1)],
                    players=[
                        f"(player: 0, life: {active_life})",
                        f"(player: 1, life: {20 - power})",
                    ],
                ),
            )
        )

    return scenarios


def cleanup_scenarios() -> list[Scenario]:
    scenarios = []
    for index in range(10):
        active = index % 2
        scripts = [step("start_turn", player=active)]
        scripts.extend(step("advance_step") for _ in range(8))
        if index % 2 == 0:
            scripts.append(step("request_cleanup_priority"))
        scripts.append(step("advance_step"))
        priority = active if index % 2 == 0 else "none"
        scenarios.append(
            Scenario(
                slug=f"cleanup_{index:03d}",
                name=f"T1 generated cleanup priority {index:03d}",
                setup=setup_block(libraries=[library(active, 16_000 + index * 10, 3)]),
                script=scripts,
                expect=expect_block(
                    active_player=active,
                    priority_player=priority,
                    current_step="Cleanup",
                ),
            )
        )
    return scenarios


def generated_scenarios() -> list[Scenario]:
    scenarios = []
    scenarios.extend(setup_zone_scenarios())
    scenarios.extend(zone_move_scenarios())
    scenarios.extend(opening_hand_scenarios())
    scenarios.extend(turn_priority_scenarios())
    scenarios.extend(mana_scenarios())
    scenarios.extend(life_scenarios())
    scenarios.extend(poison_scenarios())
    scenarios.extend(creature_sba_scenarios())
    scenarios.extend(combat_scenarios())
    scenarios.extend(cleanup_scenarios())
    return scenarios


def manual_oracle_count() -> int:
    count = 0
    for path in ORACLE_DIR.rglob("*.ron"):
        try:
            path.relative_to(GENERATED_DIR)
        except ValueError:
            count += 1
    return count


def main() -> None:
    manual_count = manual_oracle_count()
    required = TARGET_TOTAL - manual_count
    scenarios = generated_scenarios()
    if required < 0:
        raise SystemExit(f"manual oracle count {manual_count} exceeds target {TARGET_TOTAL}")
    if len(scenarios) != required:
        raise SystemExit(
            f"generator produced {len(scenarios)} scenarios, expected {required} "
            f"for {manual_count} manual oracles"
        )
    GENERATED_DIR.mkdir(parents=True, exist_ok=True)
    for stale in GENERATED_DIR.glob("*.ron"):
        stale.unlink()
    for index, scenario in enumerate(scenarios, start=1):
        path = GENERATED_DIR / f"t1_14_{index:03d}_{scenario.slug}.ron"
        path.write_text(render_scenario(scenario), encoding="utf-8")
    print(
        f"wrote {len(scenarios)} generated scenarios; "
        f"{manual_count + len(scenarios)} oracle scenarios total"
    )


if __name__ == "__main__":
    main()
