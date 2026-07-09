#!/usr/bin/env python3
"""Generate deterministic malformed CP-DSL diagnostics fixtures."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


BASE = '''card "Test Card" {
  id: "test-card"
  layout: normal
  status: verified_playable
  face "Test Card" {
    cost: "{1}{G}"
    types: "Creature — Elf"
    oracle: "Flying. {T}: Draw a card."
    power: "2"
    toughness: "2"
    keywords: [flying]
    ability activated {
      costs: [tap_self()]
      effect: draw(1, you())
    }
  }
}
'''


def changed(old: str, new: str) -> str:
    if BASE.count(old) != 1:
        raise ValueError(f"fixture mutation anchor is not unique: {old!r}")
    return BASE.replace(old, new)


def without(block: str) -> str:
    return changed(block, "")


def custom_face(layout: str = "normal", types: str = "Instant") -> str:
    return f'''card "Test Card" {{
  id: "test-card"
  layout: {layout}
  status: verified_playable
  face "Test Card" {{
    cost: "{{U}}"
    types: "{types}"
    oracle: "Test."
    keywords: []
  }}
}}
'''


CASES: list[tuple[str, str, str]] = [
    ("unknown_card_keyword", changed('card "Test Card"', 'crad "Test Card"'), ""),
    ("missing_card_name", changed('card "Test Card" {', 'card {'), ""),
    ("unknown_card_field", changed('  id: "test-card"', '  mystery: "x"\n  id: "test-card"'), ""),
    ("empty_id", changed('id: "test-card"', 'id: ""'), "invalid id"),
    ("id_with_spaces", changed('id: "test-card"', 'id: "test card"'), "invalid id"),
    ("missing_id", without('  id: "test-card"\n'), "missing `id:`"),
    ("duplicate_id", changed('  id: "test-card"', '  id: "test-card"\n  id: "other"'), "duplicate `id:`"),
    ("unknown_layout", changed('layout: normal', 'layout: impossible'), "unknown layout"),
    ("missing_layout", without('  layout: normal\n'), "missing `layout:`"),
    ("duplicate_layout", changed('  layout: normal', '  layout: normal\n  layout: normal'), "duplicate `layout:`"),
    ("invalid_status", changed('status: verified_playable', 'status: quarantined'), "cannot use status"),
    ("missing_status", without('  status: verified_playable\n'), "missing `status:`"),
    ("no_face", custom_face().replace('  face "Test Card" {\n    cost: "{U}"\n    types: "Instant"\n    oracle: "Test."\n    keywords: []\n  }\n', ''), "card has no faces"),
    ("empty_card_name", changed('card "Test Card"', 'card ""'), "card name is empty"),
    ("empty_face_name", changed('face "Test Card"', 'face ""'), "face name is empty"),
    ("missing_cost", without('    cost: "{1}{G}"\n'), "missing `cost:`"),
    ("duplicate_cost", changed('    cost: "{1}{G}"', '    cost: "{1}{G}"\n    cost: "{G}"'), "duplicate `cost:`"),
    ("mana_without_open", changed('cost: "{1}{G}"', 'cost: "1{G}"'), "must start"),
    ("mana_without_close", changed('cost: "{1}{G}"', 'cost: "{1{G}"'), "unknown mana symbol"),
    ("unknown_mana", changed('cost: "{1}{G}"', 'cost: "{Q}"'), "unknown mana symbol"),
    ("generic_mana_overflow", changed('cost: "{1}{G}"', 'cost: "{70000}"'), "unknown mana symbol"),
    ("missing_types", without('    types: "Creature — Elf"\n'), "missing `types:`"),
    ("duplicate_types", changed('    types: "Creature — Elf"', '    types: "Creature — Elf"\n    types: "Creature"'), "duplicate `types:`"),
    ("unknown_card_type", changed('types: "Creature — Elf"', 'types: "Contraption — Widget"'), "unknown card type"),
    ("no_card_type", changed('types: "Creature — Elf"', 'types: "Legendary — Elf"'), "no card type"),
    ("missing_oracle", without('    oracle: "Flying. {T}: Draw a card."\n'), "missing `oracle:`"),
    ("duplicate_oracle", changed('    oracle: "Flying. {T}: Draw a card."', '    oracle: "One"\n    oracle: "Two"'), "duplicate `oracle:`"),
    ("creature_missing_power", without('    power: "2"\n'), "requires both"),
    ("creature_missing_toughness", without('    toughness: "2"\n'), "requires both"),
    ("planeswalker_missing_loyalty", custom_face(types="Legendary Planeswalker — Test"), "requires `loyalty:`"),
    ("battle_missing_defense", custom_face(types="Battle — Siege"), "requires `defense:`"),
    ("unknown_keyword", changed('keywords: [flying]', 'keywords: [invented_keyword]'), "unknown keyword"),
    ("integer_keyword", changed('keywords: [flying]', 'keywords: [1]'), "keywords must"),
    ("uppercase_keyword_symbol", changed('keywords: [flying]', 'keywords: [Flying]'), "invalid symbol"),
    ("unknown_ability_kind", changed('ability activated', 'ability mystery'), "unknown ability kind"),
    ("activated_without_cost", without('      costs: [tap_self()]\n'), "requires at least one cost"),
    ("triggered_without_event", changed('ability activated', 'ability triggered'), "requires `event:`"),
    ("replacement_without_event", changed('ability activated', 'ability replacement'), "requires `event:`"),
    ("spell_marked_mana_ability", changed('ability activated {', 'ability spell {\n      mana_ability: true'), "only an activated"),
    ("missing_effect", without('      effect: draw(1, you())\n'), "missing `effect:`"),
    ("duplicate_effect", changed('      effect: draw(1, you())', '      effect: draw(1, you())\n      effect: draw(1, you())'), "duplicate `effect:`"),
    ("unknown_operation", changed('draw(1, you())', 'invent_rule(1, you())'), "unknown operation"),
    ("effect_wrong_category", changed('draw(1, you())', 'you()'), "expected Effect"),
    ("cost_wrong_category", changed('costs: [tap_self()]', 'costs: [draw(1, you())]'), "expected Cost"),
    ("event_wrong_category", changed('ability activated {', 'ability triggered {\n      event: draw(1, you())'), "expected Event"),
    ("condition_wrong_category", changed('      effect: draw(1, you())', '      condition: draw(1, you())\n      effect: draw(1, you())'), "expected Predicate"),
    ("timing_wrong_category", changed('      effect: draw(1, you())', '      timing: draw(1, you())\n      effect: draw(1, you())'), "expected Timing"),
    ("operation_too_few_args", changed('draw(1, you())', 'draw()'), "received 0 argument"),
    ("operation_too_many_args", changed('draw(1, you())', 'tap(source(), you())'), "received 2 argument"),
    ("invalid_uppercase_symbol", changed('draw(1, you())', 'draw(Player, you())'), "invalid symbol"),
    ("split_with_one_face", changed('layout: normal', 'layout: split'), "requires at least two faces"),
    ("transform_with_one_face", changed('layout: normal', 'layout: transform'), "requires at least two faces"),
    ("unterminated_string", changed('oracle: "Flying. {T}: Draw a card."', 'oracle: "unterminated'), ""),
    ("invalid_escape", changed('oracle: "Flying. {T}: Draw a card."', 'oracle: "bad \\q escape"'), ""),
    ("integer_expression_overflow", changed('draw(1, you())', 'draw(999999999999999999999999999, you())'), "outside i64 range"),
    ("duplicate_mana_ability", changed('      effect: draw(1, you())', '      effect: draw(1, you())\n      mana_ability: true\n      mana_ability: false'), "duplicate `mana_ability:`"),
    ("number_argument_gets_selector", changed('draw(1, you())', 'draw(source(), you())'), "argument 1 requires integer or value"),
    ("selector_argument_gets_cost", changed('draw(1, you())', 'draw(1, mana_cost("{G}"))'), "argument 2 requires selector"),
    ("variadic_effect_gets_selector", changed('draw(1, you())', 'sequence(draw(1, you()), source())'), "argument 2 requires effect"),
    ("target_gets_integer", changed('draw(1, you())', 'draw(1, target(1))'), "argument 1 requires selector or predicate"),
    ("nested_bare_symbol", changed('draw(1, you())', 'draw(1, player_binding)'), "received bare symbol"),
    ("alternate_cost_gets_effect", changed('draw(1, you())', 'continuous(source(), alternate_cost(source(), draw(1, you())))'), "argument 2 requires cost"),
    ("continuous_gets_prose", changed('draw(1, you())', 'continuous(source(), "rules prose")'), "argument 2 requires effect"),
    ("predicate_gets_selector", changed('draw(1, you())', 'draw(1, target(permanents(and(type_is("creature"), you()))))'), "argument 2 requires predicate"),
]


REQUIRED_ARGUMENT_KINDS = {
    "integer",
    "boolean",
    "text",
    "selector",
    "predicate",
    "cost",
    "event",
    "effect",
    "timing",
    "value",
    "number",
    "selector_or_text",
    "selector_or_predicate",
    "selector_text_or_number",
    "selector_or_number",
    "predicate_or_text",
    "selector_or_event",
    "scalar",
    "comparable",
    "remembered_value",
}


def recursive_case(
    name: str,
    expression: str,
    diagnostic: str,
    argument_kind: str,
    depth: int,
    *features: str,
) -> tuple[str, str, str, dict[str, object]]:
    return (
        name,
        changed("draw(1, you())", expression),
        diagnostic,
        {
            "category": "recursive_argument",
            "argument_kind": argument_kind,
            "depth": depth,
            "features": ["category_correct_wrong_argument", *features],
        },
    )


RECURSIVE_ARGUMENT_METADATA: dict[str, dict[str, object]] = {
    "number_argument_gets_selector": {
        "category": "recursive_argument",
        "argument_kind": "number",
        "depth": 1,
        "features": ["category_correct_wrong_argument"],
    },
    "selector_argument_gets_cost": {
        "category": "recursive_argument",
        "argument_kind": "selector",
        "depth": 1,
        "features": ["category_correct_wrong_argument"],
    },
    "variadic_effect_gets_selector": {
        "category": "recursive_argument",
        "argument_kind": "effect",
        "depth": 2,
        "features": ["category_correct_wrong_argument", "variadic"],
    },
    "target_gets_integer": {
        "category": "recursive_argument",
        "argument_kind": "selector_or_predicate",
        "depth": 2,
        "features": ["category_correct_wrong_argument"],
    },
    "nested_bare_symbol": {
        "category": "recursive_argument",
        "argument_kind": "selector",
        "depth": 1,
        "features": ["bare_symbol"],
    },
    "alternate_cost_gets_effect": {
        "category": "recursive_argument",
        "argument_kind": "cost",
        "depth": 2,
        "features": ["category_correct_wrong_argument"],
    },
    "continuous_gets_prose": {
        "category": "recursive_argument",
        "argument_kind": "effect",
        "depth": 1,
        "features": ["prose"],
    },
    "predicate_gets_selector": {
        "category": "recursive_argument",
        "argument_kind": "predicate",
        "depth": 4,
        "features": ["category_correct_wrong_argument"],
    },
}


ADDITIONAL_RECURSIVE_ARGUMENT_CASES = [
    recursive_case("integer_set_text_gets_value", "set_text_marker(source(), amount(you()))", "requires integer", "integer", 1),
    recursive_case("integer_layer_dependency_gets_value", "layer_effect(you(), source(), tap(source()), 1, amount(you()))", "requires integer", "integer", 2, "variadic"),
    recursive_case("integer_deep_gets_boolean", "sequence(until_end_of_turn(set_text_marker(source(), true)))", "requires integer", "integer", 3),
    recursive_case("boolean_gets_integer", "while_condition(boolean_is(1), draw(1, you()))", "requires boolean", "boolean", 2),
    recursive_case("boolean_gets_text", 'while_condition(boolean_is("true"), draw(1, you()))', "requires boolean", "boolean", 2),
    recursive_case("boolean_gets_selector", "while_condition(boolean_is(source()), draw(1, you()))", "requires boolean", "boolean", 2),
    recursive_case("text_add_mana_gets_selector", "add_mana(source(), you())", "requires text", "text", 1),
    recursive_case("text_remember_key_gets_integer", "remember(1, source())", "requires text", "text", 1),
    recursive_case("text_deep_type_gets_selector", "destroy(permanents(type_is(source())))", "requires text", "text", 3),
    recursive_case("selector_tap_gets_effect", "tap(draw(1, you()))", "requires selector", "selector", 1),
    recursive_case("selector_controller_gets_predicate", 'draw(1, controller_of(type_is("creature")))', "requires selector", "selector", 2),
    recursive_case("selector_damage_gets_cost", 'deal_damage(mana_cost("{R}"), 1)', "requires selector", "selector", 1),
    recursive_case("predicate_while_gets_selector", "while_condition(source(), draw(1, you()))", "requires predicate", "predicate", 1),
    recursive_case("predicate_and_gets_event", 'destroy(permanents(and(type_is("creature"), event_cast())))', "requires predicate", "predicate", 3, "variadic"),
    recursive_case("predicate_attack_gets_cost", 'cannot_attack(source(), mana_cost("{1}"))', "requires predicate", "predicate", 1),
    recursive_case("cost_alternate_gets_selector", "continuous(source(), alternate_cost(source(), source()))", "requires cost", "cost", 2),
    recursive_case("cost_alternate_variadic_gets_event", 'continuous(source(), alternate_cost(source(), mana_cost("{1}"), event_cast()))', "requires cost", "cost", 2, "variadic"),
    recursive_case("cost_deep_gets_effect", "sequence(until_end_of_turn(continuous(source(), alternate_cost(source(), draw(1, you())))))", "requires cost", "cost", 4),
    recursive_case("event_delayed_gets_selector", "register_delayed_trigger(source(), draw(1, you()))", "requires event", "event", 1),
    recursive_case("event_deep_gets_predicate", 'sequence(register_delayed_trigger(event_cast(), register_delayed_trigger(type_is("creature"), draw(1, you())), "outer"))', "requires event", "event", 3),
    recursive_case("effect_choose_gets_selector", "choose_one(draw(1, you()), source())", "requires effect", "effect", 1, "variadic"),
    recursive_case("effect_choose_up_to_gets_event", "choose_up_to(2, draw(1, you()), event_cast())", "requires effect", "effect", 1, "variadic"),
    recursive_case("effect_until_gets_cost", 'until_end_of_turn(mana_cost("{1}"))', "requires effect", "effect", 1),
    recursive_case("effect_at_timing_gets_selector", "at_timing(timing_instant(), source())", "requires effect", "effect", 1),
    recursive_case("timing_gets_selector", "at_timing(source(), draw(1, you()))", "requires timing", "timing", 1),
    recursive_case("timing_gets_effect", "at_timing(draw(1, you()), draw(1, you()))", "requires timing", "timing", 1),
    recursive_case("timing_deep_gets_cost", 'sequence(at_timing(mana_cost("{1}"), draw(1, you())))', "requires timing", "timing", 2),
    recursive_case("value_nonzero_gets_integer", "while_condition(nonzero(1), draw(1, you()))", "requires value", "value", 2),
    recursive_case("value_nonzero_gets_selector", "while_condition(nonzero(source()), draw(1, you()))", "requires value", "value", 2),
    recursive_case("value_nonzero_gets_effect", "while_condition(nonzero(draw(1, you())), draw(1, you()))", "requires value", "value", 2),
    recursive_case("number_choose_gets_text", 'choose_up_to("two", draw(1, you()))', "requires integer or value", "number", 1),
    recursive_case("number_modify_gets_selector", "modify_pt(source(), source(), 1)", "requires integer or value", "number", 1),
    recursive_case("number_deep_draw_gets_event", "sequence(until_end_of_turn(draw(event_cast(), you())))", "requires integer or value", "number", 3),
    recursive_case("selector_or_text_damage_gets_predicate", 'deal_damage(source(), 1, type_is("creature"))', "requires selector or text", "selector_or_text", 1),
    recursive_case("selector_or_text_event_gets_cost", 'register_delayed_trigger(event_cast(source(), mana_cost("{1}")), draw(1, you()))', "requires selector or text", "selector_or_text", 2),
    recursive_case("selector_or_predicate_chosen_gets_text", 'draw(1, chosen("name"))', "requires selector or predicate", "selector_or_predicate", 2),
    recursive_case("selector_or_predicate_target_gets_cost", 'tap(target(mana_cost("{1}")))', "requires selector or predicate", "selector_or_predicate", 2),
    recursive_case("selector_text_number_exile_gets_event", "exile(source(), event_cast())", "requires selector, text, integer, or value", "selector_text_or_number", 1),
    recursive_case("selector_text_number_deep_gets_predicate", 'sequence(exile(source(), and(type_is("creature"), type_is("artifact"))))', "requires selector, text, integer, or value", "selector_text_or_number", 2),
    recursive_case("selector_or_number_look_gets_predicate", 'look_at(source(), type_is("creature"))', "requires selector, integer, or value", "selector_or_number", 1),
    recursive_case("selector_or_number_look_gets_cost", 'sequence(look_at(source(), mana_cost("{1}")))', "requires selector, integer, or value", "selector_or_number", 2),
    recursive_case("predicate_or_text_cards_gets_selector", "draw(1, cards(source()))", "requires predicate or text", "predicate_or_text", 2),
    recursive_case("predicate_or_text_permanents_gets_event", "tap(permanents(event_cast()))", "requires predicate or text", "predicate_or_text", 2),
    recursive_case("selector_or_event_replace_gets_predicate", 'replace_event(type_is("creature"), draw(1, you()))', "requires selector or event", "selector_or_event", 1),
    recursive_case("selector_or_event_double_gets_text", 'double_event("damage", 1)', "requires selector or event", "selector_or_event", 1),
    recursive_case("scalar_search_gets_predicate", 'search_library(cards(), you(), type_is("creature"))', "requires scalar literal or value", "scalar", 1, "variadic"),
    recursive_case("scalar_if_else_gets_selector", 'draw(if_else(type_is("creature"), source(), 1), you())', "requires scalar literal or value", "scalar", 2),
    recursive_case("comparable_equals_gets_cost", 'while_condition(equals(mana_cost("{1}"), 1), draw(1, you()))', "requires comparable expression", "comparable", 2),
    recursive_case("comparable_move_gets_cost", 'move_zone(source(), "exile", mana_cost("{1}"))', "requires comparable expression", "comparable", 1),
    recursive_case("remembered_value_gets_predicate", 'remember("x", type_is("creature"))', "requires rememberable value", "remembered_value", 1),
    recursive_case("remembered_value_gets_event", 'remember("x", event_cast())', "requires rememberable value", "remembered_value", 1),
]


def generate(root: Path, check: bool) -> int:
    output_dir = root / "cards/cp_dsl/malformed"
    output_dir.mkdir(parents=True, exist_ok=True)
    expected: dict[Path, bytes] = {}
    rows = []
    all_cases = [
        (name, source, diagnostic, RECURSIVE_ARGUMENT_METADATA.get(name))
        for name, source, diagnostic in CASES
    ]
    all_cases.extend(ADDITIONAL_RECURSIVE_ARGUMENT_CASES)
    for index, (name, source, diagnostic, metadata) in enumerate(all_cases, start=1):
        path = output_dir / f"{index:03d}_{name}.frs"
        content = source.encode("utf-8")
        expected[path] = content
        row: dict[str, object] = {
            "id": name,
            "file": str(path.relative_to(root)),
            "expected_diagnostic": diagnostic,
            "sha256": hashlib.sha256(content).hexdigest(),
        }
        if metadata is not None:
            row.update(metadata)
        rows.append(row)
    recursive_rows = [row for row in rows if row.get("category") == "recursive_argument"]
    recursive_kinds = sorted({str(row["argument_kind"]) for row in recursive_rows})
    recursive_features = sorted(
        {
            str(feature)
            for row in recursive_rows
            for feature in row.get("features", [])
        }
    )
    manifest = {
        "schema_version": 1,
        "case_count": len(rows),
        "minimum_required": 50,
        "recursive_argument_case_count": len(recursive_rows),
        "recursive_argument_minimum_required": 50,
        "recursive_argument_kinds": recursive_kinds,
        "required_argument_kinds": sorted(REQUIRED_ARGUMENT_KINDS),
        "missing_argument_kinds": sorted(REQUIRED_ARGUMENT_KINDS - set(recursive_kinds)),
        "recursive_argument_depths": sorted({int(row["depth"]) for row in recursive_rows}),
        "recursive_argument_features": recursive_features,
        "cases": rows,
    }
    manifest_path = output_dir / "manifest.json"
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode()
    existing = set(output_dir.glob("*.frs"))
    failures = []
    for path, content in expected.items():
        if check:
            if not path.exists() or path.read_bytes() != content:
                failures.append(str(path.relative_to(root)))
        else:
            path.write_bytes(content)
    failures.extend(str(path.relative_to(root)) for path in sorted(existing - set(expected)))
    if check:
        if not manifest_path.exists() or manifest_path.read_bytes() != manifest_bytes:
            failures.append(str(manifest_path.relative_to(root)))
    else:
        if existing - set(expected):
            raise ValueError("stale malformed fixtures exist; remove them explicitly")
        manifest_path.write_bytes(manifest_bytes)
    if failures:
        print("malformed fixture drift: " + ", ".join(failures), file=sys.stderr)
        return 1
    print(f"CP-DSL malformed corpus: {len(rows)} cases")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        return generate(args.root, args.check)
    except (OSError, ValueError) as error:
        print(f"generate_cp_dsl_negative.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
