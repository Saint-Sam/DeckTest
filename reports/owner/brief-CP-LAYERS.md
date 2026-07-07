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

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo run -p forge-testkit -- oracle --path tests/oracle/layers --no-junit`
- EXPECT: `oracle scenarios: 80 passed, 0 failed`.
- RED FLAG: any failed `tests/oracle/layers/*.ron` file.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`
- EXPECT: `Finished` with no compile errors.
- RED FLAG: any Rust error in `fuzz_characteristics`.

- DO: open `reports/gates/CP-LAYERS/SIGNOFF.md`
- EXPECT: `Verdict: PENDING` and a checklist of the human review items.
- RED FLAG: this file says PASS before the 15 novel scenarios and legacy
  differential are complete.

## 3. NUMBERS THAT MATTER

- 80 layer oracle scenarios pass.
- 5 layer-focused `forge-core` unit tests pass.
- Full VL passed during T2.4 prep: 382 oracle scenarios, 0 failures.
- Coverage during T2.4 prep: 81.65% lines; clone-surface baseline:
  `persistent_allocation_field_count=24`.
- Remote CI passed for T2.4: `ci #23` run `28891474213`; manual confirmation
  `ci #24` run `28892313697`; evidence commit `ci #25` run `28892638060`.

## 4. KNOWN ROUGH EDGES

The current layer engine is data-only and intentionally smaller than full Magic
card text. It has no real card compiler yet and no derived-characteristics
memoization cache. That absence is good for correctness right now because every
query recomputes, but performance/memoization will need fresh evidence later.

The required legacy differential has not been performed yet. That is a real
checkpoint item, not a paperwork formality.

## 5. WHAT YOU SHOULD EXPECT NEXT

T2.5+ stays blocked. The next step is CP-LAYERS human review: 15 novel layer
scenarios, a 100-card legacy layered differential with written adjudication,
memoization/invalidation acceptance, and the explicit signoff sentence.

## 6. CURRENT OWNER DECISIONS

You supplied the CP-LAYERS review direction on 2026-07-07:

- Codex should interview you and turn your answers into 15 reviewer scenarios
  for your approval.
- Legacy differential work starts with local-only search; Codex must ask before
  any network access or download.
- Longer sanitizer fuzz may run only if the tooling is already installed;
  otherwise Codex must ask before installing anything.
- CP-LAYERS is not approved yet. Codex must bring the results back before you
  decide proceed or fail.

## 7. WHAT WE NEED FROM YOU

You are the CP-LAYERS human reviewer under O1 Option C. To proceed, review the
packet and answer the scenario interview in
`reports/gates/CP-LAYERS/scenario-interview-2026-07-07.md`. I will turn your
answers into 15 reviewer scenarios for your approval, run the approved cases,
and record the results. T2.5 stays blocked until CP-LAYERS is signed off.
