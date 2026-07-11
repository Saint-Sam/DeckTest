#!/usr/bin/env python3
"""Generate truthful two-axis card maturity and Owner-facing status artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SUMMARY_PATH = ROOT / "metrics/card_maturity.json"
DETAIL_PATH = ROOT / "target/card-maturity/identities.json"
STATUS_PATH = ROOT / "STATUS.md"
CARD_ID = re.compile(r'^\s*id:\s*"([^"]+)"', re.MULTILINE)
GIT_OBJECT_ID = re.compile(r"^[0-9a-f]{40}$")
MATURITY_STAGES = (
    "absent",
    "parsed",
    "mapped_partial",
    "structurally_translated",
    "compiler_valid",
    "runtime_smoke_passed",
    "semantic_verified",
    "pod_integration_verified",
    "ai_supported",
    "product_eligible",
)
OPTIONAL_STAGE_FILES = {
    "runtime_smoke_passed": ROOT / "metrics/card_runtime_smoke.json",
    "semantic_verified": ROOT / "metrics/card_semantics_100.json",
    "pod_integration_verified": ROOT / "metrics/pod_integration.json",
    "ai_supported": ROOT / "metrics/ai_card_support.json",
    "product_eligible": ROOT / "metrics/product_eligibility.json",
}


def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)


def classification_key(value: object) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict) and len(value) == 1:
        return str(next(iter(value)))
    raise ValueError(f"unsupported catalog classification: {value!r}")


def scope_for(classification: object) -> str:
    key = classification_key(classification)
    if key == "OutOfV1":
        return "out_of_v1"
    if key == "CatalogOnly":
        return "catalog_only"
    if key in {"VerifiedPlayable", "UnverifiedPlayable", "Quarantined"}:
        return "in_v1_scope"
    raise ValueError(f"unknown catalog classification: {key}")


def definition_ids(root: Path, expected_count: int, label: str) -> set[str]:
    files = sorted(root.rglob("*.frs")) if root.is_dir() else []
    if len(files) != expected_count:
        raise ValueError(
            f"{label} has {len(files)} definition files; expected {expected_count}. "
            "Run the local T3 checkpoint before generating maturity."
        )
    ids: set[str] = set()
    for path in files:
        match = CARD_ID.search(path.read_text(encoding="utf-8"))
        if match is None:
            raise ValueError(f"{path} has no card identity")
        card_id = match.group(1)
        if card_id in ids:
            raise ValueError(f"duplicate identity {card_id} in {label}")
        ids.add(card_id)
    return ids


def definition_tree_sha256(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*.frs")):
        digest.update(path.relative_to(root).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def validate_stage_evidence(
    value: dict,
    stage: str,
    expected_source: dict[str, str],
    catalog_ids: set[str],
    in_scope_ids: set[str],
) -> set[str]:
    if value.get("schema_version") != 1:
        raise ValueError(f"{stage} evidence has unsupported schema_version")
    if value.get("stage") != stage:
        raise ValueError(f"{stage} evidence names the wrong stage")
    if value.get("passed") is not True:
        raise ValueError(f"{stage} evidence did not pass")
    if not isinstance(value.get("generated_at"), str) or not value["generated_at"]:
        raise ValueError(f"{stage} evidence has no generated_at provenance")
    if not isinstance(value.get("generator"), str) or not value["generator"]:
        raise ValueError(f"{stage} evidence has no generator provenance")
    for field in ("product_commit", "product_tree"):
        object_id = value.get(field)
        if not isinstance(object_id, str) or GIT_OBJECT_ID.fullmatch(object_id) is None:
            raise ValueError(f"{stage} evidence has invalid {field}")
    source = value.get("source")
    if not isinstance(source, dict):
        raise ValueError(f"{stage} evidence has no source binding")
    for field, expected in expected_source.items():
        if source.get(field) != expected:
            raise ValueError(f"{stage} evidence has stale source field {field}")
    identities = value.get("identity_ids")
    if not isinstance(identities, list) or not all(
        isinstance(identity, str) for identity in identities
    ):
        raise ValueError(f"{stage} evidence must contain a string identity_ids array")
    if len(identities) != len(set(identities)):
        raise ValueError(f"{stage} evidence contains duplicate identity IDs")
    ids = set(identities)
    unknown = ids - catalog_ids
    if unknown:
        raise ValueError(f"{stage} evidence references unknown identities")
    outside_scope = ids - in_scope_ids
    if outside_scope:
        raise ValueError(f"{stage} evidence references identities outside v1 scope")
    return ids


def optional_stage_ids(
    path: Path,
    stage: str,
    expected_source: dict[str, str],
    catalog_ids: set[str],
    in_scope_ids: set[str],
) -> set[str]:
    if not path.is_file():
        return set()
    value = load_json(path)
    return validate_stage_evidence(
        value, stage, expected_source, catalog_ids, in_scope_ids
    )


def enforce_stage_closure(
    stage_ids: dict[str, set[str]], compiler_ids: set[str]
) -> None:
    prerequisite = compiler_ids
    for stage in OPTIONAL_STAGE_FILES:
        ids = stage_ids[stage]
        skipped = ids - prerequisite
        if skipped:
            raise ValueError(
                f"{stage} evidence skips its prerequisite for {len(skipped)} identities"
            )
        prerequisite = ids


def percent(numerator: int, denominator: int) -> float:
    return 0.0 if denominator == 0 else numerator * 100.0 / denominator


def build(timestamp: str) -> tuple[dict, dict, str]:
    state = load_json(ROOT / "PLAN_STATE.json")
    catalog = load_json(ROOT / "assets/card_catalog.json")
    catalog_metrics = load_json(ROOT / "metrics/card_catalog.json")
    parse_metrics = load_json(ROOT / "metrics/legacy_parse.json")
    translation = load_json(ROOT / "metrics/translation.json")
    api = load_json(ROOT / "metrics/api_coverage.json")
    priority = load_json(ROOT / "metrics/priority_coverage.json")
    oracle = load_json(ROOT / "metrics/oracle_semantics.json")
    platforms = load_json(ROOT / "metrics/local_platforms.json")
    corpus = load_json(ROOT / "metrics/cp_dsl_corpus.json")
    parallel = load_json(ROOT / "metrics/t3_parallel_validation.json")

    identities = catalog.get("identities")
    printings = catalog.get("printings")
    if not isinstance(identities, list) or not isinstance(printings, list):
        raise ValueError("card catalog identities/printings must be arrays")

    identity_by_id: dict[str, dict] = {}
    scope_counts: Counter[str] = Counter()
    for identity in identities:
        if not isinstance(identity, dict) or not isinstance(identity.get("id"), str):
            raise ValueError("invalid card catalog identity")
        card_id = identity["id"]
        if card_id in identity_by_id:
            raise ValueError(f"duplicate catalog identity {card_id}")
        identity_by_id[card_id] = identity
        scope_counts[scope_for(identity.get("classification"))] += 1

    printing_counts: Counter[str] = Counter()
    for printing in printings:
        if not isinstance(printing, dict) or not isinstance(
            printing.get("oracle_id"), str
        ):
            raise ValueError("invalid catalog printing")
        oracle_id = printing["oracle_id"]
        if oracle_id not in identity_by_id:
            raise ValueError(f"printing references unknown identity {oracle_id}")
        printing_counts[oracle_id] += 1

    translated_ids = definition_ids(
        ROOT / "target/translated-cards",
        int(translation["emitted_scripts"]),
        "translated corpus",
    )
    stress_ids = definition_ids(
        ROOT / "cards/cp_dsl/definitions",
        int(corpus["reviewed_card_count"]),
        "CP-DSL stress corpus",
    )
    compiler_ids = translated_ids | stress_ids
    unknown_compiler_ids = sorted(compiler_ids - set(identity_by_id))
    if unknown_compiler_ids:
        raise ValueError(
            f"compiler evidence references {len(unknown_compiler_ids)} unknown identities"
        )

    in_scope_ids = {
        card_id
        for card_id, identity in identity_by_id.items()
        if scope_for(identity.get("classification")) == "in_v1_scope"
    }
    stage_by_id = {card_id: "absent" for card_id in in_scope_ids}
    for card_id in compiler_ids & in_scope_ids:
        stage_by_id[card_id] = "compiler_valid"

    card_database = ROOT / "target/t3-parallel/translated-carddb.bin"
    if not card_database.is_file():
        raise ValueError("translated card database is missing; run the T3 checkpoint")
    expected_stage_source = {
        "card_catalog_sha256": sha256_file(ROOT / "assets/card_catalog.json"),
        "card_database_sha256": sha256_file(card_database),
        "translation_fingerprint": str(translation.get("output_fingerprint")),
    }
    optional_evidence: dict[str, dict] = {}
    optional_ids: dict[str, set[str]] = {}
    stage_index = {stage: index for index, stage in enumerate(MATURITY_STAGES)}
    for stage, path in OPTIONAL_STAGE_FILES.items():
        ids = optional_stage_ids(
            path, stage, expected_stage_source, set(identity_by_id), in_scope_ids
        )
        optional_ids[stage] = ids
        optional_evidence[stage] = {
            "path": str(path.relative_to(ROOT)),
            "present": path.is_file(),
            "identity_count": len(ids),
            "sha256": sha256_file(path) if path.is_file() else None,
        }
    enforce_stage_closure(optional_ids, compiler_ids & in_scope_ids)
    for stage, ids in optional_ids.items():
        for card_id in ids:
            if stage_index[stage] > stage_index[stage_by_id[card_id]]:
                stage_by_id[card_id] = stage

    maturity_counts = Counter(stage_by_id.values())
    exclusive_counts = {stage: maturity_counts[stage] for stage in MATURITY_STAGES}
    cumulative_counts = {
        stage: sum(
            count
            for candidate, count in exclusive_counts.items()
            if stage_index[candidate] >= stage_index[stage]
        )
        for stage in MATURITY_STAGES
    }

    detail_records = []
    for card_id, identity in sorted(identity_by_id.items()):
        scope = scope_for(identity.get("classification"))
        detail_records.append(
            {
                "id": card_id,
                "name": identity.get("name"),
                "scope": scope,
                "maturity": stage_by_id.get(card_id),
                "printing_count": printing_counts[card_id],
            }
        )
    detail = {
        "schema_version": 1,
        "generated_at": timestamp,
        "generator": "tools/write_card_maturity.py",
        "records": detail_records,
    }
    detail_data = json_bytes(detail)

    measured = oracle.get("measured", {})
    if not isinstance(measured, dict):
        raise ValueError("oracle semantics measured field must be an object")
    targets = platforms.get("targets", [])
    if not isinstance(targets, list):
        raise ValueError("local platform targets must be an array")
    target_names = []
    for target in targets:
        if not isinstance(target, dict) or not isinstance(target.get("target"), str):
            raise ValueError("local platform target must name its target triple")
        target_names.append(target["target"])

    summary = {
        "schema_version": 1,
        "generated_at": timestamp,
        "generator": "tools/write_card_maturity.py",
        "plan_version": state.get("plan_version"),
        "units_are_separate": True,
        "overall_completion_percent": None,
        "scope": {
            "unit": "oracle_identity",
            "english_printings_represented": len(printings),
            "oracle_identities_classified": len(identities),
            "counts": {
                scope: scope_counts[scope]
                for scope in ("in_v1_scope", "out_of_v1", "catalog_only")
            },
        },
        "implementation_maturity": {
            "unit": "in_v1_scope_oracle_identity",
            "population": len(in_scope_ids),
            "stages_low_to_high": list(MATURITY_STAGES),
            "exclusive_counts": exclusive_counts,
            "cumulative_counts": cumulative_counts,
            "compiler_evidence": {
                "translated_identity_count": len(translated_ids),
                "cp_dsl_stress_identity_count": len(stress_ids),
                "union_identity_count": len(compiler_ids),
                "outside_in_v1_scope": len(compiler_ids - in_scope_ids),
                "translation_fingerprint": translation.get("output_fingerprint"),
                "card_database_sha256": expected_stage_source[
                    "card_database_sha256"
                ],
                "translated_definition_tree_sha256": definition_tree_sha256(
                    ROOT / "target/translated-cards"
                ),
                "cp_dsl_definition_tree_sha256": definition_tree_sha256(
                    ROOT / "cards/cp_dsl/definitions"
                ),
            },
            "optional_stage_evidence": optional_evidence,
        },
        "legacy_script_pipeline": {
            "unit": "legacy_script",
            "total": parse_metrics.get("total_files"),
            "parsed": parse_metrics.get("parsed_files"),
            "compiler_valid_translated_definitions": translation.get("emitted_scripts"),
            "quarantined_definitions": translation.get("quarantined_scripts"),
        },
        "legacy_ability_use_pipeline": {
            "unit": "legacy_ability_use",
            "total": api.get("legacy_uses"),
            "structurally_tested_uses": api.get("mapped_uses"),
            "quarantined_uses": api.get("quarantined_uses"),
        },
        "priority_cards": {
            "unit": "owner_priority_card_name",
            "requested": priority.get("total_requested"),
            "catalog_resolved": priority.get("catalog_resolved"),
            "compiler_valid_translated_definitions": priority.get("emitted"),
        },
        "scenario_evidence": {
            "unit": "scenario_or_observed_combination_as_named",
            "scenario_files": measured.get("raw_scenarios"),
            "distinct_scenario_commands": measured.get("distinct_actions"),
            "distinct_operations": measured.get("distinct_operations"),
            "observed_semantic_atom_combinations": measured.get("rule_interactions"),
            "hand_authored_scenarios": measured.get("hand_authored_scenarios"),
        },
        "platform_evidence": {
            "unit": "cross_compile_artifact",
            "cross_compile_artifacts_passed": len(targets)
            if platforms.get("passed")
            else 0,
            "targets": target_names,
        },
        "local_verification": {
            "github_actions_used": False,
            "deterministic_translation_replay": parallel.get(
                "deterministic_parallel_replay"
            ),
            "deterministic_blocker_plan_replay": parallel.get(
                "deterministic_blocker_plan_replay"
            ),
        },
        "detail": {
            "path": str(DETAIL_PATH.relative_to(ROOT)),
            "tracked": False,
            "sha256": sha256_bytes(detail_data),
            "record_count": len(detail_records),
        },
        "source_artifacts": {
            str(path.relative_to(ROOT)): sha256_file(path)
            for path in (
                ROOT / "FORGE_REBUILD_MASTER_PLAN.md",
                ROOT / "PLAN_STATE.json",
                ROOT / "tools/write_card_maturity.py",
                ROOT / "assets/card_catalog.json",
                ROOT / "metrics/card_catalog.json",
                ROOT / "metrics/legacy_parse.json",
                ROOT / "metrics/translation.json",
                ROOT / "metrics/api_coverage.json",
                ROOT / "metrics/priority_coverage.json",
                ROOT / "metrics/oracle_semantics.json",
                ROOT / "metrics/local_platforms.json",
                ROOT / "metrics/cp_dsl_corpus.json",
                ROOT / "metrics/t3_parallel_validation.json",
            )
        },
        "caveats": [
            "All English printings are catalog-represented; printing count is not mechanics maturity.",
            "Parsing and mapping use script/ability units and are not projected onto Oracle identity maturity.",
            "Compiler-valid includes the explicitly unverified CP-DSL stress corpus and is not semantic or product eligibility.",
            "Missing optional stage evidence produces zero promotions and never an implicit pass.",
        ],
    }
    return summary, detail, render_status(summary)


def render_status(summary: dict) -> str:
    scope = summary["scope"]
    maturity = summary["implementation_maturity"]
    scripts = summary["legacy_script_pipeline"]
    uses = summary["legacy_ability_use_pipeline"]
    priority = summary["priority_cards"]
    scenarios = summary["scenario_evidence"]
    platforms = summary["platform_evidence"]
    counts = maturity["exclusive_counts"]
    in_scope = int(maturity["population"])
    compiler_valid = int(maturity["cumulative_counts"]["compiler_valid"])
    return f"""# DeckTest / Forge 2.0 Status

