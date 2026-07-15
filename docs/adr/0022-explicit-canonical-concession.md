# ADR 0022: Explicit Canonical Concession

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The kernel already supported an immediate typed concession, but the production
human controller could only abort the process and AI or benchmark consumers had
no live canonical concession context. Adding concession to every normal policy
action set would be dangerous: seeded random play could concede arbitrarily and
search could treat a non-rule action as an ordinary tactical move.

## Decision

- `concession_decision_context` is a public, hidden-safe singleton
  `DecisionKind::Concession` context whose only option maps
  `DecisionDescriptor::Concede` to the typed kernel action.
- A human may explicitly type `concede` or `c` from a live main-phase or
  priority prompt. The normal prompt is not recorded as a selection; instead,
  the dedicated concession context is recorded and dispatched.
- Canonical human replay detects the dedicated concession record before the
  surrounding main or priority prompt, reconstructs the same context, and
  verifies both the decision and typed action exactly.
- AI and benchmark consumers may request the same public context. Because it is
  a singleton, policy search is unnecessary and the selected ID is still
  checked against canonical membership before applying its action.
- Ordinary random, heuristic, and search legal-action sets do not include
  concession. Legacy human replays retain their original behavior.

## Consequences

Concession is now an explicit product action rather than a process-abort error,
and human, AI, benchmark, replay, and kernel boundaries agree on one typed
identity. Unknown opponent card identities do not change its context or state
key. The production adapters are complete, while sealed Track B fixtures and
promotion evidence remain pending.
