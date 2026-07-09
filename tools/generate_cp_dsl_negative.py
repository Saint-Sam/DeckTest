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


def generate(root: Path, check: bool) -> int:
    output_dir = root / "cards/cp_dsl/malformed"
    output_dir.mkdir(parents=True, exist_ok=True)
    expected: dict[Path, bytes] = {}
    rows = []
    for index, (name, source, diagnostic) in enumerate(CASES, start=1):
        path = output_dir / f"{index:03d}_{name}.frs"
        content = source.encode("utf-8")
        expected[path] = content
        rows.append(
            {
                "id": name,
                "file": str(path.relative_to(root)),
                "expected_diagnostic": diagnostic,
                "sha256": hashlib.sha256(content).hexdigest(),
            }
        )
    manifest = {
        "schema_version": 1,
        "case_count": len(rows),
        "minimum_required": 50,
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
