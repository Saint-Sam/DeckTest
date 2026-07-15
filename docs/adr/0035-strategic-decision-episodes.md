# ADR 0035: Strategic Decision Episodes

## Status

Accepted for T4 diagnostic implementation. Promotion remains blocked until all
hierarchical prompt families and Track B consumers use the contract.

## Context

Canonical hierarchical prompts avoid exponential action products, but they can
turn one strategic declaration into several replay records. A cast may select a
spell, X, additional costs, and payment. Counting those records independently
would inflate human-teacher volume and acceptable-action agreement.

## Decision

Every recorded prompt carries these additive fields:

- `decision_episode_id`
- `root_context_id`
- `parent_context_id`
- `path_depth`
- `is_forced`
- `is_strategic_root`
- `is_terminal_subchoice`
- `final_concrete_action_id`

An ordinary prompt is a one-record episode. A hierarchical chain shares one
episode ID and root context. Child records identify their immediate parent and
increase `path_depth`. Exactly one record is terminal. Forced prompts remain in
the exact replay but are excluded from strategic-decision counts.

`final_concrete_action_id` is the selected canonical action ID for a singleton
episode. For a hierarchical episode it is a stable 128-bit ID over the ordered
canonical action path. This preserves exact replay without materializing a
Cartesian root action set.

The ID is a deterministic identity key, not a security digest. Hidden card
identity is never an input; it is derived only from the seed, root context,
record ordinal, and already-redacted canonical action IDs.

## Compatibility

Fields are additive and default during deserialization. Existing human replays
without episode metadata continue through the frozen compatibility adapter.
New exact AI evidence must be regenerated because episode linkage is part of
the recorded diagnostic product.

The durable learning schemas under `schemas/learning/v1` remain frozen and are
not silently changed. A promoted teacher-corpus schema will require these
fields only after the full prompt surface and CP-HUMAN-TRACE pass.

## Rollout

The T4 runner links every currently implemented hierarchical prompt family:
main and priority action chains, X, additional and activation costs, mana
payment, trigger targeting and divided allocation, trigger ordering, spell and
ability resolution object slots, trigger unless intent and payment, attacker
and blocker declarations, and combat-damage order and amounts. New hierarchical
variants must route through the same episode helpers and add focused linkage
tests before entering exact evidence.

Track B must report both raw prompt records and unique strategic episodes. It
must compare complete episode action IDs or an explicitly scoped subdecision;
it may not count forced range narrowing as a separate expert decision.
