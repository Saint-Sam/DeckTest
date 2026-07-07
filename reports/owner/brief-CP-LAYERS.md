# OWNER BRIEF - CP-LAYERS Checkpoint

Date: 2026-07-07

## 1. WHAT JUST HAPPENED

T2.4 implemented the first CR 613 continuous-effects layer engine: copy,
control, text, type, color, ability, and power/toughness sublayers 7a-7d. It is
remote-green on GitHub Actions. I also added a checkpoint-specific
`fuzz_characteristics` target so the reviewer can probe random mutation/query
interleavings before signing off.

This is not a pass yet. CP-LAYERS is the plan's human checkpoint before any
T2.5+ work can depend on layers.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo run -p forge-testkit -- oracle --path tests/oracle/reviewer_layers --no-junit`
- EXPECT: `oracle scenarios: 100 passed, 0 failed`.
- RED FLAG: any failed `tests/oracle/reviewer_layers/*.ron` file.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`
- EXPECT: `Finished` with no compile errors.
- RED FLAG: any Rust error in `fuzz_characteristics`.

- DO: open `reports/gates/CP-LAYERS/SIGNOFF.md`
- EXPECT: `Verdict: PENDING` and a checklist of the human review items.
- RED FLAG: this file says PASS before you decide what to do about the blocked
  100-card legacy engine differential.

## 3. NUMBERS THAT MATTER

- 80 original layer oracle scenarios pass.
- 100 owner-approved reviewer oracle scenarios pass.
- 5 layer-focused `forge-core` unit tests pass.
- Full VL passed after the reviewer pack: 482 oracle scenarios, 0 failures.
- Coverage after the reviewer pack: 81.82% lines; clone-surface baseline:
  `persistent_allocation_field_count=24`.
- Remote CI passed for T2.4: `ci #23` run `28891474213`; manual confirmation
  `ci #24` run `28892313697`; evidence commit `ci #25` run `28892638060`.

## 4. KNOWN ROUGH EDGES

The current layer engine is data-only and intentionally smaller than full Magic
card text. It has no real card compiler yet and no derived-characteristics
memoization cache. That absence is good for correctness right now because every
query recomputes, but performance/memoization will need fresh evidence later.

The true 100-card legacy engine differential is blocked locally. The vendored
legacy scripts are present and a 100-card layered subset is selected, but Forge
2.0 does not yet have a legacy card-script importer/card compiler capable of
executing those real cards in the new engine. The local Java legacy engine also
is not runnable as-is without JDK17/Maven/dependencies.

## 5. WHAT YOU SHOULD EXPECT NEXT

T2.5+ stays blocked. The next step is your CP-LAYERS decision on the blocked
legacy differential clause, then either remediation, explicit de-scope, or
fail/reopen.

## 6. CURRENT OWNER DECISIONS

You supplied the CP-LAYERS review direction on 2026-07-07:

- Codex interviewed you and turned your answers into 100 reviewer scenarios.
  You approved them with `approve 100 scenarios`, and all 100 pass locally.
- Legacy differential work starts with local-only search; Codex must ask before
  any network access, download, or install. Local-only search selected the
  100-card subset, but true execution is blocked.
- Longer sanitizer fuzz may run only if the tooling is already installed;
  otherwise Codex must ask before installing anything.
- CP-LAYERS is not approved yet. Codex must bring the results back before you
  decide proceed or fail.

## 7. WHAT WE NEED FROM YOU

You are the CP-LAYERS human reviewer under O1 Option C. To proceed, decide one:

- Remediate: build/import enough real card-script support or approve the
  toolchain/dependency install path needed for a true differential.
- De-scope: explicitly waive the true 100-card engine differential for this
  checkpoint and accept the 100 synthetic oracles plus static legacy subset
  evidence.
- Fail/reopen: fail CP-LAYERS and reopen layer/card-import remediation before
  T2.5.

T2.5 stays blocked until you choose and give the explicit CP-LAYERS signoff.
