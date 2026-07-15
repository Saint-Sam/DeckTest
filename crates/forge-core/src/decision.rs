use super::{
    Action, ActivatedAbilityId, AnnouncedTarget, AttackDeclaration, BlockDeclaration, ManaKind,
    ObjectCharacteristics, ObjectId, ObjectRecord, ObjectView, PaymentPlan, PlayerId, PlayerView,
    PlayerViewHash, SpellAlternateCost, StackEntry, StackEntryId, Step, TargetChoice, TriggerId,
    ZoneId, ZoneKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

/// Current canonical decision-context schema version.
pub const DECISION_CONTEXT_SCHEMA_VERSION: u32 = 1;

/// Stable identifier for one legal action inside a decision context.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalActionId(u128);

impl CanonicalActionId {
    /// Returns the raw stable identifier value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for CanonicalActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// Stable identifier for one complete legal decision context.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DecisionContextId(u128);

impl DecisionContextId {
    /// Returns the raw stable identifier value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for DecisionContextId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// Stable near-state key for benchmark deduplication.
///
/// The key contains exactly the actor's redacted view hash and the sorted
/// canonical legal-action IDs. Presentation groups and labels do not affect it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DecisionStateKey(u128);

impl DecisionStateKey {
    /// Returns the raw stable identifier value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for DecisionStateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// Allocation-independent identity for one normalized legal action.
///
/// This ID is evidence-only. Exact replay and action execution continue to use
/// [`CanonicalActionId`]. Duplicate normalized IDs are retained when multiple
/// strategically interchangeable objects produce the same legal choice.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NormalizedActionId(u128);

impl NormalizedActionId {
    /// Returns the raw stable identifier value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for NormalizedActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// Allocation-independent benchmark identity for one visible decision state.
///
/// Exact replay IDs deliberately retain runtime handles. This separate key
/// canonicalizes actor-visible object handles and caller-bound runtime ability,
/// trigger, and stack-entry semantics for benchmark split and leakage checks.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NormalizedBenchmarkKey(u128);

impl NormalizedBenchmarkKey {
    /// Returns the raw stable identifier value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for NormalizedBenchmarkKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeSemanticBinding {
    source: ObjectId,
    tag: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeStackBinding {
    position: u32,
    entry: StackEntry,
}

/// Stable semantic bindings for game-local runtime handles.
///
/// Production adapters bind each registered ability or trigger to its source
/// plus an immutable program identity. Missing bindings remain exact and mark
/// the resulting normalization incomplete instead of collapsing unknown
/// semantics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BenchmarkRuntimeSemantics {
    abilities: BTreeMap<ActivatedAbilityId, RuntimeSemanticBinding>,
    triggers: BTreeMap<TriggerId, RuntimeSemanticBinding>,
    stack_entries: BTreeMap<StackEntryId, RuntimeStackBinding>,
}

impl BenchmarkRuntimeSemantics {
    /// Binds one activated ability to allocation-independent program semantics.
    pub fn bind_ability(
        &mut self,
        ability: ActivatedAbilityId,
        source: ObjectId,
        semantic_identity: &[u8],
    ) {
        self.abilities.insert(
            ability,
            RuntimeSemanticBinding {
                source,
                tag: stable_hash(b"forge-benchmark-ability-v1", semantic_identity),
            },
        );
    }

    /// Binds one trigger to allocation-independent program semantics.
    pub fn bind_trigger(&mut self, trigger: TriggerId, source: ObjectId, semantic_identity: &[u8]) {
        self.triggers.insert(
            trigger,
            RuntimeSemanticBinding {
                source,
                tag: stable_hash(b"forge-benchmark-trigger-v1", semantic_identity),
            },
        );
    }

    /// Binds a stack-entry handle to its visible position in stack order.
    pub fn bind_stack_entry(&mut self, entry: StackEntry, stack_position: u32) {
        self.stack_entries.insert(
            entry.id(),
            RuntimeStackBinding {
                position: stack_position,
                entry,
            },
        );
    }
}

/// Semantic kind of a production decision prompt.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DecisionKind {
    /// London mulligan or opening-hand keep decision.
    OpeningHand,
    /// Priority action, including passing.
    Priority,
    /// Main-phase special action, activation, spell, or pass.
    MainPhase,
    /// Declare attackers.
    DeclareAttackers,
    /// Declare blockers.
    DeclareBlockers,
    /// Commander replacement-zone decision.
    CommanderZone,
    /// Spell or ability target selection.
    Target,
    /// Modal spell or ability selection.
    Mode,
    /// Variable numeric value such as X.
    NumericValue,
    /// Mana or other cost payment selection.
    Payment,
    /// Optional effect or optional cost choice.
    Optional,
    /// Trigger ordering choice.
    TriggerOrder,
    /// Choice among identities hidden from another seat.
    HiddenChoice,
    /// Library or zone search choice.
    Search,
    /// Combat-damage assignment or ordering choice.
    CombatDamage,
    /// Player concession.
    Concession,
}

impl DecisionKind {
    /// Complete schema-v1 decision-kind registry.
    pub const ALL: [Self; 16] = [
        Self::OpeningHand,
        Self::Priority,
        Self::MainPhase,
        Self::DeclareAttackers,
        Self::DeclareBlockers,
        Self::CommanderZone,
        Self::Target,
        Self::Mode,
        Self::NumericValue,
        Self::Payment,
        Self::Optional,
        Self::TriggerOrder,
        Self::HiddenChoice,
        Self::Search,
        Self::CombatDamage,
        Self::Concession,
    ];

    /// Returns the stable registry key used by adapter-coverage evidence.
    #[must_use]
    pub const fn registry_key(self) -> &'static str {
        match self {
            Self::OpeningHand => "opening_hand",
            Self::Priority => "priority",
            Self::MainPhase => "main_phase",
            Self::DeclareAttackers => "declare_attackers",
            Self::DeclareBlockers => "declare_blockers",
            Self::CommanderZone => "commander_zone",
            Self::Target => "target",
            Self::Mode => "mode",
            Self::NumericValue => "numeric_value",
            Self::Payment => "payment",
            Self::Optional => "optional",
            Self::TriggerOrder => "trigger_order",
            Self::HiddenChoice => "hidden_choice",
            Self::Search => "search",
            Self::CombatDamage => "combat_damage",
            Self::Concession => "concession",
        }
    }

    const fn canonical_code(self) -> u8 {
        match self {
            Self::OpeningHand => 0,
            Self::Priority => 1,
            Self::MainPhase => 2,
            Self::DeclareAttackers => 3,
            Self::DeclareBlockers => 4,
            Self::CommanderZone => 5,
            Self::Target => 6,
            Self::Mode => 7,
            Self::NumericValue => 8,
            Self::Payment => 9,
            Self::Optional => 10,
            Self::TriggerOrder => 11,
            Self::HiddenChoice => 12,
            Self::Search => 13,
            Self::CombatDamage => 14,
            Self::Concession => 15,
        }
    }
}

/// Typed semantic descriptor used to derive a canonical legal-action ID.
///
/// Presentation labels are deliberately absent. Object and player handles are
/// game-local typed IDs, while payments, targets, and grouped declarations are
/// encoded structurally.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionDescriptor {
    /// Pass priority.
    PassPriority,
    /// Take a London mulligan.
    TakeMulligan,
    /// Keep an opening hand and bottom the listed cards in order.
    KeepOpeningHand {
        /// Cards put on the library bottom, bottom first.
        bottom: Vec<ObjectId>,
    },
    /// Play a land.
    PlayLand {
        /// Land object in hand.
        object: ObjectId,
    },
    /// Activate a registered ability with an explicit payment.
    ActivateAbility {
        /// Source object used by presentation and auditing.
        source: ObjectId,
        /// Registered ability ID.
        ability: ActivatedAbilityId,
        /// Explicit mana payment.
        payment: PaymentPlan,
    },
    /// Activate a program-bound non-mana ability with announced choices.
    ActivateProgramAbility {
        /// Source object used by presentation and auditing.
        source: ObjectId,
        /// Registered ability ID.
        ability: ActivatedAbilityId,
        /// Explicit mana payment.
        payment: PaymentPlan,
        /// Bound targets in requirement order.
        targets: Vec<TargetChoice>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Begin a program-bound activation with registered extra costs deferred.
    BeginActivateProgramAbilityWithCosts {
        /// Source object used by presentation and auditing.
        source: ObjectId,
        /// Registered ability ID.
        ability: ActivatedAbilityId,
        /// Bound targets in requirement order.
        targets: Vec<TargetChoice>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Cast a spell with all currently bound choices.
    CastSpell {
        /// Spell object.
        object: ObjectId,
        /// Chosen mana payment.
        payment: PaymentPlan,
        /// Bound targets in requirement order.
        targets: Vec<TargetChoice>,
        /// Bound mode indexes in canonical order.
        modes: Vec<u32>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Begin casting a spell whose numeric and payment choices remain deferred.
    BeginCastSpell {
        /// Spell object.
        object: ObjectId,
        /// Targets already bound by the enclosing action-family choice.
        targets: Vec<TargetChoice>,
        /// Mode indexes already bound by the enclosing action-family choice.
        modes: Vec<u32>,
        /// Optional-effect answers already bound by the enclosing action-family choice.
        optional: Vec<bool>,
    },
    /// Begin casting a spell with a selected alternate cost and deferred payment.
    BeginCastSpellAlternate {
        /// Spell object.
        object: ObjectId,
        /// Closed alternate-cost meaning selected for this cast.
        alternate: SpellAlternateCost,
        /// Targets already bound by the enclosing action-family choice.
        targets: Vec<TargetChoice>,
        /// Mode indexes already bound by the enclosing action-family choice.
        modes: Vec<u32>,
        /// Optional-effect answers already bound by the enclosing action-family choice.
        optional: Vec<bool>,
    },
    /// Activate a program ability with grouped or divided targets.
    ActivateProgramAbilityTargetGroups {
        /// Source object used by presentation and auditing.
        source: ObjectId,
        /// Registered ability ID.
        ability: ActivatedAbilityId,
        /// Explicit mana payment.
        payment: PaymentPlan,
        /// Canonical target groups and announced allocations.
        targets: Vec<AnnouncedTarget>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Begin an extra-cost activation with grouped or divided targets.
    BeginActivateProgramAbilityWithCostsTargetGroups {
        /// Source object used by presentation and auditing.
        source: ObjectId,
        /// Registered ability ID.
        ability: ActivatedAbilityId,
        /// Canonical target groups and announced allocations.
        targets: Vec<AnnouncedTarget>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Cast a spell with grouped or divided targets.
    CastSpellTargetGroups {
        /// Spell object.
        object: ObjectId,
        /// Chosen mana payment.
        payment: PaymentPlan,
        /// Canonical target groups and announced allocations.
        targets: Vec<AnnouncedTarget>,
        /// Bound mode indexes in canonical order.
        modes: Vec<u32>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Begin a grouped-target cast with numeric and payment choices deferred.
    BeginCastSpellTargetGroups {
        /// Spell object.
        object: ObjectId,
        /// Canonical target groups and announced allocations.
        targets: Vec<AnnouncedTarget>,
        /// Bound mode indexes in canonical order.
        modes: Vec<u32>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Begin an alternate-cost cast with grouped or divided targets.
    BeginCastSpellAlternateTargetGroups {
        /// Spell object.
        object: ObjectId,
        /// Closed alternate-cost meaning selected for this cast.
        alternate: SpellAlternateCost,
        /// Canonical target groups and announced allocations.
        targets: Vec<AnnouncedTarget>,
        /// Bound mode indexes in canonical order.
        modes: Vec<u32>,
        /// Optional-effect answers in prompt order.
        optional: Vec<bool>,
    },
    /// Declare a complete attacker set.
    DeclareAttackers {
        /// Attack declarations for this option.
        attacks: Vec<AttackDeclaration>,
    },
    /// Assign one attacker while building a complete declaration hierarchically.
    AssignAttacker {
        /// Creature currently being assigned.
        attacker: ObjectId,
        /// Defending player, or `None` when this creature does not attack.
        defender: Option<PlayerId>,
    },
    /// Declare a complete blocker set.
    DeclareBlockers {
        /// Block declarations for this option.
        blocks: Vec<BlockDeclaration>,
    },
    /// Assign one blocker while building a complete declaration hierarchically.
    AssignBlocker {
        /// Creature currently being assigned.
        blocker: ObjectId,
        /// Attacker to block, or `None` when this creature does not block.
        attacker: Option<ObjectId>,
    },
    /// Move a commander to the command zone.
    MoveCommanderToCommand {
        /// Commander object.
        object: ObjectId,
    },
    /// Leave a commander in its current zone.
    LeaveCommander {
        /// Commander object.
        object: ObjectId,
        /// Zone in which the commander remains.
        zone: ZoneId,
    },
    /// Choose one target.
    ChooseTarget {
        /// Selected target.
        target: TargetChoice,
    },
    /// Extend or finish a triggered ability's grouped target announcement.
    ChooseTriggerTargetGroups {
        /// Trigger whose target announcement is being built.
        trigger: TriggerId,
        /// Canonical announcement prefix after choosing this option.
        targets: Vec<AnnouncedTarget>,
    },
    /// Choose one mode index.
    ChooseMode {
        /// Stable mode index.
        mode: u32,
    },
    /// Choose one numeric value.
    ChooseNumber {
        /// Selected value.
        value: u32,
    },
    /// Narrow a large legal numeric range hierarchically.
    ChooseNumberRange {
        /// Inclusive lower bound selected by this option.
        minimum: u32,
        /// Inclusive upper bound selected by this option.
        maximum: u32,
    },
    /// Choose one mana payment.
    ChoosePayment {
        /// Selected payment plan.
        payment: PaymentPlan,
    },
    /// Choose the exact objects paying one non-mana additional cost.
    ChooseAdditionalCost {
        /// Zero-based additional-cost slot in printed order.
        cost: u32,
        /// Exact objects selected for this cost, in canonical order.
        objects: Vec<ObjectId>,
    },
    /// Choose the exact permanents sacrificed to activate an ability.
    ChooseActivationCostObjects {
        /// Permanents selected in canonical object order.
        objects: Vec<ObjectId>,
    },
    /// Accept or decline one optional choice.
    ChooseOptional {
        /// Stable prompt index.
        prompt: u32,
        /// Whether the option is accepted.
        accept: bool,
    },
    /// Order simultaneous triggers.
    OrderTriggers {
        /// Trigger IDs in the selected order.
        triggers: Vec<TriggerId>,
    },
    /// Choose a hidden zone slot without exposing its identity.
    ChooseHiddenSlot {
        /// Hidden zone containing the slot.
        zone: ZoneId,
        /// Zero-based visible slot index.
        slot: u32,
    },
    /// Choose a known object during a search.
    ChooseSearchObject {
        /// Selected object.
        object: ObjectId,
    },
    /// Choose complete ordered object groups while resolving an effect.
    ChooseResolutionObjects {
        /// Selected objects in compiled choice-slot order.
        choices: Vec<Vec<ObjectId>>,
    },
    /// Extend the ordered combat-damage target prefix for one source.
    OrderCombatDamage {
        /// Damage source.
        source: ObjectId,
        /// Ordered targets selected so far.
        targets: Vec<TargetChoice>,
    },
    /// Narrow a large legal combat-damage amount range hierarchically.
    ChooseCombatDamageRange {
        /// Damage source.
        source: ObjectId,
        /// Damage target.
        target: TargetChoice,
        /// Inclusive lower bound selected by this option.
        minimum: u32,
        /// Inclusive upper bound selected by this option.
        maximum: u32,
    },
    /// Choose one combat-damage amount for a typed target.
    AssignCombatDamage {
        /// Damage source.
        source: ObjectId,
        /// Damage target.
        target: TargetChoice,
        /// Assigned amount.
        amount: u32,
    },
    /// Concede the game.
    Concede,
}

impl DecisionDescriptor {
    fn canonical_bytes(&self) -> Vec<u8> {
        self.encoded_bytes(None).0
    }

    fn normalized_bytes(&self, projection: &BenchmarkIdProjection) -> (Vec<u8>, bool) {
        self.encoded_bytes(Some(projection))
    }

    fn encoded_bytes(&self, projection: Option<&BenchmarkIdProjection>) -> (Vec<u8>, bool) {
        let mut bytes = projection.map_or_else(
            CanonicalDecisionBytes::default,
            CanonicalDecisionBytes::normalized,
        );
        match self {
            Self::PassPriority => bytes.u8(0),
            Self::TakeMulligan => bytes.u8(1),
            Self::KeepOpeningHand { bottom } => {
                bytes.u8(2);
                bytes.objects(bottom);
            }
            Self::PlayLand { object } => {
                bytes.u8(3);
                bytes.object(*object);
            }
            Self::ActivateAbility {
                source,
                ability,
                payment,
            } => {
                bytes.u8(4);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.payment(*payment);
            }
            Self::ActivateProgramAbility {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                bytes.u8(20);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.payment(*payment);
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginActivateProgramAbilityWithCosts {
                source,
                ability,
                targets,
                optional,
            } => {
                bytes.u8(30);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::CastSpell {
                object,
                payment,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(5);
                bytes.object(*object);
                bytes.payment(*payment);
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginCastSpell {
                object,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(26);
                bytes.object(*object);
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginCastSpellAlternate {
                object,
                alternate,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(29);
                bytes.object(*object);
                bytes.u8(alternate.canonical_code());
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::ActivateProgramAbilityTargetGroups {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                bytes.u8(32);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.payment(*payment);
                bytes.announced_targets(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginActivateProgramAbilityWithCostsTargetGroups {
                source,
                ability,
                targets,
                optional,
            } => {
                bytes.u8(33);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.announced_targets(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::CastSpellTargetGroups {
                object,
                payment,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(34);
                bytes.object(*object);
                bytes.payment(*payment);
                bytes.announced_targets(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginCastSpellTargetGroups {
                object,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(35);
                bytes.object(*object);
                bytes.announced_targets(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::BeginCastSpellAlternateTargetGroups {
                object,
                alternate,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(36);
                bytes.object(*object);
                bytes.u8(alternate.canonical_code());
                bytes.announced_targets(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
            }
            Self::DeclareAttackers { attacks } => {
                bytes.u8(6);
                bytes.u32(attacks.len() as u32);
                for attack in attacks {
                    bytes.object(attack.attacker());
                    bytes.player(attack.defending_player());
                }
            }
            Self::AssignAttacker { attacker, defender } => {
                bytes.u8(22);
                bytes.object(*attacker);
                bytes.optional_player(*defender);
            }
            Self::DeclareBlockers { blocks } => {
                bytes.u8(7);
                bytes.u32(blocks.len() as u32);
                for block in blocks {
                    bytes.object(block.blocker());
                    bytes.object(block.attacker());
                }
            }
            Self::AssignBlocker { blocker, attacker } => {
                bytes.u8(23);
                bytes.object(*blocker);
                match attacker {
                    Some(attacker) => {
                        bytes.u8(1);
                        bytes.object(*attacker);
                    }
                    None => bytes.u8(0),
                }
            }
            Self::MoveCommanderToCommand { object } => {
                bytes.u8(8);
                bytes.object(*object);
            }
            Self::LeaveCommander { object, zone } => {
                bytes.u8(9);
                bytes.object(*object);
                bytes.zone(*zone);
            }
            Self::ChooseTarget { target } => {
                bytes.u8(10);
                bytes.target(*target);
            }
            Self::ChooseTriggerTargetGroups { trigger, targets } => {
                bytes.u8(37);
                bytes.trigger(*trigger);
                bytes.announced_targets(targets);
            }
            Self::ChooseMode { mode } => {
                bytes.u8(11);
                bytes.u32(*mode);
            }
            Self::ChooseNumber { value } => {
                bytes.u8(12);
                bytes.u32(*value);
            }
            Self::ChooseNumberRange { minimum, maximum } => {
                bytes.u8(27);
                bytes.u32(*minimum);
                bytes.u32(*maximum);
            }
            Self::ChoosePayment { payment } => {
                bytes.u8(13);
                bytes.payment(*payment);
            }
            Self::ChooseAdditionalCost { cost, objects } => {
                bytes.u8(28);
                bytes.u32(*cost);
                bytes.objects(objects);
            }
            Self::ChooseActivationCostObjects { objects } => {
                bytes.u8(31);
                bytes.objects(objects);
            }
            Self::ChooseOptional { prompt, accept } => {
                bytes.u8(14);
                bytes.u32(*prompt);
                bytes.u8(u8::from(*accept));
            }
            Self::OrderTriggers { triggers } => {
                bytes.u8(15);
                bytes.u32(triggers.len() as u32);
                for trigger in triggers {
                    bytes.trigger(*trigger);
                }
            }
            Self::ChooseHiddenSlot { zone, slot } => {
                bytes.u8(16);
                bytes.zone(*zone);
                bytes.u32(*slot);
            }
            Self::ChooseSearchObject { object } => {
                bytes.u8(17);
                bytes.object(*object);
            }
            Self::ChooseResolutionObjects { choices } => {
                bytes.u8(21);
                bytes.u32(choices.len() as u32);
                for choice in choices {
                    bytes.objects(choice);
                }
            }
            Self::OrderCombatDamage { source, targets } => {
                bytes.u8(24);
                bytes.object(*source);
                bytes.u32(targets.len() as u32);
                for target in targets {
                    bytes.target(*target);
                }
            }
            Self::ChooseCombatDamageRange {
                source,
                target,
                minimum,
                maximum,
            } => {
                bytes.u8(25);
                bytes.object(*source);
                bytes.target(*target);
                bytes.u32(*minimum);
                bytes.u32(*maximum);
            }
            Self::AssignCombatDamage {
                source,
                target,
                amount,
            } => {
                bytes.u8(18);
                bytes.object(*source);
                bytes.target(*target);
                bytes.u32(*amount);
            }
            Self::Concede => bytes.u8(19),
        }
        bytes.finish_with_status()
    }

    fn widening_group(&self, concrete_id: CanonicalActionId) -> u64 {
        let mut bytes = CanonicalDecisionBytes::default();
        let target_family = match self {
            Self::ActivateProgramAbility {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                bytes.u8(20);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.payment(*payment);
                bytes.target_kinds(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::ActivateProgramAbilityTargetGroups {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                bytes.u8(32);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.payment(*payment);
                bytes.announced_target_kinds(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::BeginActivateProgramAbilityWithCostsTargetGroups {
                source,
                ability,
                targets,
                optional,
            } => {
                bytes.u8(33);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.announced_target_kinds(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::CastSpellTargetGroups {
                object,
                payment,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(34);
                bytes.object(*object);
                bytes.payment(*payment);
                bytes.announced_target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::BeginCastSpellTargetGroups {
                object,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(35);
                bytes.object(*object);
                bytes.announced_target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::BeginCastSpellAlternateTargetGroups {
                object,
                alternate,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(36);
                bytes.object(*object);
                bytes.u8(alternate.canonical_code());
                bytes.announced_target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::ChooseTriggerTargetGroups { trigger, targets } => {
                bytes.u8(37);
                bytes.trigger(*trigger);
                bytes.announced_target_kinds(targets);
                true
            }
            Self::BeginActivateProgramAbilityWithCosts {
                source,
                ability,
                targets,
                optional,
            } => {
                bytes.u8(30);
                bytes.object(*source);
                bytes.ability(*ability);
                bytes.target_kinds(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::CastSpell {
                object,
                payment,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(5);
                bytes.object(*object);
                bytes.payment(*payment);
                bytes.target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::BeginCastSpell {
                object,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(26);
                bytes.object(*object);
                bytes.target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::BeginCastSpellAlternate {
                object,
                alternate,
                targets,
                modes,
                optional,
            } => {
                bytes.u8(29);
                bytes.object(*object);
                bytes.u8(alternate.canonical_code());
                bytes.target_kinds(targets);
                bytes.u32(modes.len() as u32);
                for mode in modes {
                    bytes.u32(*mode);
                }
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
                true
            }
            Self::ChooseTarget { target } => {
                bytes.u8(10);
                bytes.u8(target.canonical_code());
                true
            }
            _ => false,
        };
        let value = if target_family {
            stable_hash(b"forge-target-family-v1", &bytes.bytes)
        } else {
            concrete_id.get()
        };
        value as u64 ^ (value >> 64) as u64
    }
}

/// One canonical option and its executable kernel-action mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionOption {
    id: CanonicalActionId,
    descriptor: DecisionDescriptor,
    actions: Vec<Action>,
}

impl DecisionOption {
    /// Creates one option from a typed descriptor and its executable actions.
    #[must_use]
    pub fn new(descriptor: DecisionDescriptor, actions: Vec<Action>) -> Self {
        let id = CanonicalActionId(stable_hash(
            b"forge-action-v1",
            &descriptor.canonical_bytes(),
        ));
        Self {
            id,
            descriptor,
            actions,
        }
    }

    /// Returns this option's canonical action ID.
    #[must_use]
    pub const fn id(&self) -> CanonicalActionId {
        self.id
    }

    /// Returns the typed semantic descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &DecisionDescriptor {
        &self.descriptor
    }

    /// Returns the production kernel actions selected by this option.
    #[must_use]
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Returns the typed target-family key used only for widening order.
    ///
    /// Target-bearing options normalize target handles while retaining the
    /// source, payment, modes, and optional answers. Every concrete option
    /// keeps its own canonical ID, legal-set membership, edge statistics, and
    /// executable action.
    #[must_use]
    pub fn widening_group(&self) -> u64 {
        self.descriptor.widening_group(self.id)
    }
}

/// Presentation grouping over canonical options.
///
/// Groups may change without changing legal action IDs. They never replace or
/// hide the complete canonical option set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionGroup {
    id: u32,
    options: Vec<CanonicalActionId>,
}

impl DecisionGroup {
    /// Creates a non-authoritative presentation group.
    #[must_use]
    pub fn new(id: u32, options: Vec<CanonicalActionId>) -> Self {
        Self { id, options }
    }

    /// Returns the presentation group ID.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the canonical options represented by this group.
    #[must_use]
    pub fn options(&self) -> &[CanonicalActionId] {
        &self.options
    }
}

#[derive(Clone, Copy)]
struct VisibleBenchmarkObject {
    zone: ZoneId,
    position: u32,
    object: ObjectRecord,
    characteristics: ObjectCharacteristics,
}

const fn zone_order_is_strategic(kind: ZoneKind) -> bool {
    matches!(
        kind,
        ZoneKind::Library | ZoneKind::Graveyard | ZoneKind::Stack
    )
}

fn benchmark_object_fingerprint(
    object: ObjectId,
    visible: &BTreeMap<ObjectId, VisibleBenchmarkObject>,
    memo: &mut BTreeMap<ObjectId, (u128, bool)>,
    visiting: &mut BTreeSet<ObjectId>,
) -> (u128, bool) {
    if let Some(fingerprint) = memo.get(&object) {
        return *fingerprint;
    }
    let Some(entry) = visible.get(&object).copied() else {
        let mut fallback = CanonicalDecisionBytes::default();
        fallback.u32(object.index() as u32);
        return (
            stable_hash(
                b"forge-benchmark-opaque-object-v1",
                &fallback.finish_with_status().0,
            ),
            false,
        );
    };
    if !visiting.insert(object) {
        let mut fallback = CanonicalDecisionBytes::default();
        fallback.u32(object.index() as u32);
        return (
            stable_hash(
                b"forge-benchmark-cyclic-object-v1",
                &fallback.finish_with_status().0,
            ),
            false,
        );
    }

    let mut normalized_record = entry.object;
    normalized_record.id = ObjectId(0);
    normalized_record.copy_source = None;
    normalized_record.attached_to = None;
    let mut base = super::Fnva64::new();
    for byte in b"forge-benchmark-object-base-v1" {
        base.write_u8(*byte);
    }
    base.write_zone_id(entry.zone);
    if zone_order_is_strategic(entry.zone.kind()) {
        base.write_u32(entry.position);
    }
    base.write_object_record(normalized_record);
    base.write_object_characteristics(entry.characteristics);

    let mut complete = true;
    let mut payload = CanonicalDecisionBytes::default();
    payload.u64(base.finish());
    for relation in [entry.object.copy_source(), entry.object.attached_to()] {
        match relation {
            Some(related) => {
                payload.u8(1);
                let (fingerprint, relation_complete) =
                    benchmark_object_fingerprint(related, visible, memo, visiting);
                payload.u128(fingerprint);
                complete &= relation_complete;
            }
            None => payload.u8(0),
        }
    }
    visiting.remove(&object);
    let fingerprint = (
        stable_hash(
            b"forge-benchmark-object-v1",
            &payload.finish_with_status().0,
        ),
        complete,
    );
    memo.insert(object, fingerprint);
    fingerprint
}

fn normalized_stack_entry_bytes(
    binding: &RuntimeStackBinding,
    projection: &BenchmarkIdProjection,
) -> (Vec<u8>, bool) {
    let entry = &binding.entry;
    let mut bytes = CanonicalDecisionBytes::normalized(projection);
    bytes.u32(binding.position);
    bytes.player(entry.controller());
    match entry.object() {
        Some(object) => {
            bytes.u8(1);
            bytes.object(object);
        }
        None => bytes.u8(0),
    }
    match entry.trigger() {
        Some(trigger) => {
            bytes.u8(1);
            bytes.trigger(trigger);
        }
        None => bytes.u8(0),
    }
    match entry.activated_ability() {
        Some(ability) => {
            bytes.u8(1);
            bytes.ability(ability);
        }
        None => bytes.u8(0),
    }
    bytes.u8(entry.kind().canonical_code());
    bytes.u32(entry.targets().len() as u32);
    for target in entry.targets() {
        let mut requirement = super::Fnva64::new();
        for byte in b"forge-benchmark-target-requirement-v1" {
            requirement.write_u8(*byte);
        }
        requirement.write_target_requirement(target.requirement());
        bytes.u64(requirement.finish());
        bytes.target(target.choice());
        match target.original_zone() {
            Some(zone) => {
                bytes.u8(1);
                bytes.zone(zone);
            }
            None => bytes.u8(0),
        }
        let mut ward = super::Fnva64::new();
        for byte in b"forge-benchmark-ward-cost-v1" {
            ward.write_u8(*byte);
        }
        ward.write_mana_cost(target.ward_cost());
        bytes.u64(ward.finish());
    }
    match entry.payment() {
        Some(payment) => {
            bytes.u8(1);
            bytes.payment(payment);
        }
        None => bytes.u8(0),
    }
    match entry.copy_info() {
        Some(copy) => {
            bytes.u8(1);
            bytes.stack_entry(copy.source_entry());
            match copy.source_object() {
                Some(object) => {
                    bytes.u8(1);
                    bytes.object(object);
                }
                None => bytes.u8(0),
            }
        }
        None => bytes.u8(0),
    }
    bytes.u8(u8::from(entry.kicked()));
    bytes.u8(u8::from(entry.flashback()));
    bytes.u8(u8::from(entry.split_second()));
    let decisions = entry.decisions();
    match decisions.mode() {
        Some(mode) => {
            bytes.u8(1);
            bytes.u32(mode);
        }
        None => bytes.u8(0),
    }
    bytes.u32(decisions.optional_choice_count() as u32);
    for index in 0..decisions.optional_choice_count() {
        bytes.u8(u8::from(decisions.optional_choice(index).unwrap_or(false)));
    }
    match decisions.alternate_cost() {
        Some(alternate) => {
            bytes.u8(1);
            bytes.u8(alternate.canonical_code());
        }
        None => bytes.u8(0),
    }
    bytes.finish_with_status()
}

fn benchmark_projection(
    view: &PlayerView,
    runtime: Option<&BenchmarkRuntimeSemantics>,
) -> (PlayerViewHash, BenchmarkIdProjection, u128, bool) {
    let mut visible = BTreeMap::new();
    for zone in view.zones() {
        for (position, object_view) in zone.objects().iter().copied().enumerate() {
            if let ObjectView::Known {
                object,
                characteristics,
            } = object_view
            {
                visible.insert(
                    object.id(),
                    VisibleBenchmarkObject {
                        zone: zone.id(),
                        position: position as u32,
                        object,
                        characteristics,
                    },
                );
            }
        }
    }

    let mut fingerprints = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut complete = true;
    for object in visible.keys().copied().collect::<Vec<_>>() {
        let (_, object_complete) =
            benchmark_object_fingerprint(object, &visible, &mut fingerprints, &mut visiting);
        complete &= object_complete;
    }

    let mut class_keys = visible
        .iter()
        .map(|(object, entry)| {
            (
                entry.zone,
                zone_order_is_strategic(entry.zone.kind()).then_some(entry.position),
                fingerprints[object].0,
            )
        })
        .collect::<Vec<_>>();
    class_keys.sort_unstable();
    class_keys.dedup();
    let classes = class_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| (key, index as u32))
        .collect::<BTreeMap<_, _>>();

    let mut projection = BenchmarkIdProjection::default();
    for (object, entry) in &visible {
        let key = (
            entry.zone,
            zone_order_is_strategic(entry.zone.kind()).then_some(entry.position),
            fingerprints[object].0,
        );
        projection.objects.insert(*object, classes[&key]);
    }

    let mut normalized_view = view.clone();
    for zone in &mut normalized_view.zones {
        for object_view in &mut zone.objects {
            if let ObjectView::Known { object, .. } = object_view {
                object.id = ObjectId(projection.objects[&object.id()]);
                object.copy_source = object
                    .copy_source()
                    .and_then(|related| projection.objects.get(&related).copied())
                    .map(ObjectId);
                object.attached_to = object
                    .attached_to()
                    .and_then(|related| projection.objects.get(&related).copied())
                    .map(ObjectId);
            }
        }
        if !zone_order_is_strategic(zone.id.kind()) {
            zone.objects.sort_by_key(|object_view| match object_view {
                ObjectView::Hidden => (0_u8, 0_u32),
                ObjectView::Known { object, .. } => (1_u8, object.id().0),
            });
        }
    }
    let normalized_view_hash = normalized_view.deterministic_hash();

    if let Some(runtime) = runtime {
        for (ability, binding) in &runtime.abilities {
            if let Some(source) = projection.objects.get(&binding.source).copied() {
                let mut payload = CanonicalDecisionBytes::default();
                payload.u128(binding.tag);
                payload.u32(source);
                projection.abilities.insert(
                    *ability,
                    stable_hash(
                        b"forge-benchmark-bound-ability-v1",
                        &payload.finish_with_status().0,
                    ),
                );
            }
        }

        for (trigger, binding) in &runtime.triggers {
            if let Some(source) = projection.objects.get(&binding.source).copied() {
                let mut payload = CanonicalDecisionBytes::default();
                payload.u128(binding.tag);
                payload.u32(source);
                projection.triggers.insert(
                    *trigger,
                    stable_hash(
                        b"forge-benchmark-bound-trigger-v1",
                        &payload.finish_with_status().0,
                    ),
                );
            }
        }
    }

    let mut ordered_stack = runtime
        .map(|runtime| runtime.stack_entries.values().collect::<Vec<_>>())
        .unwrap_or_default();
    ordered_stack.sort_by_key(|binding| binding.position);
    let unique_stack_positions = ordered_stack
        .iter()
        .map(|binding| binding.position)
        .collect::<BTreeSet<_>>()
        .len()
        == ordered_stack.len();
    complete &= unique_stack_positions;
    let mut stack_state = CanonicalDecisionBytes::default();
    stack_state.u32(ordered_stack.len() as u32);
    for binding in ordered_stack {
        let (bytes, entry_complete) = normalized_stack_entry_bytes(binding, &projection);
        complete &= entry_complete;
        let semantic_id = stable_hash(b"forge-benchmark-stack-entry-v1", &bytes);
        projection
            .stack_entries
            .insert(binding.entry.id(), semantic_id);
        stack_state.u32(binding.position);
        stack_state.u128(semantic_id);
    }
    let runtime_state_hash = stable_hash(
        b"forge-benchmark-runtime-state-v1",
        &stack_state.finish_with_status().0,
    );

    if runtime.is_none()
        && view
            .zone(ZoneId::new(None, ZoneKind::Stack))
            .is_some_and(|zone| !zone.objects().is_empty())
    {
        complete = false;
    }

    (
        normalized_view_hash,
        projection,
        runtime_state_hash,
        complete,
    )
}

/// Complete typed decision surface for one actor at one visible state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionContext {
    schema_version: u32,
    id: DecisionContextId,
    state_key: DecisionStateKey,
    normalized_benchmark_key: NormalizedBenchmarkKey,
    normalized_player_view_hash: PlayerViewHash,
    normalized_action_ids: Vec<NormalizedActionId>,
    benchmark_normalization_complete: bool,
    kind: DecisionKind,
    actor: PlayerId,
    player_view_hash: PlayerViewHash,
    turn: u32,
    step: Option<Step>,
    priority: Option<PlayerId>,
    stack_depth: u32,
    path_discriminator: Option<u64>,
    options: Vec<DecisionOption>,
    groups: Vec<DecisionGroup>,
}

impl DecisionContext {
    /// Builds and validates a canonical context from the actor's redacted view.
    pub fn new(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
    ) -> Result<Self, DecisionContextError> {
        Self::build(kind, actor, view, options, groups, None, None)
    }

    /// Builds a context with stable semantics for game-local runtime handles.
    pub fn new_with_benchmark_semantics(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
        runtime: &BenchmarkRuntimeSemantics,
    ) -> Result<Self, DecisionContextError> {
        Self::build(kind, actor, view, options, groups, None, Some(runtime))
    }

    /// Builds a hierarchical subcontext bound to its prior canonical choices.
    ///
    /// The discriminator must be derived only from typed, actor-visible path
    /// state. It prevents two otherwise identical subprompts from sharing a
    /// benchmark key when they were reached through different legal choices.
    pub fn new_scoped(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
        path_discriminator: u64,
    ) -> Result<Self, DecisionContextError> {
        Self::build(
            kind,
            actor,
            view,
            options,
            groups,
            Some(path_discriminator),
            None,
        )
    }

    /// Builds a hierarchical context with stable runtime-handle semantics.
    pub fn new_scoped_with_benchmark_semantics(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
        path_discriminator: u64,
        runtime: &BenchmarkRuntimeSemantics,
    ) -> Result<Self, DecisionContextError> {
        Self::build(
            kind,
            actor,
            view,
            options,
            groups,
            Some(path_discriminator),
            Some(runtime),
        )
    }

    fn build(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        mut options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
        path_discriminator: Option<u64>,
        runtime: Option<&BenchmarkRuntimeSemantics>,
    ) -> Result<Self, DecisionContextError> {
        if view.observer() != actor {
            return Err(DecisionContextError::ObserverMismatch {
                actor,
                observer: view.observer(),
            });
        }
        if options.is_empty() {
            return Err(DecisionContextError::NoLegalOptions);
        }
        options.sort_by_key(DecisionOption::id);
        for pair in options.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(DecisionContextError::DuplicateActionId(pair[0].id()));
            }
        }
        let legal = options
            .iter()
            .map(DecisionOption::id)
            .collect::<BTreeSet<_>>();
        for group in &groups {
            for option in group.options() {
                if !legal.contains(option) {
                    return Err(DecisionContextError::UnknownGroupedAction(*option));
                }
            }
        }
        let stack_depth = view
            .zone(ZoneId::new(None, ZoneKind::Stack))
            .map_or(0, |zone| zone.objects().len() as u32);
        let player_view_hash = view.deterministic_hash();
        let mut canonical = CanonicalDecisionBytes::default();
        canonical.u32(DECISION_CONTEXT_SCHEMA_VERSION);
        canonical.u8(kind.canonical_code());
        canonical.player(actor);
        canonical.u64(player_view_hash.get());
        canonical.u32(view.turn_number());
        canonical.optional_step(view.current_step());
        canonical.optional_player(view.priority_player());
        canonical.u32(stack_depth);
        canonical.u32(options.len() as u32);
        for option in &options {
            canonical.u128(option.id().get());
        }
        if let Some(discriminator) = path_discriminator {
            canonical.u8(1);
            canonical.u64(discriminator);
        }
        let id = DecisionContextId(stable_hash(b"forge-context-v1", &canonical.bytes));
        let mut state_key_bytes = CanonicalDecisionBytes::default();
        state_key_bytes.u64(player_view_hash.get());
        state_key_bytes.u32(options.len() as u32);
        for option in &options {
            state_key_bytes.u128(option.id().get());
        }
        if let Some(discriminator) = path_discriminator {
            state_key_bytes.u8(1);
            state_key_bytes.u64(discriminator);
        }
        let state_key = DecisionStateKey(stable_hash(
            b"forge-decision-state-v1",
            &state_key_bytes.bytes,
        ));

        let (
            normalized_player_view_hash,
            projection,
            runtime_state_hash,
            view_normalization_complete,
        ) = benchmark_projection(view, runtime);
        let mut benchmark_normalization_complete = view_normalization_complete;
        let mut normalized_action_ids = options
            .iter()
            .map(|option| {
                let (bytes, complete) = option.descriptor().normalized_bytes(&projection);
                benchmark_normalization_complete &= complete;
                NormalizedActionId(stable_hash(b"forge-normalized-action-v1", &bytes))
            })
            .collect::<Vec<_>>();
        normalized_action_ids.sort_unstable();
        let mut normalized = CanonicalDecisionBytes::default();
        normalized.u32(1);
        normalized.u8(kind.canonical_code());
        normalized.player(actor);
        normalized.u64(normalized_player_view_hash.get());
        normalized.u128(runtime_state_hash);
        normalized.u32(normalized_action_ids.len() as u32);
        for action in &normalized_action_ids {
            normalized.u128(action.get());
        }
        match path_discriminator {
            Some(discriminator) => {
                normalized.u8(1);
                normalized.u64(discriminator);
            }
            None => normalized.u8(0),
        }
        normalized.u8(u8::from(benchmark_normalization_complete));
        if !benchmark_normalization_complete {
            normalized.u64(player_view_hash.get());
            normalized.u32(options.len() as u32);
            for option in &options {
                normalized.u128(option.id().get());
            }
        }
        let normalized_benchmark_key = NormalizedBenchmarkKey(stable_hash(
            b"forge-normalized-benchmark-state-v1",
            &normalized.bytes,
        ));
        Ok(Self {
            schema_version: DECISION_CONTEXT_SCHEMA_VERSION,
            id,
            state_key,
            normalized_benchmark_key,
            normalized_player_view_hash,
            normalized_action_ids,
            benchmark_normalization_complete,
            kind,
            actor,
            player_view_hash,
            turn: view.turn_number(),
            step: view.current_step(),
            priority: view.priority_player(),
            stack_depth,
            path_discriminator,
            options,
            groups,
        })
    }

    /// Returns the decision schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns this full context's stable identifier.
    #[must_use]
    pub const fn id(&self) -> DecisionContextId {
        self.id
    }

    /// Returns the redacted-view plus legal-action benchmark deduplication key.
    #[must_use]
    pub const fn state_key(&self) -> DecisionStateKey {
        self.state_key
    }

    /// Returns the allocation-independent benchmark and leakage-split key.
    #[must_use]
    pub const fn normalized_benchmark_key(&self) -> NormalizedBenchmarkKey {
        self.normalized_benchmark_key
    }

    /// Returns the allocation-normalized actor-visible state hash.
    #[must_use]
    pub const fn normalized_player_view_hash(&self) -> PlayerViewHash {
        self.normalized_player_view_hash
    }

    /// Returns the sorted normalized legal-action multiset.
    #[must_use]
    pub fn normalized_action_ids(&self) -> &[NormalizedActionId] {
        &self.normalized_action_ids
    }

    /// Returns whether every referenced runtime handle was semantically bound.
    #[must_use]
    pub const fn benchmark_normalization_complete(&self) -> bool {
        self.benchmark_normalization_complete
    }

    /// Returns the semantic prompt kind.
    #[must_use]
    pub const fn kind(&self) -> DecisionKind {
        self.kind
    }

    /// Returns the player making the decision.
    #[must_use]
    pub const fn actor(&self) -> PlayerId {
        self.actor
    }

    /// Returns the canonical hash of the actor's redacted state projection.
    #[must_use]
    pub const fn player_view_hash(&self) -> PlayerViewHash {
        self.player_view_hash
    }

    /// Returns the visible turn number.
    #[must_use]
    pub const fn turn(&self) -> u32 {
        self.turn
    }

    /// Returns the visible step.
    #[must_use]
    pub const fn step(&self) -> Option<Step> {
        self.step
    }

    /// Returns the visible priority player.
    #[must_use]
    pub const fn priority(&self) -> Option<PlayerId> {
        self.priority
    }

    /// Returns the visible stack depth.
    #[must_use]
    pub const fn stack_depth(&self) -> u32 {
        self.stack_depth
    }

    /// Returns the hierarchical path binding, when this is a subcontext.
    #[must_use]
    pub const fn path_discriminator(&self) -> Option<u64> {
        self.path_discriminator
    }

    /// Returns every canonical legal option in stable ID order.
    #[must_use]
    pub fn options(&self) -> &[DecisionOption] {
        &self.options
    }

    /// Returns non-authoritative presentation groups.
    #[must_use]
    pub fn groups(&self) -> &[DecisionGroup] {
        &self.groups
    }

    /// Resolves a selected ID only when it is a member of this legal set.
    pub fn select(&self, id: CanonicalActionId) -> Result<&DecisionOption, DecisionContextError> {
        self.options
            .binary_search_by_key(&id, DecisionOption::id)
            .map(|index| &self.options[index])
            .map_err(|_| DecisionContextError::IllegalSelection(id))
    }
}

/// Fail-closed canonical decision-surface errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionContextError {
    /// The supplied projection belongs to a different player.
    ObserverMismatch {
        /// Requested actor.
        actor: PlayerId,
        /// Projection observer.
        observer: PlayerId,
    },
    /// A decision was offered with no legal options.
    NoLegalOptions,
    /// Two semantic options produced the same canonical ID.
    DuplicateActionId(CanonicalActionId),
    /// A presentation group references an option outside the legal set.
    UnknownGroupedAction(CanonicalActionId),
    /// A selected ID is not a member of this context.
    IllegalSelection(CanonicalActionId),
}

impl fmt::Display for DecisionContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObserverMismatch { actor, observer } => write!(
                formatter,
                "decision actor seat {} does not match view observer seat {}",
                actor.index() + 1,
                observer.index() + 1
            ),
            Self::NoLegalOptions => write!(formatter, "decision context has no legal options"),
            Self::DuplicateActionId(id) => write!(formatter, "duplicate canonical action ID {id}"),
            Self::UnknownGroupedAction(id) => {
                write!(
                    formatter,
                    "presentation group references unknown action {id}"
                )
            }
            Self::IllegalSelection(id) => write!(formatter, "action {id} is not legal here"),
        }
    }
}

impl Error for DecisionContextError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct BenchmarkIdProjection {
    objects: BTreeMap<ObjectId, u32>,
    abilities: BTreeMap<ActivatedAbilityId, u128>,
    triggers: BTreeMap<TriggerId, u128>,
    stack_entries: BTreeMap<StackEntryId, u128>,
}

struct CanonicalDecisionBytes<'a> {
    bytes: Vec<u8>,
    projection: Option<&'a BenchmarkIdProjection>,
    complete: bool,
}

impl Default for CanonicalDecisionBytes<'_> {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            projection: None,
            complete: true,
        }
    }
}

impl<'a> CanonicalDecisionBytes<'a> {
    fn normalized(projection: &'a BenchmarkIdProjection) -> Self {
        Self {
            bytes: Vec::new(),
            projection: Some(projection),
            complete: true,
        }
    }

    fn finish_with_status(self) -> (Vec<u8>, bool) {
        (self.bytes, self.complete)
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn player(&mut self, player: PlayerId) {
        self.u32(player.index() as u32);
    }

    fn object(&mut self, object: ObjectId) {
        match self.projection {
            Some(projection) => match projection.objects.get(&object).copied() {
                Some(value) => {
                    self.u8(0);
                    self.u32(value);
                }
                None => {
                    self.complete = false;
                    self.u8(1);
                    self.u32(object.index() as u32);
                }
            },
            None => self.u32(object.index() as u32),
        }
    }

    fn ability(&mut self, ability: ActivatedAbilityId) {
        match self.projection {
            Some(projection) => match projection.abilities.get(&ability).copied() {
                Some(value) => {
                    self.u8(0);
                    self.u128(value);
                }
                None => {
                    self.complete = false;
                    self.u8(1);
                    self.u32(ability.get());
                }
            },
            None => self.u32(ability.get()),
        }
    }

    fn trigger(&mut self, trigger: TriggerId) {
        match self.projection {
            Some(projection) => match projection.triggers.get(&trigger).copied() {
                Some(value) => {
                    self.u8(0);
                    self.u128(value);
                }
                None => {
                    self.complete = false;
                    self.u8(1);
                    self.u32(trigger.get());
                }
            },
            None => self.u32(trigger.get()),
        }
    }

    fn stack_entry(&mut self, entry: StackEntryId) {
        match self.projection {
            Some(projection) => match projection.stack_entries.get(&entry).copied() {
                Some(value) => {
                    self.u8(0);
                    self.u128(value);
                }
                None => {
                    self.complete = false;
                    self.u8(1);
                    self.u32(entry.get());
                }
            },
            None => self.u32(entry.get()),
        }
    }

    fn objects(&mut self, objects: &[ObjectId]) {
        self.u32(objects.len() as u32);
        for object in objects {
            self.object(*object);
        }
    }

    fn optional_player(&mut self, player: Option<PlayerId>) {
        match player {
            Some(player) => {
                self.u8(1);
                self.player(player);
            }
            None => self.u8(0),
        }
    }

    fn optional_step(&mut self, step: Option<Step>) {
        match step {
            Some(step) => {
                self.u8(1);
                self.u8(step.canonical_code());
            }
            None => self.u8(0),
        }
    }

    fn zone(&mut self, zone: ZoneId) {
        self.optional_player(zone.owner());
        self.u8(zone.kind().canonical_code());
    }

    fn target(&mut self, target: TargetChoice) {
        self.u8(target.canonical_code());
        match target {
            TargetChoice::Player(player) => self.player(player),
            TargetChoice::Object(object) => self.object(object),
            TargetChoice::StackEntry(entry) => self.stack_entry(entry),
        }
    }

    fn target_kinds(&mut self, targets: &[TargetChoice]) {
        self.u32(targets.len() as u32);
        for target in targets {
            self.u8(target.canonical_code());
        }
    }

    fn announced_targets(&mut self, targets: &[AnnouncedTarget]) {
        self.u32(targets.len() as u32);
        for target in targets {
            self.u8(target.group());
            self.target(target.target());
            match target.allocation() {
                Some(amount) => {
                    self.u8(1);
                    self.u32(amount);
                }
                None => self.u8(0),
            }
        }
    }

    fn announced_target_kinds(&mut self, targets: &[AnnouncedTarget]) {
        self.u32(targets.len() as u32);
        for target in targets {
            self.u8(target.group());
            self.u8(target.target().canonical_code());
            match target.allocation() {
                Some(amount) => {
                    self.u8(1);
                    self.u32(amount);
                }
                None => self.u8(0),
            }
        }
    }

    fn payment(&mut self, payment: PaymentPlan) {
        for kind in [
            ManaKind::White,
            ManaKind::Blue,
            ManaKind::Black,
            ManaKind::Red,
            ManaKind::Green,
            ManaKind::Colorless,
        ] {
            self.u32(payment.paid().get(kind));
        }
        for kind in [
            ManaKind::White,
            ManaKind::Blue,
            ManaKind::Black,
            ManaKind::Red,
            ManaKind::Green,
            ManaKind::Colorless,
        ] {
            self.u32(payment.generic_paid().get(kind));
        }
        self.u32(payment.generic_required());
        self.u32(payment.x_value());
        self.u32(payment.waste_score());
    }
}

fn stable_hash(domain: &[u8], payload: &[u8]) -> u128 {
    let mut low = 0xcbf2_9ce4_8422_2325_u64;
    let mut high = 0x8422_2325_cbf2_9ce4_u64;
    for byte in domain.iter().chain(payload) {
        low ^= u64::from(*byte);
        low = low.wrapping_mul(0x0000_0100_0000_01b3);
        high ^= u64::from(*byte).wrapping_add(0x9d);
        high = high.wrapping_mul(0x9e37_79b1_85eb_ca87);
    }
    (u128::from(high) << 64) | u128::from(low)
}

#[cfg(test)]
mod tests {
    use super::{
        BenchmarkRuntimeSemantics, CanonicalActionId, DecisionContext, DecisionContextError,
        DecisionDescriptor, DecisionKind, DecisionOption,
    };
    use crate::{
        apply, Action, AttackDeclaration, BlockDeclaration, CardId, GameState, ManaCost, ManaPool,
        Outcome, StackObjectKind, ZoneId, ZoneKind,
    };

    fn setup_view() -> (GameState, crate::PlayerId) {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected AddPlayer outcome: {other:?}"),
        };
        (state, player)
    }

    #[test]
    fn context_rejects_ids_outside_its_legal_set() {
        let (state, player) = setup_view();
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let pass = DecisionOption::new(
            DecisionDescriptor::PassPriority,
            vec![Action::PassPriority { player }],
        );
        let context = DecisionContext::new(
            DecisionKind::Priority,
            player,
            &view,
            vec![pass.clone()],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("context failed: {error}"));
        assert_eq!(context.player_view_hash(), view.deterministic_hash());
        assert_eq!(context.select(pass.id()), Ok(&pass));
        assert!(matches!(
            context.select(CanonicalActionId(1)),
            Err(DecisionContextError::IllegalSelection(_))
        ));
    }

    #[test]
    fn scoped_contexts_bind_identical_options_to_their_visible_choice_path() {
        let (state, player) = setup_view();
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let pass = DecisionOption::new(
            DecisionDescriptor::PassPriority,
            vec![Action::PassPriority { player }],
        );
        let first = DecisionContext::new_scoped(
            DecisionKind::Priority,
            player,
            &view,
            vec![pass.clone()],
            Vec::new(),
            7,
        )
        .unwrap_or_else(|error| panic!("first scoped context failed: {error}"));
        let same = DecisionContext::new_scoped(
            DecisionKind::Priority,
            player,
            &view,
            vec![pass.clone()],
            Vec::new(),
            7,
        )
        .unwrap_or_else(|error| panic!("same scoped context failed: {error}"));
        let different = DecisionContext::new_scoped(
            DecisionKind::Priority,
            player,
            &view,
            vec![pass],
            Vec::new(),
            8,
        )
        .unwrap_or_else(|error| panic!("different scoped context failed: {error}"));

        assert_eq!(first.id(), same.id());
        assert_eq!(first.state_key(), same.state_key());
        assert_eq!(
            first.normalized_benchmark_key(),
            same.normalized_benchmark_key()
        );
        assert_eq!(first.path_discriminator(), Some(7));
        assert_ne!(first.id(), different.id());
        assert_ne!(first.state_key(), different.state_key());
        assert_ne!(
            first.normalized_benchmark_key(),
            different.normalized_benchmark_key()
        );
    }

    fn create_visible_object(
        state: &mut GameState,
        player: crate::PlayerId,
        card: u32,
        zone: ZoneKind,
    ) -> crate::ObjectId {
        match apply(
            state,
            Action::CreateObject {
                card: CardId::new(card),
                owner: player,
                controller: player,
                zone: ZoneId::new(
                    matches!(
                        zone,
                        ZoneKind::Hand | ZoneKind::Library | ZoneKind::Graveyard
                    )
                    .then_some(player),
                    zone,
                ),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected object creation outcome: {other:?}"),
        }
    }

    #[test]
    fn normalized_keys_ignore_visible_object_allocation_order() {
        let (mut first_state, first_player) = setup_view();
        let first_land = create_visible_object(&mut first_state, first_player, 11, ZoneKind::Hand);
        let _first_other =
            create_visible_object(&mut first_state, first_player, 22, ZoneKind::Battlefield);

        let (mut second_state, second_player) = setup_view();
        let _second_other =
            create_visible_object(&mut second_state, second_player, 22, ZoneKind::Battlefield);
        let second_land =
            create_visible_object(&mut second_state, second_player, 11, ZoneKind::Hand);

        let first_view = first_state
            .player_view(first_player)
            .unwrap_or_else(|error| panic!("first view failed: {error:?}"));
        let second_view = second_state
            .player_view(second_player)
            .unwrap_or_else(|error| panic!("second view failed: {error:?}"));
        let first = DecisionContext::new(
            DecisionKind::MainPhase,
            first_player,
            &first_view,
            vec![DecisionOption::new(
                DecisionDescriptor::PlayLand { object: first_land },
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("first context failed: {error}"));
        let second = DecisionContext::new(
            DecisionKind::MainPhase,
            second_player,
            &second_view,
            vec![DecisionOption::new(
                DecisionDescriptor::PlayLand {
                    object: second_land,
                },
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("second context failed: {error}"));

        assert_ne!(first.id(), second.id());
        assert_ne!(first.state_key(), second.state_key());
        assert_eq!(
            first.normalized_player_view_hash(),
            second.normalized_player_view_hash()
        );
        assert_eq!(
            first.normalized_benchmark_key(),
            second.normalized_benchmark_key()
        );
        assert!(first.benchmark_normalization_complete());
        assert!(second.benchmark_normalization_complete());
    }

    #[test]
    fn normalized_keys_use_bound_ability_semantics_not_registration_order() {
        let (mut state, player) = setup_view();
        let source = create_visible_object(&mut state, player, 33, ZoneKind::Battlefield);
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let payment = state
            .payment_plans_for_player(player, ManaCost::new(0, 0, 0, 0, 0, 0))
            .unwrap_or_else(|error| panic!("payment enumeration failed: {error:?}"))
            .best()
            .unwrap_or_else(|| panic!("zero payment should exist"));
        let first_ability = crate::ActivatedAbilityId(0);
        let reordered_ability = crate::ActivatedAbilityId(7);
        let first_option = DecisionOption::new(
            DecisionDescriptor::ActivateAbility {
                source,
                ability: first_ability,
                payment,
            },
            Vec::new(),
        );
        let reordered_option = DecisionOption::new(
            DecisionDescriptor::ActivateAbility {
                source,
                ability: reordered_ability,
                payment,
            },
            Vec::new(),
        );
        let mut first_runtime = BenchmarkRuntimeSemantics::default();
        first_runtime.bind_ability(first_ability, source, b"oracle-33/mana/0");
        let mut reordered_runtime = BenchmarkRuntimeSemantics::default();
        reordered_runtime.bind_ability(reordered_ability, source, b"oracle-33/mana/0");
        let first = DecisionContext::new_with_benchmark_semantics(
            DecisionKind::MainPhase,
            player,
            &view,
            vec![first_option],
            Vec::new(),
            &first_runtime,
        )
        .unwrap_or_else(|error| panic!("first context failed: {error}"));
        let reordered = DecisionContext::new_with_benchmark_semantics(
            DecisionKind::MainPhase,
            player,
            &view,
            vec![reordered_option],
            Vec::new(),
            &reordered_runtime,
        )
        .unwrap_or_else(|error| panic!("reordered context failed: {error}"));

        assert_ne!(first.id(), reordered.id());
        assert_eq!(
            first.normalized_benchmark_key(),
            reordered.normalized_benchmark_key()
        );
        assert!(first.benchmark_normalization_complete());
        assert!(reordered.benchmark_normalization_complete());

        let mut unequal_runtime = BenchmarkRuntimeSemantics::default();
        unequal_runtime.bind_ability(reordered_ability, source, b"oracle-33/draw-card/0");
        let unequal = DecisionContext::new_with_benchmark_semantics(
            DecisionKind::MainPhase,
            player,
            &view,
            vec![DecisionOption::new(
                DecisionDescriptor::ActivateAbility {
                    source,
                    ability: reordered_ability,
                    payment,
                },
                Vec::new(),
            )],
            Vec::new(),
            &unequal_runtime,
        )
        .unwrap_or_else(|error| panic!("unequal context failed: {error}"));
        assert_ne!(
            first.normalized_benchmark_key(),
            unequal.normalized_benchmark_key()
        );
    }

    fn context_with_stack_kind(kind: StackObjectKind) -> DecisionContext {
        let (mut state, player) = setup_view();
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: player,
                },
            ),
            Outcome::Applied
        );
        assert!(matches!(
            apply(&mut state, Action::AdvanceStep),
            Outcome::StepAdvanced(_)
        ));
        let entry = match apply(
            &mut state,
            Action::PutAbilityOnStack {
                player,
                kind,
                hold_priority: true,
            },
        ) {
            Outcome::StackEntryAdded(entry) => entry,
            other => panic!("unexpected stack-entry outcome: {other:?}"),
        };
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("stack view failed: {error:?}"));
        let mut runtime = BenchmarkRuntimeSemantics::default();
        let stack_entry = state
            .stack_entries()
            .iter()
            .find(|candidate| candidate.id() == entry)
            .cloned()
            .unwrap_or_else(|| panic!("stack entry should exist"));
        runtime.bind_stack_entry(stack_entry, 0);
        DecisionContext::new_with_benchmark_semantics(
            DecisionKind::Priority,
            player,
            &view,
            vec![DecisionOption::new(
                DecisionDescriptor::PassPriority,
                vec![Action::PassPriority { player }],
            )],
            Vec::new(),
            &runtime,
        )
        .unwrap_or_else(|error| panic!("stack context failed: {error}"))
    }

    #[test]
    fn normalized_keys_include_visible_stack_semantics() {
        let activated = context_with_stack_kind(StackObjectKind::ActivatedAbility);
        let triggered = context_with_stack_kind(StackObjectKind::TriggeredAbility);

        assert_eq!(activated.state_key(), triggered.state_key());
        assert_ne!(
            activated.normalized_benchmark_key(),
            triggered.normalized_benchmark_key()
        );
        assert!(activated.benchmark_normalization_complete());
        assert!(triggered.benchmark_normalization_complete());
    }

    #[test]
    fn stable_ids_use_typed_fields_not_display_text() {
        let first = DecisionOption::new(
            DecisionDescriptor::PlayLand {
                object: crate::ObjectId(3),
            },
            Vec::new(),
        );
        let same = DecisionOption::new(
            DecisionDescriptor::PlayLand {
                object: crate::ObjectId(3),
            },
            Vec::new(),
        );
        let different = DecisionOption::new(
            DecisionDescriptor::PlayLand {
                object: crate::ObjectId(4),
            },
            Vec::new(),
        );
        assert_eq!(first.id(), same.id());
        assert_ne!(first.id(), different.id());
    }

    #[test]
    fn target_family_grouping_preserves_concrete_ids_and_typed_boundaries() {
        let (mut state, player) = setup_view();
        let opponent = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected opponent result: {other:?}"),
        };
        let spell = crate::ObjectId(7);
        let first = DecisionOption::new(
            DecisionDescriptor::BeginCastSpell {
                object: spell,
                targets: vec![crate::TargetChoice::Player(player)],
                modes: vec![1],
                optional: vec![true],
            },
            Vec::new(),
        );
        let other_player = DecisionOption::new(
            DecisionDescriptor::BeginCastSpell {
                object: spell,
                targets: vec![crate::TargetChoice::Player(opponent)],
                modes: vec![1],
                optional: vec![true],
            },
            Vec::new(),
        );
        let permanent = DecisionOption::new(
            DecisionDescriptor::BeginCastSpell {
                object: spell,
                targets: vec![crate::TargetChoice::Object(crate::ObjectId(8))],
                modes: vec![1],
                optional: vec![true],
            },
            Vec::new(),
        );
        let other_spell = DecisionOption::new(
            DecisionDescriptor::BeginCastSpell {
                object: crate::ObjectId(9),
                targets: vec![crate::TargetChoice::Player(player)],
                modes: vec![1],
                optional: vec![true],
            },
            Vec::new(),
        );

        assert_ne!(first.id(), other_player.id());
        assert_eq!(first.widening_group(), other_player.widening_group());
        assert_ne!(first.widening_group(), permanent.widening_group());
        assert_ne!(first.widening_group(), other_spell.widening_group());
    }

    #[test]
    fn concession_context_maps_to_the_typed_kernel_action() {
        let (state, player) = setup_view();
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let concede = DecisionOption::new(
            DecisionDescriptor::Concede,
            vec![Action::Concede { player }],
        );
        let context = DecisionContext::new(
            DecisionKind::Concession,
            player,
            &view,
            vec![concede.clone()],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("context failed: {error}"));

        let selected = context
            .select(concede.id())
            .unwrap_or_else(|error| panic!("selection failed: {error}"));
        assert_eq!(selected.descriptor(), &DecisionDescriptor::Concede);
        assert_eq!(selected.actions(), &[Action::Concede { player }]);
    }

    #[test]
    fn descriptor_contract_covers_every_prompt_family() {
        let (mut state, player) = setup_view();
        let opponent = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected opponent outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                &mut state,
                Action::AddManaToPool {
                    player,
                    mana: ManaPool::new(1, 1, 1, 1, 1, 3),
                },
            ),
            Outcome::Applied
        );
        let payment = state
            .payment_plans_for_player(player, ManaCost::new(1, 0, 0, 0, 0, 1))
            .unwrap_or_else(|error| panic!("payment enumeration failed: {error:?}"))
            .best()
            .unwrap_or_else(|| panic!("payment plan should exist"));
        let object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(1),
                owner: player,
                controller: player,
                zone: ZoneId::new(Some(player), ZoneKind::Hand),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected object outcome: {other:?}"),
        };
        let descriptors = [
            DecisionDescriptor::PassPriority,
            DecisionDescriptor::TakeMulligan,
            DecisionDescriptor::KeepOpeningHand {
                bottom: vec![object],
            },
            DecisionDescriptor::PlayLand { object },
            DecisionDescriptor::ActivateAbility {
                source: object,
                ability: crate::ActivatedAbilityId(0),
                payment,
            },
            DecisionDescriptor::ActivateProgramAbility {
                source: object,
                ability: crate::ActivatedAbilityId(1),
                payment,
                targets: vec![crate::TargetChoice::Player(opponent)],
                optional: vec![true],
            },
            DecisionDescriptor::BeginActivateProgramAbilityWithCosts {
                source: object,
                ability: crate::ActivatedAbilityId(2),
                targets: vec![crate::TargetChoice::Player(opponent)],
                optional: vec![false],
            },
            DecisionDescriptor::CastSpell {
                object,
                payment,
                targets: vec![crate::TargetChoice::Player(opponent)],
                modes: vec![0],
                optional: vec![true],
            },
            DecisionDescriptor::BeginCastSpell {
                object,
                targets: vec![crate::TargetChoice::Player(opponent)],
                modes: vec![0],
                optional: vec![true],
            },
            DecisionDescriptor::BeginCastSpellAlternate {
                object,
                alternate: crate::SpellAlternateCost::Overload,
                targets: Vec::new(),
                modes: Vec::new(),
                optional: vec![false],
            },
            DecisionDescriptor::ActivateProgramAbilityTargetGroups {
                source: object,
                ability: crate::ActivatedAbilityId(3),
                payment,
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Player(opponent),
                )],
                optional: Vec::new(),
            },
            DecisionDescriptor::BeginActivateProgramAbilityWithCostsTargetGroups {
                source: object,
                ability: crate::ActivatedAbilityId(4),
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Player(opponent),
                )],
                optional: Vec::new(),
            },
            DecisionDescriptor::CastSpellTargetGroups {
                object,
                payment,
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Player(opponent),
                )],
                modes: Vec::new(),
                optional: Vec::new(),
            },
            DecisionDescriptor::BeginCastSpellTargetGroups {
                object,
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Player(opponent),
                )],
                modes: Vec::new(),
                optional: Vec::new(),
            },
            DecisionDescriptor::BeginCastSpellAlternateTargetGroups {
                object,
                alternate: crate::SpellAlternateCost::Overload,
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Player(opponent),
                )],
                modes: Vec::new(),
                optional: Vec::new(),
            },
            DecisionDescriptor::ChooseActivationCostObjects {
                objects: vec![object],
            },
            DecisionDescriptor::DeclareAttackers {
                attacks: vec![AttackDeclaration::new(object, opponent)],
            },
            DecisionDescriptor::AssignAttacker {
                attacker: object,
                defender: Some(opponent),
            },
            DecisionDescriptor::DeclareBlockers {
                blocks: vec![BlockDeclaration::new(object, object)],
            },
            DecisionDescriptor::AssignBlocker {
                blocker: object,
                attacker: Some(object),
            },
            DecisionDescriptor::MoveCommanderToCommand { object },
            DecisionDescriptor::LeaveCommander {
                object,
                zone: ZoneId::new(Some(player), ZoneKind::Graveyard),
            },
            DecisionDescriptor::ChooseTarget {
                target: crate::TargetChoice::Object(object),
            },
            DecisionDescriptor::ChooseTriggerTargetGroups {
                trigger: crate::TriggerId(0),
                targets: vec![crate::AnnouncedTarget::new(
                    0,
                    crate::TargetChoice::Object(object),
                )],
            },
            DecisionDescriptor::ChooseMode { mode: 1 },
            DecisionDescriptor::ChooseNumber { value: 2 },
            DecisionDescriptor::ChooseNumberRange {
                minimum: 3,
                maximum: 8,
            },
            DecisionDescriptor::ChoosePayment { payment },
            DecisionDescriptor::ChooseAdditionalCost {
                cost: 0,
                objects: vec![object],
            },
            DecisionDescriptor::ChooseOptional {
                prompt: 0,
                accept: true,
            },
            DecisionDescriptor::OrderTriggers {
                triggers: vec![crate::TriggerId(0)],
            },
            DecisionDescriptor::ChooseHiddenSlot {
                zone: ZoneId::new(Some(player), ZoneKind::Library),
                slot: 0,
            },
            DecisionDescriptor::ChooseSearchObject { object },
            DecisionDescriptor::ChooseResolutionObjects {
                choices: vec![vec![object]],
            },
            DecisionDescriptor::OrderCombatDamage {
                source: object,
                targets: vec![crate::TargetChoice::Object(object)],
            },
            DecisionDescriptor::ChooseCombatDamageRange {
                source: object,
                target: crate::TargetChoice::Player(player),
                minimum: 0,
                maximum: 10,
            },
            DecisionDescriptor::AssignCombatDamage {
                source: object,
                target: crate::TargetChoice::Player(player),
                amount: 1,
            },
            DecisionDescriptor::Concede,
        ];
        let ids = descriptors
            .into_iter()
            .map(|descriptor| DecisionOption::new(descriptor, Vec::new()).id())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 38);
    }
}
