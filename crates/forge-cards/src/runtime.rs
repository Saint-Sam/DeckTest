//! Card-definition interpreter that emits production kernel actions.
//!
//! Compilation is complete and fail-closed before execution starts. Execution
//! never mutates [`GameState`] directly; every mutation crosses [`apply`] with
//! a typed [`Action`]. This module contains operation-family logic only and
//! must not contain branches keyed by card identity or card name.

use forge_carddef::{
    AbilityDefinition, AbilityKind, CardClassification, CardDefinition, CardLayout, CardType,
    Color, Expression, ManaSymbol, Operation, Supertype,
};
use forge_core::{
    apply, AbilityPlayer, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect,
    ActivationCost, ActivationTiming, BaseCreatureCharacteristics, BaseObjectCharacteristics,
    BasicLandTypes, CardId, CreatureKeywords, GameState, ManaCost, ManaKind, ManaPool,
    ObjectColors, ObjectId, ObjectSubtype, ObjectSubtypes, ObjectSupertypes, ObjectTargetPredicate,
    ObjectTypes, Outcome, PlayerId, PlayerTargetPredicate, StackEntryId, TargetChoice,
    TargetControllerPredicate, TargetKind, TargetRequirement, TriggerCondition, TriggerDefinition,
    TriggerObjectFilter, TriggerPlayerFilter, TriggerZoneFilter, ZoneId, ZoneKind,
};
use std::{collections::BTreeMap, error::Error, fmt};

const MAX_EFFECTS: usize = 64;
const MAX_ACTIVATED_ABILITIES: usize = 16;
const MAX_TOKEN_COUNT: u32 = 64;

/// Stable reason that a definition could not compile into a complete program.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CompileDiagnosticCode {
    /// The definition is not classified as playable.
    CardNotPlayable,
    /// The card layout is not supported by the current runtime.
    CardLayout,
    /// The runtime requires exactly one face for this program kind.
    FaceCount,
    /// The top-level card-type combination is unsupported.
    CardType,
    /// Printed keyword semantics are not completely compiled.
    KeywordSemantics,
    /// An ability shape is not completely compiled.
    AbilityShape,
    /// A mana symbol cannot be represented exactly by the kernel cost model.
    ManaSymbol,
    /// An effect operation has no complete lowering.
    EffectOperation,
    /// An operation has an unsupported argument shape.
    EffectArguments,
    /// A scalar cannot be represented exactly by a production action.
    EffectAmount,
    /// A player selector cannot be bound without approximation.
    PlayerSelector,
    /// The compiled program would exceed a deterministic safety bound.
    ProgramBounds,
}

impl CompileDiagnosticCode {
    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CardNotPlayable => "card_not_playable",
            Self::CardLayout => "unsupported_card_layout",
            Self::FaceCount => "unsupported_face_count",
            Self::CardType => "unsupported_card_type",
            Self::KeywordSemantics => "unsupported_keyword_semantics",
            Self::AbilityShape => "unsupported_ability_shape",
            Self::ManaSymbol => "unsupported_mana_symbol",
            Self::EffectOperation => "unsupported_effect_operation",
            Self::EffectArguments => "unsupported_effect_arguments",
            Self::EffectAmount => "unsupported_effect_amount",
            Self::PlayerSelector => "unsupported_player_selector",
            Self::ProgramBounds => "unsupported_program_bounds",
        }
    }
}

/// One fail-closed card-program compilation diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileDiagnostic {
    code: CompileDiagnosticCode,
    path: String,
    detail: String,
}

impl CompileDiagnostic {
    fn new(
        code: CompileDiagnosticCode,
        path: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// Returns the stable diagnostic category.
    #[must_use]
    pub const fn code(&self) -> CompileDiagnosticCode {
        self.code
    }

    /// Returns the structural IR path where compilation stopped.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the human-readable diagnostic detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for CompileDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code.as_str(),
            self.path,
            self.detail
        )
    }
}

impl Error for CompileDiagnostic {}

/// Coarse lifecycle used when casting or playing a compiled definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramKind {
    /// Instant spell.
    Instant,
    /// Sorcery spell.
    Sorcery,
    /// Artifact, battle, creature, enchantment, or planeswalker spell.
    Permanent,
    /// Land played as a special action rather than cast.
    Land,
}

/// Shared runtime capability compiled from card IR.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    /// Play a land through the main-phase special action.
    LandPlay,
    /// Register and execute a fixed-output mana ability.
    ManaAbility,
    /// Pay costs and resolve a non-mana activated ability.
    ActivatedAbility,
    /// Gain life.
    GainLife,
    /// Lose life.
    LoseLife,
    /// Draw cards.
    DrawCards,
    /// Discard every card from one or more hands.
    DiscardCards,
    /// Scry and move an explicit subset to the library bottom.
    Scry,
    /// Shuffle a library.
    ShuffleLibrary,
    /// Cast and resolve an ability-free permanent spell.
    PermanentSpell,
    /// Destroy a targeted permanent.
    DestroyPermanent,
    /// Exile a targeted object.
    ExileObject,
    /// Counter a targeted stack entry.
    CounterStackEntry,
    /// Move a targeted object between explicit zones.
    MoveZone,
    /// Create one or more exact registered token templates.
    CreateToken,
    /// Search a library with an explicit validated object choice.
    SearchLibrary,
    /// Tap one or more explicit objects.
    TapObject,
}

impl Capability {
    /// Returns the stable capability identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LandPlay => "land_play",
            Self::ManaAbility => "mana_ability",
            Self::ActivatedAbility => "activated_ability",
            Self::GainLife => "gain_life",
            Self::LoseLife => "lose_life",
            Self::DrawCards => "draw_cards",
            Self::DiscardCards => "discard_cards",
            Self::Scry => "scry",
            Self::ShuffleLibrary => "shuffle_library",
            Self::PermanentSpell => "permanent_spell",
            Self::DestroyPermanent => "destroy_permanent",
            Self::ExileObject => "exile_object",
            Self::CounterStackEntry => "counter_stack_entry",
            Self::MoveZone => "move_zone",
            Self::CreateToken => "create_token",
            Self::SearchLibrary => "search_library",
            Self::TapObject => "tap_object",
        }
    }
}

/// One completely compiled fixed-output mana ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivatedAbilityProgram {
    cost: ActivationCost,
    outputs: ManaOutputChoices,
}

/// Closed legal mana outputs for one compiled mana ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManaOutputChoices {
    options: [ManaPool; 6],
    len: u8,
}

impl ManaOutputChoices {
    fn from_options(options: &[ManaPool]) -> Self {
        let mut stored = [ManaPool::empty(); 6];
        for (index, option) in options.iter().copied().enumerate() {
            stored[index] = option;
        }
        Self {
            options: stored,
            len: u8::try_from(options.len()).unwrap_or(6),
        }
    }

    /// Returns legal outputs in canonical choice order.
    #[must_use]
    pub fn options(&self) -> &[ManaPool] {
        &self.options[..usize::from(self.len)]
    }

    /// Returns whether the exact output is legal for this ability.
    #[must_use]
    pub fn contains(self, output: ManaPool) -> bool {
        self.options().contains(&output)
    }

    fn deterministic_smoke_output(self) -> ManaPool {
        self.options[0]
    }
}

impl ActivatedAbilityProgram {
    /// Returns the activation cost before binding a source object.
    #[must_use]
    pub const fn cost(self) -> ActivationCost {
        self.cost
    }

    /// Returns the deterministic mana output.
    #[must_use]
    pub const fn produces(self) -> ManaPool {
        self.outputs.options[0]
    }

    /// Returns every exact legal mana output.
    #[must_use]
    pub const fn output_choices(self) -> ManaOutputChoices {
        self.outputs
    }

    /// Binds this program to one controller and battlefield source.
    #[must_use]
    pub fn bind(self, controller: PlayerId, source: ObjectId) -> ActivatedAbilityDefinition {
        let output = self.outputs.deterministic_smoke_output();
        ActivatedAbilityDefinition::new(
            controller,
            Some(source),
            ActivationTiming::Instant,
            self.cost,
            ActivatedAbilityEffect::AddMana {
                player: AbilityPlayer::Controller,
                mana: output,
            },
        )
        .as_mana_ability()
    }

    /// Binds this program with one explicit legal output choice.
    #[must_use]
    pub fn bind_selected(
        self,
        controller: PlayerId,
        source: ObjectId,
        output: ManaPool,
    ) -> Option<ActivatedAbilityDefinition> {
        if !self.outputs.contains(output) {
            return None;
        }
        ActivatedAbilityDefinition::new(
            controller,
            Some(source),
            ActivationTiming::Instant,
            self.cost,
            ActivatedAbilityEffect::AddMana {
                player: AbilityPlayer::Controller,
                mana: output,
            },
        )
        .as_mana_ability()
        .into()
    }
}

/// A player set resolved only when a program is bound to a game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerBinding {
    /// The spell or ability controller.
    Controller,
    /// Every supplied opponent of the controller.
    Opponents,
    /// The controller followed by every supplied opponent.
    AllPlayers,
    /// One explicit player target slot.
    Target(usize),
    /// The current controller of one explicit object target slot.
    ControllerOfTargetObject(usize),
    /// The controller of one explicit stack-entry target slot.
    ControllerOfTargetStack(usize),
}

/// A nonnegative effect amount resolved during prebinding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmountProgram {
    /// Literal amount embedded in the definition.
    Literal(u32),
    /// Current power of one object target.
    PowerOfTargetObject(usize),
    /// Current number of battlefield permanents matching a closed predicate.
    CountPermanents(ObjectTargetPredicate),
}

/// One explicit hidden-zone object choice exposed by a compiled program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectChoiceRequirement {
    player: PlayerBinding,
    zone: ZoneKind,
    maximum: u32,
    required_types: ObjectTypes,
    required_any_types: ObjectTypes,
    forbidden_types: ObjectTypes,
    required_supertypes: ObjectSupertypes,
    required_land_types: BasicLandTypes,
    required_any_land_types: BasicLandTypes,
    required_subtypes: ObjectSubtypes,
}

impl ObjectChoiceRequirement {
    /// Returns the player whose zone is searched.
    #[must_use]
    pub const fn player(self) -> PlayerBinding {
        self.player
    }

    /// Returns the searched zone kind.
    #[must_use]
    pub const fn zone(self) -> ZoneKind {
        self.zone
    }

    /// Returns the maximum number of objects that may be chosen.
    #[must_use]
    pub const fn maximum(self) -> u32 {
        self.maximum
    }

    /// Returns types every chosen object must have.
    #[must_use]
    pub const fn required_types(self) -> ObjectTypes {
        self.required_types
    }

    /// Returns a type union from which every chosen object must match one.
    #[must_use]
    pub const fn required_any_types(self) -> ObjectTypes {
        self.required_any_types
    }

    /// Returns types no chosen object may have.
    #[must_use]
    pub const fn forbidden_types(self) -> ObjectTypes {
        self.forbidden_types
    }

    /// Returns supertypes every chosen object must have.
    #[must_use]
    pub const fn required_supertypes(self) -> ObjectSupertypes {
        self.required_supertypes
    }

    /// Returns basic land types every chosen object must have.
    #[must_use]
    pub const fn required_land_types(self) -> BasicLandTypes {
        self.required_land_types
    }

    /// Returns a basic-land-type union from which every chosen object must match one.
    #[must_use]
    pub const fn required_any_land_types(self) -> BasicLandTypes {
        self.required_any_land_types
    }

    /// Returns exact subtypes every chosen object must have.
    #[must_use]
    pub const fn required_subtypes(self) -> ObjectSubtypes {
        self.required_subtypes
    }
}

/// Destination semantics for objects selected by an explicit choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChosenDestination {
    /// Move into the ordinary zone.
    Zone(ZoneKind),
    /// Reorder an object already in its owner's library onto the top.
    LibraryTop,
}

/// One completely compiled effect operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectProgram {
    /// Gain life.
    GainLife {
        /// Affected players.
        players: PlayerBinding,
        /// Life amount.
        amount: AmountProgram,
    },
    /// Lose life.
    LoseLife {
        /// Affected players.
        players: PlayerBinding,
        /// Life amount.
        amount: AmountProgram,
    },
    /// Draw cards.
    DrawCards {
        /// Drawing players.
        players: PlayerBinding,
        /// Draw count.
        count: AmountProgram,
    },
    /// Discard every card from selected players' hands.
    DiscardHands {
        /// Players discarding their hands.
        players: PlayerBinding,
    },
    /// Scry with an explicit execution-time bottom choice.
    Scry {
        /// Scrying players.
        players: PlayerBinding,
        /// Inspection count.
        count: AmountProgram,
    },
    /// Shuffle a library.
    ShuffleLibrary {
        /// Players whose libraries are shuffled.
        players: PlayerBinding,
    },
    /// Destroy one targeted permanent.
    DestroyPermanent {
        /// Object target slot.
        target: usize,
    },
    /// Exile one targeted object.
    ExileObject {
        /// Object target slot.
        target: usize,
    },
    /// Counter one targeted stack entry.
    CounterStackEntry {
        /// Stack-entry target slot.
        target: usize,
    },
    /// Move one targeted object to a destination zone owned by that object.
    MoveTargetObject {
        /// Object target slot.
        target: usize,
        /// Required source zone.
        from: ZoneKind,
        /// Destination zone.
        to: ZoneKind,
    },
    /// Create one or more exact registered token templates.
    CreateTokens {
        /// Stable card identity for the token face.
        card: CardId,
        /// Token base types and colors.
        base_object: BaseObjectCharacteristics,
        /// Token base creature values.
        base_creature: Option<BaseCreatureCharacteristics>,
        /// Optional fixed-choice mana ability carried by every created token.
        mana_ability: Option<ActivatedAbilityProgram>,
        /// Number of tokens to create.
        count: AmountProgram,
        /// Token controller and owner.
        players: PlayerBinding,
    },
    /// Validate an explicit library-search choice without mutating state.
    SearchLibrary {
        /// Object-choice slot.
        choice: usize,
    },
    /// Move objects selected by a prior explicit choice.
    MoveChosenObjects {
        /// Object-choice slot.
        choice: usize,
        /// Destination semantics.
        destination: ChosenDestination,
    },
    /// Tap objects selected by a prior explicit choice.
    TapChosenObjects {
        /// Object-choice slot.
        choice: usize,
    },
}

/// One completely compiled triggered ability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggeredAbilityProgram {
    event: TriggeredEventProgram,
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
}

/// Closed event families supported by a compiled triggered ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggeredEventProgram {
    /// This permanent enters the battlefield.
    SourceEnters,
    /// This creature attacks.
    SourceAttacks,
    /// This permanent's controller begins their upkeep.
    ControllerUpkeep,
    /// This permanent's controller casts or copies a matching spell.
    ControllerCasts(ObjectTargetPredicate),
}

/// One completely compiled non-mana activated ability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatedEffectProgram {
    mana_cost: ManaCost,
    exact_payment: ManaPool,
    tap_source: bool,
    sacrifice_source: bool,
    pay_life: u32,
    timing: ActivationTiming,
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
}

/// One completely compiled additional spell cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpellAdditionalCostProgram {
    /// Discard exactly this many cards from the caster's hand.
    DiscardCards {
        /// Number of cards discarded.
        count: u32,
    },
    /// Sacrifice matching permanents controlled by the caster.
    SacrificePermanents {
        /// Number of permanents sacrificed.
        count: u32,
        /// Closed permanent predicate for every sacrificed object.
        predicate: ObjectTargetPredicate,
    },
}

impl ActivatedEffectProgram {
    /// Returns the mana portion of the activation cost.
    #[must_use]
    pub const fn mana_cost(&self) -> ManaCost {
        self.mana_cost
    }

    /// Returns one exact payment pool for deterministic smoke execution.
    #[must_use]
    pub const fn exact_payment(&self) -> ManaPool {
        self.exact_payment
    }

    /// Returns whether the source must be tapped.
    #[must_use]
    pub const fn tap_source(&self) -> bool {
        self.tap_source
    }

    /// Returns whether the source is sacrificed as a cost.
    #[must_use]
    pub const fn sacrifice_source(&self) -> bool {
        self.sacrifice_source
    }

    /// Returns life paid as an activation cost.
    #[must_use]
    pub const fn pay_life(&self) -> u32 {
        self.pay_life
    }

    /// Returns the activation timing restriction.
    #[must_use]
    pub const fn timing(&self) -> ActivationTiming {
        self.timing
    }

    /// Returns target slots chosen during activation.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns explicit hidden-zone choices used while resolving.
    #[must_use]
    pub fn object_choice_requirements(&self) -> &[ObjectChoiceRequirement] {
        &self.object_choice_requirements
    }

    /// Returns compiled effect operations in resolution order.
    #[must_use]
    pub fn effects(&self) -> &[EffectProgram] {
        &self.effects
    }

    /// Returns the number of explicit optional-effect decisions.
    #[must_use]
    pub fn optional_choice_count(&self) -> usize {
        self.optional_effect_groups.len()
    }
}

