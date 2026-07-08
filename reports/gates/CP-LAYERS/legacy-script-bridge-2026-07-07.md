# CP-LAYERS Legacy Script Bridge

Date: 2026-07-07

Mode: local-only translation of the selected 100 vendored legacy Forge card scripts into the current Forge 2.0 layer-oracle vocabulary.

Result: PARTIAL.

This is not a full card compiler and not the final true engine-vs-engine differential. It is the strongest executable bridge available before implementing the real Forge 2.0 card importer/compiler: every selected script is parsed, representable continuous-effect fragments are emitted as RON scenarios, and unsupported keys remain explicit blockers.

## Counts

| Metric | Count |
| --- | ---: |
| Selected legacy scripts | 100 |
| Generated executable Forge 2.0 scenarios | 53 |
| Scripts with no representable current-model operation | 47 |
| Generated continuous-effect operations | 138 |
| Generated scenarios whose modeled fields match the legacy snapshot | 43 |
| Generated scenarios whose modeled fields differ from the legacy snapshot | 10 |

## Represented Fragment Counts

| Fragment | Count |
| --- | ---: |
| remove supported combat keywords | 32 |
| numeric set_pt | 26 |
| add supported combat keywords | 21 |
| set colors | 18 |
| set top-level types | 16 |
| numeric modify_pt | 13 |
| gain control | 5 |
| add top-level types | 4 |

## Unsupported Blocker Counts

| Blocker | Count |
| --- | ---: |
| RemoveAllAbilities$ True represented only as supported combat-keyword removal | 30 |
| creature/subtype removal is not represented | 17 |
| RemoveCardTypes$ True has no subtype/supertype fidelity | 16 |
| unsupported Affected$ predicate `Creature.EquippedBy` | 12 |
| fixture has no represented land target for `Land.AttachedBy` | 7 |
| unsupported Affected$ predicate `Card.Self` | 7 |
| dynamic or incomplete P/T modifier | 4 |
| CantHaveKeyword$ suppression is not represented | 3 |
| unsupported keyword `Defender` | 3 |
| unsupported type/subtype `Citizen` | 3 |
| unsupported type/subtype `Demon` | 3 |
| unsupported type/subtype `Spirit` | 3 |
| RemoveLandTypes$ True is not represented beyond top-level land | 2 |
| fixture has no represented land target for `Land.nonBasic` | 2 |
| unsupported Affected$ predicate `Artifact.nonCreature` | 2 |
| unsupported Affected$ predicate `Vehicle.AttachedBy` | 2 |
| unsupported type/subtype `Angel` | 2 |
| unsupported type/subtype `Dragon` | 2 |
| unsupported type/subtype `Elk` | 2 |
| unsupported type/subtype `Frog` | 2 |
| unsupported type/subtype `Treefolk` | 2 |
| fixture has no represented land target for `Land.NamedCard+OppCtrl` | 1 |
| fixture has no represented land target for `Land.YouCtrl` | 1 |
| missing Affected$ predicate | 1 |
| unsupported Affected$ predicate `Artifact.nonEquipment+YouCtrl+cmcGE4,Enchantment.nonAura+YouCtrl+cmcGE4` | 1 |
| unsupported Affected$ predicate `Card.Self+IsSolved` | 1 |
| unsupported Affected$ predicate `Card.Self+counters_GE8_P1P1` | 1 |
| unsupported Affected$ predicate `Creature.EnchantedPlayerCtrl` | 1 |
| unsupported Affected$ predicate `Creature.Goblin` | 1 |
| unsupported Affected$ predicate `Creature.YouCtrl+counters_GE1_P1P1` | 1 |
| unsupported Affected$ predicate `Creature.YouDontOwn+YouCtrl` | 1 |
| unsupported Affected$ predicate `Creature.nonHorror+counters_GE1_SLIME` | 1 |
| unsupported Affected$ predicate `Creature.token+YouCtrl` | 1 |
| unsupported Affected$ predicate `Enchantment.nonAura+Other` | 1 |
| unsupported Affected$ predicate `Forest,Saproling` | 1 |
| unsupported Affected$ predicate `Forest.YouCtrl` | 1 |
| unsupported Affected$ predicate `Goblin` | 1 |
| unsupported Affected$ predicate `Permanent.Eldrazi+YouCtrl` | 1 |
| unsupported Affected$ predicate `Permanent.EquippedBy` | 1 |
| unsupported Affected$ predicate `Permanent.Self+counters_GE1_LOYALTY` | 1 |
| unsupported Affected$ predicate `Permanent.Sliver+YouCtrl` | 1 |
| unsupported Affected$ predicate `Planeswalker.counters_GE1_LOYALTY` | 1 |
| unsupported Affected$ predicate `Swamp` | 1 |
| unsupported keyword `Hexproof` | 1 |
| unsupported keyword `Indestructible` | 1 |
| unsupported keyword `Protection:Vampire` | 1 |
| unsupported type/subtype `Assassin` | 1 |
| unsupported type/subtype `Bird` | 1 |
| unsupported type/subtype `Clue` | 1 |
| unsupported type/subtype `Coward` | 1 |
| unsupported type/subtype `Fish` | 1 |
| unsupported type/subtype `Food` | 1 |
| unsupported type/subtype `Forest` | 1 |
| unsupported type/subtype `Insect` | 1 |
| unsupported type/subtype `Knight` | 1 |
| unsupported type/subtype `Legendary` | 1 |
| unsupported type/subtype `Noggle` | 1 |
| unsupported type/subtype `Skeleton` | 1 |
| unsupported type/subtype `Symbiote` | 1 |
| unsupported type/subtype `Toy` | 1 |
| unsupported type/subtype `Treasure` | 1 |
| unsupported type/subtype `Turtle` | 1 |
| unsupported type/subtype `Wall` | 1 |

