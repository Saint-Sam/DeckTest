#![allow(missing_docs)]
#![forbid(unsafe_code)]

//! Headless arena smoke CLI.

mod ladder;

use forge_carddef::{AbilityKind, CardDefinition, Color, Expression, ManaSymbol, Operation};
use forge_core::{
    apply, legal_actions, validate_payment_plan, Action, BaseCreatureCharacteristics, CardId,
    CastSpellRequest, ContinuousEffectDefinition, ContinuousEffectId, ContinuousEffectOperation,
    ContinuousEffectTarget, CreatureKeywords, GameOutcome, GameState, ManaCost as CoreManaCost,
    ManaPool, ObjectCharacteristics, ObjectColors, ObjectId, ObjectTypes, Outcome, PlayerId,
    SpellTiming, StackObjectKind, StateError, ZoneId, ZoneKind,
};
use std::{collections::BTreeSet, env, path::Path, process::ExitCode};

const DEFAULT_MAX_TURNS: u32 = 4;
const DEFAULT_NIGHTMARE_GAMES: u32 = 1_000;
const DEFAULT_NIGHTMARE_MAX_TURNS: u32 = 6;
const MAX_STEPS_PER_GAME: u32 = 512;
const LIBRARY_CARDS_PER_PLAYER: u32 = 32;

#[derive(Clone, Copy)]
struct SmokeConfig {
    games: u32,
    random: bool,
    max_turns: u32,
}

#[derive(Clone, Copy)]
struct NightmareConfig {
    games: u32,
    max_turns: u32,
}

#[derive(Clone, Copy)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn below(&mut self, upper: usize) -> usize {
        if upper == 0 {
            0
        } else {
            (self.next_u64() as usize) % upper
        }
    }
}

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--ladder") {
        return ladder::run(&args);
    }
    if args.iter().any(|arg| arg == "--calibrate") {
        return ladder::calibrate(&args);
    }
    if args.iter().any(|arg| arg == "--search-knee") {
        return ladder::search_knee(&args);
    }
    if args.iter().any(|arg| arg == "--nightmare-suite") {
        let games = value_after(&args, "--games")?.unwrap_or(DEFAULT_NIGHTMARE_GAMES);
        let max_turns = value_after(&args, "--max-turns")?.unwrap_or(DEFAULT_NIGHTMARE_MAX_TURNS);
        return run_nightmare_suite(NightmareConfig { games, max_turns });
    }
    let config = parse_smoke_args(&args)?;
    run_smoke(config)
}

fn print_help() {
    println!("forge-arena --smoke <games> --random [--max-turns <turns>]");
    println!("forge-arena --nightmare-suite [--games <games>] [--max-turns <turns>]");
    println!(
        "forge-arena --ladder [--games <even games/rung>] [--jobs <jobs>] [--rung <name>] [--manifest <path>] [--output <path>]"
    );
    println!(
        "forge-arena --calibrate [--tier expert|master] [--budgets 10,25,50,...] [--games <even games/budget>] [--jobs <jobs>]"
    );
    println!(
        "forge-arena --search-knee [--tier standard|expert|master] [--budgets 10,20,40,...] [--games <even games/comparison>] [--jobs <jobs>] [--skip-adaptive-ablation]"
    );
}

fn parse_smoke_args(args: &[String]) -> Result<SmokeConfig, String> {
    let games =
        value_after(args, "--smoke")?.ok_or_else(|| "expected --smoke <games>".to_string())?;
    let max_turns = value_after(args, "--max-turns")?.unwrap_or(DEFAULT_MAX_TURNS);
    Ok(SmokeConfig {
        games,
        random: args.iter().any(|arg| arg == "--random"),
        max_turns,
    })
}

fn value_after(args: &[String], flag: &str) -> Result<Option<u32>, String> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let Some(raw) = args.get(index + 1) else {
        return Err(format!("missing value after {flag}"));
    };
    raw.parse::<u32>()
        .map(Some)
        .map_err(|_| format!("invalid integer after {flag}: {raw}"))
}

fn run_smoke(config: SmokeConfig) -> Result<(), String> {
    let mut violations = 0_u32;
    for game_index in 0..config.games {
        let seed = if config.random {
            0xF0_26_E2_u64 ^ u64::from(game_index).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        } else {
            0xF0_26_E2_u64
        };
        match run_one_game(seed, config.max_turns) {
            Ok(()) => {}
            Err(error) => {
                violations = violations.saturating_add(1);
                eprintln!("game {game_index} seed {seed}: {error}");
            }
        }
    }
    if violations == 0 {
        println!(
            "PASS arena smoke: {} game(s), 0 invariant violations",
            config.games
        );
        Ok(())
    } else {
        Err(format!(
            "arena smoke found {violations} invariant violation(s) across {} game(s)",
            config.games
        ))
    }
}

fn run_nightmare_suite(config: NightmareConfig) -> Result<(), String> {
    let fixtures = nightmare_fixtures();
    let drivers = compile_nightmare_cards(&fixtures)?;
    let mut violations = 0_u32;
    for game_index in 0..config.games {
        let fixture_index = game_index as usize % fixtures.len();
        let fixture = &fixtures[fixture_index];
        let driver = &drivers[fixture_index];
        let seed = 0xF0_26_E2_10_u64 ^ u64::from(game_index).wrapping_mul(0x517C_C1B7_2722_0A95);
        match run_one_nightmare_game(seed, fixture, driver, config.max_turns) {
            Ok(()) => {}
            Err(error) => {
                violations = violations.saturating_add(1);
                eprintln!(
                    "nightmare game {game_index} fixture {} seed {seed}: {error}",
                    fixture.name
                );
            }
        }
    }
    if violations == 0 {
        println!(
            "PASS nightmare suite: {} game(s), {} compiled card-driven fixture(s), 0 invariant violations",
            config.games,
            fixtures.len()
        );
        Ok(())
    } else {
        Err(format!(
            "nightmare suite found {violations} invariant violation(s) across {} game(s)",
            config.games
        ))
    }
}

fn run_one_game(seed: u64, max_turns: u32) -> Result<(), String> {
    let mut rng = DeterministicRng::new(seed);
    let mut state = setup_game(seed)?;
    check_invariants(&state)?;
    let active = state
        .starting_player()
        .ok_or_else(|| "setup did not choose a starting player".to_string())?;
    expect_applied(
        apply(
            &mut state,
            Action::StartTurn {
                active_player: active,
            },
        ),
        "StartTurn",
    )?;
    check_invariants(&state)?;

    let mut steps = 0_u32;
    while state.game_outcome() == GameOutcome::InProgress && state.turn_number() <= max_turns {
        steps = steps.saturating_add(1);
        if steps > MAX_STEPS_PER_GAME {
            return Err(format!(
                "step limit exceeded at turn {}",
                state.turn_number()
            ));
        }
        let action = next_smoke_action(&state, &mut rng)?;
        expect_applied(apply(&mut state, action), "smoke action")?;
        check_invariants(&state)?;
    }
    Ok(())
}

