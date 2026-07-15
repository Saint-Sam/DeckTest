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
    apply, auto_payment_plan, AbilityPlayer, Action, ActivatedAbilityDefinition,
    ActivatedAbilityEffect, ActivationCondition, ActivationCost, ActivationTiming,
    BaseCreatureCharacteristics, BaseObjectCharacteristics, BasicLandTypes, CardId,
    CombatDamageTarget, CombatRestriction, CombatRestrictionSubject, ContinuousEffectCondition,
    ContinuousEffectDefinition, ContinuousEffectDuration, ContinuousEffectOperation,
    ContinuousEffectTarget, CostModifierDefinition, CostModifierOperation, CostModifierScope,
    CounterKind, CreatureKeywords, GameState, ManaCost, ManaKind, ManaPool, ObjectColors, ObjectId,
    ObjectSubtype, ObjectSubtypes, ObjectSupertypes, ObjectTargetPredicate, ObjectTypes, Outcome,
    PlayerId, PlayerRule, PlayerRuleSubject, PlayerTargetPredicate, RestrictionDefinition,
    RestrictionEffect, StackEntryId, TargetChoice, TargetControllerPredicate, TargetKind,
    TargetRequirement, TargetRestriction, TargetRestrictionSubject, TriggerCondition,
    TriggerDefinition, TriggerObjectFilter, TriggerPlayerFilter, TriggerZoneFilter, ZoneId,
    ZoneKind,
};
use std::{collections::BTreeMap, error::Error, fmt};

const MAX_EFFECTS: usize = 64;
const MAX_ACTIVATED_ABILITIES: usize = 16;
const MAX_TOKEN_COUNT: u32 = 64;
const MAX_SPELL_MODES: usize = 8;

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
    /// Cycle a card from hand through the production cycling action.
    Cycling,
    /// Gain life.
    GainLife,
    /// Lose life.
    LoseLife,
    /// Deal noncombat damage through the production damage pipeline.
    DealDamage,
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
    /// Sacrifice a source permanent without applying destruction rules.
    SacrificePermanent,
    /// Create one or more exact registered token templates.
    CreateToken,
    /// Search a library with an explicit validated object choice.
    SearchLibrary,
    /// Tap one or more explicit objects.
    TapObject,
    /// Apply a typed continuous characteristic change.
    ModifyCharacteristics,
    /// Reduce generic mana costs for matching spells.
    ReduceSpellCost,
    /// Apply a typed continuous player-rule change.
    ModifyPlayerRules,
    /// Attach one source object to a validated object target.
    AttachObject,
    /// Apply a typed targeting restriction to an object.
    TargetingRestriction,
    /// Prevent one or more exact permanents from being destroyed.
    Indestructible,
    /// Add typed counters to an object.
    AddCounters,
    /// Apply a typed combat declaration restriction.
    CombatRestriction,
    /// Offer a typed alternate casting cost under a closed condition.
    AlternateCost,
    /// Announce and execute exactly one mode of a modal spell.
    ChooseMode,
    /// Replace targeted spell text with the compiled each-object form.
    Overload,
    /// Prevent spell casts and non-mana activations while this spell is on the stack.
    SplitSecond,
    /// Choose and execute either face of a modal double-faced card.
    ModalDfc,
    /// Publicly reveal one or more explicitly chosen objects.
    RevealObjects,
}

impl Capability {
    /// Returns the stable capability identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LandPlay => "land_play",
            Self::ManaAbility => "mana_ability",
            Self::ActivatedAbility => "activated_ability",
            Self::Cycling => "cycling",
            Self::GainLife => "gain_life",
            Self::LoseLife => "lose_life",
            Self::DealDamage => "deal_damage",
            Self::DrawCards => "draw_cards",
            Self::DiscardCards => "discard_cards",
            Self::Scry => "scry",
            Self::ShuffleLibrary => "shuffle_library",
            Self::PermanentSpell => "permanent_spell",
            Self::DestroyPermanent => "destroy_permanent",
            Self::ExileObject => "exile_object",
            Self::CounterStackEntry => "counter_stack_entry",
            Self::MoveZone => "move_zone",
            Self::SacrificePermanent => "sacrifice_permanent",
            Self::CreateToken => "create_token",
            Self::SearchLibrary => "search_library",
            Self::TapObject => "tap_object",
            Self::ModifyCharacteristics => "modify_characteristics",
            Self::ReduceSpellCost => "reduce_spell_cost",
            Self::ModifyPlayerRules => "modify_player_rules",
            Self::AttachObject => "attach_object",
            Self::TargetingRestriction => "targeting_restriction",
            Self::Indestructible => "indestructible",
            Self::AddCounters => "add_counters",
            Self::CombatRestriction => "combat_restriction",
            Self::AlternateCost => "alternate_cost",
            Self::ChooseMode => "choose_mode",
            Self::Overload => "overload",
            Self::SplitSecond => "split_second",
            Self::ModalDfc => "modal_dfc",
            Self::RevealObjects => "reveal_objects",
        }
    }
}

/// One completely compiled fixed-output mana ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivatedAbilityProgram {
    cost: ActivationCost,
    outputs: ManaOutputChoices,
    damage_to_controller: u32,
    condition: Option<ActivationCondition>,
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

    /// Returns noncombat damage dealt to the controller on resolution.
    #[must_use]
    pub const fn damage_to_controller(self) -> u32 {
        self.damage_to_controller
    }

    /// Returns the closed activation condition, when present.
    #[must_use]
    pub const fn condition(self) -> Option<ActivationCondition> {
        self.condition
    }

    /// Binds this program to one controller and battlefield source.
    #[must_use]
    pub fn bind(self, controller: PlayerId, source: ObjectId) -> ActivatedAbilityDefinition {
        let output = self.outputs.deterministic_smoke_output();
        let effect = if self.damage_to_controller == 0 {
            ActivatedAbilityEffect::AddMana {
                player: AbilityPlayer::Controller,
                mana: output,
            }
        } else {
            ActivatedAbilityEffect::AddManaAndDealDamage {
                player: AbilityPlayer::Controller,
                mana: output,
                amount: self.damage_to_controller,
            }
        };
        let mut definition = ActivatedAbilityDefinition::new(
            controller,
            Some(source),
            ActivationTiming::Instant,
            self.cost,
            effect,
        );
        if let Some(condition) = self.condition {
            definition = definition.with_condition(condition);
        }
        definition.as_mana_ability()
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
        let effect = if self.damage_to_controller == 0 {
            ActivatedAbilityEffect::AddMana {
                player: AbilityPlayer::Controller,
                mana: output,
            }
        } else {
            ActivatedAbilityEffect::AddManaAndDealDamage {
                player: AbilityPlayer::Controller,
                mana: output,
                amount: self.damage_to_controller,
            }
        };
        let mut definition = ActivatedAbilityDefinition::new(
            controller,
            Some(source),
            ActivationTiming::Instant,
            self.cost,
            effect,
        );
        if let Some(condition) = self.condition {
            definition = definition.with_condition(condition);
        }
        Some(definition.as_mana_ability())
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
    /// The player carried by the event that caused this trigger.
    TriggeringPlayer,
}

/// A nonnegative effect amount resolved during prebinding.
// Closed predicates remain inline so this public value type stays Copy.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmountProgram {
    /// Literal amount embedded in the definition.
    Literal(u32),
    /// Current power of one object target.
    PowerOfTargetObject(usize),
    /// Current number of battlefield permanents matching a closed predicate.
    CountPermanents(ObjectTargetPredicate),
}

/// One closed object set resolved when an effect is bound to a game state.
// Closed predicates remain inline so this public value type stays Copy.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectSetProgram {
    /// One announced object target slot.
    Target(usize),
    /// Every current battlefield permanent matching a closed predicate.
    Battlefield(ObjectTargetPredicate),
}

/// One explicit hidden-zone object choice exposed by a compiled program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectChoiceRequirement {
    player: PlayerBinding,
    zone: ZoneKind,
    minimum: u32,
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

    /// Returns the minimum number of objects that must be chosen.
    #[must_use]
    pub const fn minimum(self) -> u32 {
        self.minimum
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
    /// Deal noncombat damage to one announced player-or-object target.
    DealDamageToTarget {
        /// Target slot.
        target: usize,
        /// Damage amount.
        amount: AmountProgram,
    },
    /// Deal noncombat damage to a resolved player set without targeting.
    DealDamageToPlayers {
        /// Affected players.
        players: PlayerBinding,
        /// Damage amount.
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
    /// Destroy one targeted permanent without allowing regeneration.
    DestroyPermanentWithoutRegeneration {
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
        /// Closed each-object selector used only by an overload cast.
        overload_predicate: Option<ObjectTargetPredicate>,
    },
    /// Sacrifice the executing ability's source permanent.
    SacrificeSource,
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
    /// Discard explicitly chosen cards from their owner's hand.
    DiscardChosenObjects {
        /// Object-choice slot containing the cards to discard.
        choice: usize,
    },
    /// Modify current power and toughness until a represented duration ends.
    ModifyPowerToughness {
        /// Objects affected when the effect resolves.
        objects: ObjectSetProgram,
        /// Power delta.
        power: AmountProgram,
        /// Toughness delta.
        toughness: AmountProgram,
        /// Exact continuous-effect duration.
        duration: ContinuousEffectDuration,
    },
    /// Grant represented creature keywords until a represented duration ends.
    GrantKeywords {
        /// Objects affected when the effect resolves.
        objects: ObjectSetProgram,
        /// Keywords granted.
        keywords: CreatureKeywords,
        /// Exact continuous-effect duration.
        duration: ContinuousEffectDuration,
    },
    /// Grant an object-level targeting restriction for a represented duration.
    GrantTargetingRestriction {
        /// Objects protected when the effect resolves.
        objects: ObjectSetProgram,
        /// Exact targeting restriction granted.
        restriction: TargetRestriction,
        /// Exact restriction duration.
        duration: ContinuousEffectDuration,
    },
    /// Prevent exact objects from being destroyed for a represented duration.
    GrantIndestructible {
        /// Objects protected when the effect resolves.
        objects: ObjectSetProgram,
        /// Exact restriction duration.
        duration: ContinuousEffectDuration,
    },
    /// Attach the executing ability's source to one announced object target.
    AttachSourceToTarget {
        /// Object target slot.
        target: usize,
    },
    /// Add counters to the executing ability's source.
    AddCountersToSource {
        /// Counter kind.
        kind: CounterKind,
        /// Counter amount.
        amount: u32,
    },
    /// Publicly reveal objects selected by a prior explicit choice.
    RevealChosenObjects {
        /// Object-choice slot containing the objects to reveal.
        choice: usize,
    },
}

/// One completely compiled triggered ability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggeredAbilityProgram {
    event: TriggeredEventProgram,
    required_alternate_cost: Option<AlternateCostKind>,
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
    unless_paid: Option<UnlessPaidProgram>,
}

/// One exact "unless that player pays" branch on a triggered ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnlessPaidProgram {
    payer: PlayerBinding,
    mana_cost: ManaCost,
    exact_payment: ManaPool,
}

impl UnlessPaidProgram {
    /// Returns the event-bound player allowed to pay.
    #[must_use]
    pub const fn payer(self) -> PlayerBinding {
        self.payer
    }

    /// Returns the exact payment cost.
    #[must_use]
    pub const fn mana_cost(self) -> ManaCost {
        self.mana_cost
    }

    /// Returns one deterministic pool that exactly pays the cost.
    #[must_use]
    pub const fn exact_payment(self) -> ManaPool {
        self.exact_payment
    }
}

/// Closed event families supported by a compiled triggered ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggeredEventProgram {
    /// This permanent enters the battlefield.
    SourceEnters,
    /// This creature attacks.
    SourceAttacks,
    /// The object currently equipped by this source attacks.
    AttachedObjectAttacks,
    /// This permanent's controller begins their upkeep.
    ControllerUpkeep,
    /// This permanent's controller casts a matching spell.
    ControllerCasts(ObjectTargetPredicate),
    /// This permanent's controller casts or copies a matching spell.
    ControllerCastsOrCopies(ObjectTargetPredicate),
    /// A matching permanent controlled by this source's controller deals combat damage to a player.
    ControllerPermanentDealsCombatDamageToPlayer(ObjectTargetPredicate),
    /// An opponent of this source's controller draws a card.
    OpponentDrawsCard,
    /// Another matching permanent controlled by this source's controller enters.
    ControllerPermanentEnters {
        /// Closed entering-permanent predicate.
        predicate: ObjectTargetPredicate,
        /// Whether the source itself is excluded.
        exclude_source: bool,
    },
}

/// One completely compiled non-mana activated ability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatedEffectProgram {
    mana_cost: ManaCost,
    exact_payment: ManaPool,
    tap_source: bool,
    sacrifice_source: bool,
    pay_life: u32,
    sacrifice_cost: Option<(ObjectTargetPredicate, u32)>,
    timing: ActivationTiming,
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
}

/// Exact hand-zone cycling cost compiled from the intrinsic keyword ability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CyclingProgram {
    mana_cost: ManaCost,
    exact_payment: ManaPool,
}

impl CyclingProgram {
    /// Returns the exact cycling mana cost.
    #[must_use]
    pub const fn mana_cost(self) -> ManaCost {
        self.mana_cost
    }

    /// Returns one deterministic exact payment pool for smoke synthesis.
    #[must_use]
    pub const fn exact_payment(self) -> ManaPool {
        self.exact_payment
    }
}

/// One source-bound static ability compiled without card branches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaticAbilityProgram {
    /// CR 613 operations over a live permanent predicate.
    Continuous {
        /// Predicate interpreted relative to the source controller.
        predicate: ObjectTargetPredicate,
        /// Whether the source is excluded from the affected set.
        exclude_source: bool,
        /// Layer operations registered in source order.
        operations: Vec<ContinuousEffectOperation>,
    },
    /// Generic reduction for matching spells while the source is on the battlefield.
    SpellCostReduction {
        /// Predicate interpreted relative to the source controller.
        predicate: ObjectTargetPredicate,
        /// Generic mana reduction.
        amount: u32,
    },
    /// A continuous rule change for the current controller of the source.
    PlayerRule {
        /// Closed player-level rule change.
        rule: PlayerRule,
    },
    /// Live characteristics and targeting restrictions for the attached object.
    AttachedObject {
        /// Layer operations following the current attachment.
        operations: Vec<ContinuousEffectOperation>,
        /// Targeting restrictions following the current attachment.
        restrictions: Vec<TargetRestriction>,
    },
    /// A combat declaration restriction on the source itself.
    SourceCombatRestriction {
        /// Closed combat restriction.
        restriction: CombatRestriction,
    },
    /// Remove types from the source while controller devotion remains below a threshold.
    DevotionSourceTypeRemoval {
        /// Colored mana symbol counted for devotion.
        color: ManaKind,
        /// Exclusive devotion threshold.
        threshold: u32,
        /// Types removed while the condition is true.
        types: ObjectTypes,
    },
}

/// One completely compiled additional spell cost.
// Closed predicates remain inline so this public value type stays Copy.
#[allow(clippy::large_enum_variant)]
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

/// Closed condition for offering an alternate casting cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlternateCostCondition {
    /// The caster currently controls a designated commander permanent.
    ControllerControlsCommander,
    /// The spell object is in its controller's graveyard and is cast with flashback.
    SourceInControllerGraveyard,
    /// The spell object is in its controller's hand.
    SourceInControllerHand,
}

/// Rule meaning attached to one alternate casting cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlternateCostKind {
    /// Commander-presence conditional cost.
    Commander,
    /// Graveyard flashback cost.
    Flashback,
    /// Permanent-spell evoke cost with a source-entered sacrifice trigger.
    Evoke,
    /// Target-to-each overload cost.
    Overload,
}

/// One completely compiled conditional alternate casting cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlternateCastCostProgram {
    kind: AlternateCostKind,
    condition: AlternateCostCondition,
    mana_cost: ManaCost,
    exact_payment: ManaPool,
}

impl AlternateCastCostProgram {
    /// Returns the rule meaning of this alternate cost.
    #[must_use]
    pub const fn kind(self) -> AlternateCostKind {
        self.kind
    }

    /// Returns the condition that makes this alternate cost available.
    #[must_use]
    pub const fn condition(self) -> AlternateCostCondition {
        self.condition
    }

    /// Returns the exact alternate mana cost.
    #[must_use]
    pub const fn mana_cost(self) -> ManaCost {
        self.mana_cost
    }

    /// Returns one exact deterministic payment pool for the alternate cost.
    #[must_use]
    pub const fn exact_payment(self) -> ManaPool {
        self.exact_payment
    }

    /// Returns whether the current state satisfies this alternate-cost condition.
    #[must_use]
    pub fn is_available(
        self,
        state: &GameState,
        controller: PlayerId,
        source: Option<ObjectId>,
    ) -> bool {
        match self.condition {
            AlternateCostCondition::ControllerControlsCommander => state
                .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
                .is_some_and(|objects| {
                    objects.iter().copied().any(|object| {
                        state.object(object).is_some_and(|record| {
                            record.controller() == controller && record.is_commander()
                        })
                    })
                }),
            AlternateCostCondition::SourceInControllerGraveyard => source.is_some_and(|source| {
                state
                    .object(source)
                    .is_some_and(|record| record.owner() == controller)
                    && state.object_zone(source)
                        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard))
            }),
            AlternateCostCondition::SourceInControllerHand => source.is_some_and(|source| {
                state
                    .object(source)
                    .is_some_and(|record| record.owner() == controller)
                    && state.object_zone(source)
                        == Some(ZoneId::new(Some(controller), ZoneKind::Hand))
            }),
        }
    }
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

    /// Returns an optional matching-permanent sacrifice cost.
    #[must_use]
    pub const fn sacrifice_cost(&self) -> Option<(ObjectTargetPredicate, u32)> {
        self.sacrifice_cost
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

    /// Returns whether execution requires the actual battlefield source object.
    #[must_use]
    pub fn uses_source_object(&self) -> bool {
        self.effects.iter().any(|effect| {
            matches!(
                effect,
                EffectProgram::AttachSourceToTarget { .. }
                    | EffectProgram::AddCountersToSource { .. }
            )
        })
    }
}

