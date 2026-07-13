//! Card-definition interpreter that emits production kernel actions.
//!
//! Compilation is complete and fail-closed before execution starts. Execution
//! never mutates [`GameState`] directly; every mutation crosses [`apply`] with
//! a typed [`Action`]. This module contains operation-family logic only and
//! must not contain branches keyed by card identity or card name.

use forge_carddef::{
    AbilityKind, CardClassification, CardDefinition, CardLayout, CardType, Color, Expression,
    ManaSymbol, Operation,
};
use forge_core::{
    apply, Action, BaseObjectCharacteristics, GameState, ManaCost, ManaPool, ObjectColors,
    ObjectId, ObjectTargetPredicate, ObjectTypes, Outcome, PlayerId, PlayerTargetPredicate,
    StackEntryId, TargetChoice, TargetControllerPredicate, TargetKind, TargetRequirement, ZoneId,
    ZoneKind,
};
use std::{collections::BTreeMap, error::Error, fmt};

const MAX_EFFECTS: usize = 64;

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
    /// Gain life.
    GainLife,
    /// Lose life.
    LoseLife,
    /// Draw cards.
    DrawCards,
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
}

impl Capability {
    /// Returns the stable capability identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GainLife => "gain_life",
            Self::LoseLife => "lose_life",
            Self::DrawCards => "draw_cards",
            Self::Scry => "scry",
            Self::ShuffleLibrary => "shuffle_library",
            Self::PermanentSpell => "permanent_spell",
            Self::DestroyPermanent => "destroy_permanent",
            Self::ExileObject => "exile_object",
            Self::CounterStackEntry => "counter_stack_entry",
            Self::MoveZone => "move_zone",
        }
    }
}

/// A player set resolved only when a program is bound to a game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerBinding {
    /// The spell or ability controller.
    Controller,
    /// Every supplied opponent of the controller.
    Opponents,
    /// One explicit player target slot.
    Target(usize),
    /// The current controller of one explicit object target slot.
    ControllerOfTargetObject(usize),
}

/// A nonnegative effect amount resolved during prebinding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmountProgram {
    /// Literal amount embedded in the definition.
    Literal(u32),
    /// Current power of one object target.
    PowerOfTargetObject(usize),
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
}

impl EffectProgram {
    const fn capability(&self) -> Capability {
        match self {
            Self::GainLife { .. } => Capability::GainLife,
            Self::LoseLife { .. } => Capability::LoseLife,
            Self::DrawCards { .. } => Capability::DrawCards,
            Self::Scry { .. } => Capability::Scry,
            Self::ShuffleLibrary { .. } => Capability::ShuffleLibrary,
            Self::DestroyPermanent { .. } => Capability::DestroyPermanent,
            Self::ExileObject { .. } => Capability::ExileObject,
            Self::CounterStackEntry { .. } => Capability::CounterStackEntry,
            Self::MoveTargetObject { .. } => Capability::MoveZone,
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
    target_requirements: Vec<TargetRequirement>,
    effects: Vec<EffectProgram>,
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

    /// Returns target slots in announcement order.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns compiled effect operations in source execution order.
    #[must_use]
    pub fn effects(&self) -> &[EffectProgram] {
        &self.effects
    }

    /// Returns all compiled capabilities in source execution order.
    #[must_use]
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut capabilities = Vec::with_capacity(self.effects.len() + 1);
        if self.kind == ProgramKind::Permanent {
            capabilities.push(Capability::PermanentSpell);
        }
        capabilities.extend(self.effects.iter().map(EffectProgram::capability));
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
    if !face.keywords.is_empty() {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::KeywordSemantics,
            "card.faces[0].keywords",
            format!(
                "{} keyword(s) require runtime lowering",
                face.keywords.len()
            ),
        ));
    }
    let (mana_cost, exact_payment) = compile_mana_cost(&face.mana_cost.symbols)?;
    let base_object = compile_base_object(&face.type_line.card_types, &face.mana_cost.symbols)?;
    let mut compiler = ProgramCompiler::default();
    match face.abilities.as_slice() {
        [] if kind == ProgramKind::Permanent => {}
        [ability]
            if ability.kind == AbilityKind::Spell
                && ability.costs.is_empty()
                && ability.event.is_none()
                && ability.condition.is_none()
                && ability.timing.is_none()
                && !ability.mana_ability =>
        {
            compile_effect(
                &ability.effect,
                "card.faces[0].abilities[0].effect",
                &mut compiler,
            )?;
        }
        abilities => {
            return Err(CompileDiagnostic::new(
                CompileDiagnosticCode::AbilityShape,
                "card.faces[0].abilities",
                format!(
                    "expected an ability-free permanent or one unconditional spell ability, found {} ability record(s)",
                    abilities.len()
                ),
            ));
        }
    }
    if compiler.effects.len() > MAX_EFFECTS {
        return Err(CompileDiagnostic::new(
            CompileDiagnosticCode::ProgramBounds,
            "card.faces[0].abilities[0].effect",
            format!(
                "compiled {} effects; maximum is {MAX_EFFECTS}",
                compiler.effects.len()
            ),
        ));
    }
    if compiler.effects.is_empty() && !matches!(kind, ProgramKind::Permanent) {
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
        target_requirements: compiler
            .targets
            .into_iter()
            .map(|target| target.requirement)
            .collect(),
        effects: compiler.effects,
    })
}

