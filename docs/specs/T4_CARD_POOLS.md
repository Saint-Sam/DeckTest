# T4 Card Pools

Status: Wave 0 regression-pod freeze recorded. This document does not pass a
T4 gate, CP-AI-REALISTIC-POD, or CP-AI-BENCH.

## Regression Pod v1

`assets/ai/pods/regression-v1.json` is the immutable, source-bound name for the
existing four-deck integration environment. Its canonical source remains
`assets/t3_9/integration_decks.json`; the source file is deliberately not
changed by the card-pool lane.

The pod is a deterministic engineering regression fixture only. It is
authoritative for exact replay, deterministic search regression, canonical
decision-surface regression, hidden-information canaries, stack/trigger/cost/
combat regression, before-and-after performance comparisons, and quick local
preflight. It is not representative of normal Commander game length or deck
construction, and it must not be used alone for AI strength, archetype
calibration, search-knee selection, product compute estimates, or product
promotion.

### Frozen taxonomy

The source has four decks with 99 mainboard slots each and one separate
commander entry per deck:

| Measure | Frozen value |
| --- | ---: |
| Decks | 4 |
| Mainboard slots | 396 |
| Commander slots | 4 |
| Total deck slots | 400 |
| Unique mainboard identities | 21 |
| Unique commander identities | 4 |
| Distinct identities including commanders | 25 |

The 21-identity figure used by the T3.9/T4 regression language is the unique
mainboard identity count. Commander paths live in the source `commander` field
and are counted separately here so the fixture does not blur semantic breadth
with deck size.

The mainboard taxonomy is:

| Family | Unique identities | Engineering role |
| --- | ---: | --- |
| Mana, ramp, and equipment | 8 | Land development, mana production, protection, and combat development |
| Lands and fixing | 9 | Basic mana, fixing, and utility-land paths |
| Card advantage and tax | 2 | Draw and opponent-resource pressure |
| Finishers and payoffs | 2 | Large-creature and damage-payoff paths |

The deck role labels are intentionally broad engineering labels, not realistic
benchmark archetype claims: Isamaru is proactive combat pressure, Tobias is
durable material and simple value, Yargle is large-creature combat pressure,
and Rorix is haste and combat pressure. The pool is highly overlapping and
deliberately narrow; that narrowness is part of the regression surface.

## Provenance and exact identity

The frozen fixture records all of the following bindings:

| Binding | Value |
| --- | --- |
| Product commit | `19ef3302c40db3e916d2a60925546d4ebc28608d` |
| Product tree | `e79efa91e0146f23f7219367e117db34ce13867a` |
| Source manifest | `assets/t3_9/integration_decks.json` |
| Source SHA-256 | `0ed6260e37d1f62ad3d5463bbe9235730a31860d2c1c69c4b69f0735979c40c1` |
| Source Git blob SHA-1 | `f6ab74fe7fcc5befdb6f158ea065bf68bf9e9e41` |
| Translated-card root | `target/translated-cards` |
| Semantic registry | `metrics/card_semantics_100.json` |

The fixture contains an ordered snapshot of the source `decks` array so the
source deck IDs, commander paths, card paths, and counts remain inspectable.
The source manifest is still authoritative. The duplicate snapshot is an
integrity witness, not permission to edit the source or to substitute a newer
manifest under the v1 name.

The prior deterministic four-player and exact typed-action replay result is
referenced by `reports/gates/T3.9/cp-four-player-pod-2026-07-13.json`. That
artifact is historical evidence from an earlier product binding and is not
treated as current-head evidence by this freeze. New replay evidence must bind
its own product commit, tree, and artifact hashes.

## Reproducibility contract

The fixture defines no random seed or benchmark schedule. A caller must record
the explicit game or replay seed, policy, runtime configuration, and fixture
version. Reproduction requires the pinned product commit/tree, the exact source
manifest hashes above, the referenced translated-card root and semantic
registry, and the exact v1 fixture bytes. JSON array order is preserved as part
of the source projection.

This fixture is suitable for matched before/after engineering comparisons when
all of those bindings are held fixed. It is not a development, validation, or
sealed realistic benchmark split, and measurements from it do not become
strength labels merely because they are repeatable.

## Fail-closed drift detection

Any consumer of the fixture must reject it before loading a game when any of
these checks fails:

1. Resolve the source path as a repository-relative path and reject missing,
   absolute, or path-traversal paths.
2. Recompute the source SHA-256 and Git blob SHA-1 and compare both recorded
   values.
3. Compare `schema_version`, `source_root`, `semantic_registry`, and the
   ordered `decks` array with the source-bound values in the fixture.
