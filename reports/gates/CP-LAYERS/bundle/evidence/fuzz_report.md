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

Owner decision on 2026-07-07: run a longer sanitizer fuzz if it is already
installed; ask before installing anything.

Installed local prerequisites found:

- `cargo-fuzz`: `/Users/juanlopez2016/.cargo/bin/cargo-fuzz`
- Rust toolchains: `stable-aarch64-apple-darwin` and
  `nightly-aarch64-apple-darwin`

Long sanitizer run:

- Command:
  `cargo +nightly fuzz run --sanitizer address fuzz_characteristics -- -max_total_time=300`
- Status: PASS.
- Result: 378,902 runs in 301 seconds.
- Final coverage/features/corpus summary:
  `cov: 1546 ft: 8926 corp: 1479/180Kb`.
- Artifacts: no crash artifact was produced in
  `fuzz/artifacts/fuzz_characteristics`.
- Corpus: 1,478 local corpus files were generated under
  `fuzz/corpus/fuzz_characteristics`; this path is ignored by `.gitignore`.

Non-fatal local warnings:

- macOS sandbox `xcrun` cache-file warnings.
- libFuzzer sanitizer-symbol warnings in the non-sanitizer smoke invocation.

No crash, panic, or invariant failure was observed in the smoke run.

No crash, panic, or invariant failure was observed in the 301-second
address-sanitizer run.