impl TriggeredAbilityProgram {
    /// Binds this source-enter trigger to its controller and source object.
    #[must_use]
    pub const fn bind(&self, controller: PlayerId, source: ObjectId) -> TriggerDefinition {
        let condition = match self.event {
            TriggeredEventProgram::SourceEnters => TriggerCondition::ObjectMoved {
                object: TriggerObjectFilter::Source,
                from: TriggerZoneFilter::Any,
                to: TriggerZoneFilter::Kind(ZoneKind::Battlefield),
            },
            TriggeredEventProgram::SourceAttacks => TriggerCondition::AttackDeclared {
                attacker: TriggerObjectFilter::Source,
            },
            TriggeredEventProgram::ControllerUpkeep => TriggerCondition::StepBeganFor {
                step: forge_core::Step::Upkeep,
                player: TriggerPlayerFilter::Controller,
            },
            TriggeredEventProgram::ControllerCasts(predicate) => {
                TriggerCondition::StackEntryAdded {
                    controller: TriggerPlayerFilter::Controller,
                    required_types: predicate.required_types(),
                    required_any_types: predicate.required_any_types(),
                    forbidden_types: predicate.forbidden_types(),
                }
            }
        };
        TriggerDefinition::new(controller, condition).with_source(source)
    }

    /// Returns the closed event family that queues this trigger.
    #[must_use]
    pub const fn event(&self) -> TriggeredEventProgram {
        self.event
    }

    /// Returns target slots chosen when this trigger is put on the stack.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns explicit hidden-zone choices used while this trigger resolves.
    #[must_use]
    pub fn object_choice_requirements(&self) -> &[ObjectChoiceRequirement] {
        &self.object_choice_requirements
    }

    /// Returns compiled effect operations in resolution order.
    #[must_use]
    pub fn effects(&self) -> &[EffectProgram] {
        &self.effects
    }

    /// Returns the number of explicit optional-effect decisions.
    #[must_use]
    pub fn optional_choice_count(&self) -> usize {
        self.optional_effect_groups.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OptionalEffectGroup {
    start: usize,
    end: usize,
}

impl EffectProgram {
    const fn capability(&self) -> Capability {
        match self {
            Self::GainLife { .. } => Capability::GainLife,
            Self::LoseLife { .. } => Capability::LoseLife,
            Self::DrawCards { .. } => Capability::DrawCards,
            Self::DiscardHands { .. } => Capability::DiscardCards,
            Self::Scry { .. } => Capability::Scry,
            Self::ShuffleLibrary { .. } => Capability::ShuffleLibrary,
            Self::DestroyPermanent { .. } => Capability::DestroyPermanent,
            Self::ExileObject { .. } => Capability::ExileObject,
            Self::CounterStackEntry { .. } => Capability::CounterStackEntry,
            Self::MoveTargetObject { .. } => Capability::MoveZone,
            Self::CreateTokens { .. } => Capability::CreateToken,
            Self::SearchLibrary { .. } => Capability::SearchLibrary,
            Self::MoveChosenObjects { .. } => Capability::MoveZone,
            Self::TapChosenObjects { .. } => Capability::TapObject,
        }
    }
}

/// Immutable, completely compiled behavior for one card definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardProgram {
    oracle_id: String,
    name: String,
    kind: ProgramKind,
    mana_cost: ManaCost,
    exact_payment: ManaPool,
    base_object: BaseObjectCharacteristics,
    base_creature: Option<BaseCreatureCharacteristics>,
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
    additional_costs: Vec<SpellAdditionalCostProgram>,
    activated_abilities: Vec<ActivatedAbilityProgram>,
    activated_effects: Vec<ActivatedEffectProgram>,
    triggered_abilities: Vec<TriggeredAbilityProgram>,
}

impl CardProgram {
    /// Returns the source Oracle identity.
    #[must_use]
    pub fn oracle_id(&self) -> &str {
        &self.oracle_id
    }

    /// Returns the source card name for diagnostics only.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the card lifecycle kind.
    #[must_use]
    pub const fn kind(&self) -> ProgramKind {
        self.kind
    }

    /// Returns the exact kernel mana cost.
    #[must_use]
    pub const fn mana_cost(&self) -> ManaCost {
        self.mana_cost
    }

    /// Returns one deterministic exact payment pool for smoke synthesis.
    #[must_use]
    pub const fn exact_payment(&self) -> ManaPool {
        self.exact_payment
    }

    /// Returns base printed types and colors for object setup.
    #[must_use]
    pub const fn base_object(&self) -> BaseObjectCharacteristics {
        self.base_object
    }

    /// Returns exact printed creature characteristics when this is a creature.
    #[must_use]
    pub const fn base_creature(&self) -> Option<BaseCreatureCharacteristics> {
        self.base_creature
    }

    /// Returns target slots in announcement order.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns explicit hidden-zone choice slots in announcement order.
    #[must_use]
    pub fn object_choice_requirements(&self) -> &[ObjectChoiceRequirement] {
        &self.object_choice_requirements
    }

    /// Returns compiled effect operations in source execution order.
    #[must_use]
    pub fn effects(&self) -> &[EffectProgram] {
        &self.effects
    }

    /// Returns the number of explicit optional-effect decisions.
    #[must_use]
    pub fn optional_choice_count(&self) -> usize {
        self.optional_effect_groups.len()
    }

    /// Returns additional spell costs in announcement order.
    #[must_use]
    pub fn additional_costs(&self) -> &[SpellAdditionalCostProgram] {
        &self.additional_costs
    }

    /// Returns completely compiled activated abilities in printed order.
    #[must_use]
    pub fn activated_abilities(&self) -> &[ActivatedAbilityProgram] {
        &self.activated_abilities
    }

    /// Returns completely compiled non-mana activated abilities in printed order.
    #[must_use]
    pub fn activated_effects(&self) -> &[ActivatedEffectProgram] {
        &self.activated_effects
    }

    /// Returns completely compiled triggered abilities in printed order.
    #[must_use]
    pub fn triggered_abilities(&self) -> &[TriggeredAbilityProgram] {
        &self.triggered_abilities
    }

    /// Returns all compiled capabilities in source execution order.
    #[must_use]
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut capabilities = Vec::with_capacity(self.effects.len() + 1);
        match self.kind {
            ProgramKind::Permanent => capabilities.push(Capability::PermanentSpell),
            ProgramKind::Land => capabilities.push(Capability::LandPlay),
            ProgramKind::Instant | ProgramKind::Sorcery => {}
        }
        if !self.activated_abilities.is_empty() {
            capabilities.push(Capability::ManaAbility);
        }
        if !self.activated_effects.is_empty() {
            capabilities.push(Capability::ActivatedAbility);
        }
        capabilities.extend(self.effects.iter().map(EffectProgram::capability));
        capabilities.extend(
            self.activated_effects
                .iter()
                .flat_map(|ability| ability.effects.iter().map(EffectProgram::capability)),
        );
        capabilities.extend(
            self.triggered_abilities
                .iter()
                .flat_map(|ability| ability.effects.iter().map(EffectProgram::capability)),
        );
        capabilities
    }
}

/// Compiles one validated definition into a complete executable program.
///
/// No game state is accepted by this function, so compilation failure cannot
/// partially mutate a game.
pub fn compile_card_program(definition: &CardDefinition) -> Result<CardProgram, CompileDiagnostic> {
    if !matches!(
        definition.status,
        CardClassification::VerifiedPlayable | CardClassification::UnverifiedPlayable
    ) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::CardNotPlayable,
            "card.status",
            format!("classification is {:?}", definition.status),
        ));
    }
    if definition.layout != CardLayout::Normal {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::CardLayout,
            "card.layout",
            format!("layout {:?} is not compiled", definition.layout),
        ));
    }
    let [face] = definition.faces.as_slice() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::FaceCount,
            "card.faces",
            format!("expected one face, found {}", definition.faces.len()),
        ));
    };
    let kind = compile_program_kind(&face.type_line.card_types)?;
    let (mana_cost, exact_payment) = compile_mana_cost(&face.mana_cost.symbols)?;
    let mana_value = compile_printed_mana_value(&face.mana_cost.symbols)?;
    let base_object = compile_base_object(
        &face.type_line.supertypes,
        &face.type_line.card_types,
        &face.type_line.subtypes,
        &face.mana_cost.symbols,
        mana_value,
    )?;
    let base_creature = compile_base_creature(
        &face.type_line.card_types,
        face.power.as_deref(),
        face.toughness.as_deref(),
        &face.keywords,
    )?;
    let mut compiler = ProgramCompiler::default();
    let mut activated_abilities =
        compile_intrinsic_basic_mana_ability(&face.type_line.supertypes, &face.type_line.subtypes)?
            .into_iter()
            .collect::<Vec<_>>();
    let mut activated_effects = Vec::new();
    let mut triggered_abilities = Vec::new();
    let mut additional_costs = Vec::new();
    let mut spell_abilities = 0_usize;
    for (index, ability) in face.abilities.iter().enumerate() {
        let path = format!("card.faces[0].abilities[{index}]");
        match ability.kind {
            AbilityKind::Spell
                if matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery)
                    && ability.event.is_none()
                    && ability.condition.is_none()
                    && ability.timing.is_none()
                    && !ability.mana_ability =>
            {
                spell_abilities = spell_abilities.saturating_add(1);
                if spell_abilities > 1 {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        path,
                        "multiple spell abilities are not compiled",
                    ));
                }
                additional_costs = compile_spell_additional_costs(ability, mana_cost, &path)?;
                compile_effect(&ability.effect, &format!("{path}.effect"), &mut compiler)?;
            }
            AbilityKind::Activated
                if matches!(kind, ProgramKind::Permanent | ProgramKind::Land) =>
            {
                if ability.mana_ability {
                    activated_abilities.push(compile_fixed_mana_ability(ability, &path)?);
                } else {
                    activated_effects.push(compile_activated_effect(ability, &path)?);
                }
            }
            AbilityKind::Triggered if matches!(kind, ProgramKind::Permanent) => {
                triggered_abilities.push(compile_triggered_ability(ability, &path)?);
            }
            _ => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    path,
                    format!(
                        "ability kind {:?} is not compiled for {kind:?}",
                        ability.kind
                    ),
                ));
            }
        }
    }
    let compiled_effect_count = triggered_abilities
        .iter()
        .map(|ability: &TriggeredAbilityProgram| ability.effects.len())
        .sum::<usize>()
        .saturating_add(
            activated_effects
                .iter()
                .map(|ability: &ActivatedEffectProgram| ability.effects.len())
                .sum::<usize>(),
        )
        .saturating_add(compiler.effects.len());
    if compiled_effect_count > MAX_EFFECTS {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            "card.faces[0].abilities[0].effect",
            format!("compiled {compiled_effect_count} effects; maximum is {MAX_EFFECTS}"),
        ));
    }
    if activated_abilities
        .len()
        .saturating_add(activated_effects.len())
        > MAX_ACTIVATED_ABILITIES
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            "card.faces[0].abilities",
            format!(
                "compiled {} activated abilities; maximum is {MAX_ACTIVATED_ABILITIES}",
                activated_abilities
                    .len()
                    .saturating_add(activated_effects.len())
            ),
        ));
    }
    if compiler.effects.is_empty() && matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            "card.faces[0].abilities[0].effect",
            "spell compiled no executable effect",
        ));
    }
    Ok(CardProgram {
        oracle_id: definition.id.as_str().to_owned(),
        name: definition.name.clone(),
        kind,
        mana_cost,
        exact_payment,
        base_object,
        base_creature,
        target_requirements: compiler
            .targets
            .into_iter()
            .map(|target| target.requirement)
            .collect(),
        object_choice_requirements: compiler
            .object_choices
            .into_iter()
            .map(|choice| choice.requirement)
            .collect(),
        effects: compiler.effects,
        optional_effect_groups: compiler.optional_effect_groups,
        additional_costs,
        activated_abilities,
        activated_effects,
        triggered_abilities,
    })
}

fn compile_triggered_ability(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<TriggeredAbilityProgram, CompileDiagnostic> {
    if !ability.costs.is_empty()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "trigger must have no costs, timing, condition, or mana flag",
        ));
    }
    let Some(event_expression) = ability.event.as_ref() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.event"),
            "trigger has no event expression",
        ));
    };
    let event = compile_trigger_event(event_expression, &format!("{path}.event"))?;

    let mut compiler = ProgramCompiler::default();
    compile_effect(&ability.effect, &format!("{path}.effect"), &mut compiler)?;
    if compiler.effects.is_empty() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.effect"),
            "trigger compiled no executable effect",
        ));
    }
    Ok(TriggeredAbilityProgram {
        event,
        target_requirements: compiler
            .targets
            .into_iter()
            .map(|target| target.requirement)
            .collect(),
        object_choice_requirements: compiler
            .object_choices
            .into_iter()
            .map(|choice| choice.requirement)
            .collect(),
        effects: compiler.effects,
        optional_effect_groups: compiler.optional_effect_groups,
    })
}

fn compile_trigger_event(
    expression: &Expression,
    path: &str,
) -> Result<TriggeredEventProgram, CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "trigger event is not an operation call",
        ));
    };

    match operation {
        Operation::EventEnters | Operation::EventAttacks => {
            let [Expression::Call {
                operation: Operation::Source,
                arguments: source_arguments,
            }] = arguments.as_slice()
            else {
                return Err(effect_arity(path, operation, "source()"));
            };
            if !source_arguments.is_empty() {
                return Err(effect_arity(
                    &format!("{path}.source"),
                    &Operation::Source,
                    "no arguments",
                ));
            }
            Ok(if matches!(operation, Operation::EventEnters) {
                TriggeredEventProgram::SourceEnters
            } else {
                TriggeredEventProgram::SourceAttacks
            })
        }
        Operation::EventUpkeep => {
            let [Expression::Call {
                operation: Operation::You,
                arguments: player_arguments,
            }] = arguments.as_slice()
            else {
                return Err(effect_arity(path, operation, "you()"));
            };
            if !player_arguments.is_empty() {
                return Err(effect_arity(
                    &format!("{path}.player"),
                    &Operation::You,
                    "no arguments",
                ));
            }
            Ok(TriggeredEventProgram::ControllerUpkeep)
        }
        Operation::EventCast => {
            let [spells, controller] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "spells(...) and you() or \"cast_or_copy:you\"",
                ));
            };
            let Expression::Call {
                operation: Operation::Spells,
                arguments: spell_arguments,
            } = spells
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    format!("{path}.spells"),
                    "event_cast first argument must be spells(...) ",
                ));
            };
            let predicate = match spell_arguments.as_slice() {
                [] => ObjectTargetPredicate::any(),
                [predicate] => compile_stack_spell_predicate(predicate, &format!("{path}.spells"))?,
                _ => {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        format!("{path}.spells"),
                        "spells() accepts at most one closed type predicate",
                    ));
                }
            };
            let controller_is_you = matches!(
                controller,
                Expression::Call {
                    operation: Operation::You,
                    arguments
                } if arguments.is_empty()
            ) || matches!(controller, Expression::Text(value) if value == "cast_or_copy:you");
            if !controller_is_you {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::PlayerSelector,
                    format!("{path}.controller"),
                    "event_cast controller must be you() or exact cast_or_copy:you",
                ));
            }
            Ok(TriggeredEventProgram::ControllerCasts(predicate))
        }
        unsupported => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            format!(
                "trigger event `{}` has no complete runtime lowering",
                unsupported.as_str()
            ),
        )),
    }
}

fn compile_base_creature(
    card_types: &[CardType],
    power: Option<&str>,
    toughness: Option<&str>,
    keywords: &[forge_carddef::KeywordId],
) -> Result<Option<BaseCreatureCharacteristics>, CompileDiagnostic> {
    if !card_types.contains(&CardType::Creature) {
        if keywords.is_empty() {
            return Ok(None);
        }
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            format!(
                "{} noncreature keyword(s) require runtime lowering",
                keywords.len()
            ),
        ));
    }

    let parse_stat = |value: Option<&str>, label: &str| {
        let Some(value) = value else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectAmount,
                format!("card.faces[0].{label}"),
                format!("creature {label} is absent"),
            ));
        };
        value.parse::<i32>().map_err(|_| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::EffectAmount,
                format!("card.faces[0].{label}"),
                format!("creature {label} `{value}` is not a fixed i32"),
            )
        })
    };

    let mut runtime_keywords = CreatureKeywords::none();
    for (index, keyword) in keywords.iter().enumerate() {
        runtime_keywords = match keyword.as_str() {
            "first_strike" => runtime_keywords.with_first_strike(),
            "double_strike" => runtime_keywords.with_double_strike(),
            "trample" => runtime_keywords.with_trample(),
            "deathtouch" => runtime_keywords.with_deathtouch(),
            "lifelink" => runtime_keywords.with_lifelink(),
            "flying" => runtime_keywords.with_flying(),
            "reach" => runtime_keywords.with_reach(),
            "menace" => runtime_keywords.with_menace(),
            "vigilance" => runtime_keywords.with_vigilance(),
            "haste" => runtime_keywords.with_haste(),
            "defender" => runtime_keywords.with_defender(),
            "indestructible" => runtime_keywords.with_indestructible(),
            "prowess" => runtime_keywords.with_prowess(),
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::KeywordSemantics,
                    format!("card.faces[0].keywords[{index}]"),
                    format!("keyword `{unsupported}` requires runtime lowering"),
                ));
            }
        };
    }

    Ok(Some(
        BaseCreatureCharacteristics::new(
            parse_stat(power, "power")?,
            parse_stat(toughness, "toughness")?,
        )
        .with_keywords(runtime_keywords),
    ))
}

