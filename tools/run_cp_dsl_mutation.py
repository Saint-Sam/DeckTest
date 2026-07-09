#!/usr/bin/env python3
"""Run curated compiler mutations in isolated local workspaces."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Mutant:
    id: str
    risk: str
    file: str
    old: str
    new: str
    replace_count: int = 1


MUTANTS = [
    Mutant(
        "id_allows_empty",
        "P0",
        "crates/forge-carddef/src/lib.rs",
        "let valid = !value.is_empty()\n                    && value.chars().all(|character| {",
        "let valid = value.chars().all(|character| {",
    ),
    Mutant(
        "id_allows_spaces",
        "P0",
        "crates/forge-carddef/src/lib.rs",
        "|| matches!(character, '-' | '_' | ':' | '.')",
        "|| matches!(character, '-' | '_' | ':' | '.' | ' ')",
    ),
    Mutant(
        "modal_dfc_collapses_to_normal",
        "P1",
        "crates/forge-carddef/src/lib.rs",
        '"modal_dfc" => Some(Self::ModalDfc),',
        '"modal_dfc" => Some(Self::Normal),',
    ),
    Mutant(
        "damage_operation_has_value_category",
        "P0",
        "crates/forge-carddef/src/lib.rs",
        'DealDamage => ("deal_damage", Effect, 2, Some(3)),',
        'DealDamage => ("deal_damage", Value, 2, Some(3)),',
    ),
    Mutant(
        "draw_accepts_zero_arguments",
        "P1",
        "crates/forge-carddef/src/lib.rs",
        'Draw => ("draw", Effect, 1, Some(2)),',
        'Draw => ("draw", Effect, 0, Some(2)),',
    ),
    Mutant(
        "tap_accepts_two_arguments",
        "P1",
        "crates/forge-carddef/src/lib.rs",
        'Tap => ("tap", Effect, 1, Some(1)),',
        'Tap => ("tap", Effect, 1, Some(2)),',
    ),
    Mutant(
        "unknown_keywords_are_accepted",
        "P1",
        "crates/forge-cardc/src/validate.rs",
        "if KNOWN_KEYWORDS.contains(&keyword) {",
        "if true || KNOWN_KEYWORDS.contains(&keyword) {",
    ),
    Mutant(
        "expression_category_check_disabled",
        "P0",
        "crates/forge-cardc/src/validate.rs",
        "if operation.category() != expected {",
        "if false && operation.category() != expected {",
    ),
    Mutant(
        "operation_argument_type_check_disabled",
        "P0",
        "crates/forge-cardc/src/validate.rs",
        "if !kind.accepts(argument) {",
        "if false && !kind.accepts(argument) {",
    ),
    Mutant(
        "continuous_accepts_prose_effect",
        "P1",
        "crates/forge-carddef/src/lib.rs",
        "Self::Continuous => match index {\n                0 => Some(Selector),\n                1 => Some(Effect),\n                2 => Some(Text),\n                _ => None,\n            },",
        "Self::Continuous => match index {\n                0 => Some(Selector),\n                1 => Some(SelectorOrText),\n                2 => Some(Text),\n                _ => None,\n            },",
    ),
    Mutant(
        "number_arguments_accept_selectors",
        "P1",
        "crates/forge-carddef/src/lib.rs",
        "Self::Number => {\n                matches!(expression, Expression::Integer(_))\n                    || matches!(category, Some(OperationCategory::Value))\n            }",
        "Self::Number => {\n                matches!(expression, Expression::Integer(_))\n                    || matches!(category, Some(OperationCategory::Value | OperationCategory::Selector))\n            }",
    ),
    Mutant(
        "activated_cost_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if kind == AbilityKind::Activated && costs.is_empty() {",
        "if false && kind == AbilityKind::Activated && costs.is_empty() {",
    ),
    Mutant(
        "trigger_event_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if matches!(kind, AbilityKind::Triggered | AbilityKind::Replacement) && event.is_none() {",
        "if false\n        && matches!(kind, AbilityKind::Triggered | AbilityKind::Replacement)\n        && event.is_none()\n    {",
    ),
    Mutant(
        "mana_ability_kind_check_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if mana_ability && kind != AbilityKind::Activated {",
        "if false && mana_ability && kind != AbilityKind::Activated {",
    ),
    Mutant(
        "creature_stats_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if type_line.card_types.contains(&CardType::Creature)\n        && (power.is_none() || toughness.is_none())",
        "if false\n        && type_line.card_types.contains(&CardType::Creature)\n        && (power.is_none() || toughness.is_none())",
    ),
    Mutant(
        "planeswalker_loyalty_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if type_line.card_types.contains(&CardType::Planeswalker) && loyalty.is_none() {",
        "if false\n        && type_line.card_types.contains(&CardType::Planeswalker)\n        && loyalty.is_none()\n    {",
    ),
    Mutant(
        "battle_defense_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if type_line.card_types.contains(&CardType::Battle) && defense.is_none() {",
        "if false && type_line.card_types.contains(&CardType::Battle) && defense.is_none() {",
    ),
    Mutant(
        "empty_names_are_accepted",
        "P1",
        "crates/forge-cardc/src/parse.rs",
        "if name.is_empty() {",
        "if false && name.is_empty() {",
        2,
    ),
    Mutant(
        "multiface_requirement_disabled",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if needs_multiple && count < 2 {",
        "if false && needs_multiple && count < 2 {",
    ),
    Mutant(
        "duplicate_fields_are_accepted",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "if slot.is_some() {",
        "if false && slot.is_some() {",
    ),
    Mutant(
        "unknown_card_types_become_creatures",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        '_ => return Err(at(path, pair, format!("unknown card type `{word}`"))),',
        "_ => card_types.push(CardType::Creature),",
    ),
    Mutant(
        "empty_card_type_set_is_accepted",
        "P1",
        "crates/forge-cardc/src/parse.rs",
        "if card_types.is_empty() {",
        "if false && card_types.is_empty() {",
    ),
    Mutant(
        "definition_status_is_open",
        "P0",
        "crates/forge-cardc/src/parse.rs",
        "value => Err(at(\n            path,\n            pair,\n            format!(\"mechanics definition cannot use status `{value}`\"),\n        )),",
        "_value => Ok(CardClassification::UnverifiedPlayable),",
    ),
    Mutant(
        "runtime_magic_check_disabled",
        "P0",
        "crates/forge-cards/src/lib.rs",
        "if bytes.get(..CARD_DATABASE_MAGIC.len()) != Some(CARD_DATABASE_MAGIC.as_slice()) {",
        "if false\n        && bytes.get(..CARD_DATABASE_MAGIC.len()) != Some(CARD_DATABASE_MAGIC.as_slice())\n    {",
    ),
    Mutant(
        "runtime_trailing_data_check_disabled",
        "P0",
        "crates/forge-cards/src/lib.rs",
        "if consumed != payload.len() {",
        "if false && consumed != payload.len() {",
    ),
    Mutant(
        "runtime_sort_checks_disabled",
        "P0",
        "crates/forge-cards/src/lib.rs",
        "if pair[0].id >= pair[1].id {",
        "if false && pair[0].id >= pair[1].id {",
        3,
    ),
    Mutant(
        "runtime_header_schema_check_disabled",
        "P0",
        "crates/forge-cards/src/lib.rs",
        "if header_schema != CARD_DATABASE_SCHEMA_VERSION {",
        "if false && header_schema != CARD_DATABASE_SCHEMA_VERSION {",
    ),
    Mutant(
        "runtime_payload_schema_check_disabled",
        "P0",
        "crates/forge-cards/src/lib.rs",
        "if database.schema_version != header_schema {",
        "if false && database.schema_version != header_schema {",
    ),
]


def tree_hash(root: Path) -> str:
    digest = hashlib.sha256()
    paths = set()
    for mutant in MUTANTS:
        paths.add(root / mutant.file)
    paths.update((root / "crates/forge-cardc/tests").glob("*.rs"))
    paths.update((root / "cards/cp_dsl/malformed").glob("*"))
    for path in sorted(paths):
        if not path.is_file():
            continue
        digest.update(str(path.relative_to(root)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def local_workers(root: Path) -> int:
    override = os.environ.get("FORGE_MUTATION_WORKERS")
    if override:
        workers = int(override)
        if workers < 1:
            raise ValueError("FORGE_MUTATION_WORKERS must be positive")
        return min(workers, len(MUTANTS))
    result = subprocess.run(
        [str(root / "scripts/local_workers.sh")],
        cwd=root,
        check=True,
        text=True,
        capture_output=True,
    )
    return min(int(result.stdout.strip()), len(MUTANTS))


def validate_anchors(root: Path) -> None:
    for mutant in MUTANTS:
        text = (root / mutant.file).read_text()
        count = text.count(mutant.old)
        if count != mutant.replace_count:
            raise ValueError(
                f"{mutant.id}: expected {mutant.replace_count} mutation anchor(s), found {count}"
            )


def prepare_workspace(root: Path, base: Path, mutant: Mutant) -> Path:
    workspace = base / "workspaces" / mutant.id
    workspace.mkdir(parents=True, exist_ok=True)
    for filename in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
        shutil.copy2(root / filename, workspace / filename)
    shutil.copytree(root / "crates", workspace / "crates", dirs_exist_ok=True)
    malformed = workspace / "cards/cp_dsl/malformed"
    malformed.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(root / "cards/cp_dsl/malformed", malformed, dirs_exist_ok=True)
    path = workspace / mutant.file
    text = path.read_text()
    path.write_text(text.replace(mutant.old, mutant.new))
    return workspace


def run_mutant(root: Path, base: Path, mutant: Mutant) -> dict[str, object]:
    workspace = prepare_workspace(root, base, mutant)
    target = base / "targets" / mutant.id
    target.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment.update(
        {
            "CARGO_BUILD_JOBS": "1",
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(target),
        }
    )
    try:
        result = subprocess.run(
            [
                "cargo",
                "test",
                "--quiet",
                "-p",
                "forge-carddef",
                "-p",
                "forge-cardc",
                "-p",
                "forge-cards",
                "--all-targets",
            ],
            cwd=workspace,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=240,
        )
    except subprocess.TimeoutExpired:
        return {"id": mutant.id, "risk": mutant.risk, "status": "invalid", "reason": "timeout"}
    output = result.stdout
    if result.returncode == 0:
        status = "survived"
        reason = "all tests passed"
    elif "could not compile" in output or "aborting due to" in output:
        status = "invalid"
        reason = "mutant did not compile"
    else:
        status = "killed"
        reason = "test suite failed"
    return {"id": mutant.id, "risk": mutant.risk, "status": status, "reason": reason}


def evaluate(report: dict[str, object]) -> tuple[bool, str]:
    killed = int(report["killed"])
    survived = int(report["survived"])
    invalid = int(report["invalid"])
    score = float(report["mutation_score_percent"])
    high_survivors = [
        row["id"]
        for row in report["mutants"]
        if row["status"] == "survived" and row["risk"] in {"P0", "P1"}
    ]
    passed = killed + survived >= 20 and invalid == 0 and score >= 90.0 and not high_survivors
    reason = (
        f"killed={killed} survived={survived} invalid={invalid} "
        f"score={score:.2f}% high_survivors={high_survivors}"
    )
    return passed, reason


def run(root: Path, output: Path) -> int:
    validate_anchors(root)
    source_hash = tree_hash(root)
    base = root / "target/cp-dsl-mutation/current"
    workers = local_workers(root)
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        rows = list(executor.map(lambda mutant: run_mutant(root, base, mutant), MUTANTS))
    counts = {status: sum(row["status"] == status for row in rows) for status in ("killed", "survived", "invalid")}
    valid = counts["killed"] + counts["survived"]
    score = 0.0 if valid == 0 else round(100.0 * counts["killed"] / valid, 2)
    report: dict[str, object] = {
        "schema_version": 1,
        "source_sha256": source_hash,
        "worker_count": workers,
        "mutant_count": len(rows),
        **counts,
        "mutation_score_percent": score,
        "minimum_score_percent": 90.0,
        "mutants": rows,
    }
    passed, reason = evaluate(report)
    report["passed"] = passed
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"CP-DSL mutation: {reason}")
    return 0 if passed else 1


def check(root: Path, output: Path) -> int:
    if not output.exists():
        print(f"missing mutation report: {output}", file=sys.stderr)
        return 1
    report = json.loads(output.read_text())
    if report.get("source_sha256") != tree_hash(root):
        print("mutation report is stale for the current compiler/tests", file=sys.stderr)
        return 1
    passed, reason = evaluate(report)
    print(f"CP-DSL mutation report: {reason}")
    return 0 if passed and report.get("passed") is True else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = args.output or args.root / "metrics/cp_dsl_mutation.json"
    try:
        return check(args.root, output) if args.check else run(args.root, output)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_cp_dsl_mutation.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
