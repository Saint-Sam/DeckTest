//! Capability-specific runtime smoke for compiler-valid translated cards.
//!
//! This first T3.5 slice deliberately supports only ordinary instant and
//! sorcery spells with literal `gain_life`, `lose_life`, and `sequence`
//! effects addressed to `you()` or `opponent()`. Unsupported definitions are
//! returned as data with stable reason codes; they are never counted as
//! passing smoke.

use forge_carddef::{
    AbilityKind, CardClassification, CardDefinition, CardLayout, CardType, Color, Expression,
    ManaSymbol, Operation,
};
use forge_core::{
    apply, auto_payment_plan, Action, CardId, CastSpellRequest, GameOutcome, GameState, ManaCost,
    ManaPool, Outcome, PlayerId, PriorityOutcome, SpellTiming, StackEntryId, StackObjectKind, Step,
    ZoneId, ZoneKind,
};

const PLAYER_COUNT: usize = 2;
const DEFAULT_LIFE: u64 = 20;
const LIFE_SANITY_LIMIT: u64 = 1_000_000;
const MAX_EFFECT_ACTIONS: usize = 64;
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

/// Capability executed by this first runtime-smoke slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSmokeCapability {
    /// Life gain lowered to [`Action::GainLife`].
    GainLife,
    /// Life loss lowered to [`Action::LoseLife`].
    LoseLife,
}

impl RuntimeSmokeCapability {
    /// Returns the stable capability name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GainLife => "gain_life",
            Self::LoseLife => "lose_life",
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
    /// A life amount is dynamic, negative, or outside the production action range.
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
        "owner_graveyard"
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

#[derive(Clone, Copy)]
enum CompiledSpellKind {
    Instant,
    Sorcery,
}

impl CompiledSpellKind {
    const fn stack_kind(self) -> StackObjectKind {
        match self {
            Self::Instant => StackObjectKind::InstantSpell,
            Self::Sorcery => StackObjectKind::SorcerySpell,
        }
    }

    const fn timing(self) -> SpellTiming {
        match self {
            Self::Instant => SpellTiming::Instant,
            Self::Sorcery => SpellTiming::Sorcery,
        }
    }
}

#[derive(Clone, Copy)]
struct CompiledLifeEffect {
    capability: RuntimeSmokeCapability,
    player: usize,
    amount: u32,
}

struct CompiledSmoke {
    kind: CompiledSpellKind,
    cost: ManaCost,
    mana: ManaPool,
    effects: Vec<CompiledLifeEffect>,
    initial_life: [i32; PLAYER_COUNT],
    expected_life: [i32; PLAYER_COUNT],
}

fn compile_smoke(definition: &CardDefinition) -> Result<CompiledSmoke, RuntimeSmokeUnsupported> {
    if !matches!(
        definition.status,
        CardClassification::VerifiedPlayable | CardClassification::UnverifiedPlayable
    ) {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::CardNotPlayable,
            format!("card classification is {:?}", definition.status),
        ));
    }
    if definition.layout != CardLayout::Normal {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::CardLayout,
            format!("layout {:?} is outside the first slice", definition.layout),
        ));
    }
    let [face] = definition.faces.as_slice() else {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::FaceCount,
            format!("expected one face, found {}", definition.faces.len()),
        ));
    };
    let kind = match face.type_line.card_types.as_slice() {
        [CardType::Instant] => CompiledSpellKind::Instant,
        [CardType::Sorcery] => CompiledSpellKind::Sorcery,
        card_types => {
            return Err(RuntimeSmokeUnsupported::new(
                UnsupportedSetupCode::CardType,
                format!("unsupported top-level card types: {card_types:?}"),
            ))
        }
    };
    if !face.keywords.is_empty() {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::KeywordSemantics,
            format!("face has {} keyword(s)", face.keywords.len()),
        ));
    }
    let [ability] = face.abilities.as_slice() else {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::AbilityShape,
            format!("expected one spell ability, found {}", face.abilities.len()),
        ));
    };
    if ability.kind != AbilityKind::Spell
        || !ability.costs.is_empty()
        || ability.event.is_some()
        || ability.condition.is_some()
        || ability.timing.is_some()
        || ability.mana_ability
    {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::AbilityShape,
            "spell ability is not unconditional and cost-free",
        ));
    }

    let (cost, mana) = compile_mana_cost(&face.mana_cost.symbols)?;
    let mut effects = Vec::new();
    compile_effect(&ability.effect, &mut effects)?;
    if effects.is_empty() || effects.len() > MAX_EFFECT_ACTIONS {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::SetupBounds,
            format!(
                "effect action count {} is outside 1..={MAX_EFFECT_ACTIONS}",
                effects.len()
            ),
        ));
    }
    let (initial_life, expected_life) = compile_life_totals(&effects)?;
    Ok(CompiledSmoke {
        kind,
        cost,
        mana,
        effects,
        initial_life,
        expected_life,
    })
}

