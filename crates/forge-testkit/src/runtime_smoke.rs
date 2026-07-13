//! Capability-specific runtime smoke for compiler-valid translated cards.
//!
//! The T3.5 harness drives compiled card programs through legal casting or land
//! play, targeting, production effects, activated mana abilities, zones, and
//! invariant checks. Unsupported definitions are returned as data with stable
//! reason codes; they are never counted as passing smoke.

use forge_carddef::CardDefinition;
use forge_cards::runtime::{
    bind_activated_effect_actions, bind_program_actions, bind_triggered_ability_actions,
    compile_card_program, ActivatedEffectProgram, AlternateCostCondition, AmountProgram,
    CardProgram, ChosenDestination, CompileDiagnostic, CompileDiagnosticCode, EffectProgram,
    ExecutionBindings, PlayerBinding, ProgramKind, SpellAdditionalCostProgram,
    TriggeredEventProgram,
};
use forge_core::{
    apply, auto_payment_plan, Action, ActivatedAbilityId, ActivationCondition, ActivationTiming,
    AttackDeclaration, BaseCreatureCharacteristics, BaseObjectCharacteristics, BasicLandTypes,
    CardId, CastSpellRequest, GameOutcome, GameState, ManaPool, ObjectColors, ObjectId,
    ObjectTypes, Outcome, PlayerId, PlayerTargetPredicate, PriorityOutcome, SpellTiming,
    StackEntryId, StackObjectKind, Step, TargetChoice, TargetControllerPredicate, TargetKind,
    TargetPredicate, TriggerId, ZoneId, ZoneKind,
};

/// Capability executed through the shared card runtime interpreter.
pub use forge_cards::runtime::Capability as RuntimeSmokeCapability;

const PLAYER_COUNT: usize = 2;
const DEFAULT_LIFE: u64 = 20;
const LIFE_SANITY_LIMIT: u64 = 1_000_000;
const OPENING_HAND_SIZE: u32 = 7;

/// Top-level outcome category for one translated-card runtime smoke attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSmokeDisposition {
    /// The supported capability executed and all assertions passed.
    Passed,
    /// The harness cannot honestly synthesize this card setup yet.
    UnsupportedSetup,
    /// A supported setup reached an unexpected production-runtime failure.
    Failed,
}

impl RuntimeSmokeDisposition {
    /// Returns the stable machine-readable disposition.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::UnsupportedSetup => "unsupported_setup",
            Self::Failed => "failed",
        }
    }
}

/// Stable reason that setup synthesis was not supported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedSetupCode {
    /// The definition is not classified as playable.
    CardNotPlayable,
    /// Only the ordinary one-face layout is supported.
    CardLayout,
    /// The definition does not contain exactly one face.
    FaceCount,
    /// The face is not exactly an instant or sorcery.
    CardType,
    /// Keyword semantics would affect the smoke but are not synthesized.
    KeywordSemantics,
    /// The face does not contain exactly one unconditional spell ability.
    AbilityShape,
    /// A mana symbol cannot be represented exactly by this setup.
    ManaSymbol,
    /// The effect operation has no first-slice runtime lowering.
    EffectOperation,
    /// A supported effect has an unsupported argument shape.
    EffectArguments,
    /// An effect amount is dynamic, negative, or outside the production action range.
    EffectAmount,
    /// A player selector cannot be bound by the two-player synthesizer.
    PlayerSelector,
    /// The bounded setup would exceed action-count or scalar safety limits.
    SetupBounds,
}

impl UnsupportedSetupCode {
    /// Returns the stable machine-readable reason code.
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
            Self::SetupBounds => "unsupported_setup_bounds",
        }
    }
}

/// Stable category for a failure after a setup was accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSmokeFailureCode {
    /// A production action returned [`Outcome::Failed`].
    ProductionActionRejected,
    /// A production action returned an unexpected success payload.
    UnexpectedOutcome,
    /// A required kernel invariant failed after an action.
    InvariantViolation,
    /// The translated spell reached an unexpected zone.
    ZoneDestinationMismatch,
    /// A life capability produced an unexpected total.
    LifeTotalMismatch,
    /// A draw capability produced an unexpected hand size.
    HandSizeMismatch,
}

impl RuntimeSmokeFailureCode {
    /// Returns the stable machine-readable failure code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProductionActionRejected => "production_action_rejected",
            Self::UnexpectedOutcome => "unexpected_action_outcome",
            Self::InvariantViolation => "invariant_violation",
            Self::ZoneDestinationMismatch => "zone_destination_mismatch",
            Self::LifeTotalMismatch => "life_total_mismatch",
            Self::HandSizeMismatch => "hand_size_mismatch",
        }
    }
}

/// Successful runtime evidence for one supported translated card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSmokePass {
    capabilities: Vec<RuntimeSmokeCapability>,
    effect_actions: usize,
    production_actions: usize,
    final_life_totals: [i32; PLAYER_COUNT],
    final_hash: u64,
    destination: &'static str,
}

impl RuntimeSmokePass {
    /// Returns capabilities executed in translated effect order.
    #[must_use]
    pub fn capabilities(&self) -> &[RuntimeSmokeCapability] {
        &self.capabilities
    }

    /// Returns the number of translated effect actions executed.
    #[must_use]
    pub const fn effect_actions(&self) -> usize {
        self.effect_actions
    }

    /// Returns the total number of successful production actions dispatched.
    #[must_use]
    pub const fn production_actions(&self) -> usize {
        self.production_actions
    }

    /// Returns final life totals in caster, opponent order.
    #[must_use]
    pub const fn final_life_totals(&self) -> [i32; PLAYER_COUNT] {
        self.final_life_totals
    }

    /// Returns the final deterministic state hash.
    #[must_use]
    pub const fn final_hash(&self) -> u64 {
        self.final_hash
    }

    /// Returns the asserted translated-spell destination.
    #[must_use]
    pub const fn destination(&self) -> &'static str {
        self.destination
    }
}

/// Explicit unsupported-setup result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSmokeUnsupported {
    code: UnsupportedSetupCode,
    detail: String,
}

impl RuntimeSmokeUnsupported {
    fn new(code: UnsupportedSetupCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// Returns the stable unsupported-setup code.
    #[must_use]
    pub const fn code(&self) -> UnsupportedSetupCode {
        self.code
    }

    /// Returns a card-specific diagnostic without changing the stable code.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Runtime failure for a setup the harness claimed to support.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSmokeFailure {
    code: RuntimeSmokeFailureCode,
    phase: String,
    detail: String,
}

impl RuntimeSmokeFailure {
    fn new(
        code: RuntimeSmokeFailureCode,
        phase: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            phase: phase.into(),
            detail: detail.into(),
        }
    }

    /// Returns the stable failure code.
    #[must_use]
    pub const fn code(&self) -> RuntimeSmokeFailureCode {
        self.code
    }

    /// Returns the production-action or assertion phase that failed.
    #[must_use]
    pub fn phase(&self) -> &str {
        &self.phase
    }

    /// Returns the failure detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Variant payload for one translated-card runtime smoke report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeSmokeResult {
    /// Supported setup and runtime assertions passed.
    Passed(RuntimeSmokePass),
    /// Setup synthesis is unsupported for the supplied card.
    UnsupportedSetup(RuntimeSmokeUnsupported),
    /// Accepted setup failed during production execution or assertion.
    Failed(RuntimeSmokeFailure),
}

/// Identity-bound result for one translated-card runtime smoke attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSmokeReport {
    oracle_id: String,
    card_name: String,
    result: RuntimeSmokeResult,
}

impl RuntimeSmokeReport {
    /// Returns the card's stable Oracle identity.
    #[must_use]
    pub fn oracle_id(&self) -> &str {
        &self.oracle_id
    }

    /// Returns the card name.
    #[must_use]
    pub fn card_name(&self) -> &str {
        &self.card_name
    }

    /// Returns the typed runtime result.
    #[must_use]
    pub const fn result(&self) -> &RuntimeSmokeResult {
        &self.result
    }

    /// Returns the top-level disposition.
    #[must_use]
    pub const fn disposition(&self) -> RuntimeSmokeDisposition {
        match self.result {
            RuntimeSmokeResult::Passed(_) => RuntimeSmokeDisposition::Passed,
            RuntimeSmokeResult::UnsupportedSetup(_) => RuntimeSmokeDisposition::UnsupportedSetup,
            RuntimeSmokeResult::Failed(_) => RuntimeSmokeDisposition::Failed,
        }
    }

    /// Returns true only for an executed, assertion-clean capability smoke.
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self.result, RuntimeSmokeResult::Passed(_))
    }
}

/// Synthesizes and executes one capability-specific smoke for a translated card.
///
/// Unsupported cards produce [`RuntimeSmokeResult::UnsupportedSetup`]. For the
/// supported first slice, casting and zone movement use [`Action::CastSpell`]
/// and priority passes. The translated life effects are then dispatched through
/// [`apply`] with no intervening gameplay action.
#[must_use]
pub fn run_translated_card_runtime_smoke(definition: &CardDefinition) -> RuntimeSmokeReport {
    let result = match compile_smoke(definition) {
        Ok(compiled) => match execute_smoke(definition, &compiled) {
            Ok(pass) => RuntimeSmokeResult::Passed(pass),
            Err(failure) => RuntimeSmokeResult::Failed(failure),
        },
        Err(unsupported) => RuntimeSmokeResult::UnsupportedSetup(unsupported),
    };
    RuntimeSmokeReport {
        oracle_id: definition.id.as_str().to_owned(),
        card_name: definition.name.clone(),
        result,
    }
}

struct CompiledSmoke {
    program: CardProgram,
    lifecycle: SmokeSpellKind,
    initial_life: [i32; PLAYER_COUNT],
    expected_life: [i32; PLAYER_COUNT],
    library_reserve: [u32; PLAYER_COUNT],
}

#[derive(Clone, Copy)]
enum SmokeSpellKind {
    Instant,
    Sorcery,
    Permanent,
    Land,
}

impl SmokeSpellKind {
    const fn spell_profile(self) -> Option<(StackObjectKind, SpellTiming)> {
        match self {
            Self::Instant => Some((StackObjectKind::InstantSpell, SpellTiming::Instant)),
            Self::Sorcery => Some((StackObjectKind::SorcerySpell, SpellTiming::Sorcery)),
            Self::Permanent => Some((StackObjectKind::PermanentSpell, SpellTiming::Sorcery)),
            Self::Land => None,
        }
    }

    const fn destination(self, owner: PlayerId) -> (ZoneId, &'static str) {
        match self {
            Self::Instant | Self::Sorcery => (
                ZoneId::new(Some(owner), ZoneKind::Graveyard),
                "owner_graveyard",
            ),
            Self::Permanent | Self::Land => {
                (ZoneId::new(None, ZoneKind::Battlefield), "battlefield")
            }
        }
    }
}

