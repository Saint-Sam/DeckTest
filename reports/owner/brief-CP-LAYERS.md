# OWNER BRIEF - CP-LAYERS Checkpoint

Date: 2026-07-07

## 1. WHAT JUST HAPPENED

T2.4 implemented the first CR 613 continuous-effects layer engine: copy,
control, text, type, color, ability, and power/toughness sublayers 7a-7d. It is
remote-green on GitHub Actions. I also added a checkpoint-specific
`fuzz_characteristics` target so the reviewer can probe random mutation/query
interleavings before signing off.

CP-LAYERS is now signed off. It was the plan's human checkpoint before any
T2.5+ work could depend on layers.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo run -p forge-testkit -- oracle --path tests/oracle/reviewer_layers --no-junit`
- EXPECT: `oracle scenarios: 100 passed, 0 failed`.
- RED FLAG: any failed `tests/oracle/reviewer_layers/*.ron` file.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`
- EXPECT: `Finished` with no compile errors.
- RED FLAG: any Rust error in `fuzz_characteristics`.

- DO: open `reports/gates/CP-LAYERS/SIGNOFF.md`
- EXPECT: `Verdict: PASS` and your 2026-07-08 owner signoff sentence.
- RED FLAG: missing `ci #33` evidence for commit `a3d16af`.

## 3. NUMBERS THAT MATTER

- 80 original layer oracle scenarios pass.
- 100 owner-approved reviewer oracle scenarios pass.
- 100 selected legacy Forge layered-card snapshots now execute locally: 100 OK,
  0 legacy harness errors.
- 53 generated Forge 2.0 legacy-script fragment scenarios pass. They remain as
  supplemental executable coverage.
- The true importer differential now passes for the selected 100-card subset:
  100/100 exact stable-role snapshots, 0 mismatches, 186 imported layer
  operations from 117 active-face continuous lines.
- 5 layer-focused `forge-core` unit tests pass.
- Full VL passed after the legacy bridge: 535 oracle scenarios, 0 failures.
- Coverage after the reviewer pack: 81.82% lines; clone-surface baseline:
  `persistent_allocation_field_count=24`.
- Remote CI passed for T2.4: `ci #23` run `28891474213`; manual confirmation
  `ci #24` run `28892313697`; evidence commit `ci #25` run `28892638060`.

## 4. KNOWN ROUGH EDGES

The current layer engine is data-only and intentionally smaller than full Magic
card text. It has no real card compiler yet and no derived-characteristics
memoization cache. That absence is good for correctness right now because every
query recomputes, but performance/memoization will need fresh evidence later.

The legacy side of the 100-card differential is no longer blocked: Codex used
repo-local Corretto 17 and Maven artifacts to run the vendored legacy Java
engine over all 100 selected cards. Codex also added a local true importer
differential for this CP-LAYERS fixture. It parses the active legacy card face,
recreates the same three-object fixture with stable object roles, applies
layer-ordered continuous effects, and matches the Java snapshots for all 100
selected cards.

## 5. WHAT YOU SHOULD EXPECT NEXT

T2.5 may now start. The previous true differential blocker has been remediated
locally and the evidence commit is remote-green.

## 6. CURRENT OWNER DECISIONS

You supplied the CP-LAYERS review direction on 2026-07-07:

- Codex interviewed you and turned your answers into 100 reviewer scenarios.
  You approved them with `approve 100 scenarios`, and all 100 pass locally.
- Legacy differential work started with local-only search; Codex asked before
  network/download/toolchain setup. Local-only search selected the 100-card
  subset, the legacy Java engine emits 100/100 OK snapshots, the supplemental
  bridge emits 53/53 passing real-script fragment scenarios, and the true
  importer differential now matches 100/100 selected legacy snapshots.
- Longer sanitizer fuzz may run only if the tooling is already installed;
  otherwise Codex must ask before installing anything.
- CP-LAYERS was approved by you on 2026-07-08 with:
  `CP-LAYERS signoff: proceed. I approve CP-LAYERS based on the 100
  owner-approved reviewer scenarios passing, the 100-card true importer legacy
  differential matching 100/100 with 0 mismatches, fuzz/local verification
  passing, and no current blocking divergences.`

## 7. WHAT WE NEED FROM YOU

No owner action is needed for CP-LAYERS. T2.5 is open.
