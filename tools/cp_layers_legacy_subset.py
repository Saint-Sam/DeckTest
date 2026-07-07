#!/usr/bin/env python3
"""Build a local-only CP-LAYERS legacy 100-card subset report.

This is a static differential readiness/adjudication pass over vendored legacy
Forge scripts. It uses no network and does not execute the Java legacy engine.
"""

from __future__ import annotations

import csv
import re
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CARD_ROOT = ROOT / "vendor" / "legacy-forge" / "forge-gui" / "res" / "cardsfolder"
OUT_DIR = ROOT / "reports" / "gates" / "CP-LAYERS"
CSV_OUT = OUT_DIR / "legacy-100-layered-subset-2026-07-07.csv"
MD_OUT = OUT_DIR / "legacy-100-layered-subset-2026-07-07.md"
TARGET_COUNT = 100


ANCHOR_NAMES = [
    "Humility",
    "Opalescence",
    "Blood Moon",
    "Song of the Dryads",
    "Darksteel Mutation",
    "Clone",
    "Vesuvan Shapeshifter",
    "Archetype of Imagination",
    "Archetype of Endurance",
    "Archetype of Courage",
    "March of the Machines",
    "Magus of the Moon",
    "Imprisoned in the Moon",
    "Kenrith's Transformation",
    "Dress Down",
    "Ichthyomorphosis",
    "Witness Protection",
    "Kasmina's Transmutation",
    "Mystic Subdual",
    "Frogify",
]


LAND_SUBTYPES = {
    "Plains",
    "Island",
    "Swamp",
    "Mountain",
    "Forest",
    "Wastes",
    "Desert",
    "Gate",
    "Locus",
    "Urza",
}

CARD_TYPES = {
    "Artifact",
    "Creature",
    "Enchantment",
    "Instant",
    "Land",
    "Planeswalker",
    "Sorcery",
    "Battle",
    "Kindred",
    "Tribal",
}

SUPPORTED_KEYWORDS = {
    "First Strike",
    "Double Strike",
    "Trample",
    "Deathtouch",
    "Lifelink",
    "Flying",
    "Reach",
    "Menace",
    "Vigilance",
    "Haste",
}


@dataclass(frozen=True)
class LegacyCard:
    name: str
    relpath: str
    continuous: tuple[str, ...]
    oracle: str
    features: tuple[str, ...]
    gaps: tuple[str, ...]
    status: str
    score: int


def read_card(path: Path) -> tuple[str, list[str], str]:
    name = path.stem
    continuous: list[str] = []
    oracle = ""
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if line.startswith("Name:"):
            name = line.split(":", 1)[1].strip()
        elif line.startswith("S:") and "Mode$ Continuous" in line:
            continuous.append(line)
        elif line.startswith("Oracle:"):
            oracle = line.split(":", 1)[1].strip()
    return name, continuous, oracle


def value_after(raw: str, key: str) -> list[str]:
    values: list[str] = []
    pattern = re.compile(rf"{re.escape(key)}\$\s*([^|]+)")
    for match in pattern.finditer(raw):
        values.append(match.group(1).strip())
    return values


def is_numeric(value: str) -> bool:
    return bool(re.fullmatch(r"-?\d+", value.strip()))


def split_type_tokens(value: str) -> set[str]:
    return {
        token
        for token in re.split(r"[^A-Za-z]+", value)
        if token and token not in {"non", "nonBasic", "Other", "YouCtrl", "OppCtrl"}
    }


