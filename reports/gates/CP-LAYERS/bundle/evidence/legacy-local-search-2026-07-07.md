# CP-LAYERS Local Legacy Search

Date: 2026-07-07

Scope: local-only search under `vendor/legacy-forge`. No network access, clone,
download, or upstream fetch was used.

## Engine Anchors Found

- `vendor/legacy-forge/forge-game/src/main/java/forge/game/staticability/StaticAbilityLayer.java`
  defines legacy continuous-effect layers for copy, control, text, type, color,
  abilities, 7a characteristic P/T, 7b set P/T, and 7c modify P/T. Legacy
  comments show 7d switch P/T is present as a commented layer.
- `vendor/legacy-forge/forge-game/src/main/java/forge/game/GameAction.java`
  iterates `StaticAbilityLayer.CONTINUOUS_LAYERS`, tracks affected objects per
  ability for CR 613.6 style continued application, reevaluates remaining
  effects after each application, and builds a same-layer dependency graph with
  cycle removal before timestamp fallback.
- `vendor/legacy-forge/forge-game/src/main/java/forge/game/staticability/StaticAbilityContinuous.java`
  applies layer-specific operations for text changes, type changes, color
  changes, ability/keyword addition and removal, 7a/7b set P/T, and 7c P/T
  boosts.
- `vendor/legacy-forge/forge-game/src/main/java/forge/game/card/Card.java`
  stores characteristic changes keyed by timestamp and static ability id for
  card names, mana costs, types, colors, keywords, text, copy state, set P/T,
  and P/T boosts.
- `vendor/legacy-forge/forge-game/src/main/java/forge/game/card/CardCopyService.java`
  and `vendor/legacy-forge/forge-game/src/main/java/forge/game/card/CardFactory.java`
  contain local legacy copy-characteristic handling.

## Representative Card Scripts Found

- `vendor/legacy-forge/forge-gui/res/cardsfolder/h/humility.txt`: ability
  removal plus base P/T setting.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/o/opalescence.txt`: type
  addition plus base P/T setting from mana value.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/b/blood_moon.txt`: nonbasic
  land type replacement.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/s/song_of_the_dryads.txt`:
  colorless effect, type replacement, and land type replacement.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/d/darksteel_mutation.txt`:
  type replacement, ability removal, keyword addition, and base P/T setting.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/c/clone.txt`: copy entry
  replacement.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/v/vesuvan_shapeshifter.txt`:
  copy entry/face-up replacement with additional gained text.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/a/archetype_of_imagination.txt`:
  keyword granting, keyword removal, and "can't have" keyword effect.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/a/archetype_of_endurance.txt`:
  hexproof granting and opponent hexproof suppression.
- `vendor/legacy-forge/forge-gui/res/cardsfolder/a/archetype_of_courage.txt`:
  first strike granting and opponent first strike suppression.

## Local Search Counts

These counts are raw local card-script hits and intentionally include cards that
still need human pruning before the 100-card differential set is final.

| Search family | Local hits |
| --- | ---: |
| `RemoveAllAbilities$` | 97 |
| type add/remove terms | 538 |
| P/T set or modify terms | 2402 |
| keyword add/remove/suppress terms | 1842 |
| control/text/copy terms | 1911 |
| `S:Mode$ Continuous` scripts | 4342 |

## Status

Local legacy evidence is sufficient to build a layered subset without network
egress. The actual 100-card differential is not complete yet; CP-LAYERS must
remain pending until the selected subset is run and every divergence is
adjudicated in writing.