fn run_one_nightmare_game(
    seed: u64,
    fixture: &NightmareFixture,
    driver: &NightmareCardDriver,
    max_turns: u32,
) -> Result<(), String> {
    let mut rng = DeterministicRng::new(seed);
    let (mut state, objects, players, driver_object) = setup_nightmare_game(seed, fixture, driver)?;
    check_invariants(&state)?;
    let active = state
        .starting_player()
        .ok_or_else(|| "setup did not choose a starting player".to_string())?;
    expect_applied(
        apply(
            &mut state,
            Action::StartTurn {
                active_player: active,
            },
        ),
        "StartTurn",
    )?;
    if state.priority_player().is_none() {
        expect_applied(
            apply(&mut state, Action::AdvanceStep),
            "nightmare advance to first priority",
        )?;
    }
    exercise_nightmare_card(&mut state, &players, driver, driver_object)?;
    register_nightmare_effects(&mut state, driver, &players, &objects)?;
    verify_nightmare_fixture(&state, fixture, &objects)?;
    check_invariants(&state)?;

    let mut steps = 0_u32;
    while state.game_outcome() == GameOutcome::InProgress && state.turn_number() <= max_turns {
        steps = steps.saturating_add(1);
        if steps > MAX_STEPS_PER_GAME {
            return Err(format!(
                "step limit exceeded at turn {}",
                state.turn_number()
            ));
        }
        let action = next_smoke_action(&state, &mut rng)?;
        expect_applied(apply(&mut state, action), "nightmare action")?;
        verify_nightmare_fixture(&state, fixture, &objects)?;
        check_invariants(&state)?;
    }
    Ok(())
}

fn setup_game(seed: u64) -> Result<GameState, String> {
    let mut state = GameState::new();
    expect_applied(apply(&mut state, Action::SetSeed { seed }), "SetSeed")?;
    let first = expect_player(apply(&mut state, Action::AddPlayer))?;
    let second = expect_player(apply(&mut state, Action::AddPlayer))?;
    seed_library(&mut state, first, 1_000)?;
    seed_library(&mut state, second, 2_000)?;
    expect_applied(
        apply(&mut state, Action::DecideTurnOrder),
        "DecideTurnOrder",
    )?;
    expect_applied(
        apply(&mut state, Action::DrawOpeningHands),
        "DrawOpeningHands",
    )?;
    let players: Vec<PlayerId> = state.players().iter().map(|player| player.id()).collect();
    for player in players {
        expect_applied(
            apply(
                &mut state,
                Action::KeepOpeningHand {
                    player,
                    bottom: Vec::new(),
                },
            ),
            "KeepOpeningHand",
        )?;
    }
    Ok(state)
}

fn setup_nightmare_game(
    seed: u64,
    fixture: &NightmareFixture,
    driver: &NightmareCardDriver,
) -> Result<(GameState, Vec<ObjectId>, Vec<PlayerId>, ObjectId), String> {
    let mut state = setup_game(seed)?;
    let players: Vec<PlayerId> = state.players().iter().map(|player| player.id()).collect();
    let mut objects = Vec::with_capacity(fixture.objects.len());
    for object in &fixture.objects {
        let owner = players
            .get(object.owner)
            .copied()
            .ok_or_else(|| format!("fixture {} has invalid owner index", fixture.name))?;
        let controller = players
            .get(object.controller)
            .copied()
            .ok_or_else(|| format!("fixture {} has invalid controller index", fixture.name))?;
        let created = expect_created_object(apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(fixture.card_base + object.card_offset),
                owner,
                controller,
                zone: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ))?;
        if let Some(base) = object.base {
            expect_applied(
                apply(
                    &mut state,
                    Action::SetBaseCreatureCharacteristics {
                        object: created,
                        base,
                    },
                ),
                "SetBaseCreatureCharacteristics",
            )?;
        }
        objects.push(created);
    }

    let active = state
        .starting_player()
        .ok_or_else(|| "nightmare setup has no starting player".to_string())?;
    let driver_object = expect_created_object(apply(
        &mut state,
        Action::CreateObject {
            card: driver.card_id,
            owner: active,
            controller: active,
            zone: ZoneId::new(Some(active), ZoneKind::Hand),
        },
    ))?;

    Ok((state, objects, players, driver_object))
}

fn register_nightmare_effects(
    state: &mut GameState,
    driver: &NightmareCardDriver,
    players: &[PlayerId],
    objects: &[ObjectId],
) -> Result<(), String> {
    let mut effects = Vec::with_capacity(driver.layer_effects.len());
    for effect in &driver.layer_effects {
        let definition = effect_definition(effect, players, objects, &effects, driver.name)?;
        effects.push(expect_continuous_effect(apply(
            state,
            Action::RegisterContinuousEffect { definition },
        ))?);
    }
    Ok(())
}

fn compile_nightmare_cards(
    fixtures: &[NightmareFixture],
) -> Result<Vec<NightmareCardDriver>, String> {
    if fixtures.len() != NIGHTMARE_CARD_SEEDS.len() {
        return Err(format!(
            "nightmare fixture/card binding mismatch: {} fixtures, {} cards",
            fixtures.len(),
            NIGHTMARE_CARD_SEEDS.len()
        ));
    }
    let database_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/layer_scenarios.carddb.bin");
    let database = forge_cards::load_card_database_file(&database_path).map_err(|error| {
        format!(
            "could not load nightmare card database {}: {error}",
            database_path.display()
        )
    })?;
    let mut drivers = Vec::with_capacity(fixtures.len());
    let mut card_ids = BTreeSet::new();
    for (fixture, seed) in fixtures.iter().zip(NIGHTMARE_CARD_SEEDS.iter()) {
        if fixture.name != seed.fixture_name {
            return Err(format!(
                "nightmare card `{}` is bound to fixture `{}` instead of `{}`",
                seed.name, seed.fixture_name, fixture.name
            ));
        }
        let parsed = forge_cardc::parse_card_named(seed.name, seed.source)
            .map_err(|error| format!("nightmare card source failed: {error}"))?;
        if parsed.id.as_str() != seed.oracle_id || parsed.name != seed.name {
            return Err(format!(
                "nightmare card source identity mismatch for {}",
                seed.name
            ));
        }
        let compiled = database.definition(seed.oracle_id).ok_or_else(|| {
            format!(
                "nightmare card {} ({}) is absent from carddb",
                seed.name, seed.oracle_id
            )
        })?;
        if compiled != &parsed {
            return Err(format!(
                "nightmare card {} differs between cardc and carddb",
                seed.name
            ));
        }
        let mut operations = BTreeSet::new();
        collect_definition_operations(compiled, &mut operations);
        for required in seed.required_operations {
            if !operations.contains(required) {
                return Err(format!(
                    "nightmare card {} is missing required operation `{required}`",
                    seed.name
                ));
            }
        }
        let face = compiled
            .faces
            .first()
            .ok_or_else(|| format!("nightmare card {} has no face", seed.name))?;
        let (cost, payment) = core_cost_and_payment(&face.mana_cost.symbols, seed.name)?;
        let layer_effects = compile_layer_effects(compiled, fixture)?;
        let card_id = CardId::new(stable_card_id(seed.oracle_id));
        if !card_ids.insert(card_id.get()) {
            return Err(format!(
                "nightmare card id collision for {} ({})",
                seed.name, seed.oracle_id
            ));
        }
        drivers.push(NightmareCardDriver {
            name: seed.name,
            card_id,
            cost,
            payment,
            layer_effects,
        });
    }
    Ok(drivers)
}

