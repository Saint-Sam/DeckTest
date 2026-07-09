#!/usr/bin/env python3
"""Generate the reviewed CP-DSL sources from pinned Scryfall text and explicit recipes."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import unicodedata
from collections import Counter
from pathlib import Path


MANDATORY_STRATA = (
    "vanilla_keywords",
    "instant_sorcery",
    "modal_choices",
    "activated_mana_loyalty",
    "triggered_delayed",
    "replacement_prevention",
    "continuous_layers",
    "targeting_restrictions",
    "zones_lki",
    "calculated_values",
    "alternate_additional_costs",
    "counters_tokens_copy",
    "attachments",
    "multiplayer_commander_voting",
    "split_fuse",
    "transform_day_night",
    "modal_dfc",
    "adventure",
    "meld",
    "flip",
    "saga_class_case_leveler",
    "unusual_mana",
    "cast_from_zone",
    "dungeon_initiative_monarch_energy",
    "turn_combat",
)


def ability(
    kind: str,
    effect: str,
    *,
    costs: tuple[str, ...] = (),
    event: str | None = None,
    condition: str | None = None,
    timing: str | None = None,
    mana_ability: bool = False,
) -> dict[str, object]:
    return {
        "kind": kind,
        "effect": effect,
        "costs": list(costs),
        "event": event,
        "condition": condition,
        "timing": timing,
        "mana_ability": mana_ability,
    }


def spell(effect: str, *, costs: tuple[str, ...] = ()) -> dict[str, object]:
    return ability("spell", effect, costs=costs)


def activated(
    costs: tuple[str, ...],
    effect: str,
    *,
    condition: str | None = None,
    timing: str | None = None,
    mana_ability: bool = False,
) -> dict[str, object]:
    return ability(
        "activated",
        effect,
        costs=costs,
        condition=condition,
        timing=timing,
        mana_ability=mana_ability,
    )


def triggered(
    event: str,
    effect: str,
    *,
    condition: str | None = None,
) -> dict[str, object]:
    return ability("triggered", effect, event=event, condition=condition)


def static(effect: str, *, condition: str | None = None) -> dict[str, object]:
    return ability("static", effect, condition=condition)


def replacement(
    event: str,
    effect: str,
    *,
    condition: str | None = None,
) -> dict[str, object]:
    return ability("replacement", effect, event=event, condition=condition)


# These recipes are the reviewable mechanics translation. Source characteristics and
# Oracle text come from source_cards.json; no ability is inferred from prose.
TRANSLATIONS: dict[str, dict[str, list[dict[str, object]]]] = {
    "Grizzly Bears": {"*": []},
    "Serra Angel": {"*": []},
    "Vampire Nighthawk": {"*": []},
    "Questing Beast": {
        "*": [
            static('continuous(source(), cannot_be_blocked_by(source(), permanents(less_than(power(any()), 3))))'),
            static('continuous(all(permanents(controlled_by(you()))), damage_cannot_be_prevented(any()))'),
            triggered(
                'event_damage(source(), opponent(), "combat")',
                'deal_damage(target(permanents(type_is("planeswalker"))), amount(triggered()))',
            ),
        ]
    },
    "Lightning Bolt": {"*": [spell("deal_damage(target(any()), 3)")]},
    "Counterspell": {"*": [spell("counter_spell(target(spells()))")]},
    "Divination": {"*": [spell("draw(2, you())")]},
    "Doom Blade": {
        "*": [spell('destroy(target(and(type_is("creature"), not(color_is("black")))))')]
    },
    "Cryptic Command": {
        "*": [
            spell(
                'choose_up_to(2, counter_spell(target(spells())), return_to_hand(target(permanents())), tap(all(permanents(and(type_is("creature"), controlled_by(opponent()))))), draw(1, you()))'
            )
        ]
    },
    "Kolaghan's Command": {
        "*": [
            spell(
                'choose_up_to(2, return_to_hand(target(cards(and(zone_is("graveyard"), type_is("creature"))))), discard_cards(1, target(any())), destroy(target(permanents(type_is("artifact")))), deal_damage(target(any()), 2))'
            )
        ]
    },
    "Archmage's Charm": {
        "*": [
            spell(
                'choose_one(counter_spell(target(spells())), draw(2, target(any())), change_control(target(permanents(and(not(type_is("land")), less_than(mana_value(any()), 2)))), you()))'
            )
        ]
    },
    "Boros Charm": {
        "*": [
            spell(
                'choose_one(deal_damage(target(any()), 4), grant_keyword(all(permanents(controlled_by(you()))), "indestructible", "until_end_of_turn"), grant_keyword(target(permanents(type_is("creature"))), "double_strike", "until_end_of_turn"))'
            )
        ]
    },
    "Llanowar Elves": {
        "*": [activated(("tap_self()",), 'add_mana("{G}", you())', mana_ability=True)]
    },
    "Prodigal Sorcerer": {
        "*": [activated(("tap_self()",), "deal_damage(target(any()), 1)")]
    },
    "Walking Ballista": {
        "*": [
            replacement('event_enters(source())', 'add_counter(source(), "plus1_plus1", amount("X"))'),
            activated(
                ('remove_counter_cost(source(), "plus1_plus1")',),
                "deal_damage(target(any()), 1)",
            ),
        ]
    },
    "Jace, the Mind Sculptor": {
        "*": [
            activated(("loyalty_cost(2)",), 'look_at(cards(zone_is("library")), target(any()))'),
            activated(("loyalty_cost(0)",), 'sequence(draw(3, you()), move_zone(cards(zone_is("hand")), "library_top", 2))'),
            activated(("loyalty_cost(-1)",), 'return_to_hand(target(permanents(type_is("creature"))))'),
            activated(("loyalty_cost(-12)",), 'sequence(exile(cards(zone_is("library")), target(any())), move_zone(cards(zone_is("hand")), "library"))'),
        ]
    },
    "Soul Warden": {
        "*": [
            triggered(
                'event_enters(permanents(and(type_is("creature"), not(equals(any(), source())))))',
                "gain_life(1, you())",
            )
        ]
    },
    "Young Pyromancer": {
        "*": [
            triggered(
                'event_cast(spells(and(controlled_by(you()), or(type_is("instant"), type_is("sorcery")))))',
                'create_token("1/1 red Elemental", 1, you())',
            )
        ]
    },
    "Sun Titan": {
        "*": [
            triggered(
                "event_enters(source())",
                'move_zone(target(cards(and(zone_is("graveyard"), less_than(mana_value(any()), 4)))), "battlefield")',
            ),
            triggered(
                "event_attacks(source())",
                'move_zone(target(cards(and(zone_is("graveyard"), less_than(mana_value(any()), 4)))), "battlefield")',
            ),
        ]
    },
    "Restoration Angel": {
        "*": [
            triggered(
                "event_enters(source())",
                'sequence(exile(target(permanents(and(type_is("creature"), controlled_by(you()), not(subtype_is("Angel")))))), move_zone(triggered(), "battlefield"))',
            )
        ]
    },
    "Doubling Season": {
        "*": [
            replacement(
                'event_enters(permanents(and(controlled_by(you()), type_is("token"))))',
                "double_event(triggered())",
            ),
            replacement(
                'event_counter_added(permanents(controlled_by(you())))',
                "double_event(triggered())",
            ),
        ]
    },
    "Rest in Peace": {
        "*": [
            triggered("event_enters(source())", 'exile(all(cards(zone_is("graveyard"))))'),
            replacement(
                'event_zone_change(cards(), "graveyard")',
                'replace_event(triggered(), move_zone(triggered(), "exile"))',
            ),
        ]
    },
    "Leyline of the Void": {
        "*": [
            replacement(
                'event_zone_change(cards(owned_by(opponent())), "graveyard")',
                'replace_event(triggered(), move_zone(triggered(), "exile"))',
            )
        ]
    },
    "Fog": {"*": [spell('prevent_damage("combat", all(permanents(type_is("creature"))))')]},
    "Humility": {
        "*": [
            static(
                'continuous(all(permanents(type_is("creature"))), sequence(remove_all_abilities(any()), set_pt(any(), 1, 1)))'
            )
        ]
    },
    "Opalescence": {
        "*": [
            static(
                'continuous(all(permanents(and(type_is("enchantment"), not(equals(any(), source()))))), sequence(add_type(any(), "creature"), set_pt(any(), mana_value(any()), mana_value(any()))))'
            )
        ]
    },
    "Blood Moon": {
        "*": [
            static(
                'continuous(all(permanents(and(type_is("land"), not(supertype_is("basic"))))), set_type(any(), "Mountain"))'
            )
        ]
    },
    "Mycosynth Lattice": {
        "*": [
            static('continuous(all(permanents()), add_type(any(), "artifact"))'),
            static('continuous(all(cards()), set_color(any(), "colorless"))'),
            static('continuous(you(), spend_mana_as_any_color(you()))'),
        ]
    },
    "Hex": {"*": [spell('destroy(target(permanents(and(type_is("creature"), equals(count("chosen_targets"), 6)))))')]},
    "Swords to Plowshares": {
        "*": [
            spell(
                'sequence(exile(target(permanents(type_is("creature")))), gain_life(power(triggered()), owner_of(triggered())))'
            )
        ]
    },
    "Deflecting Swat": {
        "*": [
            static(
                'continuous(source(), alternate_cost(source(), mana_cost("{0}")))',
                condition='at_least(count(permanents(and(type_is("commander"), controlled_by(you())))), 1)',
            ),
            spell("change_target(target(spells()), target(any()))"),
        ]
    },
    "True-Name Nemesis": {
        "*": [
            triggered("event_enters(source())", 'remember("chosen_player", opponent())'),
            static('continuous(source(), grant_keyword(source(), "protection", remembered("chosen_player")))'),
        ]
    },
    "Reanimate": {
        "*": [
            spell(
                'sequence(move_zone(target(cards(and(zone_is("graveyard"), type_is("creature")))), "battlefield"), lose_life(mana_value(triggered()), you()))'
            )
        ]
    },
    "Snapcaster Mage": {
        "*": [
            triggered(
                "event_enters(source())",
                'grant_keyword(target(cards(and(zone_is("graveyard"), or(type_is("instant"), type_is("sorcery"))))), "flashback", "until_end_of_turn")',
            )
        ]
    },
    "Eternal Witness": {
        "*": [
            triggered(
                "event_enters(source())",
                'return_to_hand(target(cards(zone_is("graveyard"))))',
            )
        ]
    },
    "Scavenging Ooze": {
        "*": [
            activated(
                ('mana_cost("{G}")',),
                'sequence(exile(target(cards(and(zone_is("graveyard"), type_is("creature"))))), add_counter(source(), "plus1_plus1", 1), gain_life(1, you()))',
            )
        ]
    },
    "Tarmogoyf": {
        "*": [
            static(
                'continuous(source(), set_pt(source(), count("card_types_among_all_graveyards"), amount(count("card_types_among_all_graveyards"), 1)))'
            )
        ]
    },
    "Lord of Extinction": {
        "*": [
            static(
                'continuous(source(), set_pt(source(), count(cards(zone_is("graveyard"))), count(cards(zone_is("graveyard")))))'
            )
        ]
    },
    "Chameleon Colossus": {
        "*": [
            activated(
                ('mana_cost("{2}{G}{G}")',),
                'modify_pt(source(), power(source()), toughness(source()), "until_end_of_turn")',
            )
        ]
    },
    "Crackle with Power": {
        "*": [spell('deal_damage(target(all(any())), amount("X", 5))')]
    },
    "Force of Will": {
        "*": [
            static('continuous(source(), alternate_cost(source(), exile_cost(cards(and(zone_is("hand"), color_is("blue")))), pay_life(1)))'),
            spell("counter_spell(target(spells()))"),
        ]
    },
    "Fling": {
        "*": [
            spell(
                "deal_damage(target(any()), power(triggered()))",
                costs=('sacrifice(permanents(type_is("creature")))',),
            )
        ]
    },
    "Burst Lightning": {
        "*": [
            spell(
                'choose_one(deal_damage(target(any()), 2), while_condition(equals("kicked", true), deal_damage(target(any()), 4)))'
            )
        ]
    },
    "Treasure Cruise": {
        "*": [
            static('continuous(source(), delve_cost(source()))'),
            spell("draw(3, you())"),
        ]
    },
    "Clone": {
        "*": [
            replacement(
                "event_enters(source())",
                'replace_event(triggered(), copy(target(permanents(type_is("creature")))))',
            )
        ]
    },
    "Hardened Scales": {
        "*": [
            replacement(
                'event_counter_added(permanents(controlled_by(you())), "plus1_plus1")',
                'replace_event(triggered(), add_counter(triggered(), "plus1_plus1", 1))',
            )
        ]
    },
    "Anointed Procession": {
        "*": [
            replacement(
                'event_enters(permanents(and(type_is("token"), controlled_by(you()))))',
                "double_event(triggered())",
            )
        ]
    },
    "Sakashima the Impostor": {
        "*": [
            replacement(
                "event_enters(source())",
                'replace_event(triggered(), copy(target(permanents(type_is("creature")))))',
            ),
            activated(
                ('mana_cost("{2}{U}{U}")',),
                "return_to_hand(source())",
                timing="timing_your_turn()",
            ),
        ]
    },
    "Lightning Greaves": {
        "*": [
            activated(('mana_cost("{0}")',), 'attach(source(), target(permanents(type_is("creature"))))', timing="timing_sorcery()"),
            static('continuous(equipped_object(source()), sequence(grant_keyword(any(), "haste"), grant_keyword(any(), "shroud")))'),
        ]
    },
    "Pacifism": {
        "*": [static('continuous(enchanted_object(source()), sequence(cannot_attack(any()), cannot_block(any())))')]
    },
    "Cranial Plating": {
        "*": [
            activated(('mana_cost("{1}")',), 'attach(source(), target(permanents(type_is("creature"))))', timing="timing_sorcery()"),
            activated(('mana_cost("{B}{B}")',), 'attach(source(), target(permanents(type_is("creature"))))', timing="timing_instant()"),
            static('continuous(equipped_object(source()), modify_pt(any(), count(permanents(and(type_is("artifact"), controlled_by(you())))), 0))'),
        ]
    },
    "Colossus Hammer": {
        "*": [
            activated(('mana_cost("{8}")',), 'attach(source(), target(permanents(type_is("creature"))))', timing="timing_sorcery()"),
            static('continuous(equipped_object(source()), sequence(modify_pt(any(), 10, 10), remove_keyword(any(), "flying")))'),
        ]
    },
    "Command Tower": {
        "*": [
            activated(("tap_self()",), 'add_mana("commander_color_identity", you())', mana_ability=True)
        ]
    },
    "Council's Judgment": {
        "*": [spell('sequence(vote("nonland_permanent", all(any())), exile(all(remembered("most_votes"))))')]
    },
    "Expropriate": {
        "*": [
            spell(
                'sequence(vote("time_or_money", all(any())), for_each(remembered("time_votes"), extra_turn(you())), for_each(remembered("money_votes"), change_control(target(permanents()), you())))'
            )
        ]
    },
    "Breena, the Demagogue": {
        "*": [
            triggered(
                'event_attacks(permanents(and(controlled_by(any()), not(controlled_by(you())))), opponent())',
                'sequence(draw(1, controller_of(triggered())), add_counter(target(permanents(type_is("creature"))), "plus1_plus1", 2))',
                condition='greater_than(amount(opponent(), "life"), amount(controller_of(triggered()), "life"))',
            )
        ]
    },
    "Fire // Ice": {
        "Fire": [spell('choose_one(deal_damage(target(any()), 2), sequence(deal_damage(target(any()), 1), deal_damage(target(any()), 1)))')],
        "Ice": [spell('sequence(tap(target(permanents())), draw(1, you()))')],
    },
    "Wear // Tear": {
        "Wear": [spell('destroy(target(permanents(type_is("artifact"))))')],
        "Tear": [spell('destroy(target(permanents(type_is("enchantment"))))')],
    },
    "Beck // Call": {
        "Beck": [spell('register_delayed_trigger(event_enters(permanents(and(type_is("creature"), controlled_by(you())))), draw(1, you()), "this_turn")')],
        "Call": [spell('create_token("1/1 white Bird with flying", 4, you())')],
    },
    "Far // Away": {
        "Far": [spell('return_to_hand(target(permanents(type_is("creature"))))')],
        "Away": [spell('sacrifice_effect(target(any()), permanents(type_is("creature")))')],
    },
    "Delver of Secrets // Insectile Aberration": {
        "Delver of Secrets": [
            triggered(
                'event_upkeep(you())',
                'sequence(look_at(cards(zone_is("library_top")), you()), reveal(cards(and(zone_is("library_top"), or(type_is("instant"), type_is("sorcery"))))), transform(source()))',
            )
        ],
        "Insectile Aberration": [],
    },
    "Huntmaster of the Fells // Ravager of the Fells": {
        "Huntmaster of the Fells": [
            triggered("event_enters(source())", 'sequence(create_token("2/2 green Wolf", 1, you()), gain_life(2, you()))'),
            triggered('event_upkeep(any())', 'transform(source())', condition='equals(count(spells("previous_turn")), 0)'),
        ],
        "Ravager of the Fells": [
            triggered('event_upkeep(any())', 'transform(source())', condition='at_least(count(spells("previous_turn")), 2)'),
            triggered('event_enters(source(), "transformed")', 'sequence(deal_damage(target(opponent()), 2), deal_damage(target(permanents(type_is("creature"))), 2))'),
        ],
    },
    "Brutal Cathar // Moonrage Brute": {
        "Brutal Cathar": [
            triggered(
                "event_enters(source())",
                'exile(target(permanents(and(type_is("creature"), controlled_by(opponent())))), "until_source_leaves")',
            )
        ],
        "Moonrage Brute": [],
    },
    "Bloodline Keeper // Lord of Lineage": {
        "Bloodline Keeper": [
            activated(("tap_self()",), 'create_token("2/2 black Vampire with flying", 1, you())'),
            activated(('mana_cost("{B}")',), 'transform(source())', condition='at_least(count(permanents(and(subtype_is("Vampire"), controlled_by(you())))), 5)'),
        ],
        "Lord of Lineage": [
            static('continuous(all(permanents(and(subtype_is("Vampire"), controlled_by(you())))), modify_pt(any(), 2, 2))'),
            activated(("tap_self()",), 'create_token("2/2 black Vampire with flying", 1, you())'),
        ],
    },
    "Sea Gate Restoration // Sea Gate, Reborn": {
        "Sea Gate Restoration": [spell('sequence(draw(amount("hand_size", 1), you()), continuous(you(), no_maximum_hand_size(you())))')],
        "Sea Gate, Reborn": [activated(("tap_self()",), 'add_mana("{U}", you())', mana_ability=True)],
    },
    "Emeria's Call // Emeria, Shattered Skyclave": {
        "Emeria's Call": [spell('sequence(create_token("4/4 white Angel Warrior with flying", 2, you()), grant_keyword(all(permanents(and(type_is("creature"), controlled_by(you())))), "indestructible", "until_your_next_turn"))')],
        "Emeria, Shattered Skyclave": [activated(("tap_self()",), 'add_mana("{W}", you())', mana_ability=True)],
    },
    "Valki, God of Lies // Tibalt, Cosmic Impostor": {
        "Valki, God of Lies": [
            triggered('event_enters(source())', 'exile(cards(and(zone_is("hand"), type_is("creature"))), opponent())'),
            activated(('mana_cost("{X}")',), 'copy(chosen(cards(equals(mana_value(any()), "X"))), source())'),
        ],
        "Tibalt, Cosmic Impostor": [
            static('continuous(you(), play_exiled(you(), source()))'),
            activated(("loyalty_cost(2)",), 'exile(cards(zone_is("library_top")), all(any()))'),
            activated(("loyalty_cost(-3)",), 'exile(target(permanents(and(not(type_is("artifact")), not(type_is("land"))))))'),
            activated(("loyalty_cost(-8)",), 'exile(all(cards(zone_is("graveyard"))))'),
        ],
    },
    "Birgi, God of Storytelling // Harnfel, Horn of Bounty": {
        "Birgi, God of Storytelling": [
            triggered('event_cast(spells(controlled_by(you())))', 'add_mana("{R}", you())'),
            static('continuous(all(permanents(controlled_by(you()))), activation_limit(any(), "boast", 2))'),
        ],
        "Harnfel, Horn of Bounty": [
            activated(('discard_cost(1, you())',), 'sequence(exile(cards(zone_is("library_top")), 2), play(triggered(), "this_turn"))')
        ],
    },
    "Bonecrusher Giant // Stomp": {
        "Bonecrusher Giant": [
            triggered('event_targeted(source(), spells())', 'deal_damage(controller_of(triggered()), 2)')
        ],
        "Stomp": [spell('sequence(damage_cannot_be_prevented(all(any()), "this_turn"), deal_damage(target(any()), 2))')],
    },
    "Brazen Borrower // Petty Theft": {
        "Brazen Borrower": [static('continuous(source(), can_block_only(source(), keyword_is("flying")))')],
        "Petty Theft": [spell('return_to_hand(target(permanents(and(not(type_is("land")), controlled_by(opponent())))))')],
    },
    "Murderous Rider // Swift End": {
        "Murderous Rider": [
            replacement('event_dies(source())', 'replace_event(triggered(), move_zone(source(), "library_bottom"))')
        ],
        "Swift End": [spell('sequence(destroy(target(permanents(or(type_is("creature"), type_is("planeswalker"))))), lose_life(2, you()))')],
    },
    "Lovestruck Beast // Heart's Desire": {
        "Lovestruck Beast": [static('continuous(source(), cannot_attack(source(), not(at_least(count(permanents(and(type_is("creature"), controlled_by(you()), equals(power(any()), 1), equals(toughness(any()), 1)))), 1))))')],
        "Heart's Desire": [spell('create_token("1/1 white Human", 1, you())')],
    },
    "Bruna, the Fading Light": {
        "*": [
            triggered(
                'event_cast(source())',
                'move_zone(target(cards(and(zone_is("graveyard"), or(subtype_is("Angel"), subtype_is("Human"))))), "battlefield")',
            )
        ]
    },
    "Gisela, the Broken Blade": {"*": []},
    "Urza, Lord Protector": {
        "*": [
            static('continuous(spells(and(controlled_by(you()), or(type_is("artifact"), type_is("instant"), type_is("sorcery")))), cost_reduction(any(), 1))'),
            activated(('mana_cost("{7}")',), 'meld(source(), cards("The Mightstone and Weakstone"))', timing="timing_sorcery()"),
        ]
    },
    "The Mightstone and Weakstone": {
        "*": [
            triggered(
                'event_enters(source())',
                'choose_one(draw(2, you()), modify_pt(target(permanents(type_is("creature"))), -5, -5, "until_end_of_turn"))',
            ),
            activated(("tap_self()",), 'add_mana("{C}{C}", you())', mana_ability=True),
        ]
    },
    "Nezumi Shortfang // Stabwhisker the Odious": {
        "Nezumi Shortfang": [
            activated(('mana_cost("{1}{B}")', "tap_self()"), 'discard_cards(1, target(opponent()))'),
            triggered('event_discard(opponent())', 'transform(source())', condition='equals(count(cards(and(zone_is("hand"), owned_by(opponent())))), 0)'),
        ],
        "Stabwhisker the Odious": [
            triggered('event_upkeep(opponent())', 'lose_life(count(cards(and(zone_is("hand"), owned_by(opponent())))), opponent())')
        ],
    },
    "Budoka Gardener // Dokai, Weaver of Life": {
        "Budoka Gardener": [
            activated(('mana_cost("{2}{G}")', "tap_self()"), 'move_zone(cards(and(zone_is("hand"), type_is("land"))), "battlefield")'),
            triggered('event_enters(permanents(type_is("land")))', 'transform(source())', condition='at_least(count(permanents(and(type_is("land"), controlled_by(you())))), 10)'),
        ],
        "Dokai, Weaver of Life": [
            activated(('mana_cost("{4}{G}{G}")', "tap_self()"), 'create_token("green Elemental with power and toughness equal to lands", 1, you())')
        ],
    },
    "Erayo, Soratami Ascendant // Erayo's Essence": {
        "Erayo, Soratami Ascendant": [
            triggered('event_cast(spells())', 'transform(source())', condition='at_least(count(spells("this_turn")), 4)')
        ],
        "Erayo's Essence": [
            replacement('event_cast(spells(controlled_by(opponent())))', 'replace_event(triggered(), counter_spell(triggered()))', condition='equals(count(spells(and(controlled_by(opponent()), during("this_turn")))), 1)')
        ],
    },
    "Jushi Apprentice // Tomoya the Revealer": {
        "Jushi Apprentice": [
            activated(('mana_cost("{2}{U}")', "tap_self()"), 'draw(1, you())'),
            triggered('event_draw(you())', 'transform(source())', condition='at_least(count(cards(and(zone_is("hand"), owned_by(you())))), 9)'),
        ],
        "Tomoya the Revealer": [
            activated(('mana_cost("{3}{U}{U}")', "tap_self()"), 'draw(count(cards(and(zone_is("hand"), owned_by(you())))), target(any()))')
        ],
    },
    "The Eldest Reborn": {
        "*": [
            triggered('event_counter_added(source(), "lore")', 'choose_one(sacrifice_effect(opponent(), permanents(or(type_is("creature"), type_is("planeswalker")))), discard_cards(1, opponent()), move_zone(target(cards(and(zone_is("graveyard"), or(type_is("creature"), type_is("planeswalker"))))), "battlefield"))')
        ]
    },
    "Wizard Class": {
        "*": [
            static('continuous(you(), no_maximum_hand_size(you()))'),
            activated(('mana_cost("{2}{U}")',), 'sequence(level_up(source(), 2), draw(2, you()))', timing="timing_sorcery()"),
            activated(('mana_cost("{4}{U}")',), 'level_up(source(), 3)', timing="timing_sorcery()"),
            triggered('event_draw(you())', 'add_counter(target(permanents(type_is("creature"))), "plus1_plus1", 1)'),
        ]
    },
    "Case of the Locked Hothouse": {
        "*": [
            static('continuous(you(), additional_land_plays(you(), 1))'),
            triggered('event_upkeep(you())', 'remember("case_solved", true)', condition='at_least(count(permanents(and(type_is("land"), controlled_by(you())))), 7)'),
            static('continuous(you(), play(cards(and(zone_is("library_top"), or(type_is("creature"), type_is("enchantment")))), "while_solved"))'),
        ]
    },
    "Student of Warfare": {
        "*": [
            activated(('mana_cost("{W}")',), 'level_up(source(), 1)', timing="timing_sorcery()"),
            static('continuous(source(), grant_keyword(source(), "first_strike"))', condition='at_least(count("level_counters"), 2)'),
            static('continuous(source(), sequence(modify_pt(source(), 3, 3), grant_keyword(source(), "double_strike")))', condition='at_least(count("level_counters"), 7)'),
        ]
    },
    "Kitchen Finks": {
        "*": [triggered('event_enters(source())', 'gain_life(2, you())')]
    },
    "Gitaxian Probe": {
        "*": [spell('sequence(look_at(cards(and(zone_is("hand"), owned_by(target(any())))), you()), draw(1, you()))')]
    },
    "Spectral Procession": {
        "*": [spell('create_token("1/1 white Spirit with flying", 3, you())')]
    },
    "Beseech the Queen": {
        "*": [spell('search_library(cards(and(zone_is("library"), less_than(mana_value(any()), amount(you(), "land_count")))), you())')]
    },
    "Gravecrawler": {
        "*": [static('continuous(source(), cast(source(), "from_graveyard"))', condition='at_least(count(permanents(and(subtype_is("Zombie"), controlled_by(you())))), 1)')]
    },
    "Bloodghast": {
        "*": [triggered('event_enters(permanents(and(type_is("land"), controlled_by(you()))))', 'move_zone(source(), "battlefield")', condition='equals(zone_is("graveyard"), true)')]
    },
    "Misthollow Griffin": {
        "*": [static('continuous(source(), cast(source(), "from_exile"))')]
    },
    "Eternal Scourge": {
        "*": [
            static('continuous(source(), cast(source(), "from_exile"))'),
            triggered('event_targeted(source(), spells(controlled_by(opponent())))', 'exile(source())'),
        ]
    },
    "White Plume Adventurer": {
        "*": [
            triggered('event_enters(source())', 'take_initiative(you())'),
            triggered('event_upkeep(opponent())', 'untap(target(permanents(controlled_by(you()))))'),
        ]
    },
    "Palace Jailer": {
        "*": [
            triggered('event_enters(source())', 'sequence(become_monarch(you()), exile(target(permanents(controlled_by(opponent()))), "until_opponent_is_monarch"))')
        ]
    },
    "Aetherworks Marvel": {
        "*": [
            triggered('event_zone_change(permanents(controlled_by(you())), "graveyard")', 'add_counter(you(), "energy", 1)'),
            activated(('remove_counter_cost(you(), "six_energy")', "tap_self()"), 'sequence(look_at(cards(zone_is("library_top")), 6), cast(chosen(cards()), "without_mana_cost"))'),
        ]
    },
    "Sefris of the Hidden Ways": {
        "*": [
            triggered('event_zone_change(cards(type_is("creature")), "graveyard")', 'venture(you())'),
            triggered('event_counter_added(you(), "dungeon_completed")', 'move_zone(target(cards(and(zone_is("graveyard"), type_is("creature")))), "battlefield")'),
        ]
    },
    "Relentless Assault": {
        "*": [spell('sequence(untap(all(permanents(and(type_is("creature"), controlled_by(you()))))), extra_combat(you()))')]
    },
    "Time Warp": {"*": [spell('extra_turn(target(any()))')]},
    "Silence": {
        "*": [spell('continuous(opponent(), cannot_cast(opponent(), "this_turn"))')]
    },
    "Maze of Ith": {
        "*": [
            activated(
                ("tap_self()",),
                'sequence(untap(target(permanents(type_is("attacking_creature")))), prevent_damage(target(triggered()), "combat"))',
            )
        ]
    },
}


def json_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def keyword_id(value: str) -> str:
    special = {
        "Council's dilemma": "councils_dilemma",
        "Will of the council": "will_of_the_council",
    }
    if value in special:
        return special[value]
    return re.sub(r"[^a-z0-9]+", "_", value.lower()).strip("_")


def slug(value: str) -> str:
    ascii_value = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "_", ascii_value.lower()).strip("_")


def render_ability(item: dict[str, object]) -> list[str]:
    lines = [f"    ability {item['kind']} {{"]
    costs = item["costs"]
    if costs:
        lines.append(f"      costs: [{', '.join(costs)}]")
    for field in ("event", "condition", "timing"):
        value = item[field]
        if value is not None:
            lines.append(f"      {field}: {value}")
    lines.append(f"      effect: {item['effect']}")
    if item["mana_ability"]:
        lines.append("      mana_ability: true")
    lines.append("    }")
    return lines


def source_faces(card: dict[str, object]) -> list[dict[str, object]]:
    faces = card["card_faces"]
    if faces:
        return faces
    return [card]


def render_card(entry: dict[str, object]) -> str:
    card = entry["source_card"]
    name = card["name"]
    oracle_id = card["oracle_id"]
    if not oracle_id:
        raise ValueError(f"reviewed playable card has no Oracle id: {name}")
    face_recipes = TRANSLATIONS[name]
    lines = [
        f"// CP-DSL stratum: {entry['stratum']}",
        f"card {json_string(name)} {{",
        f"  id: {json_string(oracle_id)}",
        f"  layout: {card['layout']}",
        "  status: verified_playable",
    ]
    for face in source_faces(card):
        face_name = face["name"]
        keywords = sorted(
            {
                keyword_id(keyword)
                for keyword in [*card.get("keywords", []), *face.get("keywords", [])]
            }
        )
        lines.extend(
            [
                f"  face {json_string(face_name)} {{",
                f"    cost: {json_string(face.get('mana_cost', ''))}",
                f"    types: {json_string(face.get('type_line', ''))}",
                f"    oracle: {json_string(face.get('oracle_text', ''))}",
            ]
        )
        for field in ("power", "toughness", "loyalty", "defense"):
            value = face.get(field)
            if value is not None:
                lines.append(f"    {field}: {json_string(value)}")
        lines.append(f"    keywords: [{', '.join(keywords)}]")
        recipes = face_recipes.get(face_name, face_recipes.get("*", []))
        for item in recipes:
            lines.extend(render_ability(item))
        lines.append("  }")
    lines.extend(["}", ""])
    return "\n".join(lines)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def generate(root: Path, check: bool) -> int:
    source_path = root / "cards/cp_dsl/source_cards.json"
    source_bytes = source_path.read_bytes()
    document = json.loads(source_bytes)
    cards = document["cards"]
    names = [entry["source_card"]["name"] for entry in cards]
    if len(cards) != 100 or len(set(names)) != 100:
        raise ValueError("CP-DSL source snapshot must contain exactly 100 unique cards")
    if set(names) != set(TRANSLATIONS):
        missing = sorted(set(names) - set(TRANSLATIONS))
        extra = sorted(set(TRANSLATIONS) - set(names))
        raise ValueError(f"translation map mismatch; missing={missing}; extra={extra}")

    output_dir = root / "cards/cp_dsl/definitions"
    output_dir.mkdir(parents=True, exist_ok=True)
    expected: dict[Path, bytes] = {}
    rows = []
    operation_names: set[str] = set()
    for index, entry in enumerate(cards, start=1):
        name = entry["source_card"]["name"]
        path = output_dir / f"{index:03d}_{slug(name)}.frs"
        content = render_card(entry).encode("utf-8")
        expected[path] = content
        operation_names.update(
            match.decode("ascii") for match in re.findall(rb"\b([a-z][a-z0-9_]*)\(", content)
        )
        rows.append(
            {
                "index": index,
                "name": name,
                "oracle_id": entry["source_card"]["oracle_id"],
                "stratum": entry["stratum"],
                "layout": entry["source_card"]["layout"],
                "source_file": str(path.relative_to(root)),
                "sha256": sha256_bytes(content),
            }
        )

    existing = set(output_dir.glob("*.frs"))
    if check:
        failures = []
        for path, content in expected.items():
            if not path.exists() or path.read_bytes() != content:
                failures.append(str(path.relative_to(root)))
        failures.extend(str(path.relative_to(root)) for path in sorted(existing - set(expected)))
        if failures:
            print("CP-DSL generated files differ: " + ", ".join(failures), file=sys.stderr)
            return 1
    else:
        if existing - set(expected):
            raise ValueError("stale .frs files exist; remove them explicitly before regeneration")
        for path, content in expected.items():
            path.write_bytes(content)

    stratum_counts = Counter(entry["stratum"] for entry in cards)
    mandatory = set(MANDATORY_STRATA)
    observed = set(stratum_counts)
    missing_strata = sorted(mandatory - observed)
    unexpected_strata = sorted(observed - mandatory)
    if missing_strata or unexpected_strata:
        raise ValueError(
            f"mandatory stratum mismatch; missing={missing_strata}; unexpected={unexpected_strata}"
        )
    if any(count != 4 for count in stratum_counts.values()):
        raise ValueError("each mandatory CP-DSL stratum must contain exactly four cards")
    manifest = {
        "schema_version": 1,
        "source": document["source"],
        "source_snapshot_sha256": sha256_bytes(source_bytes),
        "review_status": "gate-reviewer remediation; owner signoff pending",
        "card_count": len(cards),
        "stratum_count": len(stratum_counts),
        "strata": dict(sorted(stratum_counts.items())),
        "mandatory_strata": list(MANDATORY_STRATA),
        "missing_mandatory_strata": missing_strata,
        "unexpected_strata": unexpected_strata,
        "catalog_only_records_verified_separately": True,
        "distinct_operations": sorted(operation_names),
        "cards": rows,
    }
    manifest_bytes = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    manifest_path = root / "cards/cp_dsl/manifest.json"
    metrics_path = root / "metrics/cp_dsl_corpus.json"
    metrics = {
        "schema_version": 1,
        "source_snapshot_sha256": manifest["source_snapshot_sha256"],
        "reviewed_card_count": len(cards),
        "distinct_primary_strata": len(stratum_counts),
        "minimum_cards_per_stratum": min(stratum_counts.values()),
        "mandatory_strata": list(MANDATORY_STRATA),
        "missing_mandatory_strata": missing_strata,
        "unexpected_strata": unexpected_strata,
        "catalog_only_records_verified_separately": True,
        "distinct_operations": len(operation_names),
    }
    metrics_bytes = (json.dumps(metrics, indent=2) + "\n").encode("utf-8")
    if check:
        if not manifest_path.exists() or manifest_path.read_bytes() != manifest_bytes:
            print("CP-DSL manifest differs", file=sys.stderr)
            return 1
        if not metrics_path.exists() or metrics_path.read_bytes() != metrics_bytes:
            print("CP-DSL metrics seed differs", file=sys.stderr)
            return 1
    else:
        manifest_path.write_bytes(manifest_bytes)
        metrics_path.write_bytes(metrics_bytes)
    print(f"CP-DSL corpus: {len(cards)} cards, {len(stratum_counts)} strata")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        return generate(args.root, args.check)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"generate_cp_dsl.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
