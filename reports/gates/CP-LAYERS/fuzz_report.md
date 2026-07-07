# CP-LAYERS Fuzz Report

Date: 2026-07-07

Target added for checkpoint review:

- `fuzz/fuzz_targets/fuzz_characteristics.rs`

Purpose:

- Exercise the CR 613 characteristic-query path under interleaved mutations.
- Verify zone conservation and allocated-vs-streaming deterministic hash
  agreement after each fuzz step.
- Provide the CP-LAYERS reviewer a concrete target for longer sanitizer fuzzing.

Local smoke results:

- `cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`:
  PASS.
- `cargo run --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics -- -runs=16`:
  PASS.

Non-fatal local warnings:

- macOS sandbox `xcrun` cache-file warnings.
- libFuzzer sanitizer-symbol warnings in the non-sanitizer smoke invocation.

No crash, panic, or invariant failure was observed in the smoke run.