def classify(raw: str) -> tuple[tuple[str, ...], tuple[str, ...], int]:
    features: set[str] = set()
    gaps: set[str] = set()
    score = 0

    def add(feature: str, points: int = 1) -> None:
        nonlocal score
        features.add(feature)
        score += points

    if "Copy" in raw or "Clone" in raw or "CopyPermanent" in raw:
        add("copy", 8)
        gaps.add("copy is broader than current CopyBaseCreature model")
    if "GainControl" in raw or "GainControl" in raw or "Controller" in raw:
        add("control", 5)
    if "ChangeText" in raw or "AddText" in raw or "RemoveText" in raw:
        add("text", 5)
        gaps.add("real text-changing effects are not modeled")
    if "SetColor$" in raw or "AddColor$" in raw or "Colorless" in raw:
        add("color", 4)
    if "AddKeyword$" in raw or "RemoveKeyword$" in raw or "CantHaveKeyword$" in raw:
        add("keyword", 5)
        keyword_values = " ".join(value_after(raw, "AddKeyword") + value_after(raw, "RemoveKeyword"))
        if keyword_values:
            tokens = {part.strip() for part in re.split(r"[,&]", keyword_values) if part.strip()}
            if any(token not in SUPPORTED_KEYWORDS for token in tokens):
                gaps.add("keywords beyond current combat-keyword subset")
        if "CantHaveKeyword$" in raw:
            gaps.add("can't-have keyword suppression is not modeled")
    if "RemoveAllAbilities$" in raw:
        add("remove_all_abilities", 10)
        gaps.add("all-abilities removal is not modeled beyond explicit combat keywords")
    if (
        "AddType$" in raw
        or "RemoveCardTypes$" in raw
        or "RemoveType$" in raw
        or "RemoveSubTypes$" in raw
        or "RemoveLandTypes$" in raw
    ):
        add("type", 6)
        type_values = " ".join(value_after(raw, "AddType") + value_after(raw, "RemoveType"))
        type_tokens = split_type_tokens(type_values)
        if type_tokens & LAND_SUBTYPES or "RemoveLandTypes$" in raw:
            gaps.add("land subtypes/intrinsic mana abilities are not modeled")
        unsupported_card_types = type_tokens - CARD_TYPES - LAND_SUBTYPES
        if unsupported_card_types:
            gaps.add("subtypes/supertypes beyond ObjectTypes are not modeled")
    if "SetPower$" in raw or "SetToughness$" in raw:
        add("set_pt", 7)
        values = value_after(raw, "SetPower") + value_after(raw, "SetToughness")
        if any(not is_numeric(value) for value in values):
            gaps.add("dynamic P/T expressions are not modeled")
    if (
        "AddPower$" in raw
        or "AddToughness$" in raw
        or "PowerBoost$" in raw
        or "ToughnessBoost$" in raw
    ):
        add("modify_pt", 5)
        values = (
            value_after(raw, "AddPower")
            + value_after(raw, "AddToughness")
            + value_after(raw, "PowerBoost")
            + value_after(raw, "ToughnessBoost")
        )
        if any(value and not is_numeric(value) for value in values):
            gaps.add("dynamic P/T modifiers are not modeled")
    if raw.count("S:Mode$ Continuous") > 1:
        add("multi_continuous", 2)
    if "Affected$" in raw and ("Valid" in raw or "+" in raw or "." in raw):
        add("predicate_target", 3)
        gaps.add("legacy predicate targets are not modeled")
    if "Depends" in raw or "Dependency" in raw:
        add("dependency", 5)

    if not gaps:
        status = "represented_by_current_model"
    elif features & {"type", "color", "keyword", "set_pt", "modify_pt", "control", "copy"}:
        status = "partial_divergence_adjudicated"
    else:
        status = "blocked_unrepresented"
    return tuple(sorted(features)), tuple(sorted(gaps)), score


def collect_cards() -> list[LegacyCard]:
    cards: list[LegacyCard] = []
    for path in sorted(CARD_ROOT.rglob("*.txt")):
        name, continuous, oracle = read_card(path)
        if not continuous:
            continue
        raw = "\n".join(continuous)
        features, gaps, score = classify(raw)
        if not features:
            continue
        relpath = path.relative_to(ROOT).as_posix()
        anchor_bonus = 50 if name in ANCHOR_NAMES else 0
        cards.append(
            LegacyCard(
                name=name,
                relpath=relpath,
                continuous=tuple(continuous),
                oracle=oracle,
                features=features,
                gaps=gaps,
                status=status_from_gaps(gaps),
                score=score + anchor_bonus,
            )
        )
    return cards


