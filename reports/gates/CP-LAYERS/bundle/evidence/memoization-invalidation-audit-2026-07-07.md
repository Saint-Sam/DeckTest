# CP-LAYERS Memoization and Invalidation Audit

Date: 2026-07-07

## Current Design

T2.4 does not introduce a derived-characteristics cache. `GameState` stores
base object state plus registered `ContinuousEffectDefinition` entries, then
recomputes effective characteristics on each query through
`GameState::object_characteristics`, `object_controller`, and
`creature_characteristics`.

Relevant implementation paths:

- `crates/forge-core/src/lib.rs:5414` stores `continuous_effects`.
- `crates/forge-core/src/lib.rs:6736` registers continuous effects.
- `crates/forge-core/src/lib.rs:8083` starts the effective-characteristics
  query path.
- `crates/forge-core/src/lib.rs:8142` orders applicable effects per layer.
- `crates/forge-core/src/lib.rs:8173` applies one continuous effect operation.
- `crates/forge-core/src/lib.rs:7405` and `crates/forge-core/src/lib.rs:7511`
  include continuous-effect state in canonical bytes and streaming hashes.

## Invalidation Finding

Because there is no memoized derived state, invalidation is currently structural:
mutations update base state or the effect list, and the next query recomputes
from those current values. No cache-clear path exists or is required in T2.4.

This is intentionally conservative. If a later task adds memoization for
performance, that task must introduce explicit invalidation evidence before the
cache may be used by rules code.

## Fuzz Requirement

The CP-LAYERS review requirement for a mutation/query interleaving fuzz target
is now represented by `fuzz/fuzz_targets/fuzz_characteristics.rs`. It randomly
interleaves:

- continuous-effect registration across copy, control, text, type, color,
  ability, and power/toughness layers;
- base creature characteristic updates and clears;
- zone moves, damage marking, and state-based action checks;
- effective controller/characteristics queries and deterministic hash checks.

Local smoke:

- `cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`: PASS.
- `cargo run --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics -- -runs=16`:
  PASS.

## Reviewer Action Still Required

The reviewer must decide whether this fuzz target is sufficient for CP-LAYERS or
must demand a longer sanitizer run before signing off.
