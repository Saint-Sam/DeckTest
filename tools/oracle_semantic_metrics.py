#!/usr/bin/env python3
"""Measure oracle breadth without treating scalar variants as new scenarios."""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ORACLE_ROOT = ROOT / "tests" / "oracle"
OUTPUT = ROOT / "metrics" / "oracle_semantics.json"


@dataclass(frozen=True)
class Pack:
    origin: str
    source: str


PACKS = {
    "root": Pack("hand_authored", "direct oracle scenarios"),
    "generated_t1_300": Pack("generated", "tools/generate_t1_oracle_pack.py"),
    "generated_t2_gate_622": Pack("generated", "tools/generate_t2_gate_oracle_pack.py"),
    "layers": Pack("hand_authored", "T2.4 layer specification pack"),
    "legacy_layers": Pack("imported", "tools/cp_layers_legacy_script_bridge.py"),
    "reviewer_layers": Pack("generated_reviewed", "tools/generate_cp_layers_reviewer_oracles.py"),
    "t2_5_activated": Pack("hand_authored", "T2.5 task scenarios"),
    "t2_6_targeting": Pack("hand_authored", "T2.6 task scenarios"),
    "t2_7_counters_tokens_copy": Pack("hand_authored", "T2.7 task scenarios"),
    "t2_8_multiplayer_commander": Pack("hand_authored", "T2.8 task scenarios"),
    "t2_9_keyword_wave1": Pack("hand_authored", "T2.9 task scenarios"),
}

THRESHOLDS = {
    "raw_scenarios": 1200,
    "structural_families": 150,
    "hand_authored_scenarios": 100,
    "distinct_actions": 40,
    "distinct_operations": 15,
    "rule_interactions": 250,
    "origins": 4,
}

QUOTED = r'"((?:\\.|[^"\\])*)"'
ACTION_RE = re.compile(rf"\baction\s*:\s*{QUOTED}")
OPERATION_RE = re.compile(rf"\boperation\s*:\s*{QUOTED}")
SEMANTIC_FIELD_RE = re.compile(
    rf"\b(zone|kind|timing|outcome|current_step)\s*:\s*{QUOTED}"
)
ARRAY_FIELD_RE = re.compile(r"\b(keywords|invariants|types|colors)\s*:\s*\[([^\]]*)\]", re.DOTALL)
STRING_RE = re.compile(QUOTED)
NAME_RE = re.compile(rf"\bname\s*:\s*{QUOTED}")
NUMBER_RE = re.compile(r"(?<![A-Za-z_])[-+]?\d+(?:\.\d+)?")
WHITESPACE_RE = re.compile(r"\s+")


def pack_name(path: Path) -> str:
    relative = path.relative_to(ORACLE_ROOT)
    return "root" if len(relative.parts) == 1 else relative.parts[0]


def structural_form(source: str) -> str:
    without_name = NAME_RE.sub('name:"<scenario>"', source, count=1)
    without_scalars = NUMBER_RE.sub("<number>", without_name)
    return WHITESPACE_RE.sub("", without_scalars)


