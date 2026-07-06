# OWNER BRIEF - T0 Foundations Gate

Date: 2026-07-06

## 1. WHAT JUST HAPPENED

T0 foundations are built locally: the Rust workspace, toolchain bootstrap,
scripts, CI workflow files, vendored rules text, legacy Forge submodule, and
legacy inventory report all exist. `INSTALL.md` now documents the one-command
fresh install path, and `scripts/bootstrap_toolchain.sh` supports both recursive
git clones and GitHub ZIP downloads. The local T0 gate script passes, and a
fresh recursive clone from commit `93cad32` passed `scripts/gates/gate_T0.sh`.
The Gate Reviewer did not sign off yet because the project still needs remote
GitHub CI evidence.

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

- DO: open `reports/gates/T0/SIGNOFF.md`
- EXPECT: it says `Verdict: FAIL`; this remains correct until remote GitHub CI
  evidence exists and the Gate Reviewer re-reviews.
- RED FLAG: any sign-off that claims T0 passed before GitHub CI is complete.

## 3. NUMBERS THAT MATTER

- 15 workspace crates compile and each has one bootstrap test.
- 33,290 legacy card scripts were inventoried.
- 43,649 legacy ability lines were counted.
- 251 distinct keyword rows were found in the legacy card scripts.
- Local target build smoke passed for WASM, Android, iOS, and Windows targets.

## 4. KNOWN ROUGH EDGES

There is no playable game yet; T0 is infrastructure only. Remote GitHub CI has
not run because the repository remote is not configured yet. The current local
Gate Reviewer verdict is intentionally a fail until that remote evidence exists.

## 5. WHAT YOU SHOULD EXPECT NEXT

The Orchestrator will configure or attach the GitHub repository so CI can run.
After that evidence gap is closed, the Gate Reviewer will re-review T0.

## 6. WHAT WE NEED FROM YOU

The exact GitHub destination for publishing this local repository: provide the
`owner/repo` or remote URL, or create an empty GitHub repository under
`Saint-Sam` and send its URL.
