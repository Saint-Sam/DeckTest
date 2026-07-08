# CP-LAYERS Legacy Engine Snapshot

Date: 2026-07-07

Mode: local-only execution of the vendored legacy Forge Java engine against the selected CP-LAYERS 100-card layered subset.

Result: PASS

This evidence upgrades the earlier local-search finding: the legacy Java engine is now runnable through repo-local Corretto 17 and Maven artifacts. The remaining true differential blocker is on the Forge 2.0 side: it still has no real legacy card-script importer/card compiler for executing these same 100 cards in the new engine.

## Harness

- Runner: `tools/run_legacy_layer_snapshot.sh`
- Fixture per card: `Runeclaw Bear` under opponent control, `Memnite` under controller control, then the selected source card.
- Attachments: Auras attach to the opponent creature when legal; Equipment/Fortifications attach to the controller artifact when legal.
- Snapshot fields: controller, type line, colors, net power/toughness, and keyword originals after `checkStaticAbilities(false)`.
- Network: none during this run.

## Counts

| Metric | Count |
| --- | ---: |
| Selected subset cards | 100 |
| Legacy snapshots emitted | 100 |
| OK snapshots | 100 |
| Error snapshots | 0 |
| Missing selected names | 0 |
| Java process return code | 0 |

## Artifacts

- Machine-readable summary: `metrics/cp_layers_legacy_engine_snapshot.json`
- Legacy snapshot JSONL: `metrics/cp_layers_legacy_engine_snapshot.jsonl`

## Sample Snapshots

| ID | Scenario | Snapshot summary |
| --- | --- | --- |
| L001 | Humility | Humility [Enchantment; W; 0/0; ] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 1/1; ] |
| L002 | Opalescence | Memnite [Artifact Creature - Construct; C; 1/1; ] / Opalescence [Enchantment; W; 0/0; ] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L003 | Blood Moon | Blood Moon [Enchantment; R; 0/0; ] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L004 | Song of the Dryads | Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Land - Forest; C; 0/0; ] / Song of the Dryads [Enchantment - Aura; G; 0/0; Enchant:Permanent:permanent] |
| L005 | Darksteel Mutation | Darksteel Mutation [Enchantment - Aura; W; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Artifact Creature - Insect; G; 0/1; Indestructible] |
| L006 | Archetype of Imagination | Archetype of Imagination [Enchantment Creature - Human Wizard; U; 3/2; Flying] / Memnite [Artifact Creature - Construct; C; 1/1; Flying] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L007 | Archetype of Endurance | Archetype of Endurance [Enchantment Creature - Boar; G; 6/5; Hexproof] / Memnite [Artifact Creature - Construct; C; 1/1; Hexproof] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L008 | Archetype of Courage | Archetype of Courage [Enchantment Creature - Human Soldier; W; 2/2; First Strike] / Memnite [Artifact Creature - Construct; C; 1/1; First Strike] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L009 | March of the Machines | March of the Machines [Enchantment; U; 0/0; ] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L010 | Magus of the Moon | Magus of the Moon [Creature - Human Wizard; R; 2/2; ] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L011 | Imprisoned in the Moon | Imprisoned in the Moon [Enchantment - Aura; U; 0/0; Enchant:Creature,Land,Planeswalker:creature, land, or planeswalker] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Land; C; 0/0; ] |
| L012 | Kenrith's Transformation | Kenrith's Transformation [Enchantment - Aura; G; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Elk; G; 3/3; ] |
| L013 | Dress Down | Dress Down [Enchantment; U; 0/0; Flash] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 2/2; ] |
| L014 | Ichthyomorphosis | Ichthyomorphosis [Enchantment - Aura; U; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Fish; U; 0/1; ] |
| L015 | Witness Protection | Legitimate Businessperson [Creature - Citizen; GW; 1/1; ] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Witness Protection [Enchantment - Aura; U; 0/0; Enchant:Creature] |
| L016 | Kasmina's Transmutation | Kasmina's Transmutation [Enchantment - Aura; U; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear; G; 1/1; ] |
| L017 | Mystic Subdual | Memnite [Artifact Creature - Construct; C; 1/1; ] / Mystic Subdual [Enchantment - Aura; U; 0/0; Enchant:Creature, Flash] / Runeclaw Bear [Creature - Bear; G; 0/2; ] |
| L018 | Frogify | Frogify [Enchantment - Aura; U; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Frog; U; 1/1; ] |
| L019 | Deep Freeze | Deep Freeze [Enchantment - Aura; U; 0/0; Enchant:Creature] / Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Bear Wall; GU; 0/4; Defender] |
| L020 | Spider-Man No More | Memnite [Artifact Creature - Construct; C; 1/1; ] / Runeclaw Bear [Creature - Citizen; G; 1/1; Defender] / Spider-Man No More [Enchantment - Aura; U; 0/0; Enchant:Creature] |

## Gate Consequence

The legacy side of the 100-card differential is now executable and recorded. CP-LAYERS still cannot honestly be marked PASS for the true engine-vs-engine clause until Forge 2.0 can translate/import these selected card scripts into executable new-engine layer definitions, or the owner explicitly changes the checkpoint requirement.
