//! Capability-specific runtime smoke for compiler-valid translated cards.
//!
//! The T3.5 harness drives compiled card programs through legal casting or land
//! play, targeting, production effects, activated mana abilities, zones, and
//! invariant checks. Unsupported definitions are returned as data with stable
//! reason codes; they are never counted as passing smoke.

use forge_carddef::CardDefinition;
use forge_cards::runtime::{
    bind_program_actions, compile_card_program, AmountProgram, CardProgram, CompileDiagnostic,
    CompileDiagnosticCode, EffectProgram, ExecutionBindings, PlayerBinding, ProgramKind,
};
use forge_core::{
    apply, auto_payment_plan, Action, ActivatedAbilityId, BaseCreatureCharacteristics,
    BaseObjectCharacteristics, CardId, CastSpellRequest, GameOutcome, GameState, ManaPool,
    ObjectColors, ObjectId, ObjectTypes, Outcome, PlayerId, PlayerTargetPredicate, PriorityOutcome,
    SpellTiming, StackEntryId, StackObjectKind, Step, TargetChoice, TargetControllerPredicate,
    TargetKind, TargetPredicate, ZoneId, ZoneKind,
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
    if compiled_mana_ability_needs_haste(&program) {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::AbilityShape,
            "creature mana abilities require a post-summoning-sickness smoke turn",
        ));
    }
    let (initial_life, expected_life) = compile_life_totals(program.effects())?;
    let library_reserve = compile_library_reserve(program.effects())?;
    Ok(CompiledSmoke {
        program,
        lifecycle,
        initial_life,
        expected_life,
        library_reserve,
    })
}

