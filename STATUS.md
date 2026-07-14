# DeckTest / Forge 2.0 Status

Generated: 2026-07-14T08:18:18.889209+00:00 by `tools/write_card_maturity.py`

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
| Absent identity-bound definition evidence | 12,674 |
| Parsed | 0 |
| Mapped partial | 0 |
| Structurally translated | 0 |
| Compiler valid | 20,018 |
| Runtime smoke passed | 0 |
| Semantic verified | 79 |
| Pod integration verified | 21 |
| AI supported | 0 |
| Product eligible | 0 |

Compiler-valid evidence currently reaches 20,118/32,792
in-v1 identities (61.3503%). This includes the
unverified CP-DSL language-stress corpus and therefore is not a playable claim.
Parsing and mapping remain below in their own units rather than being guessed
onto identities.

## Card Factory

| Literal unit | Result |
| --- | ---: |
| Legacy scripts parsed | 33,290/33,290 |
| Compiler-valid translated legacy definitions | 20,082 |
| Fail-closed quarantined legacy definitions | 13,208 |
| Structurally tested legacy ability uses | 32,080/43,649 |
| Quarantined legacy ability uses | 11,569 |
| Owner-priority compiler-valid definitions | 281/365 |

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

1. CP-HUMAN-PLAY-CLI passed with a real Owner game and exact replay. Begin
   T4.1-T4.3 on the exact card-driven runtime proven by Tier 3.
2. After the baseline AI is green, proceed to T4.4+ search and calibration.
3. Reopen T3 only for a concrete T4 blocker, fix the smallest shared primitive,
   add semantic regressions, and resume T4. Broad mapper expansion remains closed.

Per-identity generated detail: `target/card-maturity/identities.json` (untracked,
38,306 records; SHA-256
`370f9b945ac82201338c3250d61e80fe9b9248fbce05a878a43af77d22093c20`).