#[derive(Default)]
struct ProgramCompiler {
    effects: Vec<EffectProgram>,
    targets: Vec<CompiledTarget>,
    object_choices: Vec<CompiledObjectChoice>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
}

struct CompiledTarget {
    selector: Expression,
    requirement: TargetRequirement,
}

struct CompiledObjectChoice {
    selector: Expression,
    requirement: ObjectChoiceRequirement,
}

fn compile_intrinsic_basic_mana_ability(
    supertypes: &[Supertype],
    subtypes: &[String],
) -> Result<Option<ActivatedAbilityProgram>, CompileDiagnostic> {
    if !supertypes.contains(&Supertype::Basic) {
        return Ok(None);
    }
    let mut outputs = subtypes
        .iter()
        .filter_map(|subtype| match subtype.as_str() {
            "Plains" => Some(ManaKind::White),
            "Island" => Some(ManaKind::Blue),
            "Swamp" => Some(ManaKind::Black),
            "Mountain" => Some(ManaKind::Red),
            "Forest" => Some(ManaKind::Green),
            _ => None,
        });
    let Some(kind) = outputs.next() else {
        return Ok(None);
    };
    if outputs.next().is_some() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            "card.faces[0].types",
            "multiple intrinsic basic-land mana abilities require an explicit choice",
        ));
    }
    Ok(Some(ActivatedAbilityProgram {
        cost: ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_tap_source(),
        outputs: ManaOutputChoices::from_options(&[ManaPool::of(kind, 1)]),
    }))
}

fn compile_fixed_mana_ability(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<ActivatedAbilityProgram, CompileDiagnostic> {
    if !ability.mana_ability
        || ability.event.is_some()
        || ability.condition.is_some()
        || ability.timing.is_some()
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "activated mana ability must be unconditional and marked mana_ability",
        ));
    }
    let [Expression::Call {
        operation: Operation::TapSelf,
        arguments,
    }] = ability.costs.as_slice()
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.costs"),
            "fixed mana ability currently requires exactly tap_self()",
        ));
    };
    if !arguments.is_empty() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.costs[0]"),
            "tap_self does not accept arguments",
        ));
    }
    let Expression::Call {
        operation: Operation::AddMana,
        arguments,
    } = &ability.effect
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            format!("{path}.effect"),
            "activated ability is not one fixed add_mana operation",
        ));
    };
    let [Expression::Text(mana), player] = arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect"),
            &Operation::AddMana,
            "one fixed mana string and you()",
        ));
    };
    if !matches!(
        player,
        Expression::Call {
            operation: Operation::You,
            arguments,
        } if arguments.is_empty()
    ) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::PlayerSelector,
            format!("{path}.effect.player"),
            "fixed mana ability must add mana to you()",
        ));
    }
    Ok(ActivatedAbilityProgram {
        cost: ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_tap_source(),
        outputs: compile_mana_outputs(mana, &format!("{path}.effect.mana"))?,
    })
}

fn compile_spell_additional_costs(
    ability: &AbilityDefinition,
    printed_mana_cost: ManaCost,
    path: &str,
) -> Result<Vec<SpellAdditionalCostProgram>, CompileDiagnostic> {
    let mut compiled = Vec::new();
    let mut saw_mana_cost = false;
    for (index, cost) in ability.costs.iter().enumerate() {
        let cost_path = format!("{path}.costs[{index}]");
        let Expression::Call {
            operation,
            arguments,
        } = cost
        else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                cost_path,
                "spell cost is not an operation call",
            ));
        };
        match operation {
            Operation::ManaCost => {
                if saw_mana_cost {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        cost_path,
                        "spell repeats its mana cost",
                    ));
                }
                let [Expression::Text(value)] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one fixed mana-cost string",
                    ));
                };
                let (ability_cost, _) = compile_mana_cost_text(value, &cost_path)?;
                if ability_cost != printed_mana_cost {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        cost_path,
                        "ability mana cost does not match the printed face cost",
                    ));
                }
                saw_mana_cost = true;
            }
            Operation::DiscardCost => {
                let [count, selector] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one literal count and cards()",
                    ));
                };
                if !matches!(
                    selector,
                    Expression::Call {
                        operation: Operation::Cards,
                        arguments,
                    } if arguments.is_empty()
                ) {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        format!("{cost_path}.selector"),
                        "discard cost currently requires unconstrained cards()",
                    ));
                }
                compiled.push(SpellAdditionalCostProgram::DiscardCards {
                    count: compile_bounded_positive_literal(
                        count,
                        &format!("{cost_path}.count"),
                        MAX_EFFECTS as u32,
                        "discard count",
                    )?,
                });
            }
            Operation::Sacrifice => {
                let [selector, count] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one permanent selector and literal count",
                    ));
                };
                let spec = compile_object_selector(selector, &format!("{cost_path}.selector"))?;
                if spec.kind != TargetKind::Permanent
                    || spec.controller != Some(TargetControllerPredicate::You)
                {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        format!("{cost_path}.selector"),
                        "sacrifice cost requires permanents controlled by you()",
                    ));
                }
                let mut predicate = ObjectTargetPredicate::any()
                    .with_owner(spec.owner)
                    .with_controller(TargetControllerPredicate::You)
                    .with_required_types(spec.required_types)
                    .with_required_any_types(spec.required_any_types)
                    .with_forbidden_types(spec.forbidden_types)
                    .with_required_subtypes(spec.required_subtypes);
                if let Some(minimum) = spec.minimum_mana_value {
                    predicate = predicate.with_minimum_mana_value(minimum);
                }
                if let Some(maximum) = spec.maximum_mana_value {
                    predicate = predicate.with_maximum_mana_value(maximum);
                }
                compiled.push(SpellAdditionalCostProgram::SacrificePermanents {
                    count: compile_bounded_positive_literal(
                        count,
                        &format!("{cost_path}.count"),
                        MAX_EFFECTS as u32,
                        "sacrifice count",
                    )?,
                    predicate,
                });
            }
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    cost_path,
                    format!(
                        "spell cost `{}` has no complete runtime lowering",
                        unsupported.as_str()
                    ),
                ));
            }
        }
    }
    Ok(compiled)
}

fn compile_activated_effect(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<ActivatedEffectProgram, CompileDiagnostic> {
    if ability.event.is_some() || ability.condition.is_some() || ability.mana_ability {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "non-mana activated ability cannot carry an event, condition, or mana flag",
        ));
    }
    let timing = match ability.timing.as_ref() {
        None => ActivationTiming::Instant,
        Some(Expression::Call {
            operation: Operation::TimingSorcery,
            arguments,
        }) if arguments.is_empty() => ActivationTiming::Sorcery,
        Some(_) => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                format!("{path}.timing"),
                "only timing_sorcery() is compiled for non-mana activations",
            ));
        }
    };

    let zero_cost = ManaCost::new(0, 0, 0, 0, 0, 0);
    let mut mana_cost = zero_cost;
    let mut exact_payment = ManaPool::empty();
    let mut tap_source = false;
    let mut sacrifice_source = false;
    let mut pay_life = 0_u32;
    for (index, cost) in ability.costs.iter().enumerate() {
        let cost_path = format!("{path}.costs[{index}]");
        let Expression::Call {
            operation,
            arguments,
        } = cost
        else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                cost_path,
                "activation cost is not an operation call",
            ));
        };
        match operation {
            Operation::ManaCost => {
                if mana_cost != zero_cost {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        cost_path,
                        "activation repeats its mana cost",
                    ));
                }
                let [Expression::Text(value)] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one fixed mana-cost string",
                    ));
                };
                (mana_cost, exact_payment) = compile_mana_cost_text(value, &cost_path)?;
            }
            Operation::TapSelf if arguments.is_empty() && !tap_source => tap_source = true,
            Operation::SacrificeSelf if arguments.is_empty() && !sacrifice_source => {
                sacrifice_source = true;
            }
            Operation::PayLife => {
                let [Expression::Integer(amount)] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one literal life amount",
                    ));
                };
                let amount = u32::try_from(*amount).map_err(|_| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectAmount,
                        &cost_path,
                        format!("life payment {amount} is outside the u32 action range"),
                    )
                })?;
                if amount == 0 || pay_life != 0 {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        cost_path,
                        "life payment must be positive and appear once",
                    ));
                }
                pay_life = amount;
            }
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    cost_path,
                    format!(
                        "activation cost `{}` has no complete runtime lowering",
                        unsupported.as_str()
                    ),
                ));
            }
        }
    }

    let mut compiler = ProgramCompiler::default();
    compile_effect(&ability.effect, &format!("{path}.effect"), &mut compiler)?;
    if compiler.effects.is_empty() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.effect"),
            "activation compiled no executable effect",
        ));
    }
    Ok(ActivatedEffectProgram {
        mana_cost,
        exact_payment,
        tap_source,
        sacrifice_source,
        pay_life,
        timing,
        target_requirements: compiler
            .targets
            .into_iter()
            .map(|target| target.requirement)
            .collect(),
        object_choice_requirements: compiler
            .object_choices
            .into_iter()
            .map(|choice| choice.requirement)
            .collect(),
        effects: compiler.effects,
        optional_effect_groups: compiler.optional_effect_groups,
    })
}

fn compile_mana_cost_text(
    value: &str,
    path: &str,
) -> Result<(ManaCost, ManaPool), CompileDiagnostic> {
    let mut symbols = Vec::new();
    let mut rest = value;
    while !rest.is_empty() {
        let Some(opened) = rest.strip_prefix('{') else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::ManaSymbol,
                path,
                format!("mana cost `{value}` has text outside braces"),
            ));
        };
        let Some(end) = opened.find('}') else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::ManaSymbol,
                path,
                format!("mana cost `{value}` has an unterminated symbol"),
            ));
        };
        let symbol = &opened[..end];
        if let Some(color) = Color::parse(symbol) {
            symbols.push(ManaSymbol::Color(color));
        } else if let Ok(generic) = symbol.parse::<u16>() {
            symbols.push(ManaSymbol::Generic(generic));
        } else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::ManaSymbol,
                path,
                format!("mana symbol `{{{symbol}}}` has no exact activation lowering"),
            ));
        }
        rest = &opened[end + 1..];
    }
    compile_mana_cost(&symbols)
}

fn compile_mana_outputs(value: &str, path: &str) -> Result<ManaOutputChoices, CompileDiagnostic> {
    if value == "any_color" {
        return Ok(ManaOutputChoices::from_options(&[
            ManaPool::of(ManaKind::White, 1),
            ManaPool::of(ManaKind::Blue, 1),
            ManaPool::of(ManaKind::Black, 1),
            ManaPool::of(ManaKind::Red, 1),
            ManaPool::of(ManaKind::Green, 1),
        ]));
    }
    if value.contains(" or ") {
        let mut outputs = Vec::new();
        for choice in value.split(" or ") {
            let output = compile_fixed_mana_output(choice, path)?;
            if outputs.contains(&output) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    format!("mana choice `{value}` repeats an output"),
                ));
            }
            outputs.push(output);
        }
        if outputs.len() < 2 || outputs.len() > 6 {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                format!("mana choice `{value}` must contain 2..=6 outputs"),
            ));
        }
        return Ok(ManaOutputChoices::from_options(&outputs));
    }
    Ok(ManaOutputChoices::from_options(&[
        compile_fixed_mana_output(value, path)?,
    ]))
}

fn compile_fixed_mana_output(value: &str, path: &str) -> Result<ManaPool, CompileDiagnostic> {
    let (count, symbol) = if let Some((count, symbol)) = value.split_once(" x ") {
        let count = count.parse::<u32>().map_err(|_| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                format!("mana multiplier '{count}' is not a u32"),
            )
        })?;
        (count, symbol)
    } else {
        (1, value)
    };
    if count == 0 {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "fixed mana output must be positive",
        ));
    }
    let kind = match symbol {
        "{W}" => ManaKind::White,
        "{U}" => ManaKind::Blue,
        "{B}" => ManaKind::Black,
        "{R}" => ManaKind::Red,
        "{G}" => ManaKind::Green,
        "{C}" => ManaKind::Colorless,
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                format!("mana output '{value}' requires an explicit runtime choice"),
            ));
        }
    };
    Ok(ManaPool::of(kind, count))
}

fn compile_program_kind(card_types: &[CardType]) -> Result<ProgramKind, CompileDiagnostic> {
    match card_types {
        [CardType::Instant] => Ok(ProgramKind::Instant),
        [CardType::Sorcery] => Ok(ProgramKind::Sorcery),
        [CardType::Land] => Ok(ProgramKind::Land),
        types
            if !types.is_empty()
                && types.iter().all(|card_type| {
                    matches!(
                        card_type,
                        CardType::Artifact
                            | CardType::Battle
                            | CardType::Creature
                            | CardType::Enchantment
                            | CardType::Planeswalker
                    )
                }) =>
        {
            Ok(ProgramKind::Permanent)
        }
        types => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::CardType,
            "card.faces[0].types",
            format!("unsupported top-level card types: {types:?}"),
        )),
    }
}

fn compile_base_object(
    supertypes: &[Supertype],
    card_types: &[CardType],
    subtypes: &[String],
    mana_symbols: &[ManaSymbol],
    mana_value: u32,
) -> Result<BaseObjectCharacteristics, CompileDiagnostic> {
    let mut types = ObjectTypes::none();
    for card_type in card_types {
        types = match card_type {
            CardType::Artifact => types.with_artifact(),
            CardType::Creature => types.with_creature(),
            CardType::Enchantment => types.with_enchantment(),
            CardType::Instant => types.with_instant(),
            CardType::Land => types.with_land(),
            CardType::Planeswalker => types.with_planeswalker(),
            CardType::Sorcery => types.with_sorcery(),
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::CardType,
                    "card.faces[0].types",
                    format!("card type {unsupported:?} has no kernel type bit"),
                ));
            }
        };
    }
    let mut colors = ObjectColors::none();
    for symbol in mana_symbols {
        if let ManaSymbol::Color(color) = symbol {
            colors = match color {
                Color::White => colors.with_white(),
                Color::Blue => colors.with_blue(),
                Color::Black => colors.with_black(),
                Color::Red => colors.with_red(),
                Color::Green => colors.with_green(),
            };
        }
    }
    let mut runtime_supertypes = ObjectSupertypes::none();
    for supertype in supertypes {
        runtime_supertypes = match supertype {
            Supertype::Basic => runtime_supertypes.with_basic(),
            Supertype::Legendary => runtime_supertypes.with_legendary(),
            Supertype::Ongoing => runtime_supertypes.with_ongoing(),
            Supertype::Snow => runtime_supertypes.with_snow(),
            Supertype::World => runtime_supertypes.with_world(),
        };
    }
    let mut basic_land_types = BasicLandTypes::none();
    let mut runtime_subtypes = ObjectSubtypes::none();
    for (index, subtype) in subtypes.iter().enumerate() {
        let parsed = ObjectSubtype::parse(subtype).ok_or_else(|| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::CardType,
                format!("card.faces[0].types.subtypes[{index}]"),
                format!("subtype `{subtype}` is empty, non-ASCII, or exceeds runtime bounds"),
            )
        })?;
        runtime_subtypes = runtime_subtypes.try_with(parsed).ok_or_else(|| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::ProgramBounds,
                "card.faces[0].types.subtypes",
                "printed subtype count exceeds the bounded runtime set",
            )
        })?;
    }
    if types.land() {
        for subtype in subtypes {
            basic_land_types = match subtype.to_ascii_lowercase().as_str() {
                "plains" => basic_land_types.with_plains(),
                "island" => basic_land_types.with_island(),
                "swamp" => basic_land_types.with_swamp(),
                "mountain" => basic_land_types.with_mountain(),
                "forest" => basic_land_types.with_forest(),
                _ => basic_land_types,
            };
        }
    }
    Ok(BaseObjectCharacteristics::new(types, colors)
        .with_supertypes(runtime_supertypes)
        .with_basic_land_types(basic_land_types)
        .with_subtypes(runtime_subtypes)
        .with_mana_value(mana_value))
}

fn compile_printed_mana_value(symbols: &[ManaSymbol]) -> Result<u32, CompileDiagnostic> {
    let mut mana_value = 0_u32;
    for (index, symbol) in symbols.iter().enumerate() {
        let contribution = match symbol {
            ManaSymbol::Color(_) => 1,
            ManaSymbol::Generic(amount) => u32::from(*amount),
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::ManaSymbol,
                    format!("card.faces[0].cost[{index}]"),
                    format!("mana symbol {unsupported:?} has no exact mana-value lowering"),
                ));
            }
        };
        mana_value = mana_value.checked_add(contribution).ok_or_else(|| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::ProgramBounds,
                format!("card.faces[0].cost[{index}]"),
                "printed mana value overflowed",
            )
        })?;
    }
    Ok(mana_value)
}

