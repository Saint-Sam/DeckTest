# OWNER BRIEF - T0 Foundations Gate

Date: 2026-07-06

## 1. WHAT JUST HAPPENED

T0 foundations are built locally: the Rust workspace, toolchain bootstrap,
scripts, CI workflow files, vendored rules text, legacy Forge submodule, and
legacy inventory report all exist. `INSTALL.md` now documents the one-command
fresh install path, and `scripts/bootstrap_toolchain.sh` supports both recursive
git clones and GitHub ZIP downloads. The local T0 gate script passes, and a
fresh recursive clone from commit `93cad32` passed `scripts/gates/gate_T0.sh`.
Remote GitHub CI run `ci #3` also passed on commit `b9f13fa`.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && source "$HOME/.cargo/env" && scripts/vl.sh`
- EXPECT: the command ends with `ALL CHECKS PASSED`.
- RED FLAG: any red error, `command not found`, or missing script. Reply with
  the output; an agent investigates within 24 hours.

- DO: open `docs/legacy_inventory.md`
- EXPECT: it reports 33,290 legacy card scripts and a top-40 ability API table.
- RED FLAG: missing file, zero counts, or no top API table.

- DO: open `reports/gates/T0/fresh-clone-2026-07-06.md`
- EXPECT: it records a passing fresh-clone T0 gate from commit `93cad32`.
- RED FLAG: any sign-off that claims T0 passed before GitHub CI and a
  fresh-clone run are complete.

- DO: open `reports/gates/T0/remote-ci-2026-07-06.md`
- EXPECT: it records `ci #3` passing on commit `b9f13fa` with fmt, clippy,
  Linux/macOS/Windows tests, WASM, Android, coverage, deny-audit,
  verification-loop, and determinism-replay all green.
- RED FLAG: any later CI run on `main` failing before T0 Gate re-review.

- DO: open `reports/gates/T0/SIGNOFF.md`
- EXPECT: it still contains the prior Gate Reviewer fail, plus the remediation
  evidence files now exist for re-review.
- RED FLAG: any plan state claiming T0 passed before a new Gate Reviewer
  sign-off.

## 3. NUMBERS THAT MATTER

- 15 workspace crates compile and each has one bootstrap test.
- 33,290 legacy card scripts were inventoried.
- 43,649 legacy ability lines were counted.
- 251 distinct keyword rows were found in the legacy card scripts.
- Local target build smoke passed for WASM, Android, iOS, and Windows targets.
- Remote CI run `28815618762` passed all T0.3 jobs.

## 4. KNOWN ROUGH EDGES

There is no playable game yet; T0 is infrastructure only. Branch protection has
not been changed automatically because that is a GitHub repo-settings change.
Before collaboration branches or release, configure `main` protection to require
the T0.3 CI jobs. The current Gate Reviewer verdict remains the prior fail until
the re-review is complete.

## 5. WHAT YOU SHOULD EXPECT NEXT

The Gate Reviewer will re-review T0 using the refreshed bundle and the green
remote CI evidence.

## 6. WHAT WE NEED FROM YOU

No immediate input unless the Gate Reviewer requires branch protection before
sign-off. If that happens, the requested setting will be explicit and narrow:
protect `main` and require the T0.3 CI jobs.
