# CP-LAYERS Local Verification

Date: 2026-07-07

Scope: local checkpoint-prep tree after T2.4 continuous effects, remote CI
evidence recording, and the CP-LAYERS fuzz-target addition.

## T2.4 Implementation Verification

- `cargo fmt --all --check`: PASS.
- `cargo test -p forge-core layers::`: PASS, 5 layer-focused unit tests.
- `cargo test -p forge-core`: PASS, 89 tests.
- `cargo test -p forge-testkit`: PASS.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  PASS.
- `cargo run -p forge-testkit -- oracle --filter layers --no-junit`: PASS
  during T2.4 prep; 81 matching scenarios passed because the filter also
  matched one non-layer path string.
- `cargo run -p forge-testkit -- oracle --path tests/oracle/layers --no-junit`:
  PASS after CP-LAYERS prep; 80 layer scenarios passed, 0 failed.
- `cargo run -p forge-testkit -- lint tests/oracle/reviewer_layers`: PASS
  after owner approval; 100 CP-LAYERS reviewer scenarios parsed.
- `cargo run -p forge-testkit -- oracle --path tests/oracle/reviewer_layers --no-junit`:
  PASS after owner approval; 100 CP-LAYERS reviewer scenarios passed, 0 failed.
- `scripts/review/clone_surface_guard.sh`: PASS,
  `persistent_allocation_field_count=24`.
- `scripts/vl.sh`: PASS during T2.4 prep; 382 oracle scenarios passed, 0
  failed, coverage 81.65% lines, and perf smoke reported 0 regressions.
- `scripts/vl.sh`: PASS after owner-approved reviewer oracle pack; 482 oracle
  scenarios passed, 0 failed, coverage 81.82% lines, and perf smoke reported 0
  regressions.
- `python3 tools/cp_layers_legacy_subset.py`: PASS, selected a local-only
  100-card legacy layered subset and wrote script-level divergence adjudication.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 -m py_compile tools/cp_layers_legacy_subset.py`:
  PASS.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 tools/scryfall_cache_summary.py`:
  PASS, recorded the approved local Scryfall card-data cache summary.
- `tools/run_legacy_layer_snapshot.sh Humility "Darksteel Mutation" "Angelic Armaments"`:
  PASS, compiled the local legacy harness and emitted three real legacy Forge
  post-layer snapshots.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 tools/cp_layers_legacy_engine_snapshot.py`:
  PASS, ran the vendored legacy Java engine over the selected 100-card subset;
  100 legacy snapshots emitted, 100 OK, 0 errors.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 -m py_compile tools/scryfall_cache_summary.py tools/cp_layers_legacy_engine_snapshot.py`:
  PASS.
- `bash -n tools/run_legacy_layer_snapshot.sh`: PASS.

## CP-LAYERS Fuzz Target Verification

- `cargo fmt --manifest-path fuzz/Cargo.toml --all --check`: PASS.
- `cargo check --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics`:
  PASS.
- `cargo run --manifest-path fuzz/Cargo.toml --bin fuzz_characteristics -- -runs=16`:
  PASS; the target executed 16 libFuzzer runs without a crash. macOS sandbox
  emitted non-fatal `xcrun` cache warnings and libFuzzer sanitizer-symbol
  warnings.

## Local Risk Notes

No derived-characteristics memoization cache exists yet. Effective
characteristics are recomputed per query from stored base object state plus the
registered continuous-effect list. This simplifies invalidation for CP-LAYERS,
but the checkpoint must still verify that future memoization work does not
weaken this invariant.

The legacy side of the 100-card engine differential is now executable and
recorded. The remaining true differential blocker is Forge 2.0 card-script
support: the new engine still has no importer/compiler capable of executing
those 100 real legacy card scripts for engine-vs-engine comparison.
