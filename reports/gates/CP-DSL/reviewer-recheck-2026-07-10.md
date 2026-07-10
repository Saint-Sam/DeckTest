# CP-DSL Independent Re-review

Date: 2026-07-10

Reviewed commit: `0a53df82ba9e06791e1f0844811d25de718a21c9`

Reviewer: `gpt-5.6-luna`, xhigh reasoning, independent read-only Gate Reviewer

Result: FAIL

## Accepted Remediation

- PC-0004 honestly classifies all 100 review definitions as
  `unverified_playable`; zero claim semantic promotion.
- Exact and up-to modal counts are distinct, and cross-face keyword leakage is
  corrected.
- Pest grammar and relevant source trees invalidate mutation/fuzz evidence.
- Linked platform artifacts, exact target identities, ASAN commands, final
  statistics, commit/tree bindings, and full CP-DSL validators are checked.
- The supplied packet records 117/59 diagnostics, 28/28 mutation kills, 2,408
  ASAN worker-seconds, 1,200 oracles, and 80.3077% line coverage.

## Remaining Findings

### P1: Coverage was not replayed

The packet trusted the summary's passing flag and did not rehash the raw LLVM
report or recompute its 80% floor.

Remediation implemented after review: `coverage_summary.py --check` rebuilds
the summary from the retained raw report, and the full CP-DSL validator invokes
it after local verification produces current coverage.

### P1: Provenance manifests were under-checked

Oracle evidence did not bind its generator, and packet `source_bindings` and
`acceptance` values were not compared during recheck. Corpus generators also
were not replayed by the independent checker.

Remediation implemented after review: Oracle source hashing includes its
generator; packet source and acceptance manifests are recomputed; and positive
plus malformed corpus generators rerun in `cp_dsl_metrics.py --check`.

### P2: Exact counts were under-enforced

The checker accepted threshold-level malformed and mutation totals rather than
the packet's declared 117/59 diagnostics and unique 28/28 mutant set.

Remediation implemented after review: all four counts and the exact unique
mutant identities are now fail-closed.

## Subsequent Offline Re-review

Commit `dd828b2d96498f81cbe404cf74095d1d734e9e69` passed the fully hardened exact
packet. The reviewer reconciled every requested artifact and count, but failed
the gate because `CARGO_NET_OFFLINE=false` could override the local-only
default. Gate, local verification, card regression, and metric subprocesses now
force offline Cargo mode. Preflight and packet validation require explicit zero
network egress and `cargo_net_offline=true` evidence.

## Exact Checkout-binding Re-review

Commit `257f207cfe9bc227484c9b6dc4abf5125d759cdb` passed the exact packet with
forced offline Cargo mode. The reviewer reconciled all requested evidence and
confirmed that `CARGO_NET_OFFLINE=false` is overwritten, then failed the gate
because packet reuse did not compare the reviewed commit/tree to the current
`HEAD`/tree or reject non-evidence worktree changes. Packet validation now
enforces both bindings and limits checkout changes to exact-gate outputs.
