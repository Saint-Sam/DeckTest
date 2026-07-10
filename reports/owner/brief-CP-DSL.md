# OWNER BRIEF - CP-DSL Checkpoint

Date: 2026-07-09

## What Approval Freezes

Approval freezes the Oracle-id/printing-id split, ordered multi-face model,
typed `.frs` language, closed operation and argument-type registry,
deterministic database format, defensive loader, and the compiled
card-to-runtime lowering boundary used by the layer integration scenarios.

It does not claim that all catalog identities already have complete playable
mechanics. The catalog classifies every identity; this checkpoint contains 100
reviewed mechanics definitions used to stress the language.

## Evidence

- 113,234/113,234 English printings imported, 38,306/38,306 identities
  classified, and zero dangling references.
- Exactly 100 reviewed definitions in the closed 25-stratum set, four cards in
  every stratum, with catalog-only records checked separately.
- 127 typed operations, recursive argument validation, 100/100 canonical
  round-trips, and 117/117 positioned malformed diagnostics. The tagged subset
  contains 59 recursive-argument cases covering every argument kind.
- Three isolated clean builds produced byte-identical main and layer-scenario
  databases.
- The nightmare suite casts ten compiled scenario cards and lowers their
  validated `layer_effect` trees; 100 games pass with zero invariant failures.
- The unmutated control passes and 28/28 curated mutants are killed by expected
  tests, with zero survivor or invalid mutant; all full logs are retained and
  hashed.
- All five address-sanitizer targets pass 2,408 verified worker-seconds with
  retained libFuzzer final-stat logs.
- Four clean platform-package builds emit a WASM module plus Android, iOS, and
  Windows static libraries; every linked artifact has a retained log, nonzero
  size, expected magic, and SHA-256.
- All 1,200 semantic scenarios pass; the breadth audit measures 379 structural
  families and 1,839 interactions.
- Workspace line coverage remains above the unchanged 80% floor.
- Full GPL-3.0 text, offline dependency-license checks, and a simulated GitHub
  ZIP bootstrap without submodule contents all pass.
- Hosted Actions use is zero.
- The exact detached packet binds its command logs, linked-platform records,
  and artifacts to the reviewed commit and must pass its independent checker.

## Reviewer Remediation

The first independent review failed. Its four P1 findings were fixed: effects
are no longer supplied by hardcoded fixture seeds, nested operation arguments
are typed and fail closed, strata are exact rather than count-only, token-set
classification is corrected, and the gate now executes real platform,
semantic, and isolated clean-build checks.

The second independent review failed on evidence depth rather than runtime
behavior. Its two P1 findings were fixed with 59 tagged recursive diagnostics
and an exact packet that verifies controls, expected killer assertions, actual
fuzz runtime/final statistics, complete logs, hashes, toolchains, and isolated
targets. During the exact rerun, fuzzing found a type-line round-trip crash;
that parser bug is fixed and the exact input is now a permanent regression
seed.

The third independent review rejected metadata-only cross-target checks. Those
lanes now perform clean `cargo rustc` builds for the platform app packages and
fail unless a linked target artifact exists and its retained log, size, magic,
and SHA-256 validate.

The fourth independent review confirmed that the exact detached gate and its
supplied artifacts pass, but rejected the freeze. The generator labels all 100
definitions `verified_playable`, which promises semantic verification, while
the packet proves typing, round-tripping, compilation, and broad engine
oracles rather than card-by-card fidelity. Sampled recipes contain intentional
or accidental approximations. The reviewer also found three evidence-checker
gaps; complete source hashing, direct linked-artifact validation, exact target
coverage, full validator replay, and address-sanitizer command checks are now
implemented locally and await the next exact rerun.

The fifth independent review accepted PC-0004 and the honest unverified status,
but rejected remaining checker assumptions. Coverage now replays and hashes the
raw LLVM report against the exact 80% floor; Oracle, source-binding, and
acceptance manifests are recomputed; both corpus generators rerun; and the
packet enforces exactly 117/59 diagnostics plus the unique 28/28 mutant set.

The sixth independent review reconciled every semantic and evidence result but
found that a caller could set `CARGO_NET_OFFLINE=false`. All gate entry points
now force offline Cargo mode, and exact preflight plus packet validation require
both `cargo_net_offline=true` and `network_egress_used=false`.

## Owner Decision

The Owner chose the honest staged-verification option. PC-0004 relabels all 100
recipes as `unverified_playable`, reserves `verified_playable` for definitions
with card-specific semantic tests, and leaves promotion to T3.6 and
CP-PORT-20. O4 remains pending until the resulting remediation passes a new
exact detached packet and independent re-review.