fn compile_smoke(definition: &CardDefinition) -> Result<CompiledSmoke, RuntimeSmokeUnsupported> {
    let program = compile_card_program(definition).map_err(map_compile_diagnostic)?;
    let lifecycle = match program.kind() {
        ProgramKind::Instant => SmokeSpellKind::Instant,
        ProgramKind::Sorcery => SmokeSpellKind::Sorcery,
        ProgramKind::Permanent => SmokeSpellKind::Permanent,
        ProgramKind::Land => SmokeSpellKind::Land,
    };
    let all_effects = program
        .effects()
        .iter()
        .chain(
            program
                .activated_effects()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .chain(
            program
                .triggered_abilities()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .cloned()
        .collect::<Vec<_>>();
    let activation_life_cost = program
        .activated_effects()
        .iter()
        .map(|ability| u64::from(ability.pay_life()))
        .try_fold(0_u64, u64::checked_add)
        .and_then(|total| {
            program
                .activated_abilities()
                .iter()
                .map(|ability| u64::from(ability.damage_to_controller()))
                .try_fold(total, u64::checked_add)
        })
        .ok_or_else(|| {
            RuntimeSmokeUnsupported::new(
                UnsupportedSetupCode::SetupBounds,
                "cumulative activation life cost overflowed",
            )
        })?;
    let (initial_life, expected_life) = compile_life_totals(&all_effects, activation_life_cost)?;
    let mut library_reserve = compile_library_reserve(&all_effects)?;
    let needs_later_turn = program.triggered_abilities().iter().any(|ability| {
        matches!(
            ability.event(),
            TriggeredEventProgram::SourceAttacks
                | TriggeredEventProgram::AttachedObjectAttacks
                | TriggeredEventProgram::ControllerUpkeep
        )
    });
    if needs_later_turn
        || (program
            .base_creature()
            .is_some_and(|base| !base.keywords().haste())
            && (!program.activated_abilities().is_empty()
                || program
                    .activated_effects()
                    .iter()
                    .any(ActivatedEffectProgram::tap_source)))
    {
        for reserve in &mut library_reserve {
            *reserve = reserve.checked_add(1).ok_or_else(|| {
                RuntimeSmokeUnsupported::new(
                    UnsupportedSetupCode::SetupBounds,
                    "creature-ability turn reserve overflowed",
                )
            })?;
        }
    }
    Ok(CompiledSmoke {
        program,
        lifecycle,
        initial_life,
        expected_life,
        library_reserve,
    })
}

fn map_compile_diagnostic(diagnostic: CompileDiagnostic) -> RuntimeSmokeUnsupported {
    let code = match diagnostic.code() {
        CompileDiagnosticCode::CardNotPlayable => UnsupportedSetupCode::CardNotPlayable,
        CompileDiagnosticCode::CardLayout => UnsupportedSetupCode::CardLayout,
        CompileDiagnosticCode::FaceCount => UnsupportedSetupCode::FaceCount,
        CompileDiagnosticCode::CardType => UnsupportedSetupCode::CardType,
        CompileDiagnosticCode::KeywordSemantics => UnsupportedSetupCode::KeywordSemantics,
        CompileDiagnosticCode::AbilityShape => UnsupportedSetupCode::AbilityShape,
        CompileDiagnosticCode::ManaSymbol => UnsupportedSetupCode::ManaSymbol,
        CompileDiagnosticCode::EffectOperation => UnsupportedSetupCode::EffectOperation,
        CompileDiagnosticCode::EffectArguments => UnsupportedSetupCode::EffectArguments,
        CompileDiagnosticCode::EffectAmount => UnsupportedSetupCode::EffectAmount,
        CompileDiagnosticCode::PlayerSelector => UnsupportedSetupCode::PlayerSelector,
        CompileDiagnosticCode::ProgramBounds => UnsupportedSetupCode::SetupBounds,
    };
    RuntimeSmokeUnsupported::new(
        code,
        format!("{}: {}", diagnostic.path(), diagnostic.detail()),
    )
}

fn compile_life_totals(
    effects: &[EffectProgram],
    controller_life_cost: u64,
) -> Result<([i32; PLAYER_COUNT], [i32; PLAYER_COUNT]), RuntimeSmokeUnsupported> {
    let mut gains = [0_u64; PLAYER_COUNT];
    let mut losses = [0_u64; PLAYER_COUNT];
    losses[0] = controller_life_cost;
    for effect in effects {
        let (totals, players, amount) = match effect {
            EffectProgram::GainLife { players, amount } => {
                (&mut gains, *players, smoke_amount(*amount)?)
            }
            EffectProgram::LoseLife { players, amount } => {
                (&mut losses, *players, smoke_amount(*amount)?)
            }
            EffectProgram::DrawCards { .. }
            | EffectProgram::DiscardHands { .. }
            | EffectProgram::Scry { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::MoveTargetObject { .. }
            | EffectProgram::CreateTokens { .. }
            | EffectProgram::SearchLibrary { .. }
            | EffectProgram::MoveChosenObjects { .. }
            | EffectProgram::TapChosenObjects { .. }
            | EffectProgram::ModifyPowerToughness { .. }
            | EffectProgram::GrantKeywords { .. }
            | EffectProgram::GrantTargetingRestriction { .. }
            | EffectProgram::GrantIndestructible { .. }
            | EffectProgram::AttachSourceToTarget { .. }
            | EffectProgram::AddCountersToSource { .. } => continue,
        };
        for (player, selected) in smoke_player_mask(players).into_iter().enumerate() {
            if selected {
                totals[player] =
                    totals[player]
                        .checked_add(u64::from(amount))
                        .ok_or_else(|| {
                            RuntimeSmokeUnsupported::new(
                                UnsupportedSetupCode::SetupBounds,
                                "cumulative life delta overflowed",
                            )
                        })?;
            }
        }
    }

    let mut initial = [0_i32; PLAYER_COUNT];
    let mut expected = [0_i32; PLAYER_COUNT];
    for player in 0..PLAYER_COUNT {
        let minimum_surviving_life = losses[player].checked_add(1).ok_or_else(|| {
            RuntimeSmokeUnsupported::new(UnsupportedSetupCode::SetupBounds, "loss setup overflowed")
        })?;
        let start = DEFAULT_LIFE.max(minimum_surviving_life);
        let maximum = start.checked_add(gains[player]).ok_or_else(|| {
            RuntimeSmokeUnsupported::new(UnsupportedSetupCode::SetupBounds, "gain setup overflowed")
        })?;
        if maximum > LIFE_SANITY_LIMIT {
            return Err(RuntimeSmokeUnsupported::new(
                UnsupportedSetupCode::SetupBounds,
                format!("player {player} setup life would reach {maximum}"),
            ));
        }
        let final_life = start + gains[player] - losses[player];
        initial[player] = i32::try_from(start).map_err(|_| {
            RuntimeSmokeUnsupported::new(
                UnsupportedSetupCode::SetupBounds,
                "initial life does not fit i32",
            )
        })?;
        expected[player] = i32::try_from(final_life).map_err(|_| {
            RuntimeSmokeUnsupported::new(
                UnsupportedSetupCode::SetupBounds,
                "expected life does not fit i32",
            )
        })?;
    }
    Ok((initial, expected))
}

fn compile_library_reserve(
    effects: &[EffectProgram],
) -> Result<[u32; PLAYER_COUNT], RuntimeSmokeUnsupported> {
    let mut draws = [0_u32; PLAYER_COUNT];
    let mut max_inspection = [0_u32; PLAYER_COUNT];
    for effect in effects {
        match effect {
            EffectProgram::DrawCards { players, count } => {
                for (player, selected) in smoke_player_mask(*players).into_iter().enumerate() {
                    if selected {
                        draws[player] = draws[player]
                            .checked_add(smoke_amount(*count)?)
                            .ok_or_else(|| {
                                RuntimeSmokeUnsupported::new(
                                    UnsupportedSetupCode::SetupBounds,
                                    "cumulative draw count overflowed",
                                )
                            })?;
                    }
                }
            }
            EffectProgram::Scry { players, count } => {
                for (player, selected) in smoke_player_mask(*players).into_iter().enumerate() {
                    if selected {
                        max_inspection[player] = max_inspection[player].max(smoke_amount(*count)?);
                    }
                }
            }
            EffectProgram::GainLife { .. }
            | EffectProgram::LoseLife { .. }
            | EffectProgram::DiscardHands { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::MoveTargetObject { .. }
            | EffectProgram::CreateTokens { .. }
            | EffectProgram::SearchLibrary { .. }
            | EffectProgram::MoveChosenObjects { .. }
            | EffectProgram::TapChosenObjects { .. }
            | EffectProgram::ModifyPowerToughness { .. }
            | EffectProgram::GrantKeywords { .. }
            | EffectProgram::GrantTargetingRestriction { .. }
            | EffectProgram::GrantIndestructible { .. }
            | EffectProgram::AttachSourceToTarget { .. }
            | EffectProgram::AddCountersToSource { .. } => {}
        }
    }
    let mut reserve = [0_u32; PLAYER_COUNT];
    for player in 0..PLAYER_COUNT {
        reserve[player] = draws[player]
            .checked_add(max_inspection[player])
            .ok_or_else(|| {
                RuntimeSmokeUnsupported::new(
                    UnsupportedSetupCode::SetupBounds,
                    "library reserve count overflowed",
                )
            })?;
    }
    Ok(reserve)
}

const fn smoke_player_mask(binding: PlayerBinding) -> [bool; PLAYER_COUNT] {
    match binding {
        PlayerBinding::Controller => [true, false],
        PlayerBinding::Opponents => [false, true],
        PlayerBinding::AllPlayers => [true, true],
        PlayerBinding::Target(_)
        | PlayerBinding::ControllerOfTargetObject(_)
        | PlayerBinding::ControllerOfTargetStack(_) => [false, true],
    }
}

fn smoke_bound_players(
    binding: PlayerBinding,
    caster: PlayerId,
    opponent: PlayerId,
) -> Vec<PlayerId> {
    smoke_player_mask(binding)
        .into_iter()
        .zip([caster, opponent])
        .filter_map(|(selected, player)| selected.then_some(player))
        .collect()
}

fn smoke_amount(amount: AmountProgram) -> Result<u32, RuntimeSmokeUnsupported> {
    match amount {
        AmountProgram::Literal(amount) => Ok(amount),
        AmountProgram::PowerOfTargetObject(_) => Ok(2),
        AmountProgram::CountPermanents(_) => Ok(2),
    }
}

struct Execution {
    state: GameState,
    production_actions: usize,
}

impl Execution {
    fn new() -> Self {
        Self {
            state: GameState::new(),
            production_actions: 0,
        }
    }

    fn dispatch(&mut self, phase: &str, action: Action) -> Result<Outcome, RuntimeSmokeFailure> {
        let outcome = apply(&mut self.state, action);
        if let Outcome::Failed(error) = &outcome {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::ProductionActionRejected,
                phase,
                format!("kernel rejected action: {error:?}"),
            ));
        }
        self.production_actions = self.production_actions.saturating_add(1);
        check_invariants(&self.state).map_err(|detail| {
            RuntimeSmokeFailure::new(RuntimeSmokeFailureCode::InvariantViolation, phase, detail)
        })?;
        Ok(outcome)
    }
}

fn execute_smoke(
    definition: &CardDefinition,
    compiled: &CompiledSmoke,
) -> Result<RuntimeSmokePass, RuntimeSmokeFailure> {
    let mut execution = Execution::new();
    execution.dispatch(
        "setup.set_seed",
        Action::SetSeed {
            seed: u64::from(stable_card_id(definition.id.as_str())),
        },
    )?;
    let caster = expect_player(execution.dispatch("setup.add_caster", Action::AddPlayer)?)?;
    let opponent = expect_player(execution.dispatch("setup.add_opponent", Action::AddPlayer)?)?;
    let players = [caster, opponent];
    let turn_order = execution.dispatch(
        "setup.turn_order",
        Action::SetTurnOrder {
            order: players.to_vec(),
        },
    )?;
    if !matches!(turn_order, Outcome::TurnOrderDecided(player) if player == caster) {
        return Err(unexpected_outcome("setup.turn_order", turn_order));
    }

    for (index, life) in compiled.initial_life.iter().copied().enumerate() {
        execution.dispatch(
            &format!("setup.life[{index}]"),
            Action::SetPlayerLife {
                player: players[index],
                life,
            },
        )?;
    }

    let base_card_id = stable_card_id(definition.id.as_str());
    for (player_index, player) in players.iter().copied().enumerate() {
        let draw_step_filler = u32::from(
            player_index == 0
                && matches!(
                    compiled.lifecycle,
                    SmokeSpellKind::Sorcery | SmokeSpellKind::Permanent | SmokeSpellKind::Land
                ),
        );
        let library_size = OPENING_HAND_SIZE
            .checked_add(draw_step_filler)
            .and_then(|size| size.checked_add(compiled.library_reserve[player_index]))
            .ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    "setup.library_size",
                    "synthesized library size overflowed",
                )
            })?;
        for offset in 0..library_size {
            let filler = execution.dispatch(
                &format!("setup.library[{player_index}][{offset}]"),
                Action::CreateObject {
                    card: CardId::new(
                        base_card_id
                            .wrapping_add(10_000)
                            .wrapping_add((player_index as u32).saturating_mul(16))
                            .wrapping_add(offset),
                    ),
                    owner: player,
                    controller: player,
                    zone: ZoneId::new(Some(player), ZoneKind::Library),
                },
            )?;
            if !matches!(filler, Outcome::ObjectCreated(_)) {
                return Err(unexpected_outcome("setup.library", filler));
            }
        }
    }
    let opening_hands = execution.dispatch("setup.draw_opening_hands", Action::DrawOpeningHands)?;
    if !matches!(opening_hands, Outcome::Applied) {
        return Err(unexpected_outcome(
            "setup.draw_opening_hands",
            opening_hands,
        ));
    }
    for (player_index, player) in players.iter().copied().enumerate() {
        let kept = execution.dispatch(
            &format!("setup.keep_opening_hand[{player_index}]"),
            Action::KeepOpeningHand {
                player,
                bottom: Vec::new(),
            },
        )?;
        if !matches!(kept, Outcome::Applied) {
            return Err(unexpected_outcome("setup.keep_opening_hand", kept));
        }
    }

    let spell = expect_object(execution.dispatch(
        "setup.create_spell",
        Action::CreateObject {
            card: CardId::new(base_card_id),
            owner: caster,
            controller: caster,
            zone: ZoneId::new(Some(caster), ZoneKind::Hand),
        },
    )?)?;
    execution.dispatch(
        "setup.spell_characteristics",
        Action::SetBaseObjectCharacteristics {
            object: spell,
            base: compiled.program.base_object(),
        },
    )?;
    if let Some(base) = compiled.program.base_creature() {
        execution.dispatch(
            "setup.spell_creature_characteristics",
            Action::SetBaseCreatureCharacteristics {
                object: spell,
                base,
            },
        )?;
    }
    synthesize_alternate_cost_conditions(&mut execution, &compiled.program, caster, base_card_id)?;
    let mut registered_triggers = Vec::with_capacity(compiled.program.triggered_abilities().len());
    for (index, ability) in compiled.program.triggered_abilities().iter().enumerate() {
        if ability.event() != TriggeredEventProgram::SourceEnters {
            continue;
        }
        let outcome = execution.dispatch(
            &format!("trigger[{index}].register"),
            Action::RegisterTriggeredAbility {
                definition: ability.bind(caster, spell),
            },
        )?;
        let Outcome::TriggerRegistered(trigger) = outcome else {
            return Err(unexpected_outcome("trigger.register", outcome));
        };
        registered_triggers.push((trigger, index));
    }

    let started = execution.dispatch(
        "cast_window.start_turn",
        Action::StartTurn {
            active_player: caster,
        },
    )?;
    if !matches!(started, Outcome::Applied) || execution.state.current_step() != Some(Step::Untap) {
        return Err(unexpected_outcome("cast_window.start_turn", started));
    }
    let upkeep = execution.dispatch("cast_window.upkeep", Action::AdvanceStep)?;
    if !matches!(upkeep, Outcome::StepAdvanced(Step::Upkeep)) {
        return Err(unexpected_outcome("cast_window.upkeep", upkeep));
    }
    if matches!(
        compiled.lifecycle,
        SmokeSpellKind::Sorcery | SmokeSpellKind::Permanent | SmokeSpellKind::Land
    ) {
        finish_empty_stack_step(&mut execution, "cast_window.finish_upkeep")?;
        if execution.state.current_step() != Some(Step::Draw) {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "cast_window.finish_upkeep",
                format!("expected Draw, found {:?}", execution.state.current_step()),
            ));
        }
        finish_empty_stack_step(&mut execution, "cast_window.finish_draw")?;
        if execution.state.current_step() != Some(Step::PrecombatMain) {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "cast_window.finish_draw",
                format!(
                    "expected PrecombatMain, found {:?}",
                    execution.state.current_step()
                ),
            ));
        }
    }

    let targets = synthesize_targets(
        &mut execution,
        compiled.program.target_requirements(),
        caster,
        opponent,
        base_card_id,
        "setup.spell_target",
    )?;
    let object_choices = synthesize_object_choices(
        &mut execution,
        compiled.program.object_choice_requirements(),
        caster,
        base_card_id,
        "setup.spell_choice",
    )?;

    if let Some((stack_kind, timing)) = compiled.lifecycle.spell_profile() {
        pay_spell_additional_costs(
            &mut execution,
            &compiled.program,
            caster,
            opponent,
            base_card_id,
        )?;
        let alternate_cost = compiled
            .program
            .alternate_costs()
            .iter()
            .copied()
            .find(|cost| cost.is_available(&execution.state, caster));
        let (cast_cost, exact_payment) = alternate_cost.map_or_else(
            || {
                (
                    compiled.program.mana_cost(),
                    compiled.program.exact_payment(),
                )
            },
            |cost| (cost.mana_cost(), cost.exact_payment()),
        );
        execution.dispatch(
            "cast.add_mana",
            Action::AddManaToPool {
                player: caster,
                mana: exact_payment,
            },
        )?;
        let payment = auto_payment_plan(exact_payment, cast_cost)
            .map_err(|error| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    "cast.payment",
                    format!("payment planner failed: {error:?}"),
                )
            })?
            .ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    "cast.payment",
                    "exact synthesized mana did not produce a payment plan",
                )
            })?;
        let cast = execution.dispatch(
            "cast.cast_spell",
            Action::CastSpell {
                player: caster,
                object: spell,
                request: CastSpellRequest::new(stack_kind, timing, cast_cost, payment)
                    .with_targets(
                        compiled.program.target_requirements().to_vec(),
                        targets.clone(),
                    ),
            },
        )?;
        let stack_entry = match cast {
            Outcome::StackEntryAdded(entry) => entry,
            other => return Err(unexpected_outcome("cast.cast_spell", other)),
        };
        assert_zone(
            &execution.state,
            spell,
            ZoneId::new(None, ZoneKind::Stack),
            "cast.stack_destination",
        )?;
        resolve_stack_entry(&mut execution, stack_entry)?;
    } else {
        let played = execution.dispatch(
            "land.play",
            Action::PlayLand {
                player: caster,
                object: spell,
            },
        )?;
        if !matches!(played, Outcome::Applied) {
            return Err(unexpected_outcome("land.play", played));
        }
    }

    let (destination, destination_name) = compiled.lifecycle.destination(caster);
    assert_zone(
        &execution.state,
        spell,
        destination,
        "resolve.spell_destination",
    )?;
    for (ability_index, ability) in compiled.program.static_abilities().iter().enumerate() {
        for (operation_index, action) in ability.bind_actions(caster, spell).into_iter().enumerate()
        {
            let outcome = execution.dispatch(
                &format!("static[{ability_index}].register[{operation_index}]"),
                action,
            )?;
            if !matches!(
                outcome,
                Outcome::ContinuousEffectRegistered(_)
                    | Outcome::CostModifierRegistered(_)
                    | Outcome::RestrictionRegistered(_)
            ) {
                return Err(unexpected_outcome("static.register", outcome));
            }
        }
    }

    let initial_hand_sizes = [
        hand_size(&execution.state, caster)?,
        hand_size(&execution.state, opponent)?,
    ];
    let mut hand_delta = [0_i64; PLAYER_COUNT];
    setup_dynamic_amount_state(
        &mut execution,
        &compiled.program,
        caster,
        opponent,
        base_card_id,
    )?;
    execute_pending_triggers(
        &mut execution,
        &compiled.program,
        &registered_triggers,
        registered_triggers.len(),
        "trigger.source_enters",
        caster,
        opponent,
        base_card_id,
        &mut hand_delta,
    )?;
    for (index, ability) in compiled.program.triggered_abilities().iter().enumerate() {
        if ability.event() == TriggeredEventProgram::SourceEnters {
            continue;
        }
        let outcome = execution.dispatch(
            &format!("trigger[{index}].register"),
            Action::RegisterTriggeredAbility {
                definition: ability.bind(caster, spell),
            },
        )?;
        let Outcome::TriggerRegistered(trigger) = outcome else {
            return Err(unexpected_outcome("trigger.register", outcome));
        };
        registered_triggers.push((trigger, index));
    }
    execute_controller_cast_triggers(
        &mut execution,
        &compiled.program,
        &registered_triggers,
        caster,
        opponent,
        base_card_id,
        &mut hand_delta,
    )?;

    let activated_effect_sources = setup_activated_effect_sources(
        &mut execution,
        &compiled.program,
        spell,
        caster,
        base_card_id,
    )?;
    let needs_creature_tap = !compiled.program.activated_abilities().is_empty()
        || compiled
            .program
            .activated_effects()
            .iter()
            .any(ActivatedEffectProgram::tap_source);
    if needs_creature_tap {
        let before_wait = [
            hand_size(&execution.state, caster)?,
            hand_size(&execution.state, opponent)?,
        ];
        prepare_creature_mana_activation(&mut execution, caster, &compiled.program)?;
        let after_wait = [
            hand_size(&execution.state, caster)?,
            hand_size(&execution.state, opponent)?,
        ];
        for player in 0..PLAYER_COUNT {
            hand_delta[player] = hand_delta[player].saturating_add(
                i64::try_from(after_wait[player]).unwrap_or(i64::MAX)
                    - i64::try_from(before_wait[player]).unwrap_or(i64::MAX),
            );
        }
    }

    setup_activation_conditions(
        &mut execution,
        &compiled.program,
        caster,
        opponent,
        base_card_id,
    )?;
    let mut registered_abilities = Vec::with_capacity(compiled.program.activated_abilities().len());
    for (index, ability) in compiled
        .program
        .activated_abilities()
        .iter()
        .copied()
        .enumerate()
    {
        let outcome = execution.dispatch(
            &format!("ability[{index}].register"),
            Action::RegisterActivatedAbility {
                definition: ability.bind(caster, spell),
            },
        )?;
        let Outcome::ActivatedAbilityRegistered(id) = outcome else {
            return Err(unexpected_outcome("ability.register", outcome));
        };
        registered_abilities.push(id);
    }
    for (index, (id, program)) in registered_abilities
        .iter()
        .copied()
        .zip(compiled.program.activated_abilities().iter().copied())
        .enumerate()
    {
        if index > 0 {
            execution.dispatch(
                &format!("ability[{index}].reset_tap"),
                Action::SetObjectTapped {
                    object: spell,
                    tapped: false,
                },
            )?;
        }
        let before = execution.state.players()[caster.index()].mana_pool();
        activate_mana_ability(&mut execution, caster, id, program.cost().mana())?;
        let actual = execution.state.players()[caster.index()].mana_pool();
        let expected = before.checked_add(program.produces()).ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "ability.assert_mana",
                "expected mana pool overflowed",
            )
        })?;
        if actual != expected {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "ability.assert_mana",
                format!("expected {expected:?}, found {actual:?}"),
            ));
        }
    }

    execute_activated_effects(
        &mut execution,
        &compiled.program,
        &activated_effect_sources,
        caster,
        opponent,
        base_card_id,
        &mut hand_delta,
    )?;

    let bindings = prepare_effect_bindings_and_hand_delta(
        &execution.state,
        compiled.program.effects(),
        compiled.program.optional_choice_count(),
        targets,
        object_choices.clone(),
        caster,
        opponent,
        &mut hand_delta,
        "effect",
    )?;
    let bound_actions = bind_program_actions(&execution.state, &compiled.program, &bindings)
        .map_err(|error| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "effect.bind",
                error.to_string(),
            )
        })?;
    dispatch_bound_actions(
        &mut execution,
        bound_actions,
        compiled.program.effects(),
        "effect",
    )?;
    assert_object_choice_destinations(
        &execution.state,
        compiled.program.effects(),
        &object_choices,
        caster,
    )?;

    execute_turn_triggers(
        &mut execution,
        &compiled.program,
        &registered_triggers,
        spell,
        caster,
        opponent,
        base_card_id,
        &mut hand_delta,
    )?;

    let final_life_totals = [
        player_life(&execution.state, caster)?,
        player_life(&execution.state, opponent)?,
    ];
    if final_life_totals != compiled.expected_life {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::LifeTotalMismatch,
            "assert.final_life",
            format!(
                "expected {:?}, found {final_life_totals:?}",
                compiled.expected_life
            ),
        ));
    }
    for (index, player) in players.iter().copied().enumerate() {
        let expected = i64::try_from(initial_hand_sizes[index])
            .unwrap_or(i64::MAX)
            .saturating_add(hand_delta[index]);
        let actual = hand_size(&execution.state, player)?;
        if i64::try_from(actual).unwrap_or(i64::MAX) != expected {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::HandSizeMismatch,
                "assert.final_hand_size",
                format!("player {index}: expected {expected}, found {actual}"),
            ));
        }
    }
    assert_zone(
        &execution.state,
        spell,
        destination,
        "assert.final_destination",
    )?;
    if execution.state.game_outcome() != GameOutcome::InProgress {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::InvariantViolation,
            "assert.game_outcome",
            format!(
                "synthesized smoke unexpectedly ended the game: {:?}",
                execution.state.game_outcome()
            ),
        ));
    }

    Ok(RuntimeSmokePass {
        capabilities: compiled.program.capabilities(),
        effect_actions: compiled.program.effects().len()
            + compiled
                .program
                .activated_effects()
                .iter()
                .map(|ability| ability.effects().len())
                .sum::<usize>()
            + compiled
                .program
                .triggered_abilities()
                .iter()
                .map(|ability| ability.effects().len())
                .sum::<usize>()
            + compiled
                .program
                .static_abilities()
                .iter()
                .map(|ability| ability.operation_count())
                .sum::<usize>(),
        production_actions: execution.production_actions,
        final_life_totals,
        final_hash: execution.state.deterministic_hash().get(),
        destination: destination_name,
    })
}

