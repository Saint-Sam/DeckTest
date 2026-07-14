# ADR-0013: Data-Driven Card Runtime Interpreter

Date: 2026-07-13

## Status

Accepted for incremental implementation by the Owner on 2026-07-13 under
T3.5, T3.6, and T3.9. This ADR adds no external dependency and does not change
the frozen card DSL. The architecture decision is accepted; Tier gate
acceptance still requires the local verification listed below and review with
the Tier 3 evidence packet.

## Context

The card compiler produces validated, recursive `CardDefinition` IR, while
`forge-core` exposes deterministic production actions for game rules. Before
this decision, the repository had no general component that interpreted card
IR into those actions. `forge-testkit` contained a deliberately bounded smoke
adapter for a few literal effects, and `forge-arena` contained a separate
driver for the ten continuous-layer nightmare fixtures. Neither is a product
card runtime.

That gap blocks the next vertical gates. T3.5 must execute supported compiled
capabilities through production actions, T3.6 must prove the frozen 100-card
Commander set with card-specific expected behavior, and T3.9 must run complete
decks through the same mechanics used by the client and AI. Extending
test-only adapters would create multiple semantic implementations and could
allow tests to pass without proving product behavior.

The following constraints are binding:

- `forge-core` remains card-agnostic and contains no card names or
  card-specific branches.
- Card behavior comes from validated IR, never from Oracle text parsing at
  runtime.
- Every state mutation crosses `forge_core::apply` as a typed `Action`.
- Unsupported operations, selector shapes, values, costs, events, or choices
  fail closed with stable diagnostics; they never become no-ops or passes.
- Compilation and execution are deterministic for the same definition,
  binding context, choices, and game state.
- Existing serialized IR and kernel encodings remain append-only.
- The interpreter must serve tests, clients, and AI rather than becoming a
  private testing hook.

## Decision

Add the card runtime interpreter to `forge-cards`, the plan-assigned runtime
card-data crate. `forge-cards` gains an internal workspace dependency on
`forge-core`; `forge-core` does not depend on `forge-cards`, so the rules kernel
remains reusable and card-agnostic. No new third-party crate is introduced.

The runtime has two explicit phases:

1. **Compile:** Validate one `CardDefinition` into an immutable typed
   `CardProgram`. Compilation traverses every relevant face, ability, cost,
   event, condition, timing expression, selector, and effect. It either
   produces a complete program or a stable `CompileDiagnostic` carrying the
   IR path and unsupported operation. Compilation completes before gameplay
   state is mutated.
2. **Execute:** Bind a compiled program to runtime objects, players, stack
   entries, trigger subjects, remembered values, and explicit choices in an
   `ExecutionContext`. Execution emits and applies production `Action` values,
   records the action/outcome trace, and checks the expected postconditions for
   the selected capability. A rejected production action becomes a stable
   `ExecutionDiagnostic`; it is never converted to success.

The interpreter owns card-language semantics and binding, but not kernel rules.
For example, the interpreter selects `Action::DrawCards` for `draw`, while the
kernel owns empty-library consequences and zone movement. It selects a legal
target and supplies a `CastSpellRequest`, while the kernel revalidates timing,
targeting restrictions, payment, priority, and resolution. This double
boundary is intentional: IR validation prevents unsupported meanings, and
kernel validation prevents illegal game actions.

Programs and execution paths are capability-oriented, not card-oriented. A
source search for frozen card names in runtime and kernel code must remain
empty. Shared operations such as draw, move-zone, target selection, triggered
events, activated costs, continuous effects, and token creation are
implemented once and reused by every definition.

Choice-dependent semantics use explicit choice inputs or typed prompts. The
interpreter must not select a strategically convenient choice and call that
general execution. Deterministic T3 smoke scenarios may supply documented
fixed choices through the same public choice boundary. Missing choice support
is reason-coded as unsupported until the prompt exists.

The initial migration is incremental but one-way:

1. Move the existing life, draw, scry, shuffle, mana-cost, cast, and permanent
   destination lowering into `forge-cards` programs.
2. Add the high-fanout T3.6 families: lands and mana abilities, targeted zone
   changes, counterspells, search/move/shuffle, tokens, activated costs,
   triggers, keywords, and continuous effects.
3. Make T3.5 and T3.6 consume the interpreter and delete equivalent lowering
   from `forge-testkit`.
4. Make T3.9 deck and pod execution consume the same interpreter entry points.
5. Expose explicit prompts for human and AI clients as interactive choices are
   reached; do not add card-specific client workarounds.

## Initial Implementation Record

