# ADR-0018: Safe Server Resource Telemetry

Status: Accepted for local implementation; measured promotion evidence pending.

## Context

T4 search records wall latency, but wall time is not CPU cost. Root-parallel
search also makes wall time an especially poor proxy because worker CPU can
overlap. The T4 engineering assessment requires measured per-decision CPU and
resident-memory fields while preserving the workspace-wide prohibition on
unsafe Rust and the no-network dependency policy.

## Decision

`forge-ai` exposes a small `ResourceSnapshot` captured before a production
adapter constructs the canonical decision context. On Linux and Android it
uses safe reads from:

- `/proc/thread-self/schedstat` for the calling thread's scheduled CPU
  nanoseconds; and
- `/proc/self/status` for process `VmRSS`.

Single-worker search reports the calling-thread CPU delta. Parallel search
adds each worker thread's independently measured CPU delta to the parent
thread delta, so waiting time is not reported as CPU. Resident memory is a
process-wide before/after delta and may include concurrent process activity;
campaign evidence must therefore freeze job configuration and record that
limitation.

Unsupported platforms return `None`. DeckTest will not substitute wall time,
estimated utilization, or a fixed multiplier. Physical reference-device and
macOS adapters require separate measured implementations.

## Consequences

- Linux/Android server campaigns can populate existing
  `actual_cpu_time_us` and `memory_delta_bytes` records without a new crate or
  unsafe code.
- Caller-side context construction is included when the runner supplies the
  pre-context snapshot.
- Worker pricing, utilization, failed-pod overhead, and 100-pod cost remain
  campaign evidence, not source-code constants.
- Exact replay selection is unchanged because resource fields are telemetry,
  not policy inputs.

## Acceptance

- parser tests fail closed on malformed or absent counters;
- strict clippy and fixed-iteration search tests pass;
- Android and WASM cross-compilation remain green;
- a supported Linux campaign must show non-null CPU and memory fields before
  any cost claim is promoted.
