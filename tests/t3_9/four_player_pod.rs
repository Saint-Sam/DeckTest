#![forbid(unsafe_code)]

//! Local T3.9 and CP-FOUR-PLAYER-POD integration runner.
//!
//! The controller is deliberately generic: deck contents come from a
//! deterministic manifest, card behavior comes from `forge-cards::runtime`,
//! and every mutation crosses `forge_core::apply`.

use forge_cards::runtime::{
    bind_triggered_ability_actions, compile_card_program, CardProgram, ExecutionBindings,
    ProgramKind, TriggeredAbilityProgram,
};
use forge_core::{
    apply, Action, ActivatedAbilityId, AttackDeclaration, BlockDeclaration, CardId,
    CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest, CombatDamageStepKind,
    CombatDamageTarget, GameOutcome, GameState, ManaKind, ObjectColors, ObjectId, ObjectView,
    Outcome, PaymentPlan, PlayerId, PlayerView, PriorityOutcome, SpellTiming, StackEntryId,
    StackObjectKind, Step, TriggerId, ZoneId, ZoneKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    env, fs,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Instant,
};

const PLAYER_COUNT: usize = 4;
const COMMANDER_DECK_SIZE: usize = 100;
const STARTING_LIFE: i32 = 40;
const MAX_WORKERS: usize = 24;
const DEFAULT_GAMES: usize = 1_000;
const DEFAULT_MAX_TURNS: u32 = 160;
const DEFAULT_SEED_BASE: u64 = 0xF02D_0000_0000_0000;
const POD_REPLAY_MAGIC: &str = "forge-pod-replay-v1";
const HUMAN_REPLAY_MAGIC: &str = "forge-human-play-replay-v1";
const RETAINED_REPLAYS: usize = 10;

#[allow(dead_code)]
fn main() {
    if let Err(error) = run() {
        eprintln!("T3.9 pod gate failed: {error}");
        std::process::exit(1);
    }
}

/// Runs the complete local T3.9 pod campaign from process arguments.
pub fn run() -> Result<(), String> {
    let options = Options::parse()?;
    let load_started = Instant::now();
    let pod = Arc::new(PodTemplate::load(&options.manifest)?);
    let load_ms = load_started.elapsed().as_millis();

    if options.validate_only {
        println!(
            "validated {} legal decks ({} cards each, {} semantic mainboard cards)",
            pod.decks.len(),
            COMMANDER_DECK_SIZE,
            pod.semantic_mainboard_cards
        );
        return Ok(());
    }

    let campaign_started = Instant::now();
    let campaign = run_campaign(
        Arc::clone(&pod),
        options.games,
        options.jobs,
        options.max_turns,
        options.seed_base,
        &options.manifest,
    )?;
    let campaign_ms = campaign_started.elapsed().as_millis();

    validate_campaign(&campaign.summaries, options.games, &pod.semantic_identities)?;
    write_replays(&options.replay_dir, &campaign.replays)?;
    let report = build_report(
        &pod,
        &options,
        &campaign.summaries,
        load_ms,
        campaign_ms,
        campaign.primary_worker_ms,
        campaign.replay_worker_ms,
    );
    if let Some(parent) = options.output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("failed to serialize pod report: {error}"))?;
    fs::write(&options.output, payload)
        .map_err(|error| format!("failed to write {}: {error}", options.output.display()))?;
    println!(
        "PASS: {} deterministic four-player games replayed exactly; report={}",
        options.games,
        options.output.display()
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct Options {
    manifest: PathBuf,
    output: PathBuf,
    replay_dir: PathBuf,
    games: usize,
    jobs: usize,
    max_turns: u32,
    seed_base: u64,
    validate_only: bool,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let default_jobs = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(MAX_WORKERS);
        let mut options = Self {
            manifest: PathBuf::from("assets/t3_9/integration_decks.json"),
            output: PathBuf::from("metrics/four_player_pod.json"),
            replay_dir: PathBuf::from("reports/gates/T3.9/replays"),
            games: DEFAULT_GAMES,
            jobs: default_jobs,
            max_turns: DEFAULT_MAX_TURNS,
            seed_base: DEFAULT_SEED_BASE,
            validate_only: false,
        };
        let mut args = env::args().skip(1);
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--manifest" => options.manifest = PathBuf::from(next_arg(&mut args, &argument)?),
                "--output" => options.output = PathBuf::from(next_arg(&mut args, &argument)?),
                "--replay-dir" => {
                    options.replay_dir = PathBuf::from(next_arg(&mut args, &argument)?);
                }
                "--games" => {
                    options.games = parse_arg(&next_arg(&mut args, &argument)?, &argument)?;
                }
                "--jobs" => {
                    options.jobs = parse_arg(&next_arg(&mut args, &argument)?, &argument)?;
                }
                "--max-turns" => {
                    options.max_turns = parse_arg(&next_arg(&mut args, &argument)?, &argument)?;
                }
                "--seed-base" => {
                    options.seed_base = parse_seed(&next_arg(&mut args, &argument)?)?;
                }
                "--validate-only" => options.validate_only = true,
                "--help" | "-h" => {
                    println!(
                        "forge-t3-9-four-player-pod [--manifest PATH] [--output PATH] \
                         [--replay-dir PATH] \
                         [--games N] [--jobs 1..24] [--max-turns N] [--seed-base N] \
                         [--validate-only]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        if options.games == 0 {
            return Err("--games must be positive".to_owned());
        }
        if options.jobs == 0 || options.jobs > MAX_WORKERS {
            return Err(format!("--jobs must be in 1..={MAX_WORKERS}"));
        }
        if options.max_turns < 20 {
            return Err("--max-turns must be at least 20".to_owned());
        }
        Ok(options)
    }
}

fn next_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_arg<T>(value: &str, flag: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| format!("invalid {flag} value `{value}`: {error}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|error| format!("invalid --seed-base value `{value}`: {error}"))
}

#[derive(Clone)]
struct CardAsset {
    path: String,
    program: Arc<CardProgram>,
    color_identity: ObjectColors,
}

#[derive(Clone)]
struct DeckTemplate {
    id: String,
    name: String,
    color_identity: ObjectColors,
    commander: CardAsset,
    mainboard: Vec<CardAsset>,
}

struct PodTemplate {
    manifest_path: PathBuf,
    source_root: PathBuf,
    semantic_registry: PathBuf,
    decks: Vec<DeckTemplate>,
    semantic_mainboard_cards: usize,
    semantic_identities: BTreeMap<String, bool>,
}

impl PodTemplate {
    fn load(manifest_path: &Path) -> Result<Self, String> {
        let manifest_payload = fs::read_to_string(manifest_path)
            .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
        let manifest: Value = serde_json::from_str(&manifest_payload)
            .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
        require_u64(&manifest, "schema_version")?
            .eq(&1)
            .then_some(())
            .ok_or_else(|| format!("{} has unsupported schema_version", manifest_path.display()))?;
        let source_root = PathBuf::from(require_str(&manifest, "source_root")?);
        let semantic_registry = PathBuf::from(require_str(&manifest, "semantic_registry")?);
        let semantic_ids = load_semantic_ids(&semantic_registry)?;
        let deck_values = require_array(&manifest, "decks")?;
        if deck_values.len() != PLAYER_COUNT {
            return Err(format!(
                "manifest must contain exactly {PLAYER_COUNT} decks, found {}",
                deck_values.len()
            ));
        }

        let mut cache = BTreeMap::<String, CardAsset>::new();
        let mut deck_ids = BTreeSet::new();
        let mut decks = Vec::with_capacity(PLAYER_COUNT);
        let mut semantic_mainboard_cards = 0_usize;
        for deck_value in deck_values {
            let id = require_str(deck_value, "id")?.to_owned();
            if !deck_ids.insert(id.clone()) {
                return Err(format!("duplicate deck id `{id}`"));
            }
            let name = require_str(deck_value, "name")?.to_owned();
            let expected_colors = parse_colors(require_array(deck_value, "color_identity")?)?;
            let commander_path = require_str(deck_value, "commander")?;
            let commander = load_card(&source_root, commander_path, &mut cache)?;
            validate_commander(&commander, expected_colors, &id)?;

            let mut mainboard = Vec::with_capacity(COMMANDER_DECK_SIZE - 1);
            let mut identity_counts = BTreeMap::<String, usize>::new();
            for entry in require_array(deck_value, "cards")? {
                let path = require_str(entry, "path")?;
                let count = usize::try_from(require_u64(entry, "count")?)
                    .map_err(|_| format!("card count for `{path}` does not fit usize"))?;
                if count == 0 || count >= COMMANDER_DECK_SIZE {
                    return Err(format!("invalid count {count} for `{path}` in `{id}`"));
                }
                let card = load_card(&source_root, path, &mut cache)?;
                if !colors_subset(card.color_identity, expected_colors) {
                    return Err(format!(
                        "{} ({}) is outside commander color identity for `{id}`",
                        card.program.name(),
                        path
                    ));
                }
                if !semantic_ids.contains(card.program.oracle_id()) {
                    return Err(format!(
                        "{} ({}) is not in the frozen semantic-verified registry",
                        card.program.name(),
                        path
                    ));
                }
                *identity_counts
                    .entry(card.program.oracle_id().to_owned())
                    .or_default() += count;
                for _ in 0..count {
                    mainboard.push(card.clone());
                    semantic_mainboard_cards = semantic_mainboard_cards.saturating_add(1);
                }
            }
            if mainboard.len() + 1 != COMMANDER_DECK_SIZE {
                return Err(format!(
                    "deck `{id}` has {} cards including commander; expected {COMMANDER_DECK_SIZE}",
                    mainboard.len() + 1
                ));
            }
            for (oracle_id, count) in identity_counts {
                if count <= 1 {
                    continue;
                }
                let card = mainboard
                    .iter()
                    .find(|card| card.program.oracle_id() == oracle_id)
                    .ok_or_else(|| format!("missing duplicate identity {oracle_id}"))?;
                if !card.program.base_object().supertypes().basic() {
                    return Err(format!(
                        "deck `{id}` contains {count} copies of nonbasic {}",
                        card.program.name()
                    ));
                }
            }
            decks.push(DeckTemplate {
                id,
                name,
                color_identity: expected_colors,
                commander,
                mainboard,
            });
        }

        let mut semantic_identities = BTreeMap::new();
        for deck in &decks {
            for card in &deck.mainboard {
                let oracle_id = card.program.oracle_id().to_owned();
                let is_land = card.program.kind() == ProgramKind::Land;
                if let Some(previous) = semantic_identities.insert(oracle_id.clone(), is_land) {
                    if previous != is_land {
                        return Err(format!(
                            "semantic identity {oracle_id} compiled as both land and nonland"
                        ));
                    }
                }
            }
        }

        Ok(Self {
            manifest_path: manifest_path.to_path_buf(),
            source_root,
            semantic_registry,
            decks,
            semantic_mainboard_cards,
            semantic_identities,
        })
    }
}

fn load_semantic_ids(path: &Path) -> Result<HashSet<String>, String> {
    let payload = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if value.get("passed").and_then(Value::as_bool) != Some(true)
        || value.get("stage").and_then(Value::as_str) != Some("semantic_verified")
    {
        return Err(format!(
            "{} is not a passing semantic_verified registry",
            path.display()
        ));
    }
    let ids = require_array(&value, "identity_ids")?;
    if ids.len() != 100 {
        return Err(format!(
            "{} must bind exactly 100 semantic identities, found {}",
            path.display(),
            ids.len()
        ));
    }
    ids.iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "semantic identity_ids must contain strings".to_owned())
        })
        .collect()
}