fn compiled_mana_ability_needs_haste(program: &CardProgram) -> bool {
    !program.activated_abilities().is_empty() && program.base_object().types().creature()
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
) -> Result<([i32; PLAYER_COUNT], [i32; PLAYER_COUNT]), RuntimeSmokeUnsupported> {
    let mut gains = [0_u64; PLAYER_COUNT];
    let mut losses = [0_u64; PLAYER_COUNT];
    for effect in effects {
        let (totals, players, amount) = match effect {
            EffectProgram::GainLife { players, amount } => {
                (&mut gains, *players, smoke_amount(*amount)?)
            }
            EffectProgram::LoseLife { players, amount } => {
                (&mut losses, *players, smoke_amount(*amount)?)
            }
            EffectProgram::DrawCards { .. }
            | EffectProgram::Scry { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::MoveTargetObject { .. }
            | EffectProgram::CreateTokens { .. } => continue,
        };
        let player = smoke_player_index(players);
        totals[player] = totals[player]
            .checked_add(u64::from(amount))
            .ok_or_else(|| {
                RuntimeSmokeUnsupported::new(
                    UnsupportedSetupCode::SetupBounds,
                    "cumulative life delta overflowed",
                )
            })?;
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
                let player = smoke_player_index(*players);
                draws[player] = draws[player]
                    .checked_add(smoke_amount(*count)?)
                    .ok_or_else(|| {
                        RuntimeSmokeUnsupported::new(
                            UnsupportedSetupCode::SetupBounds,
                            "cumulative draw count overflowed",
                        )
                    })?;
            }
            EffectProgram::Scry { players, count } => {
                let player = smoke_player_index(*players);
                max_inspection[player] = max_inspection[player].max(smoke_amount(*count)?);
            }
            EffectProgram::GainLife { .. }
            | EffectProgram::LoseLife { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::MoveTargetObject { .. }
            | EffectProgram::CreateTokens { .. } => {}
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

const fn smoke_player_index(binding: PlayerBinding) -> usize {
    match binding {
        PlayerBinding::Controller => 0,
        PlayerBinding::Opponents => 1,
        PlayerBinding::Target(_) | PlayerBinding::ControllerOfTargetObject(_) => 1,
    }
}

fn smoke_amount(amount: AmountProgram) -> Result<u32, RuntimeSmokeUnsupported> {
    match amount {
        AmountProgram::Literal(amount) => Ok(amount),
        AmountProgram::PowerOfTargetObject(_) => Ok(2),
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

    let targets = synthesize_targets(&mut execution, compiled, caster, opponent, base_card_id)?;

    if let Some((stack_kind, timing)) = compiled.lifecycle.spell_profile() {
        execution.dispatch(
            "cast.add_mana",
            Action::AddManaToPool {
                player: caster,
                mana: compiled.program.exact_payment(),
            },
        )?;
        let payment = auto_payment_plan(
            compiled.program.exact_payment(),
            compiled.program.mana_cost(),
        )
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
                request: CastSpellRequest::new(
                    stack_kind,
                    timing,
                    compiled.program.mana_cost(),
                    payment,
                )
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
    if let Some((id, program)) = registered_abilities
        .first()
        .copied()
        .zip(compiled.program.activated_abilities().first().copied())
    {
        activate_mana_ability(&mut execution, caster, id, program.cost().mana())?;
        let actual = execution.state.players()[caster.index()].mana_pool();
        if actual != program.produces() {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "ability.assert_mana",
                format!("expected {:?}, found {actual:?}", program.produces()),
            ));
        }
    }

    let initial_hand_sizes = [
        hand_size(&execution.state, caster)?,
        hand_size(&execution.state, opponent)?,
    ];
    let mut hand_delta = [0_i64; PLAYER_COUNT];
    let mut bindings = ExecutionBindings::new(caster, vec![opponent]).with_targets(targets);
    for (index, effect) in compiled.program.effects().iter().enumerate() {
        match effect {
            EffectProgram::DrawCards { players, count } => {
                let player = smoke_player_index(*players);
                let count = match count {
                    AmountProgram::Literal(count) => *count,
                    AmountProgram::PowerOfTargetObject(_) => 2,
                };
                hand_delta[player] = hand_delta[player].saturating_add(i64::from(count));
            }
            EffectProgram::Scry { players, .. } => {
                let player = match players {
                    PlayerBinding::Controller => caster,
                    PlayerBinding::Opponents => opponent,
                    PlayerBinding::Target(_) | PlayerBinding::ControllerOfTargetObject(_) => {
                        opponent
                    }
                };
                bindings = bindings.with_scry_bottom(index, player, Vec::new());
            }
            EffectProgram::GainLife { .. }
            | EffectProgram::LoseLife { .. }
            | EffectProgram::ShuffleLibrary { .. }
            | EffectProgram::DestroyPermanent { .. }
            | EffectProgram::ExileObject { .. }
            | EffectProgram::CounterStackEntry { .. }
            | EffectProgram::CreateTokens { .. } => {}
            EffectProgram::MoveTargetObject {
                target, from, to, ..
            } => {
                let Some(TargetChoice::Object(object)) = bindings.targets().get(*target) else {
                    return Err(RuntimeSmokeFailure::new(
                        RuntimeSmokeFailureCode::UnexpectedOutcome,
                        format!("effect[{index}]"),
                        "move-zone effect target is not an object",
                    ));
                };
                let owner = execution
                    .state
                    .object(*object)
                    .ok_or_else(|| {
                        RuntimeSmokeFailure::new(
                            RuntimeSmokeFailureCode::UnexpectedOutcome,
                            format!("effect[{index}]"),
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
        }
    }
    let bound_actions = bind_program_actions(&execution.state, &compiled.program, &bindings)
        .map_err(|error| {
            RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                "effect.bind",
                error.to_string(),
            )
        })?;
    for bound in bound_actions {
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
        let outcome = execution.dispatch(&format!("effect[{}]", bound.effect_index()), action)?;
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
        }
    }

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
        effect_actions: compiled.program.effects().len(),
        production_actions: execution.production_actions,
        final_life_totals,
        final_hash: execution.state.deterministic_hash().get(),
        destination: destination_name,
    })
}

fn synthesize_targets(
    execution: &mut Execution,
    compiled: &CompiledSmoke,
    caster: PlayerId,
    opponent: PlayerId,
    base_card_id: u32,
) -> Result<Vec<TargetChoice>, RuntimeSmokeFailure> {
    let mut choices = Vec::with_capacity(compiled.program.target_requirements().len());
    for (index, requirement) in compiled
        .program
        .target_requirements()
        .iter()
        .copied()
        .enumerate()
    {
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
                            format!("setup.target[{index}]"),
                            "player target carries an object predicate",
                        ));
                    }
                };
                TargetChoice::Player(player)
            }
            TargetKind::StackEntry => {
                let (types, kind) = synthesize_stack_spell_shape(
                    requirement.predicate(),
                    &format!("setup.target[{index}]"),
                )?;
                let object = expect_object(execution.dispatch(
                    &format!("setup.target[{index}].stack_object"),
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
                    &format!("setup.target[{index}].stack_characteristics"),
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
                        &format!("setup.target[{index}].stack_creature"),
                        Action::SetBaseCreatureCharacteristics {
                            object,
                            base: BaseCreatureCharacteristics::new(2, 2),
                        },
                    )?;
                }
                let outcome = execution.dispatch(
                    &format!("setup.target[{index}].put_on_stack"),
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
            )?),
        };
        choices.push(choice);
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

fn synthesize_object_target(
    execution: &mut Execution,
    kind: TargetKind,
    predicate: TargetPredicate,
    caster: PlayerId,
    opponent: PlayerId,
    card: u32,
    index: usize,
) -> Result<ObjectId, RuntimeSmokeFailure> {
    let predicate = match predicate {
        TargetPredicate::Any => None,
        TargetPredicate::Object(predicate) => Some(predicate),
        TargetPredicate::Player(_) => {
            return Err(RuntimeSmokeFailure::new(
                RuntimeSmokeFailureCode::UnexpectedOutcome,
                format!("setup.target[{index}]"),
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
        &format!("setup.target[{index}].create"),
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
    execution.dispatch(
        &format!("setup.target[{index}].characteristics"),
        Action::SetBaseObjectCharacteristics {
            object,
            base: BaseObjectCharacteristics::new(types, ObjectColors::none()),
        },
    )?;
    if types.creature() {
        execution.dispatch(
            &format!("setup.target[{index}].creature"),
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
        if pass + 1 == PLAYER_COUNT
            && !matches!(outcome, Outcome::Priority(PriorityOutcome::StepComplete))
        {
            return Err(unexpected_outcome(phase, outcome));
        }
    }
    Ok(())
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