fn pay_spell_additional_costs(
    execution: &mut Execution,
    program: &CardProgram,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
) -> Result<(), RuntimeSmokeFailure> {
    for (cost_index, cost) in program.additional_costs().iter().copied().enumerate() {
        match cost {
            SpellAdditionalCostProgram::DiscardCards { count } => {
                for object_index in 0..count {
                    let object = expect_object(
                        execution.dispatch(
                            &format!("cast.additional[{cost_index}].discard.setup[{object_index}]"),
                            Action::CreateObject {
                                card: CardId::new(
                                    base_card_id
                                        .wrapping_add(600_000)
                                        .wrapping_add((cost_index as u32).saturating_mul(1_000))
                                        .wrapping_add(object_index),
                                ),
                                owner: caster,
                                controller: caster,
                                zone: ZoneId::new(Some(caster), ZoneKind::Hand),
                            },
                        )?,
                    )?;
                    execution.dispatch(
                        &format!("cast.additional[{cost_index}].discard.pay[{object_index}]"),
                        Action::MoveObject {
                            object,
                            to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
                        },
                    )?;
                    assert_zone(
                        &execution.state,
                        object,
                        ZoneId::new(Some(caster), ZoneKind::Graveyard),
                        "cast.additional.discard_destination",
                    )?;
                }
            }
            SpellAdditionalCostProgram::SacrificePermanents { count, predicate } => {
                for object_index in 0..count {
                    let object = synthesize_object_target(
                        execution,
                        TargetKind::Permanent,
                        TargetPredicate::Object(predicate),
                        caster,
                        opponent,
                        base_card_id
                            .wrapping_add(700_000)
                            .wrapping_add((cost_index as u32).saturating_mul(1_000)),
                        object_index as usize,
                        &format!("cast.additional[{cost_index}].sacrifice.setup"),
                    )?;
                    execution.dispatch(
                        &format!("cast.additional[{cost_index}].sacrifice.pay[{object_index}]"),
                        Action::MoveObject {
                            object,
                            to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
                        },
                    )?;
                    assert_zone(
                        &execution.state,
                        object,
                        ZoneId::new(Some(caster), ZoneKind::Graveyard),
                        "cast.additional.sacrifice_destination",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn synthesize_alternate_cost_conditions(
    execution: &mut Execution,
    program: &CardProgram,
    caster: PlayerId,
    base_card_id: u32,
) -> Result<(), RuntimeSmokeFailure> {
    if !program
        .alternate_costs()
        .iter()
        .any(|cost| cost.condition() == AlternateCostCondition::ControllerControlsCommander)
    {
        return Ok(());
    }
    let commander = expect_object(execution.dispatch(
        "cast.alternate_cost.commander.create",
        Action::CreateObject {
            card: CardId::new(base_card_id.wrapping_add(900_000)),
            owner: caster,
            controller: caster,
            zone: ZoneId::new(None, ZoneKind::Battlefield),
        },
    )?)?;
    execution.dispatch(
        "cast.alternate_cost.commander.characteristics",
        Action::SetBaseObjectCharacteristics {
            object: commander,
            base: BaseObjectCharacteristics::new(
                ObjectTypes::none().with_creature(),
                ObjectColors::none(),
            ),
        },
    )?;
    execution.dispatch(
        "cast.alternate_cost.commander.creature",
        Action::SetBaseCreatureCharacteristics {
            object: commander,
            base: BaseCreatureCharacteristics::new(2, 2),
        },
    )?;
    execution.dispatch(
        "cast.alternate_cost.commander.designate",
        Action::DesignateCommander {
            object: commander,
            color_identity: ObjectColors::none(),
        },
    )?;
    if program
        .alternate_costs()
        .iter()
        .copied()
        .all(|cost| !cost.is_available(&execution.state, caster))
    {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            "cast.alternate_cost.commander",
            "synthesized commander did not enable alternate cost",
        ));
    }
    Ok(())
}

fn setup_dynamic_amount_state(
    execution: &mut Execution,
    program: &CardProgram,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
) -> Result<(), RuntimeSmokeFailure> {
    let effects = program
        .effects()
        .iter()
        .chain(
            program
                .activated_effects()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .chain(
            program
                .triggered_abilities()
                .iter()
                .flat_map(|ability| ability.effects()),
        );
    let mut predicates = Vec::new();
    for effect in effects {
        let amounts = match effect {
            EffectProgram::GainLife { amount, .. } | EffectProgram::LoseLife { amount, .. } => {
                [Some(*amount), None]
            }
            EffectProgram::DrawCards { count, .. }
            | EffectProgram::Scry { count, .. }
            | EffectProgram::CreateTokens { count, .. } => [Some(*count), None],
            EffectProgram::ModifyPowerToughness {
                power, toughness, ..
            } => [Some(*power), Some(*toughness)],
            EffectProgram::DiscardHands { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::MoveTargetObject { .. }
            | EffectProgram::SearchLibrary { .. }
            | EffectProgram::MoveChosenObjects { .. }
            | EffectProgram::TapChosenObjects { .. }
            | EffectProgram::GrantKeywords { .. }
            | EffectProgram::GrantTargetingRestriction { .. }
            | EffectProgram::GrantIndestructible { .. }
            | EffectProgram::AttachSourceToTarget { .. }
            | EffectProgram::AddCountersToSource { .. } => [None, None],
        };
        for amount in amounts.into_iter().flatten() {
            if let AmountProgram::CountPermanents(predicate) = amount {
                if !predicates.contains(&predicate) {
                    predicates.push(predicate);
                }
            }
        }
    }
    for (predicate_index, predicate) in predicates.into_iter().enumerate() {
        for object_index in 0..2 {
            synthesize_object_target(
                execution,
                TargetKind::Permanent,
                TargetPredicate::Object(predicate),
                caster,
                opponent,
                base_card_id
                    .wrapping_add(800_000)
                    .wrapping_add((predicate_index as u32).saturating_mul(1_000)),
                object_index,
                &format!("setup.dynamic_amount[{predicate_index}]"),
            )?;
        }
    }
    Ok(())
}

fn setup_activation_conditions(
    execution: &mut Execution,
    program: &CardProgram,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
) -> Result<(), RuntimeSmokeFailure> {
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    for (ability_index, ability) in program.activated_abilities().iter().copied().enumerate() {
        let Some(ActivationCondition::ControllerControlsAtLeast { predicate, count }) =
            ability.condition()
        else {
            continue;
        };
        let current = execution
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    format!("ability[{ability_index}].condition"),
                    "battlefield zone is unavailable",
                )
            })?
            .iter()
            .copied()
            .filter(|object| {
                execution
                    .state
                    .object_matches_target_predicate(caster, predicate, *object)
            })
            .count();
        let required = usize::try_from(count).unwrap_or(usize::MAX);
        for object_index in current..required {
            synthesize_object_target(
                execution,
                TargetKind::Permanent,
                TargetPredicate::Object(predicate),
                caster,
                opponent,
                base_card_id
                    .wrapping_add(1_100_000)
                    .wrapping_add((ability_index as u32).saturating_mul(1_000)),
                object_index,
                &format!("ability[{ability_index}].condition.setup"),
            )?;
        }
    }
    Ok(())
}

fn execute_controller_cast_triggers(
    execution: &mut Execution,
    program: &CardProgram,
    registered: &[(TriggerId, usize)],
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
    hand_delta: &mut [i64; PLAYER_COUNT],
) -> Result<(), RuntimeSmokeFailure> {
    let predicates = program
        .triggered_abilities()
        .iter()
        .filter_map(|ability| match ability.event() {
            TriggeredEventProgram::ControllerCasts(predicate) => Some(predicate),
            _ => None,
        })
        .collect::<Vec<_>>();
    if predicates.is_empty() {
        return Ok(());
    }

    let mut types = ObjectTypes::none();
    let mut forbidden = ObjectTypes::none();
    for predicate in &predicates {
        types = types.union(predicate.required_types());
        forbidden = forbidden.union(predicate.forbidden_types());
        if predicate.required_any_types() != ObjectTypes::none() {
            types = types.union(pick_one_type(predicate.required_any_types()));
        }
    }
    if types == ObjectTypes::none() {
        for candidate in [
            ObjectTypes::none().with_instant(),
            ObjectTypes::none().with_sorcery(),
            ObjectTypes::none().with_creature(),
            ObjectTypes::none().with_artifact(),
            ObjectTypes::none().with_enchantment(),
            ObjectTypes::none().with_planeswalker(),
        ] {
            if !candidate.intersects(forbidden) {
                types = candidate;
                break;
            }
        }
    }
    let satisfies_all = predicates.iter().all(|predicate| {
        types.contains_all(predicate.required_types())
            && (predicate.required_any_types() == ObjectTypes::none()
                || types.intersects(predicate.required_any_types()))
            && !types.intersects(predicate.forbidden_types())
    });
    if !satisfies_all || types.land() {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            "trigger.controller_cast.setup",
            "cast-trigger predicates have no shared castable type",
        ));
    }
    let categories = usize::from(types.instant())
        + usize::from(types.sorcery())
        + usize::from(
            types.artifact() || types.creature() || types.enchantment() || types.planeswalker(),
        );
    if categories != 1 {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            "trigger.controller_cast.setup",
            "cast-trigger predicates require incompatible spell categories",
        ));
    }
    let (kind, timing) = if types.instant() {
        (StackObjectKind::InstantSpell, SpellTiming::Instant)
    } else if types.sorcery() {
        (StackObjectKind::SorcerySpell, SpellTiming::Sorcery)
    } else {
        (StackObjectKind::PermanentSpell, SpellTiming::Sorcery)
    };

    let object = expect_object(execution.dispatch(
        "trigger.controller_cast.create",
        Action::CreateObject {
            card: CardId::new(base_card_id.wrapping_add(900_000)),
            owner: caster,
            controller: caster,
            zone: ZoneId::new(Some(caster), ZoneKind::Hand),
        },
    )?)?;
    execution.dispatch(
        "trigger.controller_cast.characteristics",
        Action::SetBaseObjectCharacteristics {
            object,
            base: BaseObjectCharacteristics::new(types, ObjectColors::none()),
        },
    )?;
    if types.creature() {
        execution.dispatch(
            "trigger.controller_cast.creature",
            Action::SetBaseCreatureCharacteristics {
                object,
                base: BaseCreatureCharacteristics::new(2, 2),
            },
        )?;
    }
    let cost = forge_core::ManaCost::new(0, 0, 0, 0, 0, 0);
    let payment = auto_payment_plan(ManaPool::empty(), cost)
        .map_err(|error| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "trigger.controller_cast.payment",
                format!("payment planner failed: {error:?}"),
            )
        })?
        .ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "trigger.controller_cast.payment",
                "zero-cost test spell did not produce a payment plan",
            )
        })?;
    let outcome = execution.dispatch(
        "trigger.controller_cast.cast",
        Action::CastSpell {
            player: caster,
            object,
            request: CastSpellRequest::new(kind, timing, cost, payment),
        },
    )?;
    let Outcome::StackEntryAdded(test_spell) = outcome else {
        return Err(unexpected_outcome("trigger.controller_cast.cast", outcome));
    };
    execute_pending_triggers(
        execution,
        program,
        registered,
        predicates.len(),
        "trigger.controller_cast",
        caster,
        opponent,
        base_card_id.wrapping_add(910_000),
        hand_delta,
    )?;
    resolve_stack_entry(execution, test_spell)?;
    Ok(())
}