fn load_card(
    source_root: &Path,
    relative_path: &str,
    cache: &mut BTreeMap<String, CardAsset>,
) -> Result<CardAsset, String> {
    if let Some(cached) = cache.get(relative_path) {
        return Ok(cached.clone());
    }
    let path = source_root.join(relative_path);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let definition = forge_cardc::parse_card_named(relative_path, &source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let program = compile_card_program(&definition)
        .map_err(|error| format!("failed to compile {}: {error}", path.display()))?;
    let color_identity = program_color_identity(&program);
    let asset = CardAsset {
        path: relative_path.to_owned(),
        program: Arc::new(program),
        color_identity,
    };
    cache.insert(relative_path.to_owned(), asset.clone());
    Ok(asset)
}

fn validate_commander(
    card: &CardAsset,
    expected_colors: ObjectColors,
    deck_id: &str,
) -> Result<(), String> {
    let base = card.program.base_object();
    if card.program.kind() != ProgramKind::Permanent
        || !base.types().creature()
        || !base.supertypes().legendary()
    {
        return Err(format!(
            "{} ({}) is not a compiled legendary creature commander",
            card.program.name(),
            card.path
        ));
    }
    if card.color_identity != expected_colors {
        return Err(format!(
            "commander {} identity does not match manifest identity for `{deck_id}`",
            card.program.name()
        ));
    }
    Ok(())
}

fn program_color_identity(program: &CardProgram) -> ObjectColors {
    let base = program.base_object();
    let colors = base.colors();
    let symbols = base.printed_mana_symbols();
    let mut identity = ObjectColors::none();
    if colors.white() || symbols.get(ManaKind::White) > 0 {
        identity = identity.with_white();
    }
    if colors.blue() || symbols.get(ManaKind::Blue) > 0 {
        identity = identity.with_blue();
    }
    if colors.black() || symbols.get(ManaKind::Black) > 0 {
        identity = identity.with_black();
    }
    if colors.red() || symbols.get(ManaKind::Red) > 0 {
        identity = identity.with_red();
    }
    if colors.green() || symbols.get(ManaKind::Green) > 0 {
        identity = identity.with_green();
    }
    for ability in program.activated_abilities() {
        for output in ability.output_choices().options() {
            if output.get(ManaKind::White) > 0 {
                identity = identity.with_white();
            }
            if output.get(ManaKind::Blue) > 0 {
                identity = identity.with_blue();
            }
            if output.get(ManaKind::Black) > 0 {
                identity = identity.with_black();
            }
            if output.get(ManaKind::Red) > 0 {
                identity = identity.with_red();
            }
            if output.get(ManaKind::Green) > 0 {
                identity = identity.with_green();
            }
        }
    }
    identity
}

fn colors_subset(candidate: ObjectColors, allowed: ObjectColors) -> bool {
    (!candidate.white() || allowed.white())
        && (!candidate.blue() || allowed.blue())
        && (!candidate.black() || allowed.black())
        && (!candidate.red() || allowed.red())
        && (!candidate.green() || allowed.green())
}

fn parse_colors(values: &[Value]) -> Result<ObjectColors, String> {
    let mut colors = ObjectColors::none();
    let mut seen = BTreeSet::new();
    for value in values {
        let symbol = value
            .as_str()
            .ok_or_else(|| "color_identity entries must be strings".to_owned())?;
        if !seen.insert(symbol) {
            return Err(format!("duplicate color identity symbol `{symbol}`"));
        }
        colors = match symbol {
            "W" => colors.with_white(),
            "U" => colors.with_blue(),
            "B" => colors.with_black(),
            "R" => colors.with_red(),
            "G" => colors.with_green(),
            other => return Err(format!("unsupported color identity symbol `{other}`")),
        };
    }
    Ok(colors)
}

fn require_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string field `{key}`"))
}

fn require_u64(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid integer field `{key}`"))
}

fn require_array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("missing or invalid array field `{key}`"))
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct IdentityExercise {
    land_plays: u64,
    casts: u64,
    resolutions: u64,
    effect_actions: u64,
    trigger_resolutions: u64,
}

