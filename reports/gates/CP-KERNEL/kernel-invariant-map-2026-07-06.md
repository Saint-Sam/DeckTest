# CP-KERNEL Kernel Invariant Map - 2026-07-06

Checkpoint: `CP-KERNEL`, required after T1.7 merges.

Reviewed head: `6491d5f` (`T1.7: add state-based actions`)

Plan scope: `FORGE_REBUILD_MASTER_PLAN.md` Section 3.3 and Section 15.4.

This is Orchestrator evidence for the Gate Reviewer. It is not a sign-off.

## Section 3.3 Invariants

| Invariant | Evidence in current code | Review focus |
| --- | --- | --- |
| 1. Flat state | `GameState` is `Clone`; state is organized in index-addressed IDs, `Vec` arenas, and `Copy`-friendly records. | Clone allocation/bench gates are not implemented until later T1 tasks, so the reviewer should decide whether this is acceptable at CP-KERNEL or must become remediation now. |
| 2. Two-function surface | No callback-based kernel escape is visible. | High risk: `GameState` currently exposes multiple public mutating helpers (`add_player`, `start_turn`, `cast_spell`, `declare_attackers`, `assign_combat_damage`, `check_state_based_actions`, etc.) and does not yet expose the final `legal_actions/apply` surface. The reviewer should decide whether temporary T1 scaffolding is acceptable or whether API narrowing must happen before T1.8+. |
| 3. Determinism | State hash is derived from canonical serialization; streaming and allocated hash paths include seed, turn, zones, stack, combat, outcome, and pending SBA markers. Remote `determinism-replay` is green. | The replay corpus is still absent, so determinism is checked structurally and by unit tests, not by full-game replay yet. |
| 4. Characteristics computed, not stored | T1 vanilla combat stores `CreatureCharacteristics` on objects as temporary base characteristics. No CR 613 layer system exists yet. | High risk: the present-tense invariant says characteristics are computed from base printed values plus continuous effects. The reviewer should decide whether T1 temporary storage is acceptable or whether a base/derived split must be introduced before layers. |
| 5. Cards are data | `forge-core` has `CardId` and no intentional card-name implementations. The card-name review hook is present. | The hook currently skips because card-name source is absent, so this is mostly structural until card definitions arrive. |
| 6. Hidden information honesty | No AI or `PlayerView` boundary exists yet. `canonical_bytes()` is documented as full-information diagnostics only. | T1.8 introduces opening hands and libraries as gameplay-critical hidden zones; the reviewer should decide whether the `PlayerView` boundary needs to land before or with T1.8. |
| 7. No panics in the kernel | Public fallible paths return `Result` or an outcome enum; `no_unwrap` review hook passes. | Continue checking that new code avoids panicking helpers in engine paths. |

## Public Mutating API Observed

The current public mutating API includes at least:

- `set_seed`
- `add_player`
- `set_player_life`
- `lose_life`
- `gain_life`
- `add_poison_counters`
- `add_mana_to_pool`
- `clear_mana_pool`
- `clear_creature_characteristics`
- `set_object_tapped`
- `check_state_based_actions`
- `start_turn`
- `advance_step`
- `pass_priority`
- `set_attackers_declared_this_combat`
- `request_cleanup_priority`
- `add_duration_marker`
- `move_object`

Additional public mutating methods use multi-line signatures and should also be
reviewed directly in `crates/forge-core/src/lib.rs`.

## Checkpoint Question For Reviewer

Does CP-KERNEL pass with temporary T1 scaffolding, or should the project stop
and add remediation tickets to narrow the kernel surface and separate base
printed characteristics from computed characteristics before T1.8+ proceeds?
