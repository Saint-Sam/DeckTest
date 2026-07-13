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
