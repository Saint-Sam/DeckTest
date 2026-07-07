# ADR-0010: Criterion Benchmark Dependency

## Context

T1.13 requires Criterion benchmarks for core clone, legal-action
enumeration, action application, and deterministic playout performance. The
master plan keeps `forge-core` dependency-free for normal builds, but lists
`criterion` as the benchmark dev-dependency for the performance gate.

## Decision

Add `criterion` as a `forge-core` dev-dependency only, with a single
`kernel` bench target. Normal library, test, CLI, UI, mobile, and WASM builds
do not link Criterion. Export Criterion's measured mean estimates into
`metrics/perf_current.json`, then compare them against a committed
`metrics/perf_baseline.json` through `tools/perf_diff.py`.

## Consequences

Fresh clones can reproduce the benchmark dependency set from `Cargo.lock`.
The VL perf smoke now produces current metrics before diffing, so regressions
stop being silently skipped once the baseline exists. Criterion remains outside
runtime artifacts and does not weaken the `forge-core` production dependency
boundary.

The direct third-party benchmark dependency is expected to be GPL-compatible
by manifest (`criterion`, Apache-2.0 OR MIT). Transitive license auditing
remains the responsibility of the planned `cargo deny` gate.

## Alternatives

A custom benchmark harness was rejected because T1.13 explicitly calls for
Criterion and because Criterion output gives stable estimate files that can be
converted into the existing JSON perf gate.