fn compile_mana_cost(symbols: &[ManaSymbol]) -> Result<(ManaCost, ManaPool), CompileDiagnostic> {
    let mut colored = [0_u32; 5];
    let mut generic = 0_u32;
    for (index, symbol) in symbols.iter().enumerate() {
        match symbol {
            ManaSymbol::Color(color) => {
                let color_index = match color {
                    Color::White => 0,
                    Color::Blue => 1,
                    Color::Black => 2,
                    Color::Red => 3,
                    Color::Green => 4,
                };
                colored[color_index] = colored[color_index].checked_add(1).ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        format!("card.faces[0].cost[{index}]"),
                        "colored mana count overflowed",
                    )
                })?;
            }
            ManaSymbol::Generic(amount) => {
                generic = generic.checked_add(u32::from(*amount)).ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        format!("card.faces[0].cost[{index}]"),
                        "generic mana count overflowed",
                    )
                })?;
            }
            unsupported => {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::ManaSymbol,
                    format!("card.faces[0].cost[{index}]"),
                    format!("mana symbol {unsupported:?} has no exact kernel lowering"),
                ));
            }
        }
    }
    Ok((
        ManaCost::new(
            colored[0], colored[1], colored[2], colored[3], colored[4], generic,
        ),
        ManaPool::new(
            colored[0], colored[1], colored[2], colored[3], colored[4], generic,
        ),
    ))
}

fn compile_effect(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<(), CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "effect root is not an operation call",
        ));
    };
    match operation {
        Operation::Sequence => {
            for (index, argument) in arguments.iter().enumerate() {
                compile_effect(argument, &format!("{path}.sequence[{index}]"), compiler)?;
                if compiler.effects.len() > MAX_EFFECTS {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        path,
                        format!("effect sequence exceeds {MAX_EFFECTS} operations"),
                    ));
                }
            }
            Ok(())
        }
        Operation::ChooseUpTo => {
            let [maximum, choice] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "literal maximum and one optional effect",
                ));
            };
            let maximum = compile_bounded_positive_literal(
                maximum,
                &format!("{path}.maximum"),
                1,
                "optional-effect maximum",
            )?;
            if maximum != 1 {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.maximum"),
                    "the current optional-effect grammar requires maximum 1",
                ));
            }
            let start = compiler.effects.len();
            compile_effect(choice, &format!("{path}.choice[0]"), compiler)?;
            let end = compiler.effects.len();
            if start == end {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "optional effect compiled no operations",
                ));
            }
            compiler
                .optional_effect_groups
                .push(OptionalEffectGroup { start, end });
            Ok(())
        }
        Operation::GainLife | Operation::LoseLife => {
            let [amount, players] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "amount and players"));
            };
            let amount = compile_amount(amount, &format!("{path}.amount"), compiler)?;
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            compiler.effects.push(if *operation == Operation::GainLife {
                EffectProgram::GainLife { players, amount }
            } else {
                EffectProgram::LoseLife { players, amount }
            });
            Ok(())
        }
        Operation::Draw | Operation::Scry => {
            let [count, players] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "count and players"));
            };
            let count = compile_amount(count, &format!("{path}.count"), compiler)?;
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            compiler.effects.push(if *operation == Operation::Draw {
                EffectProgram::DrawCards { players, count }
            } else {
                EffectProgram::Scry { players, count }
            });
            Ok(())
        }
        Operation::DiscardCards => {
            let [Expression::Integer(placeholder), players, Expression::Text(mode)] =
                arguments.as_slice()
            else {
                return Err(effect_arity(
                    path,
                    operation,
                    "the canonical hand placeholder, players, and mode",
                ));
            };
            if *placeholder != 1 || mode != "hand" {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "only discard_cards(1, players, \"hand\") is compiled",
                ));
            }
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            compiler
                .effects
                .push(EffectProgram::DiscardHands { players });
            Ok(())
        }
        Operation::Shuffle => {
            let players = match arguments.as_slice() {
                [] => PlayerBinding::Controller,
                [players] => compile_player_binding(players, &format!("{path}.players"), compiler)?,
                _ => return Err(effect_arity(path, operation, "zero or one player selector")),
            };
            compiler
                .effects
                .push(EffectProgram::ShuffleLibrary { players });
            Ok(())
        }
        Operation::Destroy | Operation::Exile => {
            let ([target] | [target, _]) = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one target selector"));
            };
            let target = compile_object_target(target, &format!("{path}.target"), compiler)?;
            compiler.effects.push(if *operation == Operation::Destroy {
                EffectProgram::DestroyPermanent { target }
            } else {
                EffectProgram::ExileObject { target }
            });
            Ok(())
        }
        Operation::CounterSpell => {
            let ([target] | [target, _]) = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one stack target selector"));
            };
            let target = compile_stack_target(target, &format!("{path}.target"), compiler)?;
            compiler
                .effects
                .push(EffectProgram::CounterStackEntry { target });
            Ok(())
        }
        Operation::MoveZoneFrom => {
            let ([target, from, to] | [target, from, to, _]) = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "target selector, source zone, and destination zone",
                ));
            };
            let from = compile_zone_kind(from, &format!("{path}.from"))?;
            let to = compile_zone_kind(to, &format!("{path}.to"))?;
            let target = compile_object_target(target, &format!("{path}.target"), compiler)?;
            compiler
                .effects
                .push(EffectProgram::MoveTargetObject { target, from, to });
            Ok(())
        }
        Operation::CreateToken => {
            let [template, count, players] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "registered token template, literal count, and player selector",
                ));
            };
            let template = compile_token_template(template, &format!("{path}.template"))?;
            let count = compile_token_count(count, &format!("{path}.count"), compiler)?;
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            compiler.effects.push(EffectProgram::CreateTokens {
                card: template.card,
                base_object: template.base_object,
                base_creature: template.base_creature,
                mana_ability: template.mana_ability,
                count,
                players,
            });
            Ok(())
        }
        Operation::SearchLibrary => {
            let [selector, players, count] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "library card selector, player selector, and literal maximum",
                ));
            };
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            if players != PlayerBinding::Controller {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::PlayerSelector,
                    format!("{path}.players"),
                    "the first search slice supports only you()",
                ));
            }
            let maximum = compile_bounded_positive_literal(
                count,
                &format!("{path}.maximum"),
                MAX_EFFECTS as u32,
                "search maximum",
            )?;
            let (
                required_types,
                required_any_types,
                forbidden_types,
                required_supertypes,
                required_land_types,
                required_any_land_types,
                required_subtypes,
            ) = compile_library_choice_selector(selector, &format!("{path}.selector"))?;
            let choice = intern_object_choice(
                compiler,
                selector,
                ObjectChoiceRequirement {
                    player: players,
                    zone: ZoneKind::Library,
                    maximum,
                    required_types,
                    required_any_types,
                    forbidden_types,
                    required_supertypes,
                    required_land_types,
                    required_any_land_types,
                    required_subtypes,
                },
            );
            compiler
                .effects
                .push(EffectProgram::SearchLibrary { choice });
            Ok(())
        }
        Operation::MoveZone => {
            let [selected, destination, count] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "chosen library objects, destination, and literal count",
                ));
            };
            let selector = chosen_selector(selected, &format!("{path}.chosen"))?;
            let Some(choice) = compiler
                .object_choices
                .iter()
                .position(|candidate| candidate.selector == *selector)
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.chosen"),
                    "chosen selector has no preceding compiled search_library choice",
                ));
            };
            let moved = compile_bounded_positive_literal(
                count,
                &format!("{path}.count"),
                MAX_EFFECTS as u32,
                "chosen move count",
            )?;
            if moved != compiler.object_choices[choice].requirement.maximum {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.count"),
                    "chosen move count must equal the linked search maximum",
                ));
            }
            let destination = compile_chosen_destination(destination, &format!("{path}.to"))?;
            if destination == ChosenDestination::LibraryTop && moved != 1 {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.count"),
                    "library_top currently requires exactly one chosen object",
                ));
            }
            compiler.effects.push(EffectProgram::MoveChosenObjects {
                choice,
                destination,
            });
            Ok(())
        }
        Operation::Tap => {
            let [selected] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one chosen object selector"));
            };
            let selector = chosen_selector(selected, &format!("{path}.chosen"))?;
            let Some(choice) = compiler
                .object_choices
                .iter()
                .position(|candidate| candidate.selector == *selector)
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.chosen"),
                    "chosen selector has no preceding compiled search_library choice",
                ));
            };
            compiler
                .effects
                .push(EffectProgram::TapChosenObjects { choice });
            Ok(())
        }
        unsupported => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            path,
            format!(
                "effect operation `{}` has no complete runtime lowering",
                unsupported.as_str()
            ),
        )),
    }
}

#[derive(Clone, Copy)]
struct TokenTemplate {
    card: CardId,
    base_object: BaseObjectCharacteristics,
    base_creature: Option<BaseCreatureCharacteristics>,
    mana_ability: Option<ActivatedAbilityProgram>,
}

fn compile_token_template(
    expression: &Expression,
    path: &str,
) -> Result<TokenTemplate, CompileDiagnostic> {
    let Expression::Text(script) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "token template is not text",
        ));
    };
    if script == "c_a_treasure_sac" {
        return Ok(TokenTemplate {
            card: CardId::new(stable_runtime_id(script)),
            base_object: BaseObjectCharacteristics::new(
                ObjectTypes::none().with_artifact(),
                ObjectColors::none(),
            ),
            base_creature: None,
            mana_ability: Some(ActivatedAbilityProgram {
                cost: ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0))
                    .with_tap_source()
                    .with_sacrifice_source(),
                outputs: ManaOutputChoices::from_options(&[
                    ManaPool::of(ManaKind::White, 1),
                    ManaPool::of(ManaKind::Blue, 1),
                    ManaPool::of(ManaKind::Black, 1),
                    ManaPool::of(ManaKind::Red, 1),
                    ManaPool::of(ManaKind::Green, 1),
                ]),
            }),
        });
    }
    let (colors, base_creature) = match script.as_str() {
        "g_3_3_beast" | "g_3_3_elephant" | "g_3_3_ape" | "g_3_3_frog_lizard" => (
            ObjectColors::none().with_green(),
            BaseCreatureCharacteristics::new(3, 3),
        ),
        "u_2_2_bird_flying" => (
            ObjectColors::none().with_blue(),
            BaseCreatureCharacteristics::new(2, 2)
                .with_keywords(CreatureKeywords::none().with_flying()),
        ),
        "b_2_2_zombie" => (
            ObjectColors::none().with_black(),
            BaseCreatureCharacteristics::new(2, 2),
        ),
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                format!("token template `{script}` is not in the exact runtime registry"),
            ));
        }
    };
    Ok(TokenTemplate {
        card: CardId::new(stable_runtime_id(script)),
        base_object: BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), colors),
        base_creature: Some(base_creature),
        mana_ability: None,
    })
}

fn compile_token_count(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<AmountProgram, CompileDiagnostic> {
    let count = compile_amount(expression, path, compiler)?;
    if let AmountProgram::Literal(count) = count {
        if count == 0 || count > MAX_TOKEN_COUNT {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::ProgramBounds,
                path,
                format!("token count must be in 1..={MAX_TOKEN_COUNT}, found {count}"),
            ));
        }
    }
    Ok(count)
}

fn compile_bounded_positive_literal(
    expression: &Expression,
    path: &str,
    maximum: u32,
    label: &str,
) -> Result<u32, CompileDiagnostic> {
    let Expression::Integer(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            path,
            format!("{label} is not a literal integer"),
        ));
    };
    let value = u32::try_from(*value).map_err(|_| {
        CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            path,
            format!("{label} {value} is outside the u32 action range"),
        )
    })?;
    if value == 0 || value > maximum {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            path,
            format!("{label} must be in 1..={maximum}, found {value}"),
        ));
    }
    Ok(value)
}

fn chosen_selector<'a>(
    expression: &'a Expression,
    path: &str,
) -> Result<&'a Expression, CompileDiagnostic> {
    let Expression::Call {
        operation: Operation::Chosen,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library move is not wrapped in chosen(...)[]",
        ));
    };
    let [selector] = arguments.as_slice() else {
        return Err(effect_arity(path, &Operation::Chosen, "one selector"));
    };
    Ok(selector)
}

fn compile_chosen_destination(
    expression: &Expression,
    path: &str,
) -> Result<ChosenDestination, CompileDiagnostic> {
    let Expression::Text(destination) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "chosen destination is not text",
        ));
    };
    match destination.to_ascii_lowercase().as_str() {
        "battlefield" => Ok(ChosenDestination::Zone(ZoneKind::Battlefield)),
        "hand" => Ok(ChosenDestination::Zone(ZoneKind::Hand)),
        "library_top" => Ok(ChosenDestination::LibraryTop),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("chosen destination `{destination}` is not compiled"),
        )),
    }
}

fn stable_runtime_id(value: &str) -> u32 {
    value.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

fn effect_arity(path: &str, operation: &Operation, expected: &str) -> CompileDiagnostic {
    CompileDiagnostic::new(
        CompileDiagnosticCode::EffectArguments,
        path,
        format!("{} requires {expected}", operation.as_str()),
    )
}

fn compile_amount(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<AmountProgram, CompileDiagnostic> {
    match expression {
        Expression::Integer(value) => {
            u32::try_from(*value)
                .map(AmountProgram::Literal)
                .map_err(|_| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectAmount,
                        path,
                        format!("amount {value} is outside the u32 action range"),
                    )
                })
        }
        Expression::Call {
            operation: Operation::Power,
            arguments,
        } => {
            let [target] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::Power, "one object selector"));
            };
            Ok(AmountProgram::PowerOfTargetObject(compile_object_target(
                target,
                &format!("{path}.power"),
                compiler,
            )?))
        }
        Expression::Call {
            operation: Operation::Count,
            arguments,
        } => {
            let [selector] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    &Operation::Count,
                    "one permanent selector",
                ));
            };
            let spec = compile_object_selector(selector, &format!("{path}.count"))?;
            if spec.kind != TargetKind::Permanent {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    path,
                    "count amount currently requires a permanent selector",
                ));
            }
            let mut predicate = ObjectTargetPredicate::any()
                .with_owner(spec.owner)
                .with_required_types(spec.required_types)
                .with_required_any_types(spec.required_any_types)
                .with_forbidden_types(spec.forbidden_types)
                .with_required_subtypes(spec.required_subtypes);
            if let Some(controller) = spec.controller {
                predicate = predicate.with_controller(controller);
            }
            if let Some(minimum) = spec.minimum_mana_value {
                predicate = predicate.with_minimum_mana_value(minimum);
            }
            if let Some(maximum) = spec.maximum_mana_value {
                predicate = predicate.with_maximum_mana_value(maximum);
            }
            Ok(AmountProgram::CountPermanents(predicate))
        }
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            path,
            "amount is neither a literal integer, target power, nor closed permanent count",
        )),
    }
}

fn compile_player_binding(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<PlayerBinding, CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::PlayerSelector,
            path,
            "player selector is not an operation call",
        ));
    };
    match operation {
        Operation::You if arguments.is_empty() => Ok(PlayerBinding::Controller),
        Operation::Opponent if arguments.is_empty() => Ok(PlayerBinding::Opponents),
        Operation::Any if arguments.is_empty() => Ok(PlayerBinding::AllPlayers),
        Operation::Target => Ok(PlayerBinding::Target(compile_player_target(
            expression, compiler, path,
        )?)),
        Operation::ControllerOf => {
            let [target] = arguments.as_slice() else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::PlayerSelector,
                    path,
                    "controller_of requires one object target",
                ));
            };
            let selector = target_selector(target, path)?;
            if is_any_selector(selector) {
                let object = unique_existing_target(compiler, TargetClass::Object, path)?;
                let stack = unique_existing_target(compiler, TargetClass::Stack, path)?;
                return match (object, stack) {
                    (Some(target), None) => Ok(PlayerBinding::ControllerOfTargetObject(target)),
                    (None, Some(target)) => Ok(PlayerBinding::ControllerOfTargetStack(target)),
                    (None, None) => Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::PlayerSelector,
                        path,
                        "controller_of(target(any())) has no preceding object or stack target",
                    )),
                    (Some(_), Some(_)) => Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::PlayerSelector,
                        path,
                        "controller_of(target(any())) is ambiguous across object and stack targets",
                    )),
                };
            }
            Ok(PlayerBinding::ControllerOfTargetObject(
                compile_object_target(target, &format!("{path}.object"), compiler)?,
            ))
        }
        unsupported => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::PlayerSelector,
            path,
            format!("player selector `{}` is not compiled", unsupported.as_str()),
        )),
    }
}

fn compile_player_target(
    expression: &Expression,
    compiler: &mut ProgramCompiler,
    path: &str,
) -> Result<usize, CompileDiagnostic> {
    let selector = target_selector(expression, path)?;
    if is_any_selector(selector) {
        if let Some(index) = unique_existing_target(compiler, TargetClass::Player, path)? {
            return Ok(index);
        }
    }
    let predicate = match selector {
        Expression::Call {
            operation: Operation::Any,
            arguments,
        } if arguments.is_empty() => PlayerTargetPredicate::Any,
        Expression::Call {
            operation: Operation::You,
            arguments,
        } if arguments.is_empty() => PlayerTargetPredicate::You,
        Expression::Call {
            operation: Operation::Opponent,
            arguments,
        } if arguments.is_empty() => PlayerTargetPredicate::Opponent,
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::PlayerSelector,
                path,
                "target does not contain a supported player selector",
            ));
        }
    };
    intern_target(
        compiler,
        selector,
        TargetRequirement::new(TargetKind::Player).with_player_predicate(predicate),
    )
}

