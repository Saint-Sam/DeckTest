#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Developer command-line interface crate for Forge 2.0.

use forge_core::{
    apply, Action, CardId, GameOutcome, GameState, Outcome, PlayerId, StateHash, ZoneId, ZoneKind,
};
use std::{
    fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

/// Runs the Forge CLI with already-tokenized arguments and returns stdout text.
///
/// Supported commands:
/// - `play --human [--seed N] [--seat 1..4] [--replay-out PATH]`
/// - `play --ai [--random-legal | --search] [--seed N] [--policy-seed N]`
/// - `play --demo [--seed N] [--replay-out PATH]`
/// - `demo [--seed N] [--replay-out PATH]`
/// - `replay PATH`
/// - `roundtrip PATH`
/// - `help`
pub fn run_cli(args: Vec<String>) -> Result<String, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(usage());
    };
    match command {
        "play" => play_command(&args[1..]),
        "demo" => demo_command(&args[1..]),
        "replay" => replay_command(&args[1..]),
        "roundtrip" => roundtrip_command(&args[1..]),
        "--help" | "-h" | "help" => Ok(usage()),
        other => Err(format!("unknown forge-cli command `{other}`")),
    }
}

/// Runs the CLI with explicit input/output streams for interactive commands.
///
/// Noninteractive commands are delegated to [`run_cli`].
pub fn run_cli_with_io(
    args: Vec<String>,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<String, String> {
    if args.first().map(String::as_str) == Some("play")
        && args.iter().skip(1).any(|argument| argument == "--human")
    {
        return human_play_command(&args[1..], input, output);
    }
    run_cli(args)
}

fn usage() -> String {
    "forge-cli commands:\n  play --human [--seed N] [--seat 1..4] [--manifest PATH] [--replay-out PATH] [--max-turns N]\n  play --ai [--random-legal | --search] [--search-iterations N] [--determinizations N] [--search-workers N] [--seed N] [--policy-seed N] [--noise-span N] [--manifest PATH] [--replay-out PATH] [--max-turns N]\n  play --demo [--seed N] [--replay-out PATH]\n  demo [--seed N] [--replay-out PATH]\n  replay PATH\n  roundtrip PATH\n"
        .to_owned()
}

fn human_play_command(
    args: &[String],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<String, String> {
    let mut seed = 0xF02D_0000_0000_0A10_u64;
    let mut seat = 1_usize;
    let mut manifest = PathBuf::from("assets/t3_9/integration_decks.json");
    let mut replay_out = PathBuf::from("reports/gates/T1.R10/owner-game.frsreplay");
    let mut max_turns = 160_u32;
    let mut saw_human = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--human" => saw_human = true,
            "--seed" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--seed requires a value".to_owned())?;
                seed = value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --seed value `{value}`: {error}"))?;
                index += 1;
            }
            "--seat" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--seat requires a value".to_owned())?;
                seat = value
                    .parse::<usize>()
                    .map_err(|error| format!("invalid --seat value `{value}`: {error}"))?;
                index += 1;
            }
            "--manifest" => {
                manifest = PathBuf::from(
                    args.get(index + 1)
                        .ok_or_else(|| "--manifest requires a path".to_owned())?,
                );
                index += 1;
            }
            "--replay-out" => {
                replay_out = PathBuf::from(
                    args.get(index + 1)
                        .ok_or_else(|| "--replay-out requires a path".to_owned())?,
                );
                index += 1;
            }
            "--max-turns" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--max-turns requires a value".to_owned())?;
                max_turns = value
                    .parse::<u32>()
                    .map_err(|error| format!("invalid --max-turns value `{value}`: {error}"))?;
                index += 1;
            }
            "--demo" => return Err("choose either --human or --demo, not both".to_owned()),
            other => return Err(format!("unknown human-play option `{other}`")),
        }
        index += 1;
    }
    if !saw_human {
        return Err("interactive play requires --human".to_owned());
    }
    if !(1..=4).contains(&seat) {
        return Err("--seat must be in 1..=4".to_owned());
    }
    forge_game_runner::run_prompted_game(
        manifest,
        replay_out,
        seed,
        max_turns,
        seat - 1,
        input,
        output,
    )
}

fn play_command(args: &[String]) -> Result<String, String> {
    if args.iter().any(|argument| argument == "--ai") {
        return ai_play_command(args);
    }
    let mut forwarded = Vec::new();
    let mut saw_demo = false;
    for arg in args {
        if arg == "--demo" {
            saw_demo = true;
        } else {
            forwarded.push(arg.clone());
        }
    }
    if !saw_demo {
        return Err("play currently requires --demo for the T1.11 starter client".to_owned());
    }
    demo_command(&forwarded)
}