fn compile_mana_cost(
    symbols: &[ManaSymbol],
) -> Result<(ManaCost, ManaPool), RuntimeSmokeUnsupported> {
    let mut colored = [0_u32; 5];
    let mut generic = 0_u32;
    for symbol in symbols {
        match symbol {
            ManaSymbol::Color(color) => {
                let index = match color {
                    Color::White => 0,
                    Color::Blue => 1,
                    Color::Black => 2,
                    Color::Red => 3,
                    Color::Green => 4,
                };
                colored[index] = colored[index].checked_add(1).ok_or_else(|| {
                    RuntimeSmokeUnsupported::new(
                        UnsupportedSetupCode::SetupBounds,
                        "colored mana count overflowed",
                    )
                })?;
            }
            ManaSymbol::Generic(amount) => {
                generic = generic.checked_add(u32::from(*amount)).ok_or_else(|| {
                    RuntimeSmokeUnsupported::new(
                        UnsupportedSetupCode::SetupBounds,
                        "generic mana count overflowed",
                    )
                })?;
            }
            unsupported => {
                return Err(RuntimeSmokeUnsupported::new(
                    UnsupportedSetupCode::ManaSymbol,
                    format!("mana symbol {unsupported:?} is not lowered exactly"),
                ))
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
    effects: &mut Vec<CompiledLifeEffect>,
) -> Result<(), RuntimeSmokeUnsupported> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::EffectArguments,
            "effect root is not an operation call",
        ));
    };
    match operation {
        Operation::Sequence => {
            for argument in arguments {
                compile_effect(argument, effects)?;
                if effects.len() > MAX_EFFECT_ACTIONS {
                    return Err(RuntimeSmokeUnsupported::new(
                        UnsupportedSetupCode::SetupBounds,
                        format!("effect sequence exceeds {MAX_EFFECT_ACTIONS} actions"),
                    ));
                }
            }
            Ok(())
        }
        Operation::GainLife | Operation::LoseLife => {
            let [amount, player] = arguments.as_slice() else {
                return Err(RuntimeSmokeUnsupported::new(
                    UnsupportedSetupCode::EffectArguments,
                    format!("{} requires amount and player", operation.as_str()),
                ));
            };
            effects.push(CompiledLifeEffect {
                capability: if *operation == Operation::GainLife {
                    RuntimeSmokeCapability::GainLife
                } else {
                    RuntimeSmokeCapability::LoseLife
                },
                player: compile_player_selector(player)?,
                amount: compile_life_amount(amount)?,
            });
            Ok(())
        }
        unsupported => Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::EffectOperation,
            format!(
                "effect operation `{}` has no first-slice lowering",
                unsupported.as_str()
            ),
        )),
    }
}

fn compile_life_amount(expression: &Expression) -> Result<u32, RuntimeSmokeUnsupported> {
    let Expression::Integer(value) = expression else {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::EffectAmount,
            "life amount is not a literal integer",
        ));
    };
    u32::try_from(*value).map_err(|_| {
        RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::EffectAmount,
            format!("life amount {value} is outside the u32 action range"),
        )
    })
}