impl StaticAbilityProgram {
    /// Binds this static ability to one battlefield source.
    #[must_use]
    pub fn bind_actions(&self, controller: PlayerId, source: ObjectId) -> Vec<Action> {
        match self {
            Self::Continuous {
                predicate,
                exclude_source,
                operations,
            } => {
                let target = ContinuousEffectTarget::Objects {
                    predicate: *predicate,
                    excluded: exclude_source.then_some(source),
                };
                operations
                    .iter()
                    .copied()
                    .map(|operation| Action::RegisterContinuousEffect {
                        definition: ContinuousEffectDefinition::new(controller, target, operation)
                            .with_source(source)
                            .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield),
                    })
                    .collect()
            }
            Self::SpellCostReduction { predicate, amount } => {
                vec![Action::RegisterCostModifier {
                    definition: CostModifierDefinition::new(
                        controller,
                        Some(source),
                        CostModifierScope::Spells(*predicate),
                        CostModifierOperation::ReduceGeneric(*amount),
                    ),
                }]
            }
            Self::PlayerRule { rule } => vec![Action::RegisterRestriction {
                definition: RestrictionDefinition::new(
                    controller,
                    RestrictionEffect::PlayerRule {
                        subject: PlayerRuleSubject::ControllerOfSource,
                        rule: *rule,
                    },
                )
                .with_source(source),
            }],
            Self::AttachedObject {
                operations,
                restrictions,
            } => operations
                .iter()
                .copied()
                .map(|operation| Action::RegisterContinuousEffect {
                    definition: ContinuousEffectDefinition::new(
                        controller,
                        ContinuousEffectTarget::AttachedToSource,
                        operation,
                    )
                    .with_source(source)
                    .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield),
                })
                .chain(restrictions.iter().copied().map(|restriction| {
                    Action::RegisterRestriction {
                        definition: RestrictionDefinition::new(
                            controller,
                            RestrictionEffect::Targeting {
                                subject: TargetRestrictionSubject::AttachedToSource,
                                restriction,
                            },
                        )
                        .with_source(source),
                    }
                }))
                .collect(),
            Self::SourceCombatRestriction { restriction } => {
                vec![Action::RegisterRestriction {
                    definition: RestrictionDefinition::new(
                        controller,
                        RestrictionEffect::Combat {
                            subject: CombatRestrictionSubject::Object(source),
                            restriction: *restriction,
                        },
                    )
                    .with_source(source),
                }]
            }
            Self::DevotionSourceTypeRemoval {
                color,
                threshold,
                types,
            } => vec![Action::RegisterContinuousEffect {
                definition: ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(source),
                    ContinuousEffectOperation::RemoveTypes { types: *types },
                )
                .with_source(source)
                .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield)
                .with_condition(
                    ContinuousEffectCondition::ControllerDevotionLessThan {
                        color: *color,
                        threshold: *threshold,
                    },
                ),
            }],
        }
    }

    /// Returns the number of production registrations emitted by this ability.
    #[must_use]
    pub fn operation_count(&self) -> usize {
        match self {
            Self::Continuous { operations, .. } => operations.len(),
            Self::SpellCostReduction { .. } => 1,
            Self::PlayerRule { .. } => 1,
            Self::AttachedObject {
                operations,
                restrictions,
            } => operations.len().saturating_add(restrictions.len()),
            Self::SourceCombatRestriction { .. } => 1,
            Self::DevotionSourceTypeRemoval { .. } => 1,
        }
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
            TriggeredEventProgram::AttachedObjectAttacks => TriggerCondition::AttackDeclared {
                attacker: TriggerObjectFilter::AttachedToSource,
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
            TriggeredEventProgram::ControllerCastsOrCopies(predicate) => {
                TriggerCondition::StackEntryAddedOrCopied {
                    controller: TriggerPlayerFilter::Controller,
                    required_types: predicate.required_types(),
                    required_any_types: predicate.required_any_types(),
                    forbidden_types: predicate.forbidden_types(),
                }
            }
            TriggeredEventProgram::ControllerPermanentDealsCombatDamageToPlayer(source) => {
                TriggerCondition::CombatDamageToPlayer { source }
            }
            TriggeredEventProgram::OpponentDrawsCard => TriggerCondition::PlayerDrewCard {
                player: TriggerPlayerFilter::OpponentOfController,
            },
            TriggeredEventProgram::ControllerPermanentEnters {
                predicate,
                exclude_source,
            } => TriggerCondition::PermanentEnteredBattlefield {
                predicate,
                exclude_source,
            },
        };
        TriggerDefinition::new(controller, condition).with_source(source)
    }

    /// Returns the closed event family that queues this trigger.
    #[must_use]
    pub const fn event(&self) -> TriggeredEventProgram {
        self.event
    }

    /// Returns the alternate cost that must have been selected for this trigger.
    #[must_use]
    pub const fn required_alternate_cost(&self) -> Option<AlternateCostKind> {
        self.required_alternate_cost
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

    /// Returns the exact conditional payment branch, when present.
    #[must_use]
    pub const fn unless_paid(&self) -> Option<UnlessPaidProgram> {
        self.unless_paid
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OptionalEffectGroup {
    start: usize,
    end: usize,
}

/// One complete branch of a modal spell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpellModeProgram {
    target_requirements: Vec<TargetRequirement>,
    object_choice_requirements: Vec<ObjectChoiceRequirement>,
    effects: Vec<EffectProgram>,
    optional_effect_groups: Vec<OptionalEffectGroup>,
}

impl SpellModeProgram {
    /// Returns target slots announced only when this mode is selected.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns hidden-zone choices announced only when this mode is selected.
    #[must_use]
    pub fn object_choice_requirements(&self) -> &[ObjectChoiceRequirement] {
        &self.object_choice_requirements
    }

    /// Returns this mode's effect operations in source order.
    #[must_use]
    pub fn effects(&self) -> &[EffectProgram] {
        &self.effects
    }

    /// Returns this mode's explicit optional-effect decision count.
    #[must_use]
    pub fn optional_choice_count(&self) -> usize {
        self.optional_effect_groups.len()
    }
}

impl EffectProgram {
    const fn capability(&self) -> Capability {
        match self {
            Self::GainLife { .. } => Capability::GainLife,
            Self::LoseLife { .. } => Capability::LoseLife,
            Self::DealDamageToTarget { .. } => Capability::DealDamage,
            Self::DealDamageToPlayers { .. } => Capability::DealDamage,
            Self::DrawCards { .. } => Capability::DrawCards,
            Self::DiscardHands { .. } => Capability::DiscardCards,
            Self::Scry { .. } => Capability::Scry,
            Self::ShuffleLibrary { .. } => Capability::ShuffleLibrary,
            Self::DestroyPermanent { .. } | Self::DestroyPermanentWithoutRegeneration { .. } => {
                Capability::DestroyPermanent
            }
            Self::ExileObject { .. } => Capability::ExileObject,
            Self::CounterStackEntry { .. } => Capability::CounterStackEntry,
            Self::MoveTargetObject { .. } => Capability::MoveZone,
            Self::SacrificeSource => Capability::SacrificePermanent,
            Self::CreateTokens { .. } => Capability::CreateToken,
            Self::SearchLibrary { .. } => Capability::SearchLibrary,
            Self::MoveChosenObjects { .. } => Capability::MoveZone,
            Self::TapChosenObjects { .. } => Capability::TapObject,
            Self::DiscardChosenObjects { .. } => Capability::DiscardCards,
            Self::ModifyPowerToughness { .. } | Self::GrantKeywords { .. } => {
                Capability::ModifyCharacteristics
            }
            Self::GrantTargetingRestriction { .. } => Capability::TargetingRestriction,
            Self::GrantIndestructible { .. } => Capability::Indestructible,
            Self::AttachSourceToTarget { .. } => Capability::AttachObject,
            Self::AddCountersToSource { .. } => Capability::AddCounters,
            Self::RevealChosenObjects { .. } => Capability::RevealObjects,
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
    spell_modes: Vec<SpellModeProgram>,
    additional_costs: Vec<SpellAdditionalCostProgram>,
    alternate_costs: Vec<AlternateCastCostProgram>,
    overload: bool,
    split_second: bool,
    cycling: Option<CyclingProgram>,
    activated_abilities: Vec<ActivatedAbilityProgram>,
    activated_effects: Vec<ActivatedEffectProgram>,
    triggered_abilities: Vec<TriggeredAbilityProgram>,
    static_abilities: Vec<StaticAbilityProgram>,
    modal_dfc_back: Option<Box<CardProgram>>,
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

    /// Returns target slots for the selected casting mode.
    #[must_use]
    pub fn target_requirements_for_alternate(
        &self,
        alternate: Option<AlternateCostKind>,
    ) -> &[TargetRequirement] {
        if alternate == Some(AlternateCostKind::Overload) && self.overload {
            &[]
        } else {
            &self.target_requirements
        }
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

    /// Returns the complete modal branches in printed order.
    #[must_use]
    pub fn spell_modes(&self) -> &[SpellModeProgram] {
        &self.spell_modes
    }

    /// Returns additional spell costs in announcement order.
    #[must_use]
    pub fn additional_costs(&self) -> &[SpellAdditionalCostProgram] {
        &self.additional_costs
    }

    /// Returns conditional alternate casting costs in printed order.
    #[must_use]
    pub fn alternate_costs(&self) -> &[AlternateCastCostProgram] {
        &self.alternate_costs
    }

    /// Returns true when this spell has a complete overload lowering.
    #[must_use]
    pub const fn overload(&self) -> bool {
        self.overload
    }

    /// Returns true when this spell carries split second on the stack.
    #[must_use]
    pub const fn split_second(&self) -> bool {
        self.split_second
    }

    /// Returns the exact hand-zone cycling program, when present.
    #[must_use]
    pub const fn cycling(&self) -> Option<CyclingProgram> {
        self.cycling
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

    /// Returns source-bound static continuous abilities in printed order.
    #[must_use]
    pub fn static_abilities(&self) -> &[StaticAbilityProgram] {
        &self.static_abilities
    }

    /// Returns the independently compiled back-face program of a modal DFC.
    #[must_use]
    pub fn modal_dfc_back(&self) -> Option<&CardProgram> {
        self.modal_dfc_back.as_deref()
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
        if self.cycling.is_some() {
            capabilities.push(Capability::Cycling);
        }
        capabilities.extend(
            self.alternate_costs
                .iter()
                .map(|_| Capability::AlternateCost),
        );
        if self.overload {
            capabilities.push(Capability::Overload);
        }
        if self.split_second {
            capabilities.push(Capability::SplitSecond);
        }
        if !self.spell_modes.is_empty() {
            capabilities.push(Capability::ChooseMode);
        }
        for ability in &self.static_abilities {
            match ability {
                StaticAbilityProgram::Continuous { operations, .. } => capabilities
                    .extend(operations.iter().map(|_| Capability::ModifyCharacteristics)),
                StaticAbilityProgram::SpellCostReduction { .. } => {
                    capabilities.push(Capability::ReduceSpellCost);
                }
                StaticAbilityProgram::PlayerRule { .. } => {
                    capabilities.push(Capability::ModifyPlayerRules);
                }
                StaticAbilityProgram::AttachedObject {
                    operations,
                    restrictions,
                } => {
                    capabilities
                        .extend(operations.iter().map(|_| Capability::ModifyCharacteristics));
                    capabilities.extend(
                        restrictions
                            .iter()
                            .map(|_| Capability::TargetingRestriction),
                    );
                }
                StaticAbilityProgram::SourceCombatRestriction { .. } => {
                    capabilities.push(Capability::CombatRestriction);
                }
                StaticAbilityProgram::DevotionSourceTypeRemoval { .. } => {
                    capabilities.push(Capability::ModifyCharacteristics);
                }
            }
        }
        capabilities.extend(self.effects.iter().map(EffectProgram::capability));
        capabilities.extend(
            self.spell_modes
                .iter()
                .flat_map(|mode| mode.effects.iter().map(EffectProgram::capability)),
        );
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
        if let Some(back) = &self.modal_dfc_back {
            let mut combined = Vec::with_capacity(
                capabilities
                    .len()
                    .saturating_add(back.capabilities().len())
                    .saturating_add(1),
            );
            combined.push(Capability::ModalDfc);
            combined.extend(capabilities);
            combined.extend(back.capabilities());
            return combined;
        }
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
    if definition.layout == CardLayout::ModalDfc {
        return compile_modal_dfc_program(definition);
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
    let mut base_object = compile_base_object(
        &face.type_line.supertypes,
        &face.type_line.card_types,
        &face.type_line.subtypes,
        &face.mana_cost.symbols,
        mana_value,
    )?;
    let flashback_count = face
        .keywords
        .iter()
        .filter(|keyword| keyword.as_str() == "flashback")
        .count();
    let overload_count = face
        .keywords
        .iter()
        .filter(|keyword| keyword.as_str() == "overload")
        .count();
    let evoke_count = face
        .keywords
        .iter()
        .filter(|keyword| keyword.as_str() == "evoke")
        .count();
    if flashback_count > 1
        || overload_count > 1
        || evoke_count > 1
        || flashback_count
            .saturating_add(overload_count)
            .saturating_add(evoke_count)
            > 1
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "exactly one supported intrinsic alternate-cost keyword is allowed",
        ));
    }
    let has_flashback_keyword = flashback_count == 1;
    let overload = overload_count == 1;
    let evoke = evoke_count == 1;
    if overload && !matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "overload is valid only on instant or sorcery spells",
        ));
    }
    if evoke && kind != ProgramKind::Permanent {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "evoke is valid only on permanent spells",
        ));
    }
    let alternate_keyword = if has_flashback_keyword {
        Some(AlternateCostKind::Flashback)
    } else if overload {
        Some(AlternateCostKind::Overload)
    } else if evoke {
        Some(AlternateCostKind::Evoke)
    } else {
        None
    };
    let split_second_count = face
        .keywords
        .iter()
        .filter(|keyword| keyword.as_str() == "split_second")
        .count();
    if split_second_count > 1 {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "split_second must appear exactly once when present",
        ));
    }
    let split_second = split_second_count == 1;
    if split_second && !matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "split_second is valid only on instant or sorcery spells",
        ));
    }
    let cycling_count = face
        .keywords
        .iter()
        .filter(|keyword| keyword.as_str() == "cycling")
        .count();
    if cycling_count > 1 {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "cycling must appear exactly once when present",
        ));
    }
    let has_cycling_keyword = cycling_count == 1;
    let intrinsic_keywords = face
        .keywords
        .iter()
        .filter(|keyword| {
            !matches!(
                keyword.as_str(),
                "cycling" | "evoke" | "flashback" | "overload" | "split_second"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let base_creature = compile_base_creature(
        &face.type_line.card_types,
        face.power.as_deref(),
        face.toughness.as_deref(),
        &intrinsic_keywords,
    )?;
    let has_equip_keyword = face
        .keywords
        .iter()
        .any(|keyword| keyword.as_str() == "equip");
    let mut compiler = ProgramCompiler::default();
    let mut activated_abilities =
        compile_intrinsic_basic_mana_ability(&face.type_line.supertypes, &face.type_line.subtypes)?
            .into_iter()
            .collect::<Vec<_>>();
    let mut activated_effects = Vec::new();
    let mut triggered_abilities = Vec::new();
    let mut static_abilities = Vec::new();
    let mut additional_costs = Vec::new();
    let mut alternate_costs = Vec::new();
    let mut spell_modes = Vec::new();
    let mut cycling = None;
    let mut enters_tapped = false;
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
                if matches!(
                    ability.effect,
                    Expression::Call {
                        operation: Operation::ChooseOne,
                        ..
                    }
                ) {
                    spell_modes = compile_spell_modes(&ability.effect, &format!("{path}.effect"))?;
                } else {
                    compile_effect(&ability.effect, &format!("{path}.effect"), &mut compiler)?;
                }
            }
            AbilityKind::Activated
                if matches!(kind, ProgramKind::Permanent | ProgramKind::Land) =>
            {
                if ability.mana_ability {
                    activated_abilities.push(compile_fixed_mana_ability(ability, &path)?);
                } else if has_cycling_keyword && ability_has_discard_source_cost(ability) {
                    if cycling.is_some() {
                        return Err(CompileDiagnostic::new(
                            CompileDiagnosticCode::KeywordSemantics,
                            path,
                            "card repeats its intrinsic cycling ability",
                        ));
                    }
                    cycling = Some(compile_cycling_ability(ability, &path)?);
                } else {
                    activated_effects.push(compile_activated_effect(ability, &path)?);
                }
            }
            AbilityKind::Triggered if matches!(kind, ProgramKind::Permanent) => {
                triggered_abilities.push(compile_triggered_ability(ability, &path)?);
            }
            AbilityKind::Static
                if kind == ProgramKind::Permanent
                    && alternate_keyword == Some(AlternateCostKind::Evoke) =>
            {
                alternate_costs.push(compile_spell_alternate_cost(
                    ability,
                    &path,
                    alternate_keyword,
                )?);
            }
            AbilityKind::Static if matches!(kind, ProgramKind::Permanent | ProgramKind::Land) => {
                static_abilities.push(compile_static_ability(ability, &path)?);
            }
            AbilityKind::Static if matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery) => {
                alternate_costs.push(compile_spell_alternate_cost(
                    ability,
                    &path,
                    alternate_keyword,
                )?);
            }
            AbilityKind::Replacement
                if matches!(kind, ProgramKind::Permanent | ProgramKind::Land) =>
            {
                if enters_tapped {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        path,
                        "multiple enters-tapped replacements are not compiled",
                    ));
                }
                compile_enters_tapped_replacement(ability, &path)?;
                enters_tapped = true;
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
    if enters_tapped {
        base_object = base_object.with_enters_tapped();
    }
    if has_equip_keyword
        && !activated_effects.iter().any(|ability| {
            ability.timing() == ActivationTiming::Sorcery && ability.uses_source_object()
        })
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "equip requires a sorcery-speed source-to-target attachment ability",
        ));
    }
    if has_cycling_keyword && cycling.is_none() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "cycling requires one exact mana-plus-source-discard draw ability",
        ));
    }
    if has_flashback_keyword
        && !alternate_costs
            .iter()
            .any(|cost| cost.kind == AlternateCostKind::Flashback)
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "flashback requires an exact source-bound graveyard alternate cost",
        ));
    }
    if overload
        && !alternate_costs
            .iter()
            .any(|cost| cost.kind == AlternateCostKind::Overload)
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "overload requires an exact source-bound hand alternate cost",
        ));
    }
    if evoke
        && !alternate_costs
            .iter()
            .any(|cost| cost.kind == AlternateCostKind::Evoke)
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            "evoke requires an exact source-bound hand alternate cost",
        ));
    }
    if overload
        && !matches!(
            compiler.effects.as_slice(),
            [EffectProgram::MoveTargetObject {
                overload_predicate: Some(_),
                ..
            }]
        )
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].abilities",
            "overload currently requires one exact target-to-each object move",
        ));
    }
    if evoke {
        triggered_abilities.push(TriggeredAbilityProgram {
            event: TriggeredEventProgram::SourceEnters,
            required_alternate_cost: Some(AlternateCostKind::Evoke),
            target_requirements: Vec::new(),
            object_choice_requirements: Vec::new(),
            effects: vec![EffectProgram::SacrificeSource],
            optional_effect_groups: Vec::new(),
            unless_paid: None,
        });
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
    let compiled_effect_count = compiled_effect_count.saturating_add(
        spell_modes
            .iter()
            .map(|mode: &SpellModeProgram| mode.effects.len())
            .sum::<usize>(),
    );
    let compiled_effect_count = compiled_effect_count.saturating_add(
        static_abilities
            .iter()
            .map(StaticAbilityProgram::operation_count)
            .sum::<usize>(),
    );
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
    if compiler.effects.is_empty()
        && spell_modes.is_empty()
        && matches!(kind, ProgramKind::Instant | ProgramKind::Sorcery)
    {
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
        spell_modes,
        additional_costs,
        alternate_costs,
        overload,
        split_second,
        cycling,
        activated_abilities,
        activated_effects,
        triggered_abilities,
        static_abilities,
        modal_dfc_back: None,
    })
}