fn compile_stack_target(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<usize, CompileDiagnostic> {
    let selector = target_selector(expression, path)?;
    if is_any_selector(selector) {
        if let Some(index) = unique_existing_target(compiler, TargetClass::Stack, path)? {
            return Ok(index);
        }
    }
    match selector {
        Expression::Call {
            operation: Operation::Spells,
            arguments,
        } => {
            let requirement = match arguments.as_slice() {
                [] => TargetRequirement::new(TargetKind::StackEntry),
                [predicate] => TargetRequirement::new(TargetKind::StackEntry)
                    .with_object_predicate(compile_stack_spell_predicate(predicate, path)?),
                _ => {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "spells() accepts at most one closed type predicate",
                    ));
                }
            };
            intern_target(compiler, selector, requirement)
        }
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target does not contain spells()",
        )),
    }
}

fn compile_stack_spell_predicate(
    expression: &Expression,
    path: &str,
) -> Result<ObjectTargetPredicate, CompileDiagnostic> {
    let mut spec = ObjectSelectorSpec::new(TargetKind::StackEntry);
    match expression {
        Expression::Call {
            operation: Operation::TypeIs,
            arguments,
        } => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::TypeIs, "one type string"));
            };
            spec.required_types = compile_object_type(value, path)?;
        }
        Expression::Call {
            operation: Operation::Not,
            arguments,
        } => {
            let [predicate] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::Not, "one predicate"));
            };
            let Expression::Call {
                operation: Operation::TypeIs,
                arguments,
            } = predicate
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "stack predicate not() only accepts type_is(...)",
                ));
            };
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::TypeIs, "one type string"));
            };
            spec.forbidden_types = compile_object_type(value, path)?;
        }
        Expression::Call {
            operation: Operation::Or,
            arguments,
        } => {
            if arguments.is_empty() {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "or() stack predicate is empty",
                ));
            }
            for (index, predicate) in arguments.iter().enumerate() {
                let Expression::Call {
                    operation: Operation::TypeIs,
                    arguments,
                } = predicate
                else {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        format!("{path}.or[{index}]"),
                        "stack predicate or() only accepts type_is(...) members",
                    ));
                };
                let [value] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &format!("{path}.or[{index}]"),
                        &Operation::TypeIs,
                        "one type string",
                    ));
                };
                spec.required_any_types = spec
                    .required_any_types
                    .union(compile_object_type(value, &format!("{path}.or[{index}]"))?);
            }
        }
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                "stack predicate must be type_is(...), not(type_is(...)), or a type_is(...) union",
            ));
        }
    }
    Ok(ObjectTargetPredicate::any()
        .with_required_types(spec.required_types)
        .with_required_any_types(spec.required_any_types)
        .with_forbidden_types(spec.forbidden_types)
        .with_required_subtypes(spec.required_subtypes))
}

fn compile_object_target(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<usize, CompileDiagnostic> {
    let selector = target_selector(expression, path)?;
    if is_any_selector(selector) {
        if let Some(index) = unique_existing_target(compiler, TargetClass::Object, path)? {
            return Ok(index);
        }
    }
    let spec = compile_object_selector(selector, path)?;
    let mut predicate = ObjectTargetPredicate::any()
        .with_owner(spec.owner)
        .with_required_types(spec.required_types)
        .with_required_any_types(spec.required_any_types)
        .with_forbidden_types(spec.forbidden_types)
        .with_required_subtypes(spec.required_subtypes);
    if let Some(controller) = spec.controller {
        predicate = predicate.with_controller(controller);
    }
    if let Some(minimum) = spec.minimum_mana_value {
        predicate = predicate.with_minimum_mana_value(minimum);
    }
    if let Some(maximum) = spec.maximum_mana_value {
        predicate = predicate.with_maximum_mana_value(maximum);
    }
    intern_target(
        compiler,
        selector,
        TargetRequirement::new(spec.kind).with_object_predicate(predicate),
    )
}

fn target_selector<'a>(
    expression: &'a Expression,
    path: &str,
) -> Result<&'a Expression, CompileDiagnostic> {
    let Expression::Call {
        operation: Operation::Target,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "selector is not wrapped in target(...)[]",
        ));
    };
    let [selector] = arguments.as_slice() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target requires exactly one selector",
        ));
    };
    Ok(selector)
}

fn is_any_selector(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call {
            operation: Operation::Any,
            arguments,
        } if arguments.is_empty()
    )
}

#[derive(Clone, Copy)]
enum TargetClass {
    Player,
    Object,
    Stack,
}

fn unique_existing_target(
    compiler: &ProgramCompiler,
    class: TargetClass,
    path: &str,
) -> Result<Option<usize>, CompileDiagnostic> {
    let matching: Vec<usize> = compiler
        .targets
        .iter()
        .enumerate()
        .filter_map(|(index, target)| {
            let matches = matches!(
                (class, target.requirement.kind()),
                (TargetClass::Player, TargetKind::Player)
                    | (TargetClass::Stack, TargetKind::StackEntry)
                    | (
                        TargetClass::Object,
                        TargetKind::Permanent
                            | TargetKind::ObjectInZone(_)
                            | TargetKind::ObjectInZoneKind(_),
                    )
            );
            matches.then_some(index)
        })
        .collect();
    match matching.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(*index)),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target(any()) is ambiguous after multiple compatible target slots",
        )),
    }
}

fn intern_target(
    compiler: &mut ProgramCompiler,
    selector: &Expression,
    requirement: TargetRequirement,
) -> Result<usize, CompileDiagnostic> {
    if let Some(index) = compiler
        .targets
        .iter()
        .position(|target| target.selector == *selector && target.requirement == requirement)
    {
        return Ok(index);
    }
    let index = compiler.targets.len();
    compiler.targets.push(CompiledTarget {
        selector: selector.clone(),
        requirement,
    });
    Ok(index)
}

fn intern_object_choice(
    compiler: &mut ProgramCompiler,
    selector: &Expression,
    requirement: ObjectChoiceRequirement,
) -> usize {
    if let Some(index) = compiler
        .object_choices
        .iter()
        .position(|choice| choice.selector == *selector && choice.requirement == requirement)
    {
        return index;
    }
    let index = compiler.object_choices.len();
    compiler.object_choices.push(CompiledObjectChoice {
        selector: selector.clone(),
        requirement,
    });
    index
}

#[derive(Default)]
struct LibraryChoiceConstraints {
    required: ObjectTypes,
    required_any: ObjectTypes,
    forbidden: ObjectTypes,
    required_supertypes: ObjectSupertypes,
    required_land_types: BasicLandTypes,
    required_any_land_types: BasicLandTypes,
    required_subtypes: ObjectSubtypes,
    library_zone: bool,
}

fn compile_library_choice_selector(
    selector: &Expression,
    path: &str,
) -> Result<
    (
        ObjectTypes,
        ObjectTypes,
        ObjectTypes,
        ObjectSupertypes,
        BasicLandTypes,
        BasicLandTypes,
        ObjectSubtypes,
    ),
    CompileDiagnostic,
> {
    let Expression::Call {
        operation: Operation::Cards,
        arguments,
    } = selector
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library choice selector is not cards(...)[]",
        ));
    };
    let mut constraints = LibraryChoiceConstraints::default();
    for (index, predicate) in arguments.iter().enumerate() {
        compile_library_choice_predicate(
            predicate,
            &format!("{path}.cards[{index}]"),
            &mut constraints,
        )?;
    }
    if !constraints.library_zone {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library choice requires zone_is(\"library\")",
        ));
    }
    if constraints.required == ObjectTypes::none()
        && constraints.required_any == ObjectTypes::none()
        && constraints.required_land_types == BasicLandTypes::none()
        && constraints.required_any_land_types == BasicLandTypes::none()
        && constraints.required_subtypes == ObjectSubtypes::none()
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library choice requires a closed top-level type predicate",
        ));
    }
    if constraints.required.intersects(constraints.forbidden)
        || (constraints.required_any != ObjectTypes::none()
            && constraints.forbidden.contains_all(constraints.required_any))
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library choice type constraints are contradictory",
        ));
    }
    Ok((
        constraints.required,
        constraints.required_any,
        constraints.forbidden,
        constraints.required_supertypes,
        constraints.required_land_types,
        constraints.required_any_land_types,
        constraints.required_subtypes,
    ))
}

fn compile_library_choice_predicate(
    expression: &Expression,
    path: &str,
    constraints: &mut LibraryChoiceConstraints,
) -> Result<(), CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "library choice predicate is not an operation call",
        ));
    };
    match operation {
        Operation::And => {
            for (index, predicate) in arguments.iter().enumerate() {
                compile_library_choice_predicate(
                    predicate,
                    &format!("{path}.and[{index}]"),
                    constraints,
                )?;
            }
            Ok(())
        }
        Operation::ZoneIs => {
            let [zone] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one zone string"));
            };
            if compile_zone_kind(zone, path)? != ZoneKind::Library || constraints.library_zone {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "library choice requires exactly one library zone predicate",
                ));
            }
            constraints.library_zone = true;
            Ok(())
        }
        Operation::TypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one type string"));
            };
            constraints.required = constraints
                .required
                .union(compile_object_type(value, path)?);
            Ok(())
        }
        Operation::SupertypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one supertype string"));
            };
            constraints.required_supertypes = constraints
                .required_supertypes
                .union(compile_object_supertype(value, path)?);
            Ok(())
        }
        Operation::SubtypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one subtype string"));
            };
            let subtype = compile_object_subtype(value, path)?;
            constraints.required_subtypes = constraints
                .required_subtypes
                .try_with(subtype)
                .ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        path,
                        "library subtype constraint exceeds the bounded runtime set",
                    )
                })?;
            if let Ok(land_type) = compile_basic_land_type(value, path) {
                constraints.required_land_types = constraints.required_land_types.union(land_type);
            }
            Ok(())
        }
        Operation::Not => {
            let [predicate] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one predicate"));
            };
            let Expression::Call {
                operation: Operation::TypeIs,
                arguments,
            } = predicate
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "library choice not() only accepts type_is(...)",
                ));
            };
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::TypeIs, "one type string"));
            };
            constraints.forbidden = constraints
                .forbidden
                .union(compile_object_type(value, path)?);
            Ok(())
        }
        Operation::Or => {
            if arguments.is_empty() {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "library choice or() is empty",
                ));
            }
            let mut top_level_types = None;
            for (index, predicate) in arguments.iter().enumerate() {
                let Expression::Call {
                    operation,
                    arguments,
                } = predicate
                else {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        format!("{path}.or[{index}]"),
                        "library choice or() only accepts homogeneous type_is(...) or basic-land subtype_is(...) members",
                    ));
                };
                let [value] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &format!("{path}.or[{index}]"),
                        operation,
                        "one type string",
                    ));
                };
                match operation {
                    Operation::TypeIs if top_level_types != Some(false) => {
                        top_level_types = Some(true);
                        constraints.required_any = constraints
                            .required_any
                            .union(compile_object_type(value, &format!("{path}.or[{index}]"))?);
                    }
                    Operation::SubtypeIs if top_level_types != Some(true) => {
                        top_level_types = Some(false);
                        constraints.required_any_land_types = constraints
                            .required_any_land_types
                            .union(compile_basic_land_type(
                                value,
                                &format!("{path}.or[{index}]"),
                            )?);
                    }
                    _ => {
                        return Err(CompileDiagnostic::new(
                            CompileDiagnosticCode::EffectArguments,
                            format!("{path}.or[{index}]"),
                            "library choice type union mixes incompatible predicate families",
                        ));
                    }
                }
            }
            Ok(())
        }
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!(
                "library choice predicate `{}` is not in the closed top-level type grammar",
                operation.as_str()
            ),
        )),
    }
}

#[derive(Clone, Copy)]
struct ObjectSelectorSpec {
    kind: TargetKind,
    owner: TargetControllerPredicate,
    controller: Option<TargetControllerPredicate>,
    required_types: ObjectTypes,
    required_any_types: ObjectTypes,
    forbidden_types: ObjectTypes,
    required_subtypes: ObjectSubtypes,
    minimum_mana_value: Option<u32>,
    maximum_mana_value: Option<u32>,
}

impl ObjectSelectorSpec {
    fn new(kind: TargetKind) -> Self {
        Self {
            kind,
            owner: TargetControllerPredicate::Any,
            controller: None,
            required_types: ObjectTypes::none(),
            required_any_types: ObjectTypes::none(),
            forbidden_types: ObjectTypes::none(),
            required_subtypes: ObjectSubtypes::none(),
            minimum_mana_value: None,
            maximum_mana_value: None,
        }
    }
}

fn compile_object_selector(
    selector: &Expression,
    path: &str,
) -> Result<ObjectSelectorSpec, CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = selector
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "object selector is not an operation call",
        ));
    };
    match operation {
        Operation::Permanents => {
            let mut spec = ObjectSelectorSpec::new(TargetKind::Permanent);
            for (index, predicate) in arguments.iter().enumerate() {
                compile_object_predicate(
                    predicate,
                    &format!("{path}.permanents[{index}]"),
                    &mut spec,
                )?;
            }
            Ok(spec)
        }
        Operation::Cards => {
            let mut spec = ObjectSelectorSpec::new(TargetKind::ObjectInZoneKind(ZoneKind::Exile));
            let mut zone = None;
            for (index, predicate) in arguments.iter().enumerate() {
                compile_card_predicate(
                    predicate,
                    &format!("{path}.cards[{index}]"),
                    &mut spec,
                    &mut zone,
                )?;
            }
            let Some(zone) = zone else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "cards() target requires one closed zone_is predicate",
                ));
            };
            spec.kind = TargetKind::ObjectInZoneKind(zone);
            Ok(spec)
        }
        Operation::All => compile_object_selector_union(arguments, path),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!(
                "unsupported object target selector `{}`",
                operation.as_str()
            ),
        )),
    }
}

fn compile_object_selector_union(
    selectors: &[Expression],
    path: &str,
) -> Result<ObjectSelectorSpec, CompileDiagnostic> {
    let mut specs = Vec::with_capacity(selectors.len());
    for (index, selector) in selectors.iter().enumerate() {
        specs.push(compile_object_selector(
            selector,
            &format!("{path}.all[{index}]"),
        )?);
    }
    let Some(first) = specs.first() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "all() target union is empty",
        ));
    };
    if specs.iter().any(|spec| {
        spec.kind != first.kind
            || spec.owner != first.owner
            || spec.controller != first.controller
            || spec.forbidden_types != ObjectTypes::none()
            || spec.required_any_types != ObjectTypes::none()
            || spec.required_subtypes != ObjectSubtypes::none()
            || spec.minimum_mana_value.is_some()
            || spec.maximum_mana_value.is_some()
    }) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target union has incompatible selectors",
        ));
    }
    let mut union = ObjectTypes::none();
    for spec in &specs {
        union = union.union(spec.required_types);
    }
    if union == ObjectTypes::none() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target union does not constrain any object type",
        ));
    }
    Ok(ObjectSelectorSpec {
        kind: first.kind,
        owner: first.owner,
        controller: first.controller,
        required_types: ObjectTypes::none(),
        required_any_types: union,
        forbidden_types: ObjectTypes::none(),
        required_subtypes: ObjectSubtypes::none(),
        minimum_mana_value: None,
        maximum_mana_value: None,
    })
}

fn compile_card_predicate(
    expression: &Expression,
    path: &str,
    spec: &mut ObjectSelectorSpec,
    zone: &mut Option<ZoneKind>,
) -> Result<(), CompileDiagnostic> {
    if let Expression::Call {
        operation: Operation::And,
        arguments,
    } = expression
    {
        for (index, predicate) in arguments.iter().enumerate() {
            compile_card_predicate(predicate, &format!("{path}.and[{index}]"), spec, zone)?;
        }
        return Ok(());
    }
    if let Expression::Call {
        operation: Operation::ZoneIs,
        arguments,
    } = expression
    {
        let [value] = arguments.as_slice() else {
            return Err(effect_arity(path, &Operation::ZoneIs, "one zone string"));
        };
        let parsed = compile_zone_kind(value, path)?;
        if zone.replace(parsed).is_some() {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                "multiple zone predicates are not supported",
            ));
        }
        return Ok(());
    }
    compile_object_predicate(expression, path, spec)
}

