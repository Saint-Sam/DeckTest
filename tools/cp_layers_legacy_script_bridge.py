#!/usr/bin/env python3
"""Translate representable CP-LAYERS legacy script fragments into RON oracles.

This is deliberately narrow. It parses the already-selected local 100-card
legacy subset, emits Forge 2.0 scenarios only for layer fragments the current
Rust engine can represent, and records every unsupported key it skipped.
"""

from __future__ import annotations

import csv
import json
import re
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SUBSET_CSV = ROOT / "reports" / "gates" / "CP-LAYERS" / "legacy-100-layered-subset-2026-07-07.csv"
LEGACY_JSONL = ROOT / "metrics" / "cp_layers_legacy_engine_snapshot.jsonl"
OUT_DIR = ROOT / "tests" / "oracle" / "legacy_layers"
REPORT = ROOT / "reports" / "gates" / "CP-LAYERS" / "legacy-script-bridge-2026-07-07.md"
METRICS = ROOT / "metrics" / "cp_layers_legacy_script_bridge.json"
MANIFEST = OUT_DIR / "MANIFEST.md"

BASE_OBJECTS = {
    "Runeclaw Bear": 0,
    "Memnite": 1,
}

SUPPORTED_TYPES = {
    "Artifact": "artifact",
    "Creature": "creature",
    "Enchantment": "enchantment",
    "Instant": "instant",
    "Land": "land",
    "Planeswalker": "planeswalker",
    "Sorcery": "sorcery",
}

SUPPORTED_KEYWORDS = {
    "First Strike": "first_strike",
    "Double Strike": "double_strike",
    "Trample": "trample",
    "Deathtouch": "deathtouch",
    "Lifelink": "lifelink",
    "Flying": "flying",
    "Reach": "reach",
    "Menace": "menace",
    "Vigilance": "vigilance",
    "Haste": "haste",
}

COLOR_CODES = {
    "W": "white",
    "White": "white",
    "U": "blue",
    "Blue": "blue",
    "B": "black",
    "Black": "black",
    "R": "red",
    "Red": "red",
    "G": "green",
    "Green": "green",
}

OP_LAYERS = {
    "change_controller": 2,
    "set_types": 4,
    "add_types": 4,
    "remove_types": 4,
    "set_colors": 5,
    "add_keywords": 6,
    "remove_keywords": 6,
    "set_pt": 71,
    "modify_pt": 72,
}


@dataclass
class Operation:
    target: int
    fields: dict[str, object]
    source: str


@dataclass
class BridgeRecord:
    card_id: str
    name: str
    path: str
    generated: bool
    scenario_path: str | None = None
    operation_count: int = 0
    legacy_match: bool | None = None
    legacy_mismatches: list[str] = field(default_factory=list)
    unsupported: list[str] = field(default_factory=list)
    represented: list[str] = field(default_factory=list)


def read_subset() -> list[dict[str, str]]:
    with SUBSET_CSV.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def read_legacy_snapshots() -> dict[str, dict[str, object]]:
    snapshots: dict[str, dict[str, object]] = {}
    if not LEGACY_JSONL.exists():
        return snapshots
    for line in LEGACY_JSONL.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        payload = json.loads(line)
        snapshots[str(payload.get("scenario", ""))] = payload
    return snapshots


def read_continuous_lines(path: Path) -> list[str]:
    lines = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if line.startswith("S:") and "Mode$ Continuous" in line:
            lines.append(line)
    return lines


def parts(raw: str) -> dict[str, str]:
    parsed: dict[str, str] = {}
    for item in raw.split("|"):
        item = item.strip()
        if not item:
            continue
        if item.startswith("S:"):
            item = item[2:].strip()
        if "$" in item:
            key, value = item.split("$", 1)
            parsed[key.strip()] = value.strip()
    return parsed


def numeric(value: str | None) -> int | None:
    if value is None:
        return None
    value = value.strip()
    if re.fullmatch(r"-?\d+", value):
        return int(value)
    return None


