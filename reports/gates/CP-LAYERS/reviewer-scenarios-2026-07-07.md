# CP-LAYERS Reviewer Scenarios

Date: 2026-07-07

Status: APPROVED BY OWNER

Approval: owner approved the packet in the Codex thread on 2026-07-07 with
`approve 100 scenarios`.

Owner direction: cover all layer interactions brutally, aiming toward the same
breadth as legacy Forge. The owner wants a 100-scenario synthetic rules stress
packet rather than the original 15-scenario minimum. Rules-observable bugs are
not acceptable; only text-only or visual coloring artifacts may be tolerated if
they cannot affect gameplay.

Acceptance proposal: 100 of 100 reviewer scenarios must pass for
rules-observable behavior before CP-LAYERS can pass. Any failure that changes
characteristics, legal actions, targets, state-based actions, combat, zones,
controller, hashes, or deterministic replay blocks signoff.

Coverage note: these scenarios stress rule families. They do not by themselves
prove literal coverage for every approximately 100k card. That requires a later
corpus-driven card import/differential pass that maps real card text and IR
patterns onto these families.

Execution evidence: the approved packet was mirrored into committed RON oracle
files under `tests/oracle/reviewer_layers/` and CP-LAYERS evidence mirrors
under `reports/gates/CP-LAYERS/reviewer_oracles/`. On 2026-07-07,
`cargo run -p forge-testkit -- oracle --path tests/oracle/reviewer_layers --no-junit`
passed 100 scenarios with 0 failures.

Harness scope note: the executable scenarios prove the currently modeled layer
subset. They do not yet prove true CDA/copiable-CDA semantics, land subtypes or
intrinsic mana abilities, supertypes, all-abilities removal beyond modeled
combat keywords, legal target enumeration, or the pending 100-card legacy
differential.

## Approved Scenarios

