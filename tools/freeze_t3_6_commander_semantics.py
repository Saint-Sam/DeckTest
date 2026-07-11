#!/usr/bin/env python3
"""Freeze the bounded T3.6 Commander semantic-candidate manifest."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
from collections import Counter
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Callable


ROOT = Path(__file__).resolve().parents[1]
PRIORITY_PATH = Path("assets/coverage_priority.txt")
CATALOG_PATH = Path("assets/card_catalog.json")
PRIORITY_COVERAGE_PATH = Path("metrics/priority_coverage.json")
TRANSLATION_PATH = Path("metrics/translation.json")
DEFAULT_OUTPUT_PATH = Path("assets/t3_6_commander_semantic_candidates.json")
SELECTION_LIMIT = 100
ALGORITHM_VERSION = "t3.6-freeze-v1"
CLAIM_BOUNDARY = (
    "Selection is not semantic verification. This manifest freezes candidates "
    "for later card-specific tests; no selected identity is semantic_verified "
    "by this artifact."
)


def operation(pattern: str) -> re.Pattern[str]:
    return re.compile(
        rf"(?:AB|SP|DB)\$\s*(?:{pattern})(?:\s|\||$)",
        re.IGNORECASE | re.MULTILINE,
    )


MANA_OPERATION = operation("Mana")
CHANGE_ZONE_OPERATION = operation("ChangeZone")
ANY_TRIGGER = re.compile(r"^T:", re.MULTILINE)
FIELD_PAIR = re.compile(r"(?:^|[|:])\s*([A-Za-z][A-Za-z0-9]*)\$\s*([^|]+)")
LAND_WORDS = {"land", "plains", "island", "swamp", "mountain", "forest", "wastes"}


def field_records(script: str) -> list[dict[str, str]]:
    records: list[dict[str, str]] = []
    for line in script.splitlines():
        fields = {key: value.strip() for key, value in FIELD_PAIR.findall(line)}
        if fields:
            records.append(fields)
    return records


def operation_name(fields: dict[str, str]) -> str | None:
    for key in ("AB", "SP", "DB"):
        if key in fields:
            return fields[key].casefold()
    return None


def has_operation(script: str, *names: str) -> bool:
    expected = {name.casefold() for name in names}
    return any(operation_name(fields) in expected for fields in field_records(script))


def field_words(value: str) -> set[str]:
    return {word.casefold() for word in re.findall(r"[A-Za-z]+", value)}


def stack_interaction(script: str) -> bool:
    return bool(
        has_operation(script, "Counter")
        or re.search(
            r"(?:hexproof|indestructible)", script, re.IGNORECASE | re.MULTILINE
        )
    )


def land_ramp(script: str) -> bool:
    return any(
        operation_name(fields) == "changezone"
        and any(
            bool(field_words(fields.get(key, "")) & LAND_WORDS)
            for key in ("ChangeType", "ValidCard", "ValidTgts")
        )
        for fields in field_records(script)
    )


def targeted_interaction(script: str) -> bool:
    if has_operation(script, "Destroy"):
        return True
    return any(
        operation_name(fields) == "changezone"
        and fields.get("Origin", "").casefold() == "battlefield"
        and fields.get("Destination", "").casefold()
        in {"exile", "hand", "library", "graveyard"}
        and bool(fields.get("ValidTgts"))
        for fields in field_records(script)
    )


def card_flow(script: str) -> bool:
    return has_operation(script, "Draw", "Scry", "Discard")


def library_or_graveyard_access(script: str) -> bool:
    return bool(
        not land_ramp(script)
        and any(
            operation_name(fields) == "changezone"
            and fields.get("Origin", "").casefold() in {"library", "graveyard"}
            for fields in field_records(script)
        )
    )


def token_or_sacrifice(script: str) -> bool:
    return bool(
        has_operation(script, "Token")
        or re.search(r"(?:TokenScript\$|\bSac<|\bSacrifice\b)", script, re.IGNORECASE)
    )


def triggered_synergy(script: str) -> bool:
    return bool(ANY_TRIGGER.search(script))


def continuous_combat_or_cost(script: str) -> bool:
    return any(
        fields.get("Mode", "").casefold() in {"continuous", "attacks"}
        or operation_name(fields)
        in {"pumpall", "attach", "putcounter", "removecounter", "reducecost"}
        for fields in field_records(script)
    )


def mana_source(script: str) -> bool:
    return bool(
        has_operation(script, "Mana")
        or re.search(r"^Types:Basic Land(?:\s|$)", script, re.MULTILINE)
    )


@dataclass(frozen=True)
class Stratum:
    id: str
    quota: int
    description: str
    predicate: str
    matches: Callable[[str], bool]


# Allocation order is narrow-to-broad. Within each stratum, repository priority
# rank is the sole ordering key.
STRATA = (
    Stratum(
        "stack_interaction_and_protection",
        8,
        "Counterspells and effects that grant hexproof or indestructible.",
        "legacy script has AB/SP/DB$ Counter or a hexproof/indestructible marker",
        stack_interaction,
    ),
    Stratum(
        "targeted_permanent_interaction",
        14,
        "Targeted destruction or battlefield-to-zone movement.",
        "legacy script has AB/SP/DB$ Destroy, or targeted battlefield ChangeZone to exile/hand/library/graveyard",
        targeted_interaction,
    ),
    Stratum(
        "land_ramp_and_search",
        10,
        "Effects that move a land card between zones as mana development.",
        "legacy script has AB/SP/DB$ ChangeZone with a Land ChangeType/ValidCard/ValidTgts marker",
        land_ramp,
    ),
    Stratum(
        "card_flow",
        14,
        "Draw, scry, or discard effects.",
        "legacy script has AB/SP/DB$ Draw, Scry, or Discard",
        card_flow,
    ),
    Stratum(
        "library_and_graveyard_access",
        10,
        "Non-land tutoring, reanimation, recovery, and graveyard setup.",
        "legacy script has non-land AB/SP/DB$ ChangeZone with Origin$ Library or Graveyard",
        library_or_graveyard_access,
    ),
    Stratum(
        "token_and_sacrifice",
        10,
        "Token creation and sacrifice-driven effects or costs.",
        "legacy script has AB/SP/DB$ Token, TokenScript$, Sac<...>, or Sacrifice text",
        token_or_sacrifice,
    ),
    Stratum(
        "triggered_synergies",
        10,
        "Zone, spell, combat, damage, and phase-triggered behavior.",
        "legacy script contains at least one T: trigger line",
        triggered_synergy,
    ),
    Stratum(
        "continuous_combat_and_costs",
        10,
        "Continuous modifiers, combat hooks, attachments, counters, and cost reduction.",
        "legacy script has Continuous/PumpAll/Attach/counter/Attacks/ReduceCost or a closed combat keyword marker",
        continuous_combat_or_cost,
    ),
    Stratum(
        "mana_sources",
        14,
        "Basic lands and activated or triggered mana production.",
        "legacy script has a Basic Land type or AB/SP/DB$ Mana",
        mana_source,
    ),
)


EXCLUSION_REASONS = {
    "CATALOG_IDENTITY_MISSING": "No catalog identity matches the translation report's catalog name.",
    "CATALOG_IDENTITY_NOT_IN_SCOPE": "Only catalog-only or out-of-scope identities match the catalog name.",
    "CATALOG_IDENTITY_AMBIGUOUS": "More than one in-scope catalog identity matches the catalog name.",
    "TRANSLATION_NOT_EMITTED": "The current translation report does not contain a compiler-emitted definition.",
    "DUPLICATE_ORACLE_IDENTITY": "An earlier priority entry already resolves to this Oracle identity.",
    "NO_SEMANTIC_STRATUM_MATCH": "The emitted legacy script matches none of the frozen semantic predicates.",
    "STRATUM_QUOTA_FILLED": "The entry matched a stratum whose higher-priority quota was already filled.",
}


@dataclass(frozen=True)
class PriorityEntry:
    priority_rank: int
    tier: int
    tier_label: str
    tier_rank: int
    requested_name: str


@dataclass(frozen=True)
class Candidate:
    priority: PriorityEntry
    catalog_name: str
    oracle_id: str
    classification: str
    legacy_path: str
    legacy_sha256: str
    matching_strata: tuple[str, ...]


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=True, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def payload_sha256(manifest: dict[str, object]) -> str:
    payload = copy.deepcopy(manifest)
    payload.pop("payload_sha256", None)
    return sha256_bytes(canonical_json_bytes(payload))


def render_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )


def load_json(path: Path) -> dict[str, object]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object in {path}")
    return value


def require_int(value: object, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{label} must be an integer")
    return value


def require_str(value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} must be a non-empty string")
    return value


def parse_priority(text: str) -> list[PriorityEntry]:
    entries: list[PriorityEntry] = []
    current_tier: int | None = None
    current_label = ""
    tier_rank = 0
    seen_names: set[str] = set()
    tier_header = re.compile(r"^#\s*TIER\s+(\d+)\s*-\s*(.+?)\s*$", re.IGNORECASE)

    for line_number, raw_line in enumerate(text.splitlines(), 1):
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("#"):
            match = tier_header.match(line)
            if not match:
                raise ValueError(f"invalid priority header on line {line_number}: {line}")
            tier = int(match.group(1))
            if current_tier is not None and tier <= current_tier:
                raise ValueError("priority tiers must be strictly increasing")
            current_tier = tier
            current_label = match.group(2).strip()
            tier_rank = 0
            continue
        if current_tier is None:
            raise ValueError(f"priority names precede a tier header on line {line_number}")
        names = [name.strip() for name in line.split("|")]
        if not names or any(not name for name in names):
            raise ValueError(f"empty priority name on line {line_number}")
        for name in names:
            folded = name.casefold()
            if folded in seen_names:
                raise ValueError(f"duplicate priority name: {name}")
            seen_names.add(folded)
            entries.append(
                PriorityEntry(
                    priority_rank=len(entries) + 1,
                    tier=current_tier,
                    tier_label=current_label,
                    tier_rank=tier_rank + 1,
                    requested_name=name,
                )
            )
            tier_rank += 1
    if not entries:
        raise ValueError("priority list is empty")
    return entries


def flatten_priority_coverage(
    coverage: dict[str, object], entries: list[PriorityEntry]
) -> list[dict[str, object]]:
    if require_int(coverage.get("schema_version"), "priority schema_version") != 1:
        raise ValueError("unsupported priority coverage schema")
    tiers = coverage.get("tiers")
    if not isinstance(tiers, list):
        raise ValueError("priority coverage tiers must be an array")
    flattened: list[dict[str, object]] = []
    entry_offset = 0
    for tier_value in tiers:
        if not isinstance(tier_value, dict):
            raise ValueError("priority tier must be an object")
        tier = require_int(tier_value.get("tier"), "priority tier")
        label = require_str(tier_value.get("label"), "priority tier label")
        cards = tier_value.get("cards")
        if not isinstance(cards, list):
            raise ValueError(f"priority tier {tier} cards must be an array")
        if require_int(tier_value.get("requested"), f"tier {tier} requested") != len(cards):
            raise ValueError(f"priority tier {tier} requested count is stale")
        for card in cards:
            if not isinstance(card, dict):
                raise ValueError("priority card result must be an object")
            if entry_offset >= len(entries):
                raise ValueError("priority coverage has extra cards")
            expected = entries[entry_offset]
            if tier != expected.tier or label != expected.tier_label:
                raise ValueError(f"priority tier metadata mismatch at rank {entry_offset + 1}")
            if card.get("requested_name") != expected.requested_name:
                raise ValueError(f"priority order mismatch at rank {entry_offset + 1}")
            flattened.append(card)
            entry_offset += 1
    if entry_offset != len(entries):
        raise ValueError("priority coverage omits priority cards")
    if require_int(coverage.get("total_requested"), "priority total_requested") != len(
        entries
    ):
        raise ValueError("priority coverage total_requested is stale")
    if coverage.get("source_path") != PRIORITY_PATH.as_posix():
        raise ValueError("priority coverage source_path is not the frozen priority input")
    return flattened


def classification_name(value: object) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict) and len(value) == 1:
        return require_str(next(iter(value)), "catalog classification key")
    raise ValueError("catalog classification must be a string or one-key object")


def catalog_indexes(
    catalog: dict[str, object],
) -> tuple[dict[str, list[dict[str, object]]], dict[str, list[dict[str, object]]]]:
    if require_int(catalog.get("schema_version"), "catalog schema_version") != 1:
        raise ValueError("unsupported card catalog schema")
    identities = catalog.get("identities")
    if not isinstance(identities, list):
        raise ValueError("catalog identities must be an array")
    all_by_name: dict[str, list[dict[str, object]]] = {}
    in_scope_by_name: dict[str, list[dict[str, object]]] = {}
    seen_ids: set[str] = set()
    for identity in identities:
        if not isinstance(identity, dict):
            raise ValueError("catalog identity must be an object")
        oracle_id = require_str(identity.get("id"), "catalog identity id")
        name = require_str(identity.get("name"), "catalog identity name")
        if oracle_id in seen_ids:
            raise ValueError(f"duplicate catalog identity: {oracle_id}")
        seen_ids.add(oracle_id)
        all_by_name.setdefault(name, []).append(identity)
        if classification_name(identity.get("classification")) in {
            "UnverifiedPlayable",
            "VerifiedPlayable",
        }:
            in_scope_by_name.setdefault(name, []).append(identity)
    return all_by_name, in_scope_by_name


def safe_legacy_path(root: Path, source_root: str, relative_path: str) -> Path:
    source = PurePosixPath(source_root)
    relative = PurePosixPath(relative_path)
    if source.is_absolute() or relative.is_absolute():
        raise ValueError("legacy source paths must be repository-relative")
    if ".." in source.parts or ".." in relative.parts:
        raise ValueError("legacy source paths may not traverse parents")
    if relative.suffix != ".txt":
        raise ValueError(f"legacy card path is not a .txt script: {relative_path}")
    path = root.joinpath(*source.parts, *relative.parts)
    if not path.is_file():
        raise ValueError(f"missing emitted legacy script: {path}")
    return path


def validate_translation_summary(
    translation: dict[str, object], coverage: dict[str, object], cards: list[dict[str, object]]
) -> tuple[str, str, str]:
    if require_int(translation.get("schema_version"), "translation schema_version") != 2:
        raise ValueError("unsupported translation schema")
    status_counts = Counter(require_str(card.get("status"), "translation status") for card in cards)
    expected = {
        "priority_requested": len(cards),
        "priority_catalog_resolved": require_int(
            coverage.get("catalog_resolved"), "priority catalog_resolved"
        ),
        "priority_emitted": status_counts["emitted"],
    }
    for field, expected_value in expected.items():
        if require_int(translation.get(field), f"translation {field}") != expected_value:
            raise ValueError(f"translation {field} does not match priority coverage")
    if require_int(coverage.get("emitted"), "priority emitted") != status_counts["emitted"]:
        raise ValueError("priority emitted count is stale")
    return (
        require_str(translation.get("source_root"), "translation source_root"),
        require_str(translation.get("source_revision"), "translation source_revision"),
        require_str(
            translation.get("output_fingerprint"), "translation output_fingerprint"
        ),
    )


def base_exclusion(entry: PriorityEntry, card: dict[str, object]) -> dict[str, object]:
    value: dict[str, object] = {
        "priority_rank": entry.priority_rank,
        "priority_tier": entry.tier,
        "priority_tier_rank": entry.tier_rank,
        "requested_name": entry.requested_name,
    }
    catalog_name = card.get("catalog_name")
    if isinstance(catalog_name, str):
        value["catalog_name"] = catalog_name
    return value


def allocate_candidates(
    candidates: list[Candidate], strata: tuple[Stratum, ...] = STRATA
) -> tuple[dict[str, tuple[str, int]], dict[str, list[Candidate]]]:
    assignments: dict[str, tuple[str, int]] = {}
    selected_by_stratum: dict[str, list[Candidate]] = {}
    for stratum in strata:
        selected = [
            candidate
            for candidate in candidates
            if candidate.oracle_id not in assignments
            and stratum.id in candidate.matching_strata
        ][: stratum.quota]
        if len(selected) != stratum.quota:
            raise ValueError(
                f"stratum {stratum.id} has {len(selected)} available candidates; "
                f"requires {stratum.quota}"
            )
        selected_by_stratum[stratum.id] = selected
        for stratum_rank, candidate in enumerate(selected, 1):
            assignments[candidate.oracle_id] = (stratum.id, stratum_rank)
    if len(assignments) != SELECTION_LIMIT:
        raise ValueError(
            f"strata selected {len(assignments)} identities; requires {SELECTION_LIMIT}"
        )
    return assignments, selected_by_stratum


def build_manifest(root: Path = ROOT) -> dict[str, object]:
    priority_path = root / PRIORITY_PATH
    catalog_path = root / CATALOG_PATH
    priority_coverage_path = root / PRIORITY_COVERAGE_PATH
    translation_path = root / TRANSLATION_PATH
    generator_path = root / "tools/freeze_t3_6_commander_semantics.py"

    entries = parse_priority(priority_path.read_text(encoding="utf-8"))
    catalog = load_json(catalog_path)
    coverage = load_json(priority_coverage_path)
    translation = load_json(translation_path)
    cards = flatten_priority_coverage(coverage, entries)
    all_by_name, in_scope_by_name = catalog_indexes(catalog)
    source_root, source_revision, output_fingerprint = validate_translation_summary(
        translation, coverage, cards
    )

    exclusions: list[dict[str, object]] = []
    candidates: list[Candidate] = []
    seen_oracle_ids: set[str] = set()
    candidate_source_hashes: list[dict[str, str]] = []

    for entry, card in zip(entries, cards, strict=True):
        catalog_name_value = card.get("catalog_name")
        if not isinstance(catalog_name_value, str) or not catalog_name_value:
            exclusion = base_exclusion(entry, card)
            exclusion["reason_code"] = "CATALOG_IDENTITY_MISSING"
            exclusions.append(exclusion)
            continue
        catalog_name = catalog_name_value
        all_matches = all_by_name.get(catalog_name, [])
        scope_matches = in_scope_by_name.get(catalog_name, [])
        if not all_matches:
            exclusion = base_exclusion(entry, card)
            exclusion["reason_code"] = "CATALOG_IDENTITY_MISSING"
            exclusions.append(exclusion)
            continue
        if not scope_matches:
            exclusion = base_exclusion(entry, card)
            exclusion["reason_code"] = "CATALOG_IDENTITY_NOT_IN_SCOPE"
            exclusions.append(exclusion)
            continue
        if len(scope_matches) != 1:
            exclusion = base_exclusion(entry, card)
            exclusion["reason_code"] = "CATALOG_IDENTITY_AMBIGUOUS"
            exclusion["matching_identity_count"] = len(scope_matches)
            exclusions.append(exclusion)
            continue

        identity = scope_matches[0]
        oracle_id = require_str(identity.get("id"), "catalog identity id")
        classification = classification_name(identity.get("classification"))
        status = require_str(card.get("status"), "translation status")
        if status != "emitted":
            exclusion = base_exclusion(entry, card)
            exclusion.update(
                {
                    "oracle_id": oracle_id,
                    "reason_code": "TRANSLATION_NOT_EMITTED",
                    "translation_status": status,
                }
            )
            code = card.get("code")
            if isinstance(code, str) and code:
                exclusion["translation_reason_code"] = code
            exclusions.append(exclusion)
            continue
        if oracle_id in seen_oracle_ids:
            exclusion = base_exclusion(entry, card)
            exclusion.update(
                {
                    "oracle_id": oracle_id,
                    "reason_code": "DUPLICATE_ORACLE_IDENTITY",
                    "translation_status": status,
                }
            )
            exclusions.append(exclusion)
            continue
        seen_oracle_ids.add(oracle_id)

        relative_path = require_str(card.get("path"), "emitted legacy path")
        legacy_path = safe_legacy_path(root, source_root, relative_path)
        script_bytes = legacy_path.read_bytes()
        try:
            script = script_bytes.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ValueError(f"legacy script is not UTF-8: {legacy_path}") from error
        script_name = next(
            (line.removeprefix("Name:").strip() for line in script.splitlines() if line.startswith("Name:")),
            None,
        )
        if script_name != catalog_name and not catalog_name.startswith(
            f"{script_name} // "
        ):
            raise ValueError(
                f"legacy Name mismatch for {relative_path}: {script_name!r} != {catalog_name!r}"
            )
        legacy_sha256 = sha256_bytes(script_bytes)
        candidate_source_hashes.append(
            {"path": relative_path, "sha256": legacy_sha256}
        )
        matches = tuple(stratum.id for stratum in STRATA if stratum.matches(script))
        candidates.append(
            Candidate(
                priority=entry,
                catalog_name=catalog_name,
                oracle_id=oracle_id,
                classification=classification,
                legacy_path=relative_path,
                legacy_sha256=legacy_sha256,
                matching_strata=matches,
            )
        )

    assignments, selected_by_stratum = allocate_candidates(candidates)
    selected_candidates = sorted(
        (candidate for candidate in candidates if candidate.oracle_id in assignments),
        key=lambda candidate: candidate.priority.priority_rank,
    )
    selected: list[dict[str, object]] = []
    for freeze_rank, candidate in enumerate(selected_candidates, 1):
        stratum_id, stratum_rank = assignments[candidate.oracle_id]
        selected.append(
            {
                "freeze_rank": freeze_rank,
                "stratum": stratum_id,
                "stratum_rank": stratum_rank,
                "priority_rank": candidate.priority.priority_rank,
                "priority_tier": candidate.priority.tier,
                "priority_tier_rank": candidate.priority.tier_rank,
                "requested_name": candidate.priority.requested_name,
                "catalog_name": candidate.catalog_name,
                "oracle_id": candidate.oracle_id,
                "catalog_classification": candidate.classification,
                "translation_status": "emitted",
                "legacy_source_path": candidate.legacy_path,
                "legacy_source_sha256": candidate.legacy_sha256,
            }
        )

    for candidate in candidates:
        if candidate.oracle_id in assignments:
            continue
        exclusion: dict[str, object] = {
            "priority_rank": candidate.priority.priority_rank,
            "priority_tier": candidate.priority.tier,
            "priority_tier_rank": candidate.priority.tier_rank,
            "requested_name": candidate.priority.requested_name,
            "catalog_name": candidate.catalog_name,
            "oracle_id": candidate.oracle_id,
            "translation_status": "emitted",
            "legacy_source_path": candidate.legacy_path,
            "legacy_source_sha256": candidate.legacy_sha256,
        }
        if candidate.matching_strata:
            exclusion["reason_code"] = "STRATUM_QUOTA_FILLED"
            exclusion["matching_strata"] = list(candidate.matching_strata)
        else:
            exclusion["reason_code"] = "NO_SEMANTIC_STRATUM_MATCH"
        exclusions.append(exclusion)
    exclusions.sort(key=lambda value: require_int(value.get("priority_rank"), "priority rank"))

    selected_by_tier = Counter(item["priority_tier"] for item in selected)
    reason_counts = Counter(
        require_str(item.get("reason_code"), "exclusion reason") for item in exclusions
    )
    provenance = catalog.get("provenance")
    if not isinstance(provenance, dict):
        raise ValueError("catalog provenance must be an object")

    manifest: dict[str, object] = {
        "schema_version": 1,
        "manifest_kind": "t3.6_commander_semantic_candidate_freeze",
        "algorithm_version": ALGORITHM_VERSION,
        "claim_boundary": CLAIM_BOUNDARY,
        "semantic_verification": {
            "status": "not_performed",
            "selected_identity_status": "candidate_only",
            "required_later_checkpoint": "CP-CARD-SEMANTICS-100",
        },
        "selection_policy": {
            "selection_limit": SELECTION_LIMIT,
            "eligibility": [
                "listed in assets/coverage_priority.txt",
                "resolves to exactly one in-scope catalog Oracle identity",
                "status is emitted in metrics/priority_coverage.json",
                "legacy source exists and matches at least one stratum predicate",
            ],
            "allocation_order": [stratum.id for stratum in STRATA],
            "ordering": "first repository priority rank within each stratum; final freeze ranks are repository priority order",
            "failure_mode": "fail closed when an input schema, cross-count, source path, source name, or stratum quota is invalid",
            "exclusion_reason_codes": EXCLUSION_REASONS,
        },
        "sources": {
            "generator": {
                "path": "tools/freeze_t3_6_commander_semantics.py",
                "sha256": sha256_file(generator_path),
            },
            "priority": {
                "path": PRIORITY_PATH.as_posix(),
                "sha256": sha256_file(priority_path),
                "requested_count": len(entries),
            },
            "catalog": {
                "path": CATALOG_PATH.as_posix(),
                "sha256": sha256_file(catalog_path),
                "identity_count": len(catalog.get("identities", [])),
                "declared_upstream_path": provenance.get("source_path"),
                "declared_upstream_sha256": provenance.get("source_sha256"),
            },
            "priority_translation_results": {
                "path": PRIORITY_COVERAGE_PATH.as_posix(),
                "sha256": sha256_file(priority_coverage_path),
                "emitted_count": require_int(coverage.get("emitted"), "priority emitted"),
            },
            "translation_summary": {
                "path": TRANSLATION_PATH.as_posix(),
                "sha256": sha256_file(translation_path),
                "source_root": source_root,
                "source_revision": source_revision,
                "output_fingerprint": output_fingerprint,
            },
            "emitted_priority_scripts": {
                "count": len(candidate_source_hashes),
                "aggregate_sha256": sha256_bytes(
                    canonical_json_bytes(candidate_source_hashes)
                ),
            },
        },
        "summary": {
            "priority_identity_count": len(entries),
            "compiler_emitted_candidate_pool_count": len(candidates),
            "selected_count": len(selected),
            "excluded_count": len(exclusions),
            "selected_by_priority_tier": {
                str(tier): selected_by_tier[tier] for tier in sorted(selected_by_tier)
            },
            "exclusion_reason_counts": dict(sorted(reason_counts.items())),
        },
        "strata": [
            {
                "allocation_rank": allocation_rank,
                "id": stratum.id,
                "quota": stratum.quota,
                "selected_count": len(selected_by_stratum[stratum.id]),
                "description": stratum.description,
                "predicate": stratum.predicate,
            }
            for allocation_rank, stratum in enumerate(STRATA, 1)
        ],
        "selected": selected,
        "exclusions": exclusions,
    }
    if len(selected) != SELECTION_LIMIT:
        raise ValueError("selection count does not match the frozen limit")
    if len(selected) + len(exclusions) != len(entries):
        raise ValueError("not every priority entry is selected or reason-coded")
    if len({item["oracle_id"] for item in selected}) != SELECTION_LIMIT:
        raise ValueError("selected Oracle identities are not unique")
    manifest["payload_sha256"] = payload_sha256(manifest)
    return manifest


def verify_payload_hash(manifest: dict[str, object]) -> bool:
    value = manifest.get("payload_sha256")
    return isinstance(value, str) and value == payload_sha256(manifest)


def self_test(root: Path = ROOT) -> None:
    fixture = "# TIER 0 - first\nA|B\n# TIER 2 - second\nC\n"
    parsed = parse_priority(fixture)
    assert [(entry.tier, entry.tier_rank, entry.requested_name) for entry in parsed] == [
        (0, 1, "A"),
        (0, 2, "B"),
        (2, 1, "C"),
    ]
    try:
        parse_priority("# TIER 0 - x\nA|a\n")
    except ValueError as error:
        assert "duplicate priority name" in str(error)
    else:
        raise AssertionError("case-insensitive duplicate priority name was accepted")

    assert stack_interaction("A:SP$ Counter | ValidTgts$ Spell")
    assert land_ramp(
        "A:SP$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.Basic"
    )
    assert targeted_interaction(
        "A:SP$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Permanent"
    )
    assert card_flow("SVar:X:DB$ Draw | NumCards$ 2")
    assert library_or_graveyard_access(
        "SVar:X:DB$ ChangeZone | Origin$ Graveyard | Destination$ Hand | ValidTgts$ Card"
    )
    assert token_or_sacrifice("SVar:X:DB$ Token | TokenScript$ c_treasure")
    assert triggered_synergy("T:Mode$ SpellCast | Execute$ Trig")
    assert continuous_combat_or_cost("S:Mode$ Continuous | AddPower$ 1")
    assert mana_source("Types:Basic Land Forest")

    first = build_manifest(root)
    second = build_manifest(root)
    assert render_json(first) == render_json(second)
    assert verify_payload_hash(first)
    assert first["summary"]["selected_count"] == SELECTION_LIMIT  # type: ignore[index]
    assert sum(item["quota"] for item in first["strata"]) == SELECTION_LIMIT  # type: ignore[index]
    assert len({item["oracle_id"] for item in first["selected"]}) == SELECTION_LIMIT  # type: ignore[index]
    assert len(first["selected"]) + len(first["exclusions"]) == first["summary"]["priority_identity_count"]  # type: ignore[arg-type,index]
    assert all(item.get("reason_code") in EXCLUSION_REASONS for item in first["exclusions"])  # type: ignore[union-attr]
    assert "not semantic verification" in str(first["claim_boundary"]).lower()
    tampered = copy.deepcopy(first)
    tampered["selected"][0]["catalog_name"] = "tampered"  # type: ignore[index]
    assert not verify_payload_hash(tampered)
    print(
        "PASS freeze_t3_6_commander_semantics.py self-test: "
        f"selected={SELECTION_LIMIT} payload={first['payload_sha256']}"
    )


def output_path(root: Path, value: Path) -> Path:
    return value if value.is_absolute() else root / value


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Freeze or verify the local T3.6 Commander candidate manifest."
    )
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT_PATH)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    if args.self_test:
        self_test(root)
        return
    manifest = build_manifest(root)
    rendered = render_json(manifest)
    destination = output_path(root, args.output)
    if args.check:
        if not destination.is_file():
            raise SystemExit(f"missing T3.6 freeze manifest: {destination}")
        existing_bytes = destination.read_bytes()
        try:
            existing = json.loads(existing_bytes)
        except json.JSONDecodeError as error:
            raise SystemExit(f"invalid T3.6 freeze manifest: {error}") from error
        if not isinstance(existing, dict) or not verify_payload_hash(existing):
            raise SystemExit(f"invalid payload hash in T3.6 freeze manifest: {destination}")
        if existing_bytes != rendered:
            raise SystemExit(f"stale T3.6 freeze manifest: {destination}")
        print(
            f"PASS T3.6 freeze current: selected={SELECTION_LIMIT} "
            f"payload={manifest['payload_sha256']} path={destination}"
        )
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(rendered)
    print(
        f"wrote T3.6 freeze: selected={SELECTION_LIMIT} "
        f"payload={manifest['payload_sha256']} path={destination}"
    )


if __name__ == "__main__":
    main()
