# CP-DSL Local Verification

Date: 2026-07-10

Mode: local-only under PC-0001. No GitHub Actions, push, network access, or new
installation was used. The detached packet records the exact reviewed commit
and tree hash.

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

A second Gate Reviewer pass found two additional P1 evidence gaps. The
malformed corpus now contains 59 explicitly tagged recursive-argument cases
covering every `ArgumentKind`, depths 1-4, variadic positions, bare symbols,
and prose misuse. Mutation and fuzz results now require a passing control,
expected killing tests, actual libFuzzer completion statistics, retained full
logs, hashes, timestamps, commands, toolchains, and isolated target paths.

The first exact rerun then found a real type-line round-trip crash after about
three million `fuzz_carddsl` executions. Ambiguous repeated delimiters and
control-character subtype tokens now fail closed, and the exact crash input is
retained in `fuzz/corpus/fuzz_carddsl/` as a permanent regression seed.

A third Gate Reviewer rejected metadata-only cross-target checks. The platform
lanes now perform clean platform-package builds and fail unless a linked WASM,
Android, iOS, or Windows artifact exists and its full log, size, magic, and
SHA-256 validate.

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
| Positioned malformed diagnostics | 117 / 117 |
| Tagged recursive-argument diagnostics | 59, every ArgumentKind represented |
| Main database clean builds | 3 / 3 byte-identical |
| Main database SHA-256 | `0dd65e72305bbc40c8da6037acc7eb2806dcc506104afd5415e5df37a667e0e5` |
| Layer scenario database clean builds | 3 / 3 byte-identical |
| Layer scenario SHA-256 | `cd1e1ce66dd6e46b958f929f6ee5ea339a4f88c976c0ba874ba813fe715c794e` |
| Curated compiler/loader mutants | passing control; 28 killed by expected tests, 0 survived, 0 invalid |
| Address-sanitizer fuzz | 8 workers, 2,408 verified worker-seconds |
| Fuzz target breadth | all 5 targets represented and clean |
| Linked cross-target builds | WASM module plus Android, iOS, and Windows static libraries: 4 / 4 built, logged, and hashed |
| Compiled nightmare suite | 100 games, 10 classes, 0 invariant violations |
| Semantic oracle execution | 1,200 passed, 0 failed |
| Oracle structural breadth | 379 scalar-collapsed families, 1,839 interactions |
| Workspace line coverage | 17,797 / 22,161 lines, 80.3077% |
| Dependency licenses/bans/sources | `cargo deny` passed offline |
| GitHub ZIP bootstrap simulation | passed offline without submodule contents |
| Hosted workflow files | 0 |
| Exact local evidence packet | PASS at `af6c8508030aed0bc56c71eac61b398f9e00ec4f`; final reviewer PASS |

## Evidence Files

- `metrics/cp_dsl_verification.json`
- `metrics/cp_dsl_mutation.json`
- `metrics/local_fuzz.json`
- `metrics/local_platforms.json`
- `metrics/oracle_semantics.json`
- `metrics/card_catalog.json`
- `metrics/coverage.json`
- `reports/gates/CP-DSL/evidence/packet.json`
- `reports/gates/CP-DSL/evidence/commands/`
- `reports/gates/CP-DSL/evidence/mutation/`
- `reports/gates/CP-DSL/evidence/fuzz/`

## Scope Boundary

This checkpoint proves the identity/catalog model, typed source language,
compiler and loader boundaries, deterministic databases, complete catalog
classification, and compiled layer integration path. It does not claim that
all 38,306 identities already have complete mechanics. Mass translation and
general lowering of the remaining operation registry are later T3 tasks.
