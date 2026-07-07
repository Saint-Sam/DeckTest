# ADR-0009: Fuzz Harness Dependencies

Date: 2026-07-07

## Status

Accepted for T1.12 implementation.

## Context

T1.12 requires `fuzz_apply` and `fuzz_scenarioparse` cargo-fuzz targets with a
6-hour clean run. The master plan's initial dependency budget explicitly
allows `arbitrary` and `libfuzzer-sys` for fuzzing, while keeping `forge-core`
dependency-free.

## Decision

Create a standalone `fuzz/` cargo-fuzz workspace that depends on
`libfuzzer-sys` and local path crates only. Keep the fuzz harness outside the
root workspace so normal build/test/release artifacts remain GPL-3.0-only
project code and `forge-core` stays std-only.

Commit `fuzz/Cargo.lock` so fresh clones reproduce the fuzz dependency set.

## Consequences

Developers can run cargo-fuzz without adding dependencies to engine crates.
Normal `scripts/vl.sh` and root workspace CI remain unchanged.

Sanitizer-backed fuzzing requires nightly Rust. Local stable smoke checks can
set `FORGE_FUZZ_SANITIZER=none`, but acceptance and recurring fuzzing should
use the default sanitizer mode on nightly.

The direct third-party fuzz dependencies are GPL-compatible by manifest:
`arbitrary` is `MIT OR Apache-2.0`; `libfuzzer-sys` is
`(MIT OR Apache-2.0) AND NCSA`. The resolved transitive set in
`fuzz/Cargo.lock` is crates.io-only and limited to cargo-fuzz build/runtime
support.

## Alternatives Considered

Putting fuzz dependencies in the root workspace was rejected because the core
engine dependency boundary is a Tier 1 invariant.

Hand-rolled byte loops under normal unit tests were rejected because they do
not provide libFuzzer corpus growth, minimization, or sanitizer integration.