def source_hash(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def semantic_atoms(source: str) -> tuple[list[str], set[str]]:
    actions = [f"action:{match.group(1)}" for match in ACTION_RE.finditer(source)]
    operations = [f"operation:{match.group(1)}" for match in OPERATION_RE.finditer(source)]
    fields = {
        f"{match.group(1)}:{match.group(2)}" for match in SEMANTIC_FIELD_RE.finditer(source)
    }
    arrays: set[str] = set()
    for match in ARRAY_FIELD_RE.finditer(source):
        field = match.group(1)
        arrays.update(f"{field}:{item.group(1)}" for item in STRING_RE.finditer(match.group(2)))
    ordered_rule_atoms = actions + operations
    return ordered_rule_atoms, set(ordered_rule_atoms) | fields | arrays


def interaction_keys(ordered: list[str], atoms: set[str]) -> set[str]:
    interactions = {
        f"sequence:{left}->{right}"
        for left, right in zip(ordered, ordered[1:])
        if left != right
    }
    rule_atoms = sorted(atom for atom in atoms if atom.startswith(("action:", "operation:")))
    context_atoms = sorted(atoms - set(rule_atoms))
    interactions.update(
        f"context:{rule}|{context}" for rule in rule_atoms for context in context_atoms
    )
    interactions.update(
        f"cooccurrence:{left}|{right}"
        for left, right in itertools.combinations(rule_atoms, 2)
        if left != right
    )
    return interactions


def build_report(root: Path = ROOT) -> dict[str, object]:
    oracle_root = root / "tests" / "oracle"
    paths = sorted(oracle_root.rglob("*.ron"))
    if not paths:
        raise ValueError("no oracle scenarios found")

    unknown_packs = sorted({pack_name(path) for path in paths} - PACKS.keys())
    if unknown_packs:
        raise ValueError(f"unclassified oracle packs: {unknown_packs}")

    families: dict[str, list[str]] = defaultdict(list)
    actions: set[str] = set()
    operations: set[str] = set()
    atoms: set[str] = set()
    interactions: set[str] = set()
    pack_counts: Counter[str] = Counter()
    origin_counts: Counter[str] = Counter()
    family_origins: dict[str, set[str]] = defaultdict(set)

    for path in paths:
        source = path.read_text(encoding="utf-8")
        pack = pack_name(path)
        origin = PACKS[pack].origin
        family = hashlib.sha256(structural_form(source).encode()).hexdigest()
        relative = str(path.relative_to(root))
        families[family].append(relative)
        family_origins[family].add(origin)
        pack_counts[pack] += 1
        origin_counts[origin] += 1

        ordered, scenario_atoms = semantic_atoms(source)
        actions.update(atom.removeprefix("action:") for atom in ordered if atom.startswith("action:"))
        operations.update(
            atom.removeprefix("operation:") for atom in ordered if atom.startswith("operation:")
        )
        atoms.update(scenario_atoms)
        interactions.update(interaction_keys(ordered, scenario_atoms))

    family_rows = []
    for digest, members in sorted(families.items()):
        family_rows.append(
            {
                "id": digest[:16],
                "scenario_count": len(members),
                "origins": sorted(family_origins[digest]),
                "representative": members[0],
            }
        )

    measured = {
        "raw_scenarios": len(paths),
        "structural_families": len(families),
        "hand_authored_scenarios": origin_counts["hand_authored"],
        "distinct_actions": len(actions),
        "distinct_operations": len(operations),
        "rule_interactions": len(interactions),
        "origins": len(origin_counts),
    }
    checks = {
        name: measured[name] >= minimum for name, minimum in THRESHOLDS.items()
    }

    return {
        "schema_version": 2,
        "generator": "tools/oracle_semantic_metrics.py",
        "source_sha256": source_hash([root / "tools/oracle_semantic_metrics.py", *paths]),
        "normalization": {
            "scenario_name": "collapsed",
            "numeric_scalars": "collapsed",
            "semantic_strings": "preserved",
            "family_identity": "sha256 of normalized RON structure",
        },
        "thresholds": THRESHOLDS,
        "measured": measured,
        "checks": checks,
        "passed": all(checks.values()),
        "origins": dict(sorted(origin_counts.items())),
        "packs": [
            {
                "name": name,
                "origin": PACKS[name].origin,
                "source": PACKS[name].source,
                "scenario_count": pack_counts[name],
            }
            for name in sorted(pack_counts)
        ],
        "actions": sorted(actions),
        "operations": sorted(operations),
        "rule_atoms": sorted(atoms),
        "interaction_keys": sorted(interactions),
        "families": family_rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        assert structural_form('(name: "a", value: 1)') == structural_form(
            '(name: "b", value: 99)'
        )
        assert structural_form('(name: "a", zone: "Hand")') != structural_form(
            '(name: "a", zone: "Library")'
        )
        print("PASS oracle_semantic_metrics.py self-test")
        return 0

    output = args.output or args.root / "metrics" / "oracle_semantics.json"
    try:
        report = build_report(args.root)
        rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.check:
            if not output.is_file() or output.read_text(encoding="utf-8") != rendered:
                raise ValueError(f"stale oracle semantic metrics: {output}")
        else:
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered, encoding="utf-8")
        measured = report["measured"]
        print(
            "Oracle semantics: "
            f"passed={report['passed']} files={measured['raw_scenarios']} "
            f"families={measured['structural_families']} "
            f"interactions={measured['rule_interactions']}"
        )
        return 0 if report["passed"] else 1
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"oracle_semantic_metrics.py: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