Generated: {summary['generated_at']} by `tools/write_card_maturity.py`

Plan: v{summary['plan_version']}

Verification: local only; GitHub Actions disabled

No single percentage represents project completion. Counts below retain their
literal units; compiler success is not semantic or product readiness.

## Product Tracks

| Track | Current state |
| --- | --- |
| Forge Standalone / Local Trainer | T3 card factory active; focused Trainer and human play remain gated |
| PodBench | Private report-only roadmap bridge; no real worker, customer exposure, training, or launch authorized |

## Catalog Scope

| Unit | Count |
| --- | ---: |
| English printings represented | {scope['english_printings_represented']:,} |
| Oracle identities classified | {scope['oracle_identities_classified']:,} |
| In v1 scope | {scope['counts']['in_v1_scope']:,} |
| Out of v1 | {scope['counts']['out_of_v1']:,} |
| Catalog only | {scope['counts']['catalog_only']:,} |

## Identity Maturity

Exclusive highest evidence stage for the {in_scope:,} in-v1 Oracle identities:

| Highest stage | Identities |
| --- | ---: |
| Absent identity-bound definition evidence | {counts['absent']:,} |
| Parsed | {counts['parsed']:,} |
| Mapped partial | {counts['mapped_partial']:,} |
| Structurally translated | {counts['structurally_translated']:,} |
| Compiler valid | {counts['compiler_valid']:,} |
| Runtime smoke passed | {counts['runtime_smoke_passed']:,} |
| Semantic verified | {counts['semantic_verified']:,} |
| Pod integration verified | {counts['pod_integration_verified']:,} |
| AI supported | {counts['ai_supported']:,} |
| Product eligible | {counts['product_eligible']:,} |

