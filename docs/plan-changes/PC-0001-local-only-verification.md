# PC-0001: Local-only verification

Date: 2026-07-09

Status: Accepted and incorporated in Master Plan v1.3.

## Motivation

The Owner has directed that Forge 2.0 use no GitHub Actions. The repository's
push, pull-request, and scheduled workflows would otherwise keep spending the
Owner's monthly Actions budget. Verification must use the Owner's local Mac and
local virtual machines, simulators, browsers, and devices.

## Exact Plan Change

1. Replace references to mandatory GitHub Actions or remote-green commits with
   an exact-commit local verification packet.
2. Keep the Verification Loop and all tier requirements intact; only the runner
   changes.
3. Require `scripts/local_verify.sh` to run the native quality suite and target
   builds with a resource-aware worker count.
4. Require local platform evidence before release: macOS native, Linux VM or
   container, Windows VM, WASM in a local browser, Android emulator or device,
   and iOS simulator or device.
5. Archive workflow definitions outside `.github/workflows/` so no GitHub event
   can execute them.
6. Replace hosted cron checks with explicit local campaigns. Results remain
   committed under `reports/`; a scheduled local runner may be added only with
   Owner approval for any required installation.
7. A gate packet is valid only for the reviewed commit and contains its SHA, a
   detached fresh worktree, isolated target directories, full command logs,
   toolchain versions, platform-matrix results, and artifact hashes. Integration
   and signoff are blocked unless that exact-commit packet succeeds.
8. Installing a launch agent, service, cron entry, VM, simulator, SDK, or other
   background runner remains an Owner-approved installation/network action.

## Affected Tasks

T0.3, all T3-T8 gates, recurring checks, gate evidence bundles, acceptance_v1,
and release platform verification.

## Risks And Mitigations

- Risk: one host can conceal platform-specific failures.
  Mitigation: the required local VM/simulator/device matrix is release-blocking.
- Risk: local runs are easier to contaminate with incremental state.
  Mitigation: gates use an exact-commit detached worktree and isolated target
  directories, with command logs and hashes in the evidence bundle.
- Risk: scheduled coverage is lost.
  Mitigation: tier gates remain mandatory, and long fuzz campaigns run in
  parallel locally using bounded shared corpora.

## Approval

Owner approval was given in the Codex thread on 2026-07-09: no GitHub Actions;
run all verification locally and use available local hardware.

Gate Reviewer recommendation: RECOMMENDED on 2026-07-09 after requiring exact
local evidence and explicit approval for installed schedulers. The final text
incorporates those requirements.
