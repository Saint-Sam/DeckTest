# T1.10 Legacy Test to Oracle Mapping

Date: 2026-07-07

## Source Audit

T1.10 asks for the top 100 legacy `forge-game` unit tests to be ported into
oracle scenarios. The vendored checkout does not currently contain 100 tests in
that path.

- Exact path audited: `vendor/legacy-forge/forge-game/src/test/java`
- Exact files found: 2
- Exact `@Test` methods found: 3
- Top-100 ranking metadata found: none

Because the exact source set is too small, this artifact records all exact
`forge-game` tests and then separately records T1-relevant supplemental legacy
game-simulation tests from `forge-gui-desktop/src/test/java/forge/gamesimulationtests`.
Treat that supplemental source as a proposed mapping extension, not as a silent
change to the T1.10 scope.

## Exact `forge-game/src/test` Mapping

| Rank | Legacy test id | Legacy source | Legacy case/assertion | Scenario path | Disposition | Oracle basis | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `forge.game.ability.AbilityKeyTest.testFromStringWorksForAllKeys` | `vendor/legacy-forge/forge-game/src/test/java/forge/game/ability/AbilityKeyTest.java:11` | Java enum string round-trip for `AbilityKey`. | n/a | `not_oracle_meaningful` | Legacy implementation invariant | Not a game-rule observable scenario. |
| 2 | `forge.game.ability.AbilityKeyTest.testCopyingEmptyMapWorks` | `vendor/legacy-forge/forge-game/src/test/java/forge/game/ability/AbilityKeyTest.java:18` | Empty `AbilityKey` map copy allocates a distinct map. | n/a | `not_oracle_meaningful` | Legacy implementation invariant | Not a game-rule observable scenario. |
| 3 | `forge.game.mana.ManaCostBeingPaidTest.testPayManaViaConvoke` | `vendor/legacy-forge/forge-game/src/test/java/forge/game/mana/ManaCostBeingPaidTest.java:15` | Convoke payments reduce colored/generic remainder in legacy order. | n/a | `blocked_schema` | Legacy mana/payment behavior | Forge 2.0 has payment plans, but no convoke-specific cost payment surface yet. |

## Supplemental Game-Simulation Mapping

These rows come from the legacy game simulation harness outside the exact
`forge-game/src/test` path. The ported rows exercise already-supported T1 kernel
behavior through `forge-testkit` scenarios.

