#!/usr/bin/env python3
"""Build a deterministic, fail-closed T4 card admission report.

The builder consumes only local JSON evidence.  It intentionally keeps
structural translation separate from runtime, semantic, AI, and pod readiness;
missing or stale evidence can never be interpreted as a pass.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
GENERATOR = "tools/build_t4_card_admission.py"
SCHEMA_VERSION = 1
GIT_OBJECT_ID = set("0123456789abcdef")
SHA256_LENGTH = 64

# Runtime evidence is intentionally bound to the product that the T4 lane was
# asked to evaluate.  HEAD is the later local evidence commit, not the runtime
# product binding and therefore must never silently replace these values.
RUNTIME_PRODUCT_COMMIT = "19ef3302c40db3e916d2a60925546d4ebc28608d"
RUNTIME_PRODUCT_TREE = "e79efa91e0146f23f7219367e117db34ce13867a"
REALISTIC_MANIFEST_PATH = "assets/ai/pods/realistic-pod-v1-candidates.json"
REALISTIC_PILOT_INTENT_PATH = "assets/ai/pilot_intents/realistic-pod-v1-candidates.json"
REALISTIC_DECK_COUNT = 4

ADMISSION_STATUSES = (
    "candidate",
    "runtime_ready",
    "family_verified",
    "card_semantic_verified",
    "ai_decision_ready",
    "benchmark_admitted",
    "blocked",
)

HANDOFF_REASON_CODES = (
    "COMPILER_UNSUPPORTED",
    "RUNTIME_UNSUPPORTED",
    "SEMANTIC_EVIDENCE_MISSING",
    "HUMAN_PROMPT_UNSUPPORTED",
    "AI_PROMPT_UNSUPPORTED",
    "BENCHMARK_FIXTURE_MISSING",
    "HIDDEN_INFORMATION_RISK",
    "REPLAY_DIVERGENCE",
    "PERFORMANCE_LIMIT",
    "RULES_AMBIGUITY",
)

# These integrity codes are deliberately additive to the handoff vocabulary.
# They prevent stale or out-of-scope evidence from being mistaken for a
# semantic pass while preserving the handoff's required blocker taxonomy.
INTEGRITY_REASON_CODES = (
    "STALE_PRODUCT_BINDING",
    "IDENTITY_OUT_OF_SCOPE",
    "INPUT_INTEGRITY_FAILURE",
)
REASON_CODES = HANDOFF_REASON_CODES + INTEGRITY_REASON_CODES

DEFAULT_EVIDENCE_PATHS: dict[str, str | list[str] | None] = {
    "structural_translation": [
        "metrics/card_maturity.json",
        "target/card-maturity/identities.json",
    ],
    "runtime": "metrics/card_runtime_smoke.json",
    "family": None,
    "semantic": "metrics/card_semantics_100.json",
    "ai": "metrics/ai_card_support.json",
    "pod": "metrics/pod_integration.json",
}

KNOWN_EVIDENCE_STATUSES = {
    "candidate",
    "blocked",
    "failed",
    "passed",
    "verified",
    "admitted",
    "runtime_ready",
    "family_verified",
    "semantic_verified",
    "ai_decision_ready",
    "benchmark_admitted",
}

STAGE_DEFINITIONS = (
    ("structural_translation", "structurally_translated", "COMPILER_UNSUPPORTED"),
    ("runtime", "runtime_ready", "RUNTIME_UNSUPPORTED"),
    ("family", "family_verified", "SEMANTIC_EVIDENCE_MISSING"),
    ("semantic", "card_semantic_verified", "SEMANTIC_EVIDENCE_MISSING"),
    ("ai", "ai_decision_ready", "AI_PROMPT_UNSUPPORTED"),
    ("pod", "benchmark_admitted", "BENCHMARK_FIXTURE_MISSING"),
)

CHECK_ALIASES: dict[str, tuple[str, ...]] = {
    "human_choice_coverage": (
        "human_choice_coverage",
        "human_choices_represented",
        "every_required_human_choice_represented",
    ),
    "ai_choice_coverage": (
        "ai_choice_coverage",
        "ai_choices_represented",
        "every_required_ai_choice_represented",
    ),
    "benchmark_adapter": (
        "benchmark_adapter",
        "benchmark_adapter_represented",
        "every_required_benchmark_adapter_represented",
    ),
    "hidden_information_redacted": (
        "hidden_information_redacted",
        "hidden_information_safe",
    ),
    "exact_action_replay": (
        "exact_action_replay",
        "exact_replay",
        "replay_passed",
    ),
    "no_unsupported_fallback": (
        "no_unsupported_fallback",
        "unsupported_fallback_absent",
    ),
    "no_card_name_branch": (
        "no_card_name_branch",
        "card_name_branch_free",
        "no_card_name_specific_runtime_logic",
    ),
    "performance_within_limit": (
        "performance_within_limit",
        "performance_ok",
    ),
    "rules_unambiguous": (
        "rules_unambiguous",
        "rules_ambiguity_absent",
    ),
}

CHECK_REASON_CODES = {
    "human_choice_coverage": "HUMAN_PROMPT_UNSUPPORTED",
    "ai_choice_coverage": "AI_PROMPT_UNSUPPORTED",
    "benchmark_adapter": "BENCHMARK_FIXTURE_MISSING",
    "hidden_information_redacted": "HIDDEN_INFORMATION_RISK",
    "exact_action_replay": "REPLAY_DIVERGENCE",
    "no_unsupported_fallback": "BENCHMARK_FIXTURE_MISSING",
    "no_card_name_branch": "RULES_AMBIGUITY",
    "performance_within_limit": "PERFORMANCE_LIMIT",
    "rules_unambiguous": "RULES_AMBIGUITY",
}


class AdmissionError(ValueError):
    """Raised when an input cannot be safely interpreted."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode(
        "utf-8"
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise AdmissionError(f"cannot load JSON input {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise AdmissionError(f"{path} must contain a JSON object")
    return value


def write_json(path: Path, value: dict[str, Any]) -> bytes:
    data = (json.dumps(value, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)
    return data


def valid_git_object_id(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and set(value) <= GIT_OBJECT_ID


def valid_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == SHA256_LENGTH and set(value) <= set("0123456789abcdef")


def require_product_id(value: Any, label: str) -> str:
    if not valid_git_object_id(value):
        raise AdmissionError(f"{label} must be a lowercase 40-character git object id")
    return str(value)


def product_from_git(root: Path) -> tuple[str, str]:
    # The local tree contains the later evidence commit 03b9e84.  Runtime
    # evidence is still evaluated against the explicitly assigned product.
    return RUNTIME_PRODUCT_COMMIT, RUNTIME_PRODUCT_TREE


def repo_relative(root: Path, raw_path: str | Path, *, allow_absolute: bool = False) -> tuple[Path, str]:
    root = root.resolve()
    candidate = Path(raw_path)
    if candidate.is_absolute() and not allow_absolute:
        raise AdmissionError("repository-relative evidence paths may not be absolute")
    if ".." in candidate.parts:
        raise AdmissionError("repository-relative evidence paths may not contain path traversal")
    resolved = (candidate if candidate.is_absolute() else root / candidate).resolve()
    try:
        relative = resolved.relative_to(root)
    except ValueError as exc:
        raise AdmissionError(f"input path escapes repository root: {raw_path}") from exc
    return resolved, relative.as_posix()


def classification_key(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict) and len(value) == 1:
        return str(next(iter(value)))
    return ""


def load_catalog(root: Path) -> tuple[dict[str, dict[str, Any]], str, str]:
    path, relative = repo_relative(root, "assets/card_catalog.json")
    catalog = load_json(path)
    if catalog.get("schema_version") != 1 or not isinstance(catalog.get("identities"), list):
        raise AdmissionError("assets/card_catalog.json has an unsupported shape")
    records: dict[str, dict[str, Any]] = {}
    for record in catalog["identities"]:
        if not isinstance(record, dict) or not isinstance(record.get("id"), str):
            raise AdmissionError("card catalog contains an identity without a string id")
        identity_id = record["id"]
        if identity_id in records:
            raise AdmissionError(f"card catalog repeats identity {identity_id}")
        records[identity_id] = record
    return records, relative, sha256_file(path)


def catalog_name_index(catalog: dict[str, dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    by_name: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in catalog.values():
        name = record.get("name")
        if isinstance(name, str) and name:
            by_name[name].append(record)
    return by_name


def validate_manifest(value: dict[str, Any]) -> None:
    if value.get("schema_version") != SCHEMA_VERSION:
        raise AdmissionError("candidate manifest has an unsupported schema_version")
    if not isinstance(value.get("manifest_id"), str) or not value["manifest_id"]:
        raise AdmissionError("candidate manifest needs a non-empty manifest_id")
    candidates = value.get("candidates")
    if not isinstance(candidates, list):
        raise AdmissionError("candidate manifest needs a candidates array")
    seen: set[str] = set()
    for candidate in candidates:
        if not isinstance(candidate, dict):
            raise AdmissionError("candidate entries must be objects")
        identity_id = candidate.get("identity_id")
        if not isinstance(identity_id, str) or not identity_id:
            raise AdmissionError("candidate identity_id must be a non-empty string")
        if identity_id in seen:
            raise AdmissionError(f"candidate manifest repeats identity {identity_id}")
        seen.add(identity_id)
        family = candidate.get("mechanic_family")
        if not isinstance(family, str) or not family:
            raise AdmissionError(f"{identity_id} has no shared mechanic_family")
        deck_ids = candidate.get("deck_ids", [])
        if not isinstance(deck_ids, list) or not all(isinstance(item, str) and item for item in deck_ids):
            raise AdmissionError(f"{identity_id} has invalid deck_ids")
        requirements = candidate.get("requirements", {})
        if not isinstance(requirements, dict):
            raise AdmissionError(f"{identity_id} has invalid requirements")
        for key, required in requirements.items():
            if not isinstance(required, bool):
                raise AdmissionError(f"{identity_id} requirement {key} must be boolean")

    evidence_paths = value.get("evidence_paths", {})
    if evidence_paths is not None and not isinstance(evidence_paths, dict):
        raise AdmissionError("candidate manifest evidence_paths must be an object")
    if isinstance(evidence_paths, dict):
        unknown = set(evidence_paths) - {name for name, _, _ in STAGE_DEFINITIONS}
        if unknown:
            raise AdmissionError(f"candidate manifest has unknown evidence stages: {sorted(unknown)}")


def _require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise AdmissionError(f"{label} must be a non-empty string")
    return value


def _validate_realistic_pilot_intents(
    root: Path,
    candidate_path: str,
    candidate: dict[str, Any],
    pilot_intents: dict[str, Any],
    product_commit: str,
    product_tree: str,
) -> None:
    if pilot_intents.get("schema_version") != SCHEMA_VERSION:
        raise AdmissionError("PilotIntent artifact has an unsupported schema_version")
    if pilot_intents.get("artifact_kind") != "pilot_intent_candidate_set":
        raise AdmissionError("PilotIntent artifact has an unsupported artifact_kind")
    intents = pilot_intents.get("intents")
    if not isinstance(intents, list):
        raise AdmissionError("PilotIntent artifact needs an intents array")
    by_deck: dict[str, dict[str, Any]] = {}
    for intent in intents:
        if not isinstance(intent, dict):
            raise AdmissionError("PilotIntent entries must be objects")
        deck_id = _require_string(intent.get("deck_id"), "PilotIntent deck_id")
        if deck_id in by_deck:
            raise AdmissionError(f"PilotIntent repeats deck {deck_id}")
        _require_string(intent.get("intent_id"), f"{deck_id} intent_id")
        _require_string(intent.get("intent_version"), f"{deck_id} intent_version")
        by_deck[deck_id] = intent

    provenance = pilot_intents.get("provenance")
    if not isinstance(provenance, dict):
        raise AdmissionError("PilotIntent artifact has no provenance object")
    assignment = provenance.get("assignment_product")
    if not isinstance(assignment, dict):
        raise AdmissionError("PilotIntent artifact has no assignment product binding")
    if assignment.get("commit") != product_commit or assignment.get("tree") != product_tree:
        raise AdmissionError("PilotIntent artifact is bound to a stale product")
    inventory_path = provenance.get("inventory_path")
    if inventory_path != candidate_path:
        raise AdmissionError("PilotIntent inventory_path does not refer to the candidate manifest")
    inventory_sha256 = provenance.get("inventory_sha256")
    if not valid_sha256(inventory_sha256):
        raise AdmissionError("PilotIntent inventory_sha256 is missing or malformed")
    candidate_file, _ = repo_relative(root, candidate_path)
    if sha256_file(candidate_file) != inventory_sha256:
        raise AdmissionError("PilotIntent inventory_sha256 does not match the candidate manifest")
    pilot_product = pilot_intents.get("product_binding")
    if not isinstance(pilot_product, dict) or pilot_product.get("commit") != product_commit or pilot_product.get("tree") != product_tree:
        raise AdmissionError("PilotIntent top-level product binding is stale")
    if pilot_intents.get("status") != "candidate_blocked_pending_admission":
        raise AdmissionError("PilotIntent artifact must remain candidate-only")
    if pilot_intents.get("freeze_status") != "not_frozen":
        raise AdmissionError("PilotIntent artifact must remain explicitly not frozen")

    candidate_decks = candidate.get("candidate_decks")
    selected_pod = candidate.get("selected_pod")
    if not isinstance(candidate_decks, list) or not isinstance(selected_pod, dict):
        raise AdmissionError("realistic candidate artifact has no deck selection packet")
    for deck in candidate_decks:
        deck_id = _require_string(deck.get("deck_id"), "candidate deck_id")
        intent_id = _require_string(deck.get("pilot_intent_id"), f"{deck_id} pilot_intent_id")
        intent_version = _require_string(deck.get("pilot_intent_version"), f"{deck_id} pilot_intent_version")
        intent = by_deck.get(deck_id)
        if intent is None or intent.get("intent_id") != intent_id or intent.get("intent_version") != intent_version:
            raise AdmissionError(f"candidate deck {deck_id} has no exact PilotIntent reference")
    if set(by_deck) != {deck.get("deck_id") for deck in candidate_decks}:
        raise AdmissionError("PilotIntent deck set does not exactly match candidate_decks")

def realistic_manifest(
    root: Path,
    candidate: dict[str, Any],
    candidate_path: str,
    pilot_intents: dict[str, Any],
    catalog: dict[str, dict[str, Any]],
    product_commit: str,
    product_tree: str,
    pilot_intent_sha256: str,
) -> dict[str, Any]:
    if candidate.get("schema_version") != SCHEMA_VERSION:
        raise AdmissionError("realistic candidate artifact has an unsupported schema_version")
    if candidate.get("artifact_kind") != "realistic_commander_candidate_inventory":
        raise AdmissionError("realistic candidate artifact has an unsupported artifact_kind")
    if candidate.get("manifest_id") != "realistic-pod-v1-candidates":
        raise AdmissionError("realistic candidate artifact has an unexpected manifest_id")
    if candidate.get("pool_kind") != "realistic":
        raise AdmissionError("realistic candidate artifact has an unexpected pool_kind")
    if candidate.get("status") != "candidate_blocked_pending_admission":
        raise AdmissionError("realistic candidate artifact must remain candidate-only")
    if candidate.get("freeze_status") != "not_frozen":
        raise AdmissionError("realistic candidate artifact must remain explicitly not frozen")
    product_binding = candidate.get("product_binding")
    if not isinstance(product_binding, dict) or product_binding.get("commit") != product_commit or product_binding.get("tree") != product_tree:
        raise AdmissionError("realistic candidate artifact is bound to a stale product")
    evidence_paths = candidate.get("evidence_paths")
    if not isinstance(evidence_paths, dict):
        raise AdmissionError("realistic candidate artifact needs an evidence_paths object")
    unknown_stages = set(evidence_paths) - {name for name, _, _ in STAGE_DEFINITIONS}
    if unknown_stages:
        raise AdmissionError(f"realistic candidate artifact has unknown evidence stages: {sorted(unknown_stages)}")

    provenance = candidate.get("provenance")
    if not isinstance(provenance, dict):
        raise AdmissionError("realistic candidate artifact has no provenance object")
    assignment = provenance.get("assignment_product")
    if not isinstance(assignment, dict) or assignment.get("commit") != product_commit or assignment.get("tree") != product_tree:
        raise AdmissionError("realistic candidate provenance is bound to a stale product")
    if provenance.get("inventory_path") != candidate_path:
        raise AdmissionError("realistic candidate provenance inventory_path mismatch")
    for label, raw_path in (
        ("candidate provenance inventory_path", provenance.get("inventory_path")),
        (
            "candidate provenance card_catalog path",
            provenance.get("card_catalog", {}).get("path") if isinstance(provenance.get("card_catalog"), dict) else None,
        ),
        (
            "candidate provenance normalization source path",
            provenance.get("normalization_source", {}).get("repository_path_if_present")
            if isinstance(provenance.get("normalization_source"), dict)
            else None,
        ),
    ):
        if raw_path is not None:
            try:
                repo_relative(root, _require_string(raw_path, label))
            except AdmissionError as exc:
                raise AdmissionError(f"{label} is unsafe: {exc}") from exc
    catalog_provenance = provenance.get("card_catalog")
    if not isinstance(catalog_provenance, dict):
        raise AdmissionError("realistic candidate provenance has no card catalog binding")
    if catalog_provenance.get("path") != "assets/card_catalog.json":
        raise AdmissionError("realistic candidate card catalog path mismatch")
    catalog_digest = catalog_provenance.get("sha256")
    if not valid_sha256(catalog_digest) or sha256_file(root / "assets/card_catalog.json") != catalog_digest:
        raise AdmissionError("realistic candidate card catalog hash mismatch")
    selected_pod = candidate.get("selected_pod")
    candidate_decks = candidate.get("candidate_decks")
    if not isinstance(selected_pod, dict) or not isinstance(candidate_decks, list):
        raise AdmissionError("realistic candidate artifact has an unsupported shape")
    selected_ids = selected_pod.get("selected_deck_ids")
    if (
        not isinstance(selected_ids, list)
        or len(selected_ids) != REALISTIC_DECK_COUNT
        or len(set(selected_ids)) != REALISTIC_DECK_COUNT
        or not all(isinstance(deck_id, str) and deck_id for deck_id in selected_ids)
    ):
        raise AdmissionError("realistic candidate packet must select exactly four unique decks")
    decks_by_id: dict[str, dict[str, Any]] = {}
    for deck in candidate_decks:
        if not isinstance(deck, dict):
            raise AdmissionError("realistic candidate deck entries must be objects")
        deck_id = _require_string(deck.get("deck_id"), "candidate deck_id")
        if deck_id in decks_by_id:
            raise AdmissionError(f"candidate artifact repeats deck {deck_id}")
        decks_by_id[deck_id] = deck
    # All candidate alternatives are part of the reviewed packet, while only
    # four are selected for production admission input.
    if not set(selected_ids) <= set(decks_by_id):
        raise AdmissionError("selected deck is absent from candidate_decks")
    if set(decks_by_id) != {
        intent.get("deck_id") for intent in pilot_intents.get("intents", []) if isinstance(intent, dict)
    }:
        raise AdmissionError("candidate deck set does not exactly match PilotIntent deck set")
    selected_decks = [decks_by_id[deck_id] for deck_id in selected_ids]
    selected_flags = {deck_id for deck_id, deck in decks_by_id.items() if deck.get("selected") is True}
    if selected_flags != set(selected_ids):
        raise AdmissionError("candidate selected flags do not exactly match selected_deck_ids")
    if not all(deck.get("selected") is True for deck in selected_decks):
        raise AdmissionError("selected deck_ids must point to selected candidate decks")
    ranks = [deck.get("selection_rank") for deck in selected_decks]
    if sorted(ranks) != list(range(1, REALISTIC_DECK_COUNT + 1)):
        raise AdmissionError("selected candidate decks must have exact selection ranks one through four")
    if any(deck.get("identity_universe_ref") != "selected_pod.candidate_identity_universe" for deck in selected_decks):
        raise AdmissionError("selected deck does not refer to the selected candidate identity universe")
    if any(deck.get("target_deck_size") != 100 or deck.get("mainboard_target") != 99 for deck in selected_decks):
        raise AdmissionError("selected candidate decks do not declare the exact Commander slot targets")

    by_name = catalog_name_index(catalog)
    membership_by_name: dict[str, set[str]] = defaultdict(set)
    slot_count_by_name: Counter[str] = Counter()
    deck_cards_by_id: dict[str, set[str]] = {}
    metadata_by_name: dict[str, dict[str, Any]] = {}
    for deck in selected_decks:
        deck_id = deck["deck_id"]
        commander = _require_string(deck.get("commander"), f"{deck_id} commander")
        commander_id = _require_string(deck.get("commander_identity_id"), f"{deck_id} commander_identity_id")
        color_identity = deck.get("color_identity")
        if (
            not isinstance(color_identity, list)
            or len(set(color_identity)) != len(color_identity)
            or not all(isinstance(color, str) and color in "WUBRG" for color in color_identity)
        ):
            raise AdmissionError(f"{deck_id} has an invalid color identity")
        commander_matches = [
            record for record in by_name.get(commander, [])
            if classification_key(record.get("classification")) in {"VerifiedPlayable", "UnverifiedPlayable", "Quarantined"}
        ]
        if len(commander_matches) != 1 or commander_matches[0].get("id") != commander_id:
            raise AdmissionError(f"{deck_id} commander does not match one playable catalog identity")
        commander_oracle_id = _require_string(
            deck.get("commander_oracle_id"), f"{deck_id} commander_oracle_id"
        )
        commander_type_line = _require_string(
            deck.get("commander_type_line"), f"{deck_id} commander_type_line"
        )
        if "Legendary" not in commander_type_line or "Creature" not in commander_type_line:
            raise AdmissionError(f"{deck_id} commander is not a legendary creature")
        commander_metadata = {
            "identity_id": commander_id,
            "oracle_id": commander_oracle_id,
            "color_identity": sorted(color_identity, key="WUBRG".index),
            "type_line": commander_type_line,
            "basic_land": False,
        }
        prior_commander_metadata = metadata_by_name.setdefault(commander, commander_metadata)
        if prior_commander_metadata != commander_metadata:
            raise AdmissionError(f"candidate metadata disagrees across decks for {commander}")
        mainboard = deck.get("mainboard")
        if not isinstance(mainboard, list) or not mainboard:
            raise AdmissionError(f"{deck_id} has no exact mainboard")
        names_in_deck: set[str] = set()
        mainboard_slots = 0
        land_slots = 0
        for index, entry in enumerate(mainboard):
            if not isinstance(entry, dict):
                raise AdmissionError(f"{deck_id} mainboard entry {index} is not an object")
            name = _require_string(entry.get("name"), f"{deck_id} mainboard name")
            identity_id = _require_string(entry.get("identity_id"), f"{deck_id} {name} identity_id")
            oracle_id = _require_string(entry.get("oracle_id"), f"{deck_id} {name} oracle_id")
            if name == commander:
                raise AdmissionError(f"{deck_id} repeats its commander in the mainboard")
            if name in names_in_deck:
                raise AdmissionError(f"{deck_id} repeats mainboard identity {name}")
            names_in_deck.add(name)
            count = entry.get("count")
            if isinstance(count, bool) or not isinstance(count, int) or count < 1:
                raise AdmissionError(f"{deck_id} {name} has an invalid count")
            basic_land = entry.get("basic_land")
            type_line = _require_string(entry.get("type_line"), f"{deck_id} {name} type_line")
            if basic_land is not ("Basic Land" in type_line):
                raise AdmissionError(f"{deck_id} {name} basic-land metadata mismatch")
            if count != 1 and basic_land is not True:
                raise AdmissionError(f"{deck_id} repeats nonbasic identity {name}")
            card_colors = entry.get("color_identity")
            if not isinstance(card_colors, list) or not all(isinstance(color, str) and color in "WUBRG" for color in card_colors):
                raise AdmissionError(f"{deck_id} {name} has invalid color identity metadata")
            if not set(card_colors) <= set(color_identity):
                raise AdmissionError(f"{deck_id} contains off-color card {name}")
            matches = [
                record for record in by_name.get(name, [])
                if classification_key(record.get("classification")) in {"VerifiedPlayable", "UnverifiedPlayable", "Quarantined"}
            ]
            if len(matches) != 1 or matches[0].get("id") != identity_id:
                raise AdmissionError(f"{deck_id} card {name} does not match one playable catalog identity")
            entry_metadata = {
                "identity_id": identity_id,
                "oracle_id": oracle_id,
                "color_identity": card_colors,
                "type_line": type_line,
                "basic_land": basic_land,
            }
            prior_entry_metadata = metadata_by_name.setdefault(name, entry_metadata)
            if prior_entry_metadata != entry_metadata:
                raise AdmissionError(f"candidate metadata disagrees across decks for {name}")
            mainboard_slots += count
            if "Land" in type_line:
                land_slots += count
            membership_by_name[name].add(deck_id)
            slot_count_by_name[name] += count
        if mainboard_slots != 99 or deck.get("mainboard_slots") != 99 or deck.get("total_slots") != 100:
            raise AdmissionError(f"{deck_id} is not an exact 100-card Commander deck")
        if deck.get("land_slots") != land_slots or deck.get("nonland_slots") != 99 - land_slots:
            raise AdmissionError(f"{deck_id} land/nonland accounting mismatch")
        membership_by_name[commander].add(deck_id)
        slot_count_by_name[commander] += 1
        deck_cards_by_id[deck_id] = names_in_deck | {commander}

    universe = selected_pod.get("candidate_identity_universe")
    if (
        not isinstance(universe, list)
        or not universe
        or len(set(universe)) != len(universe)
        or not all(isinstance(name, str) and name for name in universe)
    ):
        raise AdmissionError("realistic candidate packet has an invalid identity universe")
    if selected_pod.get("unique_identity_count") != len(universe):
        raise AdmissionError("realistic candidate packet identity count does not match its universe")
    if not (250 <= len(universe) <= 350):
        raise AdmissionError("realistic candidate identity count is outside the 250-350 Wave 1 planning band")
    if set(universe) != set(membership_by_name):
        raise AdmissionError("realistic candidate identity universe does not exactly match selected deck membership")
    if selected_pod.get("deck_count") != REALISTIC_DECK_COUNT or selected_pod.get("total_slots") != 400:
        raise AdmissionError("realistic selected pod slot accounting mismatch")

    _validate_realistic_pilot_intents(root, candidate_path, candidate, pilot_intents, product_commit, product_tree)
    intents_by_deck = {intent["deck_id"]: intent for intent in pilot_intents["intents"]}
    for deck in selected_decks:
        deck_id = deck["deck_id"]
        intent = intents_by_deck[deck_id]
        if intent.get("commander") != deck.get("commander") or intent.get("commander_identity_id") != deck.get("commander_identity_id"):
            raise AdmissionError(f"PilotIntent commander mismatch for {deck_id}")
        if intent.get("color_identity") != deck.get("color_identity"):
            raise AdmissionError(f"PilotIntent color identity mismatch for {deck_id}")
        for field in ("tutor_priorities", "signature_cards"):
            names = intent.get(field)
            if not isinstance(names, list) or not all(isinstance(name, str) and name for name in names):
                raise AdmissionError(f"PilotIntent {field} is invalid for {deck_id}")
            if not set(names) <= deck_cards_by_id[deck_id]:
                raise AdmissionError(f"PilotIntent {field} references cards outside {deck_id}")

    candidate_records = candidate.get("candidates")
    if not isinstance(candidate_records, list) or len(candidate_records) != len(universe):
        raise AdmissionError("realistic candidate records do not exactly cover the identity universe")
    records_by_name: dict[str, dict[str, Any]] = {}
    for record in candidate_records:
        if not isinstance(record, dict):
            raise AdmissionError("realistic candidate record is not an object")
        name = _require_string(record.get("name"), "realistic candidate name")
        if name in records_by_name:
            raise AdmissionError(f"realistic candidate repeats {name}")
        records_by_name[name] = record
    if set(records_by_name) != set(universe):
        raise AdmissionError("realistic candidate records and identity universe disagree")

    normalized_candidates: list[dict[str, Any]] = []
    for name in universe:
        matches = [record for record in by_name.get(name, []) if classification_key(record.get("classification")) in {
            "VerifiedPlayable",
            "UnverifiedPlayable",
            "Quarantined",
        }]
        if len(matches) != 1:
            raise AdmissionError(f"realistic candidate name {name!r} does not resolve to one playable catalog identity")
        source_record = records_by_name[name]
        if source_record.get("identity_id") != matches[0]["id"]:
            raise AdmissionError(f"realistic candidate identity_id mismatch for {name}")
        for key, expected in metadata_by_name[name].items():
            if source_record.get(key) != expected:
                raise AdmissionError(f"realistic candidate {key} mismatch for {name}")
        expected_deck_ids = sorted(membership_by_name[name])
        if source_record.get("deck_ids") != expected_deck_ids:
            raise AdmissionError(f"realistic candidate deck membership mismatch for {name}")
        if source_record.get("slot_count") != slot_count_by_name[name]:
            raise AdmissionError(f"realistic candidate slot accounting mismatch for {name}")
        family = _require_string(source_record.get("mechanic_family"), f"{name} mechanic_family")
        if source_record.get("candidate_status") != "blocked":
            raise AdmissionError(f"realistic candidate {name} must remain blocked")
        if source_record.get("primary_blocker") not in HANDOFF_REASON_CODES:
            raise AdmissionError(f"realistic candidate {name} has an unknown primary blocker")
        requirements = source_record.get("requirements")
        if not isinstance(requirements, dict) or set(requirements) != set(CHECK_ALIASES) or not all(value is True for value in requirements.values()):
            raise AdmissionError(f"realistic candidate {name} has an invalid requirement contract")
        normalized_candidates.append(
            {
                "identity_id": matches[0]["id"],
                "name": name,
                "mechanic_family": family,
                "deck_ids": expected_deck_ids,
                "requirements": requirements,
                "estimated_cards_unlocked": 1,
            }
        )

    return {
        "schema_version": SCHEMA_VERSION,
        "manifest_id": _require_string(selected_pod.get("pod_id"), "realistic pod_id"),
        "pool_kind": "realistic",
        "bootstrap": False,
        "freeze_status": candidate["freeze_status"],
        "bootstrap_note": None,
        "deck_manifest_path": candidate_path,
        "deck_manifest_sha256": sha256_file(root / candidate_path),
        "pilot_intent_path": REALISTIC_PILOT_INTENT_PATH,
        "pilot_intent_sha256": pilot_intent_sha256,
        "selected_deck_ids": list(selected_ids),
        "candidate_deck_count": REALISTIC_DECK_COUNT,
        "candidate_identity_universe_count": len(universe),
        "evidence_paths": evidence_paths,
        "candidates": normalized_candidates,
    }


def bootstrap_manifest(root: Path) -> tuple[dict[str, Any], str, list[tuple[str, str]]]:
    manifest_path, manifest_relative = repo_relative(root, "assets/t3_9/integration_decks.json")
    pod_path, pod_relative = repo_relative(root, "metrics/pod_integration.json")
    pod = load_json(pod_path)
    identity_ids = pod.get("identity_ids")
    if not isinstance(identity_ids, list) or not all(isinstance(item, str) for item in identity_ids):
        raise AdmissionError("bootstrap pod evidence has no identity_ids array")
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "manifest_id": "bootstrap-regression-pod-v1",
        "pool_kind": "bootstrap",
        "bootstrap": True,
        "freeze_status": "not_frozen",
        "bootstrap_note": (
            "Representative bootstrap only: candidates are the retained 21-identity engineering "
            "regression pod, not a realistic T4 benchmark pool."
        ),
        "candidates": [
            {
                "identity_id": identity_id,
                "mechanic_family": "bootstrap_regression_fixture",
                "deck_ids": ["regression-pod-v1"],
                "requirements": {key: True for key in CHECK_ALIASES},
            }
            for identity_id in sorted(set(identity_ids))
        ],
    }
    validate_manifest(manifest)
    return manifest, sha256_file(manifest_path), [
        (manifest_relative, sha256_file(manifest_path)),
        (pod_relative, sha256_file(pod_path)),
    ]


def _product_binding(value: dict[str, Any]) -> tuple[str | None, str | None]:
    nested = value.get("product_binding")
    if not isinstance(nested, dict):
        nested = {}
    commit = value.get("product_commit") or value.get("reviewed_commit") or nested.get("commit")
    tree = value.get("product_tree") or value.get("reviewed_tree") or nested.get("tree")
    return (str(commit) if commit is not None else None, str(tree) if tree is not None else None)


def _validate_boolean_map(value: Any, label: str) -> None:
    if not isinstance(value, dict) or any(not isinstance(item, bool) for item in value.values()):
        raise AdmissionError(f"{label} must be an object of boolean checks")


def _validate_evidence_shape(value: dict[str, Any], stage: str) -> None:
    version = value.get("schema_version")
    if isinstance(version, bool) or version != SCHEMA_VERSION:
        raise AdmissionError(f"{stage} evidence has an unsupported schema_version")
    if "passed" in value and not isinstance(value["passed"], bool):
        raise AdmissionError(f"{stage} evidence passed must be boolean")
    if "status" in value and (
        not isinstance(value["status"], str) or value["status"] not in KNOWN_EVIDENCE_STATUSES
    ):
        raise AdmissionError(f"{stage} evidence has an unsupported status")
    if "identity_ids" in value:
        ids = value["identity_ids"]
        if not isinstance(ids, list) or len(set(ids)) != len(ids) or not all(isinstance(item, str) and item for item in ids):
            raise AdmissionError(f"{stage} evidence identity_ids is malformed")
    for key in ("checks", "acceptance"):
        if key in value:
            _validate_boolean_map(value[key], f"{stage} evidence {key}")
    for key in ("records", "identity_evidence", "per_identity", "identity_records", "cards"):
        if key not in value:
            continue
        records = value[key]
        if not isinstance(records, list):
            raise AdmissionError(f"{stage} evidence {key} must be an array")
        for record in records:
            if not isinstance(record, dict):
                raise AdmissionError(f"{stage} evidence {key} contains a non-object record")
            record_id = record.get("identity_id", record.get("id"))
            if not isinstance(record_id, str) or not record_id:
                raise AdmissionError(f"{stage} evidence {key} contains a record without an identity")
            if "passed" in record and not isinstance(record["passed"], bool):
                raise AdmissionError(f"{stage} evidence record passed must be boolean")
            if "status" in record and (
                not isinstance(record["status"], str) or record["status"] not in KNOWN_EVIDENCE_STATUSES
            ):
                raise AdmissionError(f"{stage} evidence record has an unsupported status")
            if "maturity" in record and record["maturity"] is not None and not isinstance(record["maturity"], str):
                raise AdmissionError(f"{stage} evidence record maturity must be a string")
            for check_key in ("checks", "acceptance"):
                if check_key in record:
                    _validate_boolean_map(record[check_key], f"{stage} evidence record {check_key}")
    if stage == "structural_translation":
        recognized = {"records", "identity_ids", "implementation_maturity", "detail", "passed", "status"}
        if not any(key in value for key in recognized):
            raise AdmissionError("structural evidence has an unknown shape")
    elif not any(key in value for key in ("identity_ids", "records", "identity_evidence", "per_identity", "identity_records", "cards", "passed", "status")):
        raise AdmissionError(f"{stage} evidence has an unknown shape")


def _validate_artifact_hash(root: Path, raw_path: Any, raw_hash: Any, label: str) -> None:
    if not isinstance(raw_path, str) or not raw_path:
        raise AdmissionError(f"{label} path is missing")
    if not valid_sha256(raw_hash):
        raise AdmissionError(f"{label} hash is missing or malformed")
    path, _ = repo_relative(root, raw_path)
    if not path.is_file():
        raise AdmissionError(f"{label} artifact is missing")
    if sha256_file(path) != raw_hash:
        raise AdmissionError(f"{label} artifact hash does not match")


def _validate_named_path_pairs(root: Path, container: dict[str, Any], label: str) -> bool:
    authenticated = False
    path_keys = {"path", "evidence", "manifest", "replay"}
    path_keys.update(key for key in container if isinstance(key, str) and key.endswith("_path"))
    for path_key in path_keys:
        if path_key not in container:
            continue
        if path_key.endswith("_path"):
            hash_key = f"{path_key[:-5]}_sha256"
        else:
            hash_key = f"{path_key}_sha256"
        _validate_artifact_hash(root, container[path_key], container.get(hash_key), f"{label} {path_key}")
        authenticated = True
    return authenticated


def _validate_declared_hashes(root: Path, value: dict[str, Any], *, allow_known_structural_artifact: bool = False) -> None:
    authenticated = False
    direct_authenticated = False
    if "evidence" in value or "evidence_sha256" in value:
        _validate_artifact_hash(root, value.get("evidence"), value.get("evidence_sha256"), "evidence")
        authenticated = True
        direct_authenticated = True
    if "manifest" in value or "manifest_sha256" in value:
        _validate_artifact_hash(root, value.get("manifest"), value.get("manifest_sha256"), "manifest")
        authenticated = True
        direct_authenticated = True
    detail = value.get("detail")
    if isinstance(detail, dict) and ("path" in detail or "sha256" in detail):
        _validate_artifact_hash(root, detail.get("path"), detail.get("sha256"), "detail")
        authenticated = True
        direct_authenticated = True
    replays = value.get("action_replays")
    if replays is not None:
        if not isinstance(replays, list):
            raise AdmissionError("action_replays must be an array")
        for index, replay in enumerate(replays):
            if not isinstance(replay, dict):
                raise AdmissionError("action_replays contains a non-object")
            _validate_artifact_hash(root, replay.get("path"), replay.get("sha256"), f"replay {index}")
            authenticated = True
            direct_authenticated = True

    source = value.get("source")
    if source is not None and not isinstance(source, dict):
        raise AdmissionError("evidence source must be an object")
    known_sources = {
        "card_catalog_sha256": "assets/card_catalog.json",
        "card_database_sha256": "assets/carddb.bin",
        "card_database_index_sha256": "assets/carddb.index.json",
        "decision_surface_sha256": "assets/ai/decision_surface.json",
    }
    for key, path in known_sources.items():
        if isinstance(source, dict) and key in source:
            _validate_artifact_hash(root, path, source[key], f"source {key}")
            authenticated = True
    if isinstance(source, dict):
        authenticated = _validate_named_path_pairs(root, source, "source") or authenticated

    source_artifacts = value.get("source_artifacts")
    if source_artifacts is not None:
        if not isinstance(source_artifacts, dict):
            raise AdmissionError("source_artifacts must be an object")
        for path, digest in source_artifacts.items():
            if isinstance(path, str) and ("/" in path or "." in Path(path).name):
                _validate_artifact_hash(root, path, digest, "source artifact path")
                authenticated = True

    inputs = value.get("inputs")
    if inputs is not None:
        if not isinstance(inputs, dict):
            raise AdmissionError("evidence inputs must be an object")
        for key, digest in inputs.items():
            if str(key).endswith("_sha256") and not valid_sha256(digest):
                raise AdmissionError(f"input hash {key} is missing or malformed")
            if isinstance(key, str) and ("/" in key or key.endswith(".json") or key.endswith(".bin")):
                _validate_artifact_hash(root, key, digest, "input path")
                authenticated = True
        authenticated = _validate_named_path_pairs(root, inputs, "input") or authenticated
    if not allow_known_structural_artifact and (not authenticated or not direct_authenticated):
        raise AdmissionError("evidence has no authenticated repository-relative hash")


def _record_passed(record: dict[str, Any], stage: str) -> bool:
    if "passed" in record:
        return record["passed"] is True
    status = record.get("status")
    if status is not None:
        return status in {"passed", "verified", "admitted", "runtime_ready", "family_verified", "semantic_verified", "ai_decision_ready", "benchmark_admitted"}
    if stage == "structural_translation":
        return record.get("maturity") in {
            "mapped_partial",
            "structurally_translated",
            "compiler_valid",
            "runtime_smoke_passed",
            "semantic_verified",
            "pod_integration_verified",
            "ai_supported",
            "product_eligible",
        }
    return False


def _identity_evidence(value: dict[str, Any], stage: str) -> tuple[set[str], dict[str, dict[str, bool]]]:
    ids: set[str] = set()
    checks_by_identity: dict[str, dict[str, bool]] = defaultdict(dict)
    top_passed = value.get(
        "passed",
        value.get("status")
        in {
            "passed",
            "verified",
            "admitted",
            "runtime_ready",
            "family_verified",
            "semantic_verified",
            "ai_decision_ready",
            "benchmark_admitted",
        },
    )
    if top_passed and isinstance(value.get("identity_ids"), list):
        ids.update(value["identity_ids"])
        # A global check is safe only when the evidence names exactly one
        # identity.  A global boolean cannot promote unrelated cards.
        if len(ids) == 1:
            checks: dict[str, bool] = {}
            for key in ("checks", "acceptance"):
                if isinstance(value.get(key), dict):
                    checks.update(value[key])
            if checks:
                checks_by_identity[next(iter(ids))].update(checks)

    for key in ("records", "identity_evidence", "per_identity", "identity_records", "cards"):
        records = value.get(key)
        if not isinstance(records, list):
            continue
        for record in records:
            record_id = record.get("identity_id", record.get("id"))
            if not _record_passed(record, stage):
                continue
            ids.add(record_id)
            for check_key in ("checks", "acceptance"):
                if isinstance(record.get(check_key), dict):
                    checks_by_identity[record_id].update(record[check_key])
    return ids, dict(checks_by_identity)


def _invalid_stage_result(stage: str, output_status: str, reason: str, detail: str, source_paths: list[str], source_hashes: list[str]) -> dict[str, Any]:
    return {
        "stage": stage,
        "status": "invalid",
        "output_status": output_status,
        "reason_code": reason,
        "source_paths": source_paths,
        "source_hashes": source_hashes,
        "identity_ids": set(),
        "checks": {},
        "checks_by_identity": {},
        "product_bound": False,
        "product_match": False,
        "details": detail,
    }


def _stage_specs(manifest: dict[str, Any]) -> dict[str, list[str]]:
    overrides = manifest.get("evidence_paths")
    output: dict[str, list[str]] = {}
    for stage, default in DEFAULT_EVIDENCE_PATHS.items():
        raw: Any = overrides.get(stage) if isinstance(overrides, dict) and stage in overrides else default
        if raw is None:
            output[stage] = []
        elif isinstance(raw, str):
            output[stage] = [raw]
        elif isinstance(raw, list) and all(isinstance(item, str) for item in raw):
            output[stage] = list(raw)
        else:
            raise AdmissionError(f"evidence_paths.{stage} must be a string, array, or null")
    return output


def _stage_result(
    root: Path,
    stage: str,
    specs: list[str],
    product_commit: str,
    product_tree: str,
) -> dict[str, Any]:
    _, output_status, missing_reason = next(item for item in STAGE_DEFINITIONS if item[0] == stage)
    if not specs:
        return {
            "stage": stage,
            "status": "missing",
            "output_status": output_status,
            "reason_code": missing_reason,
            "source_paths": [],
            "source_hashes": [],
            "identity_ids": set(),
            "checks": {},
            "checks_by_identity": {},
            "product_bound": False,
            "product_match": False,
            "details": "No evidence source was declared; admission remains fail-closed.",
        }

    values: list[dict[str, Any]] = []
    source_paths: list[str] = []
    source_hashes: list[str] = []
    missing_paths: list[str] = []
    for raw_path in specs:
        try:
            path, relative = repo_relative(root, raw_path)
        except AdmissionError as exc:
            return _invalid_stage_result(stage, output_status, "INPUT_INTEGRITY_FAILURE", f"Evidence path rejected for {stage}: {exc}", source_paths, source_hashes)
        source_paths.append(relative)
        if not path.is_file():
            missing_paths.append(relative)
            continue
        digest = sha256_file(path)
        source_hashes.append(digest)
        try:
            value = load_json(path)
            _validate_evidence_shape(value, stage)
            _validate_declared_hashes(
                root,
                value,
                allow_known_structural_artifact=(
                    stage == "structural_translation" and relative == "target/card-maturity/identities.json"
                ),
            )
        except AdmissionError as exc:
            return _invalid_stage_result(stage, output_status, "INPUT_INTEGRITY_FAILURE", f"Evidence rejected for {relative}: {exc}", source_paths, source_hashes)
        values.append(value)
    if not values:
        return {
            "stage": stage,
            "status": "missing",
            "output_status": output_status,
            "reason_code": missing_reason,
            "source_paths": source_paths,
            "source_hashes": source_hashes,
            "identity_ids": set(),
            "checks": {},
            "checks_by_identity": {},
            "product_bound": False,
            "product_match": False,
            "details": f"Missing evidence sources: {', '.join(missing_paths)}",
        }

    product_bindings = [_product_binding(value) for value in values]
    bound = [(commit, tree) for commit, tree in product_bindings if commit is not None or tree is not None]
    product_bound = bool(bound)
    stale = any(commit != product_commit or tree != product_tree for commit, tree in bound)
    malformed_binding = any(
        (commit is None or tree is None or not valid_git_object_id(commit) or not valid_git_object_id(tree))
        for commit, tree in bound
    )
    if stale or malformed_binding:
        return {
            "stage": stage,
            "status": "stale_product",
            "output_status": output_status,
            "reason_code": "STALE_PRODUCT_BINDING",
            "source_paths": source_paths,
            "source_hashes": source_hashes,
            "identity_ids": set(),
            "checks": {},
            "checks_by_identity": {},
            "product_bound": product_bound,
            "product_match": False,
            "details": "Evidence is stale or has an invalid exact product commit/tree binding.",
        }
    if not product_bound:
        return {
            "stage": stage,
            "status": "invalid",
            "output_status": output_status,
            "reason_code": "INPUT_INTEGRITY_FAILURE",
            "source_paths": source_paths,
            "source_hashes": source_hashes,
            "identity_ids": set(),
            "checks": {},
            "checks_by_identity": {},
            "product_bound": False,
            "product_match": False,
            "details": "Evidence has no exact product commit/tree binding.",
        }

    passed_values = [value.get("passed") for value in values if "passed" in value]
    if passed_values and not all(item is True for item in passed_values):
        return {
            "stage": stage,
            "status": "failed",
            "output_status": output_status,
            "reason_code": missing_reason,
            "source_paths": source_paths,
            "source_hashes": source_hashes,
            "identity_ids": set(),
            "checks": {},
            "checks_by_identity": {},
            "product_bound": True,
            "product_match": True,
            "details": "Evidence source is explicitly not passing.",
        }

    identity_ids: set[str] = set()
    checks: dict[str, Any] = {}
    checks_by_identity: dict[str, dict[str, bool]] = defaultdict(dict)
    for value in values:
        value_ids, value_checks = _identity_evidence(value, stage)
        identity_ids.update(value_ids)
        for identity_id, identity_checks in value_checks.items():
            checks_by_identity[identity_id].update(identity_checks)
    if not identity_ids:
        return {
            "stage": stage,
            "status": "failed",
            "output_status": output_status,
            "reason_code": missing_reason,
            "source_paths": source_paths,
            "source_hashes": source_hashes,
            "identity_ids": set(),
            "checks": checks,
            "checks_by_identity": dict(checks_by_identity),
            "product_bound": True,
            "product_match": True,
            "details": "Evidence passed globally but names no admitted identity IDs.",
        }
    return {
        "stage": stage,
        "status": "passed",
        "output_status": output_status,
        "reason_code": None,
        "source_paths": source_paths,
        "source_hashes": source_hashes,
        "identity_ids": identity_ids,
        "checks": checks,
        "checks_by_identity": dict(checks_by_identity),
        "product_bound": True,
        "product_match": True,
        "details": "Exact product-bound evidence is present for the source identity set.",
    }


def _readiness_check(checks: dict[str, Any], key: str) -> bool:
    for alias in CHECK_ALIASES[key]:
        if alias in checks:
            return checks[alias] is True
    return False


def _stage_view(result: dict[str, Any], identity_id: str) -> dict[str, Any]:
    present = result["status"] == "passed" and identity_id in result["identity_ids"]
    if present:
        status = result["output_status"]
        reason = None
        verified = True
    elif result["status"] == "stale_product":
        status = "blocked"
        reason = "STALE_PRODUCT_BINDING"
        verified = False
    elif result["status"] == "invalid":
        status = "blocked"
        reason = "INPUT_INTEGRITY_FAILURE"
        verified = False
    else:
        status = result["status"]
        reason = result["reason_code"]
        verified = False
    return {
        "status": status,
        "verified": verified,
        "identity_present": present,
        "product_bound": result["product_bound"],
        "product_match": result["product_match"],
        "source_paths": result["source_paths"],
        "source_hashes": result["source_hashes"],
        "reason_code": reason,
        "details": result["details"],
    }


def _blocker(
    reason_code: str,
    stage: str,
    detail: str,
    candidate: dict[str, Any],
) -> dict[str, Any]:
    if reason_code not in REASON_CODES:
        raise AdmissionError(f"builder attempted to emit unknown blocker code {reason_code}")
    deck_ids = candidate.get("deck_ids", [])
    return {
        "reason_code": reason_code,
        "stage": stage,
        "detail": detail,
        "shared_family": candidate["mechanic_family"],
        "estimated_cards_unlocked": max(1, int(candidate.get("estimated_cards_unlocked", 1))),
        "estimated_decks_unlocked": max(1, len(deck_ids)),
    }


def _last_verified_status(passed: dict[str, bool]) -> str:
    if not passed["structural_translation"]:
        return "candidate"
    if not passed["runtime"]:
        return "candidate"
    if not passed["family"]:
        return "runtime_ready"
    if not passed["semantic"]:
        return "family_verified"
    if not passed["ai"]:
        return "card_semantic_verified"
    if not passed["pod"]:
        return "ai_decision_ready"
    return "benchmark_admitted"


def _build_card(
    candidate: dict[str, Any],
    catalog: dict[str, dict[str, Any]],
    stage_results: dict[str, dict[str, Any]],
    checks_by_identity: dict[str, dict[str, bool]],
) -> dict[str, Any]:
    identity_id = candidate["identity_id"]
    catalog_record = catalog.get(identity_id)
    blockers: list[dict[str, Any]] = []
    if catalog_record is None:
        blockers.append(
            _blocker(
                "IDENTITY_OUT_OF_SCOPE",
                "scope",
                "Identity is absent from the local card catalog; unknown identities are never admitted.",
                candidate,
            )
        )
        catalog_name = candidate.get("name", identity_id)
        in_scope = False
    else:
        catalog_name = catalog_record.get("name", identity_id)
        in_scope = classification_key(catalog_record.get("classification")) in {
            "VerifiedPlayable",
            "UnverifiedPlayable",
            "Quarantined",
        }
        if not in_scope:
            blockers.append(
                _blocker(
                    "IDENTITY_OUT_OF_SCOPE",
                    "scope",
                    "Catalog classification is outside the intended V1 playable scope.",
                    candidate,
                )
            )
        if candidate.get("name") is not None and candidate["name"] != catalog_name:
            blockers.append(
                _blocker(
                    "RULES_AMBIGUITY",
                    "scope",
                    "Candidate name does not match the identity-bound local catalog name.",
                    candidate,
                )
            )

    passed: dict[str, bool] = {}
    evidence: dict[str, Any] = {}
    for stage, _, _ in STAGE_DEFINITIONS:
        stage_result = stage_results[stage]
        passed[stage] = (
            in_scope
            and stage_result["status"] == "passed"
            and identity_id in stage_result["identity_ids"]
        )
        evidence[stage] = _stage_view(stage_result, identity_id)

    stage_detail = {
        "structural_translation": "Structural translation evidence is missing or not exact-product bound.",
        "runtime": "Production runtime execution evidence is missing or not exact-product bound.",
        "family": "Shared mechanic-family verification is missing or outside its closed assumptions.",
        "semantic": "Card-specific semantic evidence is missing or not exact-product bound.",
        "ai": "AI decision/choice evidence is missing; compiler output alone is insufficient.",
        "pod": "Benchmark-pod integration evidence is missing or not exact-product bound.",
    }
    for stage, _, missing_reason in STAGE_DEFINITIONS:
        if not passed[stage]:
            stage_result = stage_results[stage]
            reason = stage_result["reason_code"] or missing_reason
            if stage_result["status"] == "stale_product":
                reason = "STALE_PRODUCT_BINDING"
            if stage_result["status"] == "invalid":
                reason = "INPUT_INTEGRITY_FAILURE"
            blockers.append(_blocker(reason, stage, stage_result["details"] or stage_detail[stage], candidate))

    readiness_checks = {
        "identity_in_scope": in_scope,
        "structural_translation": passed["structural_translation"],
        "runtime_execution": passed["runtime"],
        "family_verification": passed["family"],
        "card_semantic_verification": passed["semantic"],
        "human_choice_coverage": _readiness_check(checks_by_identity.get(identity_id, {}), "human_choice_coverage"),
        "ai_choice_coverage": _readiness_check(checks_by_identity.get(identity_id, {}), "ai_choice_coverage"),
        "benchmark_adapter": _readiness_check(checks_by_identity.get(identity_id, {}), "benchmark_adapter"),
        "hidden_information_redacted": _readiness_check(checks_by_identity.get(identity_id, {}), "hidden_information_redacted"),
        "exact_action_replay": _readiness_check(checks_by_identity.get(identity_id, {}), "exact_action_replay"),
        "no_unsupported_fallback": _readiness_check(checks_by_identity.get(identity_id, {}), "no_unsupported_fallback"),
        "no_card_name_branch": _readiness_check(checks_by_identity.get(identity_id, {}), "no_card_name_branch"),
        "performance_within_limit": _readiness_check(checks_by_identity.get(identity_id, {}), "performance_within_limit"),
        "rules_unambiguous": _readiness_check(checks_by_identity.get(identity_id, {}), "rules_unambiguous"),
    }
    for key, reason in CHECK_REASON_CODES.items():
        if not readiness_checks[key]:
            blockers.append(
                _blocker(
                    reason,
                    "readiness_checks",
                    f"Required check {key} is absent or false; the builder does not infer it.",
                    candidate,
                )
            )

    # Preserve the first causal blocker as the primary reason while retaining
    # all visible blockers for reviewer fan-out and remediation ordering.
    unique_blockers: list[dict[str, Any]] = []
    seen_blockers: set[tuple[str, str]] = set()
    for blocker in blockers:
        key = (blocker["reason_code"], blocker["stage"])
        if key not in seen_blockers:
            unique_blockers.append(blocker)
            seen_blockers.add(key)
    last_verified = _last_verified_status(passed)
    admitted = not unique_blockers and last_verified == "benchmark_admitted"
    return {
        "identity_id": identity_id,
        "name": catalog_name,
        "mechanic_family": candidate["mechanic_family"],
        "deck_ids": sorted(candidate.get("deck_ids", [])),
        "status": "benchmark_admitted" if admitted else "blocked",
        "last_verified_status": last_verified,
        "evidence": evidence,
        "readiness_checks": readiness_checks,
        "primary_blocker": unique_blockers[0] if unique_blockers else None,
        "blockers": unique_blockers,
    }


def _aggregate_blockers(cards: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], dict[str, Any]] = {}
    for card in cards:
        for blocker in card["blockers"]:
            key = (blocker["reason_code"], blocker["shared_family"])
            item = grouped.setdefault(
                key,
                {
                    "reason_code": blocker["reason_code"],
                    "shared_family": blocker["shared_family"],
                    "affected_card_count": 0,
                    "affected_identity_ids": [],
                    "estimated_cards_unlocked": 0,
                    "estimated_decks_unlocked": 0,
                    "stages": [],
                    "details": [],
                },
            )
            item["affected_identity_ids"].append(card["identity_id"])
            item["estimated_cards_unlocked"] = max(
                item["estimated_cards_unlocked"], blocker["estimated_cards_unlocked"]
            )
            item["estimated_decks_unlocked"] = max(
                item["estimated_decks_unlocked"], blocker["estimated_decks_unlocked"]
            )
            item["stages"].append(blocker["stage"])
            item["details"].append(blocker["detail"])
    output = []
    for item in grouped.values():
        item["affected_identity_ids"] = sorted(set(item["affected_identity_ids"]))
        item["affected_card_count"] = len(item["affected_identity_ids"])
        item["stages"] = sorted(set(item["stages"]))
        item["details"] = sorted(set(item["details"]))
        output.append(item)
    return sorted(output, key=lambda item: (item["reason_code"], item["shared_family"]))


def validate_report(report: dict[str, Any], product_commit: str, product_tree: str) -> None:
    require_product_id(product_commit, "expected product_commit")
    require_product_id(product_tree, "expected product_tree")
    if report.get("schema_version") != SCHEMA_VERSION:
        raise AdmissionError("admission report has an unsupported schema_version")
    binding = report.get("product_binding")
    if not isinstance(binding, dict):
        raise AdmissionError("admission report has no product_binding")
    if binding.get("commit") != product_commit or binding.get("tree") != product_tree:
        raise AdmissionError("admission report is bound to a stale product commit/tree")
    if report.get("promotion_eligible") is True and any(
        card.get("status") != "benchmark_admitted" for card in report.get("cards", [])
    ):
        raise AdmissionError("blocked or unknown cards cannot be promotion eligible")
    for card in report.get("cards", []):
        if card.get("status") != "benchmark_admitted" and not card.get("primary_blocker"):
            raise AdmissionError("every blocked card needs a primary blocker")
        if card.get("status") == "benchmark_admitted" and card.get("blockers"):
            raise AdmissionError("admitted card has blocker records")
    for family in report.get("blocker_families", []):
        identity_ids = family.get("affected_identity_ids", [])
        if family.get("affected_card_count") != len(set(identity_ids)):
            raise AdmissionError("blocker affected_card_count is not unique-identity arithmetic")


def build_report(
    root: Path,
    manifest: dict[str, Any],
    candidate_manifest_sha256: str,
    product_commit: str,
    product_tree: str,
    *,
    artifact_kind: str = "metric",
    candidate_manifest_path: str | None = None,
    extra_input_hashes: dict[str, str | None] | None = None,
    bootstrap: bool | None = None,
) -> dict[str, Any]:
    validate_manifest(manifest)
    product_commit = require_product_id(product_commit, "product_commit")
    product_tree = require_product_id(product_tree, "product_tree")
    if artifact_kind not in {"metric", "gate"}:
        raise AdmissionError(f"unknown artifact_kind {artifact_kind}")
    catalog, catalog_path, catalog_sha256 = load_catalog(root)
    stage_paths = _stage_specs(manifest)
    stage_results = {
        stage: _stage_result(root, stage, stage_paths[stage], product_commit, product_tree)
        for stage, _, _ in STAGE_DEFINITIONS
    }
    checks_by_identity: dict[str, dict[str, bool]] = defaultdict(dict)
    for result in stage_results.values():
        for identity_id, checks in result.get("checks_by_identity", {}).items():
            checks_by_identity[identity_id].update(checks)
    cards = [
        _build_card(candidate, catalog, stage_results, checks_by_identity)
        for candidate in sorted(manifest["candidates"], key=lambda item: item["identity_id"])
    ]
    blocker_families = _aggregate_blockers(cards)
    status_counts = Counter(card["status"] for card in cards)
    last_verified_counts = Counter(card["last_verified_status"] for card in cards)
    primary_reason_counts = Counter(
        card["primary_blocker"]["reason_code"]
        for card in cards
        if card["primary_blocker"] is not None
    )
    input_hashes: dict[str, str | None] = {
        "candidate_manifest_sha256": candidate_manifest_sha256,
        catalog_path: catalog_sha256,
    }
    if extra_input_hashes:
        input_hashes.update(extra_input_hashes)
    for path_key in ("deck_manifest_path", "pilot_intent_path"):
        raw_path = manifest.get(path_key)
        if isinstance(raw_path, str):
            try:
                resolved, relative = repo_relative(root, raw_path)
            except AdmissionError:
                resolved = None
                relative = None
            if resolved is not None and relative is not None:
                input_hashes.setdefault(relative, sha256_file(resolved) if resolved.is_file() else None)
    for stage in stage_paths:
        for path in stage_paths[stage]:
            try:
                resolved, relative = repo_relative(root, path)
            except AdmissionError:
                continue
            input_hashes.setdefault(relative, sha256_file(resolved) if resolved.is_file() else None)
    # The generator is provenance, not candidate input.  It may live outside a
    # temporary fixture root used by focused tests, so hash the loaded script
    # directly instead of requiring it to be under the evidence root.
    generator_sha256 = sha256_file(Path(__file__).resolve())
    blocked = any(card["status"] != "benchmark_admitted" for card in cards)
    is_bootstrap = bool(manifest.get("bootstrap", False) if bootstrap is None else bootstrap)
    promotion_blockers: list[str] = []
    if blocked:
        promotion_blockers.append("blocked_card_identities")
    if is_bootstrap:
        promotion_blockers.append("bootstrap_input_is_not_a_realistic_benchmark_pool")
    if manifest.get("freeze_status") != "frozen":
        promotion_blockers.append("candidate_manifest_not_frozen")
    promotion_eligible = bool(cards) and not promotion_blockers
    report = {
        "schema_version": SCHEMA_VERSION,
        "artifact_kind": artifact_kind,
        "status": "blocked" if blocked else "benchmark_admitted",
        "generator": GENERATOR,
        "generator_sha256": generator_sha256,
        "product_binding": {
            "commit": product_commit,
            "tree": product_tree,
            "source": "fixed T4 runtime product binding; local evidence commit is not the runtime product",
        },
        "candidate_manifest": {
            "manifest_id": manifest["manifest_id"],
            "pool_kind": manifest.get("pool_kind", "unspecified"),
            "bootstrap": is_bootstrap,
            "freeze_status": manifest.get("freeze_status", "unspecified"),
            "path": candidate_manifest_path,
            "sha256": candidate_manifest_sha256,
            "candidate_count": len(cards),
            "selected_deck_ids": list(manifest.get("selected_deck_ids", [])),
            "deck_count": manifest.get("candidate_deck_count", len(manifest.get("selected_deck_ids", []))),
            "identity_universe_count": manifest.get("candidate_identity_universe_count", len(cards)),
        },
        "campaign_bindings": {
            "deck_manifest_sha256": input_hashes.get(
                manifest.get("deck_manifest_path", "assets/t3_9/integration_decks.json")
            ),
            "card_database_sha256": catalog_sha256,
            "pilot_intent_sha256": input_hashes.get(manifest.get("pilot_intent_path", "")),
            "policy_sha256": None,
            "weights_sha256": None,
            "seeds": [],
            "platform": {"status": "not_run", "worker_configuration": None},
        },
        "metadata": {
            "generated_at": manifest.get("generated_at") if isinstance(manifest.get("generated_at"), str) else None,
            "diagnostic": True,
            "promotion_eligible": promotion_eligible,
            "promotion_blockers": sorted(promotion_blockers),
            "bootstrap_note": manifest.get("bootstrap_note") if is_bootstrap else None,
            "uses_local_repository_data_only": True,
            "unsupported_behavior_policy": "missing, stale, failed, or unknown evidence remains blocked",
            "card_name_specific_runtime_logic": False,
            "full_candidate_inputs_present": not is_bootstrap,
        },
        "input_hashes": dict(sorted(input_hashes.items())),
        "evidence_sources": [
            {
                "stage": stage,
                "paths": result["source_paths"],
                "sha256": result["source_hashes"],
                "product_bound": result["product_bound"],
                "product_match": result["product_match"],
            }
            for stage, result in sorted(stage_results.items())
        ],
        "summary": {
            "candidate_count": len(cards),
            "admitted_count": status_counts.get("benchmark_admitted", 0),
            "blocked_count": status_counts.get("blocked", 0),
            "status_counts": {status: status_counts.get(status, 0) for status in ADMISSION_STATUSES},
            "evidence_level_counts": {
                status: last_verified_counts.get(status, 0) for status in ADMISSION_STATUSES if status != "blocked"
            },
            "primary_reason_counts": dict(sorted(primary_reason_counts.items())),
            "unresolved_blocker_reason_codes": sorted(
                {blocker["reason_code"] for blocker in blocker_families}
            ),
        },
        "blocker_families": blocker_families,
        "cards": cards,
        "promotion_eligible": promotion_eligible,
        "promotion_blockers": sorted(promotion_blockers),
    }
    validate_report(report, product_commit, product_tree)
    return report


def build_from_paths(
    root: Path,
    candidate_path: Path | None,
    product_commit: str,
    product_tree: str,
    *,
    artifact_kind: str = "metric",
    bootstrap: bool = False,
) -> tuple[dict[str, Any], dict[str, str | None]]:
    if bootstrap:
        manifest, candidate_hash, bootstrap_inputs = bootstrap_manifest(root)
        extra = dict(bootstrap_inputs)
        candidate_path_text = "assets/t3_9/integration_decks.json"
        return (
            build_report(
                root,
                manifest,
                candidate_hash,
                product_commit,
                product_tree,
                artifact_kind=artifact_kind,
                candidate_manifest_path=candidate_path_text,
                extra_input_hashes=extra,
                bootstrap=True,
            ),
            extra,
        )
    if candidate_path is None:
        candidate_path = Path(REALISTIC_MANIFEST_PATH)
    resolved, relative = repo_relative(root, candidate_path, allow_absolute=True)
    candidate_value = load_json(resolved)
    candidate_hash = sha256_file(resolved)
    extra: dict[str, str | None] = {}
    if "candidate_decks" in candidate_value:
        pilot_path, pilot_relative = repo_relative(root, REALISTIC_PILOT_INTENT_PATH)
        if not pilot_path.is_file():
            raise AdmissionError("realistic PilotIntent artifact is missing")
        pilot_hash = sha256_file(pilot_path)
        pilot_value = load_json(pilot_path)
        catalog, _, _ = load_catalog(root)
        manifest = realistic_manifest(
            root,
            candidate_value,
            relative,
            pilot_value,
            catalog,
            product_commit,
            product_tree,
            pilot_hash,
        )
        extra = {relative: candidate_hash, pilot_relative: pilot_hash}
    else:
        manifest = candidate_value
    return (
        build_report(
            root,
            manifest,
            candidate_hash,
            product_commit,
            product_tree,
            artifact_kind=artifact_kind,
            candidate_manifest_path=relative,
            extra_input_hashes=extra,
        ),
        extra,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--input", type=Path, default=None, help="local candidate manifest; defaults to the realistic candidate packet")
    parser.add_argument("--bootstrap", action="store_true", help="use the retained regression pod for diagnostics only")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "metrics/cards/t4_benchmark_admission.json",
    )
    parser.add_argument(
        "--gate-output",
        type=Path,
        default=ROOT / "reports/gates/T4-CARDS/ADMISSION.json",
    )
    parser.add_argument("--product-commit", default=None)
    parser.add_argument("--product-tree", default=None)
    parser.add_argument("--check", action="store_true", help="rebuild and compare without writing")
    args = parser.parse_args(argv)
    root = args.root.resolve()
    try:
        product_commit, product_tree = RUNTIME_PRODUCT_COMMIT, RUNTIME_PRODUCT_TREE
        if args.product_commit is not None:
            product_commit = require_product_id(args.product_commit, "--product-commit")
            if product_commit != RUNTIME_PRODUCT_COMMIT:
                raise AdmissionError("--product-commit does not match the fixed runtime product binding")
        if args.product_tree is not None:
            product_tree = require_product_id(args.product_tree, "--product-tree")
            if product_tree != RUNTIME_PRODUCT_TREE:
                raise AdmissionError("--product-tree does not match the fixed runtime product binding")
        report, _ = build_from_paths(root, args.input, product_commit, product_tree, bootstrap=args.bootstrap)
        metric_data = canonical_output(report)
        gate = copy.deepcopy(report)
        gate["artifact_kind"] = "gate"
        gate["gate"] = {
            "gate_id": "T4-CARDS/ADMISSION",
            "status": "blocked" if report["status"] == "blocked" else "pass_local",
            "metric_artifact_sha256": sha256_bytes(metric_data),
            "cp_ai_realistic_pod_passed": False,
        }
        validate_report(gate, product_commit, product_tree)
        gate_data = canonical_output(gate)
        if args.check:
            if not args.output.is_file() or not args.gate_output.is_file():
                raise AdmissionError("--check requires both expected output files")
            if args.output.read_bytes() != metric_data:
                raise AdmissionError(f"stale metric output: {args.output}")
            if args.gate_output.read_bytes() != gate_data:
                raise AdmissionError(f"stale gate output: {args.gate_output}")
            return 0
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.gate_output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(metric_data)
        args.gate_output.write_bytes(gate_data)
        return 0
    except AdmissionError as exc:
        print(f"t4 card admission: {exc}", file=sys.stderr)
        return 1


def canonical_output(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode("utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
