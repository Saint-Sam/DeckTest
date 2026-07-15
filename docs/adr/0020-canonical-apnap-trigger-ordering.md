# ADR 0020: Canonical APNAP Trigger Ordering

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The kernel queued triggered abilities and put them on the stack in APNAP order,
but triggers controlled by the same player retained registration order. Human
and AI controllers therefore had no way to make a required simultaneous-trigger
ordering decision even though `DecisionKind::TriggerOrder` already existed.

The order can grow factorially if represented as complete permutations. The
kernel must also reject an order that changes the pending trigger multiset or
crosses APNAP controller groups.

## Decision

- The append-only action
  `PutPendingTriggeredAbilitiesOnStackInOrder` carries trigger IDs from stack
  bottom to top.
- The kernel validates the exact pending multiset and requires controller
  groups to follow active-player/nonactive-player order before mutating state.
- Each controlled player chooses one next trigger at a time from a scoped
  canonical `TriggerOrder` context. Repeated instances of the same trigger ID
  remain equivalent and do not create duplicate options.
- Human and AI controllers use the same legal contexts, membership checks,
  telemetry, path identity, and replay records.
- Autonomous play and groups with no meaningful ordering choice retain the
  deterministic legacy APNAP action.

## Consequences

Context width is linear in the number of distinct remaining triggers instead
of factorial in complete orders. Invalid orders fail without consuming pending
triggers. Existing deterministic games with no same-controller ordering choice
retain their prior action stream.

The benchmark fixture and independent acceptable-action labels remain open, so
this implementation does not by itself make the trigger-order family complete
for promotion.