fn execute_turn_triggers(
    execution: &mut Execution,
    program: &CardProgram,
    registered: &[(TriggerId, usize)],
    source: ObjectId,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
    hand_delta: &mut [i64; PLAYER_COUNT],
) -> Result<(), RuntimeSmokeFailure> {
    let starting_turn = execution.state.turn_number();
    let upkeep_count = program
        .triggered_abilities()
        .iter()
        .filter(|ability| ability.event() == TriggeredEventProgram::ControllerUpkeep)
        .count();
    if upkeep_count != 0 {
        let before = smoke_hand_sizes(&execution.state, caster, opponent)?;
        advance_to_controller_step(
            execution,
            caster,
            Step::Upkeep,
            starting_turn,
            true,
            "trigger.controller_upkeep.wait",
        )?;
        record_hand_size_change(
            hand_delta,
            before,
            smoke_hand_sizes(&execution.state, caster, opponent)?,
        );
        execute_pending_triggers(
            execution,
            program,
            registered,
            upkeep_count,
            "trigger.controller_upkeep",
            caster,
            opponent,
            base_card_id.wrapping_add(920_000),
            hand_delta,
        )?;
    }

    let source_attack_count = program
        .triggered_abilities()
        .iter()
        .filter(|ability| ability.event() == TriggeredEventProgram::SourceAttacks)
        .count();
    let attached_attack_count = program
        .triggered_abilities()
        .iter()
        .filter(|ability| ability.event() == TriggeredEventProgram::AttachedObjectAttacks)
        .count();
    let attack_count = source_attack_count.saturating_add(attached_attack_count);
    if attack_count != 0 {
        let before = smoke_hand_sizes(&execution.state, caster, opponent)?;
        advance_to_controller_step(
            execution,
            caster,
            Step::DeclareAttackers,
            starting_turn,
            upkeep_count == 0,
            "trigger.source_attacks.wait",
        )?;
        record_hand_size_change(
            hand_delta,
            before,
            smoke_hand_sizes(&execution.state, caster, opponent)?,
        );
        let mut attacks = Vec::with_capacity(2);
        if source_attack_count != 0 {
            attacks.push(AttackDeclaration::new(source, opponent));
        }
        if attached_attack_count != 0 {
            let attached = execution
                .state
                .object(source)
                .and_then(|object| object.attached_to())
                .ok_or_else(|| {
                    RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        "trigger.attached_object_attacks.declare",
                        "trigger source has no attached object after equip activation",
                    )
                })?;
            if !attacks.iter().any(|attack| attack.attacker() == attached) {
                attacks.push(AttackDeclaration::new(attached, opponent));
            }
        }
        let outcome = execution.dispatch(
            "trigger.source_attacks.declare",
            Action::DeclareAttackers {
                player: caster,
                attacks,
            },
        )?;
        if !matches!(outcome, Outcome::Applied) {
            return Err(unexpected_outcome(
                "trigger.source_attacks.declare",
                outcome,
            ));
        }
        execute_pending_triggers(
            execution,
            program,
            registered,
            attack_count,
            "trigger.source_attacks",
            caster,
            opponent,
            base_card_id.wrapping_add(930_000),
            hand_delta,
        )?;
    }
    Ok(())
}

