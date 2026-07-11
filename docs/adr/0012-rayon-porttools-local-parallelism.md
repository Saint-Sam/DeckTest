# ADR-0012: Rayon for local porttools corpus parallelism

Status: Accepted by the Gate Reviewer on 2026-07-10; PC-0007 Owner ratification
pending.

Date: 2026-07-10

## Context

`forge-porttools` parses, maps, fingerprints, and analyzes 33,290 pinned legacy
scripts. PC-0006 approved local resource-aware parallel campaigns, but Rayon is
not listed in the original dependency budget and no dependency ADR was recorded
when the porttools implementation added it. This ADR is a retroactive
governance remediation; earlier PC-0006 evidence does not itself approve the
dependency. The production rules kernel must remain single-threaded and
deterministic. `forge-core` already has the approved Criterion benchmark as a
dev dependency, whose resolved benchmark-only graph transitively includes
Rayon.

## Decision

Permit `rayon` only as a direct production dependency of `forge-porttools` for
bounded local corpus orchestration. Each command creates an explicit thread
pool with a caller-provided worker count from 1 through 24. The Rust entry
points reject larger values, and the campaign script rejects an explicit
oversubscription request while clamping hardware autodetection to 24. Parallel
work produces independent records; all externally visible results are sorted
before aggregation or serialization. Deterministic checkpoint replay compares
translation fingerprints/quarantine/priority reports and normalized blocker
plans/details at one versus multiple workers.

No direct or non-dev Rayon dependency, import, or production/runtime linkage is
permitted in `forge-core`, action application, game-state mutation, canonical
hashing, replay semantics, card runtime execution, or AI decision ordering
without a separate ADR and determinism review. The approved Criterion
benchmark-only transitive graph is not production linkage. A structured
manifest/import guard enforces the direct boundary. `Cargo.lock` pins the
resolved dependency graph. Routine campaigns remain offline and use one Cargo
target/cache.

## Alternatives Considered

- A hand-built `std::thread` queue would avoid the dependency but duplicate
  scheduling, panic propagation, and work-stealing behavior without improving
  product semantics.
- Serial processing is simpler but materially lengthens repeated T3 corpus
  campaigns and underuses the Owner's local 24-core host.
- Multiple worktrees/process caches increase disk use and were rejected by
  PC-0006.

## Consequences

- Dependency and license audits must include Rayon and its transitive graph.
- Planner/translator determinism tests remain mandatory across worker counts.
- Any worker count above 24 fails closed rather than silently oversubscribing.
- Worker-count increases are evidence-driven; simultaneous materializing
  sweeps remain prohibited because measured I/O contention made them slower.
- Removing Rayon later is localized to porttools orchestration and does not
  alter engine or file-format contracts.

## Verification

- `cargo test -p forge-porttools`
- `cargo clippy -p forge-porttools --all-targets --all-features -- -D warnings`
- `python3 tools/check_rayon_boundary.py`
- `scripts/t3_parallel_sweep.sh checkpoint`
- `cargo deny --offline --locked check licenses bans sources`

Acceptance requires all commands above to pass against the exact artifact
hashes in `docs/transition/GRAND_PLAN_V2_INTAKE.json`. PC-0007 Owner approval
does not substitute for the Gate Reviewer dependency disposition.

Local evidence: `reports/gates/ADR-0012/EVIDENCE.json`.

Gate Reviewer record: `reports/gates/ADR-0012/SIGNOFF.md`.
