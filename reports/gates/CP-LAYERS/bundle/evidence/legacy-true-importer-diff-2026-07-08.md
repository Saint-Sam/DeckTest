# CP-LAYERS True Importer Differential

Date: 2026-07-08

Mode: local-only import of the selected 100 legacy card scripts into a CP-LAYERS fixture evaluator with stable object roles and layer-ordered continuous effects.

Result: PASS

## Counts

| Metric | Count |
| --- | ---: |
| Selected scripts | 100 |
| Active-face continuous lines imported | 117 |
| Layer operations instantiated | 186 |
| Exact role snapshots matching legacy | 100 |
| Role snapshots with mismatches | 0 |

## Artifacts

- Machine summary: `metrics/cp_layers_true_importer_diff.json`
- Predicted snapshots: `metrics/cp_layers_true_importer_diff_predicted.jsonl`

## Per-Card Status

| ID | Card | Active face | Ops | Result | First mismatches / diagnostics |
| --- | --- | --- | ---: | --- | --- |
| L001 | Humility | Humility | 2 | match | none |
| L002 | Opalescence | Opalescence | 0 | match | selector matched no fixture objects: Enchantment.nonAura+Other |
| L003 | Blood Moon | Blood Moon | 0 | match | selector matched no fixture objects: Land.nonBasic |
| L004 | Song of the Dryads | Song of the Dryads | 2 | match | none |
| L005 | Darksteel Mutation | Darksteel Mutation | 4 | match | none |
| L006 | Archetype of Imagination | Archetype of Imagination | 2 | match | none |
| L007 | Archetype of Endurance | Archetype of Endurance | 2 | match | none |
| L008 | Archetype of Courage | Archetype of Courage | 2 | match | none |
| L009 | March of the Machines | March of the Machines | 0 | match | selector matched no fixture objects: Artifact.nonCreature |
| L010 | Magus of the Moon | Magus of the Moon | 0 | match | selector matched no fixture objects: Land.nonBasic |
| L011 | Imprisoned in the Moon | Imprisoned in the Moon | 3 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L012 | Kenrith's Transformation | Kenrith's Transformation | 4 | match | none |
| L013 | Dress Down | Dress Down | 1 | match | none |
| L014 | Ichthyomorphosis | Ichthyomorphosis | 4 | match | none |
| L015 | Witness Protection | Witness Protection | 5 | match | none |
| L016 | Kasmina's Transmutation | Kasmina's Transmutation | 2 | match | none |
| L017 | Mystic Subdual | Mystic Subdual | 2 | match | none |
| L018 | Frogify | Frogify | 4 | match | none |
| L019 | Deep Freeze | Deep Freeze | 5 | match | none |
| L020 | Spider-Man No More | Spider-Man No More | 4 | match | none |
| L021 | Amphibian Downpour | Amphibian Downpour | 4 | match | none |
| L022 | Eaten by Piranhas | Eaten by Piranhas | 4 | match | none |
| L023 | Noggle the Mind | Noggle the Mind | 4 | match | none |
| L024 | Trickster's Elk | Trickster's Elk | 0 | match | selector matched no fixture objects: Creature.EnchantedBy |
| L025 | Coerced to Kill | Coerced to Kill | 4 | match | none |
| L026 | Fowl Play | Fowl Play | 3 | match | none |
| L027 | Honest Work | Honest Work | 4 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L028 | Lignify | Lignify | 3 | match | none |
| L029 | Reprobation | Reprobation | 3 | match | none |
| L030 | Retro-Mutation | Retro-Mutation | 3 | match | Secondary is not part of the CP-LAYERS snapshot projection |
| L031 | Spark Rupture | Spark Rupture | 0 | match | selector matched no fixture objects: Planeswalker.counters_GE1_LOYALTY |
| L032 | Titania's Song | Titania's Song | 0 | match | selector matched no fixture objects: Artifact.nonCreature |
| L033 | Unable to Scream | Unable to Scream | 3 | match | none |
| L034 | Awaken the Ancient | Awaken the Ancient | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L035 | Blade of the Oni | Blade of the Oni | 0 | match | selector matched no fixture objects: Creature.EquippedBy |
| L036 | Crackling Emergence | Crackling Emergence | 0 | match | selector matched no fixture objects: Land.EnchantedBy |
| L037 | Crusher Zendikon | Crusher Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L038 | Duskmourn's Domination | Duskmourn's Domination | 3 | match | none |
| L039 | Eye of Nidhogg | Eye of Nidhogg | 4 | match | Goad is non-snapshot-visible and ignored |
| L040 | Guardian Zendikon | Guardian Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L041 | Harmonious Emergence | Harmonious Emergence | 0 | match | selector matched no fixture objects: Land.EnchantedBy |
| L042 | Stasis Field | Stasis Field | 3 | match | none |
| L043 | Wind Zendikon | Wind Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L044 | Angelic Armaments | Angelic Armaments | 4 | match | none |
| L045 | Ensoul Ring | Ensoul Ring | 4 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L046 | In Too Deep | In Too Deep | 3 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L047 | Nim Deathmantle | Nim Deathmantle | 4 | match | none |
| L048 | Sugar Coat | Sugar Coat | 3 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L049 | Katilda's Rising Dawn | Katilda, Dawnhart Martyr | 1 | match | none |
| L050 | Aerial Modification | Aerial Modification | 2 | match | selector matched no fixture objects: Vehicle.AttachedBy |
| L051 | Alien Symbiosis | Alien Symbiosis | 3 | match | MayPlay is not part of the CP-LAYERS snapshot projection; RaiseCost is not part of the CP-LAYERS snapshot projection; AffectedZone is not part of the CP-LAYERS snapshot projection |
| L052 | Bello, Bard of the Brambles | Bello, Bard of the Brambles | 0 | match | selector matched no fixture objects: Artifact.nonEquipment+YouCtrl+cmcGE4,Enchantment.nonAura+YouCtrl+cmcGE4 |
| L053 | Case of the Gorgon's Kiss | Case of the Gorgon's Kiss | 0 | match | selector matched no fixture objects: Card.Self+IsSolved |
| L054 | Demonic Embrace | Demonic Embrace | 3 | match | MayPlay is not part of the CP-LAYERS snapshot projection; RaiseCost is not part of the CP-LAYERS snapshot projection; AffectedZone is not part of the CP-LAYERS snapshot projection |
| L055 | Dragoon's Lance | Dragoon's Lance | 3 | match | none |
| L056 | Gideon Blackblade | Gideon Blackblade | 3 | match | EffectZone is not part of the CP-LAYERS snapshot projection |
| L057 | Goddric, Cloaked Reveler | Goddric, Cloaked Reveler | 0 | match | condition skipped: Card.Self |
| L058 | Grand Master of Flowers | Grand Master of Flowers | 0 | match | condition skipped: Card.Self |
| L059 | Idol of False Gods | Idol of False Gods | 0 | match | selector matched no fixture objects: Card.Self+counters_GE8_P1P1 |
| L060 | Kaito, Bane of Nightmares | Kaito, Bane of Nightmares | 0 | match | selector matched no fixture objects: Permanent.Self+counters_GE1_LOYALTY |
| L061 | Luxior and Shadowspear | Luxior and Shadowspear | 3 | match | none |
| L062 | Natural Emergence | Natural Emergence | 0 | match | selector matched no fixture objects: Land.YouCtrl |
| L063 | Nissa's Zendikon | Nissa's Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L064 | Oni Possession | Oni Possession | 3 | match | none |
| L065 | Siege Modification | Siege Modification | 2 | match | selector matched no fixture objects: Vehicle.AttachedBy |
| L066 | Sigarda's Summons | Sigarda's Summons | 0 | match | selector matched no fixture objects: Creature.YouCtrl+counters_GE1_P1P1 |
| L067 | Warden of the Wall | Warden of the Wall | 0 | match | condition skipped: Card.Self |
| L068 | Ambush Commander | Ambush Commander | 0 | match | selector matched no fixture objects: Forest.YouCtrl |
| L069 | Corrupted Zendikon | Corrupted Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L070 | Dralnu's Crusade | Dralnu's Crusade | 0 | match | selector matched no fixture objects: Creature.Goblin; selector matched no fixture objects: Goblin |
| L071 | Echo Mage | Echo Mage | 0 | match | condition skipped: Card.Self; condition skipped: Card.Self |
| L072 | Hypnotic Siren | Hypnotic Siren | 0 | match | selector matched no fixture objects: Card.EnchantedBy; selector matched no fixture objects: Card.EnchantedBy |
| L073 | Kormus Bell | Kormus Bell | 0 | match | selector matched no fixture objects: Swamp |
| L074 | Life and Limb | Life and Limb | 0 | match | selector matched no fixture objects: Forest,Saproling |
| L075 | Living Terrain | Living Terrain | 0 | match | selector matched no fixture objects: Card.EnchantedBy |
| L076 | Overwhelming Splendor | Overwhelming Splendor | 0 | match | selector matched no fixture objects: Creature.EnchantedPlayerCtrl |
| L077 | Poppet Factory | Poppet Stitcher | 0 | match | none |
| L078 | Slivdrazi Monstrosity | Slivdrazi Monstrosity | 3 | match | none |
| L079 | Sludge Monster | Sludge Monster | 0 | match | selector matched no fixture objects: Creature.nonHorror+counters_GE1_SLIME |
| L080 | Spirit Away | Spirit Away | 3 | match | none |
| L081 | Utter Insignificance | Utter Insignificance | 2 | match | none |
| L082 | Vastwood Zendikon | Vastwood Zendikon | 0 | match | selector matched no fixture objects: Land.AttachedBy |
| L083 | Yavimaya's Embrace | Yavimaya's Embrace | 3 | match | none |
| L084 | Alpine Moon | Alpine Moon | 0 | match | selector matched no fixture objects: Land.NamedCard+OppCtrl |
| L085 | Angelic Destiny | Angelic Destiny | 3 | match | none |
| L086 | Bard's Bow | Bard's Bow | 3 | match | none |
| L087 | Call to Serve | Call to Serve | 3 | match | none |
| L088 | Captain's Hook | Captain's Hook | 3 | match | none |
| L089 | Dancer's Chakrams | Dancer's Chakrams | 3 | match | AddStaticAbility is not part of the CP-LAYERS snapshot projection |
| L090 | Don Andres, the Renegade | Don Andres, the Renegade | 0 | match | selector matched no fixture objects: Creature.YouDontOwn+YouCtrl |
| L091 | Draconic Destiny | Draconic Destiny | 3 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L092 | Dub | Dub | 3 | match | none |
| L093 | Inner Demon | Inner Demon | 3 | match | none |
| L094 | Lithoform Blight | Lithoform Blight | 0 | match | selector matched no fixture objects: Land.EnchantedBy |
| L095 | Minimus Containment | Minimus Containment | 2 | match | AddAbility is not part of the CP-LAYERS snapshot projection |
| L096 | On Serra's Wings | On Serra's Wings | 3 | match | none |
| L097 | Paladin's Arms | Paladin's Arms | 3 | match | none |
| L098 | Raven Wings | Raven Wings | 3 | match | none |
| L099 | Samurai's Katana | Samurai's Katana | 3 | match | none |
| L100 | Sigiled Sword of Valeron | Sigiled Sword of Valeron | 3 | match | none |