| Rank | Legacy test id | Legacy source | Legacy case/assertion | Scenario path | Disposition | Oracle basis | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| S1 | `ComprehensiveRulesSection103.test_103_3_players_start_at_20_life` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection103.java:19` | Both players start at 20 life. | `tests/oracle/t1_10_cr_103_3_players_start_at_20_life.ron` | `ported` | CR 103.3 | Direct player-state expectation. |
| S2 | `ComprehensiveRulesSection103.test_103_7a_first_player_skips_draw_step_of_first_turn` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection103.java:32` | Starting player skips the first draw step. | `tests/oracle/t1_10_cr_103_7a_starting_player_skips_first_draw.ron` | `ported` | CR 103.7a | Required a T1.10 kernel fix scoped to decided turn order. |
| S3 | `ComprehensiveRulesSection104.test_104_2a_player_wins_if_all_opponents_left_even_if_he_couldnt_win` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:23` | Opponent concession wins despite Abyssal Persecutor restriction. | n/a | `blocked_card_semantics` | CR 104.2a plus card replacement/restriction text | Needs concession and card-specific win/loss restriction semantics. |
| S4 | `ComprehensiveRulesSection104.test_104_2b_effect_may_state_that_player_wins` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:39` | Near-Death Experience can make a player win. | n/a | `blocked_card_semantics` | CR 104.2b | Needs triggered ability/card text support. |
| S5 | `ComprehensiveRulesSection104.test_104_3b_player_with_zero_life_loses_the_game` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:51` | Player with 0 life loses. | `tests/oracle/t1_10_cr_104_3b_zero_life_loses.ron` | `ported` | CR 104.3b, CR 704.5a | Direct SBA expectation. |
| S6 | `ComprehensiveRulesSection104.test_104_3b_player_with_less_than_zero_life_loses_the_game` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:62` | Player with negative life loses. | `tests/oracle/t1_10_cr_104_3b_negative_life_loses.ron` | `ported` | CR 104.3b, CR 704.5a | Direct SBA expectation. |
| S7 | `ComprehensiveRulesSection104.test_104_3b_player_with_less_than_zero_life_loses_the_game_only_when_a_player_receives_priority` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:73` | A player who briefly drops below 0 can survive if life gain happens before SBA. | n/a | `blocked_card_semantics` | CR 104.3b, CR 117, CR 704 | Needs Lightning Helix spell behavior. |
| S8 | `ComprehensiveRulesSection104.test_104_3b_player_with_less_than_zero_life_loses_the_game_only_when_a_player_receives_priority_variant_with_combat` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:92` | Combat damage and lifelink are applied before loss SBAs. | n/a | `blocked_schema` | CR 104.3b, CR 510, CR 704 | Kernel has combat/lifelink coverage, but scenario DSL does not yet expose combat setup/actions. |
| S9 | `ComprehensiveRulesSection104.test_104_3c_player_who_draws_card_with_empty_library_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:117` | Empty-library draw loss happens before priority. | `tests/oracle/t1_10_cr_104_3c_empty_library_draw_loses.ron` | `ported` | CR 104.3c, CR 704.5b | Direct draw-step/SBA expectation. |
| S10 | `ComprehensiveRulesSection104.test_104_3c_player_who_draws_more_cards_than_library_contains_draw_as_much_as_possible_and_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:123` | Multi-draw empties library, keeps drawn cards, then loses. | n/a | `blocked_card_semantics` | CR 104.3c, CR 121 | Needs Tidings/card draw effect support. |
| S11 | `ComprehensiveRulesSection104.test_104_3d_player_with_ten_poison_counters_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:141` | Ten poison counters lose the game. | `tests/oracle/t1_10_cr_104_3d_ten_poison_loses.ron` | `ported` | CR 104.3d, CR 704.5c | Added scenario action for poison counters. |
| S12 | `ComprehensiveRulesSection104.test_104_3d_player_with_more_than_ten_poison_counters_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:152` | More than ten poison counters also lose the game. | `tests/oracle/t1_10_cr_104_3d_more_than_ten_poison_loses.ron` | `ported` | CR 104.3d, CR 704.5c | Direct SBA expectation. |
| S13 | `ComprehensiveRulesSection104.test_104_3e_effect_may_state_that_player_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:163` | Final Fortune delayed effect can make a player lose. | n/a | `blocked_card_semantics` | CR 104.3e | Needs spell and delayed-trigger semantics. |
| S14 | `ComprehensiveRulesSection104.test_104_3f_if_a_player_would_win_and_lose_simultaneously_he_loses` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/comprehensiverules/ComprehensiveRulesSection104.java:180` | Simultaneous win/loss resolves as loss. | n/a | `blocked_legacy_disabled` | CR 104.3f | Disabled in legacy and requires multiple unsupported card effects. |
| S15 | `ReplacementHandlerTest.testPerpetualEntersTappedReplacementEffect` | `vendor/legacy-forge/forge-gui-desktop/src/test/java/forge/gamesimulationtests/ReplacementHandlerTest.java:39` | Perpetual replacement effect enters tapped without recursion. | n/a | `blocked_later_tier` | Replacement effects/perpetual effects | Not a T1 kernel rule surface. |

## Open Decision

T1.10 cannot be completed as written from the local vendored source because the
exact `forge-game/src/test` corpus has only 3 test methods. Finishing the
"top 100" requirement needs one of these owner decisions:

1. Provide or approve fetching a fuller upstream legacy test corpus.
2. Approve a scope change allowing T1.10 to use the broader legacy
   `forge-gui-desktop` game-simulation tests as the source set.
3. Keep the exact `forge-game/src/test` scope and mark the remaining 97 rows as
   blocked by missing source evidence.