fn advance_to_controller_step(
    execution: &mut Execution,
    controller: PlayerId,
    target: Step,
    starting_turn: u32,
    require_future_turn: bool,
    phase: &str,
) -> Result<(), RuntimeSmokeFailure> {
    for transition in 0..80 {
        if execution.state.active_player() == Some(controller)
            && execution.state.current_step() == Some(target)
            && (!require_future_turn || execution.state.turn_number() > starting_turn)
        {
            return Ok(());
        }
        if execution.state.priority_player().is_none() {
            let outcome = execution.dispatch(
                &format!("{phase}.advance[{transition}]"),
                Action::AdvanceStep,
            )?;
            if !matches!(outcome, Outcome::StepAdvanced(_)) {
                return Err(unexpected_outcome(phase, outcome));
            }
        } else {
            finish_empty_stack_step(execution, &format!("{phase}.step[{transition}]"))?;
        }
    }
    Err(RuntimeSmokeFailure::new(
        RuntimeSmokeFailureCode::UnexpectedOutcome,
        phase,
        format!("did not reach controller step {target:?}"),
    ))
}

fn smoke_hand_sizes(
    state: &GameState,
    caster: PlayerId,
    opponent: PlayerId,
) -> Result<[usize; PLAYER_COUNT], RuntimeSmokeFailure> {
    Ok([hand_size(state, caster)?, hand_size(state, opponent)?])
}

fn record_hand_size_change(
    hand_delta: &mut [i64; PLAYER_COUNT],
    before: [usize; PLAYER_COUNT],
    after: [usize; PLAYER_COUNT],
) {
    for player in 0..PLAYER_COUNT {
        hand_delta[player] = hand_delta[player].saturating_add(
            i64::try_from(after[player]).unwrap_or(i64::MAX)
                - i64::try_from(before[player]).unwrap_or(i64::MAX),
        );
    }
}

fn execute_pending_triggers(
    execution: &mut Execution,
    program: &CardProgram,
    registered: &[(TriggerId, usize)],
    expected_count: usize,
    phase: &str,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
    hand_delta: &mut [i64; PLAYER_COUNT],
) -> Result<(), RuntimeSmokeFailure> {
    if expected_count == 0 {
        return Ok(());
    }
    let outcome = execution.dispatch(
        &format!("{phase}.put_pending"),
        Action::PutPendingTriggeredAbilitiesOnStack,
    )?;
    let Outcome::StackEntriesAdded(entries) = outcome else {
        return Err(unexpected_outcome(&format!("{phase}.put_pending"), outcome));
    };
    if entries.len() != expected_count {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            format!("{phase}.put_pending"),
            format!(
                "expected {expected_count} trigger(s), found {}",
                entries.len()
            ),
        ));
    }

    for resolution_index in 0..expected_count {
        let stack_entry = execution
            .state
            .stack_entries()
            .last()
            .ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    format!("{phase}.resolve[{resolution_index}]"),
                    "trigger stack entry is missing",
                )
            })?
            .clone();
        let trigger = stack_entry.trigger().ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                format!("{phase}.resolve[{resolution_index}]"),
                "top stack entry is not a registered trigger",
            )
        })?;
        let ability_index = registered
            .iter()
            .find_map(|(registered_trigger, ability_index)| {
                (*registered_trigger == trigger).then_some(*ability_index)
            })
            .ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    format!("{phase}.resolve[{resolution_index}]"),
                    "trigger has no compiled card-program binding",
                )
            })?;
        let ability = &program.triggered_abilities()[ability_index];
        let targets = synthesize_targets(
            execution,
            ability.target_requirements(),
            caster,
            opponent,
            base_card_id
                .wrapping_add(100_000)
                .wrapping_add((ability_index as u32).saturating_mul(1_000)),
            &format!("setup.trigger[{ability_index}].target"),
        )?;
        let object_choices = synthesize_object_choices(
            execution,
            ability.object_choice_requirements(),
            caster,
            base_card_id
                .wrapping_add(200_000)
                .wrapping_add((ability_index as u32).saturating_mul(1_000)),
            &format!("setup.trigger[{ability_index}].choice"),
        )?;
        let bindings = prepare_effect_bindings_and_hand_delta(
            &execution.state,
            ability.effects(),
            ability.optional_choice_count(),
            targets,
            object_choices.clone(),
            caster,
            opponent,
            hand_delta,
            &format!("trigger[{ability_index}].effect"),
        )?;
        let actions = bind_triggered_ability_actions(&execution.state, ability, &bindings)
            .map_err(|error| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    format!("trigger[{ability_index}].bind"),
                    error.to_string(),
                )
            })?;
        resolve_stack_entry(execution, stack_entry.id())?;
        dispatch_bound_actions(
            execution,
            actions,
            ability.effects(),
            &format!("trigger[{ability_index}].effect"),
        )?;
        assert_object_choice_destinations(
            &execution.state,
            ability.effects(),
            &object_choices,
            caster,
        )?;
    }
    Ok(())
}

fn setup_activated_effect_sources(
    execution: &mut Execution,
    program: &CardProgram,
    primary_source: ObjectId,
    caster: PlayerId,
    base_card_id: u32,
) -> Result<Vec<ObjectId>, RuntimeSmokeFailure> {
    let mut sources = Vec::with_capacity(program.activated_effects().len());
    for (index, ability) in program.activated_effects().iter().enumerate() {
        if ability.uses_source_object() {
            sources.push(primary_source);
            continue;
        }
        let source = expect_object(
            execution.dispatch(
                &format!("setup.activated[{index}].source"),
                Action::CreateObject {
                    card: CardId::new(
                        base_card_id
                            .wrapping_add(300_000)
                            .wrapping_add(index as u32),
                    ),
                    owner: caster,
                    controller: caster,
                    zone: ZoneId::new(None, ZoneKind::Battlefield),
                },
            )?,
        )?;
        execution.dispatch(
            &format!("setup.activated[{index}].characteristics"),
            Action::SetBaseObjectCharacteristics {
                object: source,
                base: program.base_object(),
            },
        )?;
        if let Some(base) = program.base_creature() {
            execution.dispatch(
                &format!("setup.activated[{index}].creature_characteristics"),
                Action::SetBaseCreatureCharacteristics {
                    object: source,
                    base,
                },
            )?;
        }
        sources.push(source);
    }
    Ok(sources)
}

#[allow(clippy::too_many_arguments)]
fn execute_activated_effects(
    execution: &mut Execution,
    program: &CardProgram,
    sources: &[ObjectId],
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
    hand_delta: &mut [i64; PLAYER_COUNT],
) -> Result<(), RuntimeSmokeFailure> {
    for (index, (ability, source)) in program
        .activated_effects()
        .iter()
        .zip(sources.iter().copied())
        .enumerate()
    {
        let phase = format!("activated[{index}]");
        if execution.state.priority_player() != Some(caster) {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                &phase,
                "ability controller does not hold priority",
            ));
        }
        if ability.timing() == ActivationTiming::Sorcery
            && (!matches!(
                execution.state.current_step(),
                Some(Step::PrecombatMain | Step::PostcombatMain)
            ) || !execution.state.stack_entries().is_empty()
                || execution.state.active_player() != Some(caster))
        {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                &phase,
                "sorcery-speed activation was not synthesized in a legal main-phase window",
            ));
        }

        let targets = synthesize_targets(
            execution,
            ability.target_requirements(),
            caster,
            opponent,
            base_card_id
                .wrapping_add(400_000)
                .wrapping_add((index as u32).saturating_mul(1_000)),
            &format!("setup.{phase}.target"),
        )?;
        let object_choices = synthesize_object_choices(
            execution,
            ability.object_choice_requirements(),
            caster,
            base_card_id
                .wrapping_add(500_000)
                .wrapping_add((index as u32).saturating_mul(1_000)),
            &format!("setup.{phase}.choice"),
        )?;
        let bindings = prepare_effect_bindings_and_hand_delta(
            &execution.state,
            ability.effects(),
            ability.optional_choice_count(),
            targets,
            object_choices.clone(),
            caster,
            opponent,
            hand_delta,
            &format!("{phase}.effect"),
        )?
        .with_source(source);
        let actions = bind_activated_effect_actions(&execution.state, ability, &bindings).map_err(
            |error| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    format!("{phase}.bind"),
                    error.to_string(),
                )
            },
        )?;

        execution.dispatch(
            &format!("{phase}.clear_mana"),
            Action::ClearManaPool { player: caster },
        )?;
        if ability.exact_payment() != ManaPool::empty() {
            execution.dispatch(
                &format!("{phase}.add_mana"),
                Action::AddManaToPool {
                    player: caster,
                    mana: ability.exact_payment(),
                },
            )?;
            let payment = auto_payment_plan(ability.exact_payment(), ability.mana_cost())
                .map_err(|error| {
                    RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("{phase}.payment"),
                        format!("activation payment planner failed: {error:?}"),
                    )
                })?
                .ok_or_else(|| {
                    RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("{phase}.payment"),
                        "exact synthesized mana did not produce an activation payment plan",
                    )
                })?;
            execution.dispatch(
                &format!("{phase}.pay_mana"),
                Action::PayMana {
                    player: caster,
                    cost: ability.mana_cost(),
                    plan: payment,
                },
            )?;
        }
        if ability.pay_life() != 0 {
            execution.dispatch(
                &format!("{phase}.pay_life"),
                Action::LoseLife {
                    player: caster,
                    amount: ability.pay_life(),
                },
            )?;
        }
        if let Some((predicate, count)) = ability.sacrifice_cost() {
            for object_index in 0..count {
                let object = synthesize_object_target(
                    execution,
                    TargetKind::Permanent,
                    TargetPredicate::Object(predicate),
                    caster,
                    opponent,
                    base_card_id
                        .wrapping_add(1_200_000)
                        .wrapping_add((index as u32).saturating_mul(1_000)),
                    object_index as usize,
                    &format!("setup.{phase}.sacrifice"),
                )?;
                execution.dispatch(
                    &format!("{phase}.sacrifice[{object_index}]"),
                    Action::MoveObject {
                        object,
                        to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
                    },
                )?;
                assert_zone(
                    &execution.state,
                    object,
                    ZoneId::new(Some(caster), ZoneKind::Graveyard),
                    &format!("{phase}.sacrifice_destination[{object_index}]"),
                )?;
            }
        }
        if ability.tap_source() {
            execution.dispatch(
                &format!("{phase}.tap_source"),
                Action::SetObjectTapped {
                    object: source,
                    tapped: true,
                },
            )?;
        }
        if ability.sacrifice_source() {
            execution.dispatch(
                &format!("{phase}.sacrifice_source"),
                Action::MoveObject {
                    object: source,
                    to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
                },
            )?;
        }
        let stack = execution.dispatch(
            &format!("{phase}.put_on_stack"),
            Action::PutAbilityOnStack {
                player: caster,
                kind: StackObjectKind::ActivatedAbility,
                hold_priority: true,
            },
        )?;
        let Outcome::StackEntryAdded(entry) = stack else {
            return Err(unexpected_outcome("activated.put_on_stack", stack));
        };
        resolve_stack_entry(execution, entry)?;
        dispatch_bound_actions(
            execution,
            actions,
            ability.effects(),
            &format!("{phase}.effect"),
        )?;
        assert_object_choice_destinations(
            &execution.state,
            ability.effects(),
            &object_choices,
            caster,
        )?;
        let expected_source_zone = if ability.sacrifice_source() {
            ZoneId::new(Some(caster), ZoneKind::Graveyard)
        } else {
            ZoneId::new(None, ZoneKind::Battlefield)
        };
        assert_zone(
            &execution.state,
            source,
            expected_source_zone,
            &format!("{phase}.source_destination"),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_effect_bindings_and_hand_delta(
    state: &GameState,
    effects: &[EffectProgram],
    optional_choice_count: usize,
    targets: Vec<TargetChoice>,
    object_choices: Vec<Vec<ObjectId>>,
    caster: PlayerId,
    opponent: PlayerId,
    hand_delta: &mut [i64; PLAYER_COUNT],
    phase: &str,
) -> Result<ExecutionBindings, RuntimeSmokeFailure> {
    let mut bindings = ExecutionBindings::new(caster, vec![opponent])
        .with_targets(targets)
        .with_object_choices(object_choices.clone())
        .with_optional_effect_choices(vec![true; optional_choice_count]);
    for (index, effect) in effects.iter().enumerate() {
        match effect {
            EffectProgram::DrawCards { players, count } => {
                let count = i64::from(smoke_amount(*count).map_err(|unsupported| {
                    RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("{phase}[{index}]"),
                        unsupported.detail().to_owned(),
                    )
                })?);
                for (player, selected) in smoke_player_mask(*players).into_iter().enumerate() {
                    if selected {
                        hand_delta[player] = hand_delta[player].saturating_add(count);
                    }
                }
            }
            EffectProgram::DiscardHands { players } => {
                for (player_index, selected) in smoke_player_mask(*players).into_iter().enumerate()
                {
                    if !selected {
                        continue;
                    }
                    let player = [caster, opponent][player_index];
                    let hand = state
                        .zone_objects(ZoneId::new(Some(player), ZoneKind::Hand))
                        .ok_or_else(|| {
                            RuntimeSmokeFailure::new(
                                RuntimeSmokeFailureCode::UnexpectedOutcome,
                                format!("{phase}[{index}]"),
                                "discard player has no hand zone",
                            )
                        })?;
                    hand_delta[player_index] = hand_delta[player_index]
                        .saturating_sub(i64::try_from(hand.len()).unwrap_or(i64::MAX));
                }
            }
            EffectProgram::Scry { players, .. } => {
                for player in smoke_bound_players(*players, caster, opponent) {
                    bindings = bindings.with_scry_bottom(index, player, Vec::new());
                }
            }
            EffectProgram::MoveChosenObjects {
                choice,
                destination,
            } if *destination == ChosenDestination::Zone(ZoneKind::Hand) => {
                hand_delta[0] = hand_delta[0].saturating_add(
                    i64::try_from(object_choices[*choice].len()).unwrap_or(i64::MAX),
                );
            }
            EffectProgram::MoveTargetObject {
                target, from, to, ..
            } => {
                let Some(TargetChoice::Object(object)) = bindings.targets().get(*target) else {
                    return Err(RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("{phase}[{index}]"),
                        "move-zone effect target is not an object",
                    ));
                };
                let owner = state
                    .object(*object)
                    .ok_or_else(|| {
                        RuntimeSmokeFailure::new(
                            RuntimeSmokeFailureCode::UnexpectedOutcome,
                            format!("{phase}[{index}]"),
                            "move-zone target is unknown",
                        )
                    })?
                    .owner();
                let player = usize::from(owner == opponent);
                if *from == ZoneKind::Hand {
                    hand_delta[player] = hand_delta[player].saturating_sub(1);
                }
                if *to == ZoneKind::Hand {
                    hand_delta[player] = hand_delta[player].saturating_add(1);
                }
            }
            EffectProgram::GainLife { .. }
            | EffectProgram::LoseLife { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::CreateTokens { .. }
            | EffectProgram::SearchLibrary { .. }
            | EffectProgram::MoveChosenObjects { .. }
            | EffectProgram::TapChosenObjects { .. }
            | EffectProgram::ModifyPowerToughness { .. }
            | EffectProgram::GrantKeywords { .. }
            | EffectProgram::GrantTargetingRestriction { .. }
            | EffectProgram::GrantIndestructible { .. }
            | EffectProgram::AttachSourceToTarget { .. }
            | EffectProgram::AddCountersToSource { .. } => {}
        }
    }
    Ok(bindings)
}