fn compile_object_predicate(
    expression: &Expression,
    path: &str,
    spec: &mut ObjectSelectorSpec,
) -> Result<(), CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "object predicate is not an operation call",
        ));
    };
    match operation {
        Operation::And => {
            for (index, predicate) in arguments.iter().enumerate() {
                compile_object_predicate(predicate, &format!("{path}.and[{index}]"), spec)?;
            }
            Ok(())
        }
        Operation::Or => {
            if arguments.is_empty() {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "object predicate or() is empty",
                ));
            }
            let mut alternatives = Vec::with_capacity(arguments.len());
            for (index, predicate) in arguments.iter().enumerate() {
                let mut alternative = ObjectSelectorSpec::new(spec.kind);
                compile_object_predicate(
                    predicate,
                    &format!("{path}.or[{index}]"),
                    &mut alternative,
                )?;
                alternatives.push(alternative);
            }
            let first = alternatives[0];
            if first.required_types == ObjectTypes::none()
                || first.required_any_types != ObjectTypes::none()
                || first.forbidden_types != ObjectTypes::none()
                || first.required_subtypes != ObjectSubtypes::none()
                || first.minimum_mana_value.is_some()
                || first.maximum_mana_value.is_some()
                || alternatives.iter().any(|alternative| {
                    alternative.owner != first.owner
                        || alternative.controller != first.controller
                        || alternative.required_types == ObjectTypes::none()
                        || alternative.required_any_types != ObjectTypes::none()
                        || alternative.forbidden_types != ObjectTypes::none()
                        || alternative.required_subtypes != ObjectSubtypes::none()
                        || alternative.minimum_mana_value.is_some()
                        || alternative.maximum_mana_value.is_some()
                })
            {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "object predicate or() requires homogeneous type branches with identical ownership",
                ));
            }
            let mut required_any = ObjectTypes::none();
            for alternative in alternatives {
                required_any = required_any.union(alternative.required_types);
            }
            if spec.owner != TargetControllerPredicate::Any && spec.owner != first.owner {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "object predicate or() contradicts an outer owner predicate",
                ));
            }
            if spec.controller.is_some() && spec.controller != first.controller {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "object predicate or() contradicts an outer controller predicate",
                ));
            }
            spec.owner = first.owner;
            spec.controller = first.controller;
            spec.required_any_types = spec.required_any_types.union(required_any);
            Ok(())
        }
        Operation::TypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one type string"));
            };
            spec.required_types = spec.required_types.union(compile_object_type(value, path)?);
            Ok(())
        }
        Operation::SubtypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one subtype string"));
            };
            spec.required_subtypes = spec
                .required_subtypes
                .try_with(compile_object_subtype(value, path)?)
                .ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        path,
                        "subtype predicate exceeds the bounded runtime set",
                    )
                })?;
            Ok(())
        }
        Operation::Not => {
            let [predicate] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one predicate"));
            };
            let Expression::Call {
                operation: Operation::TypeIs,
                arguments,
            } = predicate
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "only not(type_is(...)) is compiled",
                ));
            };
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, &Operation::TypeIs, "one type string"));
            };
            spec.forbidden_types = spec
                .forbidden_types
                .union(compile_object_type(value, path)?);
            Ok(())
        }
        Operation::OwnedBy | Operation::ControlledBy => {
            let [selector] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one player selector"));
            };
            let relationship = compile_target_relationship(selector, path)?;
            if *operation == Operation::OwnedBy {
                spec.owner = relationship;
            } else {
                spec.controller = Some(relationship);
            }
            Ok(())
        }
        Operation::AtLeast | Operation::LessThan => {
            let [Expression::Call {
                operation: Operation::ManaValue,
                arguments: mana_value_arguments,
            }, Expression::Integer(bound)] = arguments.as_slice()
            else {
                return Err(effect_arity(
                    path,
                    operation,
                    "mana_value(any()) and a nonnegative literal",
                ));
            };
            let [Expression::Call {
                operation: Operation::Any,
                arguments: any_arguments,
            }] = mana_value_arguments.as_slice()
            else {
                return Err(effect_arity(
                    &format!("{path}.mana_value"),
                    &Operation::ManaValue,
                    "any()",
                ));
            };
            if !any_arguments.is_empty() || *bound < 0 {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "mana-value bound requires any() and a nonnegative literal",
                ));
            }
            let bound = u32::try_from(*bound).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "mana-value bound does not fit u32",
                )
            })?;
            if *operation == Operation::AtLeast {
                if spec.minimum_mana_value.replace(bound).is_some() {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "multiple minimum mana-value bounds are not compiled",
                    ));
                }
            } else {
                let maximum = bound.checked_sub(1).ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "less_than mana-value bound must be greater than zero",
                    )
                })?;
                if spec.maximum_mana_value.replace(maximum).is_some() {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "multiple maximum mana-value bounds are not compiled",
                    ));
                }
            }
            if matches!(
                (spec.minimum_mana_value, spec.maximum_mana_value),
                (Some(minimum), Some(maximum)) if minimum > maximum
            ) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "mana-value bounds are contradictory",
                ));
            }
            Ok(())
        }
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("object predicate `{}` is not compiled", operation.as_str()),
        )),
    }
}

fn compile_target_relationship(
    expression: &Expression,
    path: &str,
) -> Result<TargetControllerPredicate, CompileDiagnostic> {
    match expression {
        Expression::Call {
            operation: Operation::You,
            arguments,
        } if arguments.is_empty() => Ok(TargetControllerPredicate::You),
        Expression::Call {
            operation: Operation::Opponent,
            arguments,
        } if arguments.is_empty() => Ok(TargetControllerPredicate::Opponent),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "owner/controller predicate requires you() or opponent()",
        )),
    }
}

fn compile_object_type(
    expression: &Expression,
    path: &str,
) -> Result<ObjectTypes, CompileDiagnostic> {
    let Expression::Text(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "object type is not text",
        ));
    };
    match value.to_ascii_lowercase().as_str() {
        "artifact" => Ok(ObjectTypes::none().with_artifact()),
        "creature" => Ok(ObjectTypes::none().with_creature()),
        "enchantment" => Ok(ObjectTypes::none().with_enchantment()),
        "instant" => Ok(ObjectTypes::none().with_instant()),
        "land" => Ok(ObjectTypes::none().with_land()),
        "planeswalker" => Ok(ObjectTypes::none().with_planeswalker()),
        "sorcery" => Ok(ObjectTypes::none().with_sorcery()),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("object type `{value}` has no kernel type bit"),
        )),
    }
}

fn compile_object_supertype(
    expression: &Expression,
    path: &str,
) -> Result<ObjectSupertypes, CompileDiagnostic> {
    let Expression::Text(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "object supertype is not text",
        ));
    };
    match value.to_ascii_lowercase().as_str() {
        "basic" => Ok(ObjectSupertypes::none().with_basic()),
        "legendary" => Ok(ObjectSupertypes::none().with_legendary()),
        "ongoing" => Ok(ObjectSupertypes::none().with_ongoing()),
        "snow" => Ok(ObjectSupertypes::none().with_snow()),
        "world" => Ok(ObjectSupertypes::none().with_world()),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("object supertype `{value}` is not in the closed runtime set"),
        )),
    }
}

fn compile_basic_land_type(
    expression: &Expression,
    path: &str,
) -> Result<BasicLandTypes, CompileDiagnostic> {
    let Expression::Text(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "basic land type is not text",
        ));
    };
    match value.to_ascii_lowercase().as_str() {
        "plains" => Ok(BasicLandTypes::none().with_plains()),
        "island" => Ok(BasicLandTypes::none().with_island()),
        "swamp" => Ok(BasicLandTypes::none().with_swamp()),
        "mountain" => Ok(BasicLandTypes::none().with_mountain()),
        "forest" => Ok(BasicLandTypes::none().with_forest()),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("subtype `{value}` is not one of the five basic land types"),
        )),
    }
}

fn compile_object_subtype(
    expression: &Expression,
    path: &str,
) -> Result<ObjectSubtype, CompileDiagnostic> {
    let Expression::Text(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "object subtype is not text",
        ));
    };
    ObjectSubtype::parse(value).ok_or_else(|| {
        CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("subtype `{value}` is empty, non-ASCII, or exceeds runtime bounds"),
        )
    })
}

fn compile_zone_kind(expression: &Expression, path: &str) -> Result<ZoneKind, CompileDiagnostic> {
    let Expression::Text(value) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "zone is not text",
        ));
    };
    match value.to_ascii_lowercase().as_str() {
        "battlefield" => Ok(ZoneKind::Battlefield),
        "command" => Ok(ZoneKind::Command),
        "exile" => Ok(ZoneKind::Exile),
        "graveyard" => Ok(ZoneKind::Graveyard),
        "hand" => Ok(ZoneKind::Hand),
        "library" => Ok(ZoneKind::Library),
        "stack" => Ok(ZoneKind::Stack),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("zone `{value}` is not compiled"),
        )),
    }
}

/// Runtime bindings and explicit choices for one program execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionBindings {
    controller: PlayerId,
    opponents: Vec<PlayerId>,
    targets: Vec<TargetChoice>,
    object_choices: Vec<Vec<ObjectId>>,
    optional_effect_choices: Vec<bool>,
    scry_bottoms: BTreeMap<(usize, PlayerId), Vec<ObjectId>>,
}

impl ExecutionBindings {
    /// Creates bindings for one controller and an ordered opponent set.
    #[must_use]
    pub fn new(controller: PlayerId, opponents: Vec<PlayerId>) -> Self {
        Self {
            controller,
            opponents,
            targets: Vec::new(),
            object_choices: Vec::new(),
            optional_effect_choices: Vec::new(),
            scry_bottoms: BTreeMap::new(),
        }
    }

    /// Supplies target choices in compiled announcement order.
    #[must_use]
    pub fn with_targets(mut self, targets: Vec<TargetChoice>) -> Self {
        self.targets = targets;
        self
    }

    /// Supplies explicit hidden-zone object choices in compiled choice order.
    #[must_use]
    pub fn with_object_choices(mut self, choices: Vec<Vec<ObjectId>>) -> Self {
        self.object_choices = choices;
        self
    }

    /// Supplies execute-or-skip decisions for optional effects in source order.
    #[must_use]
    pub fn with_optional_effect_choices(mut self, choices: Vec<bool>) -> Self {
        self.optional_effect_choices = choices;
        self
    }

    /// Supplies the ordered cards moved to the bottom for one scry effect.
    #[must_use]
    pub fn with_scry_bottom(
        mut self,
        effect_index: usize,
        player: PlayerId,
        bottom: Vec<ObjectId>,
    ) -> Self {
        self.scry_bottoms.insert((effect_index, player), bottom);
        self
    }

    /// Returns the controller binding.
    #[must_use]
    pub const fn controller(&self) -> PlayerId {
        self.controller
    }

    /// Returns opponents in deterministic execution order.
    #[must_use]
    pub fn opponents(&self) -> &[PlayerId] {
        &self.opponents
    }

    /// Returns target choices in compiled announcement order.
    #[must_use]
    pub fn targets(&self) -> &[TargetChoice] {
        &self.targets
    }

    /// Returns explicit hidden-zone object choices in compiled order.
    #[must_use]
    pub fn object_choices(&self) -> &[Vec<ObjectId>] {
        &self.object_choices
    }

    /// Returns execute-or-skip decisions for optional effects.
    #[must_use]
    pub fn optional_effect_choices(&self) -> &[bool] {
        &self.optional_effect_choices
    }
}

/// Stable reason that a compiled program could not bind or execute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionDiagnosticCode {
    /// A required player or object binding was absent or contradictory.
    MissingBinding,
    /// A required explicit choice was not supplied.
    MissingChoice,
    /// A supplied explicit choice violated the compiled bounds.
    InvalidChoice,
    /// A production kernel action rejected the bound operation.
    ProductionActionRejected,
}

impl ExecutionDiagnosticCode {
    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingBinding => "missing_runtime_binding",
            Self::MissingChoice => "missing_runtime_choice",
            Self::InvalidChoice => "invalid_runtime_choice",
            Self::ProductionActionRejected => "production_action_rejected",
        }
    }
}

/// One fail-closed binding or execution diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionDiagnostic {
    code: ExecutionDiagnosticCode,
    effect_index: Option<usize>,
    detail: String,
}

impl ExecutionDiagnostic {
    fn new(
        code: ExecutionDiagnosticCode,
        effect_index: Option<usize>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            effect_index,
            detail: detail.into(),
        }
    }

    /// Returns the stable diagnostic category.
    #[must_use]
    pub const fn code(&self) -> ExecutionDiagnosticCode {
        self.code
    }

    /// Returns the flattened source-order effect index, when applicable.
    #[must_use]
    pub const fn effect_index(&self) -> Option<usize> {
        self.effect_index
    }

    /// Returns diagnostic detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for ExecutionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.effect_index {
            Some(index) => write!(
                formatter,
                "{} at effect {index}: {}",
                self.code.as_str(),
                self.detail
            ),
            None => write!(formatter, "{}: {}", self.code.as_str(), self.detail),
        }
    }
}

impl Error for ExecutionDiagnostic {}

/// One production action bound to its source-order effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundAction {
    effect_index: usize,
    action: Action,
}

impl BoundAction {
    /// Returns the source-order effect index.
    #[must_use]
    pub const fn effect_index(&self) -> usize {
        self.effect_index
    }

    /// Returns the production action.
    #[must_use]
    pub const fn action(&self) -> &Action {
        &self.action
    }
}

/// One applied action and its successful production outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionRecord {
    effect_index: usize,
    action: Action,
    outcome: Outcome,
}

impl ExecutionRecord {
    /// Returns the source-order effect index.
    #[must_use]
    pub const fn effect_index(&self) -> usize {
        self.effect_index
    }

    /// Returns the applied action.
    #[must_use]
    pub const fn action(&self) -> &Action {
        &self.action
    }

    /// Returns the successful kernel outcome.
    #[must_use]
    pub const fn outcome(&self) -> &Outcome {
        &self.outcome
    }
}

/// Deterministic production-action trace for one effect-program execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionTrace {
    records: Vec<ExecutionRecord>,
}

impl ExecutionTrace {
    /// Returns applied records in execution order.
    #[must_use]
    pub fn records(&self) -> &[ExecutionRecord] {
        &self.records
    }
}

/// Resolves every binding and choice into actions without mutating game state.
pub fn bind_program_actions(
    state: &GameState,
    program: &CardProgram,
    bindings: &ExecutionBindings,
) -> Result<Vec<BoundAction>, ExecutionDiagnostic> {
    bind_effect_actions(
        state,
        &program.target_requirements,
        &program.object_choice_requirements,
        &program.effects,
        &program.optional_effect_groups,
        bindings,
    )
}

/// Resolves one triggered ability's bindings without mutating game state.
pub fn bind_triggered_ability_actions(
    state: &GameState,
    ability: &TriggeredAbilityProgram,
    bindings: &ExecutionBindings,
) -> Result<Vec<BoundAction>, ExecutionDiagnostic> {
    bind_effect_actions(
        state,
        &ability.target_requirements,
        &ability.object_choice_requirements,
        &ability.effects,
        &ability.optional_effect_groups,
        bindings,
    )
}

/// Resolves one non-mana activated ability's bindings without mutating game state.
pub fn bind_activated_effect_actions(
    state: &GameState,
    ability: &ActivatedEffectProgram,
    bindings: &ExecutionBindings,
) -> Result<Vec<BoundAction>, ExecutionDiagnostic> {
    bind_effect_actions(
        state,
        &ability.target_requirements,
        &ability.object_choice_requirements,
        &ability.effects,
        &ability.optional_effect_groups,
        bindings,
    )
}