def split_tokens(value: str | None) -> list[str]:
    if not value:
        return []
    return [token.strip() for token in re.split(r"\s*(?:&|,|;)\s*", value) if token.strip()]


def targets_for(affected: str | None) -> tuple[list[int], list[str]]:
    if not affected:
        return [], ["missing Affected$ predicate"]
    affected = affected.strip()
    if affected in {"Creature", "Card.Creature"}:
        return [0, 1], []
    if affected in {"Creature.OppCtrl", "Card.Creature.OppCtrl", "Creature.OppCtrl+nonToken"}:
        return [0], []
    if affected in {"Creature.YouCtrl", "Card.Creature.YouCtrl"}:
        return [1], []
    if "EnchantedBy" in affected:
        return [0], []
    if affected in {"Artifact", "Card.Artifact"}:
        return [1], []
    if "Land" in affected:
        return [], [f"fixture has no represented land target for `{affected}`"]
    return [], [f"unsupported Affected$ predicate `{affected}`"]


def type_tokens(value: str | None) -> tuple[list[str], list[str]]:
    supported = []
    unsupported = []
    for token in split_tokens(value):
        if token in SUPPORTED_TYPES:
            supported.append(SUPPORTED_TYPES[token])
        elif token in {"True", "All", "Any"}:
            continue
        else:
            unsupported.append(f"unsupported type/subtype `{token}`")
    return sorted(set(supported)), unsupported


def keyword_tokens(value: str | None) -> tuple[list[str], list[str]]:
    supported = []
    unsupported = []
    for token in split_tokens(value):
        mapped = SUPPORTED_KEYWORDS.get(token)
        if mapped is None:
            unsupported.append(f"unsupported keyword `{token}`")
        else:
            supported.append(mapped)
    return sorted(set(supported)), unsupported


def color_tokens(value: str | None) -> tuple[list[str], list[str]]:
    if not value or value == "Colorless":
        return [], []
    supported = []
    unsupported = []
    for token in split_tokens(value):
        mapped = COLOR_CODES.get(token)
        if mapped is None:
            unsupported.append(f"unsupported color `{token}`")
        else:
            supported.append(mapped)
    return sorted(set(supported)), unsupported