fn ai_play_command(args: &[String]) -> Result<String, String> {
    let mut seed = 0xF02D_0000_0000_0A11_u64;
    let mut policy_seed = 0xA1_0000_0000_0001_u64;
    let mut noise_span = 0_i64;
    let mut manifest = PathBuf::from("assets/t3_9/integration_decks.json");
    let mut replay_out = PathBuf::from("reports/gates/T4.3/ai-baseline.frsreplay");
    let mut max_turns = 160_u32;
    let mut random_legal = false;
    let mut search = false;
    let mut search_iterations = 16_u32;
    let mut search_determinizations = 4_u32;
    let mut search_workers = 4_u32;
    let mut saw_ai = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--ai" => saw_ai = true,
            "--random-legal" => random_legal = true,
            "--search" => search = true,
            "--search-iterations" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--search-iterations requires a value".to_owned())?;
                search_iterations = value.parse::<u32>().map_err(|error| {
                    format!("invalid --search-iterations value `{value}`: {error}")
                })?;
                search = true;
                index += 1;
            }
            "--determinizations" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--determinizations requires a value".to_owned())?;
                search_determinizations = value.parse::<u32>().map_err(|error| {
                    format!("invalid --determinizations value `{value}`: {error}")
                })?;
                search = true;
                index += 1;
            }
            "--search-workers" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--search-workers requires a value".to_owned())?;
                search_workers = value.parse::<u32>().map_err(|error| {
                    format!("invalid --search-workers value `{value}`: {error}")
                })?;
                search = true;
                index += 1;
            }
            "--seed" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--seed requires a value".to_owned())?;
                seed = value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --seed value `{value}`: {error}"))?;
                index += 1;
            }
            "--policy-seed" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--policy-seed requires a value".to_owned())?;
                policy_seed = value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --policy-seed value `{value}`: {error}"))?;
                index += 1;
            }
            "--noise-span" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--noise-span requires a value".to_owned())?;
                noise_span = value
                    .parse::<i64>()
                    .map_err(|error| format!("invalid --noise-span value `{value}`: {error}"))?;
                index += 1;
            }
            "--manifest" => {
                manifest = PathBuf::from(
                    args.get(index + 1)
                        .ok_or_else(|| "--manifest requires a path".to_owned())?,
                );
                index += 1;
            }
            "--replay-out" => {
                replay_out = PathBuf::from(
                    args.get(index + 1)
                        .ok_or_else(|| "--replay-out requires a path".to_owned())?,
                );
                index += 1;
            }
            "--max-turns" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--max-turns requires a value".to_owned())?;
                max_turns = value
                    .parse::<u32>()
                    .map_err(|error| format!("invalid --max-turns value `{value}`: {error}"))?;
                index += 1;
            }
            "--human" | "--demo" => {
                return Err("choose exactly one of --ai, --human, or --demo".to_owned());
            }
            other => return Err(format!("unknown AI-play option `{other}`")),
        }
        index += 1;
    }
    if !saw_ai {
        return Err("AI play requires --ai".to_owned());
    }
    if search && random_legal {
        return Err("--search and --random-legal cannot be combined".to_owned());
    }
    let policy = if search {
        forge_game_runner::AiPolicyConfig::Search {
            seed: policy_seed,
            iterations: search_iterations,
            determinizations: search_determinizations,
            workers: search_workers,
        }
    } else if random_legal {
        forge_game_runner::AiPolicyConfig::RandomLegal { seed: policy_seed }
    } else {
        forge_game_runner::AiPolicyConfig::Heuristic {
            seed: policy_seed,
            noise_span,
        }
    };
    forge_game_runner::run_ai_game(
        manifest,
        replay_out,
        forge_game_runner::AiGameOptions::new(seed, max_turns, policy),
    )
}

fn demo_command(args: &[String]) -> Result<String, String> {
    let mut seed = 11;
    let mut replay_out = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--seed" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--seed requires a value".to_owned());
                };
                seed = value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --seed value `{value}`: {error}"))?;
                index += 1;
            }
            "--replay-out" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--replay-out requires a path".to_owned());
                };
                replay_out = Some(value.clone());
                index += 1;
            }
            other => return Err(format!("unknown demo option `{other}`")),
        }
        index += 1;
    }

    let replay = starter_replay(seed);
    let report = run_replay(&replay)?;
    if let Some(path) = replay_out {
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&path, replay.to_text())
            .map_err(|error| format!("failed to write {path}: {error}"))?;
    }
    Ok(format_report("demo", &report, Some(replay.actions.len())))
}

