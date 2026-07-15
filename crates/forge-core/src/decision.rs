use super::{
    Action, ActivatedAbilityId, AttackDeclaration, BlockDeclaration, ManaKind, ObjectId,
    PaymentPlan, PlayerId, PlayerView, PlayerViewHash, SpellAlternateCost, Step, TargetChoice,
    TriggerId, ZoneId, ZoneKind,
};
use std::{collections::BTreeSet, error::Error, fmt};

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
        let mut bytes = CanonicalDecisionBytes::default();
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
                bytes.u32(ability.get());
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
                bytes.u32(ability.get());
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
                bytes.u32(ability.get());
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
                    bytes.u32(trigger.get());
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
        bytes.0
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
                bytes.u32(ability.get());
                bytes.payment(*payment);
                bytes.target_kinds(targets);
                bytes.u32(optional.len() as u32);
                for accept in optional {
                    bytes.u8(u8::from(*accept));
                }
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
                bytes.u32(ability.get());
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
            stable_hash(b"forge-target-family-v1", &bytes.0)
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

/// Complete typed decision surface for one actor at one visible state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionContext {
    schema_version: u32,
    id: DecisionContextId,
    state_key: DecisionStateKey,
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
        Self::build(kind, actor, view, options, groups, None)
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
        Self::build(kind, actor, view, options, groups, Some(path_discriminator))
    }

    fn build(
        kind: DecisionKind,
        actor: PlayerId,
        view: &PlayerView,
        mut options: Vec<DecisionOption>,
        groups: Vec<DecisionGroup>,
        path_discriminator: Option<u64>,
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
        let id = DecisionContextId(stable_hash(b"forge-context-v1", &canonical.0));
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
        let state_key =
            DecisionStateKey(stable_hash(b"forge-decision-state-v1", &state_key_bytes.0));
        Ok(Self {
            schema_version: DECISION_CONTEXT_SCHEMA_VERSION,
            id,
            state_key,
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

#[derive(Default)]
struct CanonicalDecisionBytes(Vec<u8>);

impl CanonicalDecisionBytes {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u128(&mut self, value: u128) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn player(&mut self, player: PlayerId) {
        self.u32(player.index() as u32);
    }

    fn object(&mut self, object: ObjectId) {
        self.u32(object.index() as u32);
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
            TargetChoice::StackEntry(entry) => self.u32(entry.get()),
        }
    }

    fn target_kinds(&mut self, targets: &[TargetChoice]) {
        self.u32(targets.len() as u32);
        for target in targets {
            self.u8(target.canonical_code());
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
        CanonicalActionId, DecisionContext, DecisionContextError, DecisionDescriptor, DecisionKind,
        DecisionOption,
    };
    use crate::{
        apply, Action, AttackDeclaration, BlockDeclaration, CardId, GameState, ManaCost, ManaPool,
        Outcome, ZoneId, ZoneKind,
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
        assert_eq!(first.path_discriminator(), Some(7));
        assert_ne!(first.id(), different.id());
        assert_ne!(first.state_key(), different.state_key());
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
        assert_eq!(ids.len(), 32);
    }
}