The first product slice was implemented in the same change as this ADR. It
moves effect compilation and execution out of `forge-testkit` and into
`forge-cards::runtime`, while retaining `forge-testkit` only as the deterministic
scenario builder and verifier. The slice includes:

- immutable `CardProgram` compilation with stable, path-qualified diagnostics;
- explicit target, opponent, and scry-choice bindings that are all validated
  before the first state mutation;
- permanent spells plus life changes, card draw, scry, library shuffle,
  permanent destruction, object exile, stack-entry counters, and typed zone
  movement;
- generic kernel support for draw, shuffle, stack counters, base object
  characteristics, destruction, and additional target predicates;
- action-by-action invariant checks and typed pass, unsupported, and failure
  outcomes in the runtime-smoke command.

The initial local corpus audit executes 515 of 20,082 translated definitions,
reason-codes 19,567 as unsupported, and reports zero execution failures. It
raises the runtime-smoke baseline by 136 definitions. Seventeen members of the
frozen 100-card semantic set execute through the new runtime, including real
targeted exile, counterspell, destruction, dynamic life, draw, and graveyard
return examples. These are runtime-smoke results only: no card is promoted to
`semantic_verified` until its card-specific T3.6 oracle and deterministic
replay pass.

This record deliberately does not claim broad rules completeness. Activated
abilities, triggers, most keywords, tokens, searches, alternate layouts, and
other unsupported families remain fail-closed and are the next vertical
implementation work.

## Token and stack-predicate extension

The next vertical slice extends the same interpreter rather than introducing a
parallel card executor. Token creation compiles only exact token-script IDs in
an explicit data registry. The first registry contains the four legacy 3/3
green vanilla templates needed by Beast Within, Generous Gift, Pongify, and
Rapid Hybridization. The compiler binds token ownership and control before any
preceding target mutation, caps literal counts, and emits production
`Action::CreateToken` actions carrying exact base types, color, power, and
toughness. Unknown templates, metadata-bearing forms, nonliteral counts, and
tokens with keywords or abilities remain unsupported.

Stack spell predicates now compile a closed grammar of `type_is(...)`,
`not(type_is(...))`, and `or(type_is(...), ...)`. The kernel applies the
resulting object predicate to the physical spell object behind a stack entry;
ability-only stack entries do not match. The smoke synthesizer constructs a
legal spell type satisfying the predicate and verifies the same production
target checks used during casting and resolution.

This extension raises the local translated-corpus result from 547 to 567
runtime passes with zero runtime failures. The frozen Commander semantic set
moves from 24 to 30 runtime-smoke passes. These remain capability-level smoke
claims only: token subtype behavior, non-vanilla templates, and card-specific
semantic expectations remain T3.6 work.

## Type-line and explicit library-choice extension

Basic lands and library search require real object characteristics rather than
test labels. `forge-core` therefore carries all five closed Magic supertypes
and the five basic land types in base and effective object characteristics.
Both sets participate in canonical bytes and deterministic hashes. A compiled
basic land receives its exact Basic supertype, land type, and intrinsic mana
ability; runtime predicates inspect those effective characteristics.

The interpreter compiles `search_library` only for a closed selector grammar:
top-level card types, required supertypes, the five basic land types, negated
top-level types, and homogeneous type or basic-land-type unions. The caller
supplies selected object IDs through an explicit choice slot. Before any
effect action is emitted, execution verifies ownership, library membership,
cardinality, uniqueness, and every compiled characteristic predicate. Chosen
objects can then move through production actions to hand or battlefield, be
placed on top of their existing library, and be tapped when required. Unknown
subtypes remain unsupported rather than being approximated.

The legacy mapper was also corrected so library selectors such as
`Instant,Sorcery` lower to top-level type predicates instead of subtype
predicates. The corrected local corpus executes 605 of 20,082 translated
definitions, with 19,477 typed unsupported results and zero failures. The
frozen Commander set moves from 30 to 40 runtime-smoke passes. These are still
capability-level smoke claims; hidden-information prompts, card-specific
expected outcomes, and deterministic semantic replays remain T3.6 work.

## Canonical activation and resolution-choice extension

The T4 vertical integration keeps rules ownership split at the same boundary.
`forge-core` owns activation timing, costs, target announcement and snapshots,
priority, stack identity, target revalidation, and the resolution record. A
program-bound activation has no card-specific kernel effect body. Only after the
kernel records a successful resolution does the driver ask `forge-cards` to bind
the compiled effect program into ordinary production actions.