fn compile_modal_dfc_program(
    definition: &CardDefinition,
) -> Result<CardProgram, CompileDiagnostic> {
    let [front_face, back_face] = definition.faces.as_slice() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::FaceCount,
            "card.faces",
            format!(
                "modal_dfc requires exactly two ordered faces, found {}",
                definition.faces.len()
            ),
        ));
    };
    let compile_face = |face: &forge_carddef::CardFace| {
        compile_card_program(&CardDefinition {
            id: definition.id.clone(),
            name: face.name.clone(),
            layout: CardLayout::Normal,
            status: definition.status.clone(),
            faces: vec![face.clone()],
        })
    };
    let mut front = compile_face(front_face)
        .map_err(|diagnostic| repath_modal_face_diagnostic(diagnostic, 0))?;
    let back = compile_face(back_face)
        .map_err(|diagnostic| repath_modal_face_diagnostic(diagnostic, 1))?;
    front.name = definition.name.clone();
    front.modal_dfc_back = Some(Box::new(back));
    Ok(front)
}

fn repath_modal_face_diagnostic(
    diagnostic: CompileDiagnostic,
    face_index: usize,
) -> CompileDiagnostic {
    let suffix = diagnostic
        .path()
        .strip_prefix("card.faces[0]")
        .unwrap_or_else(|| {
            diagnostic
                .path()
                .strip_prefix("card")
                .unwrap_or(diagnostic.path())
        });
    CompileDiagnostic::new(
        diagnostic.code(),
        format!("card.faces[{face_index}]{suffix}"),
        diagnostic.detail(),
    )
}

fn compile_enters_tapped_replacement(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<(), CompileDiagnostic> {
    if !ability.costs.is_empty()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "enters-tapped replacement must have no costs, condition, timing, or mana flag",
        ));
    }
    let Some(Expression::Call {
        operation: Operation::EventEnters,
        arguments: event_arguments,
    }) = ability.event.as_ref()
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.event"),
            "replacement event must be event_enters(source())",
        ));
    };
    if !matches!(event_arguments.as_slice(), [source] if is_source_selector(source)) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.event"),
            "replacement event must bind exactly source()",
        ));
    }
    let Expression::Call {
        operation: Operation::EtbEffect,
        arguments: etb_arguments,
    } = &ability.effect
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect"),
            "enters-tapped replacement must use etb_effect(tap(source()))",
        ));
    };
    let [Expression::Call {
        operation: Operation::Tap,
        arguments: tap_arguments,
    }] = etb_arguments.as_slice()
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect"),
            "etb_effect must contain exactly tap(source())",
        ));
    };
    if !matches!(tap_arguments.as_slice(), [source] if is_source_selector(source)) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect"),
            "enters-tapped tap must bind exactly source()",
        ));
    }
    Ok(())
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
    let unless_paid =
        compile_triggered_effect(&ability.effect, &format!("{path}.effect"), &mut compiler)?;
    if compiler.effects.is_empty() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.effect"),
            "trigger compiled no executable effect",
        ));
    }
    Ok(TriggeredAbilityProgram {
        event,
        required_alternate_cost: None,
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
        unless_paid,
    })
}

fn compile_triggered_effect(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<Option<UnlessPaidProgram>, CompileDiagnostic> {
    let Expression::Call {
        operation: Operation::UnlessPaid,
        arguments,
    } = expression
    else {
        compile_effect(expression, path, compiler)?;
        return Ok(None);
    };
    let [effect, payer, cost] = arguments.as_slice() else {
        return Err(effect_arity(
            path,
            &Operation::UnlessPaid,
            "effect, triggering player, and mana_cost(...) ",
        ));
    };
    compile_effect(effect, &format!("{path}.effect"), compiler)?;
    let payer = compile_player_binding(payer, &format!("{path}.payer"), compiler)?;
    if payer != PlayerBinding::TriggeringPlayer {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::PlayerSelector,
            format!("{path}.payer"),
            "unless_paid payer must be controller_of(triggered())",
        ));
    }
    let Expression::Call {
        operation: Operation::ManaCost,
        arguments,
    } = cost
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.cost"),
            "unless_paid cost must be mana_cost(...) ",
        ));
    };
    let [Expression::Text(value)] = arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.cost"),
            &Operation::ManaCost,
            "one exact mana-cost string",
        ));
    };
    let (mana_cost, exact_payment) = compile_mana_cost_text(value, &format!("{path}.cost"))?;
    Ok(Some(UnlessPaidProgram {
        payer,
        mana_cost,
        exact_payment,
    }))
}

fn compile_spell_alternate_cost(
    ability: &AbilityDefinition,
    path: &str,
    alternate_keyword: Option<AlternateCostKind>,
) -> Result<AlternateCastCostProgram, CompileDiagnostic> {
    if !ability.costs.is_empty()
        || ability.event.is_some()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "spell alternate-cost ability must have no costs, event, condition, timing, or mana flag",
        ));
    }
    let (kind, condition, alternate_cost, source_selector) = match &ability.effect {
        Expression::Call {
            operation: Operation::WhileCondition,
            arguments,
        } => {
            let [condition, alternate_cost] = arguments.as_slice() else {
                return Err(effect_arity(
                    &format!("{path}.effect"),
                    &Operation::WhileCondition,
                    "commander-control condition and alternate_cost(...) ",
                ));
            };
            if !is_controller_controls_commander_condition(condition) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.effect.condition"),
                    "alternate cost requires exactly controller-controls-a-commander",
                ));
            }
            (
                AlternateCostKind::Commander,
                AlternateCostCondition::ControllerControlsCommander,
                alternate_cost,
                false,
            )
        }
        Expression::Call {
            operation: Operation::Continuous,
            arguments,
        } if alternate_keyword.is_some() => {
            let [source, alternate_cost] = arguments.as_slice() else {
                return Err(effect_arity(
                    &format!("{path}.effect"),
                    &Operation::Continuous,
                    "source() and alternate_cost(...) ",
                ));
            };
            if !is_source_selector(source) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.effect.source"),
                    "intrinsic alternate cost must be bound to source()",
                ));
            }
            let kind = alternate_keyword.ok_or_else(|| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    format!("{path}.effect"),
                    "intrinsic alternate cost requires a recognized keyword",
                )
            })?;
            let condition = match kind {
                AlternateCostKind::Flashback => AlternateCostCondition::SourceInControllerGraveyard,
                AlternateCostKind::Evoke | AlternateCostKind::Overload => {
                    AlternateCostCondition::SourceInControllerHand
                }
                AlternateCostKind::Commander => {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        format!("{path}.effect"),
                        "commander alternate cost requires a closed while_condition",
                    ));
                }
            };
            (kind, condition, alternate_cost, true)
        }
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                format!("{path}.effect"),
                "spell static ability must be a closed conditional or intrinsic alternate cost",
            ));
        }
    };
    let Expression::Call {
        operation: Operation::AlternateCost,
        arguments,
    } = alternate_cost
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.alternate_cost"),
            "conditional effect is not alternate_cost(...) ",
        ));
    };
    let [selector, mana] = arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect.alternate_cost"),
            &Operation::AlternateCost,
            "this spell selector and mana_cost(...) ",
        ));
    };
    let valid_selector = if source_selector {
        is_source_selector(selector)
    } else {
        is_this_spell_controlled_by_you(selector)
    };
    if !valid_selector {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.alternate_cost.selector"),
            "alternate cost must select exactly its source spell",
        ));
    }
    let Expression::Call {
        operation: Operation::ManaCost,
        arguments,
    } = mana
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.alternate_cost.cost"),
            "alternate cost is not mana_cost(...) ",
        ));
    };
    let [Expression::Text(value)] = arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect.alternate_cost.cost"),
            &Operation::ManaCost,
            "one exact mana-cost string",
        ));
    };
    let (mana_cost, exact_payment) =
        compile_mana_cost_text(value, &format!("{path}.effect.alternate_cost.cost"))?;
    Ok(AlternateCastCostProgram {
        kind,
        condition,
        mana_cost,
        exact_payment,
    })
}

fn is_controller_controls_commander_condition(expression: &Expression) -> bool {
    let Expression::Call {
        operation: Operation::AtLeast,
        arguments,
    } = expression
    else {
        return false;
    };
    let [Expression::Call {
        operation: Operation::Count,
        arguments: count,
    }, Expression::Integer(1)] = arguments.as_slice()
    else {
        return false;
    };
    let [Expression::Call {
        operation: Operation::Permanents,
        arguments: permanents,
    }] = count.as_slice()
    else {
        return false;
    };
    let [Expression::Call {
        operation: Operation::And,
        arguments: predicates,
    }] = permanents.as_slice()
    else {
        return false;
    };
    predicates.len() == 2 && predicates.iter().any(|predicate| {
        matches!(
            predicate,
            Expression::Call {
                operation: Operation::DesignationIs,
                arguments,
            } if matches!(arguments.as_slice(), [Expression::Text(value)] if value == "commander")
        )
    }) && predicates.iter().any(|predicate| {
        matches!(
            predicate,
            Expression::Call {
                operation: Operation::ControlledBy,
                arguments,
            } if matches!(arguments.as_slice(), [you] if is_you_selector(you))
        )
    })
}

fn is_this_spell_controlled_by_you(expression: &Expression) -> bool {
    let Expression::Call {
        operation: Operation::Spells,
        arguments,
    } = expression
    else {
        return false;
    };
    let [Expression::Call {
        operation: Operation::And,
        arguments: predicates,
    }] = arguments.as_slice()
    else {
        return false;
    };
    predicates.len() == 2
        && predicates.iter().any(|predicate| {
            matches!(
                predicate,
                Expression::Call {
                    operation: Operation::Equals,
                    arguments,
                } if matches!(arguments.as_slice(), [left, right]
                    if (is_any_selector(left) && is_source_selector(right))
                        || (is_source_selector(left) && is_any_selector(right)))
            )
        })
        && predicates.iter().any(|predicate| {
            matches!(
                predicate,
                Expression::Call {
                    operation: Operation::ControlledBy,
                    arguments,
                } if matches!(arguments.as_slice(), [you] if is_you_selector(you))
            )
        })
}

fn compile_static_ability(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<StaticAbilityProgram, CompileDiagnostic> {
    if !ability.costs.is_empty()
        || ability.event.is_some()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "static continuous ability must have no costs, event, condition, timing, or mana flag",
        ));
    }
    if matches!(
        &ability.effect,
        Expression::Call {
            operation: Operation::WhileCondition,
            ..
        }
    ) {
        return compile_devotion_source_type_removal(&ability.effect, &format!("{path}.effect"));
    }
    let Expression::Call {
        operation: Operation::Continuous,
        arguments,
    } = &ability.effect
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            format!("{path}.effect"),
            "static ability must be rooted in continuous(selector, operation)",
        ));
    };
    let [selector, effect] = arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect"),
            &Operation::Continuous,
            "one permanent selector and one continuous operation",
        ));
    };
    if is_you_selector(selector) {
        let Expression::Call {
            operation: Operation::NoMaximumHandSize,
            arguments,
        } = effect
        else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectOperation,
                format!("{path}.effect.operation"),
                "player continuous effects currently require no_maximum_hand_size(you())",
            ));
        };
        let [affected] = arguments.as_slice() else {
            return Err(effect_arity(
                &format!("{path}.effect.operation"),
                &Operation::NoMaximumHandSize,
                "you()",
            ));
        };
        if !is_you_selector(affected) {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::PlayerSelector,
                format!("{path}.effect.operation.player"),
                "no-maximum-hand-size source and affected player must both be you()",
            ));
        }
        return Ok(StaticAbilityProgram::PlayerRule {
            rule: PlayerRule::NoMaximumHandSize,
        });
    }
    if is_source_selector(selector) {
        let Expression::Call {
            operation: Operation::CannotBlock,
            arguments,
        } = effect
        else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectOperation,
                format!("{path}.effect.operation"),
                "source-bound static effects currently require cannot_block(any())",
            ));
        };
        let [subject] = arguments.as_slice() else {
            return Err(effect_arity(
                &format!("{path}.effect.operation"),
                &Operation::CannotBlock,
                "any()",
            ));
        };
        if !is_any_selector(subject) {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                format!("{path}.effect.operation.subject"),
                "source combat restriction must apply to any() selected source",
            ));
        }
        return Ok(StaticAbilityProgram::SourceCombatRestriction {
            restriction: CombatRestriction::CannotBlock,
        });
    }
    if is_equipped_object_of_source(selector) {
        let mut operations = Vec::new();
        let mut restrictions = Vec::new();
        compile_attached_static_operations(
            effect,
            &format!("{path}.effect.operation"),
            &mut operations,
            &mut restrictions,
        )?;
        let operation_count = operations.len().saturating_add(restrictions.len());
        if operation_count == 0 || operation_count > MAX_EFFECTS {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::ProgramBounds,
                format!("{path}.effect.operation"),
                format!("attached-object static ability compiled {operation_count} operations"),
            ));
        }
        return Ok(StaticAbilityProgram::AttachedObject {
            operations,
            restrictions,
        });
    }
    let spec = compile_object_selector(selector, &format!("{path}.effect.selector"))?;
    let kind = spec.kind;
    let exclude_source = spec.exclude_source;
    let predicate = object_predicate_from_spec(spec);
    if kind == TargetKind::StackEntry {
        if exclude_source {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                format!("{path}.effect.selector"),
                "spell-cost predicates cannot exclude the static source",
            ));
        }
        let Expression::Call {
            operation: Operation::CostReduction,
            arguments,
        } = effect
        else {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectOperation,
                format!("{path}.effect.operation"),
                "spell continuous effects currently require cost_reduction(any(), amount)",
            ));
        };
        let [subject, Expression::Integer(amount)] = arguments.as_slice() else {
            return Err(effect_arity(
                &format!("{path}.effect.operation"),
                &Operation::CostReduction,
                "any() and one positive generic amount",
            ));
        };
        if !is_any_selector(subject) {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                format!("{path}.effect.operation.subject"),
                "cost reduction must apply to any() selected spell",
            ));
        }
        let amount = u32::try_from(*amount).map_err(|_| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::EffectAmount,
                format!("{path}.effect.operation.amount"),
                "spell-cost reduction is outside the u32 range",
            )
        })?;
        if amount == 0 {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectAmount,
                format!("{path}.effect.operation.amount"),
                "spell-cost reduction must be positive",
            ));
        }
        return Ok(StaticAbilityProgram::SpellCostReduction { predicate, amount });
    }
    if kind != TargetKind::Permanent {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.selector"),
            "source-bound continuous effects require a permanent or spell selector",
        ));
    }
    let mut operations = Vec::new();
    compile_static_operations(effect, &format!("{path}.effect.operation"), &mut operations)?;
    if operations.is_empty() || operations.len() > MAX_EFFECTS {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            format!("{path}.effect.operation"),
            format!("static ability compiled {} operations", operations.len()),
        ));
    }
    Ok(StaticAbilityProgram::Continuous {
        predicate,
        exclude_source,
        operations,
    })
}