fn bind_effect_actions(
    state: &GameState,
    target_requirements: &[TargetRequirement],
    object_choice_requirements: &[ObjectChoiceRequirement],
    effects: &[EffectProgram],
    optional_effect_groups: &[OptionalEffectGroup],
    bindings: &ExecutionBindings,
) -> Result<Vec<BoundAction>, ExecutionDiagnostic> {
    validate_player_bindings(bindings)?;
    state
        .validate_target_choices(
            bindings.controller,
            None,
            target_requirements,
            &bindings.targets,
        )
        .map_err(|error| {
            ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!("kernel rejected target binding: {error:?}"),
            )
        })?;
    validate_object_choices(state, object_choice_requirements, bindings)?;
    if optional_effect_groups.len() != bindings.optional_effect_choices.len() {
        return Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingChoice,
            None,
            format!(
                "program requires {} optional-effect decision(s), found {}",
                optional_effect_groups.len(),
                bindings.optional_effect_choices.len()
            ),
        ));
    }
    let mut actions = Vec::new();
    for (effect_index, effect) in effects.iter().enumerate() {
        if optional_effect_groups
            .iter()
            .zip(&bindings.optional_effect_choices)
            .any(|(group, execute)| !*execute && (group.start..group.end).contains(&effect_index))
        {
            continue;
        }
        match effect {
            EffectProgram::GainLife { players, amount } => {
                let amount = resolve_amount(state, *amount, bindings, effect_index)?;
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::GainLife { player, amount },
                    });
                }
            }
            EffectProgram::LoseLife { players, amount } => {
                let amount = resolve_amount(state, *amount, bindings, effect_index)?;
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::LoseLife { player, amount },
                    });
                }
            }
            EffectProgram::DrawCards { players, count } => {
                let count = resolve_amount(state, *count, bindings, effect_index)?;
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::DrawCards { player, count },
                    });
                }
            }
            EffectProgram::DiscardHands { players } => {
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    let hand = ZoneId::new(Some(player), ZoneKind::Hand);
                    let objects = state.zone_objects(hand).ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::MissingBinding,
                            Some(effect_index),
                            format!("player {player:?} has no hand zone"),
                        )
                    })?;
                    for object in objects {
                        actions.push(BoundAction {
                            effect_index,
                            action: Action::MoveObject {
                                object: *object,
                                to: ZoneId::new(Some(player), ZoneKind::Graveyard),
                            },
                        });
                    }
                }
            }
            EffectProgram::Scry { players, count } => {
                let count = resolve_amount(state, *count, bindings, effect_index)?;
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    let bottom = if count == 0 {
                        Vec::new()
                    } else {
                        bindings
                            .scry_bottoms
                            .get(&(effect_index, player))
                            .cloned()
                            .ok_or_else(|| {
                                ExecutionDiagnostic::new(
                                    ExecutionDiagnosticCode::MissingChoice,
                                    Some(effect_index),
                                    format!("no scry-bottom choice for player {player:?}"),
                                )
                            })?
                    };
                    if bottom.len() > count as usize {
                        return Err(ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::InvalidChoice,
                            Some(effect_index),
                            format!(
                                "scry-bottom choice has {} objects for count {count}",
                                bottom.len()
                            ),
                        ));
                    }
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::Scry {
                            player,
                            count,
                            bottom,
                        },
                    });
                }
            }
            EffectProgram::ShuffleLibrary { players } => {
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::ShuffleLibrary { player },
                    });
                }
            }
            EffectProgram::DestroyPermanent { target } => {
                actions.push(BoundAction {
                    effect_index,
                    action: Action::DestroyPermanent {
                        object: resolve_object_target(bindings, *target, effect_index)?,
                    },
                });
            }
            EffectProgram::ExileObject { target } => {
                let object = resolve_object_target(bindings, *target, effect_index)?;
                actions.push(BoundAction {
                    effect_index,
                    action: Action::MoveObject {
                        object,
                        to: ZoneId::new(None, ZoneKind::Exile),
                    },
                });
            }
            EffectProgram::CounterStackEntry { target } => {
                actions.push(BoundAction {
                    effect_index,
                    action: Action::CounterStackEntry {
                        entry: resolve_stack_target(bindings, *target, effect_index)?,
                    },
                });
            }
            EffectProgram::MoveTargetObject { target, from, to } => {
                let object = resolve_object_target(bindings, *target, effect_index)?;
                let current = state.object_zone(object).ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        format!("target object {object:?} has no zone"),
                    )
                })?;
                if current.kind() != *from {
                    return Err(ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        format!(
                            "target object is in {:?}, expected {from:?}",
                            current.kind()
                        ),
                    ));
                }
                let owner = state
                    .object(object)
                    .ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::InvalidChoice,
                            Some(effect_index),
                            format!("target object {object:?} is unknown"),
                        )
                    })?
                    .owner();
                let destination_owner = match to {
                    ZoneKind::Library | ZoneKind::Hand | ZoneKind::Graveyard => Some(owner),
                    ZoneKind::Battlefield
                    | ZoneKind::Exile
                    | ZoneKind::Stack
                    | ZoneKind::Command
                    | ZoneKind::Ceased => None,
                };
                actions.push(BoundAction {
                    effect_index,
                    action: Action::MoveObject {
                        object,
                        to: ZoneId::new(destination_owner, *to),
                    },
                });
            }
            EffectProgram::CreateTokens {
                card,
                base_object,
                base_creature,
                mana_ability: _,
                count,
                players,
            } => {
                let count = resolve_amount(state, *count, bindings, effect_index)?;
                if count > MAX_TOKEN_COUNT {
                    return Err(ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        format!(
                            "resolved token count {count} exceeds the maximum {MAX_TOKEN_COUNT}"
                        ),
                    ));
                }
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    for _ in 0..count {
                        actions.push(BoundAction {
                            effect_index,
                            action: Action::CreateToken {
                                card: *card,
                                owner: player,
                                controller: player,
                                base_object: *base_object,
                                base: *base_creature,
                            },
                        });
                    }
                }
            }
            EffectProgram::SearchLibrary { .. } => {}
            EffectProgram::MoveChosenObjects {
                choice,
                destination,
            } => {
                let selected = bindings.object_choices.get(*choice).ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingChoice,
                        Some(effect_index),
                        format!("no object choice supplied for slot {choice}"),
                    )
                })?;
                for object in selected {
                    let owner = state
                        .object(*object)
                        .ok_or_else(|| {
                            ExecutionDiagnostic::new(
                                ExecutionDiagnosticCode::InvalidChoice,
                                Some(effect_index),
                                format!("chosen object {object:?} is unknown"),
                            )
                        })?
                        .owner();
                    let action = match destination {
                        ChosenDestination::LibraryTop => Action::PutObjectOnTopOfLibrary {
                            player: owner,
                            object: *object,
                        },
                        ChosenDestination::Zone(kind) => Action::MoveObject {
                            object: *object,
                            to: ZoneId::new(
                                match kind {
                                    ZoneKind::Hand | ZoneKind::Library | ZoneKind::Graveyard => {
                                        Some(owner)
                                    }
                                    ZoneKind::Battlefield
                                    | ZoneKind::Exile
                                    | ZoneKind::Stack
                                    | ZoneKind::Command
                                    | ZoneKind::Ceased => None,
                                },
                                *kind,
                            ),
                        },
                    };
                    actions.push(BoundAction {
                        effect_index,
                        action,
                    });
                }
            }
            EffectProgram::TapChosenObjects { choice } => {
                let selected = bindings.object_choices.get(*choice).ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingChoice,
                        Some(effect_index),
                        format!("no object choice supplied for slot {choice}"),
                    )
                })?;
                for object in selected {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::SetObjectTapped {
                            object: *object,
                            tapped: true,
                        },
                    });
                }
            }
        }
    }
    Ok(actions)
}

fn validate_object_choices(
    state: &GameState,
    requirements: &[ObjectChoiceRequirement],
    bindings: &ExecutionBindings,
) -> Result<(), ExecutionDiagnostic> {
    if requirements.len() != bindings.object_choices.len() {
        return Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingChoice,
            None,
            format!(
                "program requires {} object choice slot(s), found {}",
                requirements.len(),
                bindings.object_choices.len()
            ),
        ));
    }
    for (choice_index, (requirement, selected)) in requirements
        .iter()
        .zip(&bindings.object_choices)
        .enumerate()
    {
        if selected.len() > requirement.maximum as usize {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!(
                    "object choice {choice_index} selected {} objects, maximum is {}",
                    selected.len(),
                    requirement.maximum
                ),
            ));
        }
        let player = match requirement.player {
            PlayerBinding::Controller => bindings.controller,
            _ => {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    format!("object choice {choice_index} has an unsupported player binding"),
                ));
            }
        };
        let zone = ZoneId::new(Some(player), requirement.zone);
        for (selected_index, object) in selected.iter().copied().enumerate() {
            if selected[..selected_index].contains(&object)
                || state.object_zone(object) != Some(zone)
            {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    format!(
                        "object choice {choice_index} contains duplicate or out-of-zone object {object:?}"
                    ),
                ));
            }
            let characteristics = state.object_characteristics(object).map_err(|error| {
                ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    format!("object choice {choice_index} characteristics failed: {error:?}"),
                )
            })?;
            if !characteristics
                .types()
                .contains_all(requirement.required_types)
                || (requirement.required_any_types != ObjectTypes::none()
                    && !characteristics
                        .types()
                        .intersects(requirement.required_any_types))
                || characteristics
                    .types()
                    .intersects(requirement.forbidden_types)
                || !characteristics
                    .supertypes()
                    .contains_all(requirement.required_supertypes)
                || !characteristics
                    .subtypes()
                    .contains_all(requirement.required_subtypes)
                || !characteristics
                    .basic_land_types()
                    .contains_all(requirement.required_land_types)
                || (requirement.required_any_land_types != BasicLandTypes::none()
                    && !characteristics
                        .basic_land_types()
                        .intersects(requirement.required_any_land_types))
                || ((requirement.required_land_types != BasicLandTypes::none()
                    || requirement.required_any_land_types != BasicLandTypes::none())
                    && !characteristics.types().land())
            {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    format!(
                        "object choice {choice_index} object {object:?} fails its type predicate"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_player_bindings(bindings: &ExecutionBindings) -> Result<(), ExecutionDiagnostic> {
    if bindings.opponents.contains(&bindings.controller) {
        return Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingBinding,
            None,
            "controller also appears in the opponent set",
        ));
    }
    let mut ordered = bindings.opponents.clone();
    ordered.sort();
    ordered.dedup();
    if ordered.len() != bindings.opponents.len() {
        return Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingBinding,
            None,
            "opponent set contains duplicate players",
        ));
    }
    Ok(())
}

fn resolve_players(
    state: &GameState,
    binding: PlayerBinding,
    bindings: &ExecutionBindings,
    effect_index: usize,
) -> Result<Vec<PlayerId>, ExecutionDiagnostic> {
    match binding {
        PlayerBinding::Controller => Ok(vec![bindings.controller]),
        PlayerBinding::AllPlayers => {
            let mut players = Vec::with_capacity(bindings.opponents.len().saturating_add(1));
            players.push(bindings.controller);
            players.extend(bindings.opponents.iter().copied());
            Ok(players)
        }
        PlayerBinding::Opponents if bindings.opponents.is_empty() => Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingBinding,
            Some(effect_index),
            "opponent selector has no bound opponent",
        )),
        PlayerBinding::Opponents => Ok(bindings.opponents.clone()),
        PlayerBinding::Target(target) => match bindings.targets.get(target) {
            Some(TargetChoice::Player(player)) => Ok(vec![*player]),
            _ => Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::MissingBinding,
                Some(effect_index),
                format!("target slot {target} is not a player"),
            )),
        },
        PlayerBinding::ControllerOfTargetObject(target) => {
            let object = resolve_object_target(bindings, target, effect_index)?;
            state
                .object_controller(object)
                .map(|player| vec![player])
                .map_err(|error| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        format!("cannot resolve controller of target {target}: {error:?}"),
                    )
                })
        }
        PlayerBinding::ControllerOfTargetStack(target) => {
            let Some(TargetChoice::StackEntry(entry)) = bindings.targets.get(target) else {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::MissingBinding,
                    Some(effect_index),
                    format!("target slot {target} is not a stack entry"),
                ));
            };
            state
                .stack_entries()
                .iter()
                .find(|candidate| candidate.id() == *entry)
                .map(|entry| vec![entry.controller()])
                .ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        format!("target stack entry {entry:?} is unavailable"),
                    )
                })
        }
    }
}

fn resolve_amount(
    state: &GameState,
    amount: AmountProgram,
    bindings: &ExecutionBindings,
    effect_index: usize,
) -> Result<u32, ExecutionDiagnostic> {
    match amount {
        AmountProgram::Literal(amount) => Ok(amount),
        AmountProgram::PowerOfTargetObject(target) => {
            let object = resolve_object_target(bindings, target, effect_index)?;
            let power = state
                .creature_characteristics(object)
                .map_err(|error| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        format!("target power is unavailable: {error:?}"),
                    )
                })?
                .power();
            u32::try_from(power).map_err(|_| {
                ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    Some(effect_index),
                    format!("target power {power} is negative"),
                )
            })
        }
        AmountProgram::CountPermanents(predicate) => {
            let requirement =
                TargetRequirement::new(TargetKind::Permanent).with_object_predicate(predicate);
            let battlefield = state
                .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
                .ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        "battlefield zone is unavailable",
                    )
                })?;
            let count = battlefield
                .iter()
                .filter(|object| {
                    state
                        .validate_target_choices(
                            bindings.controller,
                            None,
                            &[requirement],
                            &[TargetChoice::Object(**object)],
                        )
                        .is_ok()
                })
                .count();
            u32::try_from(count).map_err(|_| {
                ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    Some(effect_index),
                    "matching permanent count exceeds u32",
                )
            })
        }
    }
}

fn resolve_object_target(
    bindings: &ExecutionBindings,
    target: usize,
    effect_index: usize,
) -> Result<ObjectId, ExecutionDiagnostic> {
    match bindings.targets.get(target) {
        Some(TargetChoice::Object(object)) => Ok(*object),
        _ => Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingBinding,
            Some(effect_index),
            format!("target slot {target} is not an object"),
        )),
    }
}

fn resolve_stack_target(
    bindings: &ExecutionBindings,
    target: usize,
    effect_index: usize,
) -> Result<StackEntryId, ExecutionDiagnostic> {
    match bindings.targets.get(target) {
        Some(TargetChoice::StackEntry(entry)) => Ok(*entry),
        _ => Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingBinding,
            Some(effect_index),
            format!("target slot {target} is not a stack entry"),
        )),
    }
}

/// Binds the complete program before applying any production action.
pub fn execute_program(
    state: &mut GameState,
    program: &CardProgram,
    bindings: &ExecutionBindings,
) -> Result<ExecutionTrace, ExecutionDiagnostic> {
    let actions = bind_program_actions(state, program, bindings)?;
    let mut records = Vec::with_capacity(actions.len());
    for bound in actions {
        let outcome = apply(state, bound.action.clone());
        if let Outcome::Failed(error) = &outcome {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::ProductionActionRejected,
                Some(bound.effect_index),
                format!("kernel rejected {:?}: {error:?}", bound.action),
            ));
        }
        records.push(ExecutionRecord {
            effect_index: bound.effect_index,
            action: bound.action,
            outcome,
        });
    }
    Ok(ExecutionTrace { records })
}

#[cfg(test)]
mod tests {
    use super::{
        compile_card_program, execute_program, Capability, CompileDiagnosticCode,
        ExecutionBindings, ExecutionDiagnosticCode, ProgramKind,
    };
    use forge_core::{
        apply, Action, BaseCreatureCharacteristics, BaseObjectCharacteristics, BasicLandTypes,
        CardId, GameState, ManaKind, ManaPool, ObjectColors, ObjectSubtype, ObjectSupertypes,
        ObjectTypes, Outcome, StackObjectKind, TargetChoice, ZoneId, ZoneKind,
    };

