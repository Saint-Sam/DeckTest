#![allow(missing_docs)]
#![forbid(unsafe_code)]

//! Headless arena smoke CLI.

use forge_core::{
    apply, legal_actions, Action, CardId, GameOutcome, GameState, Outcome, PlayerId, StateError,
    ZoneId, ZoneKind,
};
use std::env;
use std::process::ExitCode;

const DEFAULT_MAX_TURNS: u32 = 4;
const MAX_STEPS_PER_GAME: u32 = 512;
const LIBRARY_CARDS_PER_PLAYER: u32 = 32;

#[derive(Clone, Copy)]
struct SmokeConfig {
    games: u32,
    random: bool,
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
    let config = parse_smoke_args(&args)?;
    run_smoke(config)
}

fn print_help() {
    println!("forge-arena --smoke <games> --random [--max-turns <turns>]");
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

fn expect_object(outcome: Outcome) -> Result<(), String> {
    match outcome {
        Outcome::ObjectCreated(_) => Ok(()),
        Outcome::Failed(error) => Err(format_state_error("CreateObject", error)),
        other => Err(format!("unexpected CreateObject outcome: {other:?}")),
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

#[cfg(test)]
mod tests {
    use super::{run, run_smoke, SmokeConfig};

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
}
