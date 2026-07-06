#!/usr/bin/env python3
"""Mine the vendored legacy Forge card scripts for T0.6 inventory data."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple


ABILITY_PREFIXES = ("A", "T", "R", "S")
API_SELECTOR_KEYS = {"AB", "DB", "SP", "Mode", "Event"}
CARD_ROOT_CANDIDATES = (
    Path("forge-gui/res/cardsfolder"),
    Path("res/cardsfolder"),
)
EDITION_ROOT_CANDIDATES = (
    Path("forge-gui/res/editions"),
    Path("res/editions"),
)
TOKEN_RE = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")


class LegacyMineError(RuntimeError):
    """Raised for clear user-facing mining failures."""


@dataclass
class PipeField:
    key: Optional[str]
    value: str


@dataclass
class AbilityLine:
    prefix: str
    api: str
    selector_key: Optional[str]
    params: List[PipeField]
    raw: str


@dataclass
class CardScript:
    path: Path
    relpath: str
    name: Optional[str] = None
    abilities: List[AbilityLine] = field(default_factory=list)
    keywords: List[str] = field(default_factory=list)
    svars: Dict[str, str] = field(default_factory=dict)
    set_codes: List[str] = field(default_factory=list)
    warnings: List[str] = field(default_factory=list)


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Parse legacy Forge card scripts and emit the T0.6 inventory JSON "
            "and Markdown report."
        )
    )
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path("vendor/legacy-forge"),
        help="Path to the vendored Card-Forge/forge checkout.",
    )
    parser.add_argument(
        "--out",
        type=Path,
        nargs=2,
        metavar=("JSON", "MARKDOWN"),
        default=(Path("metrics/legacy_inventory.json"), Path("docs/legacy_inventory.md")),
        help="Output paths for the JSON metrics and Markdown report.",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=40,
        help="Number of rows to render in top-frequency Markdown tables.",
    )
    return parser.parse_args(argv)


def find_required_dir(root: Path, candidates: Iterable[Path], label: str) -> Path:
    searched = []
    for relpath in candidates:
        candidate = root / relpath
        searched.append(candidate)
        if candidate.is_dir():
            return candidate
    joined = ", ".join(str(path) for path in searched)
    raise LegacyMineError(f"{label} directory not found; searched: {joined}")


def find_optional_dir(root: Path, candidates: Iterable[Path]) -> Optional[Path]:
    for relpath in candidates:
        candidate = root / relpath
        if candidate.is_dir():
            return candidate
    return None


def split_pipe_fields(payload: str) -> List[PipeField]:
    fields: List[PipeField] = []
    for raw_part in payload.split("|"):
        part = raw_part.strip()
        if not part:
            continue
        if "$" in part:
            key, value = part.split("$", 1)
            fields.append(PipeField(key=key.strip() or None, value=value.strip()))
        else:
            fields.append(PipeField(key=None, value=part))
    return fields


def normalize_api(value: str) -> str:
    value = " ".join(value.strip().split())
    return value if value else "<empty>"


def parse_ability(prefix: str, payload: str) -> AbilityLine:
    fields = split_pipe_fields(payload)
    selector_index: Optional[int] = None
    selector_key: Optional[str] = None
    api = "<unparsed>"

    if fields:
        first = fields[0]
        if first.key is not None:
            selector_index = 0
            selector_key = first.key
            api = normalize_api(first.value)
        elif first.value:
            api = normalize_api(first.value)

    params: List[PipeField] = []
    for index, field_value in enumerate(fields):
        if index == selector_index:
            continue
        if field_value.key is not None:
            params.append(field_value)

    if selector_key and selector_key not in API_SELECTOR_KEYS and prefix != "A":
        api = f"{selector_key}={api}"

    return AbilityLine(
        prefix=prefix,
        api=api,
        selector_key=selector_key,
        params=params,
        raw=payload,
    )


def normalize_keyword(payload: str) -> str:
    head = payload.strip().split("|", 1)[0].strip()
    if ":" in head:
        head = head.split(":", 1)[0].strip()
    return head if head else "<empty>"


def extract_set_codes(payload: str) -> List[str]:
    codes: List[str] = []
    for record in re.split(r"\s*;\s*", payload.strip()):
        if not record:
            continue
        code = record.split("|", 1)[0].strip()
        if code:
            codes.append(code)
    return codes


def parse_svar(payload: str) -> Tuple[str, str]:
    if ":" in payload:
        name, value = payload.split(":", 1)
        return name.strip(), value.strip()
    return payload.strip(), ""


def parse_card_script(path: Path, cards_root: Path) -> CardScript:
    card = CardScript(path=path, relpath=path.relative_to(cards_root).as_posix())
    text = path.read_text(encoding="utf-8", errors="replace")

    for lineno, raw_line in enumerate(text.splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        if len(line) >= 2 and line[1] == ":" and line[0] in ABILITY_PREFIXES:
            ability = parse_ability(line[0], line[2:].strip())
            card.abilities.append(ability)
            if ability.api == "<unparsed>":
                card.warnings.append(f"{card.relpath}:{lineno}: unparsed ability line")
            continue

        if line.startswith("K:"):
            card.keywords.append(normalize_keyword(line[2:]))
            continue

        if line.startswith("SVar:"):
            name, value = parse_svar(line[len("SVar:") :])
            if name:
                card.svars[name] = value
            else:
                card.warnings.append(f"{card.relpath}:{lineno}: unnamed SVar")
            continue

        if line.startswith("SetInfo:"):
            card.set_codes.extend(extract_set_codes(line[len("SetInfo:") :]))
            continue

        if line.startswith("Name:"):
            card.name = line[len("Name:") :].strip() or None

    return card


def sorted_counter(counter: Counter) -> List[Tuple[str, int]]:
    return sorted(counter.items(), key=lambda item: (-item[1], item[0]))


def sorted_tuple_counter(counter: Counter) -> List[Tuple[Tuple[str, str], int]]:
    return sorted(counter.items(), key=lambda item: (-item[1], item[0][0], item[0][1]))


def counter_rows(counter: Counter, limit: Optional[int] = None) -> List[Dict[str, object]]:
    rows = [{"name": name, "count": count} for name, count in sorted_counter(counter)]
    return rows if limit is None else rows[:limit]


def pipe_value_type(value: str) -> str:
    first = value.split("|", 1)[0].strip()
    if "$" in first:
        key, _ = first.split("$", 1)
        return key.strip() or "<empty>"
    if not first:
        return "<empty>"
    return "<literal>"


def pipe_value_api(value: str) -> Optional[Tuple[str, str]]:
    first = value.split("|", 1)[0].strip()
    if "$" not in first:
        return None
    key, api = first.split("$", 1)
    key = key.strip()
    api = normalize_api(api)
    if key in {"AB", "DB", "SP"}:
        return key, api
    return None


def detect_svar_references(cards: Iterable[CardScript]) -> Tuple[int, Counter, Counter]:
    reference_total = 0
    reference_key_counter: Counter = Counter()
    reference_name_counter: Counter = Counter()

    for card in cards:
        svar_names = set(card.svars)
        if not svar_names:
            continue

        values_to_scan: List[Tuple[str, str]] = []
        for ability in card.abilities:
            for param in ability.params:
                values_to_scan.append((param.key or "<unknown>", param.value))
        for name, value in card.svars.items():
            values_to_scan.append((f"SVar:{name}", value))

        for key, value in values_to_scan:
            tokens = set(TOKEN_RE.findall(value))
            for svar_name in sorted(svar_names.intersection(tokens)):
                reference_total += 1
                reference_key_counter[key] += 1
                reference_name_counter[svar_name] += 1

    return reference_total, reference_key_counter, reference_name_counter


def parse_edition_code(lines: List[str], fallback: str) -> str:
    for raw_line in lines:
        line = raw_line.strip()
        for sep in (":", "="):
            prefix = f"Code{sep}"
            if line.startswith(prefix):
                code = line[len(prefix) :].strip()
                if code:
                    return code
    return fallback


def clean_card_list_candidate(line: str) -> str:
    candidate = line.strip()
    if not candidate:
        return ""
    if "|" in candidate:
        candidate = candidate.split("|", 1)[0].strip()
    if "@" in candidate:
        candidate = candidate.split("@", 1)[0].strip()
    if "\t" in candidate:
        candidate = candidate.split("\t", 1)[-1].strip()
    candidate = re.sub(r"^\d+[A-Za-z]?\s*[:.)-]?\s+", "", candidate).strip()
    candidate = re.sub(r"^(C|U|R|M|L|S|T)\s+", "", candidate).strip()
    candidate = re.sub(r"^\*\s*", "", candidate).strip()
    return candidate


def edition_card_candidates(lines: List[str]) -> Iterable[str]:
    in_cards_section = False
    for raw_line in lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]").lower()
            in_cards_section = section.startswith("cards") or section.startswith("card")
            continue

        lower_line = line.lower()
        if lower_line in {"cards:", "cards="}:
            in_cards_section = True
            continue

        if line.startswith("Name:") or line.startswith("Name="):
            if in_cards_section:
                yield line.split(line[4], 1)[1].strip()
            continue

        if (
            line.startswith("Card:")
            or line.startswith("Card=")
            or line.startswith("CardName:")
            or line.startswith("CardName=")
        ):
            yield re.split(r"[:=]", line, maxsplit=1)[1].strip()
            continue

        if in_cards_section:
            candidate = clean_card_list_candidate(line)
            if candidate:
                yield candidate


def mine_edition_set_counts(editions_root: Optional[Path], card_names: Iterable[str]) -> Counter:
    set_counts: Counter = Counter()
    if editions_root is None:
        return set_counts

    known_names = {name for name in card_names if name}
    if not known_names:
        return set_counts

    for edition_path in sorted(editions_root.rglob("*.txt")):
        if not edition_path.is_file():
            continue
        lines = edition_path.read_text(encoding="utf-8", errors="replace").splitlines()
        code = parse_edition_code(lines, edition_path.stem)
        matched_names = {
            candidate for candidate in edition_card_candidates(lines) if candidate in known_names
        }
        if matched_names:
            set_counts[code] += len(matched_names)

    return set_counts


def build_inventory(repo: Path, cards_root: Path, editions_root: Optional[Path]) -> Dict[str, object]:
    script_paths = sorted(path for path in cards_root.rglob("*.txt") if path.is_file())
    if not script_paths:
        raise LegacyMineError(f"no card script .txt files found under {cards_root}")

    cards = [parse_card_script(path, cards_root) for path in script_paths]

    ability_line_counts: Counter = Counter()
    api_counts: Counter = Counter()
    selector_key_counts: Counter = Counter()
    api_param_counts: Dict[Tuple[str, str], Counter] = defaultdict(Counter)
    keyword_counts: Counter = Counter()
    setinfo_counts: Counter = Counter()
    svar_name_counts: Counter = Counter()
    svar_value_type_counts: Counter = Counter()
    svar_ability_api_counts: Counter = Counter()
    warning_samples: List[str] = []
    missing_setinfo_scripts = 0
    missing_name_scripts = 0

    for card in cards:
        if not card.name:
            missing_name_scripts += 1
        if card.warnings and len(warning_samples) < 20:
            warning_samples.extend(card.warnings[: 20 - len(warning_samples)])

        if card.set_codes:
            for set_code in card.set_codes:
                setinfo_counts[set_code] += 1
        else:
            missing_setinfo_scripts += 1

        for ability in card.abilities:
            ability_line_counts[ability.prefix] += 1
            api_key = (ability.prefix, ability.api)
            api_counts[api_key] += 1
            if ability.selector_key:
                selector_key_counts[ability.selector_key] += 1
            for param in ability.params:
                if param.key:
                    api_param_counts[api_key][param.key] += 1

        for keyword in card.keywords:
            keyword_counts[keyword] += 1

        for name, value in card.svars.items():
            svar_name_counts[name] += 1
            svar_value_type_counts[pipe_value_type(value)] += 1
            value_api = pipe_value_api(value)
            if value_api:
                kind, api = value_api
                svar_ability_api_counts[f"{kind}:{api}"] += 1

    edition_counts = mine_edition_set_counts(editions_root, (card.name for card in cards if card.name))
    if setinfo_counts:
        primary_set_counts = setinfo_counts
        set_count_source = "card_script_setinfo"
    elif edition_counts:
        primary_set_counts = edition_counts
        set_count_source = "edition_files_matched_to_script_names"
    else:
        primary_set_counts = Counter()
        set_count_source = "none_found"

    svar_reference_total, svar_reference_key_counts, svar_reference_name_counts = detect_svar_references(
        cards
    )

    api_rows: List[Dict[str, object]] = []
    for (prefix, api), count in sorted_tuple_counter(api_counts):
        param_counter = api_param_counts[(prefix, api)]
        api_rows.append(
            {
                "prefix": prefix,
                "api": api,
                "count": count,
                "parameter_keys": counter_rows(param_counter),
            }
        )

    return {
        "schema_version": 1,
        "legacy_repo": str(repo),
        "cards_root": str(cards_root),
        "editions_root": str(editions_root) if editions_root else None,
        "total_scripts": len(cards),
        "missing_name_scripts": missing_name_scripts,
        "ability_lines_total": sum(ability_line_counts.values()),
        "ability_lines_by_prefix": {
            prefix: ability_line_counts[prefix] for prefix in ABILITY_PREFIXES
        },
        "ability_selector_key_frequency": counter_rows(selector_key_counts),
        "ability_api_frequency": api_rows,
        "keyword_frequency": counter_rows(keyword_counts),
        "sets": {
            "source": set_count_source,
            "per_set_counts": counter_rows(primary_set_counts),
            "script_setinfo_counts": counter_rows(setinfo_counts),
            "edition_file_counts": counter_rows(edition_counts),
            "missing_setinfo_scripts": missing_setinfo_scripts,
        },
        "svar": {
            "definitions_total": sum(svar_name_counts.values()),
            "unique_names": len(svar_name_counts),
            "name_frequency": counter_rows(svar_name_counts),
            "value_type_frequency": counter_rows(svar_value_type_counts),
            "ability_api_frequency": counter_rows(svar_ability_api_counts),
            "reference_total": svar_reference_total,
            "reference_key_frequency": counter_rows(svar_reference_key_counts),
            "reference_name_frequency": counter_rows(svar_reference_name_counts),
        },
        "parse_warnings": {
            "count": sum(len(card.warnings) for card in cards),
            "samples": warning_samples,
        },
    }


def md_escape(value: object) -> str:
    text = str(value)
    return text.replace("|", r"\|").replace("\n", " ")


def format_param_keys(params: List[Dict[str, object]], limit: int = 6) -> str:
    if not params:
        return ""
    rendered = [f"{item['name']} ({item['count']})" for item in params[:limit]]
    return ", ".join(rendered)


def render_count_table(rows: List[Dict[str, object]], name_header: str, limit: int) -> str:
    lines = [f"| Rank | {name_header} | Count |", "| ---: | --- | ---: |"]
    for index, row in enumerate(rows[:limit], start=1):
        lines.append(f"| {index} | {md_escape(row['name'])} | {row['count']} |")
    if len(lines) == 2:
        lines.append("|  | No data found | 0 |")
    return "\n".join(lines)


def render_markdown(inventory: Dict[str, object], top: int) -> str:
    sets = inventory["sets"]
    svar = inventory["svar"]
    parse_warnings = inventory["parse_warnings"]
    ability_rows = inventory["ability_api_frequency"]

    lines = [
        "# Legacy Inventory",
        "",
        "Generated by `tools/mine_legacy.py`.",
        "",
        "## Summary",
        "",
        f"- Legacy repo: `{inventory['legacy_repo']}`",
        f"- Card script root: `{inventory['cards_root']}`",
        f"- Total card scripts: {inventory['total_scripts']}",
        f"- Ability lines: {inventory['ability_lines_total']}",
        f"- Keyword entries: {sum(row['count'] for row in inventory['keyword_frequency'])}",
        f"- SVar definitions: {svar['definitions_total']}",
        f"- Set count source: `{sets['source']}`",
        f"- Scripts without `SetInfo:`: {sets['missing_setinfo_scripts']}",
        f"- Parse warnings: {parse_warnings['count']}",
        "",
        "## Ability Lines By Prefix",
        "",
        "| Prefix | Count |",
        "| --- | ---: |",
    ]

    for prefix, count in inventory["ability_lines_by_prefix"].items():
        lines.append(f"| `{prefix}:` | {count} |")

    lines.extend(
        [
            "",
            f"## Top {top} Ability APIs",
            "",
            "| Rank | Prefix | API | Count | Top parameter keys |",
            "| ---: | --- | --- | ---: | --- |",
        ]
    )
    for index, row in enumerate(ability_rows[:top], start=1):
        lines.append(
            "| {rank} | `{prefix}:` | `{api}` | {count} | {params} |".format(
                rank=index,
                prefix=md_escape(row["prefix"]),
                api=md_escape(row["api"]),
                count=row["count"],
                params=md_escape(format_param_keys(row["parameter_keys"])),
            )
        )
    if not ability_rows:
        lines.append("|  |  | No data found | 0 |  |")

    lines.extend(
        [
            "",
            f"## Top {top} Keywords",
            "",
            render_count_table(inventory["keyword_frequency"], "Keyword", top),
            "",
            "## SVar Usage",
            "",
            f"- Unique SVar names: {svar['unique_names']}",
            f"- Detected SVar references: {svar['reference_total']}",
            "",
            f"### Top {top} SVar Value Types",
            "",
            render_count_table(svar["value_type_frequency"], "Value type", top),
            "",
            f"### Top {top} SVar Ability APIs",
            "",
            render_count_table(svar["ability_api_frequency"], "Ability API", top),
            "",
            f"### Top {top} Referenced SVar Names",
            "",
            render_count_table(svar["reference_name_frequency"], "SVar", top),
            "",
            "## Scripts Per Set",
            "",
            render_count_table(sets["per_set_counts"], "Set", len(sets["per_set_counts"])),
        ]
    )

    if parse_warnings["samples"]:
        lines.extend(["", "## Parse Warning Samples", ""])
        for warning in parse_warnings["samples"]:
            lines.append(f"- `{md_escape(warning)}`")

    lines.append("")
    return "\n".join(lines)


def write_outputs(inventory: Dict[str, object], json_path: Path, markdown_path: Path, top: int) -> None:
    json_path.parent.mkdir(parents=True, exist_ok=True)
    markdown_path.parent.mkdir(parents=True, exist_ok=True)

    json_path.write_text(
        json.dumps(inventory, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    markdown_path.write_text(render_markdown(inventory, top), encoding="utf-8")


def run(argv: Optional[Sequence[str]] = None) -> int:
    args = parse_args(argv)
    repo = args.repo
    json_path, markdown_path = args.out

    if not repo.exists():
        raise LegacyMineError(
            f"legacy Forge repository not found at {repo}. "
            "Run T0.5 first, for example: "
            "git submodule add https://github.com/Card-Forge/forge vendor/legacy-forge"
        )
    if not repo.is_dir():
        raise LegacyMineError(f"legacy Forge repository path is not a directory: {repo}")

    cards_root = find_required_dir(repo, CARD_ROOT_CANDIDATES, "legacy card script")
    editions_root = find_optional_dir(repo, EDITION_ROOT_CANDIDATES)
    inventory = build_inventory(repo, cards_root, editions_root)
    write_outputs(inventory, json_path, markdown_path, args.top)
    print(f"wrote {json_path} and {markdown_path}")
    return 0


def main() -> int:
    try:
        return run()
    except LegacyMineError as exc:
        print(f"mine_legacy.py: error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