fn compile_devotion_source_type_removal(
    expression: &Expression,
    path: &str,
) -> Result<StaticAbilityProgram, CompileDiagnostic> {
    let Expression::Call {
        operation: Operation::WhileCondition,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            path,
            "conditional static ability is not while_condition(...) ",
        ));
    };
    let [condition, continuous] = arguments.as_slice() else {
        return Err(effect_arity(
            path,
            &Operation::WhileCondition,
            "devotion comparison and one continuous source operation",
        ));
    };
    let Expression::Call {
        operation: Operation::LessThan,
        arguments: comparison,
    } = condition
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.condition"),
            "conditional type removal requires less_than(devotion(...), threshold)",
        ));
    };
    let [devotion, Expression::Integer(threshold)] = comparison.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.condition"),
            &Operation::LessThan,
            "devotion(you(), color) and a positive threshold",
        ));
    };
    let threshold = u32::try_from(*threshold).map_err(|_| {
        CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            format!("{path}.condition.threshold"),
            "devotion threshold is outside the u32 range",
        )
    })?;
    if threshold == 0 {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            format!("{path}.condition.threshold"),
            "devotion threshold must be positive",
        ));
    }
    let Expression::Call {
        operation: Operation::Devotion,
        arguments: devotion_arguments,
    } = devotion
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.condition.devotion"),
            "condition requires devotion(you(), color)",
        ));
    };
    let [player, Expression::Text(color)] = devotion_arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.condition.devotion"),
            &Operation::Devotion,
            "you() and one color name",
        ));
    };
    if !is_you_selector(player) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::PlayerSelector,
            format!("{path}.condition.devotion.player"),
            "devotion condition currently requires you()",
        ));
    }
    let color = match color.as_str() {
        "white" => ManaKind::White,
        "blue" => ManaKind::Blue,
        "black" => ManaKind::Black,
        "red" => ManaKind::Red,
        "green" => ManaKind::Green,
        unsupported => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                format!("{path}.condition.devotion.color"),
                format!("devotion color `{unsupported}` is not represented"),
            ));
        }
    };
    let Expression::Call {
        operation: Operation::Continuous,
        arguments: continuous_arguments,
    } = continuous
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            format!("{path}.effect"),
            "devotion condition must wrap continuous(source(), remove_type(...))",
        ));
    };
    let [source, removal] = continuous_arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect"),
            &Operation::Continuous,
            "source() and remove_type(any(), type)",
        ));
    };
    if !is_source_selector(source) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.source"),
            "devotion type removal must affect source()",
        ));
    }
    let Expression::Call {
        operation: Operation::RemoveType,
        arguments: removal_arguments,
    } = removal
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            format!("{path}.effect.operation"),
            "devotion continuous effect must be remove_type(any(), type)",
        ));
    };
    let [subject, Expression::Text(removed)] = removal_arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.effect.operation"),
            &Operation::RemoveType,
            "any() and one represented type",
        ));
    };
    if !is_any_selector(subject) || removed != "Creature" {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect.operation"),
            "devotion type removal currently requires remove_type(any(), \"Creature\")",
        ));
    }
    Ok(StaticAbilityProgram::DevotionSourceTypeRemoval {
        color,
        threshold,
        types: ObjectTypes::none().with_creature(),
    })
}

fn compile_static_operations(
    expression: &Expression,
    path: &str,
    operations: &mut Vec<ContinuousEffectOperation>,
) -> Result<(), CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "continuous operation is not a call",
        ));
    };
    match operation {
        Operation::Sequence => {
            for (index, operation) in arguments.iter().enumerate() {
                compile_static_operations(
                    operation,
                    &format!("{path}.sequence[{index}]"),
                    operations,
                )?;
            }
            Ok(())
        }
        Operation::ModifyPt => {
            let [subject, Expression::Integer(power), Expression::Integer(toughness)] =
                arguments.as_slice()
            else {
                return Err(effect_arity(
                    path,
                    operation,
                    "any(), literal power delta, and literal toughness delta",
                ));
            };
            if !is_any_selector(subject) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.subject"),
                    "nested continuous operation must apply to any() selected object",
                ));
            }
            let power = i32::try_from(*power).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    format!("{path}.power"),
                    "static power delta does not fit i32",
                )
            })?;
            let toughness = i32::try_from(*toughness).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    format!("{path}.toughness"),
                    "static toughness delta does not fit i32",
                )
            })?;
            operations.push(ContinuousEffectOperation::ModifyPowerToughness { power, toughness });
            Ok(())
        }
        Operation::GrantKeyword => {
            let [subject, keyword] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "any() and one represented creature keyword",
                ));
            };
            if !is_any_selector(subject) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.subject"),
                    "nested continuous operation must apply to any() selected object",
                ));
            }
            operations.push(ContinuousEffectOperation::AddKeywords {
                keywords: compile_granted_creature_keyword(keyword, &format!("{path}.keyword"))?,
            });
            Ok(())
        }
        unsupported => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            path,
            format!(
                "static continuous operation `{}` has no complete runtime lowering",
                unsupported.as_str()
            ),
        )),
    }
}

fn compile_attached_static_operations(
    expression: &Expression,
    path: &str,
    operations: &mut Vec<ContinuousEffectOperation>,
    restrictions: &mut Vec<TargetRestriction>,
) -> Result<(), CompileDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "attached-object continuous operation is not a call",
        ));
    };
    match operation {
        Operation::Sequence => {
            for (index, operation) in arguments.iter().enumerate() {
                compile_attached_static_operations(
                    operation,
                    &format!("{path}.sequence[{index}]"),
                    operations,
                    restrictions,
                )?;
            }
            Ok(())
        }
        Operation::ModifyPt => {
            let [subject, Expression::Integer(power), Expression::Integer(toughness)] =
                arguments.as_slice()
            else {
                return Err(effect_arity(
                    path,
                    operation,
                    "any(), literal power delta, and literal toughness delta",
                ));
            };
            if !is_any_selector(subject) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.subject"),
                    "attached-object operation must apply to any() selected object",
                ));
            }
            let power = i32::try_from(*power).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    format!("{path}.power"),
                    "attached-object power delta does not fit i32",
                )
            })?;
            let toughness = i32::try_from(*toughness).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    format!("{path}.toughness"),
                    "attached-object toughness delta does not fit i32",
                )
            })?;
            operations.push(ContinuousEffectOperation::ModifyPowerToughness { power, toughness });
            Ok(())
        }
        Operation::GrantKeyword => {
            let [subject, keyword] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "any() and one represented keyword",
                ));
            };
            if !is_any_selector(subject) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.subject"),
                    "attached-object operation must apply to any() selected object",
                ));
            }
            match keyword {
                Expression::Text(value) if value == "shroud" => {
                    restrictions.push(TargetRestriction::Shroud);
                }
                Expression::Text(value) if value == "hexproof" => {
                    restrictions.push(TargetRestriction::Hexproof);
                }
                _ => operations.push(ContinuousEffectOperation::AddKeywords {
                    keywords: compile_granted_creature_keyword(
                        keyword,
                        &format!("{path}.keyword"),
                    )?,
                }),
            }
            Ok(())
        }
        unsupported => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            path,
            format!(
                "attached-object operation `{}` has no complete runtime lowering",
                unsupported.as_str()
            ),
        )),
    }
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
        Operation::EventEnters => {
            let [entering] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "source() or one closed permanent selector",
                ));
            };
            if is_source_selector(entering) {
                return Ok(TriggeredEventProgram::SourceEnters);
            }
            let spec = compile_object_selector(entering, &format!("{path}.permanent"))?;
            if spec.kind != TargetKind::Permanent
                || spec.controller != Some(TargetControllerPredicate::You)
                || !spec.required_types.creature()
                || !spec.exclude_source
            {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    format!("{path}.permanent"),
                    "enter trigger requires another creature permanent controlled by you()",
                ));
            }
            Ok(TriggeredEventProgram::ControllerPermanentEnters {
                predicate: object_predicate_from_spec(spec),
                exclude_source: true,
            })
        }
        Operation::EventAttacks => {
            let [attacker] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "source() or equipped_object(source())",
                ));
            };
            if is_source_selector(attacker) {
                Ok(TriggeredEventProgram::SourceAttacks)
            } else if is_equipped_object_of_source(attacker) {
                Ok(TriggeredEventProgram::AttachedObjectAttacks)
            } else {
                Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    format!("{path}.attacker"),
                    "attack event requires source() or equipped_object(source())",
                ))
            }
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
            match controller {
                Expression::Call {
                    operation: Operation::You,
                    arguments,
                } if arguments.is_empty() => Ok(TriggeredEventProgram::ControllerCasts(predicate)),
                Expression::Text(value) if value == "cast_or_copy:you" => {
                    Ok(TriggeredEventProgram::ControllerCastsOrCopies(predicate))
                }
                _ => Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::PlayerSelector,
                    format!("{path}.controller"),
                    "event_cast controller must be you() or exact cast_or_copy:you",
                )),
            }
        }
        Operation::EventDamage => {
            let [source, target, Expression::Text(mode)] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "matching permanent source, any(), and \"combat\"",
                ));
            };
            if mode != "combat" || !is_any_selector(target) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    path,
                    "combat-damage trigger requires any player target and exact combat mode",
                ));
            }
            let spec = compile_object_selector(source, &format!("{path}.source"))?;
            if spec.kind != TargetKind::Permanent
                || spec.controller != Some(TargetControllerPredicate::You)
                || !spec.required_types.creature()
                || spec.exclude_source
            {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::AbilityShape,
                    format!("{path}.source"),
                    "combat-damage trigger requires a creature permanent controlled by you()",
                ));
            }
            Ok(
                TriggeredEventProgram::ControllerPermanentDealsCombatDamageToPlayer(
                    object_predicate_from_spec(spec),
                ),
            )
        }
        Operation::EventDraw => {
            let [player] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "opponent()"));
            };
            if !matches!(
                player,
                Expression::Call {
                    operation: Operation::Opponent,
                    arguments
                } if arguments.is_empty()
            ) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::PlayerSelector,
                    format!("{path}.player"),
                    "draw trigger currently requires opponent()",
                ));
            }
            Ok(TriggeredEventProgram::OpponentDrawsCard)
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
        if keywords.iter().all(|keyword| keyword.as_str() == "equip") {
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

fn compile_spell_modes(
    expression: &Expression,
    path: &str,
) -> Result<Vec<SpellModeProgram>, CompileDiagnostic> {
    let Expression::Call {
        operation: Operation::ChooseOne,
        arguments,
    } = expression
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "modal spell root is not choose_one(...) ",
        ));
    };
    if !(2..=MAX_SPELL_MODES).contains(&arguments.len()) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            path,
            format!(
                "choose_one requires 2..={MAX_SPELL_MODES} modes, found {}",
                arguments.len()
            ),
        ));
    }
    let mut modes = Vec::with_capacity(arguments.len());
    for (index, effect) in arguments.iter().enumerate() {
        let mut compiler = ProgramCompiler::default();
        compile_effect(effect, &format!("{path}.mode[{index}]"), &mut compiler)?;
        if compiler.effects.is_empty() {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                format!("{path}.mode[{index}]"),
                "spell mode compiled no executable effect",
            ));
        }
        modes.push(SpellModeProgram {
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
        });
    }
    Ok(modes)
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
        damage_to_controller: 0,
        condition: None,
    }))
}

fn compile_mana_activation_condition(
    timing: Option<&Expression>,
    path: &str,
) -> Result<Option<ActivationCondition>, CompileDiagnostic> {
    let Some(timing) = timing else {
        return Ok(None);
    };
    let Expression::Call {
        operation: Operation::TimingCondition,
        arguments,
    } = timing
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "mana-ability timing is not timing_condition(...)",
        ));
    };
    let [Expression::Call {
        operation: Operation::AtLeast,
        arguments: comparison,
    }] = arguments.as_slice()
    else {
        return Err(effect_arity(
            path,
            &Operation::TimingCondition,
            "at_least(count(permanents(...)), literal)",
        ));
    };
    let [Expression::Call {
        operation: Operation::Count,
        arguments: count_arguments,
    }, Expression::Integer(count)] = comparison.as_slice()
    else {
        return Err(effect_arity(
            &format!("{path}.condition"),
            &Operation::AtLeast,
            "count(permanents(...)) and one positive literal",
        ));
    };
    let [selector] = count_arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.condition.count"),
            &Operation::Count,
            "one permanent selector",
        ));
    };
    let count = u32::try_from(*count).map_err(|_| {
        CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            format!("{path}.condition.count"),
            "activation-condition count is outside the positive u32 range",
        )
    })?;
    if count == 0 {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            format!("{path}.condition.count"),
            "activation-condition count must be positive",
        ));
    }
    let spec = compile_object_selector(selector, &format!("{path}.condition.selector"))?;
    if spec.kind != TargetKind::Permanent {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.condition.selector"),
            "activation-condition count requires battlefield permanents",
        ));
    }
    Ok(Some(ActivationCondition::ControllerControlsAtLeast {
        predicate: object_predicate_from_spec(spec),
        count,
    }))
}

fn compile_fixed_mana_ability(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<ActivatedAbilityProgram, CompileDiagnostic> {
    if !ability.mana_ability || ability.event.is_some() || ability.condition.is_some() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "activated mana ability must have no event or intervening condition and be marked mana_ability",
        ));
    }
    let condition =
        compile_mana_activation_condition(ability.timing.as_ref(), &format!("{path}.timing"))?;
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
    let (add_mana, damage_to_controller) = match &ability.effect {
        Expression::Call {
            operation: Operation::AddMana,
            ..
        } => (&ability.effect, 0),
        Expression::Call {
            operation: Operation::Sequence,
            arguments,
        } => {
            let [add_mana, damage] = arguments.as_slice() else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.effect"),
                    "mana-ability sequence requires add_mana followed by controller damage",
                ));
            };
            let Expression::Call {
                operation: Operation::DealDamage,
                arguments,
            } = damage
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectOperation,
                    format!("{path}.effect.sequence[1]"),
                    "mana-ability sequence must end with deal_damage(you(), amount)",
                ));
            };
            let [player, Expression::Integer(amount)] = arguments.as_slice() else {
                return Err(effect_arity(
                    &format!("{path}.effect.sequence[1]"),
                    &Operation::DealDamage,
                    "you() and one literal amount",
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
                    format!("{path}.effect.sequence[1].player"),
                    "mana-ability damage must be dealt to you()",
                ));
            }
            let amount = u32::try_from(*amount).map_err(|_| {
                CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectAmount,
                    format!("{path}.effect.sequence[1].amount"),
                    "mana-ability damage is outside the u32 action range",
                )
            })?;
            (add_mana, amount)
        }
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectOperation,
                format!("{path}.effect"),
                "activated mana ability is neither fixed add_mana nor add_mana plus damage",
            ));
        }
    };
    let Expression::Call {
        operation: Operation::AddMana,
        arguments,
    } = add_mana
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectOperation,
            format!("{path}.effect"),
            "mana-ability sequence must begin with add_mana",
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
        damage_to_controller,
        condition,
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
    let mut sacrifice_cost = None;
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
            Operation::Sacrifice => {
                let [selector, count] = arguments.as_slice() else {
                    return Err(effect_arity(
                        &cost_path,
                        operation,
                        "one controlled permanent selector and literal count",
                    ));
                };
                if sacrifice_cost.is_some() {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        cost_path,
                        "activation repeats its matching-permanent sacrifice cost",
                    ));
                }
                let spec = compile_object_selector(selector, &format!("{cost_path}.selector"))?;
                if spec.kind != TargetKind::Permanent
                    || spec.controller != Some(TargetControllerPredicate::You)
                    || spec.exclude_source
                {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::AbilityShape,
                        format!("{cost_path}.selector"),
                        "sacrifice cost requires a closed permanent predicate controlled by you()",
                    ));
                }
                sacrifice_cost = Some((
                    object_predicate_from_spec(spec),
                    compile_bounded_positive_literal(
                        count,
                        &format!("{cost_path}.count"),
                        MAX_EFFECTS as u32,
                        "sacrifice count",
                    )?,
                ));
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
        sacrifice_cost,
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

fn ability_has_discard_source_cost(ability: &AbilityDefinition) -> bool {
    ability.costs.iter().any(|cost| {
        matches!(
            cost,
            Expression::Call {
                operation: Operation::DiscardCost,
                ..
            }
        )
    })
}

