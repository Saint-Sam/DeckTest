# T4 Critical Mutation Gate

Exact product: `19ef3302c40db3e916d2a60925546d4ebc28608d` / `e79efa91e0146f23f7219367e117db34ce13867a`

All 15 declared critical mutants were killed by focused tests.

- Score: 100.0%
- Survivors: 0
- Isolated temporary Cargo target: `target/t4-mutation/cargo-target`

| Mutant | Category | Killing test |
|---|---|---|
| M01_SHARED_DEADLINE_IGNORED_BETWEEN_TREES | shared total-decision deadline | `total_wall_budget_does_not_start_sequential_determinizations_after_expiry` |
| M02_CALLER_CONTEXT_TIME_RESET | caller-side context time consumes deadline | `caller_side_context_time_counts_against_the_total_budget` |
| M03_EDGE_VISITS_NOT_BACKPROPAGATED | edge-level visits and values | `state_keys_share_convergent_children_and_report_hits` |
| M04_EDGE_VALUES_NOT_BACKPROPAGATED | edge-level visits and values | `root_parallel_visit_sum_selects_the_winning_action_replayably` |
| M05_TRANSPOSITION_COLLISION_MERGED_BLINDLY | transposition collision guard | `colliding_keys_do_not_merge_non_equivalent_states` |
| M06_TRANSPOSITION_CHILD_NOT_REGISTERED | transposition equivalence reuse | `state_keys_share_convergent_children_and_report_hits` |
| M07_PLAYER_VIEW_LEAKS_HIDDEN_IDENTITIES | hidden-information poisoning | `hidden_identity_poison_does_not_change_sample` |
| M08_ATTACK_PATH_IGNORES_DEFENDER_PREFIX | hierarchical path discrimination | `attack_subcontexts_expose_split_defenders_without_a_cartesian_product` |
| M09_PARTIAL_TARGET_LEGALITY_FORCED_TRUE | partial target legality | `partially_illegal_targets_skip_only_their_bound_effects` |
| M10_SAME_BATCH_TRIGGER_TARGETS_NOT_STAGED | same-batch trigger target staging | `same_batch_trigger_targeting_uses_the_staged_stack_for_human_and_ai` |
| M11_TRIGGER_ORDER_IGNORES_SELECTED_IDS | trigger ordering | `simultaneous_trigger_order_uses_shared_human_and_ai_contexts` |
| M12_NO_LEGAL_TARGET_TRIGGER_FORCED_ON_STACK | no-legal-target trigger disposition | `required_trigger_without_legal_targets_is_removed_without_prompting` |
| M13_ADDITIONAL_DISCARD_COST_DROPPED | additional costs | `additional_spell_costs_use_canonical_human_ai_search_and_replay_paths` |
| M14_ALTERNATE_COST_IDENTITY_DROPPED | alternate costs | `commander_alternate_cost_uses_the_shared_canonical_cast_hierarchy` |
| M15_SEARCH_STOPS_BEFORE_OPPONENT_PRIORITY | search through opponent priority and resolution | `main_search_crosses_opponent_priority_and_resolves_the_response_stack` |