## Per-Card Bridge Status

| ID | Card | Generated | Ops | Legacy modeled fields | Unsupported summary |
| --- | --- | --- | ---: | --- | --- |
| L001 | Humility | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal |
| L002 | Opalescence | no | 0 | n/a | unsupported Affected$ predicate `Enchantment.nonAura+Other` |
| L003 | Blood Moon | no | 0 | n/a | fixture has no represented land target for `Land.nonBasic` |
| L004 | Song of the Dryads | yes | 2 | match | RemoveCardTypes$ True has no subtype/supertype fidelity; RemoveLandTypes$ True is not represented beyond top-level land; unsupported type/subtype `Forest` |
| L005 | Darksteel Mutation | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported keyword `Indestructible`; +1 more |
| L006 | Archetype of Imagination | yes | 2 | match | CantHaveKeyword$ suppression is not represented |
| L007 | Archetype of Endurance | no | 0 | n/a | CantHaveKeyword$ suppression is not represented; unsupported keyword `Hexproof` |
| L008 | Archetype of Courage | yes | 2 | match | CantHaveKeyword$ suppression is not represented |
| L009 | March of the Machines | no | 0 | n/a | unsupported Affected$ predicate `Artifact.nonCreature` |
| L010 | Magus of the Moon | no | 0 | n/a | fixture has no represented land target for `Land.nonBasic` |
| L011 | Imprisoned in the Moon | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity |
| L012 | Kenrith's Transformation | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Elk` |
| L013 | Dress Down | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal |
| L014 | Ichthyomorphosis | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Fish` |
| L015 | Witness Protection | yes | 4 | mismatch | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Citizen` |
| L016 | Kasmina's Transmutation | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal |
| L017 | Mystic Subdual | yes | 1 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; dynamic or incomplete P/T modifier |
| L018 | Frogify | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Frog` |
| L019 | Deep Freeze | yes | 3 | mismatch | RemoveAllAbilities$ True represented only as supported combat-keyword removal; unsupported keyword `Defender`; unsupported type/subtype `Wall` |
| L020 | Spider-Man No More | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; creature/subtype removal is not represented; unsupported keyword `Defender`; unsupported type/subtype `Citizen` |
| L021 | Amphibian Downpour | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Frog` |
| L022 | Eaten by Piranhas | yes | 4 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Skeleton` |
| L023 | Noggle the Mind | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; creature/subtype removal is not represented; unsupported type/subtype `Noggle` |
| L024 | Trickster's Elk | yes | 4 | mismatch | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Elk` |
| L025 | Coerced to Kill | yes | 3 | match | unsupported type/subtype `Assassin` |
| L026 | Fowl Play | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Bird` |
| L027 | Honest Work | yes | 2 | mismatch | RemoveAllAbilities$ True represented only as supported combat-keyword removal; creature/subtype removal is not represented; unsupported type/subtype `Citizen` |
| L028 | Lignify | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; creature/subtype removal is not represented; unsupported type/subtype `Treefolk` |
| L029 | Reprobation | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; creature/subtype removal is not represented; unsupported type/subtype `Coward` |
| L030 | Retro-Mutation | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; creature/subtype removal is not represented; unsupported type/subtype `Turtle` |
| L031 | Spark Rupture | no | 0 | n/a | unsupported Affected$ predicate `Planeswalker.counters_GE1_LOYALTY` |
| L032 | Titania's Song | no | 0 | n/a | unsupported Affected$ predicate `Artifact.nonCreature` |
| L033 | Unable to Scream | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; unsupported type/subtype `Toy` |
| L034 | Awaken the Ancient | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L035 | Blade of the Oni | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L036 | Crackling Emergence | yes | 4 | mismatch | unsupported type/subtype `Spirit` |
| L037 | Crusher Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L038 | Duskmourn's Domination | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; dynamic or incomplete P/T modifier |
| L039 | Eye of Nidhogg | yes | 3 | match | creature/subtype removal is not represented; unsupported type/subtype `Dragon` |
| L040 | Guardian Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L041 | Harmonious Emergence | yes | 4 | mismatch | unsupported type/subtype `Spirit` |
| L042 | Stasis Field | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; unsupported keyword `Defender` |
| L043 | Wind Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L044 | Angelic Armaments | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L045 | Ensoul Ring | yes | 3 | mismatch | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity |
| L046 | In Too Deep | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; unsupported type/subtype `Clue` |
| L047 | Nim Deathmantle | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L048 | Sugar Coat | yes | 3 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; unsupported type/subtype `Food` |
| L049 | Katilda's Rising Dawn | yes | 1 | mismatch | dynamic or incomplete P/T modifier; missing Affected$ predicate; unsupported keyword `Protection:Vampire` |
| L050 | Aerial Modification | yes | 2 | match | unsupported Affected$ predicate `Vehicle.AttachedBy` |
| L051 | Alien Symbiosis | yes | 2 | match | unsupported Affected$ predicate `Card.Self`; unsupported type/subtype `Symbiote` |
| L052 | Bello, Bard of the Brambles | no | 0 | n/a | unsupported Affected$ predicate `Artifact.nonEquipment+YouCtrl+cmcGE4,Enchantment.nonAura+YouCtrl+cmcGE4` |
| L053 | Case of the Gorgon's Kiss | no | 0 | n/a | unsupported Affected$ predicate `Card.Self+IsSolved` |
| L054 | Demonic Embrace | yes | 2 | match | unsupported Affected$ predicate `Card.Self`; unsupported type/subtype `Demon` |
| L055 | Dragoon's Lance | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L056 | Gideon Blackblade | no | 0 | n/a | unsupported Affected$ predicate `Card.Self` |
| L057 | Goddric, Cloaked Reveler | no | 0 | n/a | unsupported Affected$ predicate `Card.Self` |
| L058 | Grand Master of Flowers | no | 0 | n/a | unsupported Affected$ predicate `Card.Self` |
| L059 | Idol of False Gods | no | 0 | n/a | unsupported Affected$ predicate `Card.Self+counters_GE8_P1P1` |
| L060 | Kaito, Bane of Nightmares | no | 0 | n/a | unsupported Affected$ predicate `Permanent.Self+counters_GE1_LOYALTY` |
| L061 | Luxior and Shadowspear | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy`; unsupported Affected$ predicate `Permanent.EquippedBy` |
| L062 | Natural Emergence | no | 0 | n/a | fixture has no represented land target for `Land.YouCtrl` |
| L063 | Nissa's Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L064 | Oni Possession | yes | 2 | match | creature/subtype removal is not represented; unsupported type/subtype `Demon`; unsupported type/subtype `Spirit` |
| L065 | Siege Modification | yes | 1 | match | dynamic or incomplete P/T modifier; unsupported Affected$ predicate `Vehicle.AttachedBy` |
| L066 | Sigarda's Summons | no | 0 | n/a | unsupported Affected$ predicate `Creature.YouCtrl+counters_GE1_P1P1` |
| L067 | Warden of the Wall | no | 0 | n/a | unsupported Affected$ predicate `Card.Self` |
| L068 | Ambush Commander | no | 0 | n/a | unsupported Affected$ predicate `Forest.YouCtrl` |
| L069 | Corrupted Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L070 | Dralnu's Crusade | no | 0 | n/a | unsupported Affected$ predicate `Creature.Goblin`; unsupported Affected$ predicate `Goblin` |
| L071 | Echo Mage | no | 0 | n/a | unsupported Affected$ predicate `Card.Self` |
| L072 | Hypnotic Siren | yes | 3 | mismatch | none |
| L073 | Kormus Bell | no | 0 | n/a | unsupported Affected$ predicate `Swamp` |
| L074 | Life and Limb | no | 0 | n/a | unsupported Affected$ predicate `Forest,Saproling` |
| L075 | Living Terrain | yes | 3 | mismatch | unsupported type/subtype `Treefolk` |
| L076 | Overwhelming Splendor | no | 0 | n/a | unsupported Affected$ predicate `Creature.EnchantedPlayerCtrl` |
| L077 | Poppet Factory | no | 0 | n/a | unsupported Affected$ predicate `Creature.token+YouCtrl` |
| L078 | Slivdrazi Monstrosity | no | 0 | n/a | unsupported Affected$ predicate `Permanent.Eldrazi+YouCtrl`; unsupported Affected$ predicate `Permanent.Sliver+YouCtrl` |
| L079 | Sludge Monster | no | 0 | n/a | unsupported Affected$ predicate `Creature.nonHorror+counters_GE1_SLIME` |
| L080 | Spirit Away | yes | 3 | match | none |
| L081 | Utter Insignificance | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal |
| L082 | Vastwood Zendikon | no | 0 | n/a | fixture has no represented land target for `Land.AttachedBy` |
| L083 | Yavimaya's Embrace | yes | 3 | match | none |
| L084 | Alpine Moon | no | 0 | n/a | fixture has no represented land target for `Land.NamedCard+OppCtrl` |
| L085 | Angelic Destiny | yes | 2 | match | unsupported type/subtype `Angel` |
| L086 | Bard's Bow | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L087 | Call to Serve | yes | 2 | match | unsupported type/subtype `Angel` |
| L088 | Captain's Hook | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L089 | Dancer's Chakrams | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L090 | Don Andres, the Renegade | no | 0 | n/a | unsupported Affected$ predicate `Creature.YouDontOwn+YouCtrl` |
| L091 | Draconic Destiny | yes | 2 | match | unsupported type/subtype `Dragon` |
| L092 | Dub | yes | 2 | match | unsupported type/subtype `Knight` |
| L093 | Inner Demon | yes | 2 | match | unsupported type/subtype `Demon` |
| L094 | Lithoform Blight | yes | 1 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveLandTypes$ True is not represented beyond top-level land |
| L095 | Minimus Containment | yes | 2 | match | RemoveAllAbilities$ True represented only as supported combat-keyword removal; RemoveCardTypes$ True has no subtype/supertype fidelity; unsupported type/subtype `Treasure` |
| L096 | On Serra's Wings | yes | 2 | match | unsupported type/subtype `Legendary` |
| L097 | Paladin's Arms | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L098 | Raven Wings | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L099 | Samurai's Katana | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |
| L100 | Sigiled Sword of Valeron | no | 0 | n/a | unsupported Affected$ predicate `Creature.EquippedBy` |

