# T2 Reviewer Oracle Pack

Date: 2026-07-09

Reviewer-only adversarial probes for the T2 exit gate. These scenarios live in
the evidence packet rather than the production oracle corpus.

| ID | File | Focus |
| --- | --- | --- |
| R001 | `reviewer_t2_copy_ignores_counters_and_modifiers.ron` | Copy values ignore counters and later modifiers. |
| R002 | `reviewer_t2_token_copy_ceases_without_original.ron` | Token copy ceases off battlefield while source remains. |
| R003 | `reviewer_t2_commander_tax_identity_combo.ron` | Commander tax and color identity checks together. |
| R004 | `reviewer_t2_protection_uses_effective_source_color.ron` | Protection reads source color through layers. |
| R005 | `reviewer_t2_haste_does_not_override_cannot_attack.ron` | Restriction beats haste in attack legality. |
| R006 | `reviewer_t2_scry_then_surveil_zone_order.ron` | Library ordering survives sequential scry/surveil. |
| R007 | `reviewer_t2_mana_then_stack_ability.ron` | Mana ability skips stack, activated life ability uses stack. |
| R008 | `reviewer_t2_layer_counter_sba_lethal.ron` | Layer/counter toughness effects feed SBA cleanup. |