4. Compare every deck ID, commander, card path, and count, including the 396
   mainboard slots and 400 total deck slots.
5. Require the recorded product commit and tree to match the product under
   test.
6. Require the gate manifest to report the same source hash and fixture hash,
   and require all integrity checks to be true.

There is no fallback to a current source file after a mismatch, no partial
deck loading, and no silent repair. A mismatch makes the v1 fixture stale and
unusable for regression, replay, or evidence generation. A semantic change
must create `regression-v2.json` and a new gate manifest; v1 is never edited in
place.

### Executable consumer and local gate

The reusable fail-closed consumer is
`tools/verify_t4_regression_pod.py`. Its `load_regression_pod` entry point
validates the source, fixture, paths, product binding, taxonomy, and gate
manifest before returning fixture data. Its CLI accepts explicit
`--product-commit` and `--product-tree` arguments; it never infers the product
from the current `git HEAD`. Evidence commits made after the runtime product
therefore do not change the product identity being checked.

The local lane gate is `scripts/gates/gate_T4_cards.sh`. It runs the validator
against regression-v1 and runs
`tools/build_t4_card_admission.py --check` with the same exact product
arguments. The gate is diagnostic and remains blocked by design: a successful
local run proves only integrity and reproducibility of the engineering pod.
It must keep `promotion_eligible` false and
`cp_ai_realistic_pod_passed` false; it cannot pass or promote
`CP-AI-REALISTIC-POD`, `CP-AI-BENCH`, or any runtime/T4 checkpoint.

## Boundary with realistic pools

Realistic and benchmark pods must use separate versioned manifests and separate
evidence. They may be compared against this fixture for game length, action
surface, or resource diagnostics, but they must not inherit its engineering
classification or use it as a proxy for realistic deck breadth. In particular,
the narrow pool must not be expanded, rebalanced, or card-substituted to make
its game lengths look more representative.

## Realistic Pod v1 candidate lane

`assets/ai/pods/realistic-pod-v1-candidates.json` is the separate Wave 1
candidate environment. It is bound to runtime product
`19ef3302c40db3e916d2a60925546d4ebc28608d` and tree
`e79efa91e0146f23f7219367e117db34ce13867a`. It does not modify or replace
Regression Pod v1.

The candidate packet contains four exact Commander lists:

| Deck | Role | Mainboard | Lands | Nonlands |
| --- | --- | ---: | ---: | ---: |
| Krenko, Mob Boss | Proactive Goblin pressure | 99 | 35 | 64 |
| Zaxara, the Exemplary | Sultai ramp and bounded X-spell value | 99 | 30 | 69 |
| Tobias Andrion | Azorius control and inevitability | 99 | 33 | 66 |
| Judith, the Scourge Diva | Rakdos sacrifice engine | 99 | 38 | 61 |

The four decks use 282 unique identities across 400 slots. The earlier shared
294-name draft was not treated as four decklists. Twenty-seven off-color draft
identities and two low-fit blue identities were removed. Seventeen legal cards
were added to make the mono-red Goblin and Sultai X-spell plans functional.
This is deck-driven scope expansion, not a renewed global mapper campaign.

Each mainboard records exact counts, catalog identity, Oracle identity, color
identity, type line, and the Basic Land singleton exception. PilotIntent v0.2
records only cards that are actually present in its deck. Zaxara is explicitly
black, green, and blue, and its list includes a bounded X-spell package rather
than borrowing off-color tutor defaults.

The packet remains `not_frozen`, `blocked_candidate`, and promotion-ineligible.
No listed card is benchmark-admitted from deck inclusion. The machine-readable
inventory and blocker reports are:

```text
reports/gates/T4-CARDS/realistic-pod-v1-inventory.json
reports/gates/T4-CARDS/realistic-pod-v1-blockers.json
metrics/cards/t4_benchmark_admission.json
reports/gates/T4-CARDS/ADMISSION.json
```

`tools/build_t4_card_admission.py` validates the exact four-deck membership,
Commander color legality, singleton rules, catalog IDs, PilotIntent references,
input hashes, and per-identity blocker arithmetic before producing admission
output. Missing, stale, malformed, or unauthenticated evidence remains blocked.
The focused adversarial contracts live in
`tests/t4/test_realistic_pod_candidates.py` and
`tests/t4/test_card_admission.py`.

The next card-pool step is to build exact current-product evidence adapters and
then address the highest-fan-out runtime, semantic, prompt, and replay blockers.
The candidate manifest may be frozen only after all four decks are admission
clean. The 100-game campaign and `CP-AI-REALISTIC-POD` remain unpassed.
