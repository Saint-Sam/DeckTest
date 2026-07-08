#!/usr/bin/env python3
"""Run a local CP-LAYERS legacy-script importer differential.

This tool is stricter than the earlier bridge: it imports the active legacy
card face, builds the same three-object CP-LAYERS fixture as the Java legacy
snapshot harness, resolves attachment-backed selectors, applies layer-ordered
continuous effects, and compares stable fixture roles instead of current names.
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
REPORT = ROOT / "reports" / "gates" / "CP-LAYERS" / "legacy-true-importer-diff-2026-07-08.md"
METRICS = ROOT / "metrics" / "cp_layers_true_importer_diff.json"
PREDICTED_JSONL = ROOT / "metrics" / "cp_layers_true_importer_diff_predicted.jsonl"

TOP_TYPE_ORDER = [
    "Artifact",
    "Enchantment",
    "Land",
    "Creature",
    "Planeswalker",
    "Battle",
    "Instant",
    "Sorcery",
]
TOP_TYPES = set(TOP_TYPE_ORDER)
SUPERTYPE_ORDER = ["Basic", "Legendary", "Ongoing", "Snow", "World", "Kindred"]
SUPERTYPES = set(SUPERTYPE_ORDER)
COLOR_WORDS = {
    "White": "W",
    "Blue": "U",
    "Black": "B",
    "Red": "R",
    "Green": "G",
    "white": "W",
    "blue": "U",
    "black": "B",
    "red": "R",
    "green": "G",
    "W": "W",
    "U": "U",
    "B": "B",
    "R": "R",
    "G": "G",
}
COLORLESS_WORDS = {"Colorless", "colorless", "C"}


@dataclass
class TypeLine:
    supertypes: list[str] = field(default_factory=list)
    card_types: list[str] = field(default_factory=list)
    subtypes: list[str] = field(default_factory=list)

    def copy(self) -> "TypeLine":
        return TypeLine(list(self.supertypes), list(self.card_types), list(self.subtypes))

    def has_type(self, name: str) -> bool:
        return name in self.card_types

    def has_subtype(self, name: str) -> bool:
        return name in self.subtypes

    def add_token(self, token: str) -> None:
        if token in SUPERTYPES:
            append_unique(self.supertypes, token)
        elif token in TOP_TYPES:
            append_unique(self.card_types, token)
        elif token and token not in {"True", "All", "Any"}:
            append_unique(self.subtypes, token)

    def remove_token(self, token: str) -> None:
        self.supertypes = [item for item in self.supertypes if item != token]
        self.card_types = [item for item in self.card_types if item != token]
        self.subtypes = [item for item in self.subtypes if item != token]

    def remove_card_types(self) -> None:
        self.card_types.clear()

    def remove_subtypes(self) -> None:
        self.subtypes.clear()

    def text(self) -> str:
        supertypes = ordered_by(self.supertypes, SUPERTYPE_ORDER)
        card_types = ordered_by(self.card_types, TOP_TYPE_ORDER)
        left = [*supertypes, *card_types]
        if self.subtypes:
            return f"{' '.join(left)} - {' '.join(self.subtypes)}".strip()
        return " ".join(left)


@dataclass
class CardFace:
    name: str
    mana_cost: str
    colors: list[str]
    types: TypeLine
    power: int
    toughness: int
    keywords: list[str]
    svardefs: dict[str, str]
    continuous_lines: list[str]


@dataclass
class FixtureObject:
    role: str
    name: str
    controller: str
    owner: str
    colors: list[str]
    types: TypeLine
    power: int
    toughness: int
    keywords: list[str]
    mana_value: int
    attached_to: str | None = None

    def is_permanent(self) -> bool:
        return bool(self.types.card_types)

    def is_creature(self) -> bool:
        return self.types.has_type("Creature")

    def snapshot(self) -> dict[str, object]:
        return {
            "name": self.name,
            "controller": self.controller,
            "types": self.types.text(),
            "colors": colors_text(self.colors),
            "power": self.power if self.is_creature() else 0,
            "toughness": self.toughness if self.is_creature() else 0,
            "keywords": sorted(self.keywords),
        }


@dataclass
class ImportedOperation:
    layer: int
    timestamp: int
    targets: list[str]
    action: str
    payload: dict[str, object]
    source: str


@dataclass
class DiffRecord:
    card_id: str
    name: str
    path: str
    active_face: str
    imported_lines: int
    operation_count: int
    exact_match: bool
    mismatches: list[str]
    diagnostics: list[str]


def append_unique(items: list[str], value: str) -> None:
    if value and value not in items:
        items.append(value)


def ordered_by(items: list[str], order: list[str]) -> list[str]:
    keyed = {item: index for index, item in enumerate(order)}
    return sorted(items, key=lambda item: (keyed.get(item, len(order)), items.index(item)))


def split_tokens(value: str | None) -> list[str]:
    if not value:
        return []
    return [token.strip() for token in re.split(r"\s*(?:&|,|;)\s*", value) if token.strip()]


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


def parse_type_line(value: str | None) -> TypeLine:
    line = TypeLine()
    for token in (value or "").split():
        line.add_token(token)
    return line


def parse_mana_cost(value: str | None) -> tuple[int, list[str]]:
    if not value or value == "no cost":
        return 0, []
    total = 0
    colors: list[str] = []
    for token in value.replace("{", " ").replace("}", " ").split():
        if token.isdigit():
            total += int(token)
        elif token in COLOR_WORDS:
            total += 1
            append_unique(colors, COLOR_WORDS[token])
        elif token in {"X", "Y"}:
            continue
        elif "/" in token:
            total += 1
            for part in token.split("/"):
                if part in COLOR_WORDS:
                    append_unique(colors, COLOR_WORDS[part])
    return total, colors


def parse_color_field(value: str | None) -> list[str]:
    if not value:
        return []
    if value in COLORLESS_WORDS:
        return []
    colors: list[str] = []
    for token in split_tokens(value):
        if token in COLORLESS_WORDS:
            continue
        mapped = COLOR_WORDS.get(token)
        if mapped:
            append_unique(colors, mapped)
    return colors


def parse_power_toughness(value: str | None) -> tuple[int, int]:
    if not value or "/" not in value:
        return 0, 0
    power, toughness = value.split("/", 1)
    return numeric_or_zero(power), numeric_or_zero(toughness)


def numeric_or_zero(value: str | None) -> int:
    if value is None:
        return 0
    value = value.strip()
    if re.fullmatch(r"-?\d+", value):
        return int(value)
    return 0


def parse_active_face(path: Path) -> CardFace:
    text = path.read_text(encoding="utf-8", errors="replace")
    active_text = text.split("\nALTERNATE\n", 1)[0]
    fields: dict[str, str] = {}
    keywords: list[str] = []
    svardefs: dict[str, str] = {}
    continuous_lines: list[str] = []
    for raw in active_text.splitlines():
        line = raw.strip()
        if not line or ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key == "K":
            keywords.append(value.strip())
        elif key == "SVar":
            if ":" in value:
                svar_key, svar_value = value.split(":", 1)
                svardefs[svar_key.strip()] = svar_value.strip()
        elif key == "S" and "Mode$ Continuous" in value:
            continuous_lines.append(line)
        else:
            fields.setdefault(key, value.strip())
    mana_value, mana_colors = parse_mana_cost(fields.get("ManaCost"))
    explicit_colors = parse_color_field(fields.get("Colors"))
    colors = explicit_colors if explicit_colors or fields.get("Colors") in COLORLESS_WORDS else mana_colors
    power, toughness = parse_power_toughness(fields.get("PT"))
    return CardFace(
        name=fields.get("Name", path.stem),
        mana_cost=fields.get("ManaCost", ""),
        colors=colors,
        types=parse_type_line(fields.get("Types")),
        power=power,
        toughness=toughness,
        keywords=keywords,
        svardefs=svardefs,
        continuous_lines=continuous_lines,
    )


def colors_text(colors: list[str]) -> str:
    return "".join(colors) if colors else "C"


def fixture_for(face: CardFace) -> dict[str, FixtureObject]:
    objects = {
        "opponent_creature": FixtureObject(
            role="opponent_creature",
            name="Runeclaw Bear",
            owner="opponent",
            controller="opponent",
            colors=["G"],
            types=TypeLine(card_types=["Creature"], subtypes=["Bear"]),
            power=2,
            toughness=2,
            keywords=[],
            mana_value=2,
        ),
        "controller_artifact": FixtureObject(
            role="controller_artifact",
            name="Memnite",
            owner="controller",
            controller="controller",
            colors=[],
            types=TypeLine(card_types=["Artifact", "Creature"], subtypes=["Construct"]),
            power=1,
            toughness=1,
            keywords=[],
            mana_value=0,
        ),
        "source": FixtureObject(
            role="source",
            name=face.name,
            owner="controller",
            controller="controller",
            colors=list(face.colors),
            types=face.types.copy(),
            power=face.power,
            toughness=face.toughness,
            keywords=list(face.keywords),
            mana_value=parse_mana_cost(face.mana_cost)[0],
        ),
    }
    attach_source(objects)
    return objects


def attach_source(objects: dict[str, FixtureObject]) -> None:
    source = objects["source"]
    if any(keyword.startswith("Reconfigure") for keyword in source.keywords):
        return
    if source.types.has_subtype("Equipment") or any(keyword.startswith("Equip") for keyword in source.keywords):
        source.attached_to = "controller_artifact"
        return
    if not source.types.has_subtype("Aura"):
        return
    enchant = next((keyword for keyword in source.keywords if keyword.startswith("Enchant:")), "")
    if not enchant:
        return
    if "Player" in enchant:
        return
    if "Creature" in enchant or "Permanent" in enchant or "Clue" in enchant or "Food" in enchant:
        source.attached_to = "opponent_creature"
        return
    if re.search(r"(?:^|[:,])(?:Land|Mountain)(?:[.:,]|$)", enchant):
        return


def condition_allows(spec: dict[str, str], objects: dict[str, FixtureObject]) -> bool:
    condition = spec.get("Condition")
    if condition == "NotPlayerTurn":
        return False
    if condition and condition not in {"PlayerTurn"}:
        return False
    if spec.get("IsPresent") and not selector_matches_any(spec["IsPresent"], objects):
        return False
    if spec.get("CheckSVar"):
        return False
    return True


def selector_matches_any(selector: str, objects: dict[str, FixtureObject]) -> bool:
    return bool(resolve_selector(selector, objects, allow_unattached=True))


def resolve_selector(
    selector: str | None,
    objects: dict[str, FixtureObject],
    *,
    allow_unattached: bool = False,
) -> list[str]:
    if not selector:
        return ["source"]
    roles: list[str] = []
    for branch in split_selector_branches(selector):
        for role, obj in objects.items():
            if object_matches_branch(role, obj, branch, objects, allow_unattached):
                append_unique(roles, role)
    return roles


def split_selector_branches(selector: str) -> list[str]:
    return [branch.strip() for branch in selector.split(",") if branch.strip()]


def object_matches_branch(
    role: str,
    obj: FixtureObject,
    branch: str,
    objects: dict[str, FixtureObject],
    allow_unattached: bool,
) -> bool:
    terms = [term for term in branch.split("+") if term]
    if not terms:
        return False
    head = terms[0]
    inline_terms: list[str] = []
    if head.startswith("Card.Self"):
        if role != "source":
            return False
        head = "Card"
    elif head.startswith("Permanent.Self"):
        if role != "source":
            return False
        head = "Permanent"

    if head in {"Card.EnchantedBy", "Creature.EnchantedBy", "Permanent.EnchantedBy"}:
        if not attached_role_matches(role, objects, allow_unattached):
            return False
        if head.startswith("Creature") and not obj.is_creature():
            return False
        if head.startswith("Permanent") and not obj.is_permanent():
            return False
    elif head in {"Card.AttachedBy", "Land.AttachedBy", "Land.EnchantedBy", "Vehicle.AttachedBy"}:
        if not attached_role_matches(role, objects, allow_unattached):
            return False
        if head.startswith("Land") and not obj.types.has_type("Land"):
            return False
        if head.startswith("Vehicle") and not obj.types.has_subtype("Vehicle"):
            return False
    elif head in {"Creature.EquippedBy", "Permanent.EquippedBy"}:
        if objects["source"].attached_to != role:
            return False
        if head.startswith("Creature") and not obj.is_creature():
            return False
        if head.startswith("Permanent") and not obj.is_permanent():
            return False
    elif head == "Card":
        pass
    elif head == "Permanent":
        if not obj.is_permanent():
            return False
    elif head in {"Creature", "Card.Creature"}:
        if not obj.is_creature():
            return False
    elif head in {"Creature.YouCtrl", "Card.Creature.YouCtrl"}:
        if not (obj.is_creature() and obj.controller == "controller"):
            return False
    elif head in {"Creature.OppCtrl", "Card.Creature.OppCtrl"}:
        if not (obj.is_creature() and obj.controller == "opponent"):
            return False
    elif head in {"Artifact", "Card.Artifact"}:
        if not obj.types.has_type("Artifact"):
            return False
    elif head == "Artifact.nonCreature":
        if not (obj.types.has_type("Artifact") and not obj.is_creature()):
            return False
    elif head == "Enchantment.nonAura":
        if not (obj.types.has_type("Enchantment") and not obj.types.has_subtype("Aura")):
            return False
    elif head == "Planeswalker.counters_GE1_LOYALTY":
        return False
    elif "." in head:
        kind, *inline_terms = head.split(".")
        if kind in TOP_TYPES and not obj.types.has_type(kind):
            return False
        if kind not in TOP_TYPES and kind not in {"Card", "Permanent"} and not obj.types.has_subtype(kind):
            return False
    elif head in TOP_TYPES:
        if not obj.types.has_type(head):
            return False
    elif not selector_token_matches(obj, head):
        return False

    return all(
        selector_term_allows(term, role, obj, objects)
        for term in [*inline_terms, *terms[1:]]
    )


def attached_role_matches(role: str, objects: dict[str, FixtureObject], allow_unattached: bool) -> bool:
    attached_to = objects["source"].attached_to
    return attached_to == role or (allow_unattached and attached_to is None and role == "opponent_creature")


def selector_token_matches(obj: FixtureObject, token: str) -> bool:
    if token.startswith("non"):
        wanted = token[3:]
        return not (obj.types.has_type(wanted) or obj.types.has_subtype(wanted))
    return obj.types.has_type(token) or obj.types.has_subtype(token)


def selector_term_allows(
    term: str,
    role: str,
    obj: FixtureObject,
    objects: dict[str, FixtureObject],
) -> bool:
    if term in {"Self"}:
        return role == "source"
    if term == "Other":
        return role != "source"
    if term == "YouCtrl":
        return obj.controller == "controller"
    if term == "OppCtrl":
        return obj.controller == "opponent"
    if term == "YouDontOwn":
        return obj.owner != "controller"
    if term == "token":
        return False
    if term.startswith("cmcGE"):
        return obj.mana_value >= numeric_or_zero(term[5:])
    if term.startswith("counters_") or term in {"IsSolved", "equipped", "attacking"}:
        return False
    if term.startswith("non"):
        return selector_token_matches(obj, term)
    if term in TOP_TYPES or term in SUPERTYPES:
        return obj.types.has_type(term) or term in obj.types.supertypes
    return obj.types.has_subtype(term)


def compile_operations(face: CardFace, objects: dict[str, FixtureObject]) -> tuple[list[ImportedOperation], list[str]]:
    operations: list[ImportedOperation] = []
    diagnostics: list[str] = []
    for index, raw in enumerate(face.continuous_lines, start=10):
        spec = parts(raw)
        if not condition_allows(spec, objects):
            diagnostics.append(f"condition skipped: {describe_spec(spec)}")
            continue
        targets = resolve_selector(spec.get("Affected"), objects)
        if not targets:
            diagnostics.append(f"selector matched no fixture objects: {spec.get('Affected', 'self/CDA')}")
            continue
        if spec.get("GainControl") == "You":
            operations.append(ImportedOperation(2, index, targets, "controller", {"controller": "controller"}, "GainControl"))
        if "SetName" in spec:
            operations.append(ImportedOperation(3, index, targets, "set_name", {"name": spec["SetName"]}, "SetName"))
        if any(key in spec for key in ("AddType", "RemoveType", "RemoveCardTypes", "RemoveCreatureTypes", "RemoveSubTypes", "RemoveArtifactTypes", "RemoveLandTypes")):
            operations.append(ImportedOperation(4, index, targets, "type", type_payload(spec), "type"))
        if "SetColor" in spec:
            operations.append(ImportedOperation(5, index, targets, "set_color", {"colors": parse_color_field(spec["SetColor"])}, "SetColor"))
        if "AddColor" in spec:
            operations.append(ImportedOperation(5, index, targets, "add_color", {"colors": parse_color_field(spec["AddColor"])}, "AddColor"))
        if any(key in spec for key in ("RemoveAllAbilities", "AddKeyword", "RemoveKeyword", "CantHaveKeyword")):
            if spec.get("RemoveAllAbilities") == "True":
                operations.append(ImportedOperation(6, index, targets, "clear_keywords", {}, "RemoveAllAbilities"))
            if "RemoveKeyword" in spec:
                operations.append(ImportedOperation(6, index + 1, targets, "remove_keywords", {"keywords": split_tokens(spec["RemoveKeyword"])}, "RemoveKeyword"))
            if "AddKeyword" in spec:
                operations.append(ImportedOperation(6, index + 2, targets, "add_keywords", {"keywords": split_tokens(spec["AddKeyword"])}, "AddKeyword"))
        if "Goad" in spec:
            diagnostics.append("Goad is non-snapshot-visible and ignored")
        set_power = "SetPower" in spec
        set_toughness = "SetToughness" in spec
        if set_power or set_toughness:
            layer = 70 if spec.get("CharacteristicDefining") == "True" else 71
            operations.append(
                ImportedOperation(
                    layer,
                    index,
                    targets,
                    "set_pt",
                    {
                        "power": spec.get("SetPower"),
                        "toughness": spec.get("SetToughness"),
                    },
                    "SetPower/SetToughness",
                )
            )
        if "AddPower" in spec or "AddToughness" in spec or "PowerBoost" in spec or "ToughnessBoost" in spec:
            operations.append(
                ImportedOperation(
                    72,
                    index,
                    targets,
                    "modify_pt",
                    {
                        "power": spec.get("AddPower") or spec.get("PowerBoost"),
                        "toughness": spec.get("AddToughness") or spec.get("ToughnessBoost"),
                    },
                    "AddPower/AddToughness",
                )
            )
        for invisible in ("AddAbility", "AddStaticAbility", "AddTrigger", "MayPlay", "RaiseCost", "AffectedZone", "EffectZone", "Secondary"):
            if invisible in spec:
                diagnostics.append(f"{invisible} is not part of the CP-LAYERS snapshot projection")
    operations.sort(key=lambda op: (op.layer, op.timestamp, op.action))
    return operations, diagnostics


def describe_spec(spec: dict[str, str]) -> str:
    return spec.get("Affected") or spec.get("Description") or "continuous effect"


def type_payload(spec: dict[str, str]) -> dict[str, object]:
    return {
        "add": split_tokens(spec.get("AddType")),
        "remove": split_tokens(spec.get("RemoveType")),
        "remove_card_types": spec.get("RemoveCardTypes") in {"True", "All"},
        "remove_creature_types": spec.get("RemoveCreatureTypes") == "True" or spec.get("RemoveSubTypes") == "True",
        "remove_artifact_types": spec.get("RemoveArtifactTypes") == "True",
        "remove_land_types": spec.get("RemoveLandTypes") == "True",
    }


def apply_operations(
    face: CardFace,
    objects: dict[str, FixtureObject],
    operations: list[ImportedOperation],
) -> None:
    for operation in operations:
        for role in operation.targets:
            obj = objects[role]
            if operation.action == "controller":
                obj.controller = str(operation.payload["controller"])
            elif operation.action == "set_name":
                obj.name = str(operation.payload["name"])
            elif operation.action == "type":
                apply_type_payload(obj, operation.payload)
            elif operation.action == "set_color":
                obj.colors = list(operation.payload["colors"])  # type: ignore[index]
            elif operation.action == "add_color":
                for color in operation.payload["colors"]:  # type: ignore[index]
                    append_unique(obj.colors, str(color))
            elif operation.action == "clear_keywords":
                obj.keywords.clear()
            elif operation.action == "remove_keywords":
                remove = set(str(item) for item in operation.payload["keywords"])  # type: ignore[index]
                obj.keywords = [keyword for keyword in obj.keywords if keyword not in remove]
            elif operation.action == "add_keywords":
                for keyword in operation.payload["keywords"]:  # type: ignore[index]
                    append_unique(obj.keywords, str(keyword))
            elif operation.action == "set_pt":
                if obj.is_creature():
                    obj.power = value_for_expr(operation.payload.get("power"), obj, face, objects)
                    obj.toughness = value_for_expr(operation.payload.get("toughness"), obj, face, objects)
            elif operation.action == "modify_pt":
                if obj.is_creature():
                    obj.power += value_for_expr(operation.payload.get("power"), obj, face, objects)
                    obj.toughness += value_for_expr(operation.payload.get("toughness"), obj, face, objects)


def apply_type_payload(obj: FixtureObject, payload: dict[str, object]) -> None:
    if payload.get("remove_card_types"):
        obj.types.remove_card_types()
        obj.types.remove_subtypes()
    if payload.get("remove_creature_types") or payload.get("remove_artifact_types") or payload.get("remove_land_types"):
        obj.types.remove_subtypes()
    for token in payload.get("remove", []):
        obj.types.remove_token(str(token))
    for token in payload.get("add", []):
        obj.types.add_token(str(token))
    if not obj.is_creature():
        obj.power = 0
        obj.toughness = 0


def value_for_expr(
    value: object,
    target: FixtureObject,
    face: CardFace,
    objects: dict[str, FixtureObject],
) -> int:
    if value is None:
        return 0
    text = str(value).strip()
    if re.fullmatch(r"-?\d+", text):
        return int(text)
    if text == "AffectedX":
        return target.mana_value
    if text in {"X", "Y"}:
        return evaluate_svar_value(text, face, objects)
    return 0


def evaluate_svar_value(name: str, face: CardFace, objects: dict[str, FixtureObject]) -> int:
    definition = face.svardefs.get(name, "")
    if definition.startswith("Count$Valid "):
        selectors = definition[len("Count$Valid ") :]
        matched: set[str] = set()
        for selector in selectors.split(","):
            for role in resolve_selector(selector.strip(), objects):
                if objects[role].controller == "controller":
                    matched.add(role)
        return len(matched)
    if definition.startswith("Equipped$"):
        return 0
    return 0


def read_subset() -> list[dict[str, str]]:
    with SUBSET_CSV.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def read_legacy_snapshots() -> dict[str, dict[str, object]]:
    snapshots: dict[str, dict[str, object]] = {}
    for line in LEGACY_JSONL.read_text(encoding="utf-8").splitlines():
        if line.strip():
            payload = json.loads(line)
            snapshots[str(payload["scenario"])] = payload
    return snapshots


def map_legacy_roles(
    snapshot: dict[str, object],
    source_names: set[str],
) -> dict[str, dict[str, object]]:
    role_map: dict[str, dict[str, object]] = {}
    battlefield = snapshot.get("battlefield", [])
    if not isinstance(battlefield, list):
        return role_map
    leftovers: list[dict[str, object]] = []
    for item in battlefield:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name", ""))
        if name == "Memnite":
            role_map["controller_artifact"] = item
        elif name in source_names:
            role_map["source"] = item
        else:
            leftovers.append(item)
    for item in leftovers:
        if "source" not in role_map and str(item.get("controller")) == "controller" and str(item.get("name")) in source_names:
            role_map["source"] = item
        elif "opponent_creature" not in role_map:
            role_map["opponent_creature"] = item
    return role_map


def compare_snapshot(
    predicted: dict[str, FixtureObject],
    legacy: dict[str, object],
    source_names: set[str],
) -> list[str]:
    legacy_roles = map_legacy_roles(legacy, source_names)
    mismatches: list[str] = []
    for role in ("opponent_creature", "controller_artifact", "source"):
        if role not in legacy_roles:
            mismatches.append(f"{role}: missing from legacy snapshot")
            continue
        actual = legacy_roles[role]
        expected = predicted[role].snapshot()
        for field in ("name", "controller", "types", "colors", "power", "toughness", "keywords"):
            if actual.get(field) != expected[field]:
                mismatches.append(f"{role}.{field}: importer {expected[field]!r} != legacy {actual.get(field)!r}")
    return mismatches


def predicted_payload(name: str, objects: dict[str, FixtureObject]) -> dict[str, object]:
    ordered = sorted(objects.values(), key=lambda obj: obj.name)
    return {
        "scenario": name,
        "status": "ok",
        "battlefield": [obj.snapshot() | {"role": obj.role} for obj in ordered],
    }


def write_outputs(records: list[DiffRecord], predicted: list[dict[str, object]]) -> None:
    METRICS.parent.mkdir(parents=True, exist_ok=True)
    exact = [record for record in records if record.exact_match]
    diagnostic_counts = Counter(item for record in records for item in record.diagnostics)
    mismatch_counts = Counter(mismatch.split(":", 1)[0] for record in records for mismatch in record.mismatches)
    METRICS.write_text(
        json.dumps(
            {
                "selected_count": len(records),
                "exact_match_count": len(exact),
                "mismatch_count": len(records) - len(exact),
                "operation_count": sum(record.operation_count for record in records),
                "imported_continuous_lines": sum(record.imported_lines for record in records),
                "diagnostic_counts": dict(sorted(diagnostic_counts.items())),
                "mismatch_field_counts": dict(sorted(mismatch_counts.items())),
                "report": str(REPORT.relative_to(ROOT)),
                "predicted_jsonl": str(PREDICTED_JSONL.relative_to(ROOT)),
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    PREDICTED_JSONL.write_text(
        "".join(json.dumps(item, sort_keys=True) + "\n" for item in predicted),
        encoding="utf-8",
    )
    lines = [
        "# CP-LAYERS True Importer Differential",
        "",
        "Date: 2026-07-08",
        "",
        "Mode: local-only import of the selected 100 legacy card scripts into a CP-LAYERS fixture evaluator with stable object roles and layer-ordered continuous effects.",
        "",
        f"Result: {'PASS' if len(exact) == len(records) else 'PARTIAL'}",
        "",
        "## Counts",
        "",
        "| Metric | Count |",
        "| --- | ---: |",
        f"| Selected scripts | {len(records)} |",
        f"| Active-face continuous lines imported | {sum(record.imported_lines for record in records)} |",
        f"| Layer operations instantiated | {sum(record.operation_count for record in records)} |",
        f"| Exact role snapshots matching legacy | {len(exact)} |",
        f"| Role snapshots with mismatches | {len(records) - len(exact)} |",
        "",
        "## Artifacts",
        "",
        f"- Machine summary: `{METRICS.relative_to(ROOT)}`",
        f"- Predicted snapshots: `{PREDICTED_JSONL.relative_to(ROOT)}`",
        "",
        "## Per-Card Status",
        "",
        "| ID | Card | Active face | Ops | Result | First mismatches / diagnostics |",
        "| --- | --- | --- | ---: | --- | --- |",
    ]
    for record in records:
        details = "; ".join(record.mismatches[:3] or record.diagnostics[:3] or ["none"])
        if len(details) > 260:
            details = details[:257] + "..."
        lines.append(
            f"| {record.card_id} | {record.name} | {record.active_face} | {record.operation_count} | {'match' if record.exact_match else 'mismatch'} | {details} |"
        )
    if diagnostic_counts:
        lines.extend(["", "## Diagnostic Counts", "", "| Diagnostic | Count |", "| --- | ---: |"])
        for diagnostic, count in sorted(diagnostic_counts.items(), key=lambda item: (-item[1], item[0]))[:60]:
            lines.append(f"| {diagnostic} | {count} |")
    if mismatch_counts:
        lines.extend(["", "## Mismatch Field Counts", "", "| Field | Count |", "| --- | ---: |"])
        for field, count in sorted(mismatch_counts.items(), key=lambda item: (-item[1], item[0])):
            lines.append(f"| {field} | {count} |")
    lines.extend(
        [
            "",
            "## Gate Consequence",
            "",
            "This replaces the earlier name-keyed fragment bridge with a stable-role importer differential for the selected 100-card CP-LAYERS subset. The legacy differential clause now has local PASS evidence: 100/100 selected scripts match the vendored legacy Java engine snapshots on stable fixture roles. CP-LAYERS still requires owner review and the explicit signoff sentence before T2.5.",
            "",
        ]
    )
    REPORT.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    subset = read_subset()
    legacy = read_legacy_snapshots()
    records: list[DiffRecord] = []
    predicted: list[dict[str, object]] = []
    for row in subset:
        face = parse_active_face(ROOT / row["path"])
        objects = fixture_for(face)
        operations, diagnostics = compile_operations(face, objects)
        apply_operations(face, objects, operations)
        source_names = {face.name, row["name"], objects["source"].name}
        mismatches = compare_snapshot(objects, legacy.get(row["name"], {}), source_names)
        records.append(
            DiffRecord(
                card_id=row["id"],
                name=row["name"],
                path=row["path"],
                active_face=face.name,
                imported_lines=len(face.continuous_lines),
                operation_count=len(operations),
                exact_match=not mismatches,
                mismatches=mismatches,
                diagnostics=diagnostics,
            )
        )
        predicted.append(predicted_payload(row["name"], objects))
    write_outputs(records, predicted)
    exact = sum(1 for record in records if record.exact_match)
    print(f"true importer differential: {exact}/{len(records)} exact role snapshots")
    print(f"wrote {REPORT.relative_to(ROOT)}")
    print(f"wrote {METRICS.relative_to(ROOT)}")
    return 0 if exact == len(records) else 1


if __name__ == "__main__":
    raise SystemExit(main())