def translate_line(raw: str, timestamp: int) -> tuple[list[Operation], list[str], list[str]]:
    spec = parts(raw)
    targets, unsupported = targets_for(spec.get("Affected"))
    represented: list[str] = []
    operations: list[Operation] = []
    if not targets:
        return [], unsupported, represented

    power = numeric(spec.get("SetPower"))
    toughness = numeric(spec.get("SetToughness"))
    if "SetPower" in spec or "SetToughness" in spec:
        if power is not None and toughness is not None:
            for target in targets:
                operations.append(
                    Operation(
                        target,
                        {
                            "operation": "set_pt",
                            "power": power,
                            "toughness": toughness,
                            "timestamp": timestamp,
                        },
                        "SetPower/SetToughness",
                    )
                )
            represented.append("numeric set_pt")
        else:
            unsupported.append("dynamic or incomplete SetPower/SetToughness")

    boost_power = numeric(spec.get("AddPower") or spec.get("PowerBoost"))
    boost_toughness = numeric(spec.get("AddToughness") or spec.get("ToughnessBoost"))
    if any(key in spec for key in ("AddPower", "AddToughness", "PowerBoost", "ToughnessBoost")):
        if boost_power is not None and boost_toughness is not None:
            for target in targets:
                operations.append(
                    Operation(
                        target,
                        {
                            "operation": "modify_pt",
                            "power": boost_power,
                            "toughness": boost_toughness,
                            "timestamp": timestamp,
                        },
                        "AddPower/AddToughness",
                    )
                )
            represented.append("numeric modify_pt")
        else:
            unsupported.append("dynamic or incomplete P/T modifier")

    if "GainControl" in spec:
        if spec["GainControl"] == "You":
            for target in targets:
                operations.append(
                    Operation(
                        target,
                        {"operation": "change_controller", "player": 1, "timestamp": timestamp},
                        "GainControl",
                    )
                )
            represented.append("gain control")
        else:
            unsupported.append(f"unsupported GainControl value `{spec['GainControl']}`")

    add_types, add_type_gaps = type_tokens(spec.get("AddType"))
    unsupported.extend(add_type_gaps)
    remove_card_types = spec.get("RemoveCardTypes") in {"True", "All"}
    if remove_card_types and add_types:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "set_types", "types": add_types, "timestamp": timestamp},
                    "AddType/RemoveCardTypes",
                )
            )
        represented.append("set top-level types")
        unsupported.append("RemoveCardTypes$ True has no subtype/supertype fidelity")
    elif add_types:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "add_types", "types": add_types, "timestamp": timestamp},
                    "AddType",
                )
            )
        represented.append("add top-level types")

    remove_type_values = [
        spec.get("RemoveType"),
        spec.get("RemoveCardTypes") if spec.get("RemoveCardTypes") not in {"True", "All"} else None,
    ]
    remove_types: list[str] = []
    for value in remove_type_values:
        parsed, gaps = type_tokens(value)
        remove_types.extend(parsed)
        unsupported.extend(gaps)
    if remove_card_types and not add_types:
        remove_types.extend(["artifact", "creature", "enchantment", "instant", "land", "planeswalker", "sorcery"])
        unsupported.append("RemoveCardTypes$ True has no subtype/supertype fidelity")
    if spec.get("RemoveLandTypes") == "True":
        unsupported.append("RemoveLandTypes$ True is not represented beyond top-level land")
    if spec.get("RemoveCreatureTypes") == "True" or spec.get("RemoveSubTypes") == "True":
        unsupported.append("creature/subtype removal is not represented")
    if remove_types:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "remove_types", "types": sorted(set(remove_types)), "timestamp": timestamp},
                    "RemoveType/RemoveCardTypes",
                )
            )
        represented.append("remove top-level types")

    colors, color_gaps = color_tokens(spec.get("SetColor") or spec.get("AddColor"))
    unsupported.extend(color_gaps)
    if "SetColor" in spec or "AddColor" in spec:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "set_colors", "colors": colors, "timestamp": timestamp},
                    "SetColor/AddColor",
                )
            )
        represented.append("set colors")

    add_keywords, add_keyword_gaps = keyword_tokens(spec.get("AddKeyword"))
    unsupported.extend(add_keyword_gaps)
    if add_keywords:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "add_keywords", "keywords": add_keywords, "timestamp": timestamp},
                    "AddKeyword",
                )
            )
        represented.append("add supported combat keywords")

    remove_keywords, remove_keyword_gaps = keyword_tokens(spec.get("RemoveKeyword"))
    unsupported.extend(remove_keyword_gaps)
    if spec.get("RemoveAllAbilities") == "True":
        remove_keywords.extend(sorted(SUPPORTED_KEYWORDS.values()))
        unsupported.append("RemoveAllAbilities$ True represented only as supported combat-keyword removal")
    if spec.get("CantHaveKeyword"):
        unsupported.append("CantHaveKeyword$ suppression is not represented")
    if remove_keywords:
        for target in targets:
            operations.append(
                Operation(
                    target,
                    {"operation": "remove_keywords", "keywords": sorted(set(remove_keywords)), "timestamp": timestamp},
                    "RemoveKeyword/RemoveAllAbilities",
                )
            )
        represented.append("remove supported combat keywords")

    for key in ("ChangeText", "AddText", "RemoveText", "Copy", "Clone", "SetPowerToughness"):
        if key in spec:
            unsupported.append(f"unsupported legacy key `{key}`")

    return operations, sorted(set(unsupported)), sorted(set(represented))


def slugify(name: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_")
    return slug or "card"


def ron_value(value: object) -> str:
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, list):
        return "[" + ", ".join(ron_value(item) for item in value) + "]"
    return str(value)