fn compile_cycling_ability(
    ability: &AbilityDefinition,
    path: &str,
) -> Result<CyclingProgram, CompileDiagnostic> {
    if ability.event.is_some()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            path,
            "cycling ability cannot carry an event, condition, timing, or mana flag",
        ));
    }
    let [mana, discard] = ability.costs.as_slice() else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.costs"),
            "cycling requires exactly mana_cost(...) followed by discard_cost(1, source())",
        ));
    };
    let Expression::Call {
        operation: Operation::ManaCost,
        arguments: mana_arguments,
    } = mana
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.costs[0]"),
            "cycling first cost must be mana_cost(...) ",
        ));
    };
    let [Expression::Text(mana)] = mana_arguments.as_slice() else {
        return Err(effect_arity(
            &format!("{path}.costs[0]"),
            &Operation::ManaCost,
            "one fixed mana-cost string",
        ));
    };
    let (mana_cost, exact_payment) = compile_mana_cost_text(mana, &format!("{path}.costs[0]"))?;
    let Expression::Call {
        operation: Operation::DiscardCost,
        arguments: discard_arguments,
    } = discard
    else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::AbilityShape,
            format!("{path}.costs[1]"),
            "cycling second cost must be discard_cost(1, source())",
        ));
    };
    if !matches!(discard_arguments.as_slice(), [Expression::Integer(1), source] if is_source_selector(source))
    {
        return Err(effect_arity(
            &format!("{path}.costs[1]"),
            &Operation::DiscardCost,
            "literal 1 and source()",
        ));
    }
    if !matches!(
        &ability.effect,
        Expression::Call {
            operation: Operation::Draw,
            arguments,
        } if matches!(arguments.as_slice(), [Expression::Integer(1), player] if is_you_selector(player))
    ) {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            format!("{path}.effect"),
            "cycling effect must be exactly draw(1, you())",
        ));
    }
    Ok(CyclingProgram {
        mana_cost,
        exact_payment,
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
    let mut printed_symbols = [0_u32; 5];
    for symbol in mana_symbols {
        if let ManaSymbol::Color(color) = symbol {
            colors = match color {
                Color::White => {
                    printed_symbols[0] = printed_symbols[0].saturating_add(1);
                    colors.with_white()
                }
                Color::Blue => {
                    printed_symbols[1] = printed_symbols[1].saturating_add(1);
                    colors.with_blue()
                }
                Color::Black => {
                    printed_symbols[2] = printed_symbols[2].saturating_add(1);
                    colors.with_black()
                }
                Color::Red => {
                    printed_symbols[3] = printed_symbols[3].saturating_add(1);
                    colors.with_red()
                }
                Color::Green => {
                    printed_symbols[4] = printed_symbols[4].saturating_add(1);
                    colors.with_green()
                }
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
        .with_mana_value(mana_value)
        .with_printed_mana_symbols(ManaPool::new(
            printed_symbols[0],
            printed_symbols[1],
            printed_symbols[2],
            printed_symbols[3],
            printed_symbols[4],
            0,
        )))
}

fn compile_printed_mana_value(symbols: &[ManaSymbol]) -> Result<u32, CompileDiagnostic> {
    let mut mana_value = 0_u32;
    for (index, symbol) in symbols.iter().enumerate() {
        let contribution = match symbol {
            ManaSymbol::Color(_) => 1,
            ManaSymbol::Generic(amount) => u32::from(*amount),
            ManaSymbol::Variable(_) => 0,
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
    let mut x_count = 0_u32;
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
            ManaSymbol::Variable('X') => {
                x_count = x_count.checked_add(1).ok_or_else(|| {
                    CompileDiagnostic::new(
                        CompileDiagnosticCode::ProgramBounds,
                        format!("card.faces[0].cost[{index}]"),
                        "variable X count overflowed",
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
        )
        .with_x(x_count, 0),
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
        Operation::DealDamage => {
            let [target, amount] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "target and amount"));
            };
            let amount = compile_amount(amount, &format!("{path}.amount"), compiler)?;
            if matches!(
                target,
                Expression::Call {
                    operation: Operation::You | Operation::Opponent | Operation::Any,
                    ..
                }
            ) {
                let players = compile_player_binding(target, &format!("{path}.players"), compiler)?;
                compiler
                    .effects
                    .push(EffectProgram::DealDamageToPlayers { players, amount });
            } else {
                let target = compile_damage_target(target, &format!("{path}.target"), compiler)?;
                compiler
                    .effects
                    .push(EffectProgram::DealDamageToTarget { target, amount });
            }
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
            let [Expression::Integer(count), players, Expression::Text(mode)] =
                arguments.as_slice()
            else {
                return Err(effect_arity(
                    path,
                    operation,
                    "the canonical hand placeholder, players, and mode",
                ));
            };
            let players = compile_player_binding(players, &format!("{path}.players"), compiler)?;
            match mode.as_str() {
                "hand" if *count == 1 => {
                    compiler
                        .effects
                        .push(EffectProgram::DiscardHands { players });
                }
                "choose" if players == PlayerBinding::Controller => {
                    let count = u32::try_from(*count).map_err(|_| {
                        CompileDiagnostic::new(
                            CompileDiagnosticCode::EffectAmount,
                            format!("{path}.count"),
                            "chosen discard count is outside the u32 range",
                        )
                    })?;
                    if count == 0 || count > MAX_EFFECTS as u32 {
                        return Err(CompileDiagnostic::new(
                            CompileDiagnosticCode::EffectAmount,
                            format!("{path}.count"),
                            "chosen discard count must be positive and bounded",
                        ));
                    }
                    let selector = Expression::Call {
                        operation: Operation::Cards,
                        arguments: Vec::new(),
                    };
                    let choice = intern_object_choice(
                        compiler,
                        &selector,
                        ObjectChoiceRequirement {
                            player: PlayerBinding::Controller,
                            zone: ZoneKind::Hand,
                            minimum: count,
                            maximum: count,
                            required_types: ObjectTypes::none(),
                            required_any_types: ObjectTypes::none(),
                            forbidden_types: ObjectTypes::none(),
                            required_supertypes: ObjectSupertypes::none(),
                            required_land_types: BasicLandTypes::none(),
                            required_any_land_types: BasicLandTypes::none(),
                            required_subtypes: ObjectSubtypes::none(),
                        },
                    );
                    compiler
                        .effects
                        .push(EffectProgram::DiscardChosenObjects { choice });
                }
                _ => {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "discard_cards requires either the complete-hand marker or an exact controller choice",
                    ));
                }
            }
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
            let (target, prohibit_regeneration) = match (operation, arguments.as_slice()) {
                (Operation::Destroy, [target]) | (Operation::Exile, [target]) => (target, false),
                (Operation::Destroy, [target, Expression::Text(mode)])
                    if mode == "cannot_regenerate" =>
                {
                    (target, true)
                }
                (Operation::Destroy, _) => {
                    return Err(effect_arity(
                        path,
                        operation,
                        "one target selector and optional exact cannot_regenerate marker",
                    ));
                }
                (Operation::Exile, _) => {
                    return Err(effect_arity(path, operation, "one target selector"));
                }
                _ => unreachable!("closed destroy/exile operation match"),
            };
            let target = compile_object_target(target, &format!("{path}.target"), compiler)?;
            compiler.effects.push(match operation {
                Operation::Destroy if prohibit_regeneration => {
                    EffectProgram::DestroyPermanentWithoutRegeneration { target }
                }
                Operation::Destroy => EffectProgram::DestroyPermanent { target },
                Operation::Exile => EffectProgram::ExileObject { target },
                _ => unreachable!("closed destroy/exile operation match"),
            });
            Ok(())
        }
        Operation::Attach => {
            let [source, target] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "source() and one object target",
                ));
            };
            if !is_source_selector(source) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.source"),
                    "attachment source must be source()",
                ));
            }
            let target = compile_object_target(target, &format!("{path}.target"), compiler)?;
            compiler
                .effects
                .push(EffectProgram::AttachSourceToTarget { target });
            Ok(())
        }
        Operation::AddCounter => {
            let [source, Expression::Text(kind), amount] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "source(), one represented counter kind, and literal amount",
                ));
            };
            if !is_source_selector(source) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.source"),
                    "source-bound counter effect must use source()",
                ));
            }
            let kind = match kind.as_str() {
                "p1p1" => CounterKind::PlusOnePlusOne,
                "m1m1" => CounterKind::MinusOneMinusOne,
                "loyalty" => CounterKind::Loyalty,
                unsupported => {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        format!("{path}.kind"),
                        format!("counter kind `{unsupported}` has no exact runtime lowering"),
                    ));
                }
            };
            let amount = compile_bounded_positive_literal(
                amount,
                &format!("{path}.amount"),
                MAX_EFFECTS as u32,
                "counter amount",
            )?;
            compiler
                .effects
                .push(EffectProgram::AddCountersToSource { kind, amount });
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
            compiler.effects.push(EffectProgram::MoveTargetObject {
                target,
                from,
                to,
                overload_predicate: None,
            });
            Ok(())
        }
        Operation::ReturnToHand => {
            let [target_expression] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "one permanent target selector",
                ));
            };
            let selector = target_selector(target_expression, &format!("{path}.target"))?;
            let selector_spec = compile_object_selector(selector, &format!("{path}.target"))?;
            if selector_spec.kind != TargetKind::Permanent {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.target"),
                    "return_to_hand requires a battlefield permanent selector",
                ));
            }
            let overload_predicate = object_predicate_from_spec(selector_spec);
            let target =
                compile_object_target(target_expression, &format!("{path}.target"), compiler)?;
            compiler.effects.push(EffectProgram::MoveTargetObject {
                target,
                from: ZoneKind::Battlefield,
                to: ZoneKind::Hand,
                overload_predicate: Some(overload_predicate),
            });
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
                    minimum: 0,
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
        Operation::Reveal => {
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
                .push(EffectProgram::RevealChosenObjects { choice });
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
        Operation::ModifyPt => {
            let [objects, power, toughness, duration] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "object set, power delta, toughness delta, and duration",
                ));
            };
            let objects =
                compile_effect_object_set(objects, &format!("{path}.objects"), compiler, true)?;
            let power = compile_amount(power, &format!("{path}.power"), compiler)?;
            let toughness = compile_amount(toughness, &format!("{path}.toughness"), compiler)?;
            let duration = compile_effect_duration(duration, &format!("{path}.duration"))?;
            compiler.effects.push(EffectProgram::ModifyPowerToughness {
                objects,
                power,
                toughness,
                duration,
            });
            Ok(())
        }
        Operation::GrantKeyword => {
            let [objects, keyword, duration] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    operation,
                    "object set, represented creature keyword, and duration",
                ));
            };
            let duration = compile_effect_duration(duration, &format!("{path}.duration"))?;
            let Expression::Text(keyword_name) = keyword else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.keyword"),
                    "granted keyword is not text",
                ));
            };
            match keyword_name.as_str() {
                "hexproof" | "shroud" => {
                    let objects = compile_effect_object_set(
                        objects,
                        &format!("{path}.objects"),
                        compiler,
                        false,
                    )?;
                    let restriction = if keyword_name == "hexproof" {
                        TargetRestriction::Hexproof
                    } else {
                        TargetRestriction::Shroud
                    };
                    compiler
                        .effects
                        .push(EffectProgram::GrantTargetingRestriction {
                            objects,
                            restriction,
                            duration,
                        });
                }
                "indestructible" => {
                    let objects = compile_effect_object_set(
                        objects,
                        &format!("{path}.objects"),
                        compiler,
                        false,
                    )?;
                    compiler
                        .effects
                        .push(EffectProgram::GrantIndestructible { objects, duration });
                }
                _ => {
                    let objects = compile_effect_object_set(
                        objects,
                        &format!("{path}.objects"),
                        compiler,
                        true,
                    )?;
                    let keywords =
                        compile_granted_creature_keyword(keyword, &format!("{path}.keyword"))?;
                    compiler.effects.push(EffectProgram::GrantKeywords {
                        objects,
                        keywords,
                        duration,
                    });
                }
            }
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
        let subtypes = compile_exact_token_subtypes(&["Treasure"], path)?;
        return Ok(TokenTemplate {
            card: CardId::new(stable_runtime_id(script)),
            base_object: BaseObjectCharacteristics::new(
                ObjectTypes::none().with_artifact(),
                ObjectColors::none(),
            )
            .with_subtypes(subtypes),
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
                damage_to_controller: 0,
                condition: None,
            }),
        });
    }
    let (colors, base_creature, subtype_names): (_, _, &[&str]) = match script.as_str() {
        "g_3_3_beast" | "g_3_3_elephant" | "g_3_3_ape" | "g_3_3_frog_lizard" => (
            ObjectColors::none().with_green(),
            BaseCreatureCharacteristics::new(3, 3),
            match script.as_str() {
                "g_3_3_beast" => &["Beast"],
                "g_3_3_elephant" => &["Elephant"],
                "g_3_3_ape" => &["Ape"],
                "g_3_3_frog_lizard" => &["Frog", "Lizard"],
                _ => &[],
            },
        ),
        "u_2_2_bird_flying" => (
            ObjectColors::none().with_blue(),
            BaseCreatureCharacteristics::new(2, 2)
                .with_keywords(CreatureKeywords::none().with_flying()),
            &["Bird"],
        ),
        "b_2_2_zombie" => (
            ObjectColors::none().with_black(),
            BaseCreatureCharacteristics::new(2, 2),
            &["Zombie"],
        ),
        _ => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                path,
                format!("token template `{script}` is not in the exact runtime registry"),
            ));
        }
    };
    let subtypes = compile_exact_token_subtypes(subtype_names, path)?;
    Ok(TokenTemplate {
        card: CardId::new(stable_runtime_id(script)),
        base_object: BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), colors)
            .with_subtypes(subtypes),
        base_creature: Some(base_creature),
        mana_ability: None,
    })
}

fn compile_exact_token_subtypes(
    names: &[&str],
    path: &str,
) -> Result<ObjectSubtypes, CompileDiagnostic> {
    let mut subtypes = ObjectSubtypes::none();
    for (index, name) in names.iter().enumerate() {
        let subtype = ObjectSubtype::parse(name).ok_or_else(|| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::EffectArguments,
                format!("{path}.subtypes[{index}]"),
                format!("registered token subtype `{name}` is invalid"),
            )
        })?;
        subtypes = subtypes.try_with(subtype).ok_or_else(|| {
            CompileDiagnostic::new(
                CompileDiagnosticCode::ProgramBounds,
                format!("{path}.subtypes"),
                "registered token subtype count exceeds the bounded runtime set",
            )
        })?;
    }
    Ok(subtypes)
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
            if matches!(
                target,
                Expression::Call {
                    operation: Operation::Triggered,
                    arguments
                } if arguments.is_empty()
            ) {
                return Ok(PlayerBinding::TriggeringPlayer);
            }
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

fn compile_damage_target(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
) -> Result<usize, CompileDiagnostic> {
    let selector = target_selector(expression, path)?;
    match selector {
        Expression::Call {
            operation: Operation::All,
            arguments,
        } => {
            let [player_selector, object_selector] = arguments.as_slice() else {
                return Err(effect_arity(
                    path,
                    &Operation::All,
                    "any() and one permanent selector",
                ));
            };
            if !is_any_selector(player_selector) {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.player"),
                    "player-or-permanent damage target requires any() for the player branch",
                ));
            }
            let spec = compile_object_selector(object_selector, &format!("{path}.object"))?;
            if spec.kind != TargetKind::Permanent || spec.exclude_source {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    format!("{path}.object"),
                    "player-or-permanent damage target requires an absolute battlefield selector",
                ));
            }
            intern_target(
                compiler,
                selector,
                TargetRequirement::new(TargetKind::PlayerOrPermanent)
                    .with_player_or_object_predicate(
                        PlayerTargetPredicate::Any,
                        object_predicate_from_spec(spec),
                    ),
            )
        }
        Expression::Call {
            operation: Operation::Any | Operation::You | Operation::Opponent,
            ..
        } => compile_player_target(expression, compiler, path),
        _ => {
            let target = compile_object_target(expression, path, compiler)?;
            if compiler.targets[target].requirement.kind() != TargetKind::Permanent {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "damage object target must be a battlefield permanent",
                ));
            }
            Ok(target)
        }
    }
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
    if spec.exclude_source {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "announced targets cannot use a source-relative exclusion",
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

fn is_you_selector(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call {
            operation: Operation::You,
            arguments,
        } if arguments.is_empty()
    )
}

fn is_source_selector(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call {
            operation: Operation::Source,
            arguments,
        } if arguments.is_empty()
    )
}

fn is_equipped_object_of_source(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call {
            operation: Operation::EquippedObject,
            arguments,
        } if matches!(arguments.as_slice(), [source] if is_source_selector(source))
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

type LibraryChoiceSelector = (
    ObjectTypes,
    ObjectTypes,
    ObjectTypes,
    ObjectSupertypes,
    BasicLandTypes,
    BasicLandTypes,
    ObjectSubtypes,
);

fn compile_library_choice_selector(
    selector: &Expression,
    path: &str,
) -> Result<LibraryChoiceSelector, CompileDiagnostic> {
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
    exclude_source: bool,
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
            exclude_source: false,
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
        Operation::Spells => {
            let mut spec = ObjectSelectorSpec::new(TargetKind::StackEntry);
            for (index, predicate) in arguments.iter().enumerate() {
                compile_object_predicate(predicate, &format!("{path}.spells[{index}]"), &mut spec)?;
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
            || spec.exclude_source != first.exclude_source
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
        exclude_source: first.exclude_source,
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
                || first.exclude_source
                || alternatives.iter().any(|alternative| {
                    alternative.owner != first.owner
                        || alternative.controller != first.controller
                        || alternative.required_types == ObjectTypes::none()
                        || alternative.required_any_types != ObjectTypes::none()
                        || alternative.forbidden_types != ObjectTypes::none()
                        || alternative.required_subtypes != ObjectSubtypes::none()
                        || alternative.minimum_mana_value.is_some()
                        || alternative.maximum_mana_value.is_some()
                        || alternative.exclude_source
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
            if matches!(
                predicate,
                Expression::Call {
                    operation: Operation::Equals,
                    arguments,
                } if matches!(
                    arguments.as_slice(),
                    [
                        Expression::Call {
                            operation: Operation::Any,
                            arguments: any_arguments,
                        },
                        Expression::Call {
                            operation: Operation::Source,
                            arguments: source_arguments,
                        },
                    ] if any_arguments.is_empty() && source_arguments.is_empty()
                )
            ) {
                if spec.exclude_source {
                    return Err(CompileDiagnostic::new(
                        CompileDiagnosticCode::EffectArguments,
                        path,
                        "duplicate source exclusion is not compiled",
                    ));
                }
                spec.exclude_source = true;
                return Ok(());
            }
            if let Expression::Call {
                operation: relationship_operation @ (Operation::OwnedBy | Operation::ControlledBy),
                arguments,
            } = predicate
            {
                let [selector] = arguments.as_slice() else {
                    return Err(effect_arity(
                        path,
                        relationship_operation,
                        "one player selector",
                    ));
                };
                let relationship = match compile_target_relationship(selector, path)? {
                    TargetControllerPredicate::You => TargetControllerPredicate::Opponent,
                    TargetControllerPredicate::Opponent => TargetControllerPredicate::You,
                    TargetControllerPredicate::Any | TargetControllerPredicate::Player(_) => {
                        return Err(CompileDiagnostic::new(
                            CompileDiagnosticCode::EffectArguments,
                            path,
                            "negated relationship requires you() or opponent()",
                        ));
                    }
                };
                if *relationship_operation == Operation::OwnedBy {
                    spec.owner = relationship;
                } else {
                    spec.controller = Some(relationship);
                }
                return Ok(());
            }
            let Expression::Call {
                operation: Operation::TypeIs,
                arguments,
            } = predicate
            else {
                return Err(CompileDiagnostic::new(
                    CompileDiagnosticCode::EffectArguments,
                    path,
                    "only closed type, source, owner, and controller negations are compiled",
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

fn compile_effect_object_set(
    expression: &Expression,
    path: &str,
    compiler: &mut ProgramCompiler,
    require_creature: bool,
) -> Result<ObjectSetProgram, CompileDiagnostic> {
    let is_target = matches!(
        expression,
        Expression::Call {
            operation: Operation::Target,
            ..
        }
    );
    let selector = if is_target {
        target_selector(expression, path)?
    } else {
        expression
    };
    let spec = compile_object_selector(selector, path)?;
    if spec.kind != TargetKind::Permanent {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "continuous characteristic changes require battlefield permanents",
        ));
    }
    if require_creature && !spec.required_types.creature() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "the current keyword and power/toughness model requires a creature-only selector",
        ));
    }
    if is_target {
        return Ok(ObjectSetProgram::Target(compile_object_target(
            expression, path, compiler,
        )?));
    }
    Ok(ObjectSetProgram::Battlefield(object_predicate_from_spec(
        spec,
    )))
}

fn object_predicate_from_spec(spec: ObjectSelectorSpec) -> ObjectTargetPredicate {
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
    predicate
}

fn compile_effect_duration(
    expression: &Expression,
    path: &str,
) -> Result<ContinuousEffectDuration, CompileDiagnostic> {
    let Expression::Text(duration) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "continuous-effect duration is not text",
        ));
    };
    match duration.as_str() {
        "until_end_of_turn" => Ok(ContinuousEffectDuration::UntilEndOfTurn),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            format!("continuous-effect duration `{duration}` is not compiled"),
        )),
    }
}