fn replay_command(args: &[String]) -> Result<String, String> {
    if args.len() != 1 {
        return Err("replay requires exactly one path".to_owned());
    }
    let payload = fs::read_to_string(&args[0])
        .map_err(|error| format!("failed to read {}: {error}", args[0]))?;
    if payload.trim_start().starts_with('{') {
        return forge_game_runner::replay_json_file(&args[0]);
    }
    let replay = Replay::parse(&payload)?;
    let report = run_replay(&replay)?;
    Ok(format_report("replay", &report, Some(replay.actions.len())))
}

fn roundtrip_command(args: &[String]) -> Result<String, String> {
    if args.len() != 1 {
        return Err("roundtrip requires exactly one path".to_owned());
    }
    let replay = Replay::from_file(&args[0])?;
    let text = replay.to_text();
    let reparsed = Replay::parse(&text)?;
    let left = run_replay(&replay)?;
    let right = run_replay(&reparsed)?;
    if replay != reparsed {
        return Err("replay parse/serialize did not preserve action list".to_owned());
    }
    if left.final_hash != right.final_hash || left.outcome != right.outcome {
        return Err("replay round-trip produced a different final state".to_owned());
    }
    Ok(format!(
        "roundtrip ok\nseed: {}\nactions: {}\nfinal_hash: {}\noutcome: {}\n",
        replay.seed,
        replay.actions.len(),
        left.final_hash.get(),
        format_outcome(left.outcome)
    ))
}

fn format_report(label: &str, report: &ReplayReport, action_count: Option<usize>) -> String {
    let mut output = format!(
        "{label} complete\nseed: {}\nturn: {}\nfinal_hash: {}\noutcome: {}\n",
        report.seed,
        report.turn_number,
        report.final_hash.get(),
        format_outcome(report.outcome)
    );
    if let Some(count) = action_count {
        output.push_str(&format!("actions: {count}\n"));
    }
    output.push_str("players:\n");
    for player in &report.players {
        output.push_str(&format!(
            "  player {} life={} poison={} lost={}\n",
            player.index, player.life, player.poison, player.lost
        ));
    }
    output
}