Compiler-valid evidence currently reaches {compiler_valid:,}/{in_scope:,}
in-v1 identities ({percent(compiler_valid, in_scope):.4f}%). This includes the
unverified CP-DSL language-stress corpus and therefore is not a playable claim.
Parsing and mapping remain below in their own units rather than being guessed
onto identities.

## Card Factory

| Literal unit | Result |
| --- | ---: |
| Legacy scripts parsed | {scripts['parsed']:,}/{scripts['total']:,} |
| Compiler-valid translated legacy definitions | {scripts['compiler_valid_translated_definitions']:,} |
| Fail-closed quarantined legacy definitions | {scripts['quarantined_definitions']:,} |
| Structurally tested legacy ability uses | {uses['structurally_tested_uses']:,}/{uses['total']:,} |
| Quarantined legacy ability uses | {uses['quarantined_uses']:,} |
| Owner-priority compiler-valid definitions | {priority['compiler_valid_translated_definitions']:,}/{priority['requested']:,} |

## Evidence Breadth

| Literal unit | Count |
| --- | ---: |
| Scenario files | {scenarios['scenario_files']:,} |
| Distinct scenario commands | {scenarios['distinct_scenario_commands']:,} |
| Distinct operations | {scenarios['distinct_operations']:,} |
| Observed semantic atom combinations | {scenarios['observed_semantic_atom_combinations']:,} |
| Hand-authored scenarios | {scenarios['hand_authored_scenarios']:,} |
| Cross-compile artifacts passed | {platforms['cross_compile_artifacts_passed']:,} |

