#!/usr/bin/env python3
"""Audit canonical T4 decision keys across exact AI replay artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any, Iterable


def canonical_digest(value: Any) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def decision_signature(decision: dict[str, Any]) -> str:
    actions = sorted(
        [
            (
                action.get("action_id"),
                action.get("descriptor_schema_version"),
                canonical_digest(action.get("descriptor")),
            )
            for action in decision.get("canonical_legal_actions", [])
        ],
        key=lambda action: (str(action[0]), str(action[1]), action[2]),
    )
    return canonical_digest(
        {
            "kind": decision.get("kind"),
            "player_view_hash": decision.get("player_view_hash"),
            "path_discriminator": decision.get("path_discriminator"),
            "actions": actions,
        }
    )


def context_signature(decision: dict[str, Any]) -> str:
    return canonical_digest(
        {
            "decision_signature": decision_signature(decision),
            "context_id": decision.get("context_id"),
        }
    )


def load_replay(path: Path) -> tuple[dict[str, Any], str]:
    payload = path.read_bytes()
    value = json.loads(payload)
    if not isinstance(value, dict) or not isinstance(value.get("decisions"), list):
        raise ValueError(f"{path} is not an AI decision replay")
    return value, hashlib.sha256(payload).hexdigest()


def build_report(
    replay_paths: Iterable[Path], product_commit: str, product_tree: str
) -> dict[str, Any]:
    key_to_signature: dict[str, str] = {}
    signature_to_key: dict[str, str] = {}
    context_to_signature: dict[str, str] = {}
    key_sources: dict[str, set[str]] = {}
    failures: list[dict[str, Any]] = []
    sources: list[dict[str, Any]] = []
    total_decisions = 0
    path_bound_decisions = 0

    for path in replay_paths:
        replay, source_sha256 = load_replay(path)
        source_name = str(path)
        decisions = replay["decisions"]
        sources.append(
            {
                "path": source_name,
                "sha256": source_sha256,
                "policy": replay.get("policy_kind"),
                "seed": str(replay.get("seed")),
                "decisions": len(decisions),
            }
        )
        for index, decision in enumerate(decisions):
            total_decisions += 1
            key = decision.get("decision_state_key")
            context_id = decision.get("context_id")
            view_hash = decision.get("player_view_hash")
            actions = decision.get("canonical_legal_actions")
            missing = [
                name
                for name, value in (
                    ("decision_state_key", key),
                    ("context_id", context_id),
                    ("player_view_hash", view_hash),
                    ("canonical_legal_actions", actions),
                )
                if value in (None, "", [])
            ]
            if missing:
                failures.append(
                    {
                        "code": "MISSING_CANONICAL_FIELD",
                        "source": source_name,
                        "index": index,
                        "fields": missing,
                    }
                )
                continue
            if decision.get("path_discriminator") is not None:
                path_bound_decisions += 1

            signature = decision_signature(decision)
            prior_signature = key_to_signature.setdefault(key, signature)
            if prior_signature != signature:
                failures.append(
                    {
                        "code": "STATE_KEY_COLLISION",
                        "source": source_name,
                        "index": index,
                        "decision_state_key": key,
                        "expected_signature": prior_signature,
                        "actual_signature": signature,
                    }
                )
            prior_key = signature_to_key.setdefault(signature, key)
            if prior_key != key:
                failures.append(
                    {
                        "code": "ISOMORPHIC_STATE_KEY_ALIAS",
                        "source": source_name,
                        "index": index,
                        "signature": signature,
                        "expected_key": prior_key,
                        "actual_key": key,
                    }
                )
            context = context_signature(decision)
            prior_context = context_to_signature.setdefault(context_id, context)
            if prior_context != context:
                failures.append(
                    {
                        "code": "CONTEXT_ID_COLLISION",
                        "source": source_name,
                        "index": index,
                        "context_id": context_id,
                    }
                )
            key_sources.setdefault(key, set()).add(source_name)

    shared_keys = sum(1 for sources_for_key in key_sources.values() if len(sources_for_key) > 1)
    return {
        "schema_version": 1,
        "status": "passed" if not failures else "failed",
        "artifact_classification": "diagnostic_not_promotion_eligible",
        "product_commit": product_commit,
        "product_tree": product_tree,
        "signature_contract": {
            "state": (
                "decision kind + PlayerViewHash + hierarchical path discriminator + "
                "sorted canonical legal descriptors"
            ),
            "context": "state signature + DecisionContextId",
        },
        "sources": sources,
        "totals": {
            "decisions": total_decisions,
            "unique_state_keys": len(key_to_signature),
            "unique_semantic_signatures": len(signature_to_key),
            "path_bound_decisions": path_bound_decisions,
            "keys_shared_across_paired_policy_replays": shared_keys,
            "failures": len(failures),
        },
        "near_state_dedup_audit": (
            "passed_exact_baseline_isomorphism" if not failures else "failed"
        ),
        "replay_family_leakage_audit": "not_applicable_paired_diagnostic_baselines",
        "promotion_limits": [
            (
                "paired baseline overlap is expected and is not a "
                "development/validation/sealed leakage audit"
            ),
            "campaign split manifests and sealed labels remain required before promotion",
        ],
        "failures": failures[:100],
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("replays", nargs="+", type=Path)
    parser.add_argument("--product-commit", required=True)
    parser.add_argument("--product-tree", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    report = build_report(args.replays, args.product_commit, args.product_tree)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.check:
        if not args.output.exists() or args.output.read_text(encoding="utf-8") != rendered:
            raise SystemExit(f"stale T4 decision-key audit: {args.output}")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    if report["status"] != "passed":
        raise SystemExit("T4 decision-key audit failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
