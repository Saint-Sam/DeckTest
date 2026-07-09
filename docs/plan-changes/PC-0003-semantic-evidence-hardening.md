# PC-0003: Semantic evidence hardening

Date: 2026-07-09

Status: Accepted and incorporated in Master Plan v1.3.

## Motivation

Raw oracle-file totals can be inflated by parameter variations of a small set
of templates. The T2 nightmare suite validates useful kernel fixtures but does
not yet compile and play real card definitions. The Owner wants defects limited
as closely as practicable to cosmetic or text-only issues.

## Exact Plan Change

1. Keep raw scenario counts, but add generated metrics for distinct scenario
   families, covered rules interactions, hand-authored cases, and mutation
   score. A tier cannot satisfy semantic breadth with file count alone.
2. Make every unopened tier gate fail closed. `SKIP` is never a successful gate
   result.
3. Before the T3 exit gate, convert all ten existing T2 nightmare fixtures into
   compiled card-driven decks or scenarios that pass through cardc, the runtime
   card loader, casting/activation, and normal game actions. The fixtures are:
   Humility class, Opalescence class, Blood Moon class, copy/full-layer stack,
   dependency/timestamp, type-removal/keyword, CDA/modifier/switch,
   global/specific color, control-change, and all-layer cleanup.
4. Make `card_regression.sh` compile all cards, run generated smoke tests, run
   semantic packs, validate coverage metrics, and fail on an uninitialized
   quarantine database.
5. Add a resource-aware multi-worker local fuzz mode. Workers share bounded
   seed corpora but use separate artifact and target directories.
6. Require the CP-DSL packet to include 100 stratified cards, parser mutation
   checks, malformed-source diagnostics, deterministic database hashes, and a
   corpus expressiveness report.
7. CP-DSL fails unless its 100 cards cover at least 25 declared strata, every
   mandatory layout/mechanic stratum has an example, all examples round-trip,
   at least 50 malformed inputs report file/line/column, three clean builds
   produce the same database hash, and curated parser/compiler mutants achieve
   at least a 90% kill rate with no surviving P0/P1 validation mutant.
8. Generated scenario metrics collapse scalar-only variations into one family.
   Each tier gate declares its required family/interaction thresholds; raw file
   totals alone can never satisfy the gate.
9. Worker counts are detected at runtime. Reserve at least two logical cores or
   10% of available cores, whichever is greater, and cap workers when disk or
   memory headroom is insufficient.
10. Card regression compiles every in-scope playable `CardDefinition` and
    validates every catalog/classification record. Catalog-only or out-of-v1
    records must be classified but are not falsely required to compile as
    playable mechanics.

## Affected Tasks

T2.10 addendum, T3.1-T3.7, all future gates, fuzz tooling, gate bundles, and
success-metric reporting.

## Risks And Mitigations

- Risk: stronger checks increase local runtime.
  Mitigation: use 22 of 24 CPU cores for ordinary builds and bounded parallel
  workers for fuzzing; reserve two cores for system responsiveness.
- Risk: mutation testing becomes expensive.
  Mitigation: target boundary predicates and representative scenario families
  at task time, then broaden at tier gates.

## Approval

The Owner approved all recommendations from the 2026-07-09 plan/progress audit
and requested fast local execution without GitHub Actions or unnecessary agent
token use.

Gate Reviewer recommendation: RECOMMENDED on 2026-07-09 after requiring
quantitative thresholds, resource-portable worker selection, named fixture
scope, and alignment with the card identity contract. The final text
incorporates those requirements.
