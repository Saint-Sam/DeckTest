#![allow(missing_docs)]
#![forbid(unsafe_code)]

//! Headless arena smoke CLI.

use forge_core::{
    apply, legal_actions, Action, BaseCreatureCharacteristics, CardId, ContinuousEffectDefinition,
    ContinuousEffectId, ContinuousEffectOperation, ContinuousEffectTarget, CreatureKeywords,
    GameOutcome, GameState, ObjectCharacteristics, ObjectColors, ObjectId, ObjectTypes, Outcome,
    PlayerId, StateError, ZoneId, ZoneKind,
};
use std::env;
use std::process::ExitCode;

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
        let games = value_after(&args, "--games")?.unwrap_or(2_000);
        println!("SKIP arena ladder: T4 ladder engine is not active yet ({games} requested games)");
        return Ok(());
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
    println!("forge-arena --ladder --games <games>");
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
    let mut violations = 0_u32;
    for game_index in 0..config.games {
        let fixture = &fixtures[game_index as usize % fixtures.len()];
        let seed = 0xF0_26_E2_10_u64 ^ u64::from(game_index).wrapping_mul(0x517C_C1B7_2722_0A95);
        match run_one_nightmare_game(seed, fixture, config.max_turns) {
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
            "PASS nightmare suite: {} game(s), {} fixture(s), 0 invariant violations",
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
    max_turns: u32,
) -> Result<(), String> {
    let mut rng = DeterministicRng::new(seed);
    let (mut state, objects) = setup_nightmare_game(seed, fixture)?;
    verify_nightmare_fixture(&state, fixture, &objects)?;
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
) -> Result<(GameState, Vec<ObjectId>), String> {
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

    let mut effects = Vec::with_capacity(fixture.effects.len());
    for effect in &fixture.effects {
        let definition = effect_definition(effect, &players, &objects, &effects, fixture.name)?;
        effects.push(expect_continuous_effect(apply(
            &mut state,
            Action::RegisterContinuousEffect { definition },
        ))?);
    }

    Ok((state, objects))
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
    effect: &EffectSeed,
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

struct NightmareFixture {
    name: &'static str,
    card_base: u32,
    objects: Vec<ObjectSeed>,
    effects: Vec<EffectSeed>,
    expectations: Vec<CharacteristicExpectation>,
}

#[derive(Clone, Copy)]
struct ObjectSeed {
    owner: usize,
    controller: usize,
    card_offset: u32,
    base: Option<BaseCreatureCharacteristics>,
}

#[derive(Clone, Copy)]
struct EffectSeed {
    controller: usize,
    target: TargetSeed,
    operation: EffectOperationSeed,
    timestamp: u64,
    dependencies: &'static [usize],
}

#[derive(Clone, Copy)]
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::RemoveKeywords {
                        keywords: CreatureKeywords::none().with_flying().with_trample(),
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::SetPowerToughness {
                        power: 1,
                        toughness: 1,
                    },
                    2,
                ),
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::SetTypes {
                        types: ObjectTypes::none().with_enchantment(),
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::AddTypes {
                        types: ObjectTypes::none().with_creature(),
                    },
                    2,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::SetPowerToughness {
                        power: 4,
                        toughness: 4,
                    },
                    3,
                ),
            ],
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::SetTypes {
                        types: ObjectTypes::none().with_artifact().with_land(),
                    },
                    1,
                ),
                effect(
                    1,
                    TargetSeed::Object(1),
                    EffectOperationSeed::SetTypes {
                        types: ObjectTypes::none().with_creature().with_land(),
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::SetTypes {
                        types: ObjectTypes::none().with_land(),
                    },
                    2,
                ),
            ],
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::CopyBaseCreature { from: 0 },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::ChangeController { controller: 1 },
                    2,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::SetTextMarker { marker: 210 },
                    3,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::AddTypes {
                        types: ObjectTypes::none().with_artifact(),
                    },
                    4,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::SetColors {
                        colors: ObjectColors::none().with_white().with_blue(),
                    },
                    5,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::RemoveKeywords {
                        keywords: CreatureKeywords::none().with_trample(),
                    },
                    6,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::AddKeywords {
                        keywords: CreatureKeywords::none().with_flying(),
                    },
                    7,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::SetPowerToughness {
                        power: 3,
                        toughness: 6,
                    },
                    8,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::ModifyPowerToughness {
                        power: 1,
                        toughness: -2,
                    },
                    9,
                ),
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::SwitchPowerToughness,
                    10,
                ),
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::SetColors {
                        colors: ObjectColors::none().with_red(),
                    },
                    5,
                ),
                EffectSeed {
                    controller: 0,
                    target: TargetSeed::Object(0),
                    operation: EffectOperationSeed::SetColors {
                        colors: ObjectColors::none().with_green(),
                    },
                    timestamp: 1,
                    dependencies: &[0],
                },
            ],
            expectations: vec![CharacteristicExpectation::object(0)
                .colors(ObjectColors::none().with_green())
                .creature(2, 2)],
        },
        NightmareFixture {
            name: "type_removal_blocks_later_keyword_grant",
            card_base: 82_600,
            objects: vec![creature(0, 0, 0, 3, 3, CreatureKeywords::none())],
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::RemoveTypes {
                        types: ObjectTypes::none().with_creature(),
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::AddKeywords {
                        keywords: CreatureKeywords::none().with_flying(),
                    },
                    2,
                ),
            ],
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::SetBasePowerToughness {
                        power: 2,
                        toughness: 7,
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::ModifyPowerToughness {
                        power: 3,
                        toughness: -1,
                    },
                    2,
                ),
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::SwitchPowerToughness,
                    3,
                ),
            ],
            expectations: vec![CharacteristicExpectation::object(0)
                .creature(6, 5)
                .requires(CreatureKeywords::none().with_vigilance())],
        },
        NightmareFixture {
            name: "specific_later_color_overrides_global",
            card_base: 82_800,
            objects: vec![creature(0, 0, 0, 2, 2, CreatureKeywords::none())],
            effects: vec![
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::SetColors {
                        colors: ObjectColors::none().with_red(),
                    },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::Object(0),
                    EffectOperationSeed::SetColors {
                        colors: ObjectColors::none().with_blue(),
                    },
                    2,
                ),
            ],
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
            effects: vec![effect(
                0,
                TargetSeed::Object(0),
                EffectOperationSeed::ChangeController { controller: 0 },
                1,
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
            effects: vec![
                effect(
                    0,
                    TargetSeed::Object(1),
                    EffectOperationSeed::CopyBaseCreature { from: 0 },
                    1,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::AddTypes {
                        types: ObjectTypes::none().with_enchantment(),
                    },
                    2,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::RemoveKeywords {
                        keywords: CreatureKeywords::none().with_flying(),
                    },
                    3,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::AddKeywords {
                        keywords: CreatureKeywords::none().with_reach(),
                    },
                    4,
                ),
                effect(
                    0,
                    TargetSeed::AllObjects,
                    EffectOperationSeed::ModifyPowerToughness {
                        power: 2,
                        toughness: 0,
                    },
                    5,
                ),
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

const fn effect(
    controller: usize,
    target: TargetSeed,
    operation: EffectOperationSeed,
    timestamp: u64,
) -> EffectSeed {
    EffectSeed {
        controller,
        target,
        operation,
        timestamp,
        dependencies: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::{run, run_nightmare_suite, run_smoke, NightmareConfig, SmokeConfig};

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
    fn ladder_placeholder_is_non_failing() {
        run(vec![
            "--ladder".to_string(),
            "--games".to_string(),
            "1".to_string(),
        ])
        .unwrap_or_else(|error| panic!("unexpected ladder placeholder failure: {error}"));
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
}