fn compile_layer_effects(
    definition: &CardDefinition,
    fixture: &NightmareFixture,
) -> Result<Vec<CompiledLayerEffect>, String> {
    let mut compiled = Vec::new();
    for face in &definition.faces {
        for ability in &face.abilities {
            if ability.kind != AbilityKind::Static
                || !ability.costs.is_empty()
                || ability.event.is_some()
                || ability.condition.is_some()
                || ability.timing.is_some()
                || ability.mana_ability
            {
                return Err(format!(
                    "layer scenario {} contains a non-static or conditional ability",
                    definition.name
                ));
            }
            let effect = compile_layer_effect(&ability.effect, fixture)?;
            for dependency in &effect.dependencies {
                if *dependency >= compiled.len() {
                    return Err(format!(
                        "layer scenario {} effect {} depends on unavailable effect {}",
                        definition.name,
                        compiled.len(),
                        dependency
                    ));
                }
            }
            compiled.push(effect);
        }
    }
    if compiled.is_empty() {
        return Err(format!(
            "layer scenario {} compiled no layer effects",
            definition.name
        ));
    }
    Ok(compiled)
}

fn compile_layer_effect(
    expression: &Expression,
    fixture: &NightmareFixture,
) -> Result<CompiledLayerEffect, String> {
    let arguments = expect_operation(expression, Operation::LayerEffect, "layer scenario root")?;
    let controller = compile_player_selector(&arguments[0])?;
    let target = compile_target_selector(&arguments[1], fixture)?;
    let operation = compile_layer_operation(&arguments[2], target, fixture)?;
    let timestamp = nonnegative_u64(&arguments[3], "layer timestamp")?;
    let dependencies = arguments[4..]
        .iter()
        .map(|argument| nonnegative_usize(argument, "layer dependency"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledLayerEffect {
        controller,
        target,
        operation,
        timestamp,
        dependencies,
    })
}

fn compile_layer_operation(
    expression: &Expression,
    target: TargetSeed,
    fixture: &NightmareFixture,
) -> Result<EffectOperationSeed, String> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err("compiled layer operation is not an operation call".to_string());
    };
    let operation_target = arguments
        .first()
        .ok_or_else(|| format!("layer operation `{}` has no target", operation.as_str()))?;
    if compile_target_selector(operation_target, fixture)? != target {
        return Err(format!(
            "layer operation `{}` target differs from its layer_effect target",
            operation.as_str()
        ));
    }

    match operation {
        Operation::Copy => Ok(EffectOperationSeed::CopyBaseCreature {
            from: compile_object_selector(&arguments[1], fixture)?,
        }),
        Operation::ChangeControl => Ok(EffectOperationSeed::ChangeController {
            controller: compile_player_selector(&arguments[1])?,
        }),
        Operation::SetTextMarker => Ok(EffectOperationSeed::SetTextMarker {
            marker: nonnegative_u32(&arguments[1], "text marker")?,
        }),
        Operation::SetType => Ok(EffectOperationSeed::SetTypes {
            types: compile_object_types(&arguments[1..])?,
        }),
        Operation::AddType => Ok(EffectOperationSeed::AddTypes {
            types: compile_object_types(&arguments[1..])?,
        }),
        Operation::RemoveType => Ok(EffectOperationSeed::RemoveTypes {
            types: compile_object_types(&arguments[1..])?,
        }),
        Operation::SetColor => Ok(EffectOperationSeed::SetColors {
            colors: compile_object_colors(&arguments[1..])?,
        }),
        Operation::GrantKeyword => Ok(EffectOperationSeed::AddKeywords {
            keywords: compile_creature_keywords(&arguments[1..])?,
        }),
        Operation::RemoveKeyword => Ok(EffectOperationSeed::RemoveKeywords {
            keywords: compile_creature_keywords(&arguments[1..])?,
        }),
        Operation::SetBasePt => Ok(EffectOperationSeed::SetBasePowerToughness {
            power: integer_i32(&arguments[1], "base power")?,
            toughness: integer_i32(&arguments[2], "base toughness")?,
        }),
        Operation::SetPt => Ok(EffectOperationSeed::SetPowerToughness {
            power: integer_i32(&arguments[1], "power")?,
            toughness: integer_i32(&arguments[2], "toughness")?,
        }),
        Operation::ModifyPt => Ok(EffectOperationSeed::ModifyPowerToughness {
            power: integer_i32(&arguments[1], "power modifier")?,
            toughness: integer_i32(&arguments[2], "toughness modifier")?,
        }),
        Operation::SwitchPt => Ok(EffectOperationSeed::SwitchPowerToughness),
        _ => Err(format!(
            "operation `{}` cannot lower to a continuous layer effect",
            operation.as_str()
        )),
    }
}

fn expect_operation<'a>(
    expression: &'a Expression,
    expected: Operation,
    context: &str,
) -> Result<&'a [Expression], String> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err(format!("{context} is not an operation call"));
    };
    if *operation != expected {
        return Err(format!(
            "{context} uses `{}` instead of `{}`",
            operation.as_str(),
            expected.as_str()
        ));
    }
    Ok(arguments)
}

fn compile_player_selector(expression: &Expression) -> Result<usize, String> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err("player binding is not a selector call".to_string());
    };
    if !arguments.is_empty() {
        return Err(format!(
            "player binding `{}` unexpectedly has arguments",
            operation.as_str()
        ));
    }
    match operation {
        Operation::You => Ok(0),
        Operation::Opponent => Ok(1),
        _ => Err(format!(
            "selector `{}` is not a closed layer-scenario player binding",
            operation.as_str()
        )),
    }
}

fn compile_target_selector(
    expression: &Expression,
    fixture: &NightmareFixture,
) -> Result<TargetSeed, String> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Err("layer target is not a selector call".to_string());
    };
    match operation {
        Operation::Remembered => Ok(TargetSeed::Object(compile_object_selector(
            expression, fixture,
        )?)),
        Operation::All => {
            let inner = arguments
                .first()
                .ok_or_else(|| "all layer target has no selector".to_string())?;
            let inner_arguments = expect_operation(inner, Operation::Permanents, "all target")?;
            if arguments.len() != 1 || !inner_arguments.is_empty() {
                return Err("all layer target must be exactly all(permanents())".to_string());
            }
            Ok(TargetSeed::AllObjects)
        }
        _ => Err(format!(
            "selector `{}` is not a closed layer-scenario target",
            operation.as_str()
        )),
    }
}

fn compile_object_selector(
    expression: &Expression,
    fixture: &NightmareFixture,
) -> Result<usize, String> {
    let arguments = expect_operation(expression, Operation::Remembered, "object binding")?;
    let [Expression::Text(binding)] = arguments else {
        return Err("object binding must contain one quoted name".to_string());
    };
    let index = binding
        .strip_prefix("fixture_object_")
        .ok_or_else(|| format!("unknown object binding `{binding}`"))?
        .parse::<usize>()
        .map_err(|_| format!("invalid object binding `{binding}`"))?;
    if index >= fixture.objects.len() {
        return Err(format!(
            "object binding `{binding}` is outside fixture {}",
            fixture.name
        ));
    }
    Ok(index)
}