def status_from_gaps(gaps: tuple[str, ...]) -> str:
    if not gaps:
        return "represented_by_current_model"
    return "partial_divergence_adjudicated"


def choose_subset(cards: list[LegacyCard]) -> list[LegacyCard]:
    anchors = {name: card for card in cards for name in [card.name] if name in ANCHOR_NAMES}
    selected: list[LegacyCard] = [anchors[name] for name in ANCHOR_NAMES if name in anchors]
    seen = {card.relpath for card in selected}
    for card in sorted(cards, key=lambda card: (-card.score, card.name, card.relpath)):
        if len(selected) >= TARGET_COUNT:
            break
        if card.relpath in seen:
            continue
        selected.append(card)
        seen.add(card.relpath)
    return selected


def write_csv(cards: list[LegacyCard]) -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    with CSV_OUT.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["id", "name", "status", "features", "gaps", "path"])
        for index, card in enumerate(cards, start=1):
            writer.writerow(
                [
                    f"L{index:03d}",
                    card.name,
                    card.status,
                    "; ".join(card.features),
                    "; ".join(card.gaps) if card.gaps else "none",
                    card.relpath,
                ]
            )


def write_markdown(cards: list[LegacyCard]) -> None:
    feature_counts = Counter(feature for card in cards for feature in card.features)
    gap_counts = Counter(gap for card in cards for gap in card.gaps)
    status_counts = Counter(card.status for card in cards)
    lines = [
        "# CP-LAYERS Legacy 100-Card Layered Subset",
        "",
        "Date: 2026-07-07",
        "",
        "Mode: local-only static differential readiness pass over vendored legacy Forge scripts.",
        "No network, download, or upstream fetch was used.",
        "",
        "Result: BLOCKED FOR TRUE ENGINE DIFFERENTIAL.",
        "",
        "Forge 2.0 currently has a data-only layer substrate and RON oracle harness,",
        "but no legacy card-script importer/card compiler capable of executing these",
        "100 real legacy scripts in the new engine. This pass therefore selects the",
        "100-card layered subset and adjudicates script-level divergence categories.",
        "",
        "## Status Counts",
        "",
        "| Status | Count |",
        "| --- | ---: |",
    ]
    for status, count in sorted(status_counts.items()):
        lines.append(f"| {status} | {count} |")
    lines.extend(["", "## Feature Counts", "", "| Feature | Count |", "| --- | ---: |"])
    for feature, count in sorted(feature_counts.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"| {feature} | {count} |")
    lines.extend(["", "## Divergence Categories", "", "| Category | Count |", "| --- | ---: |"])
    for gap, count in sorted(gap_counts.items(), key=lambda item: (-item[1], item[0])):
        lines.append(f"| {gap} | {count} |")
    if not gap_counts:
        lines.append("| none | 0 |")
    lines.extend(["", "## Selected Cards", "", "| ID | Name | Status | Features | Gaps | Path |", "| --- | --- | --- | --- | --- | --- |"])
    for index, card in enumerate(cards, start=1):
        gaps = "; ".join(card.gaps) if card.gaps else "none"
        features = "; ".join(card.features)
        lines.append(
            f"| L{index:03d} | {card.name} | {card.status} | {features} | {gaps} | `{card.relpath}` |"
        )
    lines.extend(
        [
            "",
            "## Adjudication",
            "",
            "The selected local legacy subset is valid as a review corpus, but CP-LAYERS",
            "cannot honestly pass the legacy differential clause until either:",
            "",
            "- Forge 2.0 gains enough card compiler/import support to execute these scripts,",
            "- the owner explicitly de-scopes the 100-card engine differential for this checkpoint, or",
            "- the checkpoint is failed/reopened with remediation tasks for the missing layer semantics.",
        ]
    )
    MD_OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    cards = collect_cards()
    subset = choose_subset(cards)
    if len(subset) < TARGET_COUNT:
        raise SystemExit(f"only found {len(subset)} layered cards")
    write_csv(subset)
    write_markdown(subset)
    print(f"WROTE {CSV_OUT}")
    print(f"WROTE {MD_OUT}")


if __name__ == "__main__":
    main()