## Legacy Modeled-Field Mismatches

| ID | Card | First mismatches |
| --- | --- | --- |
| L015 | Witness Protection | object 0 missing from legacy snapshot |
| L019 | Deep Freeze | object 0 colors: bridge ['blue'] != legacy ['blue', 'green'] |
| L024 | Trickster's Elk | object 0 power: bridge 3 != legacy 2; object 0 toughness: bridge 3 != legacy 2 |
| L027 | Honest Work | object 0 missing from legacy snapshot |
| L036 | Crackling Emergence | object 0 colors: bridge ['red'] != legacy ['green']; object 0 keywords: bridge ['haste'] != legacy []; object 0 power: bridge 3 != legacy 2 |
| L041 | Harmonious Emergence | object 0 keywords: bridge ['haste', 'vigilance'] != legacy []; object 0 power: bridge 4 != legacy 2; object 0 toughness: bridge 5 != legacy 2 |
| L045 | Ensoul Ring | object 0 missing from legacy snapshot |
| L049 | Katilda's Rising Dawn | object 0 keywords: bridge ['flying', 'lifelink'] != legacy [] |
| L072 | Hypnotic Siren | object 0 controller: bridge 1 != legacy 0; object 0 keywords: bridge ['flying'] != legacy []; object 0 power: bridge 3 != legacy 2 |
| L075 | Living Terrain | object 0 power: bridge 5 != legacy 2; object 0 toughness: bridge 6 != legacy 2 |

## Gate Consequence

The 100 selected real legacy scripts now have an executable Forge 2.0 bridge where the current layer engine can represent their continuous-effect fragments. CP-LAYERS still remains pending for the true differential because the bridge skips or approximates predicates, subtypes/supertypes, land subtype intrinsic mana abilities, all-abilities removal outside supported combat keywords, dynamic P/T expressions, can't-have keyword suppression, copy semantics, and full card compilation.
