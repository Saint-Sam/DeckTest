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
import time
from dataclasses import dataclass
from datetime import datetime, timezone
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


EXPECTED_KILLERS: dict[str, tuple[str, ...]] = {
    "id_allows_empty": ("ids_are_portable_and_nonempty", "every_malformed_fixture_has_a_positioned_diagnostic"),
    "id_allows_spaces": ("ids_are_portable_and_nonempty", "every_malformed_fixture_has_a_positioned_diagnostic"),
    "modal_dfc_collapses_to_normal": ("layout_registry_is_closed",),
    "damage_operation_has_value_category": ("operation_registry_reports_context_and_arity",),
    "draw_accepts_zero_arguments": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "tap_accepts_two_arguments": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "unknown_keywords_are_accepted": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "expression_category_check_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "operation_argument_type_check_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "continuous_accepts_prose_effect": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "number_arguments_accept_selectors": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "activated_cost_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "trigger_event_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "mana_ability_kind_check_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "creature_stats_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "planeswalker_loyalty_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "battle_defense_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "empty_names_are_accepted": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "multiface_requirement_disabled": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "duplicate_fields_are_accepted": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "unknown_card_types_become_creatures": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "empty_card_type_set_is_accepted": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "definition_status_is_open": ("every_malformed_fixture_has_a_positioned_diagnostic",),
    "runtime_magic_check_disabled": ("rejects_bad_magic_and_trailing_data",),
    "runtime_trailing_data_check_disabled": ("rejects_bad_magic_and_trailing_data",),
    "runtime_sort_checks_disabled": ("rejects_unsorted_identities",),
    "runtime_header_schema_check_disabled": ("rejects_header_and_payload_schema_mismatches",),
    "runtime_payload_schema_check_disabled": ("rejects_header_and_payload_schema_mismatches",),
}

TEST_COMMAND = [
    "cargo",
    "test",
    "--locked",
    "--offline",
    "--quiet",
    "-p",
    "forge-carddef",
    "-p",
    "forge-cardc",
    "-p",
    "forge-cards",
    "--all-targets",
]


def tree_hash(root: Path) -> str:
    digest = hashlib.sha256()
    paths = {
        root / "tools/run_cp_dsl_mutation.py",
        root / "tools/generate_cp_dsl_negative.py",
        root / "scripts/local_workers.sh",
        root / "rust-toolchain.toml",
        root / "Cargo.toml",
        root / "Cargo.lock",
    }
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


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def display_path(root: Path, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(root.resolve()))
    except ValueError:
        return str(path.resolve())


def resolve_report_path(root: Path, value: object) -> Path:
    path = Path(str(value))
    return path if path.is_absolute() else root / path