fn compile_object_types(arguments: &[Expression]) -> Result<ObjectTypes, String> {
    let mut result = ObjectTypes::none();
    for value in text_arguments(arguments, "object type")? {
        result = match value {
            "artifact" => result.with_artifact(),
            "creature" => result.with_creature(),
            "enchantment" => result.with_enchantment(),
            "instant" => result.with_instant(),
            "land" => result.with_land(),
            "planeswalker" => result.with_planeswalker(),
            "sorcery" => result.with_sorcery(),
            _ => return Err(format!("unsupported layer object type `{value}`")),
        };
    }
    Ok(result)
}

fn compile_object_colors(arguments: &[Expression]) -> Result<ObjectColors, String> {
    let mut result = ObjectColors::none();
    for value in text_arguments(arguments, "object color")? {
        result = match value {
            "white" => result.with_white(),
            "blue" => result.with_blue(),
            "black" => result.with_black(),
            "red" => result.with_red(),
            "green" => result.with_green(),
            _ => return Err(format!("unsupported layer object color `{value}`")),
        };
    }
    Ok(result)
}

fn compile_creature_keywords(arguments: &[Expression]) -> Result<CreatureKeywords, String> {
    let mut result = CreatureKeywords::none();
    for value in text_arguments(arguments, "creature keyword")? {
        result = match value {
            "first_strike" => result.with_first_strike(),
            "double_strike" => result.with_double_strike(),
            "trample" => result.with_trample(),
            "deathtouch" => result.with_deathtouch(),
            "lifelink" => result.with_lifelink(),
            "flying" => result.with_flying(),
            "reach" => result.with_reach(),
            "menace" => result.with_menace(),
            "vigilance" => result.with_vigilance(),
            "haste" => result.with_haste(),
            "defender" => result.with_defender(),
            "indestructible" => result.with_indestructible(),
            "prowess" => result.with_prowess(),
            _ => return Err(format!("unsupported layer creature keyword `{value}`")),
        };
    }
    Ok(result)
}

fn text_arguments<'a>(arguments: &'a [Expression], context: &str) -> Result<Vec<&'a str>, String> {
    if arguments.is_empty() {
        return Err(format!("{context} list is empty"));
    }
    arguments
        .iter()
        .map(|argument| match argument {
            Expression::Text(value) => Ok(value.as_str()),
            _ => Err(format!("{context} must be a quoted closed literal")),
        })
        .collect()
}

fn integer_i32(expression: &Expression, context: &str) -> Result<i32, String> {
    let Expression::Integer(value) = expression else {
        return Err(format!("{context} must be an integer literal"));
    };
    i32::try_from(*value).map_err(|_| format!("{context} is outside i32 range"))
}

fn nonnegative_u64(expression: &Expression, context: &str) -> Result<u64, String> {
    let Expression::Integer(value) = expression else {
        return Err(format!("{context} must be an integer literal"));
    };
    u64::try_from(*value).map_err(|_| format!("{context} cannot be negative"))
}

fn nonnegative_u32(expression: &Expression, context: &str) -> Result<u32, String> {
    nonnegative_u64(expression, context)
        .and_then(|value| u32::try_from(value).map_err(|_| format!("{context} exceeds u32")))
}

fn nonnegative_usize(expression: &Expression, context: &str) -> Result<usize, String> {
    nonnegative_u64(expression, context)
        .and_then(|value| usize::try_from(value).map_err(|_| format!("{context} exceeds usize")))
}

fn collect_definition_operations<'a>(
    definition: &'a CardDefinition,
    operations: &mut BTreeSet<&'a str>,
) {
    for face in &definition.faces {
        for ability in &face.abilities {
            for cost in &ability.costs {
                collect_expression_operations(cost, operations);
            }
            for expression in [
                ability.event.as_ref(),
                ability.condition.as_ref(),
                ability.timing.as_ref(),
                Some(&ability.effect),
            ]
            .into_iter()
            .flatten()
            {
                collect_expression_operations(expression, operations);
            }
        }
    }
}

fn collect_expression_operations<'a>(
    expression: &'a Expression,
    operations: &mut BTreeSet<&'a str>,
) {
    match expression {
        Expression::Call {
            operation,
            arguments,
        } => {
            operations.insert(operation.as_str());
            for argument in arguments {
                collect_expression_operations(argument, operations);
            }
        }
        Expression::List(items) => {
            for item in items {
                collect_expression_operations(item, operations);
            }
        }
        Expression::Integer(_)
        | Expression::Boolean(_)
        | Expression::Text(_)
        | Expression::Symbol(_) => {}
    }
}

fn core_cost_and_payment(
    symbols: &[ManaSymbol],
    card_name: &str,
) -> Result<(CoreManaCost, ManaPool), String> {
    let mut colored = [0_u32; 5];
    let mut generic = 0_u32;
    for symbol in symbols {
        match symbol {
            ManaSymbol::Color(color) => add_color(&mut colored, *color)?,
            ManaSymbol::Generic(amount) => {
                generic = generic
                    .checked_add(u32::from(*amount))
                    .ok_or_else(|| format!("mana cost overflow for {card_name}"))?;
            }
            ManaSymbol::Colorless | ManaSymbol::Snow => {
                generic = generic
                    .checked_add(1)
                    .ok_or_else(|| format!("mana cost overflow for {card_name}"))?;
            }
            ManaSymbol::Variable(_) => {}
            ManaSymbol::Hybrid(first, _)
            | ManaSymbol::MonoHybrid(first)
            | ManaSymbol::Phyrexian(first)
            | ManaSymbol::HybridPhyrexian(first, _) => add_color(&mut colored, *first)?,
            ManaSymbol::Half(_) => {
                return Err(format!(
                    "half-mana nightmare driver is unsupported for {card_name}"
                ))
            }
        }
    }
    let cost = CoreManaCost::new(
        colored[0], colored[1], colored[2], colored[3], colored[4], generic,
    );
    let payment = ManaPool::new(
        colored[0], colored[1], colored[2], colored[3], colored[4], generic,
    );
    Ok((cost, payment))
}

fn add_color(colored: &mut [u32; 5], color: Color) -> Result<(), String> {
    let index = match color {
        Color::White => 0,
        Color::Blue => 1,
        Color::Black => 2,
        Color::Red => 3,
        Color::Green => 4,
    };
    colored[index] = colored[index]
        .checked_add(1)
        .ok_or_else(|| "colored mana cost overflow".to_string())?;
    Ok(())
}

