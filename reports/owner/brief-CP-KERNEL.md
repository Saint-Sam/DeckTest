# OWNER BRIEF - CP-KERNEL Checkpoint

Date: 2026-07-06

## 0. STATUS UPDATE

CP-KERNEL initially failed at commit `6491d5f`. The remediation tickets
T1.R1-T1.R4 were implemented and locally verified. A fresh Gate Reviewer then
performed the scoped re-review at commit `d7fcb03` and returned PASS. T1.8 is no
longer blocked by CP-KERNEL.

## 1. WHAT JUST HAPPENED

T1.7 state-based actions were implemented and pushed. GitHub Actions `ci #12`
run `28824832805` passed all required jobs on commit `6491d5f`. That triggers
the mandatory CP-KERNEL checkpoint from the master plan.

The checkpoint is specifically about the kernel API surface and Section 3.3
invariants. This is early enough to catch design drift before T1.8 mulligans,
testkit scenarios, oracle ports, CLI play, fuzzing, and benches build on top of
the kernel.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: open `reports/gates/CP-KERNEL/kernel-invariant-map-2026-07-06.md`
- EXPECT: it calls out the two main review risks plainly: the current public
  mutating helper surface, and temporary stored vanilla creature characteristics.
- RED FLAG: a CP-KERNEL sign-off that ignores those two issues.

- DO: open GitHub Actions for `Saint-Sam/Forge-2.0`.
- EXPECT: `ci #12` on commit `6491d5f` is green across fmt, clippy, Linux,
  macOS, Windows, WASM, Android, coverage, deny-audit, verification-loop, and
  determinism-replay.
- RED FLAG: any newer red run on `main` before CP-KERNEL is signed.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && source "$HOME/.cargo/env" && scripts/vl.sh`
- EXPECT: the command ends with `ALL CHECKS PASSED`.
- RED FLAG: any crash, panic, or verifier failure.

## 3. NUMBERS THAT MATTER

- `forge-core` has 60 unit tests after T1.7.
- T1.7 remote CI passed 11 required jobs.
- The CP-KERNEL evidence bundle is being assembled under
  `reports/gates/CP-KERNEL/bundle/`.
- Open blockers and P0/P1 questions: none.

## 4. KNOWN ROUGH EDGES

The main checkpoint risk is architectural, not CI health. The current T1 kernel
still exposes public mutating helpers for staged implementation and tests,
while the plan's long-term invariant wants external consumers to go through
`legal_actions` and `apply` plus read-only queries. The current vanilla combat
path also stores creature characteristics directly until the later CR 613 layer
system exists.

The Gate Reviewer may pass this as temporary scaffolding, pass with remediation
conditions, or fail the checkpoint and require repair tickets before T1.8.

## 5. WHAT YOU SHOULD EXPECT NEXT

A fresh Gate Reviewer agent will inspect the bundle and real code, then produce
`reports/gates/CP-KERNEL/SIGNOFF.md` with PASS, CONDITIONAL, or FAIL. If it
fails, I will convert each finding into remediation tickets and send you a
Trouble Bulletin within the plan's 24-hour window.

## 6. WHAT WE NEED FROM YOU

Nothing right now. CP-KERNEL does not require human approval under your O1
decision unless the reviewer recommends a plan change, de-scope, licensing/IP
decision, release decision, credits decision, or network egress.