def operation_to_ron(op: Operation) -> str:
    fields: list[tuple[str, object]] = [
        ("action", "register_continuous_effect"),
        ("controller", 1),
        ("target_object", op.target),
    ]
    fields.extend((key, value) for key, value in op.fields.items())
    return "        (" + ", ".join(f"{key}: {ron_value(value)}" for key, value in fields) + "),"


def asserted_fields(operations: list[Operation]) -> dict[int, set[str]]:
    target_fields: dict[int, set[str]] = {}
    for operation in operations:
        fields = target_fields.setdefault(operation.target, set())
        name = str(operation.fields.get("operation", ""))
        if name == "change_controller":
            fields.add("controller")
        elif name in {"set_types", "add_types", "remove_types"}:
            fields.add("types")
        elif name == "set_colors":
            fields.add("colors")
        elif name in {"add_keywords", "remove_keywords"}:
            fields.add("keywords")
        elif name in {"set_pt", "modify_pt"}:
            fields.add("pt")
    return target_fields


def simulate_fragment(operations: list[Operation]) -> dict[int, dict[str, object]]:
    state = {
        0: {
            "controller": 0,
            "types": {"creature"},
            "colors": set(),
            "keywords": set(),
            "power": 2,
            "toughness": 2,
        },
        1: {
            "controller": 1,
            "types": {"artifact", "creature"},
            "colors": set(),
            "keywords": set(),
            "power": 1,
            "toughness": 1,
        },
    }
    ordered = sorted(
        enumerate(operations),
        key=lambda item: (
            OP_LAYERS.get(str(item[1].fields.get("operation", "")), 999),
            int(item[1].fields.get("timestamp", 0)),
            item[0],
        ),
    )
    for _, operation in ordered:
        target = state[operation.target]
        name = str(operation.fields.get("operation", ""))
        if name == "change_controller":
            target["controller"] = int(operation.fields["player"])
        elif name == "set_types":
            target["types"] = set(operation.fields["types"])
            if "creature" not in target["types"]:
                target["keywords"] = set()
        elif name == "add_types":
            was_creature = "creature" in target["types"]
            target["types"] = set(target["types"]) | set(operation.fields["types"])
            if not was_creature and "creature" in target["types"]:
                target["power"] = 0
                target["toughness"] = 0
        elif name == "remove_types":
            target["types"] = set(target["types"]) - set(operation.fields["types"])
            if "creature" not in target["types"]:
                target["keywords"] = set()
        elif name == "set_colors":
            target["colors"] = set(operation.fields["colors"])
        elif name == "add_keywords" and "creature" in target["types"]:
            target["keywords"] = set(target["keywords"]) | set(operation.fields["keywords"])
        elif name == "remove_keywords" and "creature" in target["types"]:
            target["keywords"] = set(target["keywords"]) - set(operation.fields["keywords"])
        elif name == "set_pt" and "creature" in target["types"]:
            target["power"] = int(operation.fields["power"])
            target["toughness"] = int(operation.fields["toughness"])
        elif name == "modify_pt" and "creature" in target["types"]:
            target["power"] = int(target["power"]) + int(operation.fields["power"])
            target["toughness"] = int(target["toughness"]) + int(operation.fields["toughness"])
    return state


def expected_for(operations: list[Operation]) -> list[str]:
    target_fields = asserted_fields(operations)
    state = simulate_fragment(operations)
    expectations = []
    for index in sorted(target_fields):
        asserted = target_fields[index]
        actual = state[index]
        fields: list[tuple[str, object]] = [("object", index)]
        if "controller" in asserted:
            fields.append(("controller", actual["controller"]))
        if "types" in asserted:
            types = sorted(actual["types"])
            fields.append(("types", types))
            fields.append(("is_creature", "creature" in types))
        if "pt" in asserted and "creature" in actual["types"]:
            fields.append(("power", actual["power"]))
            fields.append(("toughness", actual["toughness"]))
        if "colors" in asserted:
            fields.append(("colors", sorted(actual["colors"])))
        if "keywords" in asserted and "creature" in actual["types"]:
            fields.append(("keywords", sorted(actual["keywords"])))
        expectations.append(
            "            ("
            + ", ".join(f"{key}: {ron_value(value)}" for key, value in fields)
            + "),"
        )
    return expectations