fn compile_granted_creature_keyword(
    expression: &Expression,
    path: &str,
) -> Result<CreatureKeywords, CompileDiagnostic> {
    let Expression::Text(keyword) = expression else {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "granted keyword is not text",
        ));
    };
    match keyword.as_str() {
        "deathtouch" => Ok(CreatureKeywords::none().with_deathtouch()),
        "double_strike" => Ok(CreatureKeywords::none().with_double_strike()),
        "first_strike" => Ok(CreatureKeywords::none().with_first_strike()),
        "flying" => Ok(CreatureKeywords::none().with_flying()),
        "haste" => Ok(CreatureKeywords::none().with_haste()),
        "indestructible" => Ok(CreatureKeywords::none().with_indestructible()),
        "lifelink" => Ok(CreatureKeywords::none().with_lifelink()),
        "menace" => Ok(CreatureKeywords::none().with_menace()),
        "reach" => Ok(CreatureKeywords::none().with_reach()),
        "trample" => Ok(CreatureKeywords::none().with_trample()),
        "vigilance" => Ok(CreatureKeywords::none().with_vigilance()),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            path,
            format!("granted keyword `{keyword}` has no exact creature-keyword lowering"),
        )),
    }
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
    source: Option<ObjectId>,
    alternate_cost: Option<AlternateCostKind>,
    spell_mode: Option<usize>,
    triggering_player: Option<PlayerId>,
    unless_payment: Option<bool>,
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
            source: None,
            alternate_cost: None,
            spell_mode: None,
            triggering_player: None,
            unless_payment: None,
            opponents,
            targets: Vec::new(),
            object_choices: Vec::new(),
            optional_effect_choices: Vec::new(),
            scry_bottoms: BTreeMap::new(),
        }
    }

    /// Supplies the source object for source-bound effects.
    #[must_use]
    pub const fn with_source(mut self, source: ObjectId) -> Self {
        self.source = Some(source);
        self
    }

    /// Supplies the alternate-cost rule selected during casting.
    #[must_use]
    pub const fn with_alternate_cost(mut self, kind: AlternateCostKind) -> Self {
        self.alternate_cost = Some(kind);
        self
    }

    /// Supplies the zero-based mode announced while casting a modal spell.
    #[must_use]
    pub const fn with_spell_mode(mut self, mode: usize) -> Self {
        self.spell_mode = Some(mode);
        self
    }

    /// Supplies the player carried by the event that queued a trigger.
    #[must_use]
    pub const fn with_triggering_player(mut self, player: PlayerId) -> Self {
        self.triggering_player = Some(player);
        self
    }

    /// Chooses whether the event-bound player pays an exact unless cost.
    #[must_use]
    pub const fn with_unless_payment(mut self, pay: bool) -> Self {
        self.unless_payment = Some(pay);
        self
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

    /// Returns the selected alternate-cost rule, if any.
    #[must_use]
    pub const fn alternate_cost(&self) -> Option<AlternateCostKind> {
        self.alternate_cost
    }

    /// Returns the announced modal branch, when this is a modal spell.
    #[must_use]
    pub const fn spell_mode(&self) -> Option<usize> {
        self.spell_mode
    }

    /// Returns the event-bound triggering player, when supplied.
    #[must_use]
    pub const fn triggering_player(&self) -> Option<PlayerId> {
        self.triggering_player
    }

    /// Returns the explicit unless-payment decision, when supplied.
    #[must_use]
    pub const fn unless_payment(&self) -> Option<bool> {
        self.unless_payment
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
    if let Some(kind) = bindings.alternate_cost {
        if !program.alternate_costs.iter().any(|cost| cost.kind == kind) {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!("alternate cost {kind:?} is not available on this program"),
            ));
        }
    }
    let (target_requirements, object_choice_requirements, effects, optional_effect_groups) =
        if program.spell_modes.is_empty() {
            if bindings.spell_mode.is_some() {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    "a mode was supplied for a non-modal spell",
                ));
            }
            (
                program.target_requirements_for_alternate(bindings.alternate_cost),
                program.object_choice_requirements.as_slice(),
                program.effects.as_slice(),
                program.optional_effect_groups.as_slice(),
            )
        } else {
            if bindings.alternate_cost == Some(AlternateCostKind::Overload) {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    None,
                    "overload is not defined for modal spells",
                ));
            }
            let mode = bindings
                .spell_mode
                .and_then(|index| program.spell_modes.get(index))
                .ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingChoice,
                        None,
                        format!(
                            "modal spell requires one mode in 0..{}, found {:?}",
                            program.spell_modes.len(),
                            bindings.spell_mode
                        ),
                    )
                })?;
            (
                mode.target_requirements.as_slice(),
                mode.object_choice_requirements.as_slice(),
                mode.effects.as_slice(),
                mode.optional_effect_groups.as_slice(),
            )
        };
    bind_effect_actions(
        state,
        target_requirements,
        object_choice_requirements,
        effects,
        optional_effect_groups,
        bindings,
    )
}

