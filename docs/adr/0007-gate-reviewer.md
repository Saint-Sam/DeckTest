# ADR-0007: Gate Reviewer

Date: 2026-07-06

## Status

Accepted by Owner pre-flight O1.

## Context

The master plan requires an independent Gate Reviewer before Tier 0 execution.
The Orchestrator cannot hold this role, and no agent that implemented or
reviewed code in a tier may gate-review that tier.

## Decision

Use the hybrid Option C model.

A designated strong reasoning model performs normal tier gate and checkpoint
review. The Owner is the human reviewer for CP-LAYERS, plan changes, de-scope
decisions, release, licensing, IP posture, credits, and network egress.

Gate Reviewer posture and authority are exactly those in Plan §15.1 and
Appendix A.0b: look for how the tier is wrong, write `SIGNOFF.md`, answer
Gate-Reviewer Question Queue items, approve ADRs in the Gate-Reviewer column,
and recommend plan amendments.

## Consequences

Routine gates can move without waiting on human scheduling, while high-risk and
human-only decisions still come back to the Owner.

The Orchestrator must never self-sign a gate and must preserve the Gate
Reviewer evidence trail in `reports/gates/`.

## Alternatives Considered

Human-only review was rejected as too likely to create scheduling bottlenecks.

Model-only review was rejected for decisions where human accountability,
licensing judgment, release approval, or project de-scope is required.

