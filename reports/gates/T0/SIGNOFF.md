# T0 Gate Signoff

## Re-review: 2026-07-06

Reviewer identity: designated strong reasoning model (`Anscombe`, delegated Gate
Reviewer)

Verdict: PASS CONDITIONAL

Commit SHA reviewed:
`3ee6166a6a969207088c46beda5e7c29a914ad48`
(`T0.3: record remote CI remediation evidence`)

Scope reviewed: failed-item remediation from the initial T0 gate review, current
T0 bundle, current `PLAN_STATE.json`, Owner Brief, fresh-clone gate evidence,
remote CI evidence, and branch-protection equivalence note.

Current tree note: this signoff supersedes the historical FAIL below. The
packet-only evidence corrections made during re-review are explicitly included
in this signoff commit so the gate record references one reviewed state.

## Conditions

1. Before collaboration branches or release, configure GitHub `main` branch
   protection to require the T0.3 CI jobs: `fmt`, `clippy`, `test-linux`,
   `test-macos`, `test-windows`, `build-wasm`, `build-android`, `coverage`,
   `deny-audit`, `verification-loop`, and `determinism-replay`.
2. The signoff/evidence commit contains no product-code change beyond gate
   paperwork. Its push must receive the same green GitHub Actions CI before the
   remote repository is considered fully synchronized with this local gate
   state.

## Re-review Checklist

| ID | Result | Evidence |
| --- | --- | --- |
| G1 Gate script green from scratch | PASS | Fresh recursive clone of `3ee6166a6a969207088c46beda5e7c29a914ad48` to `/private/tmp/forge_t0_fresh_3ee6166_20260706_1918` checked out legacy submodule `1f0a3e0815822d8f58f798e0304b33d4534248b1`; `bash scripts/gates/gate_T0.sh` ended `ALL CHECKS PASSED` and `PASS gate_T0.sh`. |
| G2 Exit criteria and metrics | PASS | `metrics/legacy_inventory.json` remains the metrics snapshot; T0.3 remote CI run `28816494698` on `3ee6166` passed all required jobs. |
| G3 Test-quality audit | PASS WITH LIMITATION | T0 still contains intentionally trivial bootstrap tests, which match the T0.2 empty-workspace requirement. Meaningful mutation checking begins when nontrivial engine behavior exists. |
| G4 Adversarial oracle scenarios | N/A FOR T0 | No rules engine or oracle subject exists in T0. |
| G5 Blocker and quarantine hygiene | PASS | Bundle reports no blockers; quarantine is not active until later tiers. |
| G6 Determinism and fuzz | N/A FOR T0 | Determinism hook exists and passes/skips as expected; no replay/fuzz subject exists yet. |
| G7 Spot play / replays | N/A FOR T0 | No playable game or replay format exists in T0. |
| G8 ADR/spec consistency | PASS | Legacy submodule pin, vendored CR metadata, toolchain lock, and workspace/CI layout remain consistent with T0 plan requirements. |
| G9 Question Queue clear | PASS | `questions_open.md` reports no open P0/P1 questions. |
| G10 Scope integrity | PASS | Changes are confined to T0 foundation, CI, gate evidence, status, and owner-report surfaces. |
| G11 Owner Brief delivered | PASS | `reports/owner/brief-T0-gate.md` exists, includes the T0 `scripts/vl.sh` try-it row, known rough edges, and required owner-facing closing sections. |

## Remediation Closure

- T0.R1: COMPLETE. Remote GitHub Actions run `28816494698` passed the required
  T0.3 jobs on `3ee6166`; branch-protection equivalence is documented, with
  real branch protection left as a future owner-controlled repo setting before
  collaboration branches or release.
- T0.R2: COMPLETE. T0 gate Owner Brief exists and no longer repeats stale Rust
  installation claims.
- T0.R3: COMPLETE. Bundle was regenerated from current `PLAN_STATE.json`.
- T0.R4: COMPLETE. Fresh recursive clone gate run passed for current committed
  head `3ee6166`.

Blockers forcing FAIL: none.

## Historical Review: Initial Fail Superseded

Date: 2026-07-06

Reviewer identity: designated strong reasoning model

Verdict: FAIL

Reviewed tree state: no commit exists yet. `git status --short --branch` reported `## No commits yet on main`; staged `.gitmodules` and `vendor/legacy-forge`; untracked `.github/`, `.gitignore`, `Cargo.lock`, `Cargo.toml`, `FORGE_REBUILD_MASTER_PLAN.md`, `LICENSE`, `PLAN_STATE.json`, `README.md`, `crates/`, `docs/`, `metrics/`, `reports/`, `rust-toolchain.toml`, `scripts/`, and `tools/`.

Scope reviewed: plan sections requested in the invocation; evidence in `reports/gates/T0/bundle/`; T0 files needed to verify the claims.