def legacy_projection(snapshot: dict[str, object]) -> dict[int, dict[str, object]]:
    by_name = {}
    for item in snapshot.get("battlefield", []):
        if isinstance(item, dict):
            by_name[str(item.get("name", ""))] = item
    projected = {}
    for name, index in BASE_OBJECTS.items():
        if name not in by_name:
            continue
        card = by_name[name]
        type_text = str(card.get("types", ""))
        projected[index] = {
            "controller": 1 if card.get("controller") == "controller" else 0,
            "types": {
                mapped
                for legacy, mapped in SUPPORTED_TYPES.items()
                if re.search(rf"\b{legacy}\b", type_text)
            },
            "colors": {mapped for code, mapped in COLOR_CODES.items() if len(code) == 1 and code in str(card.get("colors", ""))},
            "keywords": {
                SUPPORTED_KEYWORDS[keyword]
                for keyword in card.get("keywords", [])
                if isinstance(keyword, str) and keyword in SUPPORTED_KEYWORDS
            },
            "power": int(card.get("power", 0)),
            "toughness": int(card.get("toughness", 0)),
        }
    return projected


def compare_legacy(operations: list[Operation], snapshot: dict[str, object]) -> tuple[bool | None, list[str]]:
    if not snapshot:
        return None, ["missing legacy snapshot"]
    target_fields = asserted_fields(operations)
    predicted = simulate_fragment(operations)
    legacy = legacy_projection(snapshot)
    mismatches = []
    for index, fields in sorted(target_fields.items()):
        if index not in legacy:
            mismatches.append(f"object {index} missing from legacy snapshot")
            continue
        for field in sorted(fields):
            if field == "pt":
                for scalar in ("power", "toughness"):
                    if predicted[index][scalar] != legacy[index][scalar]:
                        mismatches.append(
                            f"object {index} {scalar}: bridge {predicted[index][scalar]} != legacy {legacy[index][scalar]}"
                        )
            elif field == "types":
                if predicted[index]["types"] != legacy[index]["types"]:
                    mismatches.append(
                        f"object {index} types: bridge {sorted(predicted[index]['types'])} != legacy {sorted(legacy[index]['types'])}"
                    )
            elif field in {"colors", "keywords"}:
                if predicted[index][field] != legacy[index][field]:
                    mismatches.append(
                        f"object {index} {field}: bridge {sorted(predicted[index][field])} != legacy {sorted(legacy[index][field])}"
                    )
            else:
                if predicted[index][field] != legacy[index][field]:
                    mismatches.append(
                        f"object {index} {field}: bridge {predicted[index][field]} != legacy {legacy[index][field]}"
                    )
    return not mismatches, mismatches