| ID | Focus | Scenario | Expected result |
| --- | --- | --- | --- |
| R001 | Copy, base values | Copy a vanilla creature that later receives +3/+3 from a separate effect. | The copy takes only copiable/base values, not the later modifier. |
| R002 | Copy, ability exclusion | Copy a creature that gained flying from a continuous effect. | The copy does not copy flying unless the gain is part of copiable values. |
| R003 | Copy, type-changing source | Copy a noncreature artifact currently animated by a layer-4 effect. | The copy uses copiable/base type, not temporary animation. |
| R004 | Copy, CDA source | Copy a creature whose printed P/T is set by a CDA. | The copy includes the CDA-defined base P/T. |
| R005 | Copy, text-changing source | Copy an object whose rules text is changed by a layer-3 effect. | The copy keeps original copiable text, not the temporary text change. |
| R006 | Copy, color-changing source | Copy an object made another color by layer 5. | The copy keeps copiable color only. |
| R007 | Copy after all-types effect | A global effect makes all artifacts creatures; a Clone copies one. | Copy does not inherit the global animation as copiable value. |
| R008 | Copy plus later own modifier | Copy a 2/2, then apply a modifier to only the copy. | Source stays 2/2; copy receives only its own modifier. |
| R009 | Copy timestamp ordering | Copy effect and type-changing effect have different timestamps. | Copy layer applies before later layers regardless of timestamp across layers. |
| R010 | Copy determinism | Two identical copy setup orders are replayed. | Characteristics and hash are deterministic. |
| R011 | Control, target-specific | Change control of one permanent among several similar permanents. | Only the targeted permanent changes controller. |
| R012 | Control, global then specific | Global control effect applies, then a specific later control effect applies. | Specific later effect wins where both apply. |
| R013 | Control, specific then global | Specific control effect applies, then later global control effect applies. | Later global effect wins where both apply. |
| R014 | Control dependency | One control effect changes whether another applies. | Dependency order beats timestamp in the same layer. |
| R015 | Control tie | Two equal-timestamp control effects apply to one permanent. | Deterministic ID order resolves the tie. |
| R016 | Control and combat | Control change occurs before attackers are declared. | New controller determines attack legality. |
| R017 | Control and targeting | Control change affects "you control" targeting filters. | Legal targets update immediately after the control effect. |
| R018 | Control hash | Control effect registration order is replayed. | Deterministic hash captures final controller. |
| R019 | Text, simple replacement | Replace one object's rules text marker. | Only that object's effective text changes. |
| R020 | Text, timestamp | Two text-changing effects apply to one object. | Later timestamp wins where replacement conflicts. |
| R021 | Text, dependency | One text effect changes whether another text effect applies. | Dependency order is used in layer 3. |
| R022 | Text, target isolation | Targeted text change and global text change coexist. | Targeted effect does not leak to unrelated objects. |
| R023 | Text, copy exclusion | Copy an object after its text was changed. | Copy does not copy non-copiable text mutation. |
| R024 | Text, replay | Text-changing effects are replayed from log. | Same text state and hash result. |
| R025 | Type, add creature | Add creature type to a noncreature artifact. | Object becomes artifact creature without losing artifact unless set effect says so. |
| R026 | Type, set creature | Set an artifact to creature only. | Prior artifact type is removed if the set effect replaces types. |
| R027 | Type, add artifact | Add artifact to a creature. | Object becomes artifact creature and keeps creature. |
| R028 | Type, remove creature | Remove creature type from an animated object. | Object stops being creature and P/T no longer affects creature combat/SBA. |
| R029 | Type, land subtype | Set a nonbasic land subtype. | Old land subtypes are replaced as rules require. |
| R030 | Type, supertype | Add legendary to one permanent. | Only supertype changes; other characteristics remain. |
| R031 | Type, all objects | Global type effect applies to multiple object kinds. | All applicable objects change, in stable order. |
| R032 | Type, specific beats global | Global type set then later specific type set. | Later same-layer effect wins for the targeted object. |
| R033 | Type, global later | Specific type set then later global type set. | Later global same-layer effect wins where applicable. |
| R034 | Type, dependency | One type effect makes another effect applicable. | Dependent type effect is ordered after dependency source. |
| R035 | Type, nondependency | An effect in a later layer depends on a type change. | Cross-layer dependency does not reorder across layer boundaries. |
| R036 | Type and ability | Remove creature type before an ability-granting creature-only effect. | Ability grant no longer applies. |
| R037 | Type and P/T | Make a noncreature artifact a creature and set P/T. | Creature has expected P/T after layer 7. |
| R038 | Type replay | Register type effects in deterministic replay. | Final types and hash are stable. |
| R039 | Color, set mono | Set one object to green. | Object is exactly green. |
| R040 | Color, set multicolor | Set one object to multiple colors. | Object has exactly those colors. |
| R041 | Color, colorless | Make one object colorless. | Object has no colors. |
| R042 | Color, add vs set | Add a color, then set color later. | Later set replaces colors where rules require. |
| R043 | Color, global plus targeted | Global color effect and targeted color effect overlap. | Correct timestamp/dependency ordering applies. |
| R044 | Color, dependency | Color effect makes another same-layer effect applicable. | Dependency beats timestamp. |
| R045 | Color and targeting | Color change affects color-based target legality. | Legal target set updates immediately. |
| R046 | Color hash | Color effects replay. | Effective colors and hash are deterministic. |
| R047 | Ability, grant keyword | Grant flying to one creature. | Only target gains flying. |
| R048 | Ability, remove keyword | Remove flying from one creature that has flying. | Creature loses flying. |
| R049 | Ability, add then remove | Grant flying, then later remove flying. | Later remove wins for flying. |
| R050 | Ability, remove then add | Remove flying, then later grant flying. | Later grant wins. |
| R051 | Ability, remove all | Remove all abilities from a creature with multiple abilities. | All layer-6 abilities are gone unless later effects add them. |
| R052 | Ability, Humility-class | One effect removes all abilities and sets P/T while another grants ability. | Layer 6 and layer 7 order produce expected ability and P/T state. |
| R053 | Ability, global remove | Global ability removal hits multiple creatures. | Every applicable creature loses abilities. |
| R054 | Ability, target isolation | Targeted ability removal among identical objects. | No unrelated object loses abilities. |
| R055 | Ability, type applicability | Ability grant applies only while object is a creature. | Removing creature type removes the granted ability. |
| R056 | Ability, dependency | One ability effect changes whether another same-layer ability effect applies. | Dependency order applies. |
| R057 | Ability, tie | Equal timestamp ability add/remove effects conflict. | Deterministic ID order resolves tie. |
| R058 | Ability and combat | Flying gain/loss affects block legality. | Legal blockers update from effective abilities. |
| R059 | Ability and SBA | Indestructible gain/loss affects lethal damage handling if modeled. | SBA uses effective ability state. |
| R060 | Ability replay | Ability effects replay. | Effective abilities and hash are stable. |
| R061 | 7a CDA simple | Creature has CDA setting P/T from a deterministic property. | 7a sets base P/T before other P/T layers. |
| R062 | 7a CDA plus type | CDA exists on object made creature by layer 4. | CDA contributes when object is evaluated as creature. |
| R063 | 7a CDA copied | Copy a creature with a CDA. | Copy includes the CDA result as copiable where applicable. |
| R064 | 7a CDA removed ability | Ability removal interacts with CDA. | CDA treatment follows CR 613/604 rules, not ordinary ability-loss intuition. |
| R065 | 7a multiple CDA | Multiple CDA-like definitions compete. | Deterministic legal ordering/result. |
| R066 | 7a dependency | CDA changes whether later P/T effect applies. | Later P/T layer sees 7a result. |
| R067 | 7a and hash | CDA evaluation replayed. | Effective P/T and hash are stable. |
| R068 | 7a edge zero | CDA sets zero toughness. | SBA moves creature if toughness is 0 or less. |
| R069 | 7b set P/T | Set creature base P/T to 4/4. | Creature is 4/4 before modifiers. |
| R070 | 7b later set | Two set-P/T effects apply. | Later same-sublayer set wins. |
| R071 | 7b global plus specific | Global set P/T and targeted set P/T overlap. | Timestamp/dependency determines targeted result. |
| R072 | 7b set plus ability removal | Humility-style set P/T and ability removal stack. | Ability and P/T layers stay independent. |
| R073 | 7b type gating | P/T set applies only to creatures. | Removing creature type prevents P/T effect from applying. |
| R074 | 7b dependency | One set-P/T effect changes applicability of another same-sublayer effect. | Dependency order applies if same-layer criteria are met. |
| R075 | 7b negative set | Set P/T to a low or negative value where legal. | SBA uses final toughness after all P/T layers. |
| R076 | 7b replay | Set-P/T effects replay. | Final P/T and hash are stable. |
| R077 | 7c modifier | Apply +2/+2 to one creature. | Modifier applies after set effects. |
| R078 | 7c multiple modifiers | Apply +2/+2 and -1/-1. | Modifiers combine in layer 7c. |
| R079 | 7c counters | Apply counter-like P/T modifier if represented. | Counter/modifier contributes in the correct sublayer. |
| R080 | 7c global plus specific | Global buff and targeted debuff overlap. | Both apply with correct arithmetic. |
| R081 | 7c type gating | Buff applies only to creatures. | Removing creature type prevents buff. |
| R082 | 7c dependency | Modifier changes whether another modifier applies. | Same-sublayer dependency is honored. |
| R083 | 7c lethal | Modifier lowers toughness to 0 or less. | SBA moves creature to graveyard. |
| R084 | 7c replay | Modifier effects replay. | Final P/T and hash are stable. |
| R085 | 7d switch | Switch a creature's power and toughness. | Switch applies after set and modify layers. |
| R086 | 7d double switch | Two switches apply. | Double switch cancels out. |
| R087 | 7d set then switch | Set P/T then switch. | Final P/T is switched set value. |
| R088 | 7d modify then switch | Modify P/T then switch. | Final P/T is switched after arithmetic. |
| R089 | 7d lethal | Switch creates 0 or less toughness. | SBA uses switched toughness. |
| R090 | 7d replay | Switch effects replay. | Final P/T and hash are stable. |
| R091 | Dependency chain | A depends on B, B depends on C in same layer. | Topological order is deterministic and correct. |
| R092 | Dependency cycle | A and B mutually depend. | Cycle falls back to timestamp/ID without panic. |
| R093 | Dependency nonapplicable | Dependency source is not applicable to the object. | Nonapplicable effect does not reorder unrelated effects. |
| R094 | Dependency cross-layer guard | Layer 4 effect affects layer 6 effect. | Normal layer order applies; no illegal cross-layer reordering. |
| R095 | Equal timestamp ID | Multiple same-layer effects share timestamp. | Stable deterministic ID order resolves all ties. |
| R096 | Registration replay | Same effects registered in same order across runs. | Same final state and hash. |
| R097 | Registration reverse tie | Equal timestamps but reversed registration IDs. | Result changes only according to documented deterministic tie rule. |
| R098 | Mutation/query interleave | Query characteristics, mutate layer effects, query again. | No stale cached characteristics appear. |
| R099 | Legal-action update | Layer change affects a legal action such as block, attack, or target. | Legal action list updates immediately. |
| R100 | Full brutal stack | Copy, type, color, text, control, ability, P/T set, modifier, switch, dependency, and SBA all interact. | Final characteristics, zones, controller, legal actions, SBA result, and hash are deterministic and rules-correct. |

No T2.5+ work may start until these scenarios pass executable review evidence
and the owner later gives explicit CP-LAYERS signoff.