## Diagnostic Counts

| Diagnostic | Count |
| --- | ---: |
| AddAbility is not part of the CP-LAYERS snapshot projection | 7 |
| selector matched no fixture objects: Land.AttachedBy | 7 |
| condition skipped: Card.Self | 5 |
| EffectZone is not part of the CP-LAYERS snapshot projection | 3 |
| selector matched no fixture objects: Card.EnchantedBy | 3 |
| selector matched no fixture objects: Land.EnchantedBy | 3 |
| AffectedZone is not part of the CP-LAYERS snapshot projection | 2 |
| MayPlay is not part of the CP-LAYERS snapshot projection | 2 |
| RaiseCost is not part of the CP-LAYERS snapshot projection | 2 |
| selector matched no fixture objects: Artifact.nonCreature | 2 |
| selector matched no fixture objects: Land.nonBasic | 2 |
| selector matched no fixture objects: Vehicle.AttachedBy | 2 |
| AddStaticAbility is not part of the CP-LAYERS snapshot projection | 1 |
| Goad is non-snapshot-visible and ignored | 1 |
| Secondary is not part of the CP-LAYERS snapshot projection | 1 |
| selector matched no fixture objects: Artifact.nonEquipment+YouCtrl+cmcGE4,Enchantment.nonAura+YouCtrl+cmcGE4 | 1 |
| selector matched no fixture objects: Card.Self+IsSolved | 1 |
| selector matched no fixture objects: Card.Self+counters_GE8_P1P1 | 1 |
| selector matched no fixture objects: Creature.EnchantedBy | 1 |
| selector matched no fixture objects: Creature.EnchantedPlayerCtrl | 1 |
| selector matched no fixture objects: Creature.EquippedBy | 1 |
| selector matched no fixture objects: Creature.Goblin | 1 |
| selector matched no fixture objects: Creature.YouCtrl+counters_GE1_P1P1 | 1 |
| selector matched no fixture objects: Creature.YouDontOwn+YouCtrl | 1 |
| selector matched no fixture objects: Creature.nonHorror+counters_GE1_SLIME | 1 |
| selector matched no fixture objects: Enchantment.nonAura+Other | 1 |
| selector matched no fixture objects: Forest,Saproling | 1 |
| selector matched no fixture objects: Forest.YouCtrl | 1 |
| selector matched no fixture objects: Goblin | 1 |
| selector matched no fixture objects: Land.NamedCard+OppCtrl | 1 |
| selector matched no fixture objects: Land.YouCtrl | 1 |
| selector matched no fixture objects: Permanent.Self+counters_GE1_LOYALTY | 1 |
| selector matched no fixture objects: Planeswalker.counters_GE1_LOYALTY | 1 |
| selector matched no fixture objects: Swamp | 1 |

## Gate Consequence

This replaces the earlier name-keyed fragment bridge with a stable-role importer differential for the selected 100-card CP-LAYERS subset. The legacy differential clause now has local PASS evidence: 100/100 selected scripts match the vendored legacy Java engine snapshots on stable fixture roles. CP-LAYERS still requires owner review and the explicit signoff sentence before T2.5.
