# CP-KERNEL Signoff

## Re-review: 2026-07-06

Reviewer identity: Kierkegaard, delegated strong-reasoning Gate Reviewer
(`gpt-5.5`), no implementation edits during review.

Verdict: **PASS**

Commit SHA reviewed: `d7fcb03` (`T1.R4: guard clone surface`)

Scope reviewed: failed CP-KERNEL remediation tickets T1.R1-T1.R4 plus fresh
G1 verification evidence. This is CP-KERNEL signoff only, not the Tier 1 exit
gate.

Current tree note: this re-review supersedes the historical FAIL below while
preserving the original failure and remediation tickets for audit history.

## Re-review Checklist

| Check | Verdict | Evidence |
| --- | --- | --- |
| T1.R1 action surface | PASS | Public mutation is routed through `legal_actions(&GameState) -> ActionList` and `apply(&mut GameState, Action) -> Outcome`; `scripts/review/no_public_mutating_gamestate.sh` is wired into VL. |
| T1.R2 characteristic boundary | PASS | `ObjectRecord` stores `base_creature: Option<BaseCreatureCharacteristics>`; current characteristics derive through `GameState::creature_characteristics`; combat and SBA paths use that query. |
| T1.R3 hidden information boundary | PASS | `ObjectView::Hidden` and `PlayerView` are present; `GameState::player_view` redacts opponent hands and all libraries; `canonical_bytes` is private diagnostics; `forge-ai` public state-facing code accepts `PlayerView`. |
| T1.R4 clone-surface discipline | PASS | `GameState` no longer derives `Debug`; full-state `GameSnapshot`/`StateSnapshot`/`snapshot()` are absent and guarded against resurrection; persistent allocation-bearing fields carry `clone_surface:` invariants; `metrics/clone_surface.json` records `persistent_allocation_field_count=18`. |
| G1 fresh sanity | PASS | Fresh local clone at `d7fcb03` ran `scripts/vl.sh` and exited `0` with `ALL CHECKS PASSED`; refreshed bundle `test_log.txt` also ends `ALL CHECKS PASSED`. |

## Re-review Notes

- Oracle subset and perf smoke are scheduled skips at CP-KERNEL because oracle
  scenarios and `metrics/perf_baseline.json` are not active yet.
- Fuzz absence remains expected at this checkpoint and is documented in the
  bundle.
- No blockers remain for T1.8 from CP-KERNEL.

## Historical Review: Initial Fail Superseded

Reviewer: Codex (GPT-5) Gate Reviewer; did not implement or review T1.

Date: 2026-07-06

Commit SHA reviewed: `6491d5fd12887eb5f5b7ad8d57eda83e1ff17783` (`T1.7: add state-based actions`)

Verdict: **FAIL**

The project must stop before T1.8. CP-KERNEL exists to catch kernel API drift before hidden-information opening hands, mulligans, testkit scenarios, replay, fuzzing, and AI consumers build on the surface. The current workspace head does not satisfy the Section 3.3 kernel invariants for the public mutation surface, characteristic derivation boundary, hidden-information boundary, or clone-surface discipline.

## Evidence Reviewed

- `FORGE_REBUILD_MASTER_PLAN.md` Section 3.3, Section 15.1-15.4, and Appendix A.0b.
- `reports/gates/CP-KERNEL/bundle/`, including `test_log.txt`, `fuzz_report.md`, `replays/README.md`, `questions_open.md`, `blockers_history.md`, and evidence files.
- `reports/gates/CP-KERNEL/kernel-invariant-map-2026-07-06.md`.
- `reports/owner/brief-CP-KERNEL.md`.
- Current real code at workspace head, especially `crates/forge-core/src/lib.rs`.

## Checks