impl IdentityExercise {
    fn add_assign(&mut self, other: &Self) {
        self.land_plays = self.land_plays.saturating_add(other.land_plays);
        self.casts = self.casts.saturating_add(other.casts);
        self.resolutions = self.resolutions.saturating_add(other.resolutions);
        self.effect_actions = self.effect_actions.saturating_add(other.effect_actions);
        self.trigger_resolutions = self
            .trigger_resolutions
            .saturating_add(other.trigger_resolutions);
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TraceRecord {
    index: u64,
    action: String,
    before_hash: u64,
    outcome: String,
    after_hash: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PodReplay {
    format: String,
    manifest: PathBuf,
    seed: u64,
    max_turns: u32,
    coverage_target: Option<String>,
    actions: Vec<TraceRecord>,
    expected: GameSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct HumanDecisionRecord {
    index: u64,
    prompt: String,
    turn: u32,
    step: String,
    view_fingerprint: String,
    options: Vec<String>,
    selected: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct HumanPlayReplay {
    format: String,
    manifest: PathBuf,
    seed: u64,
    max_turns: u32,
    human_seat: usize,
    decisions: Vec<HumanDecisionRecord>,
    actions: Vec<TraceRecord>,
    expected: GameSummary,
}

struct DecisionPrompt<'a> {
    kind: &'static str,
    view: &'a PlayerView,
    options: &'a [String],
}

trait DecisionSource {
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String>;
}

struct TerminalDecisionSource<'a> {
    input: &'a mut dyn BufRead,
    output: &'a mut dyn Write,
    decisions: Vec<HumanDecisionRecord>,
}

impl<'a> TerminalDecisionSource<'a> {
    fn new(input: &'a mut dyn BufRead, output: &'a mut dyn Write) -> Self {
        Self {
            input,
            output,
            decisions: Vec::new(),
        }
    }

    fn into_decisions(self) -> Vec<HumanDecisionRecord> {
        self.decisions
    }
}

impl DecisionSource for TerminalDecisionSource<'_> {
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String> {
        if prompt.options.is_empty() {
            return Err(format!("{} prompt has no legal options", prompt.kind));
        }
        let observer = prompt.view.observer();
        writeln!(
            self.output,
            "\nTurn {} {:?} | You are seat {}",
            prompt.view.turn_number(),
            prompt.view.current_step(),
            observer.index() + 1
        )
        .map_err(|error| format!("failed to write human prompt: {error}"))?;
        write!(self.output, "Life:")
            .map_err(|error| format!("failed to write human prompt: {error}"))?;
        for player in prompt.view.players() {
            write!(
                self.output,
                " seat {}={}{}",
                player.id().index() + 1,
                player.life(),
                if player.lost() { " (out)" } else { "" }
            )
            .map_err(|error| format!("failed to write human prompt: {error}"))?;
        }
        writeln!(self.output, "\n{}", prompt.kind)
            .map_err(|error| format!("failed to write human prompt: {error}"))?;
        for (index, option) in prompt.options.iter().enumerate() {
            writeln!(self.output, "  {}. {option}", index + 1)
                .map_err(|error| format!("failed to write human prompt: {error}"))?;
        }

        let selected = loop {
            write!(self.output, "> ")
                .map_err(|error| format!("failed to write human prompt: {error}"))?;
            self.output
                .flush()
                .map_err(|error| format!("failed to flush human prompt: {error}"))?;
            let mut line = String::new();
            let read = self
                .input
                .read_line(&mut line)
                .map_err(|error| format!("failed to read human choice: {error}"))?;
            if read == 0 {
                return Err("human input ended before the game completed".to_owned());
            }
            if line.trim().eq_ignore_ascii_case("q") {
                return Err("human game aborted by owner".to_owned());
            }
            let Ok(choice) = line.trim().parse::<usize>() else {
                writeln!(self.output, "Enter an option number, or q to stop.")
                    .map_err(|error| format!("failed to write human prompt: {error}"))?;
                continue;
            };
            if (1..=prompt.options.len()).contains(&choice) {
                break choice - 1;
            }
            writeln!(
                self.output,
                "Choose a number from 1 to {}.",
                prompt.options.len()
            )
            .map_err(|error| format!("failed to write human prompt: {error}"))?;
        };
        self.decisions.push(snapshot_prompt(
            self.decisions.len() as u64,
            prompt,
            selected,
        ));
        Ok(selected)
    }
}

struct ReplayDecisionSource {
    decisions: Vec<HumanDecisionRecord>,
    cursor: usize,
}

impl ReplayDecisionSource {
    fn new(decisions: Vec<HumanDecisionRecord>) -> Self {
        Self {
            decisions,
            cursor: 0,
        }
    }

    fn finish(&self) -> Result<(), String> {
        if self.cursor == self.decisions.len() {
            Ok(())
        } else {
            Err(format!(
                "decision replay stopped after {} of {} prompts",
                self.cursor,
                self.decisions.len()
            ))
        }
    }
}

impl DecisionSource for ReplayDecisionSource {
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String> {
        let expected = self
            .decisions
            .get(self.cursor)
            .ok_or_else(|| format!("unexpected replay prompt `{}`", prompt.kind))?;
        let actual = snapshot_prompt(expected.index, prompt, expected.selected);
        if &actual != expected {
            return Err(format!(
                "decision replay diverged at prompt {}: expected {expected:?}, got {actual:?}",
                self.cursor
            ));
        }
        if expected.selected >= prompt.options.len() {
            return Err(format!(
                "decision replay selection {} is outside {} options",
                expected.selected,
                prompt.options.len()
            ));
        }
        self.cursor = self.cursor.saturating_add(1);
        Ok(expected.selected)
    }
}

fn snapshot_prompt(
    index: u64,
    prompt: &DecisionPrompt<'_>,
    selected: usize,
) -> HumanDecisionRecord {
    HumanDecisionRecord {
        index,
        prompt: prompt.kind.to_owned(),
        turn: prompt.view.turn_number(),
        step: prompt
            .view
            .current_step()
            .map_or_else(|| "none".to_owned(), |step| format!("{step:?}")),
        view_fingerprint: player_view_fingerprint(prompt.view),
        options: prompt.options.to_vec(),
        selected,
    }
}

fn player_view_fingerprint(view: &PlayerView) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in format!("{view:?}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

enum TraceMode {
    Off,
    Record(Vec<TraceRecord>),
    Verify {
        expected: Vec<TraceRecord>,
        cursor: usize,
    },
}

impl TraceMode {
    const fn enabled(&self) -> bool {
        !matches!(self, Self::Off)
    }

    fn accept(&mut self, actual: TraceRecord) -> Result<(), String> {
        match self {
            Self::Off => Ok(()),
            Self::Record(records) => {
                records.push(actual);
                Ok(())
            }
            Self::Verify { expected, cursor } => {
                let Some(wanted) = expected.get(*cursor) else {
                    return Err(format!(
                        "replay emitted unexpected action {}: {}",
                        actual.index, actual.action
                    ));
                };
                if wanted != &actual {
                    return Err(format!(
                        "replay diverged at action {}: expected {wanted:?}, got {actual:?}",
                        actual.index
                    ));
                }
                *cursor = cursor.saturating_add(1);
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<Option<Vec<TraceRecord>>, String> {
        match self {
            Self::Off => Ok(None),
            Self::Record(records) => Ok(Some(records)),
            Self::Verify { expected, cursor } if cursor == expected.len() => Ok(None),
            Self::Verify { expected, cursor } => Err(format!(
                "replay stopped after {cursor} of {} recorded actions",
                expected.len()
            )),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct GameMetrics {
    actions: u64,
    casts: u64,
    commander_casts: u64,
    taxed_commander_recasts: u64,
    commander_zone_returns: u64,
    lands_played: u64,
    mana_abilities: u64,
    priority_passes: u64,
    triggers_registered: u64,
    triggers_resolved: u64,
    interpreter_actions: u64,
    combat_declarations: u64,
    combat_damage_events: u64,
    eliminations: u64,
    invariant_checks: u64,
    hidden_information_checks: u64,
    identity_exercise: BTreeMap<String, IdentityExercise>,
}

impl GameMetrics {
    fn add_assign(&mut self, other: &Self) {
        self.actions += other.actions;
        self.casts += other.casts;
        self.commander_casts += other.commander_casts;
        self.taxed_commander_recasts += other.taxed_commander_recasts;
        self.commander_zone_returns += other.commander_zone_returns;
        self.lands_played += other.lands_played;
        self.mana_abilities += other.mana_abilities;
        self.priority_passes += other.priority_passes;
        self.triggers_registered += other.triggers_registered;
        self.triggers_resolved += other.triggers_resolved;
        self.interpreter_actions += other.interpreter_actions;
        self.combat_declarations += other.combat_declarations;
        self.combat_damage_events += other.combat_damage_events;
        self.eliminations += other.eliminations;
        self.invariant_checks += other.invariant_checks;
        self.hidden_information_checks += other.hidden_information_checks;
        for (identity, exercise) in &other.identity_exercise {
            self.identity_exercise
                .entry(identity.clone())
                .or_default()
                .add_assign(exercise);
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GameSummary {
    seed: u64,
    winner: usize,
    turns: u32,
    final_hash: u64,
    final_life: [i32; PLAYER_COUNT],
    metrics: GameMetrics,
}

struct GameRun {
    summary: GameSummary,
    trace: Option<Vec<TraceRecord>>,
    actions: Vec<Action>,
}

struct CampaignResult {
    summaries: Vec<GameSummary>,
    replays: Vec<PodReplay>,
    primary_worker_ms: u128,
    replay_worker_ms: u128,
}

#[derive(Clone)]
struct TriggerRuntime {
    program: Arc<CardProgram>,
    ability_index: usize,
    source: ObjectId,
}

#[derive(Clone, Copy)]
enum HumanMainChoice {
    PlayLand(ObjectId),
    ActivateAll,
    Activate {
        source: ObjectId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
    },
    Cast {
        object: ObjectId,
        payment: PaymentPlan,
    },
    Finish,
}

struct GameDriver {
    state: GameState,
    players: Vec<PlayerId>,
    programs: HashMap<ObjectId, Arc<CardProgram>>,
    commanders: Vec<ObjectId>,
    trigger_programs: HashMap<TriggerId, TriggerRuntime>,
    mana_abilities: Vec<(ObjectId, PlayerId, ActivatedAbilityId)>,
    triggers_registered_for: HashSet<ObjectId>,
    permanent_runtime_registered_for: HashSet<ObjectId>,
    commander_zone_decisions: HashMap<ObjectId, ZoneId>,
    current_attacks: Vec<AttackDeclaration>,
    current_defender: Option<PlayerId>,
    coverage_target: Option<String>,
    metrics: GameMetrics,
    trace: TraceMode,
    actions: Vec<Action>,
    next_hidden_check_action: u64,
    next_invariant_check_action: u64,
    seed: u64,
}

impl GameDriver {
    fn setup(
        pod: &PodTemplate,
        seed: u64,
        coverage_target: Option<String>,
        trace: TraceMode,
    ) -> Result<Self, String> {
        let mut driver = Self {
            state: GameState::new(),
            players: Vec::with_capacity(PLAYER_COUNT),
            programs: HashMap::new(),
            commanders: Vec::with_capacity(PLAYER_COUNT),
            trigger_programs: HashMap::new(),
            mana_abilities: Vec::new(),
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            current_defender: None,
            coverage_target,
            metrics: GameMetrics::default(),
            trace,
            actions: Vec::new(),
            next_hidden_check_action: 32,
            next_invariant_check_action: 64,
            seed,
        };
        driver.dispatch(Action::SetSeed { seed })?;
        for _ in 0..PLAYER_COUNT {
            let outcome = driver.dispatch(Action::AddPlayer)?;
            let Outcome::PlayerAdded(player) = outcome else {
                return Err(format!("seed {seed}: AddPlayer returned {outcome:?}"));
            };
            driver.players.push(player);
            driver.dispatch(Action::SetPlayerLife {
                player,
                life: STARTING_LIFE,
            })?;
        }
        let outcome = driver.dispatch(Action::DecideTurnOrder)?;
        let Outcome::TurnOrderDecided(starting_player) = outcome else {
            return Err(format!("seed {seed}: DecideTurnOrder returned {outcome:?}"));
        };

        let mut next_card_id = 1_u32;
        for (seat, deck) in pod.decks.iter().enumerate() {
            let player = driver.players[seat];
            let commander = driver.create_card_object(
                player,
                ZoneId::new(None, ZoneKind::Command),
                &deck.commander,
                CardId::new(next_card_id),
            )?;
            next_card_id = next_card_id.saturating_add(1);
            driver.dispatch(Action::DesignateCommander {
                object: commander,
                color_identity: deck.color_identity,
            })?;
            driver.commanders.push(commander);

            let mut deck_objects = vec![commander];
            for card in &deck.mainboard {
                let object = driver.create_card_object(
                    player,
                    ZoneId::new(Some(player), ZoneKind::Library),
                    card,
                    CardId::new(next_card_id),
                )?;
                next_card_id = next_card_id.saturating_add(1);
                deck_objects.push(object);
            }
            driver.dispatch(Action::ValidateCommanderColorIdentity {
                player,
                objects: deck_objects,
            })?;
            driver.dispatch(Action::ShuffleLibrary { player })?;
        }
        if let Some(target) = driver.coverage_target.as_deref() {
            let target_object = driver
                .programs
                .iter()
                .filter(|(_, program)| program.oracle_id() == target)
                .map(|(object, _)| *object)
                .filter(|object| {
                    driver.players.iter().any(|player| {
                        driver.state.object_zone(*object)
                            == Some(ZoneId::new(Some(*player), ZoneKind::Library))
                    })
                })
                .min_by_key(|object| object.index());
            if let Some(object) = target_object {
                let player = driver
                    .state
                    .object(object)
                    .ok_or_else(|| format!("seed {seed} missing coverage target object"))?
                    .owner();
                driver.dispatch(Action::PutObjectOnTopOfLibrary { player, object })?;
            }
        }
        driver.dispatch(Action::DrawOpeningHands)?;
        for player in driver.players.clone() {
            driver.dispatch(Action::KeepOpeningHand {
                player,
                bottom: Vec::new(),
            })?;
        }
        driver.check_hidden_information()?;
        driver.check_invariants()?;
        driver.dispatch(Action::StartTurn {
            active_player: starting_player,
        })?;
        Ok(driver)
    }

    fn create_card_object(
        &mut self,
        owner: PlayerId,
        zone: ZoneId,
        card: &CardAsset,
        card_id: CardId,
    ) -> Result<ObjectId, String> {
        let outcome = self.dispatch(Action::CreateObject {
            card: card_id,
            owner,
            controller: owner,
            zone,
        })?;
        let Outcome::ObjectCreated(object) = outcome else {
            return Err(format!(
                "seed {}: CreateObject returned {outcome:?}",
                self.seed
            ));
        };
        self.dispatch(Action::SetBaseObjectCharacteristics {
            object,
            base: card.program.base_object(),
        })?;
        if let Some(base) = card.program.base_creature() {
            self.dispatch(Action::SetBaseCreatureCharacteristics { object, base })?;
        }
        self.dispatch(Action::SetObjectColorIdentity {
            object,
            colors: card.color_identity,
        })?;
        self.programs.insert(object, Arc::clone(&card.program));
        Ok(object)
    }

    fn run(self, max_turns: u32) -> Result<GameRun, String> {
        self.run_controlled(max_turns, None, None)
    }

    fn run_human(
        self,
        max_turns: u32,
        human: PlayerId,
        decisions: &mut dyn DecisionSource,
    ) -> Result<GameRun, String> {
        self.run_controlled(max_turns, Some(human), Some(decisions))
    }

    fn run_controlled(
        mut self,
        max_turns: u32,
        human: Option<PlayerId>,
        mut decisions: Option<&mut dyn DecisionSource>,
    ) -> Result<GameRun, String> {
        let mut main_done = BTreeSet::<u32>::new();
        let mut attackers_done = BTreeSet::<u32>::new();
        let mut blockers_done = BTreeSet::<u32>::new();
        let mut damage_done = BTreeSet::<(u32, CombatDamageStepKind)>::new();

        while self.state.game_outcome() == GameOutcome::InProgress {
            let turn = self.state.turn_number();
            if turn > max_turns {
                let life = self
                    .state
                    .players()
                    .iter()
                    .map(|player| player.life())
                    .collect::<Vec<_>>();
                let commander_casts = self
                    .commanders
                    .iter()
                    .map(|object| {
                        self.state
                            .object(*object)
                            .map(|record| record.commander_cast_count())
                            .unwrap_or(0)
                    })
                    .collect::<Vec<_>>();
                return Err(format!(
                    "seed {} exceeded {max_turns} turns without a winner; life={life:?}; \
                     commander_casts={commander_casts:?}; metrics={:?}",
                    self.seed, self.metrics
                ));
            }
            if !self.state.pending_triggers().is_empty() {
                let outcome = self.dispatch(Action::PutPendingTriggeredAbilitiesOnStack)?;
                if !matches!(outcome, Outcome::StackEntriesAdded(_)) {
                    return Err(format!(
                        "seed {}: pending trigger placement returned {outcome:?}",
                        self.seed
                    ));
                }
                continue;
            }

            let Some(step) = self.state.current_step() else {
                return Err(format!("seed {} has no active step", self.seed));
            };
            let active = self
                .state
                .active_player()
                .ok_or_else(|| format!("seed {} has no active player", self.seed))?;

            if self.state.priority_player().is_none() {
                self.dispatch(Action::AdvanceStep)?;
                continue;
            }

            let active_has_priority = self.state.priority_player() == Some(active);
            match step {
                Step::PrecombatMain if active_has_priority && main_done.insert(turn) => {
                    self.check_hidden_information()?;
                    if human == Some(active) {
                        let source = decisions
                            .as_deref_mut()
                            .ok_or_else(|| "human game is missing a decision source".to_owned())?;
                        self.take_human_main_phase_actions(active, source)?;
                    } else {
                        self.take_main_phase_actions(active)?;
                    }
                }
                Step::DeclareAttackers if active_has_priority && attackers_done.insert(turn) => {
                    self.check_hidden_information()?;
                    if human == Some(active) {
                        let source = decisions
                            .as_deref_mut()
                            .ok_or_else(|| "human game is missing a decision source".to_owned())?;
                        self.declare_human_attackers(active, source)?;
                    } else {
                        self.declare_attackers(active)?;
                    }
                }
                Step::DeclareBlockers if active_has_priority && blockers_done.insert(turn) => {
                    self.check_hidden_information()?;
                    let defender = self.current_defending_player(active);
                    if defender.is_some() && defender == human {
                        let source = decisions
                            .as_deref_mut()
                            .ok_or_else(|| "human game is missing a decision source".to_owned())?;
                        self.declare_human_blocks(active, source)?;
                    } else {
                        self.declare_blocks(active)?;
                    }
                }
                Step::CombatDamage if active_has_priority => {
                    let damage_step =
                        self.state.combat_state().damage_step().ok_or_else(|| {
                            format!("seed {} missing combat damage step", self.seed)
                        })?;
                    if damage_done.insert((turn, damage_step)) {
                        self.check_hidden_information()?;
                        self.assign_combat_damage()?;
                        if let Some(human) = human {
                            let source = decisions.as_deref_mut().ok_or_else(|| {
                                "human game is missing a decision source".to_owned()
                            })?;
                            self.choose_dead_commanders_with_human(human, source)?;
                        } else {
                            self.choose_dead_commanders()?;
                        }
                    } else {
                        self.pass_priority()?;
                    }
                }
                _ => self.pass_priority()?,
            }

            while self.metrics.actions >= self.next_hidden_check_action {
                self.check_hidden_information()?;
                self.next_hidden_check_action = self.next_hidden_check_action.saturating_add(32);
            }
            while self.metrics.actions >= self.next_invariant_check_action {
                self.check_invariants()?;
                self.next_invariant_check_action =
                    self.next_invariant_check_action.saturating_add(64);
            }
        }

        self.check_invariants()?;
        self.check_hidden_information()?;
        let GameOutcome::Won(winner) = self.state.game_outcome() else {
            return Err(format!(
                "seed {} ended without exactly one winner",
                self.seed
            ));
        };
        let winner = self
            .players
            .iter()
            .position(|player| *player == winner)
            .ok_or_else(|| format!("seed {} winner is outside the pod", self.seed))?;
        let final_life = std::array::from_fn(|index| self.state.players()[index].life());
        let summary = GameSummary {
            seed: self.seed,
            winner,
            turns: self.state.turn_number(),
            final_hash: self.state.deterministic_hash().get(),
            final_life,
            metrics: self.metrics,
        };
        let trace = self.trace.finish()?;
        Ok(GameRun {
            summary,
            trace,
            actions: self.actions,
        })
    }

    fn dispatch(&mut self, action: Action) -> Result<Outcome, String> {
        let trace_header = self
            .trace
            .enabled()
            .then(|| (format!("{action:?}"), self.state.deterministic_hash().get()));
        let lost_before = self
            .state
            .players()
            .iter()
            .filter(|player| player.lost())
            .count();
        self.actions.push(action.clone());
        let outcome = apply(&mut self.state, action);
        if let Some((action, before_hash)) = trace_header {
            let trace_record = TraceRecord {
                index: self.metrics.actions,
                action,
                before_hash,
                outcome: format!("{outcome:?}"),
                after_hash: self.state.deterministic_hash().get(),
            };
            self.trace
                .accept(trace_record)
                .map_err(|error| format!("seed {}: {error}", self.seed))?;
        }
        self.metrics.actions = self.metrics.actions.saturating_add(1);
        if let Outcome::Failed(error) = &outcome {
            return Err(format!(
                "seed {}: kernel rejected action: {error:?}",
                self.seed
            ));
        }
        let lost_after = self
            .state
            .players()
            .iter()
            .filter(|player| player.lost())
            .count();
        self.metrics.eliminations = self
            .metrics
            .eliminations
            .saturating_add(lost_after.saturating_sub(lost_before) as u64);
        Ok(outcome)
    }

    fn identity_exercise_mut(&mut self, object: ObjectId) -> Option<&mut IdentityExercise> {
        let oracle_id = self.programs.get(&object)?.oracle_id().to_owned();
        Some(self.metrics.identity_exercise.entry(oracle_id).or_default())
    }

    fn take_main_phase_actions(&mut self, player: PlayerId) -> Result<(), String> {
        self.play_land(player)?;
        self.activate_mana_sources(player)?;
        self.cast_one_permanent(player)
    }

    fn prompt_choice(
        &self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        options: &[String],
    ) -> Result<usize, String> {
        let view = self
            .state
            .player_view(player)
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        let selected = source.choose(&DecisionPrompt {
            kind,
            view: &view,
            options,
        })?;
        if selected >= options.len() {
            return Err(format!(
                "seed {} prompt `{kind}` returned option {selected} outside {} choices",
                self.seed,
                options.len()
            ));
        }
        Ok(selected)
    }

    fn take_human_main_phase_actions(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        loop {
            let (labels, choices) = self.human_main_choices(player)?;
            let selected =
                self.prompt_choice(player, source, "Choose a main-phase action", &labels)?;
            match choices[selected] {
                HumanMainChoice::PlayLand(object) => {
                    self.dispatch(Action::PlayLand { player, object })?;
                    self.metrics.lands_played = self.metrics.lands_played.saturating_add(1);
                    if let Some(exercise) = self.identity_exercise_mut(object) {
                        exercise.land_plays = exercise.land_plays.saturating_add(1);
                    }
                    self.register_permanent_runtime(player, object)?;
                }
                HumanMainChoice::ActivateAll => {
                    self.activate_mana_sources(player)?;
                }
                HumanMainChoice::Activate {
                    source: ability_source,
                    ability,
                    payment,
                } => {
                    self.dispatch(Action::ActivateAbility {
                        player,
                        ability,
                        payment,
                    })?;
                    self.metrics.mana_abilities = self.metrics.mana_abilities.saturating_add(1);
                    if !self.state.stack_entries().is_empty()
                        || self.state.priority_player() != Some(player)
                    {
                        return Ok(());
                    }
                    if self.state.object(ability_source).is_none() {
                        return Err(format!(
                            "seed {} activated source disappeared without a stack transition",
                            self.seed
                        ));
                    }
                }
                HumanMainChoice::Cast { object, payment } => {
                    self.cast_permanent_with_payment(player, object, payment)?;
                    return Ok(());
                }
                HumanMainChoice::Finish => return Ok(()),
            }
        }
    }

    fn human_main_choices(
        &self,
        player: PlayerId,
    ) -> Result<(Vec<String>, Vec<HumanMainChoice>), String> {
        let mut labels = Vec::new();
        let mut choices = Vec::new();
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let mut hand_objects = self
            .state
            .zone_objects(hand)
            .ok_or_else(|| format!("seed {} missing hand zone", self.seed))?
            .to_vec();
        hand_objects.sort_by_key(|object| object.index());

        if self.state.players()[player.index()].lands_played_this_turn() == 0 {
            let mut seen_land_identities = BTreeSet::new();
            for object in hand_objects.iter().copied() {
                let Some(program) = self.programs.get(&object) else {
                    continue;
                };
                if program.kind() != ProgramKind::Land
                    || !seen_land_identities.insert(program.oracle_id().to_owned())
                {
                    continue;
                }
                let action = Action::PlayLand { player, object };
                if self.action_is_legal(&action) {
                    labels.push(format!("Play land: {}", self.object_name(object)));
                    choices.push(HumanMainChoice::PlayLand(object));
                }
            }
        }

        let mut activations = Vec::new();
        for (ability_source, controller, ability) in self.mana_abilities.iter().copied() {
            if controller != player
                || self.state.object_zone(ability_source)
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
                || self
                    .state
                    .object(ability_source)
                    .map_or(true, |record| record.tapped())
            {
                continue;
            }
            let cost = self
                .state
                .effective_activation_cost(ability)
                .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
            let Some(payment) = self
                .state
                .payment_plans_for_player(player, cost.mana())
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?
                .best()
            else {
                continue;
            };
            let action = Action::ActivateAbility {
                player,
                ability,
                payment,
            };
            if self.action_is_legal(&action) {
                activations.push((ability_source, ability, payment));
            }
        }
        if !activations.is_empty() {
            labels.push("Activate all available mana sources".to_owned());
            choices.push(HumanMainChoice::ActivateAll);
            for (ability_source, ability, payment) in activations {
                labels.push(format!(
                    "Activate ability: {}",
                    self.object_name(ability_source)
                ));
                choices.push(HumanMainChoice::Activate {
                    source: ability_source,
                    ability,
                    payment,
                });
            }
        }

        let seat = self
            .players
            .iter()
            .position(|candidate| *candidate == player)
            .ok_or_else(|| format!("seed {} unknown active player", self.seed))?;
        let commander = self.commanders[seat];
        let mut cast_candidates = Vec::new();
        if self.state.object_zone(commander) == Some(ZoneId::new(None, ZoneKind::Command)) {
            cast_candidates.push(commander);
        }
        cast_candidates.extend(hand_objects);
        for object in cast_candidates {
            let Some(program) = self.programs.get(&object) else {
                continue;
            };
            if program.kind() != ProgramKind::Permanent
                || !program.target_requirements().is_empty()
                || !program.object_choice_requirements().is_empty()
                || !program.spell_modes().is_empty()
                || program.optional_choice_count() != 0
            {
                continue;
            }
            let cost = self
                .state
                .effective_spell_cost(player, object, program.mana_cost())
                .map_err(|error| format!("seed {} spell cost failed: {error:?}", self.seed))?;
            let Some(payment) = self
                .state
                .payment_plans_for_player(player, cost)
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?
                .best()
            else {
                continue;
            };
            let action = Action::CastSpell {
                player,
                object,
                request: CastSpellRequest::new(
                    StackObjectKind::PermanentSpell,
                    SpellTiming::Sorcery,
                    program.mana_cost(),
                    payment,
                ),
            };
            if self.action_is_legal(&action) {
                labels.push(format!("Cast: {}", program.name()));
                choices.push(HumanMainChoice::Cast { object, payment });
            }
        }

        labels.push("Finish main phase".to_owned());
        choices.push(HumanMainChoice::Finish);
        Ok((labels, choices))
    }

    fn object_name(&self, object: ObjectId) -> String {
        self.programs.get(&object).map_or_else(
            || format!("object {}", object.index()),
            |program| program.name().to_owned(),
        )
    }

    fn action_is_legal(&self, action: &Action) -> bool {
        let mut state = self.state.clone();
        !matches!(apply(&mut state, action.clone()), Outcome::Failed(_))
    }

    fn play_land(&mut self, player: PlayerId) -> Result<(), String> {
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let hand_objects = self
            .state
            .zone_objects(hand)
            .ok_or_else(|| format!("seed {} missing hand zone", self.seed))?
            .to_vec();
        let land = hand_objects
            .iter()
            .copied()
            .filter_map(|object| self.programs.get(&object).map(|program| (object, program)))
            .filter(|(_, program)| program.kind() == ProgramKind::Land)
            .min_by_key(|(object, program)| {
                let prior_plays = self
                    .metrics
                    .identity_exercise
                    .get(program.oracle_id())
                    .map_or(0, |exercise| exercise.land_plays);
                (
                    self.coverage_target.as_deref() != Some(program.oracle_id()),
                    program.activated_abilities().is_empty(),
                    prior_plays,
                    object.index(),
                )
            });
        let Some((object, _)) = land else {
            return Ok(());
        };
        self.dispatch(Action::PlayLand { player, object })?;
        self.metrics.lands_played = self.metrics.lands_played.saturating_add(1);
        if let Some(exercise) = self.identity_exercise_mut(object) {
            exercise.land_plays = exercise.land_plays.saturating_add(1);
        }
        self.register_permanent_runtime(player, object)
    }

    fn activate_mana_sources(&mut self, player: PlayerId) -> Result<(), String> {
        let abilities = self.mana_abilities.clone();
        for (source, controller, ability) in abilities {
            if controller != player
                || self.state.object_zone(source) != Some(ZoneId::new(None, ZoneKind::Battlefield))
                || self
                    .state
                    .object(source)
                    .map_or(true, |record| record.tapped())
            {
                continue;
            }
            let cost = self
                .state
                .effective_activation_cost(ability)
                .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
            let Some(payment) = self
                .state
                .payment_plans_for_player(player, cost.mana())
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?
                .best()
            else {
                continue;
            };
            self.dispatch(Action::ActivateAbility {
                player,
                ability,
                payment,
            })?;
            self.metrics.mana_abilities = self.metrics.mana_abilities.saturating_add(1);
        }
        Ok(())
    }

    fn cast_one_permanent(&mut self, player: PlayerId) -> Result<(), String> {
        let seat = self
            .players
            .iter()
            .position(|candidate| *candidate == player)
            .ok_or_else(|| format!("seed {} unknown active player", self.seed))?;
        let commander = self.commanders[seat];
        let mut candidates = Vec::new();
        if self.state.object_zone(commander) == Some(ZoneId::new(None, ZoneKind::Command)) {
            candidates.push(commander);
        }
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let mut hand_objects = self
            .state
            .zone_objects(hand)
            .ok_or_else(|| format!("seed {} missing hand zone", self.seed))?
            .to_vec();
        hand_objects.sort_by_key(|object| {
            let program = self.programs.get(object);
            let coverage_rank = match program {
                Some(program) => self.coverage_target.as_deref() != Some(program.oracle_id()),
                None => true,
            };
            let trigger_rank =
                program.is_some_and(|program| !program.triggered_abilities().is_empty());
            let creature_rank =
                program.is_some_and(|program| program.base_object().types().creature());
            (coverage_rank, !trigger_rank, !creature_rank, object.index())
        });
        candidates.extend(hand_objects);

        for object in candidates {
            let Some(program) = self.programs.get(&object).cloned() else {
                continue;
            };
            if program.kind() != ProgramKind::Permanent
                || !program.target_requirements().is_empty()
                || !program.object_choice_requirements().is_empty()
                || !program.spell_modes().is_empty()
                || program.optional_choice_count() != 0
            {
                continue;
            }
            let cost = self
                .state
                .effective_spell_cost(player, object, program.mana_cost())
                .map_err(|error| format!("seed {} spell cost failed: {error:?}", self.seed))?;
            let Some(payment) = self
                .state
                .payment_plans_for_player(player, cost)
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?
                .best()
            else {
                continue;
            };
            return self.cast_permanent_with_payment(player, object, payment);
        }
        Ok(())
    }

    fn cast_permanent_with_payment(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        payment: PaymentPlan,
    ) -> Result<(), String> {
        let program = self
            .programs
            .get(&object)
            .cloned()
            .ok_or_else(|| format!("seed {} missing program for cast object", self.seed))?;
        let request = CastSpellRequest::new(
            StackObjectKind::PermanentSpell,
            SpellTiming::Sorcery,
            program.mana_cost(),
            payment,
        );
        let was_commander = self.commanders.contains(&object);
        self.dispatch(Action::CastSpell {
            player,
            object,
            request,
        })?;
        self.metrics.casts = self.metrics.casts.saturating_add(1);
        if let Some(exercise) = self.identity_exercise_mut(object) {
            exercise.casts = exercise.casts.saturating_add(1);
        }
        if was_commander {
            self.metrics.commander_casts = self.metrics.commander_casts.saturating_add(1);
            if self
                .state
                .object(object)
                .is_some_and(|record| record.commander_cast_count() >= 2)
            {
                self.metrics.taxed_commander_recasts =
                    self.metrics.taxed_commander_recasts.saturating_add(1);
            }
        }
        self.register_triggers(player, object, &program)
    }

    fn register_triggers(
        &mut self,
        controller: PlayerId,
        source: ObjectId,
        program: &Arc<CardProgram>,
    ) -> Result<(), String> {
        if !self.triggers_registered_for.insert(source) {
            return Ok(());
        }
        for (ability_index, ability) in program.triggered_abilities().iter().enumerate() {
            let outcome = self.dispatch(Action::RegisterTriggeredAbility {
                definition: ability.bind(controller, source),
            })?;
            let Outcome::TriggerRegistered(trigger) = outcome else {
                return Err(format!(
                    "seed {} trigger registration returned {outcome:?}",
                    self.seed
                ));
            };
            self.trigger_programs.insert(
                trigger,
                TriggerRuntime {
                    program: Arc::clone(program),
                    ability_index,
                    source,
                },
            );
            self.metrics.triggers_registered = self.metrics.triggers_registered.saturating_add(1);
        }
        Ok(())
    }

    fn register_permanent_runtime(
        &mut self,
        controller: PlayerId,
        source: ObjectId,
    ) -> Result<(), String> {
        if !self.permanent_runtime_registered_for.insert(source) {
            return Ok(());
        }
        let program = self
            .programs
            .get(&source)
            .cloned()
            .ok_or_else(|| format!("seed {} missing program for permanent", self.seed))?;
        for static_ability in program.static_abilities() {
            for action in static_ability.bind_actions(controller, source) {
                self.dispatch(action)?;
                self.metrics.interpreter_actions =
                    self.metrics.interpreter_actions.saturating_add(1);
            }
        }
        for ability in program.activated_abilities() {
            if ability.condition().is_some() {
                continue;
            }
            let outcome = self.dispatch(Action::RegisterActivatedAbility {
                definition: ability.bind(controller, source),
            })?;
            let Outcome::ActivatedAbilityRegistered(ability_id) = outcome else {
                return Err(format!(
                    "seed {} mana registration returned {outcome:?}",
                    self.seed
                ));
            };
            self.mana_abilities.push((source, controller, ability_id));
        }
        Ok(())
    }

    fn pass_priority(&mut self) -> Result<(), String> {
        let player = self
            .state
            .priority_player()
            .ok_or_else(|| format!("seed {} cannot pass without priority", self.seed))?;
        let outcome = self.dispatch(Action::PassPriority { player })?;
        self.metrics.priority_passes = self.metrics.priority_passes.saturating_add(1);
        if let Outcome::Priority(PriorityOutcome::Resolved(entry)) = outcome {
            self.handle_resolution(entry)?;
        }
        Ok(())
    }

    fn handle_resolution(&mut self, entry: StackEntryId) -> Result<(), String> {
        let record = self
            .state
            .resolution_log()
            .iter()
            .find(|record| record.stack_entry() == entry)
            .ok_or_else(|| format!("seed {} missing resolution record {entry:?}", self.seed))?;
        let controller = record.controller();
        let object = record.object();
        let trigger = record.trigger();
        if let Some(trigger) = trigger {
            return self.execute_trigger(controller, trigger);
        }
        let Some(object) = object else {
            return Ok(());
        };
        let program = self
            .programs
            .get(&object)
            .cloned()
            .ok_or_else(|| format!("seed {} missing program for resolved spell", self.seed))?;
        if let Some(exercise) = self.identity_exercise_mut(object) {
            exercise.resolutions = exercise.resolutions.saturating_add(1);
        }
        if program.kind() == ProgramKind::Permanent
            && self.state.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
        {
            self.register_permanent_runtime(controller, object)?;
        }
        if !program.effects().is_empty() {
            let bindings = ExecutionBindings::new(controller, self.live_opponents(controller))
                .with_source(object);
            let trace = forge_cards::runtime::execute_program(&mut self.state, &program, &bindings)
                .map_err(|error| {
                    format!("seed {} interpreter execution failed: {error}", self.seed)
                })?;
            self.metrics.interpreter_actions = self
                .metrics
                .interpreter_actions
                .saturating_add(trace.records().len() as u64);
            if let Some(exercise) = self.identity_exercise_mut(object) {
                exercise.effect_actions = exercise
                    .effect_actions
                    .saturating_add(trace.records().len() as u64);
            }
        }
        Ok(())
    }

    fn execute_trigger(&mut self, controller: PlayerId, trigger: TriggerId) -> Result<(), String> {
        let runtime = self
            .trigger_programs
            .get(&trigger)
            .cloned()
            .ok_or_else(|| format!("seed {} missing runtime trigger {trigger:?}", self.seed))?;
        let ability = runtime
            .program
            .triggered_abilities()
            .get(runtime.ability_index)
            .ok_or_else(|| format!("seed {} missing trigger ability", self.seed))?;
        ensure_trigger_is_autonomous(ability, runtime.program.name())?;
        let mut bindings = ExecutionBindings::new(controller, self.live_opponents(controller))
            .with_source(runtime.source)
            .with_optional_effect_choices(vec![true; ability.optional_choice_count()]);
        if ability.unless_paid().is_some() {
            bindings = bindings.with_unless_payment(false);
        }
        let actions = bind_triggered_ability_actions(&self.state, ability, &bindings)
            .map_err(|error| format!("seed {} trigger binding failed: {error}", self.seed))?;
        for action in actions {
            self.dispatch(action.action().clone())?;
            self.metrics.interpreter_actions = self.metrics.interpreter_actions.saturating_add(1);
        }
        self.metrics.triggers_resolved = self.metrics.triggers_resolved.saturating_add(1);
        if let Some(exercise) = self.identity_exercise_mut(runtime.source) {
            exercise.trigger_resolutions = exercise.trigger_resolutions.saturating_add(1);
        }
        Ok(())
    }

    fn declare_human_attackers(
        &mut self,
        active: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let objects = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(active))
            .collect::<Vec<_>>();
        let mut labels = Vec::new();
        let mut candidates = Vec::<Vec<AttackDeclaration>>::new();
        let mut seen = BTreeSet::new();
        for defender in self.live_opponents(active) {
            let legal = objects
                .iter()
                .copied()
                .map(|object| AttackDeclaration::new(object, defender))
                .filter(|attack| self.state.can_attack(active, *attack))
                .collect::<Vec<_>>();
            if legal.is_empty() {
                continue;
            }
            let mut variants = Vec::with_capacity(legal.len() + 1);
            variants.push(legal.clone());
            variants.extend(legal.iter().copied().map(|attack| vec![attack]));
            for attacks in variants {
                let key = format!("{attacks:?}");
                let action = Action::DeclareAttackers {
                    player: active,
                    attacks: attacks.clone(),
                };
                if !seen.insert(key) || !self.action_is_legal(&action) {
                    continue;
                }
                let names = attacks
                    .iter()
                    .map(|attack| self.object_name(attack.attacker()))
                    .collect::<Vec<_>>()
                    .join(", ");
                labels.push(format!("Attack seat {} with {names}", defender.index() + 1));
                candidates.push(attacks);
            }
        }
        let no_attacks = Vec::new();
        let no_attack_action = Action::DeclareAttackers {
            player: active,
            attacks: no_attacks.clone(),
        };
        if !self.action_is_legal(&no_attack_action) {
            return Err(format!(
                "seed {} kernel rejected the no-attack fallback",
                self.seed
            ));
        }
        labels.push("Attack no one".to_owned());
        candidates.push(no_attacks);
        let selected = self.prompt_choice(active, source, "Choose attackers", &labels)?;
        let attacks = candidates[selected].clone();
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
        self.current_defender = attacks.first().map(|attack| attack.defending_player());
        self.current_attacks = attacks;
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn declare_attackers(&mut self, active: PlayerId) -> Result<(), String> {
        let seat = self
            .players
            .iter()
            .position(|player| *player == active)
            .ok_or_else(|| format!("seed {} unknown active player", self.seed))?;
        let commander = self.commanders[seat];
        let commander_record = self.state.object(commander);
        let kill_defender = commander_record
            .filter(|record| {
                record.commander_cast_count() == 1
                    && self.state.object_zone(commander)
                        == Some(ZoneId::new(None, ZoneKind::Battlefield))
            })
            .and_then(|_| self.commander_kill_defender(active, commander));
        let Some(defender) = kill_defender.or_else(|| self.next_live_opponent(active)) else {
            return Ok(());
        };
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let objects = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .to_vec();
        let commander_may_attack = commander_record
            .is_some_and(|record| record.commander_cast_count() >= 2 || kill_defender.is_some());
        let attacks = objects
            .into_iter()
            .filter(|object| *object != commander || commander_may_attack)
            .map(|object| AttackDeclaration::new(object, defender))
            .filter(|attack| self.state.can_attack(active, *attack))
            .collect::<Vec<_>>();
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
        self.current_attacks = attacks;
        self.current_defender = Some(defender);
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn commander_kill_defender(&self, active: PlayerId, commander: ObjectId) -> Option<PlayerId> {
        let attacker = self.state.creature_characteristics(commander).ok()?;
        let battlefield = self
            .state
            .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))?;
        self.players.iter().copied().find(|defender| {
            if *defender == active || self.state.players()[defender.index()].lost() {
                return false;
            }
            let attack = AttackDeclaration::new(commander, *defender);
            if !self.state.can_attack(active, attack) {
                return false;
            }
            battlefield.iter().copied().any(|blocker| {
                let Some(record) = self.state.object(blocker) else {
                    return false;
                };
                let Ok(controller) = self.state.object_controller(blocker) else {
                    return false;
                };
                let Ok(characteristics) = self.state.creature_characteristics(blocker) else {
                    return false;
                };
                let evasion_ok = !attacker.keywords().flying()
                    || characteristics.keywords().flying()
                    || characteristics.keywords().reach();
                controller == *defender
                    && !record.tapped()
                    && evasion_ok
                    && (characteristics.power() >= attacker.toughness()
                        || characteristics.keywords().deathtouch())
            })
        })
    }

    fn current_defending_player(&self, active: PlayerId) -> Option<PlayerId> {
        self.current_defender.or_else(|| {
            self.current_attacks
                .first()
                .map(|attack| attack.defending_player())
                .or_else(|| self.next_live_opponent(active))
        })
    }

    fn declare_human_blocks(
        &mut self,
        active: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        let defending_player = self
            .current_defending_player(active)
            .ok_or_else(|| format!("seed {} missing human defending player", self.seed))?;
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let blockers = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(defending_player))
            .collect::<Vec<_>>();
        let mut labels = Vec::new();
        let mut candidates = Vec::<Vec<BlockDeclaration>>::new();
        let mut seen = BTreeSet::new();

        for blocker in blockers.iter().copied() {
            for attack in &self.current_attacks {
                let blocks = vec![BlockDeclaration::new(blocker, attack.attacker())];
                let action = Action::DeclareBlockers {
                    defending_player,
                    blocks: blocks.clone(),
                };
                let key = format!("{blocks:?}");
                if seen.insert(key) && self.action_is_legal(&action) {
                    labels.push(format!(
                        "Block {} with {}",
                        self.object_name(attack.attacker()),
                        self.object_name(blocker)
                    ));
                    candidates.push(blocks);
                }
            }
        }

        let mut greedy = Vec::new();
        for blocker in blockers {
            if let Some(attack) = self.current_attacks.iter().find(|attack| {
                self.state.can_block(
                    defending_player,
                    BlockDeclaration::new(blocker, attack.attacker()),
                )
            }) {
                greedy.push(BlockDeclaration::new(blocker, attack.attacker()));
            }
        }
        if greedy.len() > 1 {
            let action = Action::DeclareBlockers {
                defending_player,
                blocks: greedy.clone(),
            };
            let key = format!("{greedy:?}");
            if seen.insert(key) && self.action_is_legal(&action) {
                labels.insert(0, "Use all available blockers".to_owned());
                candidates.insert(0, greedy);
            }
        }

        let no_blocks = Vec::new();
        let no_block_action = Action::DeclareBlockers {
            defending_player,
            blocks: no_blocks.clone(),
        };
        if !self.action_is_legal(&no_block_action) {
            return Err(format!(
                "seed {} kernel rejected the no-block fallback",
                self.seed
            ));
        }
        labels.push("Block no attackers".to_owned());
        candidates.push(no_blocks);
        let selected = self.prompt_choice(defending_player, source, "Choose blockers", &labels)?;
        self.dispatch(Action::DeclareBlockers {
            defending_player,
            blocks: candidates[selected].clone(),
        })?;
        Ok(())
    }

    fn declare_blocks(&mut self, active: PlayerId) -> Result<(), String> {
        let defender = self.current_defending_player(active);
        if let Some(defending_player) = defender {
            if self.metrics.commander_zone_returns > 0 {
                self.dispatch(Action::DeclareBlockers {
                    defending_player,
                    blocks: Vec::new(),
                })?;
                return Ok(());
            }
            let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
            let mut blockers = self
                .state
                .zone_objects(battlefield)
                .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
                .iter()
                .copied()
                .filter(|object| self.state.object_controller(*object) == Ok(defending_player))
                .collect::<Vec<_>>();
            blockers.sort_by_key(|object| (!self.commanders.contains(object), object.index()));
            let mut attacks = self.current_attacks.clone();
            attacks.sort_by_key(|attack| {
                (
                    !self.commanders.contains(&attack.attacker()),
                    attack.attacker().index(),
                )
            });
            let mut blocks = Vec::new();
            'attacks: for attack in attacks {
                if !self.commanders.contains(&attack.attacker()) {
                    continue;
                }
                let Ok(attacker) = self.state.creature_characteristics(attack.attacker()) else {
                    continue;
                };
                for blocker in &blockers {
                    let Ok(characteristics) = self.state.creature_characteristics(*blocker) else {
                        continue;
                    };
                    let declaration = BlockDeclaration::new(*blocker, attack.attacker());
                    if self.state.can_block(defending_player, declaration)
                        && (characteristics.power() >= attacker.toughness()
                            || characteristics.keywords().deathtouch())
                    {
                        blocks.push(declaration);
                        break 'attacks;
                    }
                }
            }
            self.dispatch(Action::DeclareBlockers {
                defending_player,
                blocks: blocks.clone(),
            })?;
        }
        Ok(())
    }

    fn assign_combat_damage(&mut self) -> Result<(), String> {
        let combat = self.state.combat_state().clone();
        let step = combat
            .damage_step()
            .ok_or_else(|| format!("seed {} missing combat damage step", self.seed))?;
        let mut assignments = Vec::new();
        for attack in combat.attackers() {
            if self.state.object_zone(attack.object())
                != Some(ZoneId::new(None, ZoneKind::Battlefield))
            {
                continue;
            }
            let characteristics = self
                .state
                .creature_characteristics(attack.object())
                .map_err(|error| {
                    format!(
                        "seed {} combat characteristics failed: {error:?}",
                        self.seed
                    )
                })?;
            if !damage_step_eligible(step, characteristics.keywords()) {
                continue;
            }
            let amount = u32::try_from(characteristics.power().max(0))
                .map_err(|error| format!("seed {} invalid combat power: {error}", self.seed))?;
            if amount == 0 {
                continue;
            }
            let active_blocker = attack.blockers().iter().copied().find(|blocker| {
                self.state.object_zone(*blocker) == Some(ZoneId::new(None, ZoneKind::Battlefield))
            });
            let target = if let Some(blocker) = active_blocker {
                CombatDamageTarget::Object(blocker)
            } else if !attack.blocked() || characteristics.keywords().trample() {
                CombatDamageTarget::Player(attack.defending_player())
            } else {
                continue;
            };
            assignments.push(CombatDamageAssignmentRequest::new(
                attack.object(),
                vec![CombatDamageAssignment::new(target, amount)],
            ));
        }
        for block in combat.blockers() {
            if self.state.object_zone(block.object())
                != Some(ZoneId::new(None, ZoneKind::Battlefield))
                || self.state.object_zone(block.attacker())
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
            {
                continue;
            }
            let characteristics = self
                .state
                .creature_characteristics(block.object())
                .map_err(|error| {
                    format!(
                        "seed {} blocker characteristics failed: {error:?}",
                        self.seed
                    )
                })?;
            if !damage_step_eligible(step, characteristics.keywords()) {
                continue;
            }
            let amount = u32::try_from(characteristics.power().max(0))
                .map_err(|error| format!("seed {} invalid blocker power: {error}", self.seed))?;
            if amount > 0 {
                assignments.push(CombatDamageAssignmentRequest::new(
                    block.object(),
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(block.attacker()),
                        amount,
                    )],
                ));
            }
        }
        let outcome = self.dispatch(Action::AssignCombatDamage { assignments })?;
        let Outcome::CombatDamageAssigned(records) = outcome else {
            return Err(format!(
                "seed {} combat assignment returned {outcome:?}",
                self.seed
            ));
        };
        self.metrics.combat_damage_events = self
            .metrics
            .combat_damage_events
            .saturating_add(records.len() as u64);
        Ok(())
    }

    fn choose_dead_commanders(&mut self) -> Result<(), String> {
        for (seat, commander) in self.commanders.clone().into_iter().enumerate() {
            let owner = self.players[seat];
            let zone = self.state.object_zone(commander);
            if zone != Some(ZoneId::new(Some(owner), ZoneKind::Graveyard))
                && zone != Some(ZoneId::new(None, ZoneKind::Exile))
            {
                self.commander_zone_decisions.remove(&commander);
                continue;
            }
            let zone = zone.ok_or_else(|| format!("seed {} missing commander zone", self.seed))?;
            if self.commander_zone_decisions.get(&commander) == Some(&zone) {
                continue;
            }
            self.commander_zone_decisions.insert(commander, zone);
            self.dispatch(Action::ChooseCommanderZone {
                player: owner,
                object: commander,
            })?;
            self.metrics.commander_zone_returns =
                self.metrics.commander_zone_returns.saturating_add(1);
        }
        Ok(())
    }

    fn choose_dead_commanders_with_human(
        &mut self,
        human: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        for (seat, commander) in self.commanders.clone().into_iter().enumerate() {
            let owner = self.players[seat];
            let zone = self.state.object_zone(commander);
            if zone != Some(ZoneId::new(Some(owner), ZoneKind::Graveyard))
                && zone != Some(ZoneId::new(None, ZoneKind::Exile))
            {
                self.commander_zone_decisions.remove(&commander);
                continue;
            }
            let zone = zone.ok_or_else(|| format!("seed {} missing commander zone", self.seed))?;
            if self.commander_zone_decisions.get(&commander) == Some(&zone) {
                continue;
            }
            self.commander_zone_decisions.insert(commander, zone);
            if owner == human {
                let labels = vec![
                    format!("Move {} to the command zone", self.object_name(commander)),
                    format!("Leave {} in {zone:?}", self.object_name(commander)),
                ];
                let selected = self.prompt_choice(
                    human,
                    source,
                    "Choose whether to move your commander",
                    &labels,
                )?;
                if selected == 1 {
                    continue;
                }
            }
            self.dispatch(Action::ChooseCommanderZone {
                player: owner,
                object: commander,
            })?;
            self.metrics.commander_zone_returns =
                self.metrics.commander_zone_returns.saturating_add(1);
        }
        Ok(())
    }

    fn next_live_opponent(&self, active: PlayerId) -> Option<PlayerId> {
        let start = self.players.iter().position(|player| *player == active)?;
        (1..PLAYER_COUNT).find_map(|offset| {
            let player = self.players[(start + offset) % PLAYER_COUNT];
            (!self.state.players()[player.index()].lost()).then_some(player)
        })
    }

    fn live_opponents(&self, controller: PlayerId) -> Vec<PlayerId> {
        self.players
            .iter()
            .copied()
            .filter(|player| *player != controller && !self.state.players()[player.index()].lost())
            .collect()
    }

    fn check_invariants(&mut self) -> Result<(), String> {
        self.state
            .validate_zone_conservation()
            .map_err(|error| format!("seed {} zone invariant failed: {error:?}", self.seed))?;
        self.metrics.invariant_checks = self.metrics.invariant_checks.saturating_add(1);
        Ok(())
    }

    fn check_hidden_information(&mut self) -> Result<(), String> {
        for observer in &self.players {
            let view = self
                .state
                .player_view(*observer)
                .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
            for owner in &self.players {
                let hand = view
                    .zone(ZoneId::new(Some(*owner), ZoneKind::Hand))
                    .ok_or_else(|| format!("seed {} visible hand missing", self.seed))?;
                let should_hide = owner != observer;
                if hand
                    .objects()
                    .iter()
                    .any(|object| object.is_hidden() != should_hide)
                {
                    return Err(format!(
                        "seed {} hidden-information canary failed for observer {} hand {}",
                        self.seed,
                        observer.index(),
                        owner.index()
                    ));
                }
                let library = view
                    .zone(ZoneId::new(Some(*owner), ZoneKind::Library))
                    .ok_or_else(|| format!("seed {} visible library missing", self.seed))?;
                if library
                    .objects()
                    .iter()
                    .any(|object| !matches!(object, ObjectView::Hidden))
                {
                    return Err(format!(
                        "seed {} library leaked to observer {}",
                        self.seed,
                        observer.index()
                    ));
                }
            }
            self.metrics.hidden_information_checks =
                self.metrics.hidden_information_checks.saturating_add(1);
        }
        Ok(())
    }
}

fn damage_step_eligible(
    step: CombatDamageStepKind,
    keywords: forge_core::CreatureKeywords,
) -> bool {
    match step {
        CombatDamageStepKind::Normal => true,
        CombatDamageStepKind::FirstStrike => keywords.first_strike() || keywords.double_strike(),
        CombatDamageStepKind::Regular => !keywords.first_strike() || keywords.double_strike(),
    }
}

fn ensure_trigger_is_autonomous(
    ability: &TriggeredAbilityProgram,
    card_name: &str,
) -> Result<(), String> {
    if !ability.target_requirements().is_empty() {
        return Err(format!(
            "trigger on {card_name} requires target prompts not supplied by this random-legal controller"
        ));
    }
    if !ability.object_choice_requirements().is_empty() {
        return Err(format!(
            "trigger on {card_name} requires hidden-zone choices not supplied by this controller"
        ));
    }
    Ok(())
}

fn run_campaign(
    pod: Arc<PodTemplate>,
    games: usize,
    jobs: usize,
    max_turns: u32,
    seed_base: u64,
    manifest: &Path,
) -> Result<CampaignResult, String> {
    let worker_count = jobs.min(games);
    let manifest = manifest.to_path_buf();
    let batches = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for worker in 0..worker_count {
            let pod = Arc::clone(&pod);
            let manifest = manifest.clone();
            handles.push(scope.spawn(move || {
                let mut batch = Vec::new();
                for index in (worker..games).step_by(worker_count) {
                    let seed = campaign_seed(seed_base, index);
                    let result = run_game_pair(&pod, &manifest, index, seed, max_turns);
                    batch.push((index, result));
                }
                batch
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().map_err(|_| "pod worker panicked".to_owned()))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut ordered = vec![None; games];
    for batch in batches {
        for (index, result) in batch {
            let result = result.map_err(|error| format!("game {index}: {error}"))?;
            ordered[index] = Some(result);
        }
    }
    let mut summaries = Vec::with_capacity(games);
    let mut replays = Vec::with_capacity(RETAINED_REPLAYS.min(games));
    let mut primary_worker_ms = 0_u128;
    let mut replay_worker_ms = 0_u128;
    for (index, result) in ordered.into_iter().enumerate() {
        let (summary, replay, primary_ms, verification_ms) =
            result.ok_or_else(|| format!("worker omitted game {index}"))?;
        summaries.push(summary);
        if let Some(replay) = replay {
            replays.push(replay);
        }
        primary_worker_ms = primary_worker_ms.saturating_add(primary_ms);
        replay_worker_ms = replay_worker_ms.saturating_add(verification_ms);
    }
    Ok(CampaignResult {
        summaries,
        replays,
        primary_worker_ms,
        replay_worker_ms,
    })
}

fn campaign_seed(base: u64, index: usize) -> u64 {
    let mut value = base ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn run_game_pair(
    pod: &PodTemplate,
    manifest: &Path,
    index: usize,
    seed: u64,
    max_turns: u32,
) -> Result<(GameSummary, Option<PodReplay>, u128, u128), String> {
    let coverage_target = pod
        .semantic_identities
        .keys()
        .nth(index % pod.semantic_identities.len())
        .cloned();
    let primary_started = Instant::now();
    let primary_mode = if index < RETAINED_REPLAYS {
        TraceMode::Record(Vec::new())
    } else {
        TraceMode::Off
    };
    let primary =
        GameDriver::setup(pod, seed, coverage_target.clone(), primary_mode)?.run(max_turns)?;
    let primary_ms = primary_started.elapsed().as_millis();
    let retained_trace = primary.trace.clone();
    if index < RETAINED_REPLAYS && retained_trace.is_none() {
        return Err("recording run did not return an action trace".to_owned());
    }

    let replay_started = Instant::now();
    let replay_state = replay_captured_actions(&primary.actions, primary.trace.as_deref())?;
    let replay_ms = replay_started.elapsed().as_millis();
    let replay_life = std::array::from_fn(|seat| replay_state.players()[seat].life());
    let replay_winner = match replay_state.game_outcome() {
        GameOutcome::Won(player) => player.index(),
        outcome => return Err(format!("direct action replay ended with {outcome:?}")),
    };
    if replay_state.deterministic_hash().get() != primary.summary.final_hash
        || replay_life != primary.summary.final_life
        || replay_winner != primary.summary.winner
    {
        return Err(format!(
            "direct action replay diverged from primary summary: {:?}",
            primary.summary
        ));
    }
    let replay_artifact = retained_trace.map(|actions| PodReplay {
        format: POD_REPLAY_MAGIC.to_owned(),
        manifest: manifest.to_path_buf(),
        seed,
        max_turns,
        coverage_target,
        actions,
        expected: primary.summary.clone(),
    });
    Ok((primary.summary, replay_artifact, primary_ms, replay_ms))
}

fn replay_captured_actions(
    actions: &[Action],
    expected_trace: Option<&[TraceRecord]>,
) -> Result<GameState, String> {
    if expected_trace.is_some_and(|trace| trace.len() != actions.len()) {
        return Err("retained trace length does not match the typed action stream".to_owned());
    }
    let mut state = GameState::new();
    for (index, action) in actions.iter().enumerate() {
        let expected = expected_trace.and_then(|trace| trace.get(index));
        let before_hash = expected.map(|_| state.deterministic_hash().get());
        let outcome = apply(&mut state, action.clone());
        if let Some(expected) = expected {
            let actual = TraceRecord {
                index: index as u64,
                action: format!("{action:?}"),
                before_hash: before_hash.unwrap_or_default(),
                outcome: format!("{outcome:?}"),
                after_hash: state.deterministic_hash().get(),
            };
            if &actual != expected {
                return Err(format!(
                    "direct replay diverged at action {index}: expected {expected:?}, got {actual:?}"
                ));
            }
        }
        if let Outcome::Failed(error) = outcome {
            return Err(format!(
                "direct replay action {index} was rejected: {error:?}"
            ));
        }
    }
    Ok(state)
}

fn write_replays(directory: &Path, replays: &[PodReplay]) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    for (index, replay) in replays.iter().enumerate() {
        let path = directory.join(format!("pod-seed-{:02}.frsreplay", index + 1));
        let payload = serde_json::to_vec_pretty(replay)
            .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
        fs::write(&path, payload)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Runs one prompted four-player Commander game and writes an exact replay artifact.
pub fn run_prompted_game(
    manifest: impl AsRef<Path>,
    replay_out: impl AsRef<Path>,
    seed: u64,
    max_turns: u32,
    human_seat: usize,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<String, String> {
    if human_seat >= PLAYER_COUNT {
        return Err(format!("human seat must be in 1..={PLAYER_COUNT}"));
    }
    if max_turns < 20 {
        return Err("--max-turns must be at least 20".to_owned());
    }
    let manifest = manifest.as_ref();
    let replay_out = replay_out.as_ref();
    let pod = PodTemplate::load(manifest)?;
    writeln!(
        output,
        "Forge human play | seat {}: {} | seed {seed}",
        human_seat + 1,
        pod.decks[human_seat].name
    )
    .map_err(|error| format!("failed to write game introduction: {error}"))?;
    writeln!(output, "Choose numbered legal actions. Enter q to stop.")
        .map_err(|error| format!("failed to write game introduction: {error}"))?;

    let driver = GameDriver::setup(&pod, seed, None, TraceMode::Record(Vec::new()))?;
    let human = driver.players[human_seat];
    let mut terminal = TerminalDecisionSource::new(input, output);
    let primary = driver.run_human(max_turns, human, &mut terminal)?;
    let decisions = terminal.into_decisions();
    let actions = primary
        .trace
        .clone()
        .ok_or_else(|| "human game did not retain an action trace".to_owned())?;

    let verify_driver = GameDriver::setup(
        &pod,
        seed,
        None,
        TraceMode::Verify {
            expected: actions.clone(),
            cursor: 0,
        },
    )?;
    let verify_human = verify_driver.players[human_seat];
    let mut replay_source = ReplayDecisionSource::new(decisions.clone());
    let verified = verify_driver.run_human(max_turns, verify_human, &mut replay_source)?;
    replay_source.finish()?;
    if verified.summary != primary.summary {
        return Err(format!(
            "human decision replay summary diverged: {:?} != {:?}",
            primary.summary, verified.summary
        ));
    }
    let direct_state = replay_captured_actions(&primary.actions, Some(&actions))?;
    if direct_state.deterministic_hash().get() != primary.summary.final_hash {
        return Err("direct human-game action replay produced a different final hash".to_owned());
    }

    let replay = HumanPlayReplay {
        format: HUMAN_REPLAY_MAGIC.to_owned(),
        manifest: manifest.to_path_buf(),
        seed,
        max_turns,
        human_seat,
        decisions,
        actions,
        expected: primary.summary.clone(),
    };
    if let Some(parent) = replay_out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(&replay)
        .map_err(|error| format!("failed to serialize human replay: {error}"))?;
    fs::write(replay_out, payload)
        .map_err(|error| format!("failed to write {}: {error}", replay_out.display()))?;
    Ok(format!(
        "human game complete\nseed: {}\nturns: {}\nwinner_seat: {}\nactions: {}\ndecisions: {}\nfinal_hash: {}\nreplay: {}\n",
        seed,
        primary.summary.turns,
        primary.summary.winner + 1,
        primary.actions.len(),
        replay.decisions.len(),
        primary.summary.final_hash,
        replay_out.display()
    ))
}

/// Replays either a T3.9 pod artifact or a T1.R10 human-play artifact.
pub fn replay_json_file(path: impl AsRef<Path>) -> Result<String, String> {
    let path = path.as_ref();
    let payload = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    match value.get("format").and_then(Value::as_str) {
        Some(POD_REPLAY_MAGIC) => replay_pod_file(path),
        Some(HUMAN_REPLAY_MAGIC) => replay_human_file(path),
        Some(format) => Err(format!("unsupported JSON replay format `{format}`")),
        None => Err(format!("{} has no replay format", path.display())),
    }
}

/// Replays a T1.R10 prompted game, including every recorded human decision.
pub fn replay_human_file(path: impl AsRef<Path>) -> Result<String, String> {
    let path = path.as_ref();
    let payload =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let replay: HumanPlayReplay = serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if replay.format != HUMAN_REPLAY_MAGIC {
        return Err(format!(
            "{} is not a {HUMAN_REPLAY_MAGIC} artifact",
            path.display()
        ));
    }
    if replay.human_seat >= PLAYER_COUNT {
        return Err(format!("human replay seat must be in 1..={PLAYER_COUNT}"));
    }
    let pod = PodTemplate::load(&replay.manifest)?;
    let driver = GameDriver::setup(
        &pod,
        replay.seed,
        None,
        TraceMode::Verify {
            expected: replay.actions.clone(),
            cursor: 0,
        },
    )?;
    let human = driver.players[replay.human_seat];
    let mut decisions = ReplayDecisionSource::new(replay.decisions.clone());
    let run = driver.run_human(replay.max_turns, human, &mut decisions)?;
    decisions.finish()?;
    if run.summary != replay.expected {
        return Err(format!(
            "human replay summary diverged: {:?} != {:?}",
            replay.expected, run.summary
        ));
    }
    let direct_state = replay_captured_actions(&run.actions, None)?;
    if direct_state.deterministic_hash().get() != replay.expected.final_hash {
        return Err("direct typed-action playback produced a different final hash".to_owned());
    }
    Ok(format!(
        "human replay complete (decisions and typed actions verified)\nseed: {}\ndecisions: {}\nactions: {}\nfinal_hash: {}\nwinner_seat: {}\n",
        replay.seed,
        replay.decisions.len(),
        replay.actions.len(),
        run.summary.final_hash,
        run.summary.winner + 1
    ))
}

/// Replays a recorded T3.9 pod action stream and verifies every state transition.
pub fn replay_pod_file(path: impl AsRef<Path>) -> Result<String, String> {
    let path = path.as_ref();
    let payload =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let replay: PodReplay = serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if replay.format != POD_REPLAY_MAGIC {
        return Err(format!(
            "{} is not a {POD_REPLAY_MAGIC} artifact",
            path.display()
        ));
    }
    let pod = PodTemplate::load(&replay.manifest)?;
    let action_count = replay.actions.len();
    let run = GameDriver::setup(
        &pod,
        replay.seed,
        replay.coverage_target.clone(),
        TraceMode::Verify {
            expected: replay.actions,
            cursor: 0,
        },
    )?
    .run(replay.max_turns)?;
    if run.summary != replay.expected {
        return Err(format!(
            "replay summary diverged: {:?} != {:?}",
            replay.expected, run.summary
        ));
    }
    let direct_state = replay_captured_actions(&run.actions, None)?;
    if direct_state.deterministic_hash().get() != replay.expected.final_hash {
        return Err("direct typed-action playback produced a different final hash".to_owned());
    }
    Ok(format!(
        "pod replay complete (typed actions reapplied)\nseed: {}\nactions: {}\nfinal_hash: {}\nwinner_seat: {}\n",
        replay.seed, action_count, run.summary.final_hash, run.summary.winner
    ))
}

fn validate_campaign(
    games: &[GameSummary],
    expected: usize,
    semantic_identities: &BTreeMap<String, bool>,
) -> Result<(), String> {
    if games.len() != expected {
        return Err(format!(
            "campaign produced {} of {expected} games",
            games.len()
        ));
    }
    if let Some(game) = games.iter().find(|game| game.final_hash == 0) {
        return Err(format!(
            "seed {} produced a zero deterministic hash",
            game.seed
        ));
    }
    if let Some(game) = games
        .iter()
        .find(|game| game.metrics.commander_casts < PLAYER_COUNT as u64)
    {
        return Err(format!(
            "seed {} did not cast all four commanders ({})",
            game.seed, game.metrics.commander_casts
        ));
    }
    if let Some(game) = games
        .iter()
        .find(|game| game.metrics.taxed_commander_recasts == 0)
    {
        return Err(format!(
            "seed {} did not exercise a taxed commander recast",
            game.seed
        ));
    }
    if let Some(game) = games
        .iter()
        .find(|game| game.metrics.commander_zone_returns == 0)
    {
        return Err(format!(
            "seed {} did not exercise an owner commander-zone choice",
            game.seed
        ));
    }
    if let Some(game) = games.iter().find(|game| {
        game.metrics.hidden_information_checks <= (PLAYER_COUNT as u64).saturating_mul(2)
    }) {
        return Err(format!(
            "seed {} only ran endpoint hidden-information checks ({})",
            game.seed, game.metrics.hidden_information_checks
        ));
    }
    if let Some(game) = games
        .iter()
        .find(|game| game.metrics.combat_damage_events == 0 || game.metrics.eliminations < 3)
    {
        return Err(format!(
            "seed {} did not complete combat elimination",
            game.seed
        ));
    }
    if games
        .iter()
        .map(|game| game.metrics.triggers_resolved)
        .sum::<u64>()
        == 0
    {
        return Err("campaign did not resolve any card-driven trigger".to_owned());
    }
    let mut observed = BTreeMap::<String, IdentityExercise>::new();
    for game in games {
        for (identity, exercise) in &game.metrics.identity_exercise {
            observed
                .entry(identity.clone())
                .or_default()
                .add_assign(exercise);
        }
    }
    if expected >= semantic_identities.len() {
        for (identity, is_land) in semantic_identities {
            let exercise = observed.get(identity).cloned().unwrap_or_default();
            let exercised = if *is_land {
                exercise.land_plays > 0
            } else {
                exercise.casts > 0 && exercise.resolutions > 0
            };
            if !exercised {
                return Err(format!(
                    "semantic pod identity {identity} (land={is_land}) was not exercised: {exercise:?}"
                ));
            }
        }
    }
    Ok(())
}

fn build_report(
    pod: &PodTemplate,
    options: &Options,
    games: &[GameSummary],
    load_ms: u128,
    campaign_ms: u128,
    primary_worker_ms: u128,
    replay_worker_ms: u128,
) -> Value {
    let mut totals = GameMetrics::default();
    let mut wins = [0_u64; PLAYER_COUNT];
    let mut max_turns = 0_u32;
    let mut min_turns = u32::MAX;
    let mut sum_turns = 0_u64;
    for game in games {
        totals.add_assign(&game.metrics);
        wins[game.winner] = wins[game.winner].saturating_add(1);
        max_turns = max_turns.max(game.turns);
        min_turns = min_turns.min(game.turns);
        sum_turns = sum_turns.saturating_add(u64::from(game.turns));
    }
    let seconds = campaign_ms.max(1) as f64 / 1_000.0;
    let deck_records = pod
        .decks
        .iter()
        .map(|deck| {
            json!({
                "id": deck.id,
                "name": deck.name,
                "commander": deck.commander.program.name(),
                "commander_oracle_id": deck.commander.program.oracle_id(),
                "cards": COMMANDER_DECK_SIZE,
                "semantic_mainboard_cards": COMMANDER_DECK_SIZE - 1
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 2,
        "task": "T3.9",
        "checkpoint": "CP-FOUR-PLAYER-POD",
        "status": "passed",
        "claim_boundary": "Four legal compiled Commander decks completed deterministic card-driven games through production setup, mana, casting, priority, triggers, combat, owner-selected commander zone/tax, elimination, recurring redacted-view canaries, invariants, and direct typed-action replay against fresh kernel state.",
        "source": {
            "manifest": pod.manifest_path,
            "translated_definitions": pod.source_root,
            "semantic_registry": pod.semantic_registry,
            "runtime": "forge_cards::runtime",
            "kernel_boundary": "forge_core::apply"
        },
        "constraints": {
            "github_actions_used": false,
            "network_used": false,
            "installs_performed": false,
            "push_performed": false,
            "worker_limit": MAX_WORKERS
        },
        "configuration": {
            "games": options.games,
            "replay_runs": 2,
            "players_per_game": PLAYER_COUNT,
            "starting_life": STARTING_LIFE,
            "jobs": options.jobs.min(options.games),
            "max_turns": options.max_turns,
            "seed_base": options.seed_base.to_string(),
            "coverage_schedule": "deterministic round-robin identity placed on top before opening hands; all subsequent actions remain production-legal",
            "replay_directory": options.replay_dir,
            "decks": deck_records,
            "semantic_mainboard_cards_across_manifests": pod.semantic_mainboard_cards,
            "semantic_identity_count": pod.semantic_identities.len(),
            "semantic_identity_requirements": pod.semantic_identities
        },
        "results": {
            "games_completed": games.len(),
            "direct_typed_action_replays_matched": games.len(),
            "action_replays_matched": RETAINED_REPLAYS.min(games.len()),
            "retained_action_replays": RETAINED_REPLAYS.min(games.len()),
            "draws": 0,
            "wins_by_seat": wins,
            "turns": {
                "min": min_turns,
                "max": max_turns,
                "mean": sum_turns as f64 / games.len() as f64
            },
            "actions": totals.actions,
            "casts": totals.casts,
            "commander_casts": totals.commander_casts,
            "taxed_commander_recasts": totals.taxed_commander_recasts,
            "commander_zone_returns": totals.commander_zone_returns,
            "lands_played": totals.lands_played,
            "mana_abilities": totals.mana_abilities,
            "priority_passes": totals.priority_passes,
            "triggers_registered": totals.triggers_registered,
            "triggers_resolved": totals.triggers_resolved,
            "interpreter_actions": totals.interpreter_actions,
            "combat_declarations": totals.combat_declarations,
            "combat_damage_events": totals.combat_damage_events,
            "eliminations": totals.eliminations,
            "invariant_checks": totals.invariant_checks,
            "invariant_violations": 0,
            "hidden_information_checks": totals.hidden_information_checks,
            "hidden_information_canary_violations": 0,
            "identity_exercise": totals.identity_exercise
        },
        "runtime": {
            "manifest_load_ms": load_ms,
            "campaign_wall_ms": campaign_ms,
            "primary_worker_ms": primary_worker_ms,
            "replay_worker_ms": replay_worker_ms,
            "primary_games_per_second": games.len() as f64 / seconds
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        campaign_seed, player_view_fingerprint, replay_captured_actions, replay_human_file,
        run_prompted_game, snapshot_prompt, DecisionPrompt, DecisionSource, IdentityExercise,
        ReplayDecisionSource, TraceRecord,
    };
    use forge_core::{apply, Action, CardId, GameState, Outcome, PlayerView, ZoneId, ZoneKind};
    use std::{collections::BTreeSet, env, io::Cursor, path::Path};

    #[test]
    fn action_replay_rejects_a_tampered_transition() {
        let action = Action::SetSeed { seed: 7 };
        let mut primary = GameState::new();
        let before_hash = primary.deterministic_hash().get();
        let outcome = apply(&mut primary, action.clone());
        let expected = TraceRecord {
            index: 0,
            action: format!("{action:?}"),
            before_hash,
            outcome: format!("{outcome:?}"),
            after_hash: primary.deterministic_hash().get(),
        };
        assert!(replay_captured_actions(
            std::slice::from_ref(&action),
            Some(std::slice::from_ref(&expected)),
        )
        .is_ok());
        let mut tampered = expected;
        tampered.after_hash = tampered.after_hash.wrapping_add(1);
        let error = match replay_captured_actions(&[action], Some(&[tampered])) {
            Ok(_) => panic!("a changed state hash must fail replay"),
            Err(error) => error,
        };
        assert!(error.contains("direct replay diverged at action 0"));
    }

    #[test]
    fn campaign_seed_schedule_is_deterministic_and_disperse() {
        let first = (0..1_000)
            .map(|index| campaign_seed(17, index))
            .collect::<Vec<_>>();
        let second = (0..1_000)
            .map(|index| campaign_seed(17, index))
            .collect::<Vec<_>>();
        assert_eq!(first, second);
        assert_eq!(first.iter().copied().collect::<BTreeSet<_>>().len(), 1_000);
    }

    #[test]
    fn identity_exercise_aggregation_preserves_every_counter() {
        let mut total = IdentityExercise {
            land_plays: 1,
            casts: 2,
            resolutions: 3,
            effect_actions: 4,
            trigger_resolutions: 5,
        };
        total.add_assign(&IdentityExercise {
            land_plays: 6,
            casts: 7,
            resolutions: 8,
            effect_actions: 9,
            trigger_resolutions: 10,
        });
        assert_eq!(total.land_plays, 7);
        assert_eq!(total.casts, 9);
        assert_eq!(total.resolutions, 11);
        assert_eq!(total.effect_actions, 13);
        assert_eq!(total.trigger_resolutions, 15);
    }

    #[test]
    fn decision_replay_binds_view_options_and_selection() {
        let view = hidden_card_view(7, false);
        let options = vec!["First legal action".to_owned(), "Pass".to_owned()];
        let prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            options: &options,
        };
        let record = snapshot_prompt(0, &prompt, 1);
        let mut replay = ReplayDecisionSource::new(vec![record]);
        assert_eq!(replay.choose(&prompt), Ok(1));
        assert!(replay.finish().is_ok());

        let changed_options = vec!["Different action".to_owned(), "Pass".to_owned()];
        let changed_prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            options: &changed_options,
        };
        let record = snapshot_prompt(0, &prompt, 1);
        let mut replay = ReplayDecisionSource::new(vec![record]);
        assert!(replay.choose(&changed_prompt).is_err());
    }

    #[test]
    fn prompt_fingerprint_cannot_see_opponent_hidden_card_identity() {
        let first_hidden = hidden_card_view(11, false);
        let second_hidden = hidden_card_view(99, false);
        assert_eq!(
            player_view_fingerprint(&first_hidden),
            player_view_fingerprint(&second_hidden)
        );

        let first_known = hidden_card_view(11, true);
        let second_known = hidden_card_view(99, true);
        assert_ne!(
            player_view_fingerprint(&first_known),
            player_view_fingerprint(&second_known)
        );
    }

    #[test]
    #[ignore = "requires the local T3 translated-card output"]
    fn scripted_human_game_completes_and_replays_exactly() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if let Err(error) = env::set_current_dir(&workspace) {
            panic!("workspace root should be available: {error}");
        }
        let manifest = Path::new("assets/t3_9/integration_decks.json");
        let replay = Path::new("target/t1-r10/scripted-human.frsreplay");
        assert!(
            Path::new("target/translated-cards/i/isamaru_hound_of_konda.frs").is_file(),
            "run scripts/t3_parallel_sweep.sh development first"
        );
        let mut input = Cursor::new("1\n".repeat(20_000));
        let mut output = Vec::new();
        let report = match run_prompted_game(
            manifest,
            replay,
            20_260_714,
            160,
            0,
            &mut input,
            &mut output,
        ) {
            Ok(report) => report,
            Err(error) => panic!("scripted prompt driver should complete: {error}"),
        };
        assert!(report.contains("human game complete"));
        let replay_report = match replay_human_file(replay) {
            Ok(report) => report,
            Err(error) => panic!("saved human replay should verify: {error}"),
        };
        assert!(replay_report.contains("decisions and typed actions verified"));
    }

    fn hidden_card_view(card: u32, owned_by_observer: bool) -> PlayerView {
        let mut state = GameState::new();
        let Outcome::PlayerAdded(observer) = apply(&mut state, Action::AddPlayer) else {
            panic!("observer setup failed");
        };
        let Outcome::PlayerAdded(opponent) = apply(&mut state, Action::AddPlayer) else {
            panic!("opponent setup failed");
        };
        let owner = if owned_by_observer {
            observer
        } else {
            opponent
        };
        let outcome = apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(card),
                owner,
                controller: owner,
                zone: ZoneId::new(Some(owner), ZoneKind::Hand),
            },
        );
        assert!(matches!(outcome, Outcome::ObjectCreated(_)));
        match state.player_view(observer) {
            Ok(view) => view,
            Err(error) => panic!("player view should exist: {error:?}"),
        }
    }
}
