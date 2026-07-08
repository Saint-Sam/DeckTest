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
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 tools/cp_layers_legacy_script_bridge.py`:
  PASS, parsed the selected 100 legacy scripts and generated 53 executable
  Forge 2.0 fragment scenarios for the currently representable layer subset.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 -m py_compile tools/cp_layers_legacy_script_bridge.py`:
  PASS.
- `cargo run -p forge-testkit -- lint tests/oracle/legacy_layers`: PASS,
  53 generated legacy-fragment scenarios parsed.
- `cargo run -p forge-testkit -- oracle --path tests/oracle/legacy_layers --junit target/forge-testkit/legacy-layers-junit.xml`:
  PASS, 53 generated legacy-fragment scenarios passed, 0 failed.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 tools/cp_layers_true_importer_diff.py`:
  PASS, imported the selected 100 legacy scripts into the CP-LAYERS stable-role
  fixture differential; 100/100 predicted snapshots matched the vendored legacy
  Java snapshots with 0 mismatches.
- `PYTHONPYCACHEPREFIX=target/tmp/python-cache python3 -m py_compile tools/cp_layers_true_importer_diff.py`:
  PASS.
- `scripts/gates/make_bundle.sh CP-LAYERS`: PASS, refreshed the packaged
  checkpoint bundle after adding bridge evidence.
- `scripts/vl.sh`: PASS after the legacy-script bridge; 535 oracle scenarios
  passed, 0 failed, and perf smoke reported 0 regressions.
- `scripts/vl.sh`: PASS after the true importer differential; 535 oracle
  scenarios passed, 0 failed, and perf smoke reported 0 regressions.

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

The legacy side of the 100-card engine differential is executable and recorded.
A Forge 2.0 legacy-script bridge also parses the 100 scripts and executes 53
currently representable layer fragments. The earlier true differential blocker
has now been remediated for CP-LAYERS: the stable-role importer differential
matches 100/100 selected legacy Java snapshots with 0 mismatches. Owner/human
signoff is still required before T2.5+.
