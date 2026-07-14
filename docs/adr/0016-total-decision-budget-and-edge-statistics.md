# ADR-0016: Total Decision Budgets And Edge Statistics

Status: accepted and implemented locally, 2026-07-14; exact campaign refresh
pending.

## Context

The first product-bound 1/2/4 ms search-knee smoke reported roughly 250-273 ms
p95 latency. Inspection found two correctness problems:

1. each determinization tree created its own wall deadline, so one decision
   could consume the configured budget repeatedly;
2. visit and value totals were read from transposed child nodes, so two actions
   converging on one state inherited each other's action evidence.

The same review found that one-worker searches still spawned a thread and that
a 64-bit key alone was trusted as proof of state identity.

## Decision

Wall time is a budget for the complete decision, not each tree. Production
adapters capture an `Instant` before canonical context construction and pass it
into `SearchConfig`. `SearchEngine` creates one deadline from that start and
shares it across every worker and determinization. Once it expires, later
sequential determinizations are not started. One non-preemptible operation may
overrun the deadline, but the engine may not multiply the budget by starting
additional sequential work.

The `workers=1` path runs inline. Multi-worker fixed-iteration search keeps its
deterministic static assignment. Wall-time reports count only determinizations
that actually started and completed.

Action statistics now live on parent edges. Transposition nodes retain
state-level visits and values, while selection, root reports, adaptive
checkpoints, and aggregate action choice use edge visits and values.

Transposition lookup uses `SearchStateKey`, a two-component key, only to choose
a bucket. A domain must additionally prove complete search-state equivalence.
The default returns false, disabling sharing. Production game domains compare
the full canonical `GameState` bytes plus search-relevant terminal/context
metadata before sharing a node.

## Consequences

- `think_ms` now has one explicit total-decision meaning.
- converging legal actions cannot borrow each other's visits or values.
- hash collisions reduce sharing but cannot merge unequal states.
- fixed-iteration exact replay remains deterministic and iteration-authoritative.
- tiny wall budgets may still overrun by one expensive context build,
  determinization, or typed transition; phase-level telemetry and action-surface
  factoring remain required performance work.
- every wall-time ladder artifact generated before this decision is diagnostic
  defect evidence and must not be used for promotion.

## Verification

Focused tests require:

- root edge visits sum to completed simulations even when children converge;
- deliberate equal-key, unequal-state inputs produce no transposition hit;
- an expired one-worker budget starts only one required fallback
  determinization;
- caller-side context time consumes the same decision budget;
- fixed-iteration root-parallel selection remains exactly replayable.

The next exact T4 packet must also rerun the three baseline replays, strict
clippy, cross-target compiles, and the 1/2/4 ms smoke from one frozen product
commit before the old latency artifact is replaced.
