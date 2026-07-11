# DeckTest / Forge 2.0 Status

Generated: 2026-07-11T23:32:54.509625+00:00 by `tools/write_card_maturity.py`

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
| Absent identity-bound definition evidence | 22,014 |
| Parsed | 0 |
| Mapped partial | 0 |
| Structurally translated | 0 |
| Compiler valid | 10,778 |
| Runtime smoke passed | 0 |
| Semantic verified | 0 |
| Pod integration verified | 0 |
| AI supported | 0 |
| Product eligible | 0 |

Compiler-valid evidence currently reaches 10,778/32,792
in-v1 identities (32.8678%). This includes the
unverified CP-DSL language-stress corpus and therefore is not a playable claim.
Parsing and mapping remain below in their own units rather than being guessed
onto identities.

## Card Factory

| Literal unit | Result |
| --- | ---: |
| Legacy scripts parsed | 33,290/33,290 |
| Compiler-valid translated legacy definitions | 10,716 |
| Fail-closed quarantined legacy definitions | 22,574 |
| Structurally tested legacy ability uses | 18,672/43,649 |
| Quarantined legacy ability uses | 24,977 |
| Owner-priority compiler-valid definitions | 189/365 |

The latest exact T3.3 mapper batch (`4480224`) adds closed
`TriggeredCard$CardPower`, `$CardToughness`, `$CardManaCost`, and
`$CardCounters.<type>` value-SVar lowering through the typed triggered-object
selector. It raises complete legacy scripts to 10,716/33,290 (32.1898%) and
mapped ability uses to 18,672/43,649 (42.7776%). Aggregate, filtered,
controller, and LKI-dependent forms remain fail-closed. The detached local
checkpoint passed deterministic translation/planner replay, full workspace
tests, compiler/database validation, 235 oracles, clippy, and 80.8399% line
coverage. T3.3 remains active below its 60% exit floor.

## Evidence Breadth

| Literal unit | Count |
| --- | ---: |
| Scenario files | 1,200 |
| Distinct scenario commands | 65 |
| Distinct operations | 19 |
| Observed semantic atom combinations | 1,839 |
| Hand-authored scenarios | 133 |
| Cross-compile artifacts passed | 4 |

## Next Gates

1. Continue the measured T3.3 mapper lane at 40% capacity until the corpus floor
   is met; every batch records before/after coverage and quarantine deltas.
2. T3.5 capability-specific runtime smoke; unsupported setup is reason-coded.
3. T3.6 and CP-CARD-SEMANTICS-100 for the frozen 100-card Commander set.
4. T3.9 and CP-FOUR-PLAYER-POD with four complete decks and 1,000 seeded games.
5. T1.R10 and CP-HUMAN-PLAY-CLI before trace collection or Trainer claims.

Per-identity generated detail: `target/card-maturity/identities.json` (untracked,
38,306 records; SHA-256
`31568c841c22c2fed67e04c873adfb6af1575b3b38da790a8002d3e0360269d2`).
