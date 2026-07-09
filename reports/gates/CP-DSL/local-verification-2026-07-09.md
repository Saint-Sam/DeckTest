# CP-DSL Local Verification

Date: 2026-07-09

Mode: local-only under PC-0001. No GitHub Actions, push, network access, or new
installation was used. The reviewed implementation is the commit containing
this report; the detached-worktree log records its exact commit id.

## Remediation Result

The first Gate Reviewer pass failed with four P1 findings. All four were
remediated before the exact-commit re-review:

1. Every operation now declares recursively enforced argument types. Bare
   symbols, prose in effect positions, and category-correct but argument-wrong
   trees fail closed.
2. The ten nightmare classes now load a separate versioned database of
   compiled layer scenarios. Arena effects are lowered only from validated
   `layer_effect` trees; fixture code contains setup and expectations, not an
   independently executable effect list.
3. The 25 mandatory mechanics strata are a closed exact set with four cards
   each. Catalog-only identities are enforced separately over the full import.
   A normal-layout token-set record found by review is now catalog-only.
4. Platform evidence executes four real isolated cross-target builds, card
   regression executes all 1,200 semantic packs, and deterministic evidence
   uses three clean isolated compiler target directories.

The repository also now includes the full GPL-3.0 license text, permits only
the GPL-compatible dependency licenses actually encountered by `cargo-deny`,
and supports an offline bootstrap check from a GitHub source ZIP without the
git submodule.

## Results

| Check | Result |
| --- | --- |
| English printing import | 113,234 / 113,234 |
| Classified identities | 38,306 / 38,306 |
| Catalog-only identities | 3,614 |
| Dangling printing references | 0 |
| CP-DSL definitions | 100 |
| Mandatory mechanics strata | exact 25 / 25, four cards each |
| Typed operations represented | 127 |
| Canonical parse/emit/reparse | 100 / 100 |
| Positioned malformed diagnostics | 64 / 64 |
| Main database clean builds | 3 / 3 byte-identical |
| Main database SHA-256 | `2840834d0dd5a8b558af7587569d5f0171d4ee0545eccdb8a3a2de750c402381` |
| Layer scenario database clean builds | 3 / 3 byte-identical |
| Layer scenario SHA-256 | `3b6906e2e29da0b5c6cd10f4e4cc5da87cb58866b5391ce9c22836296d840364` |
| Curated compiler/loader mutants | 28 killed, 0 survived, 0 invalid |
| Address-sanitizer fuzz | 8 workers x 300 seconds = 2,400 worker-seconds |
| Fuzz target breadth | all 5 targets represented and clean |
| Local cross-target checks | WASM, Android, iOS, Windows: 4 / 4 executed |
| Compiled nightmare suite | 100 games, 10 classes, 0 invariant violations |
| Semantic oracle execution | 1,200 passed, 0 failed |
| Oracle structural breadth | 379 scalar-collapsed families, 1,839 interactions |
| Workspace line coverage | 80.08%, floor 80% |
| Dependency licenses/bans/sources | `cargo deny` passed offline |
| GitHub ZIP bootstrap simulation | passed offline without submodule contents |
| Hosted workflow files | 0 |

## Evidence Files

- `metrics/cp_dsl_verification.json`
- `metrics/cp_dsl_mutation.json`
- `metrics/local_fuzz.json`
- `metrics/local_platforms.json`
- `metrics/oracle_semantics.json`
- `metrics/card_catalog.json`
- `metrics/coverage.json`

## Scope Boundary

This checkpoint proves the identity/catalog model, typed source language,
compiler and loader boundaries, deterministic databases, complete catalog
classification, and compiled layer integration path. It does not claim that
all 38,306 identities already have complete mechanics. Mass translation and
general lowering of the remaining operation registry are later T3 tasks.