## Next Gates

1. Finish the current bounded T3.3 mapper batch.
2. T3.5 capability-specific runtime smoke; unsupported setup is reason-coded.
3. T3.6 and CP-CARD-SEMANTICS-100 for the frozen 100-card Commander set.
4. T3.9 and CP-FOUR-PLAYER-POD with four complete decks and 1,000 seeded games.
5. T1.R10 and CP-HUMAN-PLAY-CLI before trace collection or Trainer claims.

Per-identity generated detail: `{summary['detail']['path']}` (untracked,
{summary['detail']['record_count']:,} records; SHA-256
`{summary['detail']['sha256']}`).
"""


def self_test() -> None:
    assert scope_for("UnverifiedPlayable") == "in_v1_scope"
    assert scope_for("OutOfV1") == "out_of_v1"
    assert scope_for({"CatalogOnly": "token"}) == "catalog_only"
    assert percent(1, 4) == 25.0
    assert percent(0, 0) == 0.0
    source = {
        "card_catalog_sha256": "a" * 64,
        "card_database_sha256": "b" * 64,
        "translation_fingerprint": "fingerprint",
    }
    valid = {
        "schema_version": 1,
        "stage": "runtime_smoke_passed",
        "passed": True,
        "generated_at": "2026-07-11T00:00:00Z",
        "generator": "self-test",
        "product_commit": "1" * 40,
        "product_tree": "2" * 40,
        "source": dict(source),
        "identity_ids": ["in-scope-a"],
    }
    catalog_ids = {"in-scope-a", "in-scope-b", "out-of-scope"}
    in_scope_ids = {"in-scope-a", "in-scope-b"}
    assert validate_stage_evidence(
        valid, "runtime_smoke_passed", source, catalog_ids, in_scope_ids
    ) == {"in-scope-a"}

    def rejects(mutator) -> None:
        candidate = json.loads(json.dumps(valid))
        mutator(candidate)
        try:
            validate_stage_evidence(
                candidate,
                "runtime_smoke_passed",
                source,
                catalog_ids,
                in_scope_ids,
            )
        except ValueError:
            return
        raise AssertionError("invalid optional maturity evidence was accepted")

    rejects(lambda value: value.update(passed=False))
    rejects(lambda value: value.update(schema_version=2))
    rejects(lambda value: value.update(stage="semantic_verified"))
    rejects(lambda value: value["source"].update(card_catalog_sha256="stale"))
    rejects(lambda value: value.update(identity_ids=["in-scope-a", "in-scope-a"]))
    rejects(lambda value: value.update(identity_ids=["unknown"]))
    rejects(lambda value: value.update(identity_ids=["out-of-scope"]))
    rejects(lambda value: value.update(identity_ids="in-scope-a"))
    try:
        enforce_stage_closure(
            {
                "runtime_smoke_passed": {"in-scope-a"},
                "semantic_verified": {"in-scope-b"},
                "pod_integration_verified": set(),
                "ai_supported": set(),
                "product_eligible": set(),
            },
            in_scope_ids,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("prerequisite-skipping maturity evidence was accepted")
    print("PASS write_card_maturity.py self-test")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0

    if args.check:
        existing_summary = load_json(SUMMARY_PATH)
        timestamp = existing_summary.get("generated_at")
        if not isinstance(timestamp, str):
            raise ValueError("existing card maturity has no generated_at")
    else:
        timestamp = datetime.now(timezone.utc).isoformat()

    summary, detail, status = build(timestamp)
    summary_data = json_bytes(summary)
    detail_data = json_bytes(detail)
    status_data = status.encode("utf-8")
    if args.check:
        expected = (
            (SUMMARY_PATH, summary_data),
            (DETAIL_PATH, detail_data),
            (STATUS_PATH, status_data),
        )
        for path, data in expected:
            if not path.is_file() or path.read_bytes() != data:
                raise SystemExit(f"stale generated card maturity artifact: {path}")
        print("PASS card maturity and status artifacts are current")
        return 0

    write_atomic(DETAIL_PATH, detail_data)
    write_atomic(SUMMARY_PATH, summary_data)
    write_atomic(STATUS_PATH, status_data)
    print(f"wrote {SUMMARY_PATH.relative_to(ROOT)}")
    print(f"wrote {DETAIL_PATH.relative_to(ROOT)}")
    print(f"wrote {STATUS_PATH.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