fn format_outcome(outcome: GameOutcome) -> String {
    match outcome {
        GameOutcome::InProgress => "in_progress".to_owned(),
        GameOutcome::Won(player) => format!("won player {}", player.index()),
        GameOutcome::Draw => "draw".to_owned(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Replay {
    seed: u64,
    actions: Vec<ReplayAction>,
}

impl Replay {
    fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(input: &str) -> Result<Self, String> {
        let mut lines = input.lines().enumerate();
        let Some((_, magic)) = lines.next() else {
            return Err("empty replay".to_owned());
        };
        if magic.trim() != "forge-replay-v1" {
            return Err("replay must start with forge-replay-v1".to_owned());
        }

        let mut seed = None;
        let mut actions = Vec::new();
        for (line_number, line) in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = trimmed.split_whitespace().collect();
            match fields.as_slice() {
                ["seed", value] => {
                    if seed.is_some() {
                        return Err(format!("line {}: duplicate seed", line_number + 1));
                    }
                    seed = Some(parse_u64(value, line_number)?);
                }
                ["action", rest @ ..] => {
                    actions.push(ReplayAction::parse(rest, line_number)?);
                }
                _ => {
                    return Err(format!(
                        "line {}: expected `seed N` or `action ...`",
                        line_number + 1
                    ));
                }
            }
        }
        let seed = seed.ok_or_else(|| "replay missing seed".to_owned())?;
        Ok(Self { seed, actions })
    }

    fn to_text(&self) -> String {
        let mut output = format!("forge-replay-v1\nseed {}\n", self.seed);
        for action in &self.actions {
            output.push_str("action ");
            output.push_str(&action.to_text());
            output.push('\n');
        }
        output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ReplayAction {
    AddPlayer,
    SeedLibrary {
        player: usize,
        first_card: u32,
        count: u32,
    },
    DecideTurnOrder,
    DrawOpeningHands,
    KeepOpeningHand {
        player: usize,
    },
    StartDecidedPlayer,
    AdvanceStep,
    BotRandom,
    LoseLife {
        player: usize,
        amount: u32,
    },
    CheckStateBasedActions,
}

impl ReplayAction {
    fn parse(fields: &[&str], line_number: usize) -> Result<Self, String> {
        match fields {
            ["add_player"] => Ok(Self::AddPlayer),
            ["seed_library", player, first_card, count] => Ok(Self::SeedLibrary {
                player: parse_usize(player, line_number)?,
                first_card: parse_u32(first_card, line_number)?,
                count: parse_u32(count, line_number)?,
            }),
            ["decide_turn_order"] => Ok(Self::DecideTurnOrder),
            ["draw_opening_hands"] => Ok(Self::DrawOpeningHands),
            ["keep_opening_hand", player] => Ok(Self::KeepOpeningHand {
                player: parse_usize(player, line_number)?,
            }),
            ["start_decided_player"] => Ok(Self::StartDecidedPlayer),
            ["advance_step"] => Ok(Self::AdvanceStep),
            ["bot_random"] => Ok(Self::BotRandom),
            ["lose_life", player, amount] => Ok(Self::LoseLife {
                player: parse_usize(player, line_number)?,
                amount: parse_u32(amount, line_number)?,
            }),
            ["check_state_based_actions"] => Ok(Self::CheckStateBasedActions),
            [name, ..] => Err(format!("line {}: unknown action `{name}`", line_number + 1)),
            [] => Err(format!("line {}: empty action", line_number + 1)),
        }
    }

    fn to_text(&self) -> String {
        match self {
            Self::AddPlayer => "add_player".to_owned(),
            Self::SeedLibrary {
                player,
                first_card,
                count,
            } => {
                format!("seed_library {player} {first_card} {count}")
            }
            Self::DecideTurnOrder => "decide_turn_order".to_owned(),
            Self::DrawOpeningHands => "draw_opening_hands".to_owned(),
            Self::KeepOpeningHand { player } => format!("keep_opening_hand {player}"),
            Self::StartDecidedPlayer => "start_decided_player".to_owned(),
            Self::AdvanceStep => "advance_step".to_owned(),
            Self::BotRandom => "bot_random".to_owned(),
            Self::LoseLife { player, amount } => format!("lose_life {player} {amount}"),
            Self::CheckStateBasedActions => "check_state_based_actions".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayReport {
    seed: u64,
    turn_number: u32,
    outcome: GameOutcome,
    final_hash: StateHash,
    players: Vec<PlayerSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlayerSummary {
    index: usize,
    life: i32,
    poison: u32,
    lost: bool,
}

#[derive(Clone)]
struct ReplayContext {
    state: GameState,
    players: Vec<PlayerId>,
    bot_roll: u64,
}

fn starter_replay(seed: u64) -> Replay {
    Replay {
        seed,
        actions: vec![
            ReplayAction::AddPlayer,
            ReplayAction::AddPlayer,
            ReplayAction::SeedLibrary {
                player: 0,
                first_card: 1_000,
                count: 8,
            },
            ReplayAction::SeedLibrary {
                player: 1,
                first_card: 2_000,
                count: 8,
            },
            ReplayAction::DecideTurnOrder,
            ReplayAction::DrawOpeningHands,
            ReplayAction::KeepOpeningHand { player: 0 },
            ReplayAction::KeepOpeningHand { player: 1 },
            ReplayAction::StartDecidedPlayer,
            ReplayAction::AdvanceStep,
            ReplayAction::AdvanceStep,
            ReplayAction::BotRandom,
            ReplayAction::LoseLife {
                player: 1,
                amount: 25,
            },
            ReplayAction::CheckStateBasedActions,
        ],
    }
}

fn run_replay(replay: &Replay) -> Result<ReplayReport, String> {
    let mut context = ReplayContext {
        state: GameState::new(),
        players: Vec::new(),
        bot_roll: replay.seed,
    };
    require_applied(
        "seed",
        apply(&mut context.state, Action::SetSeed { seed: replay.seed }),
    )?;
    for action in &replay.actions {
        apply_replay_action(&mut context, action)?;
    }

    let observer = context
        .players
        .first()
        .copied()
        .ok_or_else(|| "replay must add at least one player".to_owned())?;
    let view = context
        .state
        .player_view(observer)
        .map_err(|error| format!("failed to project player view: {error:?}"))?;
    let players = view
        .players()
        .iter()
        .map(|player| PlayerSummary {
            index: player.id().index(),
            life: player.life(),
            poison: player.poison(),
            lost: player.lost(),
        })
        .collect();
    Ok(ReplayReport {
        seed: replay.seed,
        turn_number: view.turn_number(),
        outcome: view.game_outcome(),
        final_hash: context.state.deterministic_hash(),
        players,
    })
}

fn apply_replay_action(context: &mut ReplayContext, action: &ReplayAction) -> Result<(), String> {
    match *action {
        ReplayAction::AddPlayer => match apply(&mut context.state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => {
                context.players.push(player);
                Ok(())
            }
            other => Err(format!("add_player returned unexpected outcome {other:?}")),
        },
        ReplayAction::SeedLibrary {
            player,
            first_card,
            count,
        } => seed_library(context, player, first_card, count),
        ReplayAction::DecideTurnOrder => {
            require_turn_order_decided(apply(&mut context.state, Action::DecideTurnOrder))
        }
        ReplayAction::DrawOpeningHands => require_applied(
            "draw_opening_hands",
            apply(&mut context.state, Action::DrawOpeningHands),
        ),
        ReplayAction::KeepOpeningHand { player } => {
            let player = player_id(&context.players, player, "keep_opening_hand")?;
            require_applied(
                "keep_opening_hand",
                apply(
                    &mut context.state,
                    Action::KeepOpeningHand {
                        player,
                        bottom: Vec::new(),
                    },
                ),
            )
        }
        ReplayAction::StartDecidedPlayer => {
            let player = context
                .state
                .starting_player()
                .ok_or_else(|| "start_decided_player before decide_turn_order".to_owned())?;
            require_applied(
                "start_decided_player",
                apply(
                    &mut context.state,
                    Action::StartTurn {
                        active_player: player,
                    },
                ),
            )
        }
        ReplayAction::AdvanceStep => match apply(&mut context.state, Action::AdvanceStep) {
            Outcome::StepAdvanced(_) => Ok(()),
            Outcome::Failed(error) => Err(format!("advance_step failed: {error:?}")),
            other => Err(format!(
                "advance_step returned unexpected outcome {other:?}"
            )),
        },
        ReplayAction::BotRandom => apply_bot_random(context),
        ReplayAction::LoseLife { player, amount } => {
            let player = player_id(&context.players, player, "lose_life")?;
            require_applied(
                "lose_life",
                apply(&mut context.state, Action::LoseLife { player, amount }),
            )
        }
        ReplayAction::CheckStateBasedActions => {
            match apply(&mut context.state, Action::CheckStateBasedActions) {
                Outcome::StateBasedActions(_) => Ok(()),
                Outcome::Failed(error) => {
                    Err(format!("check_state_based_actions failed: {error:?}"))
                }
                other => Err(format!(
                    "check_state_based_actions returned unexpected outcome {other:?}"
                )),
            }
        }
    }
}

fn seed_library(
    context: &mut ReplayContext,
    player: usize,
    first_card: u32,
    count: u32,
) -> Result<(), String> {
    let player_id = player_id(&context.players, player, "seed_library")?;
    let zone = ZoneId::new(Some(player_id), ZoneKind::Library);
    for offset in 0..count {
        match apply(
            &mut context.state,
            Action::CreateObject {
                card: CardId::new(first_card.saturating_add(offset)),
                owner: player_id,
                controller: player_id,
                zone,
            },
        ) {
            Outcome::ObjectCreated(_) => {}
            Outcome::Failed(error) => return Err(format!("seed_library failed: {error:?}")),
            other => {
                return Err(format!(
                    "seed_library returned unexpected outcome {other:?}"
                ))
            }
        }
    }
    Ok(())
}

fn apply_bot_random(context: &mut ReplayContext) -> Result<(), String> {
    if context.state.game_outcome() != GameOutcome::InProgress {
        return Ok(());
    }
    let human = player_id(&context.players, 0, "bot_random")?;
    let bot = player_id(&context.players, 1, "bot_random")?;
    let roll = next_bot_roll(&mut context.bot_roll);
    if context.state.priority_player() == Some(bot) {
        match apply(&mut context.state, Action::PassPriority { player: bot }) {
            Outcome::Priority(_) => return Ok(()),
            Outcome::Failed(error) => return Err(format!("bot pass failed: {error:?}")),
            other => return Err(format!("bot pass returned unexpected outcome {other:?}")),
        }
    }
    match roll % 3 {
        0 => require_applied(
            "bot_gain_life",
            apply(
                &mut context.state,
                Action::GainLife {
                    player: bot,
                    amount: 1,
                },
            ),
        ),
        1 => require_applied(
            "bot_chip_damage",
            apply(
                &mut context.state,
                Action::LoseLife {
                    player: human,
                    amount: 1,
                },
            ),
        ),
        _ => match apply(&mut context.state, Action::AdvanceStep) {
            Outcome::StepAdvanced(_) => Ok(()),
            Outcome::Failed(_) => Ok(()),
            other => Err(format!("bot advance returned unexpected outcome {other:?}")),
        },
    }
}

fn next_bot_roll(value: &mut u64) -> u64 {
    *value = value
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *value
}

fn require_turn_order_decided(outcome: Outcome) -> Result<(), String> {
    match outcome {
        Outcome::TurnOrderDecided(_) => Ok(()),
        Outcome::Failed(error) => Err(format!("decide_turn_order failed: {error:?}")),
        other => Err(format!(
            "decide_turn_order returned unexpected outcome {other:?}"
        )),
    }
}

fn require_applied(label: &str, outcome: Outcome) -> Result<(), String> {
    match outcome {
        Outcome::Applied => Ok(()),
        Outcome::Failed(error) => Err(format!("{label} failed: {error:?}")),
        other => Err(format!("{label} returned unexpected outcome {other:?}")),
    }
}

fn player_id(players: &[PlayerId], index: usize, label: &str) -> Result<PlayerId, String> {
    players
        .get(index)
        .copied()
        .ok_or_else(|| format!("{label}: unknown player {index}"))
}

fn parse_usize(input: &str, line_number: usize) -> Result<usize, String> {
    input.parse::<usize>().map_err(|error| {
        format!(
            "line {}: invalid integer `{input}`: {error}",
            line_number + 1
        )
    })
}

fn parse_u32(input: &str, line_number: usize) -> Result<u32, String> {
    input
        .parse::<u32>()
        .map_err(|error| format!("line {}: invalid u32 `{input}`: {error}", line_number + 1))
}

fn parse_u64(input: &str, line_number: usize) -> Result<u64, String> {
    input
        .parse::<u64>()
        .map_err(|error| format!("line {}: invalid u64 `{input}`: {error}", line_number + 1))
}

#[cfg(test)]
mod tests {
    use super::{crate_ready, run_cli, run_replay, starter_replay, Replay};
    use std::{env, fs};

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn scripted_starter_game_finishes() {
        let replay = starter_replay(3);
        let report =
            run_replay(&replay).unwrap_or_else(|error| panic!("unexpected replay error: {error}"));

        assert!(matches!(
            report.outcome,
            forge_core::GameOutcome::Won(winner) if winner.index() == 0
        ));
        assert!(report
            .players
            .iter()
            .any(|player| player.index == 1 && player.lost));
    }

    #[test]
    fn replay_text_round_trips() {
        let replay = starter_replay(17);
        let text = replay.to_text();
        let parsed =
            Replay::parse(&text).unwrap_or_else(|error| panic!("unexpected parse error: {error}"));
        let left =
            run_replay(&replay).unwrap_or_else(|error| panic!("unexpected replay error: {error}"));
        let right =
            run_replay(&parsed).unwrap_or_else(|error| panic!("unexpected replay error: {error}"));

        assert_eq!(replay, parsed);
        assert_eq!(left.final_hash, right.final_hash);
        assert_eq!(left.outcome, right.outcome);
    }

    #[test]
    fn cli_demo_writes_replay_and_roundtrips() {
        let mut path = env::temp_dir();
        path.push(format!("forge-cli-test-{}.frsreplay", std::process::id()));
        let path_text = path.to_string_lossy().to_string();

        let demo = run_cli(vec![
            "demo".to_owned(),
            "--seed".to_owned(),
            "5".to_owned(),
            "--replay-out".to_owned(),
            path_text.clone(),
        ])
        .unwrap_or_else(|error| panic!("unexpected demo error: {error}"));
        assert!(demo.contains("demo complete"));

        let roundtrip = run_cli(vec!["roundtrip".to_owned(), path_text.clone()])
            .unwrap_or_else(|error| panic!("unexpected roundtrip error: {error}"));
        assert!(roundtrip.contains("roundtrip ok"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn replay_parser_rejects_bad_magic() {
        let error = Replay::parse("not-a-replay\nseed 1\n")
            .err()
            .unwrap_or_else(|| panic!("expected parse error"));
        assert!(error.contains("forge-replay-v1"));
    }

    #[test]
    fn cli_rejects_unknown_command() {
        let error = run_cli(vec!["nope".to_owned()])
            .err()
            .unwrap_or_else(|| panic!("expected cli error"));
        assert!(error.contains("unknown"));
    }
}
