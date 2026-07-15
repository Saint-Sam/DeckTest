# T4 Determinized Search

Status: T4.4 product path implemented locally; T4.5 upgrades active. This
specification records implementation behavior and does not declare the T4 exit
gate passed.

## Boundary

Search consumes a canonical `DecisionContext` and an opaque `SearchDomain`.
The generic search engine never receives a production `GameState`, display
label, card name, or unredacted opponent identity.

The four-player adapter constructs each sample from:

- the acting player's `PlayerView`;
- exact per-seat deck multisets;
- retained known/revealed information;
- a deterministic sample seed.

`GameState::determinized_clone` is the only bridge into a simulation state. It
requires every redacted hand/library slot exactly once and rejects partial,
duplicate, out-of-range, known, token, copy, or non-hidden assignments. It
rebinds printed characteristics and card IDs only in a clone. The adapter also
rebinds the corresponding typed `CardProgram`, so a sampled identity executes
its sampled behavior rather than the live state's hidden program.

## Search v1

`SearchEngine` currently runs one independent UCT tree per determinization and
aggregates concrete root actions. Fixed-iteration configurations are used for
development and exact replay. A wall-time limit exists for diagnostics, but is
not yet the replay-authoritative CLI mode or a valid product decision budget.

The product adapter captures one decision start before canonical context and
legal-action construction. `SearchEngine` derives one deadline from that start
and shares it across every determinization and worker. It does not start later
sequential determinizations after expiry, and `workers=1` executes inline
without thread creation. A single context build, determinization, or typed
transition remains non-preemptible, so measured wall time may exceed the
budget by that operation; the budget can no longer be multiplied by a sequence
of independently timed trees. Supported server telemetry begins before context
construction and includes parent setup/aggregation plus summed worker CPU.

The current four-player product adapter searches:

- main-phase land, mana-activation, autonomous permanent-cast, and finish
  sequences;
- bounded per-attacker choices across every legal player defender, carrying the
  partial declaration through deeper search states;
- bounded per-blocker choices for each attacked defender, including fail-closed
  menace-completion viability before a branch is offered.

Spell, activated, and triggered resolution-time object choices use the same
bounded subcontext rule outside the main search tree: one complete canonical
context per compiled requirement, with prior selections retained in typed path
state and interpreter actions bound only after the final slot. This removes
the cross-requirement Cartesian product while preserving every legal concrete
selection for human, AI, telemetry, and replay consumers.

Targeted triggered abilities use bounded target-slot contexts before the
pending trigger is put on the stack. The final typed trigger binding is
validated atomically by the kernel, snapshots targets for the priority window,
and is recovered from the resolution record by the interpreter. A required
trigger with an empty current target domain uses an explicit kernel-validated
no-stack disposition, produces no prompt, and does not block valid sibling
triggers. Same-batch inter-trigger stack targeting, target distribution, and
partial-illegality filtering remain fail closed.

Triggered optional effects are deferred until resolution and use one scoped
`Optional` context per compiled group. Compiled opponent-draw unless branches
retain the queued event's exact player through the stack-entry identity, then
offer that player a pay-or-decline context and, only after acceptance, a
separate exact `Payment` context. An unaffordable cost is a singleton decline;
it receives no search budget. Other event-bound payer families remain fail
closed rather than inheriting the trigger controller.

Spells with the currently compiled discard-card or sacrifice-permanent
additional costs use one scoped `Payment` context per printed cost group before
the X and mana-payment stages. Each partial selection must have at least one
complete continuation, and prior selections remain in typed path state, so the
adapter never constructs a cross-cost Cartesian product. The kernel validates
the entire announcement before mutation and then pays additional costs in
order. Alternate and other uncompiled non-mana cost families remain fail
closed.

Combat-damage ordering and amounts also use bounded contexts outside the main
tree. Controllers choose one next blocker and then one amount at a time; large
numeric ranges are narrowed through binary subranges before a direct context
is built. The rollout policy consumes the same contexts as a human controller,
and the kernel validates the final complete ordered assignment.

The typed kernel still receives one complete declaration. Hierarchical path
bindings participate in decision-state keys and transposition equivalence, so
the smaller action surface does not merge different partial declarations.

Opening hands use the typed mulligan policy. Other currently adapted policy
surfaces retain the deterministic rollout policy. Missing production prompt
families remain fail-closed under `DECISION_CONTEXT.md`; search does not make
them complete by implication.

## T4.5 Mechanics

The tree supports:

- a transposition table keyed only when the domain supplies a complete
  deterministic state key;
- deterministic progressive widening ordered by card-agnostic action priors;
- a typed action-family hook that can explore one option from each family
  before opening later variants while retaining every concrete legal action;
- target-family abstraction for casts and program-bound activations: target
  handles are normalized for widening order while source, ability, payment,
  target kinds, modes, and optional answers remain bound. Concrete canonical
  IDs, transitions, edge statistics, membership checks, and replay actions are
  never merged;
- fixed-visit adaptive checkpoints and forced/singleton bypasses.

Visit/value totals now live on action edges while transposition nodes retain
state-level totals. Converging actions therefore keep independent evidence.
Lookup uses a wide domain key only to select a bucket, then requires the domain
to prove complete canonical-state equivalence before sharing. The fail-closed
default disables sharing. Regression tests cover converging edges, deliberate
key collisions, shared total deadlines, caller-side context time, and inline
single-worker expiry.

The production determinization adapter constructs the sampled `GameState`
once. It no longer clones the live state into a temporary `GameDriver` and then
immediately replaces it with a second determinized clone. This preserves exact
sample semantics while removing a full-state copy from every searched
decision.

Adaptive leader/gap/uncertainty stopping remains experimental. It cannot ship
until paired ablation against fixed budgets passes Tracks A, B, and C under
`T4_SEARCH_KNEE.md`.

## Telemetry

Every genuine searched decision records policy, legal actions,
determinizations, simulations, nodes, maximum depth, transposition hits, wall
latency, value gap, visit gap, normalized uncertainty, and stop reason.
It also records the configured iteration or wall budget, leader visit share,
checkpoint count, ranking stability, bounded-solver state, and whether
experimental adaptive stopping was enabled. Singleton options report zero
search work. Linux and Android populate measured thread-CPU and process
resident-memory deltas through safe `/proc` adapters. Unsupported platforms
retain explicit unavailable fields; CPU is never inferred from wall time.
Worker price and utilization remain campaign inputs.

## Promotion Limits

The tiny fixed-iteration full-game smoke proves execution and exact replay, not
playing strength. T4 promotion still requires sealed benchmark evidence,
paired arena calibration, three archetype decks, at least 400 games per rung,
latency evidence on required reference platforms, full shipped-card support,
and Owner CP-AI-LADDER review. No broad T3 reopening is authorized.
The exact diagnostic packet binds hierarchical combat and single-construction
determinization to exact replays. Its refreshed 1/2/4 ms ladder measures
approximately 1.5-4.7 ms p95 rather than the former 240-266 ms floor. This
closes the diagnosed eager-combat timing defect, but CPU/memory campaigns,
competence labels, confidence, and reference devices still block a latency,
cost, or knee promotion claim.