def write_scenario(card_id: str, name: str, operations: list[Operation]) -> Path:
    path = OUT_DIR / f"cp_layers_legacy_{card_id.lower()}_{slugify(name)}.ron"
    expectations = expected_for(operations)
    if not expectations:
        expectations = [
            '            (object: 0, is_creature: true, power: 2, toughness: 2),',
            '            (object: 1, is_creature: true, power: 1, toughness: 1),',
        ]
    lines = [
        "(",
        f"    name: {ron_value('legacy bridge ' + card_id + ' ' + name)},",
        "    setup: (",
        "        players: 2,",
        "        objects: [",
        f"            (card: {70000 + int(card_id[1:]) * 10 + 1}, owner: 0, controller: 0, zone: \"Battlefield\"),",
        f"            (card: {70000 + int(card_id[1:]) * 10 + 2}, owner: 1, controller: 1, zone: \"Battlefield\"),",
        f"            (card: {70000 + int(card_id[1:]) * 10 + 3}, owner: 1, controller: 1, zone: \"Battlefield\"),",
        "        ],",
        "    ),",
        "    script: [",
        '        (action: "set_base_creature", object: 0, power: 2, toughness: 2, keywords: []),',
        '        (action: "set_base_creature", object: 1, power: 1, toughness: 1, keywords: []),',
        '        (action: "register_continuous_effect", controller: 1, target_object: 1, operation: "add_types", types: ["artifact"], timestamp: 1),',
    ]
    for op in operations:
        lines.append(operation_to_ron(op))
    lines.extend(
        [
            "    ],",
            "    expect: (",
            "        characteristics: [",
            *expectations,
            "        ],",
            '        outcome: "in_progress",',
            '        invariants: ["zone_conservation", "hash_consistency"],',
            "        hash_determinism: true,",
            "    ),",
            ")",
        ]
    )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def clean_output_dir() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for path in OUT_DIR.glob("cp_layers_legacy_*.ron"):
        path.unlink()


def write_manifest(records: list[BridgeRecord]) -> None:
    generated = [record for record in records if record.generated]
    lines = [
        "# CP-LAYERS Legacy Script Bridge Oracles",
        "",
        "Generated from the local 100-card legacy subset. These are representable",
        "script fragments, not a full card compiler or full-card differential.",
        "",
        f"- Selected legacy scripts: {len(records)}",
        f"- Generated executable RON scenarios: {len(generated)}",
        f"- Not generated because no current-model operation was representable: {len(records) - len(generated)}",
        "",
    ]
    MANIFEST.write_text("\n".join(lines), encoding="utf-8")