def command_output(command: list[str], root: Path) -> str:
    result = subprocess.run(
        command,
        cwd=root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return result.stdout.strip()


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
    mutant_ids = {mutant.id for mutant in MUTANTS}
    if set(EXPECTED_KILLERS) != mutant_ids:
        missing = sorted(mutant_ids - set(EXPECTED_KILLERS))
        unexpected = sorted(set(EXPECTED_KILLERS) - mutant_ids)
        raise ValueError(
            f"expected-killer registry mismatch: missing={missing} unexpected={unexpected}"
        )
    for mutant in MUTANTS:
        text = (root / mutant.file).read_text()
        count = text.count(mutant.old)
        if count != mutant.replace_count:
            raise ValueError(
                f"{mutant.id}: expected {mutant.replace_count} mutation anchor(s), found {count}"
            )


def prepare_workspace(root: Path, base: Path, label: str, mutant: Mutant | None) -> Path:
    workspace = base / "workspaces" / label
    if workspace.exists():
        shutil.rmtree(workspace)
    workspace.mkdir(parents=True)
    for filename in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
        shutil.copy2(root / filename, workspace / filename)
    shutil.copytree(root / "crates", workspace / "crates", dirs_exist_ok=True)
    malformed = workspace / "cards/cp_dsl/malformed"
    malformed.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(root / "cards/cp_dsl/malformed", malformed, dirs_exist_ok=True)
    if mutant is not None:
        path = workspace / mutant.file
        text = path.read_text()
        path.write_text(text.replace(mutant.old, mutant.new))
    return workspace


def execute_test_case(
    root: Path,
    base: Path,
    evidence_dir: Path,
    label: str,
    mutant: Mutant | None,
) -> tuple[subprocess.CompletedProcess[str] | None, dict[str, object]]:
    workspace = prepare_workspace(root, base, label, mutant)
    target = base / "targets" / label
    target.mkdir(parents=True, exist_ok=True)
    evidence_dir.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment.update(
        {
            "CARGO_BUILD_JOBS": "1",
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": str(target),
        }
    )
    started_at = utc_now()
    started = time.monotonic()
    timed_out = False
    try:
        result = subprocess.run(
            TEST_COMMAND,
            cwd=workspace,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=240,
        )
        output = result.stdout
        return_code: int | None = result.returncode
    except subprocess.TimeoutExpired as error:
        result = None
        timed_out = True
        output_value = error.stdout or ""
        output = output_value.decode() if isinstance(output_value, bytes) else output_value
        return_code = None
    finished_at = utc_now()
    elapsed_seconds = round(time.monotonic() - started, 3)
    log_path = evidence_dir / f"{label}.log"
    header = (
        f"started_at={started_at}\n"
        f"finished_at={finished_at}\n"
        f"elapsed_seconds={elapsed_seconds}\n"
        f"workspace={workspace}\n"
        f"target_directory={target}\n"
        f"command={json.dumps(TEST_COMMAND)}\n"
        f"return_code={return_code}\n"
        "--- output ---\n"
    )
    log_path.write_text(header + output)
    record: dict[str, object] = {
        "command": TEST_COMMAND,
        "started_at": started_at,
        "finished_at": finished_at,
        "elapsed_seconds": elapsed_seconds,
        "return_code": return_code,
        "timed_out": timed_out,
        "workspace": display_path(root, workspace),
        "target_directory": display_path(root, target),
        "log": display_path(root, log_path),
        "log_sha256": file_hash(log_path),
    }
    if mutant is not None:
        record["mutated_file_sha256"] = file_hash(workspace / mutant.file)
    return result, record


def run_control(root: Path, base: Path, evidence_dir: Path) -> dict[str, object]:
    result, record = execute_test_case(root, base, evidence_dir, "control", None)
    passed = result is not None and result.returncode == 0
    record.update(
        {
            "status": "passed" if passed else "failed",
            "reason": "unmutated test suite passed" if passed else "unmutated test suite failed",
        }
    )
    return record


def run_mutant(
    root: Path,
    base: Path,
    evidence_dir: Path,
    mutant: Mutant,
) -> dict[str, object]:
    result, record = execute_test_case(root, base, evidence_dir, mutant.id, mutant)
    expected_killers = EXPECTED_KILLERS[mutant.id]
    log_path = resolve_report_path(root, record["log"])
    output = log_path.read_text()
    matched_killer = next((name for name in expected_killers if name in output), None)
    test_failure = "test result: FAILED" in output and "--- FAILED" in output
    compile_failure = any(
        marker in output
        for marker in (
            "could not compile",
            "aborting due to",
            "failed to get `",
            "failed to download",
            "No space left on device",
        )
    )
    if result is None:
        status = "invalid"
        reason = "timeout"
    elif result.returncode == 0:
        status = "survived"
        reason = "all tests passed"
    elif compile_failure:
        status = "invalid"
        reason = "compile or infrastructure failure"
    elif not test_failure:
        status = "invalid"
        reason = "non-test failure"
    elif matched_killer is None:
        status = "invalid"
        reason = "test failed without an expected killing assertion"
    else:
        status = "killed"
        reason = f"expected assertion failed: {matched_killer}"
    record.update(
        {
            "id": mutant.id,
            "risk": mutant.risk,
            "status": status,
            "reason": reason,
            "expected_killers": list(expected_killers),
            "matched_killer": matched_killer,
        }
    )
    return record


def evaluate(report: dict[str, object]) -> tuple[bool, str]:
    killed = int(report["killed"])
    survived = int(report["survived"])
    invalid = int(report["invalid"])
    score = float(report["mutation_score_percent"])
    rows = report.get("mutants", [])
    if not isinstance(rows, list):
        return False, "mutant records are missing"
    high_survivors = [
        row["id"]
        for row in rows
        if row["status"] == "survived" and row["risk"] in {"P0", "P1"}
    ]
    control = report.get("control", {})
    control_passed = (
        isinstance(control, dict)
        and control.get("status") == "passed"
        and control.get("return_code") == 0
    )
    auditable_kills = all(
        isinstance(row, dict)
        and (
            row.get("status") != "killed"
            or (
                isinstance(row.get("matched_killer"), str)
                and row.get("matched_killer") in row.get("expected_killers", [])
            )
        )
        for row in rows
    )
    passed = (
        control_passed
        and len(rows) == len(MUTANTS)
        and killed + survived >= 20
        and invalid == 0
        and score >= 90.0
        and not high_survivors
        and auditable_kills
    )
    reason = (
        f"control={control_passed} killed={killed} survived={survived} invalid={invalid} "
        f"score={score:.2f}% high_survivors={high_survivors}"
    )
    return passed, reason


def validate_evidence(root: Path, report: dict[str, object]) -> tuple[bool, str]:
    control = report.get("control")
    rows = report.get("mutants")
    if not isinstance(control, dict) or not isinstance(rows, list):
        return False, "control or mutant evidence is missing"
    records = [control, *rows]
    for record in records:
        if not isinstance(record, dict):
            return False, "evidence record is not an object"
        path = resolve_report_path(root, record.get("log", ""))
        if not path.is_file():
            return False, f"missing full mutation log: {path}"
        if record.get("log_sha256") != file_hash(path):
            return False, f"mutation log hash mismatch: {path}"
        if record.get("command") != TEST_COMMAND:
            return False, f"unexpected mutation command in {path}"
        if float(record.get("elapsed_seconds", 0.0)) <= 0.0:
            return False, f"missing elapsed time in {path}"
        text = path.read_text()
        for marker in ("started_at=", "finished_at=", "target_directory=", "--- output ---"):
            if marker not in text:
                return False, f"mutation log lacks {marker}: {path}"
    if control.get("return_code") != 0 or control.get("status") != "passed":
        return False, "unmutated control did not pass"
    for row in rows:
        if not isinstance(row, dict) or row.get("status") != "killed":
            continue
        path = resolve_report_path(root, row["log"])
        text = path.read_text()
        killer = row.get("matched_killer")
        if not isinstance(killer, str) or killer not in text:
            return False, f"expected killing assertion absent from {path}"
        if "test result: FAILED" not in text or "--- FAILED" not in text:
            return False, f"killed mutant lacks a test-failure transcript: {path}"
    return True, f"verified {len(records)} full command logs"


def run(root: Path, output: Path, evidence_dir: Path) -> int:
    validate_anchors(root)
    started_at = utc_now()
    source_hash = tree_hash(root)
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    base = root / "target/cp-dsl-mutation/runs" / run_id
    evidence_dir.mkdir(parents=True, exist_ok=True)
    for old_log in evidence_dir.glob("*.log"):
        old_log.unlink()
    workers = local_workers(root)
    control = run_control(root, base, evidence_dir)
    rows: list[dict[str, object]] = []
    if control["status"] == "passed":
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
            rows = list(
                executor.map(
                    lambda mutant: run_mutant(root, base, evidence_dir, mutant), MUTANTS
                )
            )
    counts = {
        status: sum(row["status"] == status for row in rows)
        for status in ("killed", "survived", "invalid")
    }
    valid = counts["killed"] + counts["survived"]
    score = 0.0 if valid == 0 else round(100.0 * counts["killed"] / valid, 2)
    report: dict[str, object] = {
        "schema_version": 2,
        "reviewed_commit": command_output(["git", "rev-parse", "HEAD"], root),
        "reviewed_tree": command_output(["git", "rev-parse", "HEAD^{tree}"], root),
        "source_sha256": source_hash,
        "started_at": started_at,
        "finished_at": utc_now(),
        "toolchains": {
            "rustc": command_output(["rustc", "--version"], root),
            "cargo": command_output(["cargo", "--version"], root),
        },
        "test_command": TEST_COMMAND,
        "run_directory": display_path(root, base),
        "evidence_directory": display_path(root, evidence_dir),
        "worker_count": workers,
        "control": control,
        "mutant_count": len(rows),
        **counts,
        "mutation_score_percent": score,
        "minimum_score_percent": 90.0,
        "mutants": rows,
    }
    passed, reason = evaluate(report)
    evidence_passed, evidence_reason = validate_evidence(root, report)
    report["evidence_validation"] = evidence_reason
    report["passed"] = passed and evidence_passed
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"CP-DSL mutation: {reason}; {evidence_reason}")
    return 0 if report["passed"] else 1


def check(root: Path, output: Path) -> int:
    if not output.exists():
        print(f"missing mutation report: {output}", file=sys.stderr)
        return 1
    report = json.loads(output.read_text())
    if report.get("source_sha256") != tree_hash(root):
        print("mutation report is stale for the current compiler/tests", file=sys.stderr)
        return 1
    passed, reason = evaluate(report)
    evidence_passed, evidence_reason = validate_evidence(root, report)
    print(f"CP-DSL mutation report: {reason}; {evidence_reason}")
    return 0 if passed and evidence_passed and report.get("passed") is True else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = args.output or args.root / "metrics/cp_dsl_mutation.json"
    configured_evidence = os.environ.get("FORGE_CP_DSL_EVIDENCE_DIR")
    evidence_dir = (
        args.evidence_dir
        or (Path(configured_evidence) / "mutation" if configured_evidence else None)
        or args.root / "target/cp-dsl-mutation/evidence/current"
    )
    try:
        return check(args.root, output) if args.check else run(args.root, output, evidence_dir)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"run_cp_dsl_mutation.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