| Check | Verdict | Evidence |
| --- | --- | --- |
| Reviewed correct commit | PASS | `git rev-parse HEAD` returned `6491d5fd12887eb5f5b7ad8d57eda83e1ff17783`. |
| Local focused verification | PASS with scope limits | Re-ran `cargo test -p forge-core`: 60 tests passed. Re-ran `scripts/review/no_unwrap.sh`: PASS. `scripts/review/no_card_names.sh` skipped because no card-name source exists. `scripts/review/determinism.sh` skipped because `replays/` is absent. |
| Evidence bundle completeness | PARTIAL | Bundle exists, but replays and fuzz are explicitly not active yet. This is expected schedule state, not proof of kernel correctness. |
| Two-function public surface | FAIL | No `legal_actions`, `ActionList`, `Action`, or public `apply` exists in `forge-core`. `GameState` exposes many public mutating methods, including `add_player` at `crates/forge-core/src/lib.rs:2689`, `set_player_life` at `:2722`, `add_mana_to_pool` at `:2780`, `set_creature_characteristics` at `:2835`, `check_state_based_actions` at `:2890`, `start_turn` at `:2920`, `advance_step` at `:2942`, `pass_priority` at `:2950`, `cast_spell` at `:2982`, low-level stack helpers at `:3036`, `:3058`, and `:3076`, combat mutators at `:3127`, `:3168`, and `:3213`, and object/zone mutators at `:3304` and `:3320`. |
| Characteristics computed, not stored | FAIL | `ObjectRecord` stores `creature: Option<CreatureCharacteristics>` at `crates/forge-core/src/lib.rs:2204`; public setters mutate it at `:2835` and `:2849`; core characteristic queries read the stored value at `:4040`; SBAs use stored toughness at `:4245-4254`. There is no base/derived split and no continuous-effects list boundary. |
| Hidden information honesty | FAIL before T1.8 | There is no `PlayerView` projection in `forge-core` or `forge-ai`. The public API exposes full `zones()`, `objects()`, `players()`, `snapshot().state()`, and full-information `canonical_bytes()` at `crates/forge-core/src/lib.rs:3426`. T1.8 would add gameplay-critical hidden zones without a redaction boundary. |
| Flat clone surface | FAIL | `GameState` is `Clone`, but it contains nested allocation-bearing records: `Zone` has `Vec<ObjectId>`, `StackEntry` has `Vec<TargetSnapshot>`, `ResolutionRecord` has two `Vec`s, and `CombatState` has multiple `Vec`s. `snapshot()` clones the whole `GameState` at `crates/forge-core/src/lib.rs:3404-3407`. No clone allocation benchmark or guard is active at CP-KERNEL. |
| Determinism | PARTIAL | The code uses explicit canonical serialization and no hash maps/floats were found in `forge-core`, `forge-ai`, or `forge-carddef`. However, the required replay determinism script currently skips because no replay corpus exists, so determinism is structurally plausible but not gate-proven. |
| Card-data boundary | PARTIAL | No known card names were found in `forge-core` or `forge-ai`, and `forge-ai` is still bootstrap-only. The actual IR interpretation boundary is not present yet; current tests and setup create objects and characteristics through public mutating helpers. |
| No panics in production kernel | PASS | `#[cfg(test)]` starts at `crates/forge-core/src/lib.rs:5245`; no `panic!`, `unwrap()`, or `expect()` hits were found before that line. The remaining panic hits are in tests. |
| Open P0/P1 questions and blockers | PASS | Bundle says no open questions and no blockers filed. |
| Owner brief | PASS | `reports/owner/brief-CP-KERNEL.md` exists and explicitly calls out the mutating surface and stored-characteristic risks. |

## Required Remediation Tickets

```yaml
id: T1.R1
title: Seal the kernel behind the Section 3.3 action surface
priority: P0
scope_paths:
  - crates/forge-core/src/lib.rs
  - scripts/review/
acceptance:
  - forge-core exposes public mutation only through legal_actions(&GameState) -> ActionList and apply(&mut GameState, Action) -> Outcome, plus read-only queries.
  - Public GameState methods that directly mutate state are made private, pub(crate), test-only, or moved behind an explicitly non-consumer testkit/internal API.
  - Low-level helpers such as put_spell_on_stack, put_ability_on_stack, move_object, create_object, set_* mutators, declare_* mutators, and check_state_based_actions are not callable by external consumers except through Action/apply.
  - A review script fails if new public GameState methods taking &mut self appear outside an allowlist for new/apply.
  - Existing forge-core tests either use the action surface or clearly test private internals from the inline test module without expanding the external API.
```

```yaml
id: T1.R2
title: Replace stored creature characteristics with a computed characteristic boundary
priority: P0
scope_paths:
  - crates/forge-core/src/lib.rs
  - crates/forge-carddef/src/lib.rs
acceptance:
  - ObjectRecord no longer stores current power, toughness, or keyword abilities as directly mutable CreatureCharacteristics.
  - Stored card/object data is split into base printed characteristics or carddef IR references, with current characteristics derived through a query function.
  - Combat and SBA code obtains power, toughness, types, and abilities through the derived-characteristics query path.
  - Direct setters that mutate current power, toughness, or keyword abilities are removed from the public API.
  - Tests cover that combat/SBA behavior follows the derived query path and that no code path directly mutates current creature power/toughness.
```

```yaml
id: T1.R3
title: Land the hidden-information projection before opening hands and mulligans
priority: P0
scope_paths:
  - crates/forge-core/src/lib.rs
  - crates/forge-ai/src/lib.rs
acceptance:
  - forge-core defines a PlayerView or equivalent redacted state projection for one observing player.
  - Player-facing and AI-facing read APIs use PlayerView, not &GameState or full GameSnapshot.
  - Full-information diagnostics such as canonical_bytes and full snapshots are explicitly segregated for replay/testing/debug use and are not the consumer-facing state API.
  - T1.8 opening-hand and library code has tests proving an opponent cannot inspect hidden hand/library contents through the public consumer API.
```

```yaml
id: T1.R4
title: Flatten or guard the GameState clone surface
priority: P1
scope_paths:
  - crates/forge-core/src/lib.rs
  - scripts/review/
  - tools/
acceptance:
  - Per-record game-state structures are Copy-friendly or store index/range handles into arenas rather than owning nested Vec allocations.
  - Any remaining nested Vec in GameState is documented with an invariant explaining why clone allocation remains bounded.
  - A clone benchmark or allocation-count guard is added before CP-KERNEL re-review, not deferred to T1.13, and the gate evidence records the baseline.
  - GameSnapshot/StateSnapshot construction does not require unnecessary full-information deep clones for UI/AI consumers.
```

## Re-review Scope

Re-review should cover only the remediation items above plus a fresh focused verification run. CP-KERNEL should not be signed, and T1.8 should not start, until T1.R1-T1.R3 are closed. T1.R4 may close by flattening or by a narrowly justified invariant plus an active benchmark guard, but it cannot remain implicit.
