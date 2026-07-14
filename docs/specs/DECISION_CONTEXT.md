# Canonical DecisionContext v1

Status: implemented for the T4 diagnostic baseline. This specification
supplements, but does not amend, the ratified Forge 2.0 plan.

## Purpose

`DecisionContext` is the shared typed boundary between production legality,
human presentation, AI policies, arena evaluation, replay, and future learning
records. A context is built from the acting player's `PlayerView`; it never
receives `GameState` or an opponent's hidden identity.

The authoritative data is:

- schema version, context ID, decision kind, actor, turn, step, priority, and
  visible stack depth;
- the versioned canonical hash of the actor's redacted `PlayerView`;
- the complete canonical option IDs supplied by the production adapter;
- typed descriptors for targets, modes, values, payments, orders, and optional
  choices;
- the typed kernel actions selected by each option;
- optional presentation groups that reference, but never replace, canonical
  options.

Display labels and Rust `Debug` output are non-authoritative. They may be used
for the T1.R10 D0_raw evidence or diagnostics, but never as action IDs, model
features, labels, or replay-family keys.

## Stable Identity

`CanonicalActionId` is derived from a versioned binary encoding of the typed
descriptor. `PlayerViewHash` is derived only from the redacted projection:
hidden hand and library identities contribute an opaque slot marker, while
known identities, visible characteristics, zone order, and public scalars
contribute canonical typed bytes. `DecisionContextId` is derived from the
schema version, `PlayerViewHash`, visible context metadata, and sorted
canonical action IDs. Selecting an ID not present in the context fails closed.
Duplicate IDs and grouping references outside the legal set also fail closed.
`DecisionStateKey` contains exactly `PlayerViewHash` plus those sorted action
IDs and is the machine key for Track B near-state deduplication.

Adding a `DecisionDescriptor` variant requires updating its exhaustive
canonical encoder; otherwise compilation fails. The contract test constructs
all currently defined prompt families and verifies stable, distinct IDs.
Hidden-identity poisoning tests require equal `PlayerViewHash` values for
states that differ only in an opponent's unknown card identity, and unequal
hashes when a visible identity changes.

## Current Adapter Coverage

The T4.3-T4.5 diagnostics path currently adapts:

- typed London mulligan/keep/ordered-bottom decisions for both live human and
  AI seats;
- main-phase land plays, autonomous permanent casts, mana activations, every
  enumerated mana payment, and priority pass;
- complete player-defender attack assignment products, including split attacks,
  up to an explicit fail-closed option ceiling;
- complete blocker assignments for every attacked player, submitted in
  deterministic APNAP order and accumulated by the kernel, up to the same
  ceiling;
- complete canonical commander move-or-leave choices for both human and AI
  controllers, with selected-ID membership and telemetry regression coverage;
- one shared sorted context for the live human and AI main-phase, attacker,
  blocker, and commander-zone adapters; presentation labels are derived from
  those typed options rather than independently enumerated menus;
- canonical seeded random-legal and deterministic/noisy one-ply policies;
- root-parallel determinized search over the main, attacker, and blocker
  contexts, with every selected ID revalidated against its supplied context.

The current path remains `limited` and diagnostics-only because it does not yet
canonicalize the full Commander prompt surface: arbitrary priority
responses, targets, modes, X, optional and alternative costs, trigger order,
searches, all hidden choices, non-player combat defenders, and strategic damage
ordering.
A typed immediate concession action and event exist, but the always-available
human/AI/benchmark prompt adapter remains open. These gaps cannot be silently
skipped for CP-AI-BENCH,
CP-HUMAN-TRACE, teacher-corpus eligibility, or product-strength claims.

## Replay And Data Boundary

T4 AI replays persist canonical context/action IDs, `PlayerViewHash`, complete
typed legal-action descriptors, policy identity, score components, immutable
game inputs, search checkpoint history, and expected final state. Exact replay
regenerates decisions and typed actions from the same seeds and rejects any
divergence.

New T1.R10 human replay decisions additively persist the canonical context ID,
`DecisionStateKey`, `PlayerViewHash`, complete typed legal descriptors, and
selected action ID. The display labels, selected index, and legacy `Debug`
fingerprint remain only for presentation and backward replay compatibility.
Existing and new Owner human replays remain valid CP-HUMAN-PLAY-CLI evidence
and D0_raw only; they are not promoted to the versioned learning schemas under
`schemas/learning/v1/` until the full prompt surface and CP-HUMAN-TRACE pass.
Pre-canonical v1 artifacts run through a frozen compatibility adapter that
reconstructs their original menus and payment policy. Their recorded typed
action stream, state hashes, winner, life totals, and final hash remain exact;
new instrumentation counters are not allowed to weaken those semantic checks.

## Promotion Rule

T4 implementation may proceed on diagnostics. Promotion, tuning claims, and
human-derived learning remain blocked until CP-AI-BENCH and the later human
trace gates satisfy the clarification's immutable splits, leakage checks,
canaries, full prompt adapters, and review requirements.