/// Resolves one triggered ability's bindings without mutating game state.
pub fn bind_triggered_ability_actions(
    state: &GameState,
    ability: &TriggeredAbilityProgram,
    bindings: &ExecutionBindings,
) -> Result<Vec<BoundAction>, ExecutionDiagnostic> {
    let effect_actions = bind_effect_actions(
        state,
        &ability.target_requirements,
        &ability.object_choice_requirements,
        &ability.effects,
        &ability.optional_effect_groups,
        bindings,
    )?;
    let Some(unless_paid) = ability.unless_paid else {
        if bindings.unless_payment.is_some() {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                "an unless-payment decision was supplied for an unconditional trigger",
            ));
        }
        return Ok(effect_actions);
    };
    let pay = bindings.unless_payment.ok_or_else(|| {
        ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::MissingChoice,
            None,
            "trigger requires an explicit unless-payment decision",
        )
    })?;
    if !pay {
        return Ok(effect_actions);
    }
    let payers = resolve_players(state, unless_paid.payer, bindings, 0)?;
    let [payer] = payers.as_slice() else {
        return Err(ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::InvalidChoice,
            None,
            "unless_paid must resolve to exactly one payer",
        ));
    };
    let payment = auto_payment_plan(unless_paid.exact_payment, unless_paid.mana_cost)
        .map_err(|error| {
            ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!("unless-payment plan is invalid: {error:?}"),
            )
        })?
        .ok_or_else(|| {
            ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                "unless-payment cost has no exact payment plan",
            )
        })?;
    Ok(vec![BoundAction {
        effect_index: 0,
        action: Action::PayMana {
            player: *payer,
            cost: unless_paid.mana_cost,
            plan: payment,
        },
    }])
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
            bindings.source,
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
            EffectProgram::DealDamageToTarget { target, amount } => {
                let amount = resolve_amount(state, *amount, bindings, effect_index)?;
                let target = match bindings.targets.get(*target) {
                    Some(TargetChoice::Player(player)) => CombatDamageTarget::Player(*player),
                    Some(TargetChoice::Object(object)) => CombatDamageTarget::Object(*object),
                    _ => {
                        return Err(ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::MissingBinding,
                            Some(effect_index),
                            format!("target slot {target} is not a player or object"),
                        ));
                    }
                };
                actions.push(BoundAction {
                    effect_index,
                    action: Action::DealDamage {
                        source: bindings.source,
                        target,
                        amount,
                    },
                });
            }
            EffectProgram::DealDamageToPlayers { players, amount } => {
                let amount = resolve_amount(state, *amount, bindings, effect_index)?;
                for player in resolve_players(state, *players, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::DealDamage {
                            source: bindings.source,
                            target: CombatDamageTarget::Player(player),
                            amount,
                        },
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
            EffectProgram::DestroyPermanentWithoutRegeneration { target } => {
                actions.push(BoundAction {
                    effect_index,
                    action: Action::DestroyPermanentWithoutRegeneration {
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
            EffectProgram::MoveTargetObject {
                target,
                from,
                to,
                overload_predicate,
            } => {
                let objects = if bindings.alternate_cost == Some(AlternateCostKind::Overload) {
                    let predicate = overload_predicate.ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::InvalidChoice,
                            Some(effect_index),
                            "selected overload has no compiled each-object selector",
                        )
                    })?;
                    state
                        .zone_objects(ZoneId::new(None, *from))
                        .ok_or_else(|| {
                            ExecutionDiagnostic::new(
                                ExecutionDiagnosticCode::MissingBinding,
                                Some(effect_index),
                                format!("source zone {from:?} is unavailable"),
                            )
                        })?
                        .iter()
                        .copied()
                        .filter(|object| {
                            state.object_matches_target_predicate(
                                bindings.controller,
                                predicate,
                                *object,
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![resolve_object_target(bindings, *target, effect_index)?]
                };
                for object in objects {
                    let current = state.object_zone(object).ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::InvalidChoice,
                            Some(effect_index),
                            format!("object {object:?} has no zone"),
                        )
                    })?;
                    if current.kind() != *from {
                        return Err(ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::InvalidChoice,
                            Some(effect_index),
                            format!("object is in {:?}, expected {from:?}", current.kind()),
                        ));
                    }
                    let owner = state
                        .object(object)
                        .ok_or_else(|| {
                            ExecutionDiagnostic::new(
                                ExecutionDiagnosticCode::InvalidChoice,
                                Some(effect_index),
                                format!("object {object:?} is unknown"),
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
            }
            EffectProgram::SacrificeSource => {
                let source = bindings.source.ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        "source sacrifice requires the executing source object",
                    )
                })?;
                if state.object_zone(source) != Some(ZoneId::new(None, ZoneKind::Battlefield)) {
                    return Err(ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        format!("source object {source:?} is not on the battlefield"),
                    ));
                }
                let owner = state
                    .object(source)
                    .ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::MissingBinding,
                            Some(effect_index),
                            format!("source object {source:?} is unknown"),
                        )
                    })?
                    .owner();
                actions.push(BoundAction {
                    effect_index,
                    action: Action::MoveObject {
                        object: source,
                        to: ZoneId::new(Some(owner), ZoneKind::Graveyard),
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
            EffectProgram::RevealChosenObjects { choice } => {
                let objects = bindings
                    .object_choices
                    .get(*choice)
                    .cloned()
                    .ok_or_else(|| {
                        ExecutionDiagnostic::new(
                            ExecutionDiagnosticCode::MissingChoice,
                            Some(effect_index),
                            format!("no object choice for reveal slot {choice}"),
                        )
                    })?;
                actions.push(BoundAction {
                    effect_index,
                    action: Action::RevealObjects { objects },
                });
            }
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
            EffectProgram::DiscardChosenObjects { choice } => {
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
                                format!("discard choice contains unknown object {object:?}"),
                            )
                        })?
                        .owner();
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::MoveObject {
                            object: *object,
                            to: ZoneId::new(Some(owner), ZoneKind::Graveyard),
                        },
                    });
                }
            }
            EffectProgram::ModifyPowerToughness {
                objects,
                power,
                toughness,
                duration,
            } => {
                let power = i32::try_from(resolve_amount(state, *power, bindings, effect_index)?)
                    .map_err(|_| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::InvalidChoice,
                        Some(effect_index),
                        "resolved power delta exceeds i32",
                    )
                })?;
                let toughness =
                    i32::try_from(resolve_amount(state, *toughness, bindings, effect_index)?)
                        .map_err(|_| {
                            ExecutionDiagnostic::new(
                                ExecutionDiagnosticCode::InvalidChoice,
                                Some(effect_index),
                                "resolved toughness delta exceeds i32",
                            )
                        })?;
                for object in resolve_object_set(state, *objects, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::RegisterContinuousEffect {
                            definition: ContinuousEffectDefinition::new(
                                bindings.controller,
                                ContinuousEffectTarget::Object(object),
                                ContinuousEffectOperation::ModifyPowerToughness {
                                    power,
                                    toughness,
                                },
                            )
                            .with_duration(*duration),
                        },
                    });
                }
            }
            EffectProgram::GrantKeywords {
                objects,
                keywords,
                duration,
            } => {
                for object in resolve_object_set(state, *objects, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::RegisterContinuousEffect {
                            definition: ContinuousEffectDefinition::new(
                                bindings.controller,
                                ContinuousEffectTarget::Object(object),
                                ContinuousEffectOperation::AddKeywords {
                                    keywords: *keywords,
                                },
                            )
                            .with_duration(*duration),
                        },
                    });
                }
            }
            EffectProgram::GrantTargetingRestriction {
                objects,
                restriction,
                duration,
            } => {
                for object in resolve_object_set(state, *objects, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::RegisterRestriction {
                            definition: RestrictionDefinition::new(
                                bindings.controller,
                                RestrictionEffect::Targeting {
                                    subject: TargetRestrictionSubject::Object(object),
                                    restriction: *restriction,
                                },
                            )
                            .with_duration(*duration),
                        },
                    });
                }
            }
            EffectProgram::GrantIndestructible { objects, duration } => {
                for object in resolve_object_set(state, *objects, bindings, effect_index)? {
                    actions.push(BoundAction {
                        effect_index,
                        action: Action::RegisterRestriction {
                            definition: RestrictionDefinition::new(
                                bindings.controller,
                                RestrictionEffect::Indestructible { object },
                            )
                            .with_duration(*duration),
                        },
                    });
                }
            }
            EffectProgram::AttachSourceToTarget { target } => {
                let source = bindings.source.ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        "source-bound attachment has no source object",
                    )
                })?;
                actions.push(BoundAction {
                    effect_index,
                    action: Action::AttachObject {
                        attachment: source,
                        target: Some(resolve_object_target(bindings, *target, effect_index)?),
                    },
                });
            }
            EffectProgram::AddCountersToSource { kind, amount } => {
                let source = bindings.source.ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        "source-bound counter effect has no source object",
                    )
                })?;
                actions.push(BoundAction {
                    effect_index,
                    action: Action::AddObjectCounters {
                        object: source,
                        kind: *kind,
                        amount: *amount,
                    },
                });
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
        if selected.len() < requirement.minimum as usize
            || selected.len() > requirement.maximum as usize
        {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!(
                    "object choice {choice_index} selected {} objects, required range is {}..={}",
                    selected.len(),
                    requirement.minimum,
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
            if !object_satisfies_choice_requirement(
                state,
                *requirement,
                bindings.controller,
                object,
            )? {
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

/// Tests one object against a compiled resolution-time choice requirement.
///
/// This is the authoritative predicate used both to enumerate a legal search
/// surface and to validate the selected binding immediately before execution.
pub fn object_satisfies_choice_requirement(
    state: &GameState,
    requirement: ObjectChoiceRequirement,
    controller: PlayerId,
    object: ObjectId,
) -> Result<bool, ExecutionDiagnostic> {
    let player = match requirement.player {
        PlayerBinding::Controller => controller,
        _ => {
            return Err(ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                "object choice has an unsupported player binding",
            ));
        }
    };
    if state.object_zone(object) != Some(ZoneId::new(Some(player), requirement.zone)) {
        return Ok(false);
    }
    let characteristics = state.object_characteristics(object).map_err(|error| {
        ExecutionDiagnostic::new(
            ExecutionDiagnosticCode::InvalidChoice,
            None,
            format!("object choice characteristics failed: {error:?}"),
        )
    })?;
    Ok(characteristics
        .types()
        .contains_all(requirement.required_types)
        && (requirement.required_any_types == ObjectTypes::none()
            || characteristics
                .types()
                .intersects(requirement.required_any_types))
        && !characteristics
            .types()
            .intersects(requirement.forbidden_types)
        && characteristics
            .supertypes()
            .contains_all(requirement.required_supertypes)
        && characteristics
            .subtypes()
            .contains_all(requirement.required_subtypes)
        && characteristics
            .basic_land_types()
            .contains_all(requirement.required_land_types)
        && (requirement.required_any_land_types == BasicLandTypes::none()
            || characteristics
                .basic_land_types()
                .intersects(requirement.required_any_land_types))
        && ((requirement.required_land_types == BasicLandTypes::none()
            && requirement.required_any_land_types == BasicLandTypes::none())
            || characteristics.types().land()))
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
        PlayerBinding::TriggeringPlayer => {
            let player = bindings.triggering_player.ok_or_else(|| {
                ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::MissingBinding,
                    Some(effect_index),
                    "triggering-player binding is missing",
                )
            })?;
            if !bindings.opponents.contains(&player) {
                return Err(ExecutionDiagnostic::new(
                    ExecutionDiagnosticCode::InvalidChoice,
                    Some(effect_index),
                    "triggering player is not in the bound opponent set",
                ));
            }
            Ok(vec![player])
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

fn resolve_object_set(
    state: &GameState,
    objects: ObjectSetProgram,
    bindings: &ExecutionBindings,
    effect_index: usize,
) -> Result<Vec<ObjectId>, ExecutionDiagnostic> {
    match objects {
        ObjectSetProgram::Target(target) => {
            Ok(vec![resolve_object_target(bindings, target, effect_index)?])
        }
        ObjectSetProgram::Battlefield(predicate) => {
            let battlefield = state
                .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
                .ok_or_else(|| {
                    ExecutionDiagnostic::new(
                        ExecutionDiagnosticCode::MissingBinding,
                        Some(effect_index),
                        "battlefield zone is unavailable",
                    )
                })?;
            Ok(battlefield
                .iter()
                .copied()
                .filter(|object| {
                    state.object_matches_target_predicate(bindings.controller, predicate, *object)
                })
                .collect())
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
        bind_triggered_ability_actions, compile_card_program, execute_program,
        AlternateCostCondition, AlternateCostKind, Capability, CompileDiagnosticCode,
        EffectProgram, ExecutionBindings, ExecutionDiagnosticCode, PlayerBinding, ProgramKind,
        StaticAbilityProgram, TriggeredEventProgram,
    };
    use forge_core::{
        apply, Action, ActivationCondition, BaseCreatureCharacteristics, BaseObjectCharacteristics,
        BasicLandTypes, CardId, ContinuousEffectDuration, GameEvent, GameState, ManaCost, ManaKind,
        ManaPool, ObjectColors, ObjectSubtype, ObjectSupertypes, ObjectTargetPredicate,
        ObjectTypes, Outcome, RestrictionDefinition, RestrictionEffect, StackObjectKind,
        TargetChoice, TargetControllerPredicate, TargetKind, TargetRequirement, TriggerCondition,
        ZoneId, ZoneKind,
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
    const TERMINATE: &str = r#"
card "Terminate" {
  id: "forge:test:terminate"
  layout: normal
  status: unverified_playable
  face "Terminate" {
    cost: "{B}{R}"
    types: "Instant"
    oracle: "Destroy target creature. It can't be regenerated."
    keywords: []
    ability spell {
      effect: destroy(target(permanents(type_is("creature"))), "cannot_regenerate")
    }
  }
}
"#;
    const ARCHMAGE_EMERITUS: &str = r#"
card "Archmage Emeritus" {
  id: "forge:test:archmage-emeritus"
  layout: normal
  status: unverified_playable
  face "Archmage Emeritus" {
    cost: "{2}{U}{U}"
    types: "Creature - Human Wizard"
    oracle: "Magecraft - Whenever you cast or copy an instant or sorcery spell, draw a card."
    power: "2"
    toughness: "2"
    keywords: []
    ability triggered {
      event: event_cast(spells(or(type_is("instant"), type_is("sorcery"))), "cast_or_copy:you")
      effect: draw(1, you())
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
    const BALA_GED_RECOVERY: &str = r#"
card "Bala Ged Recovery // Bala Ged Sanctuary" {
  id: "d2075f58-b0e9-4e85-b7e6-0523a27a1d5b"
  layout: modal_dfc
  status: unverified_playable
  face "Bala Ged Recovery" {
    cost: "{2}{G}"
    types: "Sorcery"
    oracle: "Return target card from your graveyard to your hand."
    keywords: []
    ability spell {
      effect: move_zone_from(target(cards(and(owned_by(you()), zone_is("graveyard")))), "graveyard", "hand")
    }
  }
  face "Bala Ged Sanctuary" {
    cost: ""
    types: "Land"
    oracle: "Bala Ged Sanctuary enters tapped.\n{T}: Add {G}."
    keywords: []
    ability replacement {
      event: event_enters(source())
      effect: etb_effect(tap(source()))
    }
    ability activated {
      costs: [tap_self()]
      effect: add_mana("{G}", you())
      mana_ability: true
    }
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
    const HEROIC_INTERVENTION: &str = r#"
card "Heroic Intervention" {
  id: "24882fa2-3fe9-4c1b-aa3d-0e6488b9db27"
  layout: normal
  status: unverified_playable
  face "Heroic Intervention" {
    cost: "{1}{G}"
    types: "Instant"
    oracle: "Permanents you control gain hexproof and indestructible until end of turn."
    keywords: []
    ability spell {
      effect: sequence(grant_keyword(permanents(controlled_by(you())), "hexproof", "until_end_of_turn"), grant_keyword(permanents(controlled_by(you())), "indestructible", "until_end_of_turn"))
    }
  }
}
"#;
    const BOROS_CHARM: &str = include_str!("../../../cards/cp_dsl/definitions/012_boros_charm.frs");
    const FLAWLESS_MANEUVER: &str = r#"
card "Flawless Maneuver" {
  id: "4e183439-17d2-47ff-9d99-5e22821d91e3"
  layout: normal
  status: unverified_playable
  face "Flawless Maneuver" {
    cost: "{2}{W}"
    types: "Instant"
    oracle: "If you control a commander, you may cast this spell without paying its mana cost. Creatures you control gain indestructible until end of turn."
    keywords: []
    ability static {
      effect: while_condition(at_least(count(permanents(and(designation_is("commander"), controlled_by(you())))), 1), alternate_cost(spells(and(equals(any(), source()), controlled_by(you()))), mana_cost("{0}")))
    }
    ability spell {
      effect: grant_keyword(permanents(and(type_is("creature"), controlled_by(you()))), "indestructible", "until_end_of_turn")
    }
  }
}
"#;
    const FAITHLESS_LOOTING: &str = r#"
card "Faithless Looting" {
  id: "3d6fa57a-aa53-4b5c-b8af-a7612c823117"
  layout: normal
  status: unverified_playable
  face "Faithless Looting" {
    cost: "{R}"
    types: "Sorcery"
    oracle: "Draw two cards, then discard two cards. Flashback {2}{R}."
    keywords: [flashback]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{2}{R}")))
    }
    ability spell {
      effect: sequence(draw(2, you()), discard_cards(2, you(), "choose"))
    }
  }
}
"#;
    const KROSAN_GRIP: &str = r#"
card "Krosan Grip" {
  id: "3e39224c-72ce-4ecc-aa17-12c071ea1f3e"
  layout: normal
  status: unverified_playable
  face "Krosan Grip" {
    cost: "{2}{G}"
    types: "Instant"
    oracle: "Split second. Destroy target artifact or enchantment."
    keywords: [split_second]
    ability spell {
      effect: destroy(target(all(permanents(type_is("artifact")), permanents(type_is("enchantment")))))
    }
  }
}
"#;
    const CYCLONIC_RIFT: &str = r#"
card "Cyclonic Rift" {
  id: "d75b9c82-1b49-4c3e-a1b5-aeef57d6644b"
  layout: normal
  status: unverified_playable
  face "Cyclonic Rift" {
    cost: "{1}{U}"
    types: "Instant"
    oracle: "Return target nonland permanent you don't control to its owner's hand. Overload {6}{U}."
    keywords: [overload]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{6}{U}")))
    }
    ability spell {
      effect: return_to_hand(target(permanents(and(not(type_is("land")), not(controlled_by(you()))))))
    }
  }
}
"#;
    const MULLDRIFTER: &str = r#"
card "Mulldrifter" {
  id: "24d0f5e7-0d9e-4b76-900e-a7274e80312d"
  layout: normal
  status: unverified_playable
  face "Mulldrifter" {
    cost: "{4}{U}"
    types: "Creature - Elemental"
    oracle: "Flying. When Mulldrifter enters, draw two cards. Evoke {2}{U}."
    power: "2"
    toughness: "2"
    keywords: [evoke, flying]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{2}{U}")))
    }
    ability triggered {
      event: event_enters(source())
      effect: draw(2, you())
    }
  }
}
"#;
    const TEMPLE_OF_THE_FALSE_GOD: &str = r#"
card "Temple of the False God" {
  id: "cfdd5dc6-593e-495a-8cfe-3a56b3c4c7df"
  layout: normal
  status: unverified_playable
  face "Temple of the False God" {
    cost: ""
    types: "Land"
    oracle: "{T}: Add {C}{C}. Activate only if you control five or more lands."
    keywords: []
    ability activated {
      costs: [tap_self()]
      timing: timing_condition(at_least(count(permanents(and(type_is("land"), controlled_by(you())))), 5))
      effect: add_mana("2 x {C}", you())
      mana_ability: true
    }
  }
}
"#;
    const RELIQUARY_TOWER: &str = r#"
card "Reliquary Tower" {
  id: "c23e5b80-08d2-4e24-9908-fe2aa4f30f6f"
  layout: normal
  status: unverified_playable
  face "Reliquary Tower" {
    cost: ""
    types: "Land"
    oracle: "You have no maximum hand size.\n{T}: Add {C}."
    keywords: []
    ability static {
      effect: continuous(you(), no_maximum_hand_size(you()))
    }
    ability activated {
      costs: [tap_self()]
      effect: add_mana("{C}", you())
      mana_ability: true
    }
  }
}
"#;
    const CARRION_FEEDER: &str = r#"
card "Carrion Feeder" {
  id: "a1cc5e37-b09a-4b7f-afd5-77c1c35aa425"
  layout: normal
  status: unverified_playable
  face "Carrion Feeder" {
    cost: "{B}"
    types: "Creature - Zombie"
    oracle: "Carrion Feeder can't block. Sacrifice a creature: Put a +1/+1 counter on Carrion Feeder."
    power: "1"
    toughness: "1"
    keywords: []
    ability static {
      effect: continuous(source(), cannot_block(any()))
    }
    ability activated {
      costs: [sacrifice(permanents(and(type_is("creature"), controlled_by(you()))), 1)]
      effect: add_counter(source(), "p1p1", 1)
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
      effect: sequence(search_library(cards(and(type_is("creature"), zone_is("library"))), you(), 1), reveal(chosen(cards(and(type_is("creature"), zone_is("library"))))), move_zone(chosen(cards(and(type_is("creature"), zone_is("library")))), "hand", 1), shuffle(you()))
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
      effect: sequence(search_library(cards(and(or(type_is("artifact"), type_is("enchantment")), zone_is("library"))), you(), 1), reveal(chosen(cards(and(or(type_is("artifact"), type_is("enchantment")), zone_is("library"))))), shuffle(you()), move_zone(chosen(cards(and(or(type_is("artifact"), type_is("enchantment")), zone_is("library")))), "library_top", 1))
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

    const SMOTHERING_TITHE: &str = r#"
card "Smothering Tithe" {
  id: "153376c9-dffd-458c-8ce3-a4c8269bc4e9"
  layout: normal
  status: unverified_playable
  face "Smothering Tithe" {
    cost: "{3}{W}"
    types: "Enchantment"
    oracle: "Whenever an opponent draws a card, that player may pay {2}. If the player doesn't, you create a Treasure token."
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: unless_paid(create_token("c_a_treasure_sac", 1, you()), controller_of(triggered()), mana_cost("{2}"))
    }
  }
}
"#;

    const PURPHOROS: &str = r#"
card "Purphoros, God of the Forge" {
  id: "4fdbbec2-e921-4b63-958d-f9ba1e417197"
  layout: normal
  status: unverified_playable
  face "Purphoros, God of the Forge" {
    cost: "{3}{R}"
    types: "Legendary Enchantment Creature - God"
    oracle: "Purphoros contract fixture."
    power: "6"
    toughness: "5"
    keywords: [indestructible]
    ability static {
      effect: while_condition(less_than(devotion(you(), "red"), 5), continuous(source(), remove_type(any(), "Creature")))
    }
    ability triggered {
      event: event_enters(permanents(and(type_is("creature"), not(equals(any(), source())), controlled_by(you()))))
      effect: deal_damage(opponent(), 2)
    }
    ability activated {
      costs: [mana_cost("{2}{R}")]
      effect: modify_pt(permanents(and(type_is("creature"), controlled_by(you()))), 1, 0, "until_end_of_turn")
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

        let temple = compile_card_program(&parse("temple.frs", TEMPLE_OF_THE_FALSE_GOD))
            .unwrap_or_else(|error| panic!("unexpected Temple compile error: {error}"));
        assert_eq!(temple.kind(), ProgramKind::Land);
        assert_eq!(
            temple.activated_abilities()[0].condition(),
            Some(ActivationCondition::ControllerControlsAtLeast {
                predicate: ObjectTargetPredicate::any()
                    .with_controller(TargetControllerPredicate::You)
                    .with_required_types(ObjectTypes::none().with_land()),
                count: 5,
            })
        );
    }

    #[test]
    fn source_bound_player_rule_compiles_without_mutating_base_player_state() {
        let tower = compile_card_program(&parse("reliquary_tower.frs", RELIQUARY_TOWER))
            .unwrap_or_else(|error| panic!("unexpected Reliquary Tower compile error: {error}"));
        assert_eq!(tower.kind(), ProgramKind::Land);
        assert_eq!(tower.static_abilities().len(), 1);
        assert!(tower
            .capabilities()
            .contains(&Capability::ModifyPlayerRules));
    }

    #[test]
    fn source_bound_counter_activation_compiles_with_real_sacrifice_cost() {
        let feeder = compile_card_program(&parse("carrion_feeder.frs", CARRION_FEEDER))
            .unwrap_or_else(|error| panic!("unexpected Carrion Feeder compile error: {error}"));
        assert_eq!(
            feeder.capabilities(),
            vec![
                Capability::PermanentSpell,
                Capability::ActivatedAbility,
                Capability::CombatRestriction,
                Capability::AddCounters,
            ]
        );
        let ability = &feeder.activated_effects()[0];
        assert!(ability.uses_source_object());
        let (predicate, count) = ability
            .sacrifice_cost()
            .unwrap_or_else(|| panic!("Carrion Feeder must retain its sacrifice cost"));
        assert_eq!(count, 1);
        assert!(predicate.required_types().creature());
        assert_eq!(predicate.controller(), TargetControllerPredicate::You);
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
        let Some(beast) = ObjectSubtype::parse("Beast") else {
            panic!("fixture subtype must parse");
        };
        assert_eq!(
            token.base_object().types(),
            ObjectTypes::none().with_creature()
        );
        assert_eq!(
            token.base_object().colors(),
            ObjectColors::none().with_green()
        );
        assert_eq!(token.base_object().subtypes().as_slice(), &[beast]);
        assert_eq!(
            token.base_creature(),
            Some(BaseCreatureCharacteristics::new(3, 3))
        );
    }

    #[test]
    fn prohibited_regeneration_binds_to_distinct_destruction_action() {
        let program = compile_card_program(&parse("terminate.frs", TERMINATE))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert!(matches!(
            program.effects(),
            [EffectProgram::DestroyPermanentWithoutRegeneration { target: 0 }]
        ));
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let target = create_object(
            &mut state,
            CardId::new(2_012),
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
                        ObjectColors::none(),
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
                    base: BaseCreatureCharacteristics::new(2, 2),
                },
            ),
            Outcome::Applied
        );
        assert!(matches!(
            apply(
                &mut state,
                Action::RegisterRestriction {
                    definition: RestrictionDefinition::new(
                        opponent,
                        RestrictionEffect::RegenerationShield { object: target },
                    )
                    .with_duration(ContinuousEffectDuration::UntilEndOfTurn),
                },
            ),
            Outcome::RestrictionRegistered(_)
        ));

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
        assert_eq!(state.restrictions().count(), 0);
    }

    #[test]
    fn magecraft_compiles_to_cast_or_copy_trigger_condition() {
        let program = compile_card_program(&parse("archmage_emeritus.frs", ARCHMAGE_EMERITUS))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let [ability] = program.triggered_abilities() else {
            panic!("expected one magecraft trigger");
        };
        assert!(matches!(
            ability.event(),
            TriggeredEventProgram::ControllerCastsOrCopies(_)
        ));
        let mut state = GameState::new();
        let controller = add_player(&mut state);
        let source = create_object(
            &mut state,
            CardId::new(2_013),
            controller,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        assert!(matches!(
            ability.bind(controller, source).condition(),
            TriggerCondition::StackEntryAddedOrCopied { .. }
        ));
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
                Capability::RevealObjects,
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

        let cursor = state.event_cursor();
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
        assert!(state
            .events_since(cursor)
            .unwrap_or_else(|error| panic!("reveal event replay failed: {error:?}"))
            .iter()
            .any(|record| record.event() == GameEvent::ObjectRevealed { object: creature }));
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
        let Some(elf) = ObjectSubtype::parse("Elf") else {
            panic!("fixture subtype is valid");
        };
        assert!(requirement.required_subtypes().contains(elf));
    }

    #[test]
    fn heroic_intervention_protects_each_controlled_permanent() {
        let program = compile_card_program(&parse("heroic_intervention.frs", HEROIC_INTERVENTION))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![Capability::TargetingRestriction, Capability::Indestructible]
        );
        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let artifact = create_object(&mut state, CardId::new(2_200), caster, battlefield);
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: artifact,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_artifact(),
                        ObjectColors::none(),
                    ),
                },
            ),
            Outcome::Applied
        );
        let creature = create_object(&mut state, CardId::new(2_201), caster, battlefield);
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: creature,
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
                    object: creature,
                    base: BaseCreatureCharacteristics::new(2, 2),
                },
            ),
            Outcome::Applied
        );

        let trace = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(trace.records().len(), 4);
        assert_eq!(state.restrictions().count(), 4);
        assert!(state
            .validate_target_choices(
                opponent,
                None,
                &[TargetRequirement::new(TargetKind::Permanent)],
                &[TargetChoice::Object(creature)],
            )
            .is_err());
        assert_eq!(
            apply(&mut state, Action::DestroyPermanent { object: artifact }),
            Outcome::Applied
        );
        assert_eq!(state.object_zone(artifact), Some(battlefield));
    }

    #[test]
    fn boros_charm_compiles_and_executes_each_announced_mode() {
        let program = compile_card_program(&parse("boros_charm.frs", BOROS_CHARM))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(program.spell_modes().len(), 3);
        assert_eq!(program.spell_modes()[0].target_requirements().len(), 1);
        assert!(program.spell_modes()[1].target_requirements().is_empty());
        assert_eq!(program.spell_modes()[2].target_requirements().len(), 1);
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::ChooseMode,
                Capability::DealDamage,
                Capability::Indestructible,
                Capability::ModifyCharacteristics,
            ]
        );

        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let creature = create_object(&mut state, CardId::new(2_202), caster, battlefield);
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: creature,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature(),
                        ObjectColors::none().with_white(),
                    ),
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics {
                    object: creature,
                    base: BaseCreatureCharacteristics::new(2, 2),
                },
            ),
            Outcome::Applied
        );

        assert!(execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent]),
        )
        .is_err());
        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_spell_mode(0)
                .with_targets(vec![TargetChoice::Player(opponent)]),
        )
        .unwrap_or_else(|error| panic!("unexpected damage-mode error: {error}"));
        assert_eq!(state.players()[opponent.index()].life(), 16);

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent]).with_spell_mode(1),
        )
        .unwrap_or_else(|error| panic!("unexpected indestructible-mode error: {error}"));
        assert_eq!(
            apply(&mut state, Action::DestroyPermanent { object: creature }),
            Outcome::Applied
        );
        assert_eq!(state.object_zone(creature), Some(battlefield));

        execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_spell_mode(2)
                .with_targets(vec![TargetChoice::Object(creature)]),
        )
        .unwrap_or_else(|error| panic!("unexpected double-strike-mode error: {error}"));
        assert!(state
            .creature_characteristics(creature)
            .is_ok_and(|characteristics| characteristics.keywords().double_strike()));
    }

    #[test]
    fn commander_condition_enables_exact_zero_mana_alternate_cost() {
        let program = compile_card_program(&parse("flawless_maneuver.frs", FLAWLESS_MANEUVER))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![Capability::AlternateCost, Capability::Indestructible]
        );
        let [alternate] = program.alternate_costs() else {
            panic!("expected one alternate cost");
        };
        assert_eq!(
            alternate.condition(),
            AlternateCostCondition::ControllerControlsCommander
        );
        assert_eq!(alternate.mana_cost(), ManaCost::new(0, 0, 0, 0, 0, 0));
        assert_eq!(alternate.exact_payment(), ManaPool::empty());

        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        assert!(!alternate.is_available(&state, caster, None));
        let commander = create_object(
            &mut state,
            CardId::new(2_202),
            caster,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: commander,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature(),
                        ObjectColors::none().with_white(),
                    ),
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics {
                    object: commander,
                    base: BaseCreatureCharacteristics::new(2, 2),
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::DesignateCommander {
                    object: commander,
                    color_identity: ObjectColors::none().with_white(),
                },
            ),
            Outcome::Applied
        );
        assert!(alternate.is_available(&state, caster, None));
        let trace = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(trace.records().len(), 1);
        assert_eq!(state.restrictions().count(), 1);
    }

    #[test]
    fn flashback_and_exact_chosen_discard_are_bound_fail_closed() {
        let program = compile_card_program(&parse("faithless_looting.frs", FAITHLESS_LOOTING))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::AlternateCost,
                Capability::DrawCards,
                Capability::DiscardCards,
            ]
        );
        let [alternate] = program.alternate_costs() else {
            panic!("expected one flashback cost");
        };
        assert_eq!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerGraveyard
        );

        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let source = create_object(
            &mut state,
            CardId::new(2_203),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Hand),
        );
        assert!(!alternate.is_available(&state, caster, Some(source)));
        assert_eq!(
            apply(
                &mut state,
                Action::MoveObject {
                    object: source,
                    to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
                },
            ),
            Outcome::Applied
        );
        assert!(alternate.is_available(&state, caster, Some(source)));

        let first = create_object(
            &mut state,
            CardId::new(2_204),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Hand),
        );
        let second = create_object(
            &mut state,
            CardId::new(2_205),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Hand),
        );
        for card in 2_206..2_208 {
            create_object(
                &mut state,
                CardId::new(card),
                caster,
                ZoneId::new(Some(caster), ZoneKind::Library),
            );
        }
        let before = state.deterministic_hash();
        assert!(execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent]).with_object_choices(vec![vec![first]]),
        )
        .is_err());
        assert_eq!(state.deterministic_hash(), before);

        let trace = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_object_choices(vec![vec![first, second]]),
        )
        .unwrap_or_else(|error| panic!("unexpected execution error: {error}"));
        assert_eq!(trace.records().len(), 3);
        assert_eq!(
            state.object_zone(first),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        assert_eq!(
            state.object_zone(second),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        assert_eq!(
            state
                .zone_objects(ZoneId::new(Some(caster), ZoneKind::Hand))
                .map(<[forge_core::ObjectId]>::len),
            Some(2)
        );
    }

    #[test]
    fn split_second_is_compiled_only_as_a_stack_rule_on_spells() {
        let program = compile_card_program(&parse("krosan_grip.frs", KROSAN_GRIP))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert!(program.split_second());
        assert_eq!(
            program.capabilities(),
            vec![Capability::SplitSecond, Capability::DestroyPermanent]
        );
        assert_eq!(program.target_requirements().len(), 1);

        let invalid = KROSAN_GRIP.replace("types: \"Instant\"", "types: \"Artifact\"");
        let Err(error) = compile_card_program(&parse("invalid_split_second.frs", &invalid)) else {
            panic!("split second on a permanent must be rejected");
        };
        assert_eq!(error.code(), CompileDiagnosticCode::KeywordSemantics);
    }

    #[test]
    fn overload_replaces_target_with_each_matching_opponent_permanent() {
        let program = compile_card_program(&parse("cyclonic_rift.frs", CYCLONIC_RIFT))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert!(program.overload());
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::AlternateCost,
                Capability::Overload,
                Capability::MoveZone,
            ]
        );
        let [alternate] = program.alternate_costs() else {
            panic!("expected one overload cost");
        };
        assert_eq!(alternate.kind(), AlternateCostKind::Overload);
        assert_eq!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerHand
        );
        assert_eq!(alternate.mana_cost(), ManaCost::new(0, 1, 0, 0, 0, 6));

        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let source = create_object(
            &mut state,
            CardId::new(2_300),
            caster,
            ZoneId::new(Some(caster), ZoneKind::Hand),
        );
        assert!(alternate.is_available(&state, caster, Some(source)));
        let first = create_object(&mut state, CardId::new(2_301), opponent, battlefield);
        let second = create_object(&mut state, CardId::new(2_302), opponent, battlefield);
        let friendly = create_object(&mut state, CardId::new(2_303), caster, battlefield);
        let land = create_object(&mut state, CardId::new(2_304), opponent, battlefield);
        for object in [first, second, friendly] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(
                            ObjectTypes::none().with_artifact(),
                            ObjectColors::none(),
                        ),
                    },
                ),
                Outcome::Applied
            );
        }
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: land,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_land(),
                        ObjectColors::none(),
                    ),
                },
            ),
            Outcome::Applied
        );
        let bindings = ExecutionBindings::new(caster, vec![opponent])
            .with_alternate_cost(AlternateCostKind::Overload);
        let trace = execute_program(&mut state, &program, &bindings)
            .unwrap_or_else(|error| panic!("unexpected overload execution error: {error}"));
        assert_eq!(trace.records().len(), 2);
        let opponent_hand = ZoneId::new(Some(opponent), ZoneKind::Hand);
        assert_eq!(state.object_zone(first), Some(opponent_hand));
        assert_eq!(state.object_zone(second), Some(opponent_hand));
        assert_eq!(state.object_zone(friendly), Some(battlefield));
        assert_eq!(state.object_zone(land), Some(battlefield));
    }

    #[test]
    fn modal_dfc_compiles_and_executes_each_face_without_cross_face_leakage() {
        let program = compile_card_program(&parse("bala_ged_recovery.frs", BALA_GED_RECOVERY))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(program.kind(), ProgramKind::Sorcery);
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::ModalDfc,
                Capability::MoveZone,
                Capability::LandPlay,
                Capability::ManaAbility,
            ]
        );
        let Some(back) = program.modal_dfc_back() else {
            panic!("expected compiled back face");
        };
        assert_eq!(back.kind(), ProgramKind::Land);
        assert!(back.base_object().enters_tapped());
        let [mana_ability] = back.activated_abilities() else {
            panic!("expected one back-face mana ability");
        };
        assert_eq!(mana_ability.produces(), ManaPool::of(ManaKind::Green, 1));

        let mut state = GameState::new();
        let caster = add_player(&mut state);
        let opponent = add_player(&mut state);
        let graveyard = ZoneId::new(Some(caster), ZoneKind::Graveyard);
        let hand = ZoneId::new(Some(caster), ZoneKind::Hand);
        let recovered = create_object(&mut state, CardId::new(2_500), caster, graveyard);
        let trace = execute_program(
            &mut state,
            &program,
            &ExecutionBindings::new(caster, vec![opponent])
                .with_targets(vec![TargetChoice::Object(recovered)]),
        )
        .unwrap_or_else(|error| panic!("unexpected front-face execution error: {error}"));
        assert_eq!(trace.records().len(), 1);
        assert_eq!(state.object_zone(recovered), Some(hand));

        let land = create_object(&mut state, CardId::new(2_501), caster, hand);
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: land,
                    base: back.base_object(),
                },
            ),
            Outcome::Applied
        );
        prepare_turn(&mut state, caster, opponent);
        for _ in 0..8 {
            if state.current_step() == Some(forge_core::Step::PrecombatMain) {
                break;
            }
            let Some(player) = state.priority_player() else {
                panic!("expected priority player");
            };
            assert!(matches!(
                apply(&mut state, Action::PassPriority { player }),
                Outcome::Priority(_)
            ));
        }
        assert_eq!(state.current_step(), Some(forge_core::Step::PrecombatMain));
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: caster,
                    object: land,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(land),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert!(state.object(land).is_some_and(|record| record.tapped()));
    }

    #[test]
    fn evoke_adds_only_the_alternate_cast_source_sacrifice_trigger() {
        let program = compile_card_program(&parse("mulldrifter.frs", MULLDRIFTER))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program.capabilities(),
            vec![
                Capability::PermanentSpell,
                Capability::AlternateCost,
                Capability::DrawCards,
                Capability::SacrificePermanent,
            ]
        );
        let [alternate] = program.alternate_costs() else {
            panic!("expected one evoke cost");
        };
        assert_eq!(alternate.kind(), AlternateCostKind::Evoke);
        assert_eq!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerHand
        );
        assert_eq!(alternate.mana_cost(), ManaCost::new(0, 1, 0, 0, 0, 2));
        let [draw_trigger, evoke_trigger] = program.triggered_abilities() else {
            panic!("expected draw and evoke triggers");
        };
        assert_eq!(draw_trigger.required_alternate_cost(), None);
        assert_eq!(
            evoke_trigger.required_alternate_cost(),
            Some(AlternateCostKind::Evoke)
        );

        let state = &mut GameState::new();
        let caster = add_player(state);
        let source = create_object(
            state,
            CardId::new(2_400),
            caster,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        let bindings = ExecutionBindings::new(caster, Vec::new()).with_source(source);
        let actions = bind_triggered_ability_actions(state, evoke_trigger, &bindings)
            .unwrap_or_else(|error| panic!("unexpected evoke binding error: {error}"));
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0].action(),
            Action::MoveObject { object, to }
                if *object == source
                    && *to == ZoneId::new(Some(caster), ZoneKind::Graveyard)
        ));
    }

    #[test]
    fn opponent_draw_unless_paid_binds_both_exact_branches() {
        let program = compile_card_program(&parse("smothering_tithe.frs", SMOTHERING_TITHE))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        let [ability] = program.triggered_abilities() else {
            panic!("expected one draw trigger");
        };
        assert_eq!(ability.event(), TriggeredEventProgram::OpponentDrawsCard);
        let Some(unless_paid) = ability.unless_paid() else {
            panic!("expected exact unless branch");
        };
        assert_eq!(unless_paid.payer(), PlayerBinding::TriggeringPlayer);
        assert_eq!(unless_paid.mana_cost(), ManaCost::new(0, 0, 0, 0, 0, 2));
        assert_eq!(unless_paid.exact_payment(), ManaPool::new(0, 0, 0, 0, 0, 2));

        let mut state = GameState::new();
        let controller = add_player(&mut state);
        let opponent = add_player(&mut state);
        let base =
            ExecutionBindings::new(controller, vec![opponent]).with_triggering_player(opponent);
        let Err(missing) = bind_triggered_ability_actions(&state, ability, &base) else {
            panic!("payment decision must be explicit");
        };
        assert_eq!(missing.code(), ExecutionDiagnosticCode::MissingChoice);

        let decline = bind_triggered_ability_actions(
            &state,
            ability,
            &base.clone().with_unless_payment(false),
        )
        .unwrap_or_else(|error| panic!("unexpected decline binding error: {error}"));
        assert_eq!(decline.len(), 1);
        assert!(matches!(decline[0].action(), Action::CreateToken { .. }));

        let pay = bind_triggered_ability_actions(&state, ability, &base.with_unless_payment(true))
            .unwrap_or_else(|error| panic!("unexpected payment binding error: {error}"));
        assert_eq!(pay.len(), 1);
        assert!(matches!(
            pay[0].action(),
            Action::PayMana { player, cost, .. }
                if *player == opponent && *cost == ManaCost::new(0, 0, 0, 0, 0, 2)
        ));
    }

    #[test]
    fn purphoros_compiles_live_devotion_enter_trigger_and_player_damage() {
        let program = compile_card_program(&parse("purphoros.frs", PURPHOROS))
            .unwrap_or_else(|error| panic!("unexpected compile error: {error}"));
        assert_eq!(
            program
                .base_object()
                .printed_mana_symbols()
                .get(ManaKind::Red),
            1
        );
        let [static_ability] = program.static_abilities() else {
            panic!("expected one devotion ability");
        };
        assert!(matches!(
            static_ability,
            StaticAbilityProgram::DevotionSourceTypeRemoval {
                color: ManaKind::Red,
                threshold: 5,
                types,
            } if *types == ObjectTypes::none().with_creature()
        ));
        let [trigger] = program.triggered_abilities() else {
            panic!("expected one enter trigger");
        };
        assert!(matches!(
            trigger.event(),
            TriggeredEventProgram::ControllerPermanentEnters {
                exclude_source: true,
                ..
            }
        ));

        let mut state = GameState::new();
        let controller = add_player(&mut state);
        let opponent = add_player(&mut state);
        let source = create_object(
            &mut state,
            CardId::new(2_500),
            controller,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: source,
                    base: program.base_object(),
                },
            ),
            Outcome::Applied
        );
        let Some(base_creature) = program.base_creature() else {
            panic!("expected creature base");
        };
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics {
                    object: source,
                    base: base_creature,
                },
            ),
            Outcome::Applied
        );
        for action in static_ability.bind_actions(controller, source) {
            assert!(matches!(
                apply(&mut state, action),
                Outcome::ContinuousEffectRegistered(_)
            ));
        }
        assert_eq!(
            state.creature_characteristics(source),
            Err(forge_core::StateError::NotACreature(source))
        );

        let actions = bind_triggered_ability_actions(
            &state,
            trigger,
            &ExecutionBindings::new(controller, vec![opponent]).with_source(source),
        )
        .unwrap_or_else(|error| panic!("unexpected trigger binding error: {error}"));
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0].action(),
            Action::DealDamage {
                source: Some(bound_source),
                target: forge_core::CombatDamageTarget::Player(player),
                amount: 2,
            } if *bound_source == source && *player == opponent
        ));
    }

    #[test]
    fn x_mana_symbols_compile_as_dynamic_costs_and_zero_printed_value() {
        let source = r#"card "Double X Test" {
  id: "forge:test:double-x"
  layout: normal
  status: unverified_playable
  face "Double X Test" {
    cost: "{X}{X}{R}"
    types: "Sorcery"
    oracle: "You gain 1 life."
    keywords: []
    ability spell {
      effect: gain_life(1, you())
    }
  }
}"#;
        let program = compile_card_program(&parse("double_x_test.frs", source))
            .unwrap_or_else(|error| panic!("X-cost fixture should compile: {error}"));
        assert_eq!(
            program.mana_cost(),
            ManaCost::new(0, 0, 0, 1, 0, 0).with_x(2, 0)
        );
        assert_eq!(program.base_object().mana_value(), 1);
        assert_eq!(program.exact_payment(), ManaPool::new(0, 0, 0, 1, 0, 0));

        let unsupported = source
            .replace("{X}{X}{R}", "{Y}{R}")
            .replace("double-x", "variable-y");
        let error = match compile_card_program(&parse("variable_y_test.frs", &unsupported)) {
            Ok(_) => panic!("independent Y values must remain fail-closed"),
            Err(error) => error,
        };
        assert_eq!(error.code(), CompileDiagnosticCode::ManaSymbol);
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