fn stable_card_id(oracle_id: &str) -> u32 {
    oracle_id.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

fn exercise_nightmare_card(
    state: &mut GameState,
    players: &[PlayerId],
    driver: &NightmareCardDriver,
    object: ObjectId,
) -> Result<(), String> {
    let active = state
        .starting_player()
        .ok_or_else(|| "nightmare card cast has no active player".to_string())?;
    expect_applied(
        apply(
            state,
            Action::AddManaToPool {
                player: active,
                mana: driver.payment,
            },
        ),
        "nightmare AddManaToPool",
    )?;
    let payment = validate_payment_plan(driver.payment, driver.cost, driver.payment)
        .map_err(|error| format!("{} payment failed: {error:?}", driver.name))?;
    let request = CastSpellRequest::new(
        StackObjectKind::PermanentSpell,
        SpellTiming::Instant,
        driver.cost,
        payment,
    )
    .with_flash();
    match apply(
        state,
        Action::CastSpell {
            player: active,
            object,
            request,
        },
    ) {
        Outcome::StackEntryAdded(_) => {}
        Outcome::Failed(error) => {
            return Err(format_state_error(
                &format!("cast nightmare card {}", driver.name),
                error,
            ))
        }
        other => {
            return Err(format!(
                "unexpected cast outcome for {}: {other:?}",
                driver.name
            ))
        }
    }
    let responder = players
        .iter()
        .copied()
        .find(|player| *player != active)
        .ok_or_else(|| "nightmare card cast has no responder".to_string())?;
    expect_applied(
        apply(state, Action::PassPriority { player: active }),
        "nightmare cast priority pass",
    )?;
    expect_applied(
        apply(state, Action::PassPriority { player: responder }),
        "nightmare cast resolution",
    )?;
    if state.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Battlefield)) {
        return Err(format!(
            "nightmare card {} did not resolve to the battlefield",
            driver.name
        ));
    }
    Ok(())
}

fn seed_library(state: &mut GameState, player: PlayerId, first_card: u32) -> Result<(), String> {
    let library = ZoneId::new(Some(player), ZoneKind::Library);
    for offset in 0..LIBRARY_CARDS_PER_PLAYER {
        expect_object(apply(
            state,
            Action::CreateObject {
                card: CardId::new(first_card + offset),
                owner: player,
                controller: player,
                zone: library,
            },
        ))?;
    }
    Ok(())
}