fn dispatch_bound_actions(
    execution: &mut Execution,
    actions: Vec<forge_cards::runtime::BoundAction>,
    effects: &[EffectProgram],
    phase: &str,
) -> Result<(), RuntimeSmokeFailure> {
    for bound in actions {
        let action = bound.action().clone();
        let token_expectation = match &action {
            Action::CreateToken {
                owner,
                controller,
                base_object,
                base,
                ..
            } => Some((*owner, *controller, *base_object, *base)),
            _ => None,
        };
        let outcome = execution.dispatch(&format!("{phase}[{}]", bound.effect_index()), action)?;
        if let Some((owner, controller, base_object, base_creature)) = token_expectation {
            let Outcome::ObjectCreated(object) = outcome else {
                return Err(unexpected_outcome("effect.create_token", outcome));
            };
            let record = execution.state.object(object).ok_or_else(|| {
                RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    "effect.create_token",
                    "created token object is missing",
                )
            })?;
            if !record.is_token()
                || record.owner() != owner
                || record.controller() != controller
                || record.base_object() != base_object
                || record.base_creature() != base_creature
                || execution.state.object_zone(object)
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
            {
                return Err(RuntimeSmokeFailure::new(
                    RuntimeSmokeFailureCode::UnexpectedOutcome,
                    "effect.create_token",
                    format!("created object {object:?} does not match its exact token template"),
                ));
            }
            if let Some(EffectProgram::CreateTokens {
                mana_ability: Some(program),
                ..
            }) = effects.get(bound.effect_index())
            {
                for (choice, output) in program
                    .output_choices()
                    .options()
                    .iter()
                    .copied()
                    .enumerate()
                {
                    let definition = program
                        .bind_selected(controller, object, output)
                        .ok_or_else(|| {
                            RuntimeSmokeFailure::new(
                                RuntimeSmokeFailureCode::UnexpectedOutcome,
                                "effect.create_token.ability",
                                "registered token mana output was rejected by its own program",
                            )
                        })?;
                    let registered = execution.dispatch(
                        &format!("{phase}[{}].token_ability[{choice}]", bound.effect_index()),
                        Action::RegisterActivatedAbility { definition },
                    )?;
                    if !matches!(registered, Outcome::ActivatedAbilityRegistered(_)) {
                        return Err(unexpected_outcome(
                            "effect.create_token.ability",
                            registered,
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn prepare_creature_mana_activation(
    execution: &mut Execution,
    caster: PlayerId,
    program: &CardProgram,
) -> Result<(), RuntimeSmokeFailure> {
    let Some(base_creature) = program.base_creature() else {
        return Ok(());
    };
    if base_creature.keywords().haste() {
        return Ok(());
    }
    let cast_turn = execution.state.turn_number();
    for transition in 0..40 {
        if execution.state.turn_number() > cast_turn
            && execution.state.active_player() == Some(caster)
            && execution.state.current_step() == Some(Step::PrecombatMain)
        {
            return Ok(());
        }
        if execution.state.priority_player().is_none() {
            let outcome = execution.dispatch(
                &format!("ability.wait_for_haste.advance_no_priority[{transition}]"),
                Action::AdvanceStep,
            )?;
            if !matches!(outcome, Outcome::StepAdvanced(_)) {
                return Err(unexpected_outcome(
                    "ability.wait_for_haste.advance_no_priority",
                    outcome,
                ));
            }
        } else {
            finish_empty_stack_step(
                execution,
                &format!("ability.wait_for_haste.step[{transition}]"),
            )?;
        }
    }
    Err(RuntimeSmokeFailure::new(
        RuntimeSmokeFailureCode::UnexpectedOutcome,
        "ability.wait_for_haste",
        "did not reach the controller's next precombat main phase",
    ))
}

fn assert_object_choice_destinations(
    state: &GameState,
    effects: &[EffectProgram],
    object_choices: &[Vec<ObjectId>],
    caster: PlayerId,
) -> Result<(), RuntimeSmokeFailure> {
    for (effect_index, effect) in effects.iter().enumerate() {
        if let EffectProgram::TapChosenObjects { choice } = effect {
            for object in &object_choices[*choice] {
                if !state
                    .object(*object)
                    .is_some_and(forge_core::ObjectRecord::tapped)
                {
                    return Err(RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("assert.choice[{effect_index}].tapped"),
                        "chosen object is not tapped",
                    ));
                }
            }
            continue;
        }
        let EffectProgram::MoveChosenObjects {
            choice,
            destination,
        } = effect
        else {
            continue;
        };
        for object in &object_choices[*choice] {
            match destination {
                ChosenDestination::Zone(kind) => {
                    let owner = state.object(*object).ok_or_else(|| {
                        RuntimeSmokeFailure::new(
                            RuntimeSmokeFailureCode::UnexpectedOutcome,
                            format!("assert.choice[{effect_index}]"),
                            "chosen object is missing",
                        )
                    })?;
                    let zone_owner = match kind {
                        ZoneKind::Hand | ZoneKind::Library | ZoneKind::Graveyard => {
                            Some(owner.owner())
                        }
                        ZoneKind::Battlefield
                        | ZoneKind::Exile
                        | ZoneKind::Stack
                        | ZoneKind::Command
                        | ZoneKind::Ceased => None,
                    };
                    assert_zone(
                        state,
                        *object,
                        ZoneId::new(zone_owner, *kind),
                        &format!("assert.choice[{effect_index}]"),
                    )?;
                }
                ChosenDestination::LibraryTop => {
                    let library = ZoneId::new(Some(caster), ZoneKind::Library);
                    assert_zone(
                        state,
                        *object,
                        library,
                        &format!("assert.choice[{effect_index}]"),
                    )?;
                    if state.zone_objects(library).and_then(<[ObjectId]>::last) != Some(object) {
                        return Err(RuntimeSmokeFailure::new(
                            RuntimeSmokeFailureCode::UnexpectedOutcome,
                            format!("assert.choice[{effect_index}]"),
                            "chosen object is not on top of the library",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn synthesize_targets(
    execution: &mut Execution,
    requirements: &[forge_core::TargetRequirement],
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
    phase: &str,
) -> Result<Vec<TargetChoice>, RuntimeSmokeFailure> {
    let mut choices = Vec::with_capacity(requirements.len());
    for (index, requirement) in requirements.iter().copied().enumerate() {
        let choice = match requirement.kind() {
            TargetKind::Player => {
                let player = match requirement.predicate() {
                    TargetPredicate::Any | TargetPredicate::Player(PlayerTargetPredicate::Any) => {
                        opponent
                    }
                    TargetPredicate::Player(PlayerTargetPredicate::You) => caster,
                    TargetPredicate::Player(PlayerTargetPredicate::Opponent) => opponent,
                    TargetPredicate::Player(PlayerTargetPredicate::Player(player)) => player,
                    TargetPredicate::Object(_) => {
                        return Err(RuntimeSmokeFailure::new(
                            RuntimeSmokeFailureCode::UnexpectedOutcome,
                            format!("{phase}[{index}]"),
                            "player target carries an object predicate",
                        ));
                    }
                };
                TargetChoice::Player(player)
            }
            TargetKind::StackEntry => {
                let (types, kind) = synthesize_stack_spell_shape(
                    requirement.predicate(),
                    &format!("{phase}[{index}]"),
                )?;
                let object = expect_object(execution.dispatch(
                    &format!("{phase}[{index}].stack_object"),
                    Action::CreateObject {
                        card: CardId::new(
                            base_card_id.wrapping_add(50_000).wrapping_add(index as u32),
                        ),
                        owner: caster,
                        controller: caster,
                        zone: ZoneId::new(Some(caster), ZoneKind::Hand),
                    },
                )?)?;
                execution.dispatch(
                    &format!("{phase}[{index}].stack_characteristics"),
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(
                            types,
                            ObjectColors::none().with_blue(),
                        ),
                    },
                )?;
                if types.creature() {
                    execution.dispatch(
                        &format!("{phase}[{index}].stack_creature"),
                        Action::SetBaseCreatureCharacteristics {
                            object,
                            base: BaseCreatureCharacteristics::new(2, 2),
                        },
                    )?;
                }
                let outcome = execution.dispatch(
                    &format!("{phase}[{index}].put_on_stack"),
                    Action::PutSpellOnStack {
                        player: caster,
                        object,
                        kind,
                        hold_priority: true,
                    },
                )?;
                let Outcome::StackEntryAdded(entry) = outcome else {
                    return Err(unexpected_outcome("setup.target.stack", outcome));
                };
                TargetChoice::StackEntry(entry)
            }
            TargetKind::Permanent
            | TargetKind::ObjectInZone(_)
            | TargetKind::ObjectInZoneKind(_) => TargetChoice::Object(synthesize_object_target(
                execution,
                requirement.kind(),
                requirement.predicate(),
                caster,
                opponent,
                base_card_id.wrapping_add(40_000).wrapping_add(index as u32),
                index,
                phase,
            )?),
        };
        choices.push(choice);
    }
    Ok(choices)
}

fn synthesize_object_choices(
    execution: &mut Execution,
    requirements: &[forge_cards::runtime::ObjectChoiceRequirement],
    caster: PlayerId,
    base_card_id: u32,
    phase: &str,
) -> Result<Vec<Vec<ObjectId>>, RuntimeSmokeFailure> {
    let mut choices = Vec::with_capacity(requirements.len());
    for (choice_index, requirement) in requirements.iter().copied().enumerate() {
        if requirement.player() != PlayerBinding::Controller
            || requirement.zone() != ZoneKind::Library
        {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                format!("{phase}[{choice_index}]"),
                "smoke only synthesizes controller library choices",
            ));
        }
        let mut types = requirement.required_types();
        if requirement.required_any_types() != ObjectTypes::none() {
            types = types.union(pick_one_type(requirement.required_any_types()));
        }
        let mut basic_land_types = requirement.required_land_types();
        if requirement.required_any_land_types() != BasicLandTypes::none() {
            basic_land_types = basic_land_types.union(pick_one_basic_land_type(
                requirement.required_any_land_types(),
            ));
        }
        if basic_land_types != BasicLandTypes::none() {
            types = types.union(ObjectTypes::none().with_land());
        }
        if types == ObjectTypes::none()
            && requirement.required_subtypes() != forge_core::ObjectSubtypes::none()
        {
            types = ObjectTypes::none().with_creature();
        }
        if types == ObjectTypes::none() || types.intersects(requirement.forbidden_types()) {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                format!("{phase}[{choice_index}]"),
                "choice predicate has no supported satisfying type",
            ));
        }
        let mut selected = Vec::with_capacity(requirement.maximum() as usize);
        for offset in 0..requirement.maximum() {
            let object = expect_object(
                execution.dispatch(
                    &format!("{phase}[{choice_index}][{offset}].create"),
                    Action::CreateObject {
                        card: CardId::new(
                            base_card_id
                                .wrapping_add(60_000)
                                .wrapping_add((choice_index as u32).saturating_mul(64))
                                .wrapping_add(offset),
                        ),
                        owner: caster,
                        controller: caster,
                        zone: ZoneId::new(Some(caster), ZoneKind::Library),
                    },
                )?,
            )?;
            execution.dispatch(
                &format!("{phase}[{choice_index}][{offset}].characteristics"),
                Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(types, ObjectColors::none())
                        .with_supertypes(requirement.required_supertypes())
                        .with_basic_land_types(basic_land_types)
                        .with_subtypes(requirement.required_subtypes()),
                },
            )?;
            if types.creature() {
                execution.dispatch(
                    &format!("{phase}[{choice_index}][{offset}].creature"),
                    Action::SetBaseCreatureCharacteristics {
                        object,
                        base: BaseCreatureCharacteristics::new(2, 2),
                    },
                )?;
            }
            selected.push(object);
        }
        choices.push(selected);
    }
    Ok(choices)
}

fn synthesize_stack_spell_shape(
    predicate: TargetPredicate,
    phase: &str,
) -> Result<(ObjectTypes, StackObjectKind), RuntimeSmokeFailure> {
    let predicate = match predicate {
        TargetPredicate::Any => None,
        TargetPredicate::Object(predicate) => Some(predicate),
        TargetPredicate::Player(_) => {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                phase,
                "stack target carries a player predicate",
            ));
        }
    };
    let forbidden = predicate
        .map(|predicate| predicate.forbidden_types())
        .unwrap_or(ObjectTypes::none());
    let mut types = predicate
        .map(|predicate| predicate.required_types())
        .unwrap_or(ObjectTypes::none());
    if let Some(required_any) = predicate.map(|predicate| predicate.required_any_types()) {
        types = types.union(pick_one_type(required_any));
    }
    if types == ObjectTypes::none() {
        for candidate in [
            ObjectTypes::none().with_instant(),
            ObjectTypes::none().with_sorcery(),
            ObjectTypes::none().with_creature(),
            ObjectTypes::none().with_artifact(),
            ObjectTypes::none().with_enchantment(),
            ObjectTypes::none().with_planeswalker(),
        ] {
            if !candidate.intersects(forbidden) {
                types = candidate;
                break;
            }
        }
    }
    if types == ObjectTypes::none() || types.intersects(forbidden) || types.land() {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            phase,
            "stack spell predicate has no supported legal synthesized type",
        ));
    }
    let kind = if types.instant() {
        StackObjectKind::InstantSpell
    } else if types.sorcery() {
        StackObjectKind::SorcerySpell
    } else {
        StackObjectKind::PermanentSpell
    };
    Ok((types, kind))
}

#[allow(clippy::too_many_arguments)]
fn synthesize_object_target(
    execution: &mut Execution,
    kind: TargetKind,
    predicate: TargetPredicate,
    caster: PlayerId,
    opponent: PlayerId,
    card: u32,
    index: usize,
    phase: &str,
) -> Result<ObjectId, RuntimeSmokeFailure> {
    let predicate = match predicate {
        TargetPredicate::Any => None,
        TargetPredicate::Object(predicate) => Some(predicate),
        TargetPredicate::Player(_) => {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                format!("{phase}[{index}]"),
                "object target carries a player predicate",
            ));
        }
    };
    let owner = predicate
        .map(|predicate| smoke_relationship(predicate.owner(), caster, opponent, false))
        .unwrap_or(opponent);
    let controller = predicate
        .map(|predicate| smoke_relationship(predicate.controller(), caster, opponent, false))
        .unwrap_or(opponent);
    let zone = match kind {
        TargetKind::Permanent => ZoneId::new(None, ZoneKind::Battlefield),
        TargetKind::ObjectInZone(zone) => zone,
        TargetKind::ObjectInZoneKind(kind) => ZoneId::new(smoke_zone_owner(kind, owner), kind),
        TargetKind::Player | TargetKind::StackEntry => unreachable!("object target kind checked"),
    };
    let object = expect_object(execution.dispatch(
        &format!("{phase}[{index}].create"),
        Action::CreateObject {
            card: CardId::new(card),
            owner,
            controller,
            zone,
        },
    )?)?;
    let mut types = predicate
        .map(|predicate| predicate.required_types())
        .unwrap_or(ObjectTypes::none());
    if let Some(any) = predicate.map(|predicate| predicate.required_any_types()) {
        types = types.union(pick_one_type(any));
    }
    if types == ObjectTypes::none() && kind == TargetKind::Permanent {
        types = ObjectTypes::none().with_artifact();
    }
    let mana_value = predicate
        .and_then(forge_core::ObjectTargetPredicate::minimum_mana_value)
        .unwrap_or(0);
    if predicate
        .and_then(forge_core::ObjectTargetPredicate::maximum_mana_value)
        .is_some_and(|maximum| mana_value > maximum)
    {
        return Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::UnexpectedOutcome,
            format!("{phase}[{index}].mana_value"),
            "object target has contradictory mana-value bounds",
        ));
    }
    execution.dispatch(
        &format!("{phase}[{index}].characteristics"),
        Action::SetBaseObjectCharacteristics {
            object,
            base: BaseObjectCharacteristics::new(types, ObjectColors::none())
                .with_subtypes(
                    predicate
                        .map(forge_core::ObjectTargetPredicate::required_subtypes)
                        .unwrap_or_else(forge_core::ObjectSubtypes::none),
                )
                .with_mana_value(mana_value),
        },
    )?;
    if types.creature() {
        execution.dispatch(
            &format!("{phase}[{index}].creature"),
            Action::SetBaseCreatureCharacteristics {
                object,
                base: BaseCreatureCharacteristics::new(2, 2),
            },
        )?;
    }
    Ok(object)
}