Announcement-time targets and optional answers are carried in the stack entry
and its canonical hash. Hidden-zone object choices are deliberately not chosen
at activation: intervening priority may change the searched zone. At resolution,
the adapter enumerates every bounded legal object product through the same
`object_satisfies_choice_requirement` predicate used by execution validation.
The resulting canonical options map to the fully bound production action
sequence. Legal fail-to-find branches remain present for library searches,
human labels reveal only information the searching controller is authorized to
inspect, and AI policies evaluate successor `PlayerView` values produced by the
real action sequence.

`ChooseResolutionObjects` is an append-only composite decision descriptor for
zero, one, many, or multiple ordered choice slots. This avoids overloading the
older singular search descriptor and keeps replay IDs structural. Evolving
Wilds is the first real activated vertical regression: its activation sacrifices the
source, resolves through priority, excludes a nonbasic land, offers the legal
empty and matching-basic choices, moves the selected basic to the battlefield
tapped, and shuffles through kernel actions. A four-seat AI diagnostic then
completed 241 turns, 18,035 typed actions, and 16,000 canonical decisions with
an exact replay match after exercising the same adapter.

Triggered programs use the same delayed choice contract. Sword of the Animist
has a focused regression that resolves its attack trigger into a canonical
library-search context, excludes a nonbasic Forest, retains the legal
fail-to-find branch, moves the selected basic land to the battlefield tapped,
and shuffles through production actions. Trigger targets, ordering, and
deferred optional prompts remain separate fail-closed work.

This extension is not a claim that every search or activated cost is complete.
Unsupported extra costs still fail closed, and spell-resolution choices,
trigger targets/order/optionals, multi-zone, and large combinatorial choice
families remain explicit T4 decision-surface gaps. Exact promotion evidence
must be regenerated from the final clean T4 product commit rather than inherited
from an earlier diagnostic tree.

## Consequences

- Semantic work becomes reusable product code, so a T3.6 pass proves the path
  later used by local play and AI.
- `forge-cards` becomes responsible for both validated card-data access and
  data-driven execution. Its test and coverage burden increases accordingly.
- Some currently missing kernel primitives must be added as generic actions.
  Each must be card-neutral, deterministic, independently tested, and
  append-only where it affects canonical encodings.
- Complete fail-closed compilation can expose more unsupported definitions in
  the short term. That is preferable to partial execution incorrectly counted
  as runtime or semantic success.
- Interpreter execution traces provide a common diagnostic and replay surface
  for semantic tests, clients, and AI integration.
- The runtime must eventually support multiplayer-relative selectors and
  explicit choices; a two-player smoke binder is evidence for only the
  capability it exercises.

## Alternatives Considered

- **Keep extending `forge-testkit`:** rejected because tests could implement
  semantics the product does not use, invalidating T3.6 evidence.
- **Put the interpreter in `forge-core`:** rejected because it couples the
  deterministic rules kernel to card database schemas and violates the
  card-specific boundary.
- **Create one Rust handler per card:** rejected because it cannot scale to the
  corpus, duplicates shared mechanics, prevents clean AI support, and violates
  the no-card-names rule.
- **Interpret Oracle text directly:** rejected because Oracle text is not the
  frozen executable contract and would create a second, ambiguous parser.
- **Generate Rust source from every card:** rejected for the initial runtime
  because build size, compilation cost, and dynamic database updates are worse
  than interpreting validated typed IR. A future measured code-generation
  optimization may compile the same `CardProgram` contract without changing
  semantics.
- **Leave effects in the client/controller:** rejected because desktop, mobile,
  WASM, tests, and AI would diverge.

## Verification

Acceptance evidence must include, locally and offline:

- unit tests for every added compiler and executor operation, including
  malformed and unsupported shapes;
- deterministic replay equality for compiled programs and execution traces;
- production-action rejection tests proving fail-closed propagation;
- `cargo test --offline -p forge-core -p forge-cards -p forge-testkit`;
- `cargo clippy --offline -p forge-core -p forge-cards -p forge-testkit
  --all-targets -- -D warnings`;
- `cargo fmt --all -- --check` and `git diff --check`;
- the repository no-card-name boundary check;
- a T3.5 corpus report with zero unclassified results and zero implicit passes;
- CP-CARD-SEMANTICS-100 evidence showing 100/100 card-specific scenarios use
  interpreter entry points and production actions with deterministic replay;
- T3.9/CP-FOUR-PLAYER-POD evidence showing deck execution uses the same runtime
  rather than a test-only adapter.

This ADR does not itself claim those gates have passed. Their exact reports and
review decisions remain separate evidence artifacts.
