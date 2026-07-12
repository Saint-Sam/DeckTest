# DeckTest / Forge 2.0 Status

Generated: 2026-07-12T01:02:29.438875+00:00 by `tools/write_card_maturity.py`

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
| Absent identity-bound definition evidence | 21,683 |
| Parsed | 0 |
| Mapped partial | 0 |
| Structurally translated | 0 |
| Compiler valid | 11,109 |
| Runtime smoke passed | 0 |
| Semantic verified | 0 |
| Pod integration verified | 0 |
| AI supported | 0 |
| Product eligible | 0 |

Compiler-valid evidence currently reaches 11,109/32,792
in-v1 identities (33.8772%). This includes the
unverified CP-DSL language-stress corpus and therefore is not a playable claim.
Parsing and mapping remain below in their own units rather than being guessed
onto identities.

## Card Factory

| Literal unit | Result |
| --- | ---: |
| Legacy scripts parsed | 33,290/33,290 |
| Compiler-valid translated legacy definitions | 11,050 |
| Fail-closed quarantined legacy definitions | 22,240 |
| Structurally tested legacy ability uses | 18,958/43,649 |
| Quarantined legacy ability uses | 24,691 |
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

The latest exact `Pump` batch lowers closed `AtEOT` cleanup values
`Sacrifice`, `Destroy`, `Exile`, and `Hand` through typed delayed triggers at
the next end step. Open, conditional, combat, controller, remembered,
token-bound, and other non-equivalent forms remain quarantined. Exact product
`fdd798c` emits 11,050 of 33,290 complete scripts (33.1932%) and maps 18,958
of 43,649 ability uses (43.4328%), up 12 scripts and 15 uses from `1c89ea6`;
Owner-priority emission remains 193 of 365. Deterministic 24/12 translation
and 6/1 planner replay, full workspace tests, clippy, compiler/database
validation, 235 oracle scenarios plus gated subsets, nightmare/smoke checks,
and 80.9067% line coverage pass locally without GitHub Actions. T3.3 remains
active; the 60% complete-script exit floor is not reached.

## Next Gates

1. Continue the measured T3.3 mapper lane at 40% capacity until the corpus floor
   is met; every batch records before/after coverage and quarantine deltas.
2. T3.5 capability-specific runtime smoke; unsupported setup is reason-coded.
3. T3.6 and CP-CARD-SEMANTICS-100 for the frozen 100-card Commander set.
4. T3.9 and CP-FOUR-PLAYER-POD with four complete decks and 1,000 seeded games.
5. T1.R10 and CP-HUMAN-PLAY-CLI before trace collection or Trainer claims.

Per-identity generated detail: `target/card-maturity/identities.json` (untracked,
38,306 records; SHA-256
`e7a9977cac51112c8af292cfe81f60aa75b22418984e8fe1c2386e85b119e8df`).