fn effect_definition(
    effect: &CompiledLayerEffect,
    players: &[PlayerId],
    objects: &[ObjectId],
    effects: &[ContinuousEffectId],
    fixture_name: &str,
) -> Result<ContinuousEffectDefinition, String> {
    let controller = *players
        .get(effect.controller)
        .ok_or_else(|| format!("fixture {fixture_name} has invalid effect controller index"))?;
    let target = match effect.target {
        TargetSeed::Object(index) => {
            ContinuousEffectTarget::Object(*objects.get(index).ok_or_else(|| {
                format!("fixture {fixture_name} has invalid effect target object index")
            })?)
        }
        TargetSeed::AllObjects => ContinuousEffectTarget::AllObjects,
    };
    let operation = match effect.operation {
        EffectOperationSeed::CopyBaseCreature { from } => {
            ContinuousEffectOperation::CopyBaseCreature {
                from: *objects.get(from).ok_or_else(|| {
                    format!("fixture {fixture_name} has invalid copy-source object index")
                })?,
            }
        }
        EffectOperationSeed::ChangeController { controller } => {
            ContinuousEffectOperation::ChangeController {
                controller: *players.get(controller).ok_or_else(|| {
                    format!("fixture {fixture_name} has invalid changed-controller index")
                })?,
            }
        }
        EffectOperationSeed::SetTextMarker { marker } => {
            ContinuousEffectOperation::SetTextMarker { marker }
        }
        EffectOperationSeed::SetTypes { types } => ContinuousEffectOperation::SetTypes { types },
        EffectOperationSeed::AddTypes { types } => ContinuousEffectOperation::AddTypes { types },
        EffectOperationSeed::RemoveTypes { types } => {
            ContinuousEffectOperation::RemoveTypes { types }
        }
        EffectOperationSeed::SetColors { colors } => {
            ContinuousEffectOperation::SetColors { colors }
        }
        EffectOperationSeed::AddKeywords { keywords } => {
            ContinuousEffectOperation::AddKeywords { keywords }
        }
        EffectOperationSeed::RemoveKeywords { keywords } => {
            ContinuousEffectOperation::RemoveKeywords { keywords }
        }
        EffectOperationSeed::SetBasePowerToughness { power, toughness } => {
            ContinuousEffectOperation::SetBasePowerToughness { power, toughness }
        }
        EffectOperationSeed::SetPowerToughness { power, toughness } => {
            ContinuousEffectOperation::SetPowerToughness { power, toughness }
        }
        EffectOperationSeed::ModifyPowerToughness { power, toughness } => {
            ContinuousEffectOperation::ModifyPowerToughness { power, toughness }
        }
        EffectOperationSeed::SwitchPowerToughness => {
            ContinuousEffectOperation::SwitchPowerToughness
        }
    };
    let dependencies = effect
        .dependencies
        .iter()
        .map(|index| {
            effects.get(*index).copied().ok_or_else(|| {
                format!("fixture {fixture_name} has invalid dependency effect index")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        ContinuousEffectDefinition::new(controller, target, operation)
            .with_timestamp(effect.timestamp)
            .with_dependencies(dependencies),
    )
}

fn next_smoke_action(state: &GameState, rng: &mut DeterministicRng) -> Result<Action, String> {
    if state.priority_player().is_some() {
        let actions = legal_actions(state);
        let action = actions
            .actions()
            .get(rng.below(actions.len()))
            .cloned()
            .ok_or_else(|| "priority window had no legal action".to_string())?;
        Ok(action)
    } else {
        Ok(Action::AdvanceStep)
    }
}

fn verify_nightmare_fixture(
    state: &GameState,
    fixture: &NightmareFixture,
    objects: &[ObjectId],
) -> Result<(), String> {
    for expectation in &fixture.expectations {
        let object = *objects
            .get(expectation.object)
            .ok_or_else(|| format!("fixture {} has invalid expectation object", fixture.name))?;
        let characteristics = state
            .object_characteristics(object)
            .map_err(|error| format!("object_characteristics failed: {error:?}"))?;
        check_characteristics(fixture.name, expectation, &characteristics, state, objects)?;
    }
    Ok(())
}

fn check_characteristics(
    fixture_name: &str,
    expectation: &CharacteristicExpectation,
    characteristics: &ObjectCharacteristics,
    state: &GameState,
    objects: &[ObjectId],
) -> Result<(), String> {
    if let Some(expected_controller) = expectation.controller {
        let actual_controller = characteristics.controller();
        let expected = state
            .players()
            .get(expected_controller)
            .map(|player| player.id())
            .ok_or_else(|| format!("fixture {fixture_name} has invalid controller expectation"))?;
        if actual_controller != expected {
            return Err(format!(
                "fixture {fixture_name} object {} controller mismatch: expected {expected:?}, got {actual_controller:?}",
                expectation.object
            ));
        }
    }
    if let Some(expected) = expectation.text_marker {
        let actual = characteristics.text_marker();
        if actual != expected {
            return Err(format!(
                "fixture {fixture_name} object {} text marker mismatch: expected {expected}, got {actual}",
                expectation.object
            ));
        }
    }
    if let Some(expected) = expectation.types {
        let actual = characteristics.types();
        if actual != expected {
            return Err(format!(
                "fixture {fixture_name} object {} types mismatch: expected {expected:?}, got {actual:?}",
                expectation.object
            ));
        }
    }
    if let Some(expected) = expectation.colors {
        let actual = characteristics.colors();
        if actual != expected {
            return Err(format!(
                "fixture {fixture_name} object {} colors mismatch: expected {expected:?}, got {actual:?}",
                expectation.object
            ));
        }
    }
    if let Some(expected) = expectation.is_creature {
        let actual = characteristics.is_creature();
        if actual != expected {
            return Err(format!(
                "fixture {fixture_name} object {} creature flag mismatch: expected {expected}, got {actual}",
                expectation.object
            ));
        }
    }

    if expectation.power.is_some()
        || expectation.toughness.is_some()
        || expectation.required_keywords.is_some()
        || expectation.absent_keywords.is_some()
    {
        let creature = characteristics.creature().ok_or_else(|| {
            format!(
                "fixture {fixture_name} object {} expected creature characteristics",
                expectation.object
            )
        })?;
        if let Some(expected) = expectation.power {
            if creature.power() != expected {
                return Err(format!(
                    "fixture {fixture_name} object {} power mismatch: expected {expected}, got {}",
                    expectation.object,
                    creature.power()
                ));
            }
        }
        if let Some(expected) = expectation.toughness {
            if creature.toughness() != expected {
                return Err(format!(
                    "fixture {fixture_name} object {} toughness mismatch: expected {expected}, got {}",
                    expectation.object,
                    creature.toughness()
                ));
            }
        }
        if let Some(required) = expectation.required_keywords {
            let actual = creature.keywords();
            if !actual.contains_all(required) {
                return Err(format!(
                    "fixture {fixture_name} object {} missing keywords: required {required:?}, got {actual:?}",
                    expectation.object
                ));
            }
        }
        if let Some(absent) = expectation.absent_keywords {
            let actual = creature.keywords();
            if actual.intersects(absent) {
                return Err(format!(
                    "fixture {fixture_name} object {} had forbidden keywords: forbidden {absent:?}, got {actual:?}",
                    expectation.object
                ));
            }
        }
    }

    let object = objects
        .get(expectation.object)
        .copied()
        .ok_or_else(|| format!("fixture {fixture_name} has invalid object expectation"))?;
    state
        .object(object)
        .ok_or_else(|| format!("fixture {fixture_name} object {object:?} vanished"))?;
    Ok(())
}

fn check_invariants(state: &GameState) -> Result<(), String> {
    state
        .validate_zone_conservation()
        .map_err(|error| format!("zone conservation failed: {error:?}"))?;
    if state.deterministic_hash() != state.deterministic_hash_streaming() {
        return Err("canonical and streaming hashes diverged".to_string());
    }
    for player in state.players() {
        state
            .player_view(player.id())
            .map_err(|error| format!("player view failed: {error:?}"))?;
    }
    Ok(())
}

fn expect_player(outcome: Outcome) -> Result<PlayerId, String> {
    match outcome {
        Outcome::PlayerAdded(player) => Ok(player),
        Outcome::Failed(error) => Err(format_state_error("AddPlayer", error)),
        other => Err(format!("unexpected AddPlayer outcome: {other:?}")),
    }
}

fn expect_created_object(outcome: Outcome) -> Result<ObjectId, String> {
    match outcome {
        Outcome::ObjectCreated(object) => Ok(object),
        Outcome::Failed(error) => Err(format_state_error("CreateObject", error)),
        other => Err(format!("unexpected CreateObject outcome: {other:?}")),
    }
}

fn expect_object(outcome: Outcome) -> Result<(), String> {
    match outcome {
        Outcome::ObjectCreated(_) => Ok(()),
        Outcome::Failed(error) => Err(format_state_error("CreateObject", error)),
        other => Err(format!("unexpected CreateObject outcome: {other:?}")),
    }
}

fn expect_continuous_effect(outcome: Outcome) -> Result<ContinuousEffectId, String> {
    match outcome {
        Outcome::ContinuousEffectRegistered(effect) => Ok(effect),
        Outcome::Failed(error) => Err(format_state_error("RegisterContinuousEffect", error)),
        other => Err(format!(
            "unexpected RegisterContinuousEffect outcome: {other:?}"
        )),
    }
}

fn expect_applied(outcome: Outcome, context: &str) -> Result<(), String> {
    match outcome {
        Outcome::Applied
        | Outcome::TurnOrderDecided(_)
        | Outcome::StepAdvanced(_)
        | Outcome::Priority(_) => Ok(()),
        Outcome::Failed(error) => Err(format_state_error(context, error)),
        other => Err(format!("unexpected {context} outcome: {other:?}")),
    }
}

fn format_state_error(context: &str, error: StateError) -> String {
    format!("{context} failed: {error:?}")
}

struct NightmareCardDriver {
    name: &'static str,
    card_id: CardId,
    cost: CoreManaCost,
    payment: ManaPool,
    layer_effects: Vec<CompiledLayerEffect>,
}

#[derive(Clone, Copy)]
struct NightmareCardSeed {
    fixture_name: &'static str,
    name: &'static str,
    oracle_id: &'static str,
    source: &'static str,
    required_operations: &'static [&'static str],
}

const NIGHTMARE_CARD_SEEDS: [NightmareCardSeed; 10] = [
    NightmareCardSeed {
        fixture_name: "humility_class_global_ability_loss_and_set_pt",
        name: "Layer Scenario: Humility",
        oracle_id: "forge:scenario:layers:001",
        source: include_str!("../../../cards/integration/layers/001_humility_layers.frs"),
        required_operations: &["layer_effect", "remove_keyword", "set_pt"],
    },
    NightmareCardSeed {
        fixture_name: "opalescence_class_enchantments_animated",
        name: "Layer Scenario: Opalescence",
        oracle_id: "forge:scenario:layers:002",
        source: include_str!("../../../cards/integration/layers/002_opalescence_layers.frs"),
        required_operations: &["layer_effect", "set_type", "add_type", "set_pt"],
    },
    NightmareCardSeed {
        fixture_name: "blood_moon_class_lands_replace_complex_types",
        name: "Layer Scenario: Blood Moon",
        oracle_id: "forge:scenario:layers:003",
        source: include_str!("../../../cards/integration/layers/003_blood_moon_layers.frs"),
        required_operations: &["layer_effect", "set_type"],
    },
    NightmareCardSeed {
        fixture_name: "copy_then_full_layer_stack",
        name: "Layer Scenario: Full Stack",
        oracle_id: "forge:scenario:layers:004",
        source: include_str!("../../../cards/integration/layers/004_full_layer_stack.frs"),
        required_operations: &["layer_effect", "copy", "set_text_marker", "set_pt"],
    },
    NightmareCardSeed {
        fixture_name: "same_layer_dependency_reorders_timestamp",
        name: "Layer Scenario: Dependency Order",
        oracle_id: "forge:scenario:layers:005",
        source: include_str!("../../../cards/integration/layers/005_dependency_order.frs"),
        required_operations: &["layer_effect", "set_color"],
    },
    NightmareCardSeed {
        fixture_name: "type_removal_blocks_later_keyword_grant",
        name: "Layer Scenario: Type Removal",
        oracle_id: "forge:scenario:layers:006",
        source: include_str!("../../../cards/integration/layers/006_type_removal.frs"),
        required_operations: &["layer_effect", "remove_type", "grant_keyword"],
    },
    NightmareCardSeed {
        fixture_name: "cda_modifier_switch_stack",
        name: "Layer Scenario: CDA Modifier Switch",
        oracle_id: "forge:scenario:layers:007",
        source: include_str!("../../../cards/integration/layers/007_cda_modifier_switch.frs"),
        required_operations: &["layer_effect", "set_base_pt", "modify_pt", "switch_pt"],
    },
    NightmareCardSeed {
        fixture_name: "specific_later_color_overrides_global",
        name: "Layer Scenario: Specific Color",
        oracle_id: "forge:scenario:layers:008",
        source: include_str!("../../../cards/integration/layers/008_specific_color.frs"),
        required_operations: &["layer_effect", "set_color"],
    },
    NightmareCardSeed {
        fixture_name: "control_change_survives_random_play",
        name: "Layer Scenario: Control Change",
        oracle_id: "forge:scenario:layers:009",
        source: include_str!("../../../cards/integration/layers/009_control_change.frs"),
        required_operations: &["layer_effect", "change_control"],
    },
    NightmareCardSeed {
        fixture_name: "all_layer_dependency_and_keyword_cleanup",
        name: "Layer Scenario: All Layers",
        oracle_id: "forge:scenario:layers:010",
        source: include_str!("../../../cards/integration/layers/010_all_layers.frs"),
        required_operations: &["layer_effect", "copy", "add_type", "modify_pt"],
    },
];

struct NightmareFixture {
    name: &'static str,
    card_base: u32,
    objects: Vec<ObjectSeed>,
    expectations: Vec<CharacteristicExpectation>,
}

#[derive(Clone, Copy)]
struct ObjectSeed {
    owner: usize,
    controller: usize,
    card_offset: u32,
    base: Option<BaseCreatureCharacteristics>,
}

struct CompiledLayerEffect {
    controller: usize,
    target: TargetSeed,
    operation: EffectOperationSeed,
    timestamp: u64,
    dependencies: Vec<usize>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TargetSeed {
    Object(usize),
    AllObjects,
}

#[derive(Clone, Copy)]
enum EffectOperationSeed {
    CopyBaseCreature { from: usize },
    ChangeController { controller: usize },
    SetTextMarker { marker: u32 },
    SetTypes { types: ObjectTypes },
    AddTypes { types: ObjectTypes },
    RemoveTypes { types: ObjectTypes },
    SetColors { colors: ObjectColors },
    AddKeywords { keywords: CreatureKeywords },
    RemoveKeywords { keywords: CreatureKeywords },
    SetBasePowerToughness { power: i32, toughness: i32 },
    SetPowerToughness { power: i32, toughness: i32 },
    ModifyPowerToughness { power: i32, toughness: i32 },
    SwitchPowerToughness,
}

#[derive(Clone, Copy)]
struct CharacteristicExpectation {
    object: usize,
    controller: Option<usize>,
    text_marker: Option<u32>,
    types: Option<ObjectTypes>,
    colors: Option<ObjectColors>,
    is_creature: Option<bool>,
    power: Option<i32>,
    toughness: Option<i32>,
    required_keywords: Option<CreatureKeywords>,
    absent_keywords: Option<CreatureKeywords>,
}

impl CharacteristicExpectation {
    const fn object(object: usize) -> Self {
        Self {
            object,
            controller: None,
            text_marker: None,
            types: None,
            colors: None,
            is_creature: None,
            power: None,
            toughness: None,
            required_keywords: None,
            absent_keywords: None,
        }
    }

    const fn controller(mut self, controller: usize) -> Self {
        self.controller = Some(controller);
        self
    }

    const fn text_marker(mut self, text_marker: u32) -> Self {
        self.text_marker = Some(text_marker);
        self
    }

    const fn types(mut self, types: ObjectTypes) -> Self {
        self.types = Some(types);
        self
    }

    const fn colors(mut self, colors: ObjectColors) -> Self {
        self.colors = Some(colors);
        self
    }

    const fn creature(mut self, power: i32, toughness: i32) -> Self {
        self.is_creature = Some(true);
        self.power = Some(power);
        self.toughness = Some(toughness);
        self
    }

    const fn noncreature(mut self) -> Self {
        self.is_creature = Some(false);
        self
    }

    const fn requires(mut self, keywords: CreatureKeywords) -> Self {
        self.required_keywords = Some(keywords);
        self
    }

    const fn forbids(mut self, keywords: CreatureKeywords) -> Self {
        self.absent_keywords = Some(keywords);
        self
    }
}

fn nightmare_fixtures() -> Vec<NightmareFixture> {
    vec![
        NightmareFixture {
            name: "humility_class_global_ability_loss_and_set_pt",
            card_base: 82_100,
            objects: vec![
                creature(0, 0, 0, 4, 4, CreatureKeywords::none().with_flying()),
                creature(1, 1, 1, 6, 6, CreatureKeywords::none().with_trample()),
            ],
            expectations: vec![
                CharacteristicExpectation::object(0)
                    .creature(1, 1)
                    .forbids(CreatureKeywords::none().with_flying().with_trample()),
                CharacteristicExpectation::object(1)
                    .creature(1, 1)
                    .forbids(CreatureKeywords::none().with_flying().with_trample()),
            ],
        },
        NightmareFixture {
            name: "opalescence_class_enchantments_animated",
            card_base: 82_200,
            objects: vec![blank(0, 0, 0), blank(0, 0, 1)],
            expectations: vec![
                CharacteristicExpectation::object(0)
                    .types(ObjectTypes::none().with_enchantment().with_creature())
                    .creature(4, 4),
                CharacteristicExpectation::object(1)
                    .types(ObjectTypes::none().with_enchantment().with_creature())
                    .creature(4, 4),
            ],
        },
        NightmareFixture {
            name: "blood_moon_class_lands_replace_complex_types",
            card_base: 82_300,
            objects: vec![blank(0, 0, 0), blank(1, 1, 1)],
            expectations: vec![
                CharacteristicExpectation::object(0)
                    .types(ObjectTypes::none().with_land())
                    .noncreature(),
                CharacteristicExpectation::object(1)
                    .types(ObjectTypes::none().with_land())
                    .noncreature(),
            ],
        },
        NightmareFixture {
            name: "copy_then_full_layer_stack",
            card_base: 82_400,
            objects: vec![
                creature(0, 0, 0, 2, 5, CreatureKeywords::none().with_trample()),
                creature(0, 0, 1, 1, 1, CreatureKeywords::none()),
            ],
            expectations: vec![CharacteristicExpectation::object(1)
                .controller(1)
                .text_marker(210)
                .types(ObjectTypes::none().with_creature().with_artifact())
                .colors(ObjectColors::none().with_white().with_blue())
                .creature(4, 4)
                .requires(CreatureKeywords::none().with_flying())
                .forbids(CreatureKeywords::none().with_trample())],
        },
        NightmareFixture {
            name: "same_layer_dependency_reorders_timestamp",
            card_base: 82_500,
            objects: vec![creature(0, 0, 0, 2, 2, CreatureKeywords::none())],
            expectations: vec![CharacteristicExpectation::object(0)
                .colors(ObjectColors::none().with_green())
                .creature(2, 2)],
        },
        NightmareFixture {
            name: "type_removal_blocks_later_keyword_grant",
            card_base: 82_600,
            objects: vec![creature(0, 0, 0, 3, 3, CreatureKeywords::none())],
            expectations: vec![CharacteristicExpectation::object(0)
                .types(ObjectTypes::none())
                .noncreature()],
        },
        NightmareFixture {
            name: "cda_modifier_switch_stack",
            card_base: 82_700,
            objects: vec![creature(
                0,
                0,
                0,
                0,
                0,
                CreatureKeywords::none().with_vigilance(),
            )],
            expectations: vec![CharacteristicExpectation::object(0)
                .creature(6, 5)
                .requires(CreatureKeywords::none().with_vigilance())],
        },
        NightmareFixture {
            name: "specific_later_color_overrides_global",
            card_base: 82_800,
            objects: vec![creature(0, 0, 0, 2, 2, CreatureKeywords::none())],
            expectations: vec![CharacteristicExpectation::object(0)
                .colors(ObjectColors::none().with_blue())
                .creature(2, 2)],
        },
        NightmareFixture {
            name: "control_change_survives_random_play",
            card_base: 82_900,
            objects: vec![creature(
                1,
                1,
                0,
                5,
                5,
                CreatureKeywords::none().with_haste(),
            )],
            expectations: vec![CharacteristicExpectation::object(0)
                .controller(0)
                .creature(5, 5)
                .requires(CreatureKeywords::none().with_haste())],
        },
        NightmareFixture {
            name: "all_layer_dependency_and_keyword_cleanup",
            card_base: 83_000,
            objects: vec![
                creature(0, 0, 0, 1, 4, CreatureKeywords::none().with_flying()),
                blank(0, 0, 1),
            ],
            expectations: vec![
                CharacteristicExpectation::object(0)
                    .types(ObjectTypes::none().with_creature().with_enchantment())
                    .creature(3, 4)
                    .requires(CreatureKeywords::none().with_reach())
                    .forbids(CreatureKeywords::none().with_flying()),
                CharacteristicExpectation::object(1)
                    .types(ObjectTypes::none().with_creature().with_enchantment())
                    .creature(3, 4)
                    .requires(CreatureKeywords::none().with_reach())
                    .forbids(CreatureKeywords::none().with_flying()),
            ],
        },
    ]
}

const fn blank(owner: usize, controller: usize, card_offset: u32) -> ObjectSeed {
    ObjectSeed {
        owner,
        controller,
        card_offset,
        base: None,
    }
}

const fn creature(
    owner: usize,
    controller: usize,
    card_offset: u32,
    power: i32,
    toughness: i32,
    keywords: CreatureKeywords,
) -> ObjectSeed {
    ObjectSeed {
        owner,
        controller,
        card_offset,
        base: Some(BaseCreatureCharacteristics::new(power, toughness).with_keywords(keywords)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compile_layer_effects, nightmare_fixtures, run, run_nightmare_suite, run_smoke,
        NightmareConfig, SmokeConfig,
    };

    fn layer_scenario(ability: &str) -> forge_carddef::CardDefinition {
        let source = format!(
            r#"card "Lowerer Test" {{
  id: "forge:scenario:lowerer:test"
  layout: normal
  status: verified_playable
  face "Lowerer Test" {{
    cost: "{{0}}"
    types: "Enchantment"
    oracle: "Lowerer boundary test."
    keywords: []
{ability}
  }}
}}
"#
        );
        forge_cardc::parse_card_named("lowerer-test.frs", &source)
            .unwrap_or_else(|error| panic!("test scenario did not parse: {error}"))
    }

    fn assert_layer_error(ability: &str, expected: &str) {
        let definition = layer_scenario(ability);
        let fixture = nightmare_fixtures()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("nightmare fixtures are empty"));
        let error = match compile_layer_effects(&definition, &fixture) {
            Ok(_effects) => panic!("invalid layer scenario unexpectedly lowered"),
            Err(error) => error,
        };
        assert!(
            error.contains(expected),
            "expected {expected:?} in lowerer error, got {error:?}"
        );
    }

    #[test]
    fn smoke_two_games_passes() {
        run_smoke(SmokeConfig {
            games: 2,
            random: true,
            max_turns: 2,
        })
        .unwrap_or_else(|error| panic!("unexpected smoke failure: {error}"));
    }

    #[test]
    fn ladder_rejects_invalid_campaign_instead_of_skipping() {
        let error = match run(vec![
            "--ladder".to_string(),
            "--games".to_string(),
            "1".to_string(),
        ]) {
            Err(error) => error,
            Ok(()) => panic!("odd ladder campaign should fail"),
        };
        assert!(error.contains("even integer"));

        let error = match run(vec![
            "--calibrate".to_string(),
            "--tier".to_string(),
            "invalid".to_string(),
        ]) {
            Err(error) => error,
            Ok(()) => panic!("unknown calibration tier must fail closed"),
        };
        assert!(error.contains("unsupported calibration tier"));
    }

    #[test]
    fn nightmare_suite_one_cycle_passes() {
        run_nightmare_suite(NightmareConfig {
            games: 10,
            max_turns: 2,
        })
        .unwrap_or_else(|error| panic!("unexpected nightmare-suite failure: {error}"));
    }

    #[test]
    fn nightmare_suite_cli_passes() {
        run(vec![
            "--nightmare-suite".to_string(),
            "--games".to_string(),
            "3".to_string(),
            "--max-turns".to_string(),
            "1".to_string(),
        ])
        .unwrap_or_else(|error| panic!("unexpected nightmare-suite CLI failure: {error}"));
    }

    #[test]
    fn layer_lowerer_rejects_unclosed_runtime_bindings_and_values() {
        let static_effect =
            |effect: &str| format!("    ability static {{\n      effect: {effect}\n    }}");
        let cases = [
            (
                static_effect(
                    "layer_effect(controller_of(source()), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_0\"), 1, 1), 1)",
                ),
                "player binding",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_99\"), set_pt(remembered(\"fixture_object_99\"), 1, 1), 1)",
                ),
                "outside fixture",
            ),
            (
                static_effect(
                    "layer_effect(you(), all(permanents(type_is(\"creature\"))), set_pt(all(permanents(type_is(\"creature\"))), 1, 1), 1)",
                ),
                "exactly all(permanents())",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_1\"), 1, 1), 1)",
                ),
                "target differs",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), tap(remembered(\"fixture_object_0\")), 1)",
                ),
                "cannot lower",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), set_type(remembered(\"fixture_object_0\"), \"battle\"), 1)",
                ),
                "unsupported layer object type",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), set_color(remembered(\"fixture_object_0\"), \"purple\"), 1)",
                ),
                "unsupported layer object color",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), grant_keyword(remembered(\"fixture_object_0\"), \"shadow\"), 1)",
                ),
                "unsupported layer creature keyword",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_0\"), 1, 1), -1)",
                ),
                "cannot be negative",
            ),
            (
                static_effect(
                    "layer_effect(you(), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_0\"), 1, 1), 1, 0)",
                ),
                "depends on unavailable effect",
            ),
        ];
        for (ability, expected) in cases {
            assert_layer_error(&ability, expected);
        }

        assert_layer_error(
            "    ability spell {\n      effect: layer_effect(you(), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_0\"), 1, 1), 1)\n    }",
            "non-static or conditional",
        );
        assert_layer_error(
            "    ability static {\n      condition: equals(1, 1)\n      effect: layer_effect(you(), remembered(\"fixture_object_0\"), set_pt(remembered(\"fixture_object_0\"), 1, 1), 1)\n    }",
            "non-static or conditional",
        );
    }
}
