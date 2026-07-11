# PC-0007 Proposed Implementation Sequence

Status: Accepted execution sequence under master plan v1.8.

## Work-In-Progress Limit

Keep at most one engine/card batch, one semantic/integration batch, and one
product/research-interface batch active. No learned-model training is active.
The current bounded T3.3 mapper work may continue locally, but no next T3
integration commit lands until the dependency audit and ADR-0012 disposition
are complete.

## Sequence

| Order | Ticket/checkpoint | Deliverable | Entry condition |
| ---: | --- | --- | --- |
| 1 | Dependency/integrity remediation | Enforced 24-worker cap, translator/planner 1-vs-N evidence, boundary guard, offline deny report, T1.10/T1.11 addenda, exact-evidence ticket | Active v1.7 governance; separate ADR review |
| 2 | PC-0007 ratification | Exact-hash Owner approval, Gate Reviewer recommendation, master plan v1.8 and state update | Remediation evidence green |
| 3 | T3.3 current batch | Finish and land the existing PC-0006 recommended bounded mapper batch with before/after evidence | Current clean T3 baseline + ADR accepted |
| 3A | CP-STATUS-TRUTH | In parallel with T3.3: generated two-axis card maturity, truthful status page, renamed breadth proxies | PC-0007 ratified |
| 4 | T3.5-A | Generated runtime-smoke result schema and first capability synthesizers | Compiler-valid card path stable |
| 5 | T3.6-A | Select/freeze first 100 Commander semantic cards and author strata manifest | Owner priority list + rules sources |
| 6 | T3.9-A | Select four simple legal integration decks; produce complete-card blocker units | T3.6 selection overlaps deck cards |
| 7 | CP-CARD-SEMANTICS-100 | Card-specific behavior scenarios pass for the frozen 100-card set | T3.5 harness usable |
| 8 | CP-FOUR-PLAYER-POD | 1,000 deterministic four-seat card-driven games, hidden-info canaries, runtime metrics | Four decks compiler/runtime clean |
| 9 | T1.R10 | Interactive CLI legal-action/prompt loop, deck setup, baseline bot, complete replay | Four-player production path proven |
| 10 | CP-HUMAN-PLAY-CLI | Owner completes a real local game and replay | T1.R10 locally verified |
| 11 | T4 baseline | Random legal, deterministic heuristic, mulligan, threat/target, then bounded search and Tracks A/B/C skeleton | Human-play path stable |
| 12 | Focused Trainer | Desktop match/replay/review slices only | CLI interaction contract stable |
| 13 | CP-TRAINER-UI | Owner completes one prepared desktop Trainer game and post-game review | Focused Trainer locally verified; launch/review material ready |
| 14 | T4.H1 / CP-HUMAN-TRACE | Versioned trace schemas, replay/view validators, opt-in capture, private dataset manifest | CP-TRAINER-UI passed; V00 permits Owner-created private research data |
| 15 | CP-TEACHER-CORPUS-ALPHA | 20 Owner games, 500 validated choices, 100 reviewed states, four archetypes | CP-HUMAN-TRACE passed |
| 16 | Human-informed experiments | Action prior, preference reranker, search ordering, ablations and Tracks A/B/C | Approved V00 dataset row; CP-AI-BENCH, CP-HUMAN-TRACE, and CP-TEACHER-CORPUS-ALPHA passed; sealed replay-family splits |
| 17 | CP-PODBENCH-WORKER | Pinned promoted build emits versioned V04 report artifacts through the V03 contract | Applicable Forge gates; V00/V01/V02/V03/V04/V06; CP-BOUNDARY; SBOM, provenance, and conveyance evidence |
| 18 | S1 exposure decision | Separate Owner/professional launch decision; worker checkpoint alone is insufficient | CP-PODBENCH-WORKER plus all launch, policy, security, and rights gates |

## Parallel Business Lane

The separate PodBench lane may prepare V00 fact inventories, V04 mock reports,
CP-STATS schemas, fake-worker flows, and cost instrumentation during engine
work. It may not turn on real signup, external production sources, ads,
payments, public shares, or unapproved training.
