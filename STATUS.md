# DeckTest / Forge 2.0 Status

Generated: 2026-07-12T01:48:29.432054+00:00 by `tools/write_card_maturity.py`

Plan: v1.8

Verification: local only; GitHub Actions disabled

No single percentage represents project completion. Counts below retain their
literal units; compiler success is not semantic or product readiness.

## Product Tracks

| Track | Current state |
| --- | --- |
| Forge Standalone / Local Trainer | T3 card factory active; focused Trainer and human play remain gated |
| PodBench | Private report-only roadmap bridge; no real worker, customer exposure, training, or launch authorized |

## Catalog Scope

| Unit | Count |
| --- | ---: |
| English printings represented | 113,234 |
| Oracle identities classified | 38,306 |
| In v1 scope | 32,792 |
| Out of v1 | 1,900 |
| Catalog only | 3,614 |

## Identity Maturity

Exclusive highest evidence stage for the 32,792 in-v1 Oracle identities:

| Highest stage | Identities |
| --- | ---: |
| Absent identity-bound definition evidence | 21,492 |
| Parsed | 0 |
| Mapped partial | 0 |
| Structurally translated | 0 |
| Compiler valid | 11,300 |
| Runtime smoke passed | 0 |
| Semantic verified | 0 |
| Pod integration verified | 0 |
| AI supported | 0 |
| Product eligible | 0 |

Compiler-valid evidence currently reaches 11,300/32,792
in-v1 identities (34.4596%). This includes the
unverified CP-DSL language-stress corpus and therefore is not a playable claim.
Parsing and mapping remain below in their own units rather than being guessed
onto identities.

## Card Factory

| Literal unit | Result |
| --- | ---: |
| Legacy scripts parsed | 33,290/33,290 |
| Compiler-valid translated legacy definitions | 11,241 |
| Fail-closed quarantined legacy definitions | 22,049 |
| Structurally tested legacy ability uses | 19,246/43,649 |
| Quarantined legacy ability uses | 24,403 |
| Owner-priority compiler-valid definitions | 193/365 |

## Evidence Breadth

| Literal unit | Count |
| --- | ---: |
| Scenario files | 1,200 |
| Distinct scenario commands | 65 |
| Distinct operations | 19 |
| Observed semantic atom combinations | 1,839 |
| Hand-authored scenarios | 133 |
| Cross-compile artifacts passed | 4 |

## T3.3 Mapper Progress

The latest exact local-only product is `89a527b`. It emits 11,241/33,290
complete legacy scripts (33.7669%) and maps 19,246/43,649 top-level ability
uses (44.0926%), including 193/365 Owner-priority identities. The latest batch
maps permanent `Pump` and `PumpAll` PT and closed keyword effects by omitting
the temporary-duration argument; hidden untap prose, unsupported keywords,
mixed cleanup, and open values remain quarantined. The detached checkpoint
passed deterministic 24/12 translation, 6/1 planner replay, full workspace
tests, strict clippy, compiler/database validation, 235 oracle scenarios plus
gated subsets, nightmare/smoke, and 80.9792% line coverage. T3.3 remains
active; the 60% complete-script floor is not reached.

## Next Gates

1. Continue the measured T3.3 mapper lane at 40% capacity until the corpus floor
   is met; every batch records before/after coverage and quarantine deltas.
2. T3.5 capability-specific runtime smoke; unsupported setup is reason-coded.
3. T3.6 and CP-CARD-SEMANTICS-100 for the frozen 100-card Commander set.
4. T3.9 and CP-FOUR-PLAYER-POD with four complete decks and 1,000 seeded games.
5. T1.R10 and CP-HUMAN-PLAY-CLI before trace collection or Trainer claims.

Per-identity generated detail: `target/card-maturity/identities.json` (untracked,
38,306 records; SHA-256
`9a68c672a807f62a78f3bea73c551da32a88efc06a8a9e5bfe8402d6bc87152f`).