const fn smoke_relationship(
    relationship: TargetControllerPredicate,
    caster: PlayerId,
    opponent: PlayerId,
    prefer_caster: bool,
) -> PlayerId {
    match relationship {
        TargetControllerPredicate::Any => {
            if prefer_caster {
                caster
            } else {
                opponent
            }
        }
        TargetControllerPredicate::You => caster,
        TargetControllerPredicate::Opponent => opponent,
        TargetControllerPredicate::Player(player) => player,
    }
}

const fn smoke_zone_owner(kind: ZoneKind, owner: PlayerId) -> Option<PlayerId> {
    match kind {
        ZoneKind::Library | ZoneKind::Hand | ZoneKind::Graveyard => Some(owner),
        ZoneKind::Battlefield
        | ZoneKind::Exile
        | ZoneKind::Stack
        | ZoneKind::Command
        | ZoneKind::Ceased => None,
    }
}

const fn pick_one_type(types: ObjectTypes) -> ObjectTypes {
    if types.artifact() {
        ObjectTypes::none().with_artifact()
    } else if types.creature() {
        ObjectTypes::none().with_creature()
    } else if types.enchantment() {
        ObjectTypes::none().with_enchantment()
    } else if types.instant() {
        ObjectTypes::none().with_instant()
    } else if types.land() {
        ObjectTypes::none().with_land()
    } else if types.planeswalker() {
        ObjectTypes::none().with_planeswalker()
    } else if types.sorcery() {
        ObjectTypes::none().with_sorcery()
    } else {
        ObjectTypes::none()
    }
}

const fn pick_one_basic_land_type(types: BasicLandTypes) -> BasicLandTypes {
    if types.plains() {
        BasicLandTypes::none().with_plains()
    } else if types.island() {
        BasicLandTypes::none().with_island()
    } else if types.swamp() {
        BasicLandTypes::none().with_swamp()
    } else if types.mountain() {
        BasicLandTypes::none().with_mountain()
    } else if types.forest() {
        BasicLandTypes::none().with_forest()
    } else {
        BasicLandTypes::none()
    }
}

fn finish_empty_stack_step(
    execution: &mut Execution,
    phase: &str,
) -> Result<(), RuntimeSmokeFailure> {
    for pass in 0..PLAYER_COUNT {
        let player = execution.state.priority_player().ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                phase,
                "priority disappeared before the empty-stack pass round completed",
            )
        })?;
        let outcome = execution.dispatch(
            &format!("{phase}.pass[{pass}]"),
            Action::PassPriority { player },
        )?;
        if matches!(outcome, Outcome::Priority(PriorityOutcome::StepComplete)) {
            return Ok(());
        }
        if pass + 1 == PLAYER_COUNT {
            return Err(unexpected_outcome(phase, outcome));
        }
    }
    Err(RuntimeSmokeFailure::new(
        RuntimeSmokeFailureCode::UnexpectedOutcome,
        phase,
        "empty-stack priority round ended without completing the step",
    ))
}

fn resolve_stack_entry(
    execution: &mut Execution,
    expected: StackEntryId,
) -> Result<(), RuntimeSmokeFailure> {
    for pass in 0..PLAYER_COUNT {
        let player = execution.state.priority_player().ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "resolve.priority",
                "priority disappeared before spell resolution",
            )
        })?;
        let outcome = execution.dispatch(
            &format!("resolve.pass[{pass}]"),
            Action::PassPriority { player },
        )?;
        if pass + 1 == PLAYER_COUNT
            && !matches!(outcome, Outcome::Priority(PriorityOutcome::Resolved(entry)) if entry == expected)
        {
            return Err(unexpected_outcome("resolve.priority", outcome));
        }
    }
    Ok(())
}

fn activate_mana_ability(
    execution: &mut Execution,
    player: PlayerId,
    ability: ActivatedAbilityId,
    cost: forge_core::ManaCost,
) -> Result<(), RuntimeSmokeFailure> {
    let payment = auto_payment_plan(ManaPool::empty(), cost)
        .map_err(|error| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "ability.payment",
                format!("activation payment planner failed: {error:?}"),
            )
        })?
        .ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "ability.payment",
                "zero-mana activation did not produce a payment plan",
            )
        })?;
    let outcome = execution.dispatch(
        "ability.activate",
        Action::ActivateAbility {
            player,
            ability,
            payment,
        },
    )?;
    if !matches!(outcome, Outcome::Applied) {
        return Err(unexpected_outcome("ability.activate", outcome));
    }
    Ok(())
}

fn expect_player(outcome: Outcome) -> Result<PlayerId, RuntimeSmokeFailure> {
    match outcome {
        Outcome::PlayerAdded(player) => Ok(player),
        other => Err(unexpected_outcome("setup.add_player", other)),
    }
}