    const SWORDS: &str = r#"
card "Swords to Plowshares" {
  id: "b1544f21-7e98-461b-aed5-e748b0168c52"
  layout: normal
  status: unverified_playable
  face "Swords to Plowshares" {
    cost: "{W}"
    types: "Instant"
    oracle: "Exile target creature. Its controller gains life equal to its power."
    keywords: []
    ability spell {
      effect: sequence(exile(target(permanents(type_is("creature")))), gain_life(power(target(any())), controller_of(target(any()))))
    }
  }
}
"#;
    const COUNTERSPELL: &str =
        include_str!("../../../cards/cp_dsl/definitions/006_counterspell.frs");
    const PLAINS: &str = r#"
card "Plains" {
  id: "bc71ebf6-2056-41f7-be35-b2e5c34afa99"
  layout: normal
  status: unverified_playable
  face "Plains" {
    cost: ""
    types: "Basic Land - Plains"
    oracle: "({T}: Add {W}.)"
    keywords: []
  }
}
"#;
    const SOL_RING: &str = r#"
card "Sol Ring" {
  id: "6ad8011d-3471-4369-9d68-b264cc027487"
  layout: normal
  status: unverified_playable
  face "Sol Ring" {
    cost: "{1}"
    types: "Artifact"
    oracle: "{T}: Add {C}{C}."
    keywords: []
    ability activated {
      costs: [tap_self()]
      effect: add_mana("2 x {C}", you())
      mana_ability: true
    }
  }
}
"#;
    const BIRDS_OF_PARADISE: &str = r#"
card "Birds of Paradise" {
  id: "d3a0b660-358c-41bd-9cd2-41fbf3491b1a"
  layout: normal
  status: unverified_playable
  face "Birds of Paradise" {
    cost: "{G}"
    types: "Creature - Bird"
    oracle: "Flying\n{T}: Add one mana of any color."
    power: "0"
    toughness: "1"
    keywords: [flying]
    ability activated {
      costs: [tap_self()]
      effect: add_mana("any_color", you())
      mana_ability: true
    }
  }
}
"#;
    const BEAST_WITHIN: &str = r#"
card "Beast Within" {
  id: "adef83bc-4047-434f-9137-36d0bc473b2c"
  layout: normal
  status: unverified_playable
  face "Beast Within" {
    cost: "{2}{G}"
    types: "Instant"
    oracle: "Destroy target permanent. Its controller creates a 3/3 green Beast creature token."
    keywords: []
    ability spell {
      effect: sequence(destroy(target(permanents())), create_token("g_3_3_beast", 1, controller_of(target(any()))))
    }
  }
}
"#;
    const NEGATE: &str = r#"
card "Negate" {
  id: "f6a0c6f8-2fa7-4b0d-9acf-8fe95fbf1214"
  layout: normal
  status: unverified_playable
  face "Negate" {
    cost: "{1}{U}"
    types: "Instant"
    oracle: "Counter target noncreature spell."
    keywords: []
    ability spell {
      effect: counter_spell(target(spells(not(type_is("creature")))))
    }
  }
}
"#;
    const ELADAMRIS_CALL: &str = r#"
card "Eladamri's Call" {
  id: "4acb6612-54e8-428d-acb6-c7259a5ad6a8"
  layout: normal
  status: unverified_playable
  face "Eladamri's Call" {
    cost: "{G}{W}"
    types: "Instant"
    oracle: "Search your library for a creature card, reveal that card, put it into your hand, then shuffle."
    keywords: []
    ability spell {
      effect: sequence(search_library(cards(and(type_is("creature"), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(type_is("creature"), zone_is("library")))), "hand", 1), shuffle(you()))
    }
  }
}
"#;
    const ENLIGHTENED_TUTOR: &str = r#"
card "Enlightened Tutor" {
  id: "c5229c17-b7be-4b05-b683-f2277edc4849"
  layout: normal
  status: unverified_playable
  face "Enlightened Tutor" {
    cost: "{W}"
    types: "Instant"
    oracle: "Search your library for an artifact or enchantment card, reveal it, then shuffle and put that card on top."
    keywords: []
    ability spell {
      effect: sequence(search_library(cards(and(or(type_is("artifact"), type_is("enchantment")), zone_is("library"))), you(), 1), shuffle(you()), move_zone(chosen(cards(and(or(type_is("artifact"), type_is("enchantment")), zone_is("library")))), "library_top", 1))
    }
  }
}
"#;
    const RAMPANT_GROWTH: &str = r#"
card "Rampant Growth" {
  id: "8539f295-5d58-4436-a73a-b9277c4c7795"
  layout: normal
  status: unverified_playable
  face "Rampant Growth" {
    cost: "{1}{G}"
    types: "Sorcery"
    oracle: "Search your library for a basic land card, put that card onto the battlefield tapped, then shuffle."
    keywords: []
    ability spell {
      effect: sequence(search_library(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library")))), "battlefield", 1), tap(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))))), shuffle(you()))
    }
  }
}
"#;

    const LIBRARY_PROGRAM: &str = r#"
card "Interpreter Contract" {
  id: "forge:runtime:contract"
  layout: normal
  status: unverified_playable
  face "Interpreter Contract" {
    cost: "{1}{U}"
    types: "Sorcery"
    oracle: "Contract fixture."
    keywords: []
    ability spell {
      effect: sequence(gain_life(3, you()), lose_life(2, opponent()), draw(1, you()), scry(1, you()), shuffle(you()))
    }
  }
}
"#;

    #[test]
    fn compile_and_execute_use_production_actions_in_source_order() {
        let definition = parse("contract.frs", LIBRARY_PROGRAM);
        let program = compile_card_program(&definition)
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let controller = add_player(&mut state);
        let opponent = add_player(&mut state);
        let library = ZoneId::new(Some(controller), ZoneKind::Library);
        let mut cards = Vec::new();
        for offset in 0..4 {
            let outcome = apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(1_000 + offset),
                    owner: controller,
                    controller,
                    zone: library,
                },
            );
            let Outcome::ObjectCreated(object) = outcome else {
                panic!("unexpected create outcome: {outcome:?}");
            };
            cards.push(object);
        }
        let bindings = ExecutionBindings::new(controller, vec![opponent]).with_scry_bottom(
            3,
            controller,
            vec![cards[2]],
        );
        let trace = execute_program(&mut state, &program, &bindings)
            .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));

        assert_eq!(trace.records().len(), 5);
        assert_eq!(state.players()[controller.index()].life(), 23);
        assert_eq!(state.players()[opponent.index()].life(), 18);
        assert_eq!(
            state
                .zone_objects(ZoneId::new(Some(controller), ZoneKind::Hand))
                .map(<[forge_core::ObjectId]>::len),
            Some(1)
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn missing_choice_fails_before_mutation() {
        let definition = parse("contract.frs", LIBRARY_PROGRAM);
        let program = compile_card_program(&definition)
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let controller = add_player(&mut state);
        let opponent = add_player(&mut state);
        let before = state.deterministic_hash();
        let error = match execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(controller, vec![opponent]),
        ) {
            Ok(_) => panic!("missing scry choice must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), ExecutionDiagnosticCode::MissingChoice);
        assert_eq!(state.deterministic_hash(), before);
    }

    #[test]
    fn unsupported_operation_is_path_qualified_and_fail_closed() {
        let definition = parse(
            "unsupported.frs",
            &LIBRARY_PROGRAM.replace(
                "sequence(gain_life(3, you()), lose_life(2, opponent()), draw(1, you()), scry(1, you()), shuffle(you()))",
                "mill(3, opponent())",
            ),
        );
        let error = match compile_card_program(&definition) {
            Ok(_) => panic!("mill must not compile"),
            Err(error) => error,
        };
        assert_eq!(error.code(), CompileDiagnosticCode::EffectOperation);
        assert!(error.path().contains("abilities[0].effect"));
        assert!(error.detail().contains("mill"));
    }

    #[test]
    fn basic_land_and_fixed_mana_ability_compile_without_card_branches() {
        let plains = compile_card_program(&parse("plains.frs", PLAINS))
            .unwrap_or_else(|error| panic!("unexpected Plains compile error: {error}"));
        assert_eq!(plains.kind(), ProgramKind::Land);
        assert_eq!(
            plains.capabilities(),
            vec![Capability::LandPlay, Capability::ManaAbility]
        );
        assert_eq!(
            plains.activated_abilities()[0].produces(),
            ManaPool::of(ManaKind::White, 1)
        );
        assert!(plains.base_object().supertypes().basic());
        assert!(plains.base_object().basic_land_types().plains());

        let ring = compile_card_program(&parse("sol_ring.frs", SOL_RING))
            .unwrap_or_else(|error| panic!("unexpected Sol Ring compile error: {error}"));
        assert_eq!(ring.kind(), ProgramKind::Permanent);
        assert_eq!(
            ring.activated_abilities()[0].produces(),
            ManaPool::of(ManaKind::Colorless, 2)
        );
    }

    #[test]
    fn creature_characteristics_and_all_color_mana_choices_compile_exactly() {
        let birds = compile_card_program(&parse("birds.frs", BIRDS_OF_PARADISE))
            .unwrap_or_else(|error| panic!("unexpected Birds compile error: {error}"));
        let base = birds
            .base_creature()
            .unwrap_or_else(|| panic!("Birds must carry creature characteristics"));
        assert_eq!(base.power(), 0);
        assert_eq!(base.toughness(), 1);
        assert!(base.keywords().flying());
        let outputs = birds.activated_abilities()[0].output_choices();
        assert_eq!(outputs.options().len(), 5);
        for kind in [
            ManaKind::White,
            ManaKind::Blue,
            ManaKind::Black,
            ManaKind::Red,
            ManaKind::Green,
        ] {
            assert!(outputs.contains(ManaPool::of(kind, 1)));
        }
        assert!(!outputs.contains(ManaPool::of(ManaKind::Colorless, 1)));
    }

    #[test]
    fn targeted_exile_prebinds_power_and_controller_before_moving_object() {
        let program = compile_card_program(&parse("swords.frs", SWORDS))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let target = create_object(
            &mut state,
            CardId::new(2_000),
            opponent,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: target,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature(),
                        ObjectColors::none().with_green(),
                    ),
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics {
                    object: target,
                    base: BaseCreatureCharacteristics::new(3, 3),
                },
            ),
            Outcome::Applied
        );

        let trace = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_targets(vec![TargetChoice::Object(target)]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));

        assert_eq!(trace.records().len(), 2);
        assert_eq!(
            state.object_zone(target),
            Some(ZoneId::new(None, ZoneKind::Exile))
        );
        assert_eq!(state.players()[opponent.index()].life(), 23);
    }

    #[test]
    fn counterspell_program_removes_the_selected_stack_entry() {
        let program = compile_card_program(&parse("counterspell.frs", COUNTERSPELL))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let target = create_object(
            &mut state,
            CardId::new(2_001),
            opponent,
            ZoneId::new(Some(opponent), ZoneKind::Hand),
        );
        prepare_turn(&mut state, caster, opponent);
        let entry = match apply(
            &mut state,
            Action::PutSpellOnStack {
                player: caster,
                object: target,
                kind: StackObjectKind::InstantSpell,
                hold_priority: true,
            },
        ) {
            Outcome::StackEntryAdded(entry) => entry,
            other => panic!("unexpected stack outcome: {other:?}"),
        };

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_targets(vec![TargetChoice::StackEntry(entry)]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));

        assert!(state.stack_entries().is_empty());
        assert_eq!(
            state.object_zone(target),
            Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard))
        );
    }

    #[test]
    fn exact_token_template_prebinds_destroyed_targets_controller() {
        let program = compile_card_program(&parse("beast_within.frs", BEAST_WITHIN))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![Capability::DestroyPermanent, Capability::CreateToken]
        );
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let target = create_object(
            &mut state,
            CardId::new(2_002),
            opponent,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: target,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_artifact(),
                        ObjectColors::none(),
                    ),
                },
            ),
            Outcome::Applied
        );

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_targets(vec![TargetChoice::Object(target)]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));

        assert_eq!(
            state.object_zone(target),
            Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard))
        );
        let Some(battlefield) = state.zone_objects(ZoneId::new(None, ZoneKind::Battlefield)) else {
            panic!("battlefield must exist");
        };
        assert_eq!(battlefield.len(), 1);
        let Some(token) = state.object(battlefield[0]) else {
            panic!("token must exist");
        };
        assert!(token.is_token());
        assert_eq!(token.owner(), opponent);
        assert_eq!(token.controller(), opponent);
        assert_eq!(
            token.base_object(),
            BaseObjectCharacteristics::new(
                ObjectTypes::none().with_creature(),
                ObjectColors::none().with_green(),
            )
        );
        assert_eq!(
            token.base_creature(),
            Some(BaseCreatureCharacteristics::new(3, 3))
        );
    }

    #[test]
    fn unknown_token_template_fails_closed_with_a_qualified_path() {
        let definition = parse(
            "unknown_token.frs",
            &BEAST_WITHIN.replace("g_3_3_beast", "u_1_1_faerie_flying"),
        );
        let error = match compile_card_program(&definition) {
            Ok(_) => panic!("unknown token must not compile"),
            Err(error) => error,
        };
        assert_eq!(error.code(), CompileDiagnosticCode::EffectArguments);
        assert!(error.path().ends_with(".template"));
        assert!(error.detail().contains("exact runtime registry"));
    }

    #[test]
    fn stack_spell_type_predicate_rejects_creatures_and_accepts_instants() {
        let program = compile_card_program(&parse("negate.frs", NEGATE))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let requirement = program.target_requirements()[0];
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        prepare_turn(&mut state, caster, opponent);

        let instant = create_stack_spell(
            &mut state,
            caster,
            CardId::new(2_003),
            ObjectTypes::none().with_instant(),
            StackObjectKind::InstantSpell,
        );
        let creature = create_stack_spell(
            &mut state,
            caster,
            CardId::new(2_004),
            ObjectTypes::none().with_creature(),
            StackObjectKind::PermanentSpell,
        );

        assert!(state.can_target(caster, None, requirement, TargetChoice::StackEntry(instant)));
        assert!(!state.can_target(
            caster,
            None,
            requirement,
            TargetChoice::StackEntry(creature)
        ));
    }

    #[test]
    fn stack_spell_type_union_compiles_to_a_closed_any_type_set() {
        let definition = parse(
            "stack_union.frs",
            &NEGATE.replace(
                "not(type_is(\"creature\"))",
                "or(type_is(\"instant\"), type_is(\"sorcery\"))",
            ),
        );
        let program = compile_card_program(&definition)
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let forge_core::TargetPredicate::Object(predicate) =
            program.target_requirements()[0].predicate()
        else {
            panic!("typed spell target must use an object predicate");
        };
        assert_eq!(
            predicate.required_any_types(),
            ObjectTypes::none().with_instant().with_sorcery()
        );
    }

    #[test]
    fn explicit_library_choice_moves_matching_card_to_hand_and_rejects_wrong_type() {
        let program = compile_card_program(&parse("eladamris_call.frs", ELADAMRIS_CALL))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::SearchLibrary,
                Capability::MoveZone,
                Capability::ShuffleLibrary
            ]
        );
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let creature = create_object(
            &mut state,
            CardId::new(2_005),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Library),
        );
        let artifact = create_object(
            &mut state,
            CardId::new(2_006),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Library),
        );
        for (object, types) in [
            (creature, ObjectTypes::none().with_creature()),
            (artifact, ObjectTypes::none().with_artifact()),
        ] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(types, ObjectColors::none()),
                    },
                ),
                Outcome::Applied
            );
        }

        let before_invalid = state.deterministic_hash();
        let invalid = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, Vec::new()).with_object_choices(vec![vec![artifact]]),
        );
        assert!(invalid.is_err());
        assert_eq!(state.deterministic_hash(), before_invalid);

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, Vec::new()).with_object_choices(vec![vec![creature]]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(
            state.object_zone(creature),
            Some(ZoneId::new(Some(caster), ZoneKind::Hand))
        );
    }

    #[test]
    fn basic_land_search_rejects_nonbasic_land_and_moves_basic_land_tapped() {
        let program = compile_card_program(&parse("rampant_growth.frs", RAMPANT_GROWTH))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let library = ZoneId::new(Some(caster), ZoneKind::Library);
        let nonbasic_forest = create_object(&mut state, CardId::new(2_008), caster, library);
        let basic_forest = create_object(&mut state, CardId::new(2_009), caster, library);
        for (object, supertypes) in [
            (nonbasic_forest, ObjectSupertypes::none()),
            (basic_forest, ObjectSupertypes::none().with_basic()),
        ] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(
                            ObjectTypes::none().with_land(),
                            ObjectColors::none(),
                        )
                        .with_supertypes(supertypes)
                        .with_basic_land_types(BasicLandTypes::none().with_forest()),
                    },
                ),
                Outcome::Applied
            );
        }

        let before_invalid = state.deterministic_hash();
        assert!(execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, Vec::new())
                .with_object_choices(vec![vec![nonbasic_forest]]),
        )
        .is_err());
        assert_eq!(state.deterministic_hash(), before_invalid);

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, Vec::new())
                .with_object_choices(vec![vec![basic_forest]]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(
            state.object_zone(basic_forest),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert!(state
            .object(basic_forest)
            .is_some_and(forge_core::ObjectRecord::tapped));
    }

    #[test]
    fn tutor_choice_is_repositioned_on_top_after_shuffle() {
        let program = compile_card_program(&parse("enlightened_tutor.frs", ENLIGHTENED_TUTOR))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let selected = create_object(
            &mut state,
            CardId::new(2_007),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Library),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: selected,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_artifact(),
                        ObjectColors::none(),
                    ),
                },
            ),
            Outcome::Applied
        );
        for offset in 0..3 {
            create_object(
                &mut state,
                CardId::new(2_100 + offset),
                caster,
                ZoneId::new(Some(caster), ZoneKind::Library),
            );
        }

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, Vec::new()).with_object_choices(vec![vec![selected]]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(
            state
                .zone_objects(ZoneId::new(Some(caster), ZoneKind::Library))
                .and_then(<[forge_core::ObjectId]>::last),
            Some(&selected)
        );
    }

    #[test]
    fn library_search_subtype_predicate_compiles_to_exact_subtype_state() {
        let definition = parse(
            "subtype_search.frs",
            &ELADAMRIS_CALL.replace("type_is(\"creature\")", "subtype_is(\"elf\")"),
        );
        let program = compile_card_program(&definition)
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let requirement = program.object_choice_requirements()[0];
        let elf = ObjectSubtype::parse("Elf").expect("fixture subtype is valid");
        assert!(requirement.required_subtypes().contains(elf));
    }

    fn parse(path: &str, source: &str) -> forge_carddef::CardDefinition {
        forge_cardc::parse_card_named(path, source)
            .unwrap_or_else(|error| panic!("fixture did not parse: {error}"))
    }

    fn add_player(state: &mut GameState) -> forge_core::PlayerId {
        match apply(state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected add-player outcome: {other:?}"),
        }
    }

    fn create_object(
        state: &mut GameState,
        card: CardId,
        owner: forge_core::PlayerId,
        zone: ZoneId,
    ) -> forge_core::ObjectId {
        match apply(
            state,
            Action::CreateObject {
                card,
                owner,
                controller: owner,
                zone,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected create-object outcome: {other:?}"),
        }
    }

    fn prepare_turn(
        state: &mut GameState,
        caster: forge_core::PlayerId,
        opponent: forge_core::PlayerId,
    ) {
        assert!(matches!(
            apply(
                state,
                Action::SetTurnOrder {
                    order: vec![caster, opponent],
                }
            ),
            Outcome::TurnOrderDecided(player) if player == caster
        ));
        for player in [caster, opponent] {
            for offset in 0..7 {
                create_object(
                    state,
                    CardId::new(3_000 + player.index() as u32 * 10 + offset),
                    player,
                    ZoneId::new(Some(player), ZoneKind::Library),
                );
            }
        }
        assert_eq!(apply(state, Action::DrawOpeningHands), Outcome::Applied);
        for player in [caster, opponent] {
            assert_eq!(
                apply(
                    state,
                    Action::KeepOpeningHand {
                        player,
                        bottom: Vec::new(),
                    },
                ),
                Outcome::Applied
            );
        }
        assert_eq!(
            apply(
                state,
                Action::StartTurn {
                    active_player: caster,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(state, Action::AdvanceStep),
            Outcome::StepAdvanced(forge_core::Step::Upkeep)
        );
    }

    fn create_stack_spell(
        state: &mut GameState,
        player: forge_core::PlayerId,
        card: CardId,
        types: ObjectTypes,
        kind: StackObjectKind,
    ) -> forge_core::StackEntryId {
        let object = create_object(
            state,
            card,
            player,
            ZoneId::new(Some(player), ZoneKind::Hand),
        );
        assert_eq!(
            apply(
                state,
                Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(types, ObjectColors::none()),
                },
            ),
            Outcome::Applied
        );
        match apply(
            state,
            Action::PutSpellOnStack {
                player,
                object,
                kind,
                hold_priority: true,
            },
        ) {
            Outcome::StackEntryAdded(entry) => entry,
            other => panic!("unexpected stack outcome: {other:?}"),
        }
    }
}
