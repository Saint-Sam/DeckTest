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
`DecisionStateKey` contains `PlayerViewHash` plus those sorted action IDs and is
the machine key for Track B near-state deduplication. Hierarchical subcontexts
also carry a typed-path discriminator derived only from actor-visible prior
choices. That discriminator participates in both the context ID and state key,
so identical-looking later prompts reached through different declarations are
not collapsed.

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
- main-phase land plays, normal-cost permanent/instant/sorcery casts, mana
  activations, every enumerated mana payment, and priority pass;
- printed X spell costs through one deferred cast-family option followed by a
  scoped numeric context and a scoped payment context; values above 64 choices
  narrow through binary subranges, while the final option remains the same
  fully bound typed `CastSpell` action used by replay and the kernel;
- exact normal-spell target groups, including statically bounded ranges and
  positive fixed-total divisions, modal branches, and optional-effect answers,
  preserved in the canonical descriptor and stack hash and rebound through
  real resolution; human labels expose every bound target and allocation;
- typed program-bound non-mana activations with exact payments, targets, and
  optional answers announced through the kernel stack, followed by interpreter
  effect binding only after successful resolution;
- scoped slot-by-slot resolution-time object choices for spells and
  program-bound activated and triggered abilities,
  including legal fail-to-find branches, authoritative characteristic
  filtering, human search labels, AI successor evaluation, canonical telemetry,
  typed visible-path identity, and exact decision/action replay; multiple
  requirements no longer form an eager Cartesian product;
- every live human and AI priority window, with legal normal-cost instants and
  a forced-pass fast path that does not invoke search or one-ply evaluation;
- complete player-defender attack assignments, including split attacks, built
  as one bounded canonical subcontext per attacker rather than an exponential
  Cartesian product;
- complete blocker assignments for every attacked player, built as one bounded
  subcontext per blocker, menace-completion checked, submitted in deterministic
  APNAP order, and accumulated by the kernel;
- complete canonical commander move-or-leave choices for both human and AI
  controllers, with selected-ID membership and telemetry regression coverage;
- kernel-validated simultaneous-trigger ordering in APNAP controller groups,
  represented as one bounded next-trigger subcontext at a time for live human
  and AI controllers rather than complete-order permutations;
- kernel-validated triggered-ability targets announced while each pending
  trigger is put on the stack, represented as scoped add-or-finish group
  contexts followed by bounded allocation contexts, with exact target
  snapshots retained through priority and resolution;
- an explicit kernel-validated no-stack disposition when a required trigger
  target slot has no legal choice, consuming that pending instance without a
  human or AI prompt while preserving every valid sibling trigger;
- kernel-recorded per-slot target legality at resolution, carried into spell,
  activated, and triggered interpreter bindings so effects depending on an
  illegal target are skipped while legal-target and untargeted instructions
  continue;
- kernel-validated combat-damage ordering and sequential amounts for human and
  AI controllers, using bounded next-blocker contexts and binary range
  narrowing above 64 direct amounts rather than complete distributions;
- explicit human, AI, and benchmark concession through one hidden-safe
  singleton `Concession` context, while ordinary random and search action sets
  exclude concession unless a caller explicitly requests that context;
- one shared sorted context for the live human and AI main-phase, attacker,
  blocker, and commander-zone adapters; presentation labels are derived from
  those typed options rather than independently enumerated menus;
- canonical seeded random-legal and deterministic/noisy one-ply policies;
- root-parallel determinized search over main and hierarchical attacker/blocker
  contexts, with every selected ID revalidated against its supplied context and
  only the final complete combat declaration dispatched to the kernel.

The current path remains `limited` and diagnostics-only because it does not yet
canonicalize the full Commander prompt surface: unsupported activation cost
families, dynamic or cross-target distributions, non-cost numeric values,
remaining additional and alternative costs, trigger-order benchmark labels,
modal trigger choices, and non-player combat
defenders. Normal spells, the current
typed activated slice, and ordinary targeted triggers cover target announcement;
payments and spell/activated/triggered object searches also share the boundary.
Same-batch stack-entry targets now use deterministic prospective IDs and atomic
staged kernel validation, so later triggers can target prior entries without
opening a priority window or accepting a forward reference. The remaining
prompt families stay partial until their multi-zone, dynamic-distribution, and modal
forms use the same rules path.
The live concession adapter is complete, but its sealed benchmark fixture is
still pending with the other Track B evidence. Remaining gaps cannot be
silently skipped for CP-AI-BENCH,
CP-HUMAN-TRACE, teacher-corpus eligibility, or product-strength claims.

## Replay And Data Boundary

T4 AI replays persist canonical context/action IDs, `PlayerViewHash`, complete
typed legal-action descriptors, policy identity, score components, immutable
game inputs, search checkpoint history, and expected final state. Exact replay
regenerates decisions and typed actions from the same seeds and rejects any
divergence.

Replay records also carry the additive strategic-episode fields ratified in
ADR 0035. Ordinary prompts form singleton episodes. Main and priority action
chains, trigger announcement and ordering, resolution-time choices, trigger
unless payments, combat declarations, and combat-damage assignment each link
their hierarchical prompts under one root and one final canonical action-path
ID. Forced subchoices remain replayed but do not count as independent strategic
decisions. Exact current-product episode evidence and Track B consumer
validation remain required before teacher-corpus promotion.

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