fn expect_object(outcome: Outcome) -> Result<forge_core::ObjectId, RuntimeSmokeFailure> {
    match outcome {
        Outcome::ObjectCreated(object) => Ok(object),
        other => Err(unexpected_outcome("setup.create_object", other)),
    }
}

fn player_life(state: &GameState, player: PlayerId) -> Result<i32, RuntimeSmokeFailure> {
    state
        .players()
        .iter()
        .find(|candidate| candidate.id() == player)
        .map(|candidate| candidate.life())
        .ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::InvariantViolation,
                "assert.player_life",
                format!("player {player:?} is missing"),
            )
        })
}

fn hand_size(state: &GameState, player: PlayerId) -> Result<usize, RuntimeSmokeFailure> {
    state
        .zone_objects(ZoneId::new(Some(player), ZoneKind::Hand))
        .map(|objects| objects.len())
        .ok_or_else(|| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::InvariantViolation,
                "assert.hand_size",
                format!("player {player:?} hand zone is missing"),
            )
        })
}

fn assert_zone(
    state: &GameState,
    object: forge_core::ObjectId,
    expected: ZoneId,
    phase: &str,
) -> Result<(), RuntimeSmokeFailure> {
    let actual = state.object_zone(object);
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(RuntimeSmokeFailure::new(
            RuntimeSmokeFailureCode::ZoneDestinationMismatch,
            phase,
            format!("expected {expected:?}, found {actual:?}"),
        ))
    }
}

fn unexpected_outcome(phase: &str, outcome: Outcome) -> RuntimeSmokeFailure {
    RuntimeSmokeFailure::new(
        RuntimeSmokeFailureCode::UnexpectedOutcome,
        phase,
        format!("unexpected production outcome: {outcome:?}"),
    )
}

fn check_invariants(state: &GameState) -> Result<(), String> {
    state
        .validate_zone_conservation()
        .map_err(|error| format!("zone conservation failed: {error:?}"))?;
    if state.deterministic_hash() != state.deterministic_hash_streaming() {
        return Err("allocated and streaming deterministic hashes differ".to_owned());
    }
    for player in state.players() {
        if !(-1_000_000..=1_000_000).contains(&player.life()) {
            return Err(format!(
                "player {:?} life is outside sanity bounds",
                player.id()
            ));
        }
        if player.poison() > 1_000 {
            return Err(format!(
                "player {:?} poison is outside sanity bounds",
                player.id()
            ));
        }
        state
            .player_view(player.id())
            .map_err(|error| format!("player view failed: {error:?}"))?;
    }
    Ok(())
}

fn stable_card_id(oracle_id: &str) -> u32 {
    oracle_id.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        run_translated_card_runtime_smoke, RuntimeSmokeCapability, RuntimeSmokeDisposition,
        RuntimeSmokeResult, UnsupportedSetupCode,
    };

    const SUPPORTED: &str =
        include_str!("../tests/fixtures/runtime_smoke/supported_life_spell.frs");
    const DRAW: &str = include_str!("../tests/fixtures/runtime_smoke/unsupported_draw_spell.frs");
    const PERMANENT: &str =
        include_str!("../tests/fixtures/runtime_smoke/supported_permanent_spell.frs");
    const BASIC_LAND: &str =
        include_str!("../tests/fixtures/runtime_smoke/supported_basic_land.frs");
    const CREATURE_MANA_SOURCE: &str = r#"
card "Creature Mana Source" {
  id: "forge:testkit:runtime:creature-mana"
  layout: normal
  status: unverified_playable
  face "Creature Mana Source" {
    cost: "{G}"
    types: "Creature - Elf Druid"
    oracle: "{T}: Add {G}."
    power: "1"
    toughness: "1"
    keywords: []
    ability activated {
      costs: [tap_self()]
      effect: add_mana("{G}", you())
      mana_ability: true
    }
  }
}
"#;
    const ACTIVATED_EFFECT_SOURCE: &str = r#"
card "Activated Effect Source" {
  id: "forge:testkit:runtime:activated-effect"
  layout: normal
  status: unverified_playable
  face "Activated Effect Source" {
    cost: "{1}"
    types: "Artifact"
    oracle: "{1}, {T}, Sacrifice this artifact: Draw a card. Pay 2 life: You gain 1 life."
    keywords: []
    ability activated {
      costs: [mana_cost("{1}"), tap_self(), sacrifice_self()]
      effect: draw(1, you())
    }
    ability activated {
      costs: [pay_life(2)]
      effect: gain_life(1, you())
    }
  }
}
"#;
    const EVENT_TRIGGER_SOURCE: &str = r#"
card "Event Trigger Source" {
  id: "forge:testkit:runtime:event-triggers"
  layout: normal
  status: unverified_playable
  face "Event Trigger Source" {
    cost: "{2}{G}"
    types: "Creature - Scout"
    oracle: "Runtime event regression fixture."
    power: "2"
    toughness: "2"
    keywords: []
    ability triggered {
      event: event_enters(source())
      effect: draw(1, you())
    }
    ability triggered {
      event: event_cast(spells(type_is("creature")), you())
      effect: draw(1, you())
    }
    ability triggered {
      event: event_upkeep(you())
      effect: draw(1, you())
    }
    ability triggered {
      event: event_attacks(source())
      effect: draw(1, you())
    }
  }
}
"#;
    const EQUIPMENT_TRIGGER_SOURCE: &str = r#"
card "Equipment Trigger Source" {
  id: "forge:testkit:runtime:equipment-trigger"
  layout: normal
  status: unverified_playable
  face "Equipment Trigger Source" {
    cost: "{2}"
    types: "Legendary Artifact - Equipment"
    oracle: "Equipped creature gets +1/+1 and has hexproof. Whenever equipped creature attacks, search for a basic land. Equip {1}."
    keywords: [equip]
    ability activated {
      costs: [mana_cost("{1}")]
      timing: timing_sorcery()
      effect: attach(source(), target(permanents(and(type_is("creature"), controlled_by(you())))))
    }
    ability static {
      effect: continuous(equipped_object(source()), sequence(modify_pt(any(), 1, 1), grant_keyword(any(), "hexproof")))
    }
    ability triggered {
      event: event_attacks(equipped_object(source()))
      effect: choose_up_to(1, sequence(search_library(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library")))), "battlefield", 1), tap(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))))), shuffle(you())))
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

    #[test]
    fn supported_life_spell_executes_and_reaches_owner_graveyard() {
        let definition = parse("supported_life_spell.frs", SUPPORTED);
        let report = run_translated_card_runtime_smoke(&definition);
        assert_eq!(report.disposition(), RuntimeSmokeDisposition::Passed);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::GainLife,
                RuntimeSmokeCapability::LoseLife
            ]
        );
        assert_eq!(pass.effect_actions(), 2);
        assert!(pass.production_actions() >= 16);
        assert_eq!(pass.final_life_totals(), [24, 18]);
        assert_eq!(pass.destination(), "owner_graveyard");
        assert_ne!(pass.final_hash(), 0);
    }

    #[test]
    fn draw_spell_executes_through_production_action() {
        let definition = parse("draw_spell.frs", DRAW);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(pass.capabilities(), [RuntimeSmokeCapability::DrawCards]);
    }

    #[test]
    fn ability_free_permanent_casts_and_resolves_to_battlefield() {
        let definition = parse("supported_permanent_spell.frs", PERMANENT);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [RuntimeSmokeCapability::PermanentSpell]
        );
        assert_eq!(pass.effect_actions(), 0);
        assert_eq!(pass.destination(), "battlefield");
    }

    #[test]
    fn basic_land_is_played_and_its_intrinsic_mana_ability_executes() {
        let definition = parse("supported_basic_land.frs", BASIC_LAND);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::LandPlay,
                RuntimeSmokeCapability::ManaAbility,
            ]
        );
        assert_eq!(pass.effect_actions(), 0);
        assert_eq!(pass.destination(), "battlefield");
    }

    #[test]
    fn creature_mana_source_waits_out_summoning_sickness_and_executes() {
        let definition = parse("creature_mana_source.frs", CREATURE_MANA_SOURCE);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::PermanentSpell,
                RuntimeSmokeCapability::ManaAbility,
            ]
        );
        assert!(pass.production_actions() > 50);
        assert_eq!(pass.destination(), "battlefield");
    }

    #[test]
    fn activated_effects_pay_costs_use_the_stack_and_resolve() {
        let definition = parse("activated_effect_source.frs", ACTIVATED_EFFECT_SOURCE);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::PermanentSpell,
                RuntimeSmokeCapability::ActivatedAbility,
                RuntimeSmokeCapability::DrawCards,
                RuntimeSmokeCapability::GainLife,
            ]
        );
        assert_eq!(pass.effect_actions(), 2);
        assert_eq!(pass.final_life_totals(), [19, 20]);
        assert_eq!(pass.destination(), "battlefield");
    }

    #[test]
    fn enter_cast_upkeep_and_attack_triggers_execute_once_each() {
        let definition = parse("event_trigger_source.frs", EVENT_TRIGGER_SOURCE);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(pass.effect_actions(), 4);
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::PermanentSpell,
                RuntimeSmokeCapability::DrawCards,
                RuntimeSmokeCapability::DrawCards,
                RuntimeSmokeCapability::DrawCards,
                RuntimeSmokeCapability::DrawCards,
            ]
        );
        assert_eq!(pass.destination(), "battlefield");
    }

    #[test]
    fn equipment_attaches_and_executes_live_layers_and_attack_trigger() {
        let definition = parse("equipment_trigger_source.frs", EQUIPMENT_TRIGGER_SOURCE);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(pass.effect_actions(), 7);
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::PermanentSpell,
                RuntimeSmokeCapability::ActivatedAbility,
                RuntimeSmokeCapability::ModifyCharacteristics,
                RuntimeSmokeCapability::TargetingRestriction,
                RuntimeSmokeCapability::AttachObject,
                RuntimeSmokeCapability::SearchLibrary,
                RuntimeSmokeCapability::MoveZone,
                RuntimeSmokeCapability::TapObject,
                RuntimeSmokeCapability::ShuffleLibrary,
            ]
        );
        assert_eq!(pass.destination(), "battlefield");
        assert_ne!(pass.final_hash(), 0);
    }

    #[test]
    fn heroic_intervention_executes_object_level_protection() {
        let definition = parse("heroic_intervention.frs", HEROIC_INTERVENTION);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::TargetingRestriction,
                RuntimeSmokeCapability::Indestructible,
            ]
        );
        assert!(pass.effect_actions() >= 2);
        assert_eq!(pass.destination(), "owner_graveyard");
    }

    #[test]
    fn scry_shuffle_and_draw_sequence_executes() {
        let source = DRAW.replace(
            "draw(1, you())",
            "sequence(scry(2, you()), shuffle(you()), draw(1, you()))",
        );
        let definition = parse("library_sequence.frs", &source);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::Scry,
                RuntimeSmokeCapability::ShuffleLibrary,
                RuntimeSmokeCapability::DrawCards,
            ]
        );
    }

    #[test]
    fn unsupported_effect_operation_is_reason_coded_and_not_a_pass() {
        let source = DRAW.replace("draw(1, you())", "mill(1, you())");
        let definition = parse("unsupported_mill_spell.frs", &source);
        let report = run_translated_card_runtime_smoke(&definition);
        assert!(!report.passed());
        assert_eq!(
            report.disposition(),
            RuntimeSmokeDisposition::UnsupportedSetup
        );
        let RuntimeSmokeResult::UnsupportedSetup(unsupported) = report.result() else {
            panic!("expected unsupported setup, found {:?}", report.result());
        };
        assert_eq!(unsupported.code(), UnsupportedSetupCode::EffectOperation);
        assert_eq!(unsupported.code().as_str(), "unsupported_effect_operation");
        assert!(unsupported.detail().contains("mill"));
    }

    #[test]
    fn targeted_player_life_effect_uses_synthesized_legal_target() {
        let targeted = SUPPORTED.replace("you()", "target(any())");
        let definition = parse("targeted_life_spell.frs", &targeted);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::Passed(pass) = report.result() else {
            panic!("expected pass, found {:?}", report.result());
        };
        assert_eq!(
            pass.capabilities(),
            [
                RuntimeSmokeCapability::GainLife,
                RuntimeSmokeCapability::LoseLife,
            ]
        );
        assert_eq!(pass.final_life_totals(), [20, 22]);
    }

    fn parse(path: &str, source: &str) -> forge_carddef::CardDefinition {
        match forge_cardc::parse_card_named(path, source) {
            Ok(definition) => definition,
            Err(error) => panic!("fixture did not compile: {error}"),
        }
    }
}