fn compile_player_selector(expression: &Expression) -> Result<usize, RuntimeSmokeUnsupported> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::PlayerSelector,
            "player selector is not an operation call",
        ));
    };
    if !arguments.is_empty() {
        return Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::PlayerSelector,
            format!("player selector `{}` has arguments", operation.as_str()),
        ));
    }
    match operation {
        Operation::You => Ok(0),
        Operation::Opponent => Ok(1),
        unsupported => Err(RuntimeSmokeUnsupported::new(
            UnsupportedSetupCode::PlayerSelector,
            format!(
                "player selector `{}` is not `you()` or `opponent()`",
                unsupported.as_str()
            ),
        )),
    }
}

fn compile_life_totals(
    effects: &[CompiledLifeEffect],
) -> Result<([i32; PLAYER_COUNT], [i32; PLAYER_COUNT]), RuntimeSmokeUnsupported> {
    let mut gains = [0_u64; PLAYER_COUNT];
    let mut losses = [0_u64; PLAYER_COUNT];
    for effect in effects {
        let totals = match effect.capability {
            RuntimeSmokeCapability::GainLife => &mut gains,
            RuntimeSmokeCapability::LoseLife => &mut losses,
        };
        totals[effect.player] = totals[effect.player]
            .checked_add(u64::from(effect.amount))
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
        let draw_step_filler =
            u32::from(player_index == 0 && matches!(compiled.kind, CompiledSpellKind::Sorcery));
        for offset in 0..OPENING_HAND_SIZE + draw_step_filler {
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
    if matches!(compiled.kind, CompiledSpellKind::Sorcery) {
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

    execution.dispatch(
        "cast.add_mana",
        Action::AddManaToPool {
            player: caster,
            mana: compiled.mana,
        },
    )?;
    let payment = auto_payment_plan(compiled.mana, compiled.cost)
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
                compiled.kind.stack_kind(),
                compiled.kind.timing(),
                compiled.cost,
                payment,
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

    let graveyard = ZoneId::new(Some(caster), ZoneKind::Graveyard);
    assert_zone(
        &execution.state,
        spell,
        graveyard,
        "resolve.graveyard_destination",
    )?;

    for (index, effect) in compiled.effects.iter().enumerate() {
        let action = match effect.capability {
            RuntimeSmokeCapability::GainLife => Action::GainLife {
                player: players[effect.player],
                amount: effect.amount,
            },
            RuntimeSmokeCapability::LoseLife => Action::LoseLife {
                player: players[effect.player],
                amount: effect.amount,
            },
        };
        execution.dispatch(&format!("effect[{index}]"), action)?;
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
    assert_zone(
        &execution.state,
        spell,
        graveyard,
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
        capabilities: compiled
            .effects
            .iter()
            .map(|effect| effect.capability)
            .collect(),
        effect_actions: compiled.effects.len(),
        production_actions: execution.production_actions,
        final_life_totals,
        final_hash: execution.state.deterministic_hash().get(),
    })
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
    const UNSUPPORTED: &str =
        include_str!("../tests/fixtures/runtime_smoke/unsupported_draw_spell.frs");

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
    fn unsupported_effect_operation_is_reason_coded_and_not_a_pass() {
        let definition = parse("unsupported_draw_spell.frs", UNSUPPORTED);
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
        assert!(unsupported.detail().contains("draw"));
    }

    #[test]
    fn unsupported_target_selector_is_distinct_from_supported_life_operation() {
        let targeted = SUPPORTED.replace("you()", "target(any())");
        let definition = parse("targeted_life_spell.frs", &targeted);
        let report = run_translated_card_runtime_smoke(&definition);
        let RuntimeSmokeResult::UnsupportedSetup(unsupported) = report.result() else {
            panic!("expected unsupported setup, found {:?}", report.result());
        };
        assert_eq!(unsupported.code(), UnsupportedSetupCode::PlayerSelector);
        assert_eq!(unsupported.code().as_str(), "unsupported_player_selector");
    }

    fn parse(path: &str, source: &str) -> forge_carddef::CardDefinition {
        match forge_cardc::parse_card_named(path, source) {
            Ok(definition) => definition,
            Err(error) => panic!("fixture did not compile: {error}"),
        }
    }
}