## Checklist

| ID | Result | Evidence |
| --- | --- | --- |
| G1 Gate script green from scratch | FAIL | Current bundle `test_log.txt` ends `ALL CHECKS PASSED` and `PASS gate_T0.sh` from a refreshed local `scripts/gates/gate_T0.sh` run. This proves the local gate is green. It does not satisfy the required fresh-clone method because the repository still has no commit to clone and review. |
| G2 Exit criteria and metrics | FAIL | `metrics/legacy_inventory.json` and `docs/legacy_inventory.md` reproduce exactly from `tools/mine_legacy.py` against the pinned legacy submodule: JSON sha256 `27ccc296c777961877638997bb34dabcc1bebf3f8497507292d95d5d6b44def5`; Markdown sha256 `cd3af5835f5f4192ff500681515205457e228f76827d5de2c875755507261dfa`. Counts spot-checked: 33,290 scripts, 43,649 ability lines, top API `T: ChangesZone` count 7,397, 40 API rows rendered. However the T0 exit criterion "CI green" is unmet because no remote GitHub CI has run. |
| G3 Test-quality audit | PASS WITH LIMITATION | `tests_added.txt` lists one bootstrap test per workspace crate. These are trivial but match T0.2's explicit "one trivial test" requirement for empty crates. Mutation checking was not meaningful because there is no base commit/ref. |
| G4 Adversarial oracle scenarios | N/A FOR T0 | T0 has no rules engine or oracle subject matter. No CR-derived runtime scenarios exist yet. This should become active in T1+. |
| G5 Blocker and quarantine hygiene | PASS | Bundle reports no blockers. Quarantine is not active until T3. |
| G6 Determinism and fuzz | N/A FOR T0 | Bundle states no fuzz targets exist in T0. `scripts/review/determinism.sh` is present but skips until replays exist. This should become active when replay/game artifacts exist. |
| G7 Spot play / replays | N/A FOR T0 | No playable game or replay format exists in T0. |
| G8 ADR/spec consistency | PASS | ADR-0002 records legacy pin `1f0a3e0815822d8f58f798e0304b33d4534248b1`, matching `.gitmodules`, submodule HEAD, and git index mode `160000`. ADR-0003 records the rules URL and June 19, 2026 effective date, matching `docs/vendor/comprehensive-rules.txt`. Workspace layout and CI workflow are present. Current `docs/toolchain.lock.md` records versions for `cargo llvm-cov`, `cargo fuzz`, `cargo deny`, `cargo audit`, `wasm-bindgen`, `cargo ndk`, and `critcmp`. |
| G9 Question Queue clear | PASS | `questions_open.md` says no open questions. |
| G10 Scope integrity | PASS WITH LIMITATION | Files inspected are T0 foundation files. There is no commit log to sample because there is no commit yet. Root `PLAN_STATE.json` marks T0.3 `implementation_complete_pending_remote_ci`; bundle `PLAN_STATE.json` is stale and still marks T0.1/T0.2 blocked. |
| G11 Owner Brief delivered | FAIL | `reports/owner/` contains `brief-T0-start.md` and preflight artifacts only. No T0 gate Owner Brief exists. The start brief is stale and still says Rust is missing and asks the Owner to complete `reports/t0/T0.1-install-request.md`. |

## Supplemental CI Assessment

`reports/gates/T0/bundle/cross_target_build_smoke.md` records local successful release builds for `wasm32-unknown-unknown`, `aarch64-linux-android`, `aarch64-apple-ios`, and `x86_64-pc-windows-msvc` bootstrap crates. This is useful risk-reduction evidence, but it does not replace remote GitHub Actions evidence required by the T0 exit gate and T0.3 DoD. The absence of remote CI is material.

## Required Remediation

1. T0.R1: Configure the repository remote and run the GitHub Actions workflow on a scratch PR or equivalent remote CI run. Record evidence that required T0.3 jobs are green, including fmt, clippy, linux/mac/windows tests, wasm build, Android build, coverage, deny-audit, verification-loop, and determinism hook. Configure branch protection or record why an equivalent control is used.
2. T0.R2: Create and deliver the T0 gate Owner Brief under `reports/owner/`. It must follow the section 17.2 template, include the T0 section 17.4 `scripts/vl.sh` TRY-IT row, include current known rough edges, and not repeat stale T0-start claims that Rust is missing.
3. T0.R3: Regenerate the gate bundle after current `PLAN_STATE.json` is final for review. The bundle snapshot must not contradict the root plan state.
4. T0.R4: After the first commit exists, rerun the T0 gate from a true fresh clone or document an equivalent clean-run procedure accepted by the Gate Reviewer.

T0 may be re-requested for gate review after these items are addressed. The remote CI gap may not be waived by local cross-target smoke alone.