#[derive(Default)]
struct ProgramCompiler {
    effects: Vec<EffectProgram>,
    targets: Vec<CompiledTarget>,
}

struct CompiledTarget {
    selector: Expression,
    requirement: TargetRequirement,
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
    card_types: &[CardType],
    mana_symbols: &[ManaSymbol],
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
    Ok(BaseObjectCharacteristics::new(types, colors))
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
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectAmount,
            path,
            "amount is neither a literal integer nor target power",
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
        } if arguments.is_empty() => intern_target(
            compiler,
            selector,
            TargetRequirement::new(TargetKind::StackEntry),
        ),
        _ => Err(CompileDiagnostic::new(
            CompileDiagnosticCode::EffectArguments,
            path,
            "target does not contain spells()",
        )),
    }
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
        .with_forbidden_types(spec.forbidden_types);
    if let Some(controller) = spec.controller {
        predicate = predicate.with_controller(controller);
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

struct ObjectSelectorSpec {
    kind: TargetKind,
    owner: TargetControllerPredicate,
    controller: Option<TargetControllerPredicate>,
    required_types: ObjectTypes,
    required_any_types: ObjectTypes,
    forbidden_types: ObjectTypes,
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
        Operation::TypeIs => {
            let [value] = arguments.as_slice() else {
                return Err(effect_arity(path, operation, "one type string"));
            };
            spec.required_types = spec.required_types.union(compile_object_type(value, path)?);
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
            scry_bottoms: BTreeMap::new(),
        }
    }

    /// Supplies target choices in compiled announcement order.
    #[must_use]
    pub fn with_targets(mut self, targets: Vec<TargetChoice>) -> Self {
        self.targets = targets;
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
    validate_player_bindings(bindings)?;
    state
        .validate_target_choices(
            bindings.controller,
            None,
            &program.target_requirements,
            &bindings.targets,
        )
        .map_err(|error| {
            ExecutionDiagnostic::new(
                ExecutionDiagnosticCode::InvalidChoice,
                None,
                format!("kernel rejected target binding: {error:?}"),
            )
        })?;
    let mut actions = Vec::new();
    for (effect_index, effect) in program.effects.iter().enumerate() {
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
        }
    }
    Ok(actions)
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
        compile_card_program, execute_program, CompileDiagnosticCode, ExecutionBindings,
        ExecutionDiagnosticCode,
    };
    use forge_core::{
        apply, Action, BaseCreatureCharacteristics, BaseObjectCharacteristics, CardId, GameState,
        ObjectColors, ObjectTypes, Outcome, StackObjectKind, TargetChoice, ZoneId, ZoneKind,
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
}