def write_report(records: list[BridgeRecord]) -> None:
    generated = [record for record in records if record.generated]
    legacy_matches = [record for record in generated if record.legacy_match is True]
    legacy_mismatches = [record for record in generated if record.legacy_match is False]
    unsupported_counts = Counter(gap for record in records for gap in record.unsupported)
    represented_counts = Counter(item for record in records for item in record.represented)
    metrics = {
        "selected_count": len(records),
        "generated_scenario_count": len(generated),
        "not_generated_count": len(records) - len(generated),
        "legacy_modeled_match_count": len(legacy_matches),
        "legacy_modeled_mismatch_count": len(legacy_mismatches),
        "operation_count": sum(record.operation_count for record in records),
        "unsupported_counts": dict(sorted(unsupported_counts.items())),
        "represented_counts": dict(sorted(represented_counts.items())),
        "oracle_dir": str(OUT_DIR.relative_to(ROOT)),
        "report": str(REPORT.relative_to(ROOT)),
    }
    METRICS.write_text(json.dumps(metrics, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    lines = [
        "# CP-LAYERS Legacy Script Bridge",
        "",
        "Date: 2026-07-07",
        "",
        "Mode: local-only translation of the selected 100 vendored legacy Forge card scripts into the current Forge 2.0 layer-oracle vocabulary.",
        "",
        "Result: PARTIAL.",
        "",
        "This is not a full card compiler and not the final true engine-vs-engine differential. It is the strongest executable bridge available before implementing the real Forge 2.0 card importer/compiler: every selected script is parsed, representable continuous-effect fragments are emitted as RON scenarios, and unsupported keys remain explicit blockers.",
        "",
        "## Counts",
        "",
        "| Metric | Count |",
        "| --- | ---: |",
        f"| Selected legacy scripts | {len(records)} |",
        f"| Generated executable Forge 2.0 scenarios | {len(generated)} |",
        f"| Scripts with no representable current-model operation | {len(records) - len(generated)} |",
        f"| Generated continuous-effect operations | {sum(record.operation_count for record in records)} |",
        f"| Generated scenarios whose modeled fields match the legacy snapshot | {len(legacy_matches)} |",
        f"| Generated scenarios whose modeled fields differ from the legacy snapshot | {len(legacy_mismatches)} |",
        "",
        "## Represented Fragment Counts",
        "",
        "| Fragment | Count |",
        "| --- | ---: |",
    ]
    if represented_counts:
        for key, count in sorted(represented_counts.items(), key=lambda item: (-item[1], item[0])):
            lines.append(f"| {key} | {count} |")
    else:
        lines.append("| none | 0 |")
    lines.extend(["", "## Unsupported Blocker Counts", "", "| Blocker | Count |", "| --- | ---: |"])
    if unsupported_counts:
        for key, count in sorted(unsupported_counts.items(), key=lambda item: (-item[1], item[0])):
            lines.append(f"| {key} | {count} |")
    else:
        lines.append("| none | 0 |")
    lines.extend(["", "## Per-Card Bridge Status", "", "| ID | Card | Generated | Ops | Legacy modeled fields | Unsupported summary |", "| --- | --- | --- | ---: | --- | --- |"])
    for record in records:
        gaps = "; ".join(record.unsupported[:4])
        if len(record.unsupported) > 4:
            gaps += f"; +{len(record.unsupported) - 4} more"
        if not gaps:
            gaps = "none"
        legacy = "n/a"
        if record.legacy_match is True:
            legacy = "match"
        elif record.legacy_match is False:
            legacy = "mismatch"
        lines.append(
            f"| {record.card_id} | {record.name} | {'yes' if record.generated else 'no'} | {record.operation_count} | {legacy} | {gaps} |"
        )
    if legacy_mismatches:
        lines.extend(["", "## Legacy Modeled-Field Mismatches", "", "| ID | Card | First mismatches |", "| --- | --- | --- |"])
        for record in legacy_mismatches:
            lines.append(
                f"| {record.card_id} | {record.name} | {'; '.join(record.legacy_mismatches[:3])} |"
            )
    lines.extend(
        [
            "",
            "## Gate Consequence",
            "",
            "The 100 selected real legacy scripts now have an executable Forge 2.0 bridge where the current layer engine can represent their continuous-effect fragments. CP-LAYERS still remains pending for the true differential because the bridge skips or approximates predicates, subtypes/supertypes, land subtype intrinsic mana abilities, all-abilities removal outside supported combat keywords, dynamic P/T expressions, can't-have keyword suppression, copy semantics, and full card compilation.",
            "",
        ]
    )
    REPORT.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    subset = read_subset()
    snapshots = read_legacy_snapshots()
    clean_output_dir()
    records: list[BridgeRecord] = []
    for row in subset:
        card_id = row["id"]
        name = row["name"]
        path = ROOT / row["path"]
        operations: list[Operation] = []
        unsupported: list[str] = []
        represented: list[str] = []
        for index, raw in enumerate(read_continuous_lines(path), start=10):
            translated, gaps, reps = translate_line(raw, index)
            operations.extend(translated)
            unsupported.extend(gaps)
            represented.extend(reps)
        scenario_path = None
        if operations:
            scenario_path = write_scenario(card_id, name, operations)
        legacy_match, legacy_mismatches = (
            compare_legacy(operations, snapshots.get(name, {})) if operations else (None, [])
        )
        records.append(
            BridgeRecord(
                card_id=card_id,
                name=name,
                path=row["path"],
                generated=bool(operations),
                scenario_path=str(scenario_path.relative_to(ROOT)) if scenario_path else None,
                operation_count=len(operations),
                legacy_match=legacy_match,
                legacy_mismatches=legacy_mismatches,
                unsupported=sorted(set(unsupported)),
                represented=sorted(set(represented)),
            )
        )
    write_manifest(records)
    write_report(records)
    print(f"wrote {REPORT.relative_to(ROOT)}")
    print(f"wrote {METRICS.relative_to(ROOT)}")
    print(f"wrote {len([record for record in records if record.generated])} scenario(s) under {OUT_DIR.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
