#![forbid(unsafe_code)]

//! Local T3.9 and CP-FOUR-PLAYER-POD integration runner.
//!
//! The controller is deliberately generic: deck contents come from a
//! deterministic manifest, card behavior comes from `forge-cards::runtime`,
//! and every mutation crosses `forge_core::apply`.

use forge_ai::{
    ActionRisk, ActionRisks, AdaptiveStopping, AiWeights, DeckModel, Determinizer,
    GuardrailProfile, GuardrailTable, HeuristicPolicy, LastDecisionReport, MulliganPolicy,
    PolicyCandidate, PolicyDecision, PolicyMode, RandomLegalPolicy, SearchConfig, SearchDomain,
    SearchEngine, SearchLimit, SearchReport, SearchStopReason,
};
use forge_cards::runtime::{
    bind_triggered_ability_actions, compile_card_program, CardProgram, ExecutionBindings,
    ProgramKind, TriggeredAbilityProgram,
};
use forge_core::{
    apply, Action, ActivatedAbilityId, AttackDeclaration, BlockDeclaration, CanonicalActionId,
    CardId, CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest,
    CombatDamageStepKind, CombatDamageTarget, DecisionContext, DecisionDescriptor, DecisionKind,
    DecisionOption, GameOutcome, GameState, HiddenCardDefinition, HiddenSlotDefinition, ManaKind,
    ObjectColors, ObjectId, ObjectView, Outcome, PaymentPlan, PlayerId, PlayerView,
    PriorityOutcome, SpellTiming, StackEntryId, StackObjectKind, Step, TargetChoice, TriggerId,
    ZoneId, ZoneKind,
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

/// Number of seats in the Commander arena protocol.
pub const AI_ARENA_SEATS: usize = 4;
const PLAYER_COUNT: usize = AI_ARENA_SEATS;
const COMMANDER_DECK_SIZE: usize = 100;
const STARTING_LIFE: i32 = 40;
const MAX_WORKERS: usize = 24;
const DEFAULT_GAMES: usize = 1_000;
const DEFAULT_MAX_TURNS: u32 = 160;
const DEFAULT_SEED_BASE: u64 = 0xF02D_0000_0000_0000;
const POD_REPLAY_MAGIC: &str = "forge-pod-replay-v1";
const HUMAN_REPLAY_MAGIC: &str = "forge-human-play-replay-v1";
const AI_REPLAY_MAGIC: &str = "forge-ai-baseline-replay-v1";
const PILOT_INTENTS_PATH: &str = "assets/ai/pilot_intents.json";
const RETAINED_REPLAYS: usize = 10;
const MAX_DIAGNOSTIC_COMBAT_OPTIONS: usize = 262_144;

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

#[derive(Clone, Debug, Deserialize)]
struct PilotIntentRegistry {
    schema_version: u32,
    decks: Vec<PilotIntent>,
}

#[derive(Clone, Debug, Deserialize)]
struct PilotIntent {
    deck_id: String,
    classification: String,
    primary_plan: String,
    secondary_plan: String,
    win_conditions: Vec<String>,
    combos: Vec<String>,
    mulligan_priorities: Vec<String>,
    commander_role: String,
    tutor_priorities: Vec<String>,
    interaction_posture: String,
    protected_cards: Vec<String>,
    avoided_lines: Vec<String>,
    social_constraints: Vec<String>,
}

impl PilotIntentRegistry {
    fn load(path: &Path, pod: &PodTemplate) -> Result<Self, String> {
        let payload = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let registry: Self = serde_json::from_slice(&payload)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        if registry.schema_version != 1 {
            return Err(format!(
                "{} has unsupported PilotIntent schema {}",
                path.display(),
                registry.schema_version
            ));
        }
        let mut seen = BTreeSet::new();
        for intent in &registry.decks {
            if !seen.insert(intent.deck_id.as_str()) {
                return Err(format!("duplicate PilotIntent for `{}`", intent.deck_id));
            }
            if !matches!(
                intent.classification.as_str(),
                "ready" | "limited" | "diagnostics-only" | "blocked"
            ) {
                return Err(format!(
                    "PilotIntent `{}` has invalid classification `{}`",
                    intent.deck_id, intent.classification
                ));
            }
            if intent.primary_plan.trim().is_empty()
                || intent.secondary_plan.trim().is_empty()
                || intent.win_conditions.is_empty()
                || intent.mulligan_priorities.is_empty()
                || intent.commander_role.trim().is_empty()
                || intent.interaction_posture.trim().is_empty()
            {
                return Err(format!(
                    "PilotIntent `{}` is missing a required plan field",
                    intent.deck_id
                ));
            }
            let _ = (
                &intent.combos,
                &intent.tutor_priorities,
                &intent.protected_cards,
                &intent.avoided_lines,
                &intent.social_constraints,
            );
        }
        for deck in &pod.decks {
            if !seen.contains(deck.id.as_str()) {
                return Err(format!("missing PilotIntent for deck `{}`", deck.id));
            }
        }
        Ok(registry)
    }

    fn limited_count(&self) -> usize {
        self.decks
            .iter()
            .filter(|intent| intent.classification != "ready")
            .count()
    }
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
    #[serde(default)]
    decision_context_schema: u32,
    #[serde(default)]
    context_id: String,
    #[serde(default)]
    decision_state_key: String,
    #[serde(default)]
    player_view_hash: String,
    #[serde(default)]
    canonical_legal_actions: Vec<AiLegalAction>,
    #[serde(default)]
    selected_action_id: String,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AiPlayReplay {
    format: String,
    manifest: PathBuf,
    seed: u64,
    max_turns: u32,
    policy_seed: u64,
    policy_kind: String,
    noise_span: i64,
    #[serde(default)]
    search_iterations: u32,
    #[serde(default)]
    search_determinizations: u32,
    #[serde(default)]
    search_workers: u32,
    pilot_intents: PathBuf,
    decisions: Vec<AiDecisionRecord>,
    expected: GameSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AiDecisionRecord {
    index: u64,
    kind: String,
    policy: String,
    context_id: String,
    #[serde(default)]
    decision_state_key: String,
    #[serde(default)]
    player_view_hash: String,
    action_id: String,
    #[serde(default)]
    canonical_legal_actions: Vec<AiLegalAction>,
    evaluation: i64,
    prior: i64,
    noise: i64,
    score: i64,
    legal_actions: u32,
    evaluated_candidates: u32,
    determinizations: u32,
    #[serde(default)]
    configured_iterations: u32,
    #[serde(default)]
    configured_wall_ms: u32,
    #[serde(default)]
    adaptive_search: bool,
    think_ms: u32,
    #[serde(default)]
    simulations: u64,
    #[serde(default)]
    nodes: u64,
    #[serde(default)]
    maximum_depth: u32,
    #[serde(default)]
    transposition_hits: u64,
    #[serde(default)]
    value_gap: i64,
    #[serde(default)]
    visit_gap: u64,
    #[serde(default)]
    uncertainty_ppm: u32,
    #[serde(default)]
    leading_visit_share_ppm: u32,
    #[serde(default)]
    checkpoint_count: u32,
    #[serde(default)]
    ranking_stable: bool,
    #[serde(default)]
    bounded_solver_state: String,
    #[serde(default)]
    search_checkpoints: Vec<AiSearchCheckpoint>,
    #[serde(default)]
    actual_cpu_time_us: Option<u64>,
    #[serde(default)]
    memory_delta_bytes: Option<i64>,
    #[serde(default)]
    considered_actions: Vec<AiConsideredAction>,
    wall_latency_us: u64,
    stop_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AiConsideredAction {
    action_id: String,
    visits: u64,
    mean_value: i64,
    value_delta_from_selected: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AiLegalAction {
    action_id: String,
    descriptor_schema_version: u32,
    descriptor: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AiSearchCheckpoint {
    determinization: u32,
    simulations: u32,
    leading_action_id: String,
    leading_visit_share_ppm: u32,
    value_gap: i64,
    visit_gap: u32,
    ranking_stable: bool,
    uncertainty_ppm: u32,
    bounded_solver_state: String,
    stop_reason: Option<String>,
}

struct AiDecisionTelemetry<'a> {
    kind: &'static str,
    policy: &'static str,
    context: &'a DecisionContext,
    action_id: CanonicalActionId,
    decision: Option<PolicyDecision>,
    evaluated_candidates: usize,
    wall_latency_us: u64,
    score_override: Option<i64>,
    stop_reason: &'static str,
}

struct DecisionPrompt<'a> {
    kind: &'static str,
    view: &'a PlayerView,
    context: &'a DecisionContext,
    options: &'a [String],
}

struct LegacyDecisionPrompt<'a> {
    kind: &'static str,
    view: &'a PlayerView,
    options: &'a [String],
}

trait DecisionSource {
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String>;

    fn is_legacy_replay(&self) -> bool {
        false
    }

    fn choose_legacy(&mut self, _prompt: &LegacyDecisionPrompt<'_>) -> Result<usize, String> {
        Err("legacy decisions are available only while replaying a legacy artifact".to_owned())
    }
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
        validate_decision_prompt(prompt)?;
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
    legacy: bool,
}

impl ReplayDecisionSource {
    fn new(decisions: Vec<HumanDecisionRecord>) -> Self {
        let legacy = !decisions.is_empty()
            && decisions
                .iter()
                .all(|decision| decision.context_id.is_empty());
        Self {
            decisions,
            cursor: 0,
            legacy,
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
        validate_decision_prompt(prompt)?;
        let expected = self
            .decisions
            .get(self.cursor)
            .ok_or_else(|| format!("unexpected replay prompt `{}`", prompt.kind))?;
        let actual = snapshot_prompt(expected.index, prompt, expected.selected);
        if !human_decision_matches(expected, &actual) {
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

    fn is_legacy_replay(&self) -> bool {
        self.legacy
    }

    fn choose_legacy(&mut self, prompt: &LegacyDecisionPrompt<'_>) -> Result<usize, String> {
        if !self.legacy {
            return Err("a canonical replay cannot enter the legacy decision adapter".to_owned());
        }
        let expected = self
            .decisions
            .get(self.cursor)
            .ok_or_else(|| format!("unexpected replay prompt `{}`", prompt.kind))?;
        let actual_step = prompt
            .view
            .current_step()
            .map_or_else(|| "none".to_owned(), |step| format!("{step:?}"));
        let options_match = expected.options.len() == prompt.options.len()
            && expected
                .options
                .iter()
                .zip(prompt.options)
                .all(|(expected, actual)| legacy_option_key(expected) == legacy_option_key(actual));
        if expected.prompt != prompt.kind
            || expected.turn != prompt.view.turn_number()
            || expected.step != actual_step
            || !options_match
        {
            return Err(format!(
                "legacy decision replay diverged at prompt {}: expected {expected:?}, got kind={}, turn={}, step={}, options={:?}",
                self.cursor,
                prompt.kind,
                prompt.view.turn_number(),
                actual_step,
                prompt.options
            ));
        }
        if expected.selected >= prompt.options.len() {
            return Err(format!(
                "legacy decision replay selection {} is outside {} options",
                expected.selected,
                prompt.options.len()
            ));
        }
        self.cursor = self.cursor.saturating_add(1);
        Ok(expected.selected)
    }
}

fn legacy_option_key(label: &str) -> &str {
    label
        .split_once(" (payment waste ")
        .map_or(label, |(prefix, _)| prefix)
}

fn snapshot_prompt(
    index: u64,
    prompt: &DecisionPrompt<'_>,
    selected: usize,
) -> HumanDecisionRecord {
    let selected_action_id = prompt
        .context
        .options()
        .get(selected)
        .map_or_else(String::new, |option| option.id().to_string());
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
        decision_context_schema: prompt.context.schema_version(),
        context_id: prompt.context.id().to_string(),
        decision_state_key: prompt.context.state_key().to_string(),
        player_view_hash: format!("{:016x}", prompt.context.player_view_hash().get()),
        canonical_legal_actions: prompt
            .context
            .options()
            .iter()
            .map(|option| AiLegalAction {
                action_id: option.id().to_string(),
                descriptor_schema_version: 1,
                descriptor: decision_descriptor_value(option.descriptor()),
            })
            .collect(),
        selected_action_id,
    }
}

fn validate_decision_prompt(prompt: &DecisionPrompt<'_>) -> Result<(), String> {
    if prompt.context.actor() != prompt.view.observer() {
        return Err(format!(
            "{} prompt actor does not match its PlayerView observer",
            prompt.kind
        ));
    }
    if prompt.context.player_view_hash() != prompt.view.deterministic_hash() {
        return Err(format!(
            "{} prompt context does not match its PlayerView",
            prompt.kind
        ));
    }
    if prompt.options.len() != prompt.context.options().len() {
        return Err(format!(
            "{} prompt has {} labels for {} canonical actions",
            prompt.kind,
            prompt.options.len(),
            prompt.context.options().len()
        ));
    }
    Ok(())
}

fn human_decision_matches(expected: &HumanDecisionRecord, actual: &HumanDecisionRecord) -> bool {
    if !expected.context_id.is_empty() {
        return expected == actual;
    }
    expected.index == actual.index
        && expected.prompt == actual.prompt
        && expected.turn == actual.turn
        && expected.step == actual.step
        && expected.view_fingerprint == actual.view_fingerprint
        && expected.options == actual.options
        && expected.selected == actual.selected
}

fn player_view_fingerprint(view: &PlayerView) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in format!("{view:?}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone)]
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
    ai_decisions: Vec<AiDecisionRecord>,
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
enum MainChoice {
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

#[derive(Clone, Copy)]
enum AiController {
    Heuristic(HeuristicPolicy),
    Random(RandomLegalPolicy),
    Search(SearchController),
}

type SeatPolicies = [AiController; PLAYER_COUNT];

impl AiController {
    const fn candidate_weights(self) -> Option<AiWeights> {
        match self {
            Self::Heuristic(policy) => Some(policy.weights()),
            Self::Search(controller) => Some(controller.weights),
            Self::Random(_) => None,
        }
    }

    const fn guardrail_profile(self) -> Option<GuardrailProfile> {
        match self {
            Self::Heuristic(policy) => Some(match policy.mode() {
                PolicyMode::Novice => GuardrailProfile::Novice,
                PolicyMode::Rollout => GuardrailProfile::Standard,
            }),
            Self::Search(controller) => Some(controller.guardrail_profile),
            Self::Random(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
struct SearchController {
    weights: AiWeights,
    seed: u64,
    determinizations: u32,
    limit: SearchControllerLimit,
    workers: u32,
    adaptive: bool,
    guardrail_profile: GuardrailProfile,
}

#[derive(Clone, Copy)]
enum SearchControllerLimit {
    Iterations(u32),
    WallTimeMs(u32),
}

impl SearchController {
    const fn rollout(self, decision_index: u64) -> HeuristicPolicy {
        HeuristicPolicy::rollout(
            self.weights,
            self.seed ^ decision_index.wrapping_mul(0x9e37_79b9_7f4a_7c15),
        )
    }

    fn config(self, decision_index: u64) -> SearchConfig {
        let seed = self.seed ^ decision_index.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let config = match self.limit {
            SearchControllerLimit::Iterations(iterations) => {
                SearchConfig::fixed_iterations(seed, self.determinizations, iterations)
            }
            SearchControllerLimit::WallTimeMs(think_ms) => {
                SearchConfig::wall_time(seed, self.determinizations, u64::from(think_ms))
            }
        }
        .with_workers(self.workers);
        if self.adaptive {
            config.with_adaptive_stopping(experimental_adaptive_stopping())
        } else {
            config
        }
    }
}

#[derive(Clone)]
struct SearchCardDefinition {
    definition: HiddenCardDefinition,
    program: Arc<CardProgram>,
}

#[derive(Clone)]
struct GameDriver {
    state: GameState,
    players: Vec<PlayerId>,
    programs: Arc<HashMap<ObjectId, Arc<CardProgram>>>,
    deck_models: Arc<Vec<DeckModel>>,
    card_definitions: Arc<HashMap<CardId, SearchCardDefinition>>,
    guardrails: Arc<GuardrailTable>,
    commanders: Vec<ObjectId>,
    trigger_programs: HashMap<TriggerId, TriggerRuntime>,
    mana_abilities: Vec<(ObjectId, PlayerId, ActivatedAbilityId)>,
    triggers_registered_for: HashSet<ObjectId>,
    permanent_runtime_registered_for: HashSet<ObjectId>,
    commander_zone_decisions: HashMap<ObjectId, ZoneId>,
    current_attacks: Vec<AttackDeclaration>,
    coverage_target: Option<String>,
    metrics: GameMetrics,
    trace: TraceMode,
    actions: Vec<Action>,
    ai_decisions: Vec<AiDecisionRecord>,
    next_hidden_check_action: u64,
    next_invariant_check_action: u64,
    seed: u64,
}

#[derive(Clone)]
struct MainSearchState {
    driver: GameDriver,
    finished: bool,
    context: Arc<DecisionContext>,
    mappings: Arc<Vec<(CanonicalActionId, MainChoice)>>,
    priors: Arc<HashMap<CanonicalActionId, i64>>,
}

struct MainSearchDomain<'a> {
    root: &'a GameDriver,
    actor: PlayerId,
    weights: AiWeights,
    rollout_seed: u64,
    guardrail_profile: GuardrailProfile,
}

#[derive(Clone, Copy)]
enum CombatSearchKind {
    Attackers { active: PlayerId },
    Blockers { defending: PlayerId },
}

#[derive(Clone)]
struct CombatSearchState {
    driver: GameDriver,
    finished: bool,
    terminal_prior: i64,
    legal_actions: Arc<Vec<CanonicalActionId>>,
    options: Arc<HashMap<CanonicalActionId, CombatSearchOption>>,
}

#[derive(Clone)]
struct CombatSearchOption {
    actions: Vec<Action>,
    prior: i64,
}

struct CombatSearchDomain<'a> {
    root: &'a GameDriver,
    actor: PlayerId,
    weights: AiWeights,
    kind: CombatSearchKind,
    guardrail_profile: GuardrailProfile,
}

impl GameDriver {
    fn setup(
        pod: &PodTemplate,
        seed: u64,
        coverage_target: Option<String>,
        trace: TraceMode,
        opening_policies: Option<SeatPolicies>,
    ) -> Result<Self, String> {
        Self::setup_with_human_opening(pod, seed, coverage_target, trace, opening_policies, None)
    }

    fn setup_with_human_opening(
        pod: &PodTemplate,
        seed: u64,
        coverage_target: Option<String>,
        trace: TraceMode,
        opening_policies: Option<SeatPolicies>,
        human_opening: Option<(usize, &mut dyn DecisionSource)>,
    ) -> Result<Self, String> {
        if opening_policies.is_some() && human_opening.is_some() {
            return Err("opening hands cannot use both AI and human policies".to_owned());
        }
        let mut driver = Self {
            state: GameState::new(),
            players: Vec::with_capacity(PLAYER_COUNT),
            programs: Arc::new(HashMap::new()),
            deck_models: Arc::new(Vec::with_capacity(PLAYER_COUNT)),
            card_definitions: Arc::new(HashMap::with_capacity(PLAYER_COUNT * COMMANDER_DECK_SIZE)),
            guardrails: Arc::new(
                GuardrailTable::bundled()
                    .map_err(|error| format!("failed to load AI guardrails: {error}"))?,
            ),
            commanders: Vec::with_capacity(PLAYER_COUNT),
            trigger_programs: HashMap::new(),
            mana_abilities: Vec::new(),
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target,
            metrics: GameMetrics::default(),
            trace,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
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
            let mut deck_cards = Vec::with_capacity(COMMANDER_DECK_SIZE);
            let commander_card = CardId::new(next_card_id);
            let commander = driver.create_card_object(
                player,
                ZoneId::new(None, ZoneKind::Command),
                &deck.commander,
                commander_card,
            )?;
            deck_cards.push(commander_card);
            next_card_id = next_card_id.saturating_add(1);
            driver.dispatch(Action::DesignateCommander {
                object: commander,
                color_identity: deck.color_identity,
            })?;
            driver.commanders.push(commander);

            let mut deck_objects = vec![commander];
            for card in &deck.mainboard {
                let card_id = CardId::new(next_card_id);
                let object = driver.create_card_object(
                    player,
                    ZoneId::new(Some(player), ZoneKind::Library),
                    card,
                    card_id,
                )?;
                deck_cards.push(card_id);
                next_card_id = next_card_id.saturating_add(1);
                deck_objects.push(object);
            }
            driver.dispatch(Action::ValidateCommanderColorIdentity {
                player,
                objects: deck_objects,
            })?;
            driver.dispatch(Action::ShuffleLibrary { player })?;
            Arc::make_mut(&mut driver.deck_models).push(DeckModel::new(player, deck_cards));
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
        driver.check_initial_hidden_information()?;
        if let Some((human_seat, source)) = human_opening {
            driver.resolve_human_opening_hand(human_seat, source)?;
        } else if let Some(policies) = opening_policies {
            driver.resolve_ai_opening_hands(policies)?;
        } else {
            for player in driver.players.clone() {
                driver.dispatch(Action::KeepOpeningHand {
                    player,
                    bottom: Vec::new(),
                })?;
            }
        }
        driver.check_hidden_information()?;
        driver.check_invariants()?;
        driver.dispatch(Action::StartTurn {
            active_player: starting_player,
        })?;
        Ok(driver)
    }

    fn opening_hand_context(&self, player: PlayerId) -> Result<DecisionContext, String> {
        let view = self
            .state
            .player_view(player)
            .map_err(|error| format!("seed {} opening-hand view failed: {error:?}", self.seed))?;
        let hand =
            view.zone(ZoneId::new(Some(player), ZoneKind::Hand))
                .ok_or_else(|| format!("seed {} missing opening hand", self.seed))?
                .objects()
                .iter()
                .map(|object| {
                    object.known().map(|record| record.id()).ok_or_else(|| {
                        format!("seed {} actor opening hand was redacted", self.seed)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        let mulligans = self.state.players()[player.index()].mulligans_taken() as usize;
        if mulligans > hand.len() {
            return Err(format!(
                "seed {} opening hand cannot bottom {mulligans} cards from {}",
                self.seed,
                hand.len()
            ));
        }
        let mut options = vec![DecisionOption::new(
            DecisionDescriptor::TakeMulligan,
            vec![Action::TakeMulligan { player }],
        )];
        for bottom in ordered_bottoms(&hand, mulligans) {
            options.push(DecisionOption::new(
                DecisionDescriptor::KeepOpeningHand {
                    bottom: bottom.clone(),
                },
                vec![Action::KeepOpeningHand { player, bottom }],
            ));
        }
        DecisionContext::new(
            DecisionKind::OpeningHand,
            player,
            &view,
            options,
            Vec::new(),
        )
        .map_err(|error| format!("seed {} opening-hand context failed: {error}", self.seed))
    }

    fn resolve_ai_opening_hands(&mut self, policies: SeatPolicies) -> Result<(), String> {
        for (seat, player) in self.players.clone().into_iter().enumerate() {
            let policy = policies[seat];
            loop {
                let context = self.opening_hand_context(player)?;
                let decision_started = Instant::now();
                let mulligans = self.state.players()[player.index()].mulligans_taken();
                let (selected_id, policy_name, score, stop_reason, evaluated_candidates) =
                    match policy {
                        AiController::Heuristic(_) | AiController::Search(_) => {
                            let decision = MulliganPolicy::baseline()
                                .select(
                                    &context,
                                    &self.state.player_view(player).map_err(|error| {
                                        format!(
                                            "seed {} opening-hand policy view failed: {error:?}",
                                            self.seed
                                        )
                                    })?,
                                )
                                .map_err(|error| {
                                    format!("seed {} mulligan policy failed: {error}", self.seed)
                                })?;
                            (
                                decision.action_id(),
                                "mulligan-heuristic-v1",
                                Some(decision.score()),
                                "mulligan_heuristic_complete",
                                context.options().len(),
                            )
                        }
                        AiController::Random(_) if mulligans >= 6 => {
                            let selected = context
                                .options()
                                .iter()
                                .find(|option| {
                                    matches!(
                                        option.descriptor(),
                                        DecisionDescriptor::KeepOpeningHand { .. }
                                    )
                                })
                                .ok_or_else(|| {
                                    format!("seed {} random policy has no keep option", self.seed)
                                })?;
                            (
                                selected.id(),
                                "random-legal-v1",
                                None,
                                "mulligan_floor_keep",
                                0,
                            )
                        }
                        AiController::Random(random) => (
                            random
                                .select(&context, self.ai_decisions.len() as u64)
                                .map_err(|error| {
                                    format!("seed {} random mulligan failed: {error}", self.seed)
                                })?,
                            "random-legal-v1",
                            None,
                            "random_legal_selection",
                            0,
                        ),
                    };
                let selected = context.select(selected_id).map_err(|error| {
                    format!(
                        "seed {} AI selected illegal opening action: {error}",
                        self.seed
                    )
                })?;
                let kept = matches!(
                    selected.descriptor(),
                    DecisionDescriptor::KeepOpeningHand { .. }
                );
                let actions = selected.actions().to_vec();
                self.record_ai_decision(AiDecisionTelemetry {
                    kind: "opening_hand",
                    policy: policy_name,
                    context: &context,
                    action_id: selected_id,
                    decision: None,
                    evaluated_candidates,
                    wall_latency_us: elapsed_us(decision_started),
                    score_override: score,
                    stop_reason,
                });
                for action in actions {
                    self.dispatch(action)?;
                }
                if kept {
                    break;
                }
            }
        }
        Ok(())
    }

    fn resolve_human_opening_hand(
        &mut self,
        human_seat: usize,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        if human_seat >= self.players.len() {
            return Err(format!(
                "seed {} human opening seat {} is outside {} players",
                self.seed,
                human_seat + 1,
                self.players.len()
            ));
        }
        for (seat, player) in self.players.clone().into_iter().enumerate() {
            if seat != human_seat {
                self.dispatch(Action::KeepOpeningHand {
                    player,
                    bottom: Vec::new(),
                })?;
                continue;
            }
            loop {
                let context = self.opening_hand_context(player)?;
                let labels = context
                    .options()
                    .iter()
                    .map(|option| match option.descriptor() {
                        DecisionDescriptor::TakeMulligan => Ok("Take a mulligan".to_owned()),
                        DecisionDescriptor::KeepOpeningHand { bottom } if bottom.is_empty() => {
                            Ok("Keep this opening hand".to_owned())
                        }
                        DecisionDescriptor::KeepOpeningHand { bottom } => Ok(format!(
                            "Keep and put on bottom, in order: {}",
                            bottom
                                .iter()
                                .map(|object| self.object_name(*object))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                        descriptor => Err(format!(
                            "seed {} opening prompt cannot label descriptor {descriptor:?}",
                            self.seed
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let selected_id =
                    self.prompt_context_choice(source, "Choose opening hand", &context, &labels)?;
                let selected = context.select(selected_id).map_err(|error| {
                    format!(
                        "seed {} human selected illegal opening action: {error}",
                        self.seed
                    )
                })?;
                let kept = matches!(
                    selected.descriptor(),
                    DecisionDescriptor::KeepOpeningHand { .. }
                );
                for action in selected.actions().to_vec() {
                    self.dispatch(action)?;
                }
                if kept {
                    break;
                }
            }
        }
        Ok(())
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
        Arc::make_mut(&mut self.programs).insert(object, Arc::clone(&card.program));
        let definition = SearchCardDefinition {
            definition: HiddenCardDefinition::new(
                card_id,
                card.program.base_object(),
                card.program.base_creature(),
                card.color_identity,
            ),
            program: Arc::clone(&card.program),
        };
        if Arc::make_mut(&mut self.card_definitions)
            .insert(card_id, definition)
            .is_some()
        {
            return Err(format!(
                "seed {} reused physical card ID {}",
                self.seed,
                card_id.get()
            ));
        }
        Ok(object)
    }

    fn search_clone(&self) -> Self {
        let mut clone = self.clone();
        clone.trace = TraceMode::Off;
        clone.coverage_target = None;
        clone.metrics = GameMetrics::default();
        clone.actions.clear();
        clone.ai_decisions.clear();
        clone.next_hidden_check_action = u64::MAX;
        clone.next_invariant_check_action = u64::MAX;
        clone
    }

    fn determinize_for_search(&self, observer: PlayerId, seed: u64) -> Result<Self, String> {
        let view = self
            .state
            .player_view(observer)
            .map_err(|error| format!("seed {} search view failed: {error:?}", self.seed))?;
        let sample = Determinizer::new(seed)
            .sample(&view, self.deck_models.as_slice())
            .map_err(|error| format!("seed {} determinization failed: {error}", self.seed))?;
        let mut slots = Vec::with_capacity(sample.assignments().len());
        let mut program_updates = Vec::with_capacity(sample.assignments().len());
        for assignment in sample.assignments() {
            let definition = self
                .card_definitions
                .get(&assignment.card())
                .ok_or_else(|| {
                    format!(
                        "seed {} sampled unknown physical card {}",
                        self.seed,
                        assignment.card().get()
                    )
                })?;
            let slot = u32::try_from(assignment.slot()).map_err(|_| {
                format!(
                    "seed {} determinization slot {} does not fit u32",
                    self.seed,
                    assignment.slot()
                )
            })?;
            let object = self
                .state
                .zone_objects(assignment.zone())
                .and_then(|objects| objects.get(assignment.slot()))
                .copied()
                .ok_or_else(|| {
                    format!(
                        "seed {} sampled missing slot {:?}/{}",
                        self.seed,
                        assignment.zone(),
                        assignment.slot()
                    )
                })?;
            slots.push(HiddenSlotDefinition::new(
                assignment.zone(),
                slot,
                definition.definition,
            ));
            program_updates.push((object, Arc::clone(&definition.program)));
        }

        let mut clone = self.search_clone();
        clone.state = self
            .state
            .determinized_clone(observer, &slots)
            .map_err(|error| {
                format!(
                    "seed {} failed to bind determinized state: {error:?}",
                    self.seed
                )
            })?;
        let programs = Arc::make_mut(&mut clone.programs);
        for (object, program) in program_updates {
            programs.insert(object, program);
        }
        Ok(clone)
    }

    fn run(self, max_turns: u32) -> Result<GameRun, String> {
        self.run_controlled(max_turns, None, None, None)
    }

    fn run_human(
        self,
        max_turns: u32,
        human: PlayerId,
        decisions: &mut dyn DecisionSource,
    ) -> Result<GameRun, String> {
        self.run_controlled(max_turns, Some(human), Some(decisions), None)
    }

    fn run_ai(self, max_turns: u32, policies: SeatPolicies) -> Result<GameRun, String> {
        self.run_controlled(max_turns, None, None, Some(policies))
    }

    fn run_controlled(
        mut self,
        max_turns: u32,
        human: Option<PlayerId>,
        mut decisions: Option<&mut dyn DecisionSource>,
        ai_policies: Option<SeatPolicies>,
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
                    } else if let Some(policies) = ai_policies.as_ref() {
                        let policy = self.policy_for(active, policies)?;
                        self.take_ai_main_phase_actions(active, policy)?;
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
                    } else if let Some(policies) = ai_policies.as_ref() {
                        let policy = self.policy_for(active, policies)?;
                        self.declare_ai_attackers(active, policy)?;
                    } else {
                        self.declare_attackers(active)?;
                    }
                }
                Step::DeclareBlockers if active_has_priority && blockers_done.insert(turn) => {
                    self.check_hidden_information()?;
                    for defender in self.current_defending_players(active) {
                        if Some(defender) == human {
                            let source = decisions.as_deref_mut().ok_or_else(|| {
                                "human game is missing a decision source".to_owned()
                            })?;
                            self.declare_human_blocks(defender, source)?;
                        } else if let Some(policies) = ai_policies.as_ref() {
                            let policy = self.policy_for(defender, policies)?;
                            self.declare_ai_blocks(defender, policy)?;
                        } else {
                            self.declare_blocks(defender)?;
                        }
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
                        } else if let Some(policies) = ai_policies.as_ref() {
                            self.choose_dead_commanders_with_ai(policies)?;
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
            ai_decisions: self.ai_decisions,
        })
    }

    fn policy_for(
        &self,
        player: PlayerId,
        policies: &SeatPolicies,
    ) -> Result<AiController, String> {
        let seat = self
            .players
            .iter()
            .position(|candidate| *candidate == player)
            .ok_or_else(|| format!("seed {} AI actor is outside the pod", self.seed))?;
        Ok(policies[seat])
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

    fn prompt_context_choice(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
    ) -> Result<CanonicalActionId, String> {
        let view = self
            .state
            .player_view(context.actor())
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        let selected = source.choose(&DecisionPrompt {
            kind,
            view: &view,
            context,
            options,
        })?;
        if selected >= options.len() {
            return Err(format!(
                "seed {} prompt `{kind}` returned option {selected} outside {} choices",
                self.seed,
                options.len()
            ));
        }
        let selected = context.options().get(selected).ok_or_else(|| {
            format!(
                "seed {} prompt `{kind}` selected canonical option {selected} outside {} choices",
                self.seed,
                context.options().len()
            )
        })?;
        context.select(selected.id()).map_err(|error| {
            format!(
                "seed {} prompt `{kind}` selected an illegal canonical action: {error}",
                self.seed
            )
        })?;
        Ok(selected.id())
    }

    fn prompt_legacy_choice(
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
        let selected = source.choose_legacy(&LegacyDecisionPrompt {
            kind,
            view: &view,
            options,
        })?;
        if selected >= options.len() {
            return Err(format!(
                "seed {} legacy prompt `{kind}` returned option {selected} outside {} choices",
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
        if source.is_legacy_replay() {
            return self.take_legacy_human_main_phase_actions(player, source);
        }
        loop {
            let (context, mappings) = self.main_decision_context(player)?;
            let labels = context
                .options()
                .iter()
                .map(|option| self.main_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id = self.prompt_context_choice(
                source,
                "Choose a main-phase action",
                &context,
                &labels,
            )?;
            let choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then_some(*choice))
                .ok_or_else(|| {
                    format!(
                        "seed {} human main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.apply_main_choice(player, choice)? {
                return Ok(());
            }
        }
    }

    fn take_legacy_human_main_phase_actions(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        loop {
            let (labels, choices) = self.legacy_human_main_choices(player)?;
            let selected =
                self.prompt_legacy_choice(player, source, "Choose a main-phase action", &labels)?;
            if self.apply_main_choice(player, choices[selected])? {
                return Ok(());
            }
        }
    }

    fn main_choice_label(&self, descriptor: &DecisionDescriptor) -> Result<String, String> {
        match descriptor {
            DecisionDescriptor::PlayLand { object } => {
                Ok(format!("Play land: {}", self.object_name(*object)))
            }
            DecisionDescriptor::ActivateAbility {
                source, payment, ..
            } => Ok(format!(
                "Activate ability: {} (payment waste {})",
                self.object_name(*source),
                payment.waste_score()
            )),
            DecisionDescriptor::CastSpell {
                object, payment, ..
            } => Ok(format!(
                "Cast: {} (payment waste {})",
                self.object_name(*object),
                payment.waste_score()
            )),
            DecisionDescriptor::PassPriority => Ok("Finish main phase".to_owned()),
            other => Err(format!(
                "seed {} main prompt cannot label descriptor {other:?}",
                self.seed
            )),
        }
    }

    fn apply_main_choice(&mut self, player: PlayerId, choice: MainChoice) -> Result<bool, String> {
        match choice {
            MainChoice::PlayLand(object) => {
                self.dispatch(Action::PlayLand { player, object })?;
                self.metrics.lands_played = self.metrics.lands_played.saturating_add(1);
                if let Some(exercise) = self.identity_exercise_mut(object) {
                    exercise.land_plays = exercise.land_plays.saturating_add(1);
                }
                self.register_permanent_runtime(player, object)?;
                Ok(false)
            }
            MainChoice::ActivateAll => {
                self.activate_mana_sources(player)?;
                Ok(false)
            }
            MainChoice::Activate {
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
                    return Ok(true);
                }
                if self.state.object(ability_source).is_none() {
                    return Err(format!(
                        "seed {} activated source disappeared without a stack transition",
                        self.seed
                    ));
                }
                Ok(false)
            }
            MainChoice::Cast { object, payment } => {
                self.cast_permanent_with_payment(player, object, payment)?;
                Ok(true)
            }
            MainChoice::Finish => Ok(true),
        }
    }

    fn human_main_choices(
        &self,
        player: PlayerId,
    ) -> Result<(Vec<String>, Vec<MainChoice>), String> {
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
                    choices.push(MainChoice::PlayLand(object));
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
            let payments = self
                .state
                .payment_plans_for_player(player, cost.mana())
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?;
            for payment in payments.plans().iter().copied() {
                let action = Action::ActivateAbility {
                    player,
                    ability,
                    payment,
                };
                if self.action_is_legal(&action) {
                    activations.push((ability_source, ability, payment));
                }
            }
        }
        if !activations.is_empty() {
            labels.push("Activate all available mana sources".to_owned());
            choices.push(MainChoice::ActivateAll);
            for (ability_source, ability, payment) in activations {
                labels.push(format!(
                    "Activate ability: {} (payment waste {})",
                    self.object_name(ability_source),
                    payment.waste_score()
                ));
                choices.push(MainChoice::Activate {
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
            let payments = self
                .state
                .payment_plans_for_player(player, cost)
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?;
            for payment in payments.plans().iter().copied() {
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
                    labels.push(format!(
                        "Cast: {} (payment waste {})",
                        program.name(),
                        payment.waste_score()
                    ));
                    choices.push(MainChoice::Cast { object, payment });
                }
            }
        }

        labels.push("Finish main phase".to_owned());
        choices.push(MainChoice::Finish);
        Ok((labels, choices))
    }

    // Frozen adapter for exact replay of forge-human-play-replay-v1 artifacts
    // recorded before canonical contexts enumerated every payment plan.
    fn legacy_human_main_choices(
        &self,
        player: PlayerId,
    ) -> Result<(Vec<String>, Vec<MainChoice>), String> {
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
                    choices.push(MainChoice::PlayLand(object));
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
            choices.push(MainChoice::ActivateAll);
            for (ability_source, ability, payment) in activations {
                labels.push(format!(
                    "Activate ability: {}",
                    self.object_name(ability_source)
                ));
                choices.push(MainChoice::Activate {
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
                choices.push(MainChoice::Cast { object, payment });
            }
        }

        labels.push("Finish main phase".to_owned());
        choices.push(MainChoice::Finish);
        Ok((labels, choices))
    }

    fn take_ai_main_phase_actions(
        &mut self,
        player: PlayerId,
        policy: AiController,
    ) -> Result<(), String> {
        loop {
            let decision_started = Instant::now();
            let (context, mappings) = self.main_decision_context(player)?;
            let (selected_id, decision, policy_name, candidates, search_report) = match policy {
                AiController::Search(controller) => {
                    let decision_index = self.ai_decisions.len() as u64;
                    let domain = MainSearchDomain {
                        root: self,
                        actor: player,
                        weights: controller.weights,
                        rollout_seed: controller.seed ^ decision_index,
                        guardrail_profile: controller.guardrail_profile,
                    };
                    let report =
                        SearchEngine::search(&domain, &context, &controller.config(decision_index))
                            .map_err(|error| {
                                format!("seed {} main search failed: {error}", self.seed)
                            })?;
                    (
                        report.selected_action(),
                        None,
                        "determinized-uct-v1",
                        Vec::new(),
                        Some(report),
                    )
                }
                AiController::Heuristic(_) | AiController::Random(_) => {
                    let candidates = match policy {
                        AiController::Heuristic(_) => {
                            let profile = policy.guardrail_profile().ok_or_else(|| {
                                format!(
                                    "seed {} heuristic policy has no guardrail profile",
                                    self.seed
                                )
                            })?;
                            self.policy_candidates(&context, player, |option| {
                                self.main_action_prior(&context, option.descriptor(), profile)
                            })?
                        }
                        AiController::Random(_) => Vec::new(),
                        AiController::Search(_) => unreachable!(),
                    };
                    let (selected_id, decision, policy_name) =
                        self.select_ai_action(policy, &context, &candidates, "main")?;
                    (selected_id, decision, policy_name, candidates, None)
                }
            };
            context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} AI selected illegal main action: {error}",
                    self.seed
                )
            })?;
            if let Some(report) = search_report.as_ref() {
                self.record_search_decision(
                    "main_phase",
                    policy_name,
                    &context,
                    report,
                    matches!(policy, AiController::Search(controller) if controller.adaptive),
                    elapsed_us(decision_started),
                );
            } else {
                self.record_ai_decision(AiDecisionTelemetry {
                    kind: "main_phase",
                    policy: policy_name,
                    context: &context,
                    action_id: selected_id,
                    decision,
                    evaluated_candidates: candidates.len(),
                    wall_latency_us: elapsed_us(decision_started),
                    score_override: None,
                    stop_reason: if decision.is_some() {
                        "one_ply_complete"
                    } else {
                        "random_legal_selection"
                    },
                });
            }
            let choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then_some(*choice))
                .ok_or_else(|| {
                    format!(
                        "seed {} AI main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.apply_main_choice(player, choice)? {
                return Ok(());
            }
        }
    }

    fn main_decision_context(
        &self,
        player: PlayerId,
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        let (_, choices) = self.human_main_choices(player)?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for choice in choices {
            if matches!(choice, MainChoice::ActivateAll) {
                continue;
            }
            let option = self.main_choice_option(player, choice)?;
            mappings.push((option.id(), choice));
            options.push(option);
        }
        let context = self.decision_context(DecisionKind::MainPhase, player, options)?;
        Ok((context, mappings))
    }

    fn main_choice_option(
        &self,
        player: PlayerId,
        choice: MainChoice,
    ) -> Result<DecisionOption, String> {
        match choice {
            MainChoice::PlayLand(object) => Ok(DecisionOption::new(
                DecisionDescriptor::PlayLand { object },
                vec![Action::PlayLand { player, object }],
            )),
            MainChoice::Activate {
                source,
                ability,
                payment,
            } => Ok(DecisionOption::new(
                DecisionDescriptor::ActivateAbility {
                    source,
                    ability,
                    payment,
                },
                vec![Action::ActivateAbility {
                    player,
                    ability,
                    payment,
                }],
            )),
            MainChoice::Cast { object, payment } => {
                let program = self.programs.get(&object).ok_or_else(|| {
                    format!("seed {} missing program for AI cast option", self.seed)
                })?;
                Ok(DecisionOption::new(
                    DecisionDescriptor::CastSpell {
                        object,
                        payment,
                        targets: Vec::new(),
                        modes: Vec::new(),
                        optional: Vec::new(),
                    },
                    vec![Action::CastSpell {
                        player,
                        object,
                        request: CastSpellRequest::new(
                            StackObjectKind::PermanentSpell,
                            SpellTiming::Sorcery,
                            program.mana_cost(),
                            payment,
                        ),
                    }],
                ))
            }
            MainChoice::Finish => Ok(DecisionOption::new(
                DecisionDescriptor::PassPriority,
                vec![Action::PassPriority { player }],
            )),
            MainChoice::ActivateAll => Err(format!(
                "seed {} grouped ActivateAll presentation choice is not canonical",
                self.seed
            )),
        }
    }

    fn decision_context(
        &self,
        kind: DecisionKind,
        actor: PlayerId,
        options: Vec<DecisionOption>,
    ) -> Result<DecisionContext, String> {
        let view = self
            .state
            .player_view(actor)
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        DecisionContext::new(kind, actor, &view, options, Vec::new())
            .map_err(|error| format!("seed {} decision context failed: {error}", self.seed))
    }

    fn policy_candidates(
        &self,
        context: &DecisionContext,
        actor: PlayerId,
        prior: impl Fn(&DecisionOption) -> i64,
    ) -> Result<Vec<PolicyCandidate>, String> {
        context
            .options()
            .iter()
            .map(|option| {
                let mut state = self.state.clone();
                for action in option.actions() {
                    let outcome = apply(&mut state, action.clone());
                    if let Outcome::Failed(error) = outcome {
                        return Err(format!(
                            "seed {} canonical action {} failed preview: {error:?}",
                            self.seed,
                            option.id()
                        ));
                    }
                }
                let view = state.player_view(actor).map_err(|error| {
                    format!("seed {} successor player view failed: {error:?}", self.seed)
                })?;
                Ok(PolicyCandidate::new(option.id(), view, prior(option)))
            })
            .collect()
    }

    fn select_ai_action(
        &self,
        policy: AiController,
        context: &DecisionContext,
        candidates: &[PolicyCandidate],
        kind: &'static str,
    ) -> Result<(CanonicalActionId, Option<PolicyDecision>, &'static str), String> {
        match policy {
            AiController::Heuristic(policy) => {
                let decision = policy.select(candidates).map_err(|error| {
                    format!("seed {} AI {kind} decision failed: {error}", self.seed)
                })?;
                Ok((
                    candidates[decision.index()].action_id(),
                    Some(decision),
                    "heuristic-v1",
                ))
            }
            AiController::Random(policy) => policy
                .select(context, self.ai_decisions.len() as u64)
                .map(|action| (action, None, "random-legal-v1"))
                .map_err(|error| {
                    format!("seed {} random {kind} decision failed: {error}", self.seed)
                }),
            AiController::Search(controller) => {
                let policy = controller.rollout(self.ai_decisions.len() as u64);
                let decision = policy.select(candidates).map_err(|error| {
                    format!(
                        "seed {} AI {kind} search fallback failed: {error}",
                        self.seed
                    )
                })?;
                Ok((
                    candidates[decision.index()].action_id(),
                    Some(decision),
                    "search-rollout-v1",
                ))
            }
        }
    }

    fn record_ai_decision(&mut self, telemetry: AiDecisionTelemetry<'_>) {
        let AiDecisionTelemetry {
            kind,
            policy,
            context,
            action_id,
            decision,
            evaluated_candidates,
            wall_latency_us,
            score_override,
            stop_reason,
        } = telemetry;
        let legal_actions = context.options().len();
        self.ai_decisions.push(AiDecisionRecord {
            index: self.ai_decisions.len() as u64,
            kind: kind.to_owned(),
            policy: policy.to_owned(),
            context_id: context.id().to_string(),
            decision_state_key: context.state_key().to_string(),
            player_view_hash: format!("{:016x}", context.player_view_hash().get()),
            action_id: action_id.to_string(),
            canonical_legal_actions: canonical_legal_actions(context),
            evaluation: decision.map_or(0, |value| value.evaluation().total()),
            prior: decision.map_or(0, PolicyDecision::prior),
            noise: decision.map_or(0, PolicyDecision::noise),
            score: score_override.unwrap_or_else(|| decision.map_or(0, PolicyDecision::score)),
            legal_actions: legal_actions as u32,
            evaluated_candidates: evaluated_candidates as u32,
            determinizations: 0,
            configured_iterations: 0,
            configured_wall_ms: 0,
            adaptive_search: false,
            think_ms: 0,
            simulations: 0,
            nodes: 0,
            maximum_depth: 0,
            transposition_hits: 0,
            value_gap: 0,
            visit_gap: 0,
            uncertainty_ppm: 0,
            leading_visit_share_ppm: 0,
            checkpoint_count: 0,
            ranking_stable: false,
            bounded_solver_state: "not_run".to_owned(),
            search_checkpoints: Vec::new(),
            actual_cpu_time_us: None,
            memory_delta_bytes: None,
            considered_actions: Vec::new(),
            wall_latency_us,
            stop_reason: if legal_actions == 1 {
                "singleton_legal_action".to_owned()
            } else {
                stop_reason.to_owned()
            },
        });
    }

    fn record_search_decision(
        &mut self,
        kind: &'static str,
        policy: &'static str,
        context: &DecisionContext,
        report: &SearchReport,
        adaptive_search: bool,
        wall_latency_us: u64,
    ) {
        let explanation = LastDecisionReport::from_search(report);
        let selected = report
            .actions()
            .iter()
            .find(|action| action.action() == report.selected_action());
        let (configured_iterations, configured_wall_ms) = match report.configured_limit() {
            SearchLimit::Iterations(iterations) => (iterations, 0),
            SearchLimit::WallTime(duration) => {
                (0, u32::try_from(duration.as_millis()).unwrap_or(u32::MAX))
            }
        };
        let total_visits = report
            .actions()
            .iter()
            .map(|action| action.visits())
            .sum::<u64>();
        let leading_visit_share_ppm = selected.map_or(0, |action| {
            if total_visits == 0 {
                0
            } else {
                u32::try_from(
                    u128::from(action.visits())
                        .saturating_mul(1_000_000)
                        .checked_div(u128::from(total_visits))
                        .unwrap_or(0),
                )
                .unwrap_or(1_000_000)
            }
        });
        let final_checkpoint = report.checkpoints().last();
        let bounded_solver_state = final_checkpoint.map_or_else(
            || match report.stop_reason() {
                SearchStopReason::CertifiedWin => "certified_win".to_owned(),
                SearchStopReason::CertifiedRequiredDefense => {
                    "certified_required_defense".to_owned()
                }
                _ => "not_certified".to_owned(),
            },
            |checkpoint| checkpoint.bounded_solver_state().to_owned(),
        );
        self.ai_decisions.push(AiDecisionRecord {
            index: self.ai_decisions.len() as u64,
            kind: kind.to_owned(),
            policy: policy.to_owned(),
            context_id: context.id().to_string(),
            decision_state_key: context.state_key().to_string(),
            player_view_hash: format!("{:016x}", context.player_view_hash().get()),
            action_id: report.selected_action().to_string(),
            canonical_legal_actions: canonical_legal_actions(context),
            evaluation: selected.map_or(0, |action| action.mean_value()),
            prior: 0,
            noise: 0,
            score: selected.map_or(0, |action| action.mean_value()),
            legal_actions: context.options().len() as u32,
            evaluated_candidates: report.actions().len() as u32,
            determinizations: report.determinizations(),
            configured_iterations,
            configured_wall_ms,
            adaptive_search,
            think_ms: configured_wall_ms,
            simulations: report.simulations(),
            nodes: report.nodes(),
            maximum_depth: report.maximum_depth(),
            transposition_hits: report.transposition_hits(),
            value_gap: report.value_gap(),
            visit_gap: report.visit_gap(),
            uncertainty_ppm: report.uncertainty_ppm(),
            leading_visit_share_ppm,
            checkpoint_count: u32::try_from(report.checkpoints().len()).unwrap_or(u32::MAX),
            ranking_stable: final_checkpoint.is_some_and(|checkpoint| checkpoint.ranking_stable()),
            bounded_solver_state,
            search_checkpoints: report
                .checkpoints()
                .iter()
                .map(|checkpoint| AiSearchCheckpoint {
                    determinization: checkpoint.determinization(),
                    simulations: checkpoint.simulations(),
                    leading_action_id: checkpoint.leading_action().to_string(),
                    leading_visit_share_ppm: checkpoint.leading_visit_share_ppm(),
                    value_gap: checkpoint.value_gap(),
                    visit_gap: checkpoint.visit_gap(),
                    ranking_stable: checkpoint.ranking_stable(),
                    uncertainty_ppm: checkpoint.uncertainty_ppm(),
                    bounded_solver_state: checkpoint.bounded_solver_state().to_owned(),
                    stop_reason: checkpoint
                        .stop_reason()
                        .map(search_stop_reason)
                        .map(str::to_owned),
                })
                .collect(),
            actual_cpu_time_us: report.actual_cpu_time_us(),
            memory_delta_bytes: report.memory_delta_bytes(),
            considered_actions: explanation
                .considered()
                .iter()
                .map(|action| AiConsideredAction {
                    action_id: action.action().to_string(),
                    visits: action.visits(),
                    mean_value: action.mean_value(),
                    value_delta_from_selected: action.value_delta_from_selected(),
                })
                .collect(),
            wall_latency_us,
            stop_reason: search_stop_reason(report.stop_reason()).to_owned(),
        });
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

    fn attack_decision_context(&self, active: PlayerId) -> Result<DecisionContext, String> {
        let options = self
            .diagnostic_attack_candidates(active)?
            .into_iter()
            .map(|attacks| {
                DecisionOption::new(
                    DecisionDescriptor::DeclareAttackers {
                        attacks: attacks.clone(),
                    },
                    vec![Action::DeclareAttackers {
                        player: active,
                        attacks,
                    }],
                )
            })
            .collect();
        self.decision_context(DecisionKind::DeclareAttackers, active, options)
    }

    fn block_decision_context(&self, defending: PlayerId) -> Result<DecisionContext, String> {
        let options = self
            .diagnostic_block_candidates(defending)?
            .into_iter()
            .map(|blocks| {
                DecisionOption::new(
                    DecisionDescriptor::DeclareBlockers {
                        blocks: blocks.clone(),
                    },
                    vec![Action::DeclareBlockers {
                        defending_player: defending,
                        blocks,
                    }],
                )
            })
            .collect();
        self.decision_context(DecisionKind::DeclareBlockers, defending, options)
    }

    fn main_action_prior(
        &self,
        context: &DecisionContext,
        descriptor: &DecisionDescriptor,
        profile: GuardrailProfile,
    ) -> i64 {
        let risks = if matches!(descriptor, DecisionDescriptor::PassPriority)
            && context.options().len() > 1
        {
            ActionRisks::none().with(ActionRisk::PassWithDevelopment)
        } else {
            ActionRisks::none()
        };
        self.guardrails.penalty(profile, risks)
    }

    fn combat_action_prior(
        &self,
        descriptor: &DecisionDescriptor,
        weights: AiWeights,
        profile: GuardrailProfile,
    ) -> i64 {
        let (base, risks) = match descriptor {
            DecisionDescriptor::DeclareAttackers { attacks } => {
                let mut by_defender = BTreeMap::<usize, (PlayerId, i64, i64)>::new();
                for attack in attacks {
                    let power = self
                        .state
                        .creature_characteristics(attack.attacker())
                        .map_or(0, |creature| i64::from(creature.power().max(0)));
                    let entry = by_defender
                        .entry(attack.defending_player().index())
                        .or_insert((attack.defending_player(), 0, 0));
                    entry.1 = entry.1.saturating_add(power);
                    entry.2 = entry.2.saturating_add(1);
                }
                let base = by_defender.values().fold(
                    0_i64,
                    |score, (defender, total_power, attacker_count)| {
                        let defender_life = self.state.players()[defender.index()].life();
                        score.saturating_add(weights.attack_prior(
                            *total_power,
                            *attacker_count,
                            defender_life,
                        ))
                    },
                );
                (base, ActionRisks::none())
            }
            DecisionDescriptor::DeclareBlockers { blocks } => {
                let mut prevented_power = 0_i64;
                let mut favorable_trades = 0_i64;
                let mut losing_trades = 0_i64;
                for block in blocks {
                    let Ok(attacker) = self.state.creature_characteristics(block.attacker()) else {
                        continue;
                    };
                    let Ok(blocker) = self.state.creature_characteristics(block.blocker()) else {
                        continue;
                    };
                    prevented_power =
                        prevented_power.saturating_add(i64::from(attacker.power().max(0)));
                    if blocker.power() >= attacker.toughness() || blocker.keywords().deathtouch() {
                        favorable_trades += 1;
                    }
                    if attacker.power() >= blocker.toughness() || attacker.keywords().deathtouch() {
                        losing_trades += 1;
                    }
                }
                let risks = if losing_trades > favorable_trades {
                    ActionRisks::none().with(ActionRisk::UnfavorableCombatTrade)
                } else {
                    ActionRisks::none()
                };
                (
                    weights.block_prior(prevented_power, favorable_trades, losing_trades),
                    risks,
                )
            }
            _ => (0, ActionRisks::none()),
        };
        base.saturating_add(self.guardrails.penalty(profile, risks))
    }

    fn declare_ai_attackers(
        &mut self,
        active: PlayerId,
        policy: AiController,
    ) -> Result<(), String> {
        let decision_started = Instant::now();
        let context = self.attack_decision_context(active)?;
        let (selected_id, decision, policy_name, candidates, search_report) = match policy {
            AiController::Search(controller) => {
                let decision_index = self.ai_decisions.len() as u64;
                let domain = CombatSearchDomain {
                    root: self,
                    actor: active,
                    weights: controller.weights,
                    kind: CombatSearchKind::Attackers { active },
                    guardrail_profile: controller.guardrail_profile,
                };
                let report =
                    SearchEngine::search(&domain, &context, &controller.config(decision_index))
                        .map_err(|error| {
                            format!("seed {} attack search failed: {error}", self.seed)
                        })?;
                (
                    report.selected_action(),
                    None,
                    "determinized-uct-v1",
                    Vec::new(),
                    Some(report),
                )
            }
            AiController::Heuristic(_) | AiController::Random(_) => {
                let candidates = match policy.candidate_weights() {
                    Some(weights) => {
                        let profile = policy.guardrail_profile().ok_or_else(|| {
                            format!("seed {} combat policy has no guardrail profile", self.seed)
                        })?;
                        self.policy_candidates(&context, active, |option| {
                            self.combat_action_prior(option.descriptor(), weights, profile)
                        })?
                    }
                    None => Vec::new(),
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, &context, &candidates, "attack")?;
                (selected_id, decision, policy_name, candidates, None)
            }
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} AI selected illegal attack action: {error}",
                self.seed
            )
        })?;
        if let Some(report) = search_report.as_ref() {
            self.record_search_decision(
                "declare_attackers",
                policy_name,
                &context,
                report,
                matches!(policy, AiController::Search(controller) if controller.adaptive),
                elapsed_us(decision_started),
            );
        } else {
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "declare_attackers",
                policy: policy_name,
                context: &context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if decision.is_some() {
                    "one_ply_complete"
                } else {
                    "random_legal_selection"
                },
            });
        }
        let DecisionDescriptor::DeclareAttackers { attacks } = selected.descriptor() else {
            return Err(format!(
                "seed {} attack context returned a non-attack descriptor",
                self.seed
            ));
        };
        let attacks = attacks.clone();
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
        self.current_attacks = attacks;
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn diagnostic_attack_candidates(
        &self,
        active: PlayerId,
    ) -> Result<Vec<Vec<AttackDeclaration>>, String> {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let mut objects = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(active))
            .collect::<Vec<_>>();
        objects.sort_by_key(|object| object.index());
        let defenders = self.live_opponents(active);
        let mut candidates = vec![Vec::new()];
        for object in objects {
            let legal = defenders
                .iter()
                .copied()
                .map(|defender| AttackDeclaration::new(object, defender))
                .filter(|attack| self.state.can_attack(active, *attack))
                .collect::<Vec<_>>();
            if legal.is_empty() {
                continue;
            }
            let existing = candidates.len();
            let additional = existing.checked_mul(legal.len()).ok_or_else(|| {
                format!("seed {} attack surface option count overflowed", self.seed)
            })?;
            if existing.saturating_add(additional) > MAX_DIAGNOSTIC_COMBAT_OPTIONS {
                return Err(format!(
                    "seed {} attack surface exceeds the diagnostics-only limit of {} options",
                    self.seed, MAX_DIAGNOSTIC_COMBAT_OPTIONS
                ));
            }
            for index in 0..existing {
                for attack in &legal {
                    let mut with_attack = candidates[index].clone();
                    with_attack.push(*attack);
                    candidates.push(with_attack);
                }
            }
        }
        candidates.retain(|attacks| {
            self.action_is_legal(&Action::DeclareAttackers {
                player: active,
                attacks: attacks.clone(),
            })
        });
        if candidates.is_empty() {
            return Err(format!(
                "seed {} attack surface has no legal fallback",
                self.seed
            ));
        }
        Ok(candidates)
    }

    fn declare_human_attackers(
        &mut self,
        active: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        if source.is_legacy_replay() {
            return self.declare_legacy_human_attackers(active, source);
        }
        let context = self.attack_decision_context(active)?;
        let labels = context
            .options()
            .iter()
            .map(|option| self.attack_choice_label(option.descriptor()))
            .collect::<Result<Vec<_>, _>>()?;
        let selected_id =
            self.prompt_context_choice(source, "Choose attackers", &context, &labels)?;
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} human selected illegal attack action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::DeclareAttackers { attacks } = selected.descriptor() else {
            return Err(format!(
                "seed {} attack context returned a non-attack descriptor",
                self.seed
            ));
        };
        let attacks = attacks.clone();
        for action in selected.actions().to_vec() {
            self.dispatch(action)?;
        }
        self.current_attacks = attacks;
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn declare_legacy_human_attackers(
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
        let selected = self.prompt_legacy_choice(active, source, "Choose attackers", &labels)?;
        let attacks = candidates[selected].clone();
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
        self.current_attacks = attacks;
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn attack_choice_label(&self, descriptor: &DecisionDescriptor) -> Result<String, String> {
        let DecisionDescriptor::DeclareAttackers { attacks } = descriptor else {
            return Err(format!(
                "seed {} attack prompt cannot label descriptor {descriptor:?}",
                self.seed
            ));
        };
        if attacks.is_empty() {
            return Ok("Attack no one".to_owned());
        }
        let declarations = attacks
            .iter()
            .map(|attack| {
                format!(
                    "{} -> seat {}",
                    self.object_name(attack.attacker()),
                    attack.defending_player().index() + 1
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("Attack with {declarations}"))
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

    fn current_defending_players(&self, active: PlayerId) -> Vec<PlayerId> {
        let Some(start) = self.players.iter().position(|player| *player == active) else {
            return Vec::new();
        };
        (1..self.players.len())
            .map(|offset| self.players[(start + offset) % self.players.len()])
            .filter(|defender| {
                self.current_attacks
                    .iter()
                    .any(|attack| attack.defending_player() == *defender)
            })
            .collect()
    }

    fn declare_ai_blocks(
        &mut self,
        defending_player: PlayerId,
        policy: AiController,
    ) -> Result<(), String> {
        let decision_started = Instant::now();
        let context = self.block_decision_context(defending_player)?;
        let (selected_id, decision, policy_name, candidates, search_report) = match policy {
            AiController::Search(controller) => {
                let decision_index = self.ai_decisions.len() as u64;
                let domain = CombatSearchDomain {
                    root: self,
                    actor: defending_player,
                    weights: controller.weights,
                    kind: CombatSearchKind::Blockers {
                        defending: defending_player,
                    },
                    guardrail_profile: controller.guardrail_profile,
                };
                let report =
                    SearchEngine::search(&domain, &context, &controller.config(decision_index))
                        .map_err(|error| {
                            format!("seed {} block search failed: {error}", self.seed)
                        })?;
                (
                    report.selected_action(),
                    None,
                    "determinized-uct-v1",
                    Vec::new(),
                    Some(report),
                )
            }
            AiController::Heuristic(_) | AiController::Random(_) => {
                let candidates = match policy.candidate_weights() {
                    Some(weights) => {
                        let profile = policy.guardrail_profile().ok_or_else(|| {
                            format!("seed {} combat policy has no guardrail profile", self.seed)
                        })?;
                        self.policy_candidates(&context, defending_player, |option| {
                            self.combat_action_prior(option.descriptor(), weights, profile)
                        })?
                    }
                    None => Vec::new(),
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, &context, &candidates, "block")?;
                (selected_id, decision, policy_name, candidates, None)
            }
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} AI selected illegal block action: {error}",
                self.seed
            )
        })?;
        if let Some(report) = search_report.as_ref() {
            self.record_search_decision(
                "declare_blockers",
                policy_name,
                &context,
                report,
                matches!(policy, AiController::Search(controller) if controller.adaptive),
                elapsed_us(decision_started),
            );
        } else {
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "declare_blockers",
                policy: policy_name,
                context: &context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if decision.is_some() {
                    "one_ply_complete"
                } else {
                    "random_legal_selection"
                },
            });
        }
        let DecisionDescriptor::DeclareBlockers { blocks } = selected.descriptor() else {
            return Err(format!(
                "seed {} block context returned a non-block descriptor",
                self.seed
            ));
        };
        self.dispatch(Action::DeclareBlockers {
            defending_player,
            blocks: blocks.clone(),
        })?;
        Ok(())
    }

    fn diagnostic_block_candidates(
        &self,
        defending_player: PlayerId,
    ) -> Result<Vec<Vec<BlockDeclaration>>, String> {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let mut blockers = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(defending_player))
            .collect::<Vec<_>>();
        blockers.sort_by_key(|object| object.index());
        let mut variants = vec![Vec::new()];
        for blocker in blockers {
            let legal = self
                .current_attacks
                .iter()
                .filter(|attack| attack.defending_player() == defending_player)
                .map(|attack| BlockDeclaration::new(blocker, attack.attacker()))
                .filter(|block| self.state.can_block(defending_player, *block))
                .collect::<Vec<_>>();
            let existing = variants.clone();
            for blocks in existing {
                for block in &legal {
                    if variants.len() >= MAX_DIAGNOSTIC_COMBAT_OPTIONS {
                        return Err(format!(
                            "seed {} block surface exceeds the diagnostics-only limit of {} options",
                            self.seed, MAX_DIAGNOSTIC_COMBAT_OPTIONS
                        ));
                    }
                    let mut with_block = blocks.clone();
                    with_block.push(*block);
                    variants.push(with_block);
                }
            }
        }
        variants.retain(|blocks| {
            self.action_is_legal(&Action::DeclareBlockers {
                defending_player,
                blocks: blocks.clone(),
            })
        });
        if variants.is_empty() {
            return Err(format!(
                "seed {} block surface has no legal fallback",
                self.seed
            ));
        }
        Ok(variants)
    }

    fn declare_human_blocks(
        &mut self,
        defending_player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        if source.is_legacy_replay() {
            return self.declare_legacy_human_blocks(defending_player, source);
        }
        let context = self.block_decision_context(defending_player)?;
        let labels = context
            .options()
            .iter()
            .map(|option| self.block_choice_label(option.descriptor()))
            .collect::<Result<Vec<_>, _>>()?;
        let selected_id =
            self.prompt_context_choice(source, "Choose blockers", &context, &labels)?;
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} human selected illegal block action: {error}",
                self.seed
            )
        })?;
        for action in selected.actions().to_vec() {
            self.dispatch(action)?;
        }
        Ok(())
    }

    fn declare_legacy_human_blocks(
        &mut self,
        defending_player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
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
            for attack in self
                .current_attacks
                .iter()
                .filter(|attack| attack.defending_player() == defending_player)
            {
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
        let selected =
            self.prompt_legacy_choice(defending_player, source, "Choose blockers", &labels)?;
        self.dispatch(Action::DeclareBlockers {
            defending_player,
            blocks: candidates[selected].clone(),
        })?;
        Ok(())
    }

    fn block_choice_label(&self, descriptor: &DecisionDescriptor) -> Result<String, String> {
        let DecisionDescriptor::DeclareBlockers { blocks } = descriptor else {
            return Err(format!(
                "seed {} block prompt cannot label descriptor {descriptor:?}",
                self.seed
            ));
        };
        if blocks.is_empty() {
            return Ok("Block no attackers".to_owned());
        }
        let declarations = blocks
            .iter()
            .map(|block| {
                format!(
                    "{} -> {}",
                    self.object_name(block.blocker()),
                    self.object_name(block.attacker())
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("Block with {declarations}"))
    }

    fn declare_blocks(&mut self, defending_player: PlayerId) -> Result<(), String> {
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
        let mut attacks = self
            .current_attacks
            .iter()
            .copied()
            .filter(|attack| attack.defending_player() == defending_player)
            .collect::<Vec<_>>();
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
            blocks,
        })?;
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
            let context = self.commander_zone_context(owner, commander, zone)?;
            let actions = context.options()[0].actions().to_vec();
            for action in actions {
                self.dispatch(action)?;
            }
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
            let context = self.commander_zone_context(owner, commander, zone)?;
            if owner == human {
                if source.is_legacy_replay() {
                    let labels = vec![
                        format!("Move {} to the command zone", self.object_name(commander)),
                        format!("Leave {} in {zone:?}", self.object_name(commander)),
                    ];
                    let selected = self.prompt_legacy_choice(
                        human,
                        source,
                        "Choose whether to move your commander",
                        &labels,
                    )?;
                    let selected = context
                        .options()
                        .iter()
                        .find(|option| match (selected, option.descriptor()) {
                            (0, DecisionDescriptor::MoveCommanderToCommand { object }) => {
                                *object == commander
                            }
                            (
                                1,
                                DecisionDescriptor::LeaveCommander {
                                    object,
                                    zone: choice,
                                },
                            ) => *object == commander && *choice == zone,
                            _ => false,
                        })
                        .ok_or_else(|| {
                            format!("seed {} legacy commander option disappeared", self.seed)
                        })?;
                    let moved = matches!(
                        selected.descriptor(),
                        DecisionDescriptor::MoveCommanderToCommand { object }
                            if *object == commander
                    );
                    for action in selected.actions().to_vec() {
                        self.dispatch(action)?;
                    }
                    if moved {
                        self.metrics.commander_zone_returns =
                            self.metrics.commander_zone_returns.saturating_add(1);
                    }
                    continue;
                }
                let labels = context
                    .options()
                    .iter()
                    .map(|option| match option.descriptor() {
                        DecisionDescriptor::MoveCommanderToCommand { object } => Ok(format!(
                            "Move {} to the command zone",
                            self.object_name(*object)
                        )),
                        DecisionDescriptor::LeaveCommander { object, zone } => {
                            Ok(format!("Leave {} in {zone:?}", self.object_name(*object)))
                        }
                        descriptor => Err(format!(
                            "seed {} commander prompt cannot label descriptor {descriptor:?}",
                            self.seed
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let selected_id = self.prompt_context_choice(
                    source,
                    "Choose whether to move your commander",
                    &context,
                    &labels,
                )?;
                let selected = context.select(selected_id).map_err(|error| {
                    format!(
                        "seed {} illegal commander-zone selection: {error}",
                        self.seed
                    )
                })?;
                let moved = matches!(
                    selected.descriptor(),
                    DecisionDescriptor::MoveCommanderToCommand { .. }
                );
                let actions = selected.actions().to_vec();
                for action in actions {
                    self.dispatch(action)?;
                }
                if moved {
                    self.metrics.commander_zone_returns =
                        self.metrics.commander_zone_returns.saturating_add(1);
                }
                continue;
            }
            let actions = context.options()[0].actions().to_vec();
            for action in actions {
                self.dispatch(action)?;
            }
            self.metrics.commander_zone_returns =
                self.metrics.commander_zone_returns.saturating_add(1);
        }
        Ok(())
    }

    fn choose_dead_commanders_with_ai(&mut self, policies: &SeatPolicies) -> Result<(), String> {
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
            let context = self.commander_zone_context(owner, commander, zone)?;
            let policy = policies[seat];
            let decision_started = Instant::now();
            let candidates = if let Some(profile) = policy.guardrail_profile() {
                self.policy_candidates(&context, owner, |option| {
                    if matches!(
                        option.descriptor(),
                        DecisionDescriptor::LeaveCommander { .. }
                    ) {
                        self.guardrails.penalty(
                            profile,
                            ActionRisks::none().with(ActionRisk::UnnecessarySacrifice),
                        )
                    } else {
                        0
                    }
                })?
            } else {
                Vec::new()
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, &context, &candidates, "commander zone")?;
            let selected = context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} illegal commander-zone selection: {error}",
                    self.seed
                )
            })?;
            let moved = matches!(
                selected.descriptor(),
                DecisionDescriptor::MoveCommanderToCommand { .. }
            );
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "commander_zone",
                policy: policy_name,
                context: &context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if decision.is_some() {
                    "one_ply_complete"
                } else {
                    "random_legal_selection"
                },
            });
            let actions = selected.actions().to_vec();
            for action in actions {
                self.dispatch(action)?;
            }
            if moved {
                self.metrics.commander_zone_returns =
                    self.metrics.commander_zone_returns.saturating_add(1);
            }
        }
        Ok(())
    }

    fn commander_zone_context(
        &self,
        owner: PlayerId,
        commander: ObjectId,
        zone: ZoneId,
    ) -> Result<DecisionContext, String> {
        self.decision_context(
            DecisionKind::CommanderZone,
            owner,
            vec![
                DecisionOption::new(
                    DecisionDescriptor::MoveCommanderToCommand { object: commander },
                    vec![Action::ChooseCommanderZone {
                        player: owner,
                        object: commander,
                    }],
                ),
                DecisionOption::new(
                    DecisionDescriptor::LeaveCommander {
                        object: commander,
                        zone,
                    },
                    Vec::new(),
                ),
            ],
        )
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
                if owner == observer && hand.objects().iter().any(|object| object.is_hidden()) {
                    return Err(format!(
                        "seed {} actor hand is hidden from observer {}",
                        self.seed,
                        observer.index()
                    ));
                }
                let library = view
                    .zone(ZoneId::new(Some(*owner), ZoneKind::Library))
                    .ok_or_else(|| format!("seed {} visible library missing", self.seed))?;
                let library_id = ZoneId::new(Some(*owner), ZoneKind::Library);
                for (index, object) in library.objects().iter().enumerate() {
                    if let Some(record) = object.known() {
                        if self.state.object_zone(record.id()) != Some(library_id) {
                            return Err(format!(
                                "seed {} known library slot {} is outside the canonical library",
                                self.seed, index
                            ));
                        }
                    }
                }
            }
            self.metrics.hidden_information_checks =
                self.metrics.hidden_information_checks.saturating_add(1);
        }
        Ok(())
    }

    fn check_initial_hidden_information(&mut self) -> Result<(), String> {
        for observer in &self.players {
            let view = self
                .state
                .player_view(*observer)
                .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
            for owner in &self.players {
                let hand = view
                    .zone(ZoneId::new(Some(*owner), ZoneKind::Hand))
                    .ok_or_else(|| format!("seed {} initial hand missing", self.seed))?;
                let should_hide = owner != observer;
                if hand
                    .objects()
                    .iter()
                    .any(|object| object.is_hidden() != should_hide)
                {
                    return Err(format!(
                        "seed {} initial hidden-hand canary failed for observer {} hand {}",
                        self.seed,
                        observer.index(),
                        owner.index()
                    ));
                }
                let library = view
                    .zone(ZoneId::new(Some(*owner), ZoneKind::Library))
                    .ok_or_else(|| format!("seed {} initial library missing", self.seed))?;
                if library
                    .objects()
                    .iter()
                    .any(|object| !matches!(object, ObjectView::Hidden))
                {
                    return Err(format!(
                        "seed {} initial library leaked to observer {}",
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

impl MainSearchDomain<'_> {
    fn state(&self, driver: GameDriver) -> Result<MainSearchState, String> {
        let (context, mappings) = driver.main_decision_context(self.actor)?;
        let priors = context
            .options()
            .iter()
            .map(|option| {
                (
                    option.id(),
                    driver.main_action_prior(&context, option.descriptor(), self.guardrail_profile),
                )
            })
            .collect();
        Ok(MainSearchState {
            driver,
            finished: false,
            context: Arc::new(context),
            mappings: Arc::new(mappings),
            priors: Arc::new(priors),
        })
    }

    fn value(&self, state: &MainSearchState) -> i64 {
        state
            .driver
            .state
            .player_view(self.actor)
            .ok()
            .and_then(|view| self.weights.evaluate(&view).ok())
            .map_or(0, |evaluation| evaluation.total())
    }
}

impl SearchDomain for MainSearchDomain<'_> {
    type State = MainSearchState;

    fn determinize(&self, seed: u64) -> Result<Self::State, String> {
        self.state(self.root.determinize_for_search(self.actor, seed)?)
    }

    fn legal_actions(&self, state: &Self::State) -> Result<Vec<CanonicalActionId>, String> {
        if state.finished {
            return Ok(Vec::new());
        }
        Ok(state
            .context
            .options()
            .iter()
            .map(|option| option.id())
            .collect())
    }

    fn apply_action(
        &self,
        state: &Self::State,
        action: CanonicalActionId,
    ) -> Result<Self::State, String> {
        let choice = state
            .mappings
            .iter()
            .find_map(|(id, choice)| (*id == action).then_some(*choice))
            .ok_or_else(|| format!("search action {action} has no typed main-phase adapter"))?;
        let mut next = state.clone();
        next.finished = next.driver.apply_main_choice(self.actor, choice)?;
        if next.finished {
            Ok(next)
        } else {
            self.state(next.driver)
        }
    }

    fn terminal_value(&self, state: &Self::State) -> Option<i64> {
        (state.finished || state.driver.state.game_outcome() != GameOutcome::InProgress)
            .then(|| self.value(state))
    }

    fn evaluate(&self, state: &Self::State) -> i64 {
        self.value(state)
    }

    fn rollout_action(
        &self,
        state: &Self::State,
        actions: &[CanonicalActionId],
        seed: u64,
    ) -> Result<CanonicalActionId, String> {
        if actions.len() == 1 {
            return Ok(actions[0]);
        }
        let candidates = state
            .driver
            .policy_candidates(&state.context, self.actor, |option| {
                state.priors.get(&option.id()).copied().unwrap_or(0)
            })?;
        let decision = HeuristicPolicy::rollout(self.weights, self.rollout_seed ^ seed)
            .select(&candidates)
            .map_err(|error| format!("search rollout policy failed: {error}"))?;
        let selected = candidates[decision.index()].action_id();
        if !actions.contains(&selected) {
            return Err(format!(
                "search rollout selected {selected} outside the supplied legal set"
            ));
        }
        Ok(selected)
    }

    fn action_prior(&self, state: &Self::State, action: CanonicalActionId) -> i64 {
        state.priors.get(&action).copied().unwrap_or(0)
    }

    fn state_key(&self, state: &Self::State) -> Option<u64> {
        let terminal_marker = if state.finished {
            0xa076_1d64_78bd_642f
        } else {
            0
        };
        Some(
            state
                .driver
                .state
                .deterministic_hash()
                .get()
                .wrapping_add(terminal_marker),
        )
    }
}

impl CombatSearchDomain<'_> {
    fn context(&self, driver: &GameDriver) -> Result<DecisionContext, String> {
        match self.kind {
            CombatSearchKind::Attackers { active } => driver.attack_decision_context(active),
            CombatSearchKind::Blockers { defending } => driver.block_decision_context(defending),
        }
    }

    fn value(&self, state: &CombatSearchState) -> i64 {
        state
            .driver
            .state
            .player_view(self.actor)
            .ok()
            .and_then(|view| self.weights.evaluate(&view).ok())
            .map_or(0, |evaluation| evaluation.total())
            .saturating_add(state.terminal_prior)
    }
}

impl SearchDomain for CombatSearchDomain<'_> {
    type State = CombatSearchState;

    fn determinize(&self, seed: u64) -> Result<Self::State, String> {
        let driver = self.root.determinize_for_search(self.actor, seed)?;
        let context = self.context(&driver)?;
        let legal_actions = context
            .options()
            .iter()
            .map(|option| option.id())
            .collect::<Vec<_>>();
        let options = context
            .options()
            .iter()
            .map(|option| {
                (
                    option.id(),
                    CombatSearchOption {
                        actions: option.actions().to_vec(),
                        prior: driver.combat_action_prior(
                            option.descriptor(),
                            self.weights,
                            self.guardrail_profile,
                        ),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        Ok(CombatSearchState {
            driver,
            finished: false,
            terminal_prior: 0,
            legal_actions: Arc::new(legal_actions),
            options: Arc::new(options),
        })
    }

    fn legal_actions(&self, state: &Self::State) -> Result<Vec<CanonicalActionId>, String> {
        if state.finished {
            return Ok(Vec::new());
        }
        Ok(state.legal_actions.as_ref().clone())
    }

    fn apply_action(
        &self,
        state: &Self::State,
        action: CanonicalActionId,
    ) -> Result<Self::State, String> {
        let selected = state
            .options
            .get(&action)
            .ok_or_else(|| format!("combat search selected illegal action {action}"))?;
        let terminal_prior = selected.prior;
        let actions = selected.actions.clone();
        let mut next = state.clone();
        for action in actions {
            next.driver.dispatch(action)?;
        }
        next.finished = true;
        next.terminal_prior = terminal_prior;
        Ok(next)
    }

    fn terminal_value(&self, state: &Self::State) -> Option<i64> {
        state.finished.then(|| self.value(state))
    }

    fn evaluate(&self, state: &Self::State) -> i64 {
        self.value(state)
    }

    fn rollout_action(
        &self,
        _state: &Self::State,
        actions: &[CanonicalActionId],
        _seed: u64,
    ) -> Result<CanonicalActionId, String> {
        actions
            .first()
            .copied()
            .ok_or_else(|| "combat search rollout received no legal actions".to_owned())
    }

    fn action_prior(&self, state: &Self::State, action: CanonicalActionId) -> i64 {
        state.options.get(&action).map_or(0, |option| option.prior)
    }

    fn state_key(&self, state: &Self::State) -> Option<u64> {
        Some(
            state
                .driver
                .state
                .deterministic_hash()
                .get()
                .wrapping_add(state.terminal_prior as u64)
                .wrapping_add(if state.finished {
                    0xe703_7ed1_a0b4_28db
                } else {
                    0
                }),
        )
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

fn ordered_bottoms(cards: &[ObjectId], count: usize) -> Vec<Vec<ObjectId>> {
    fn visit(
        cards: &[ObjectId],
        count: usize,
        used: &mut [bool],
        current: &mut Vec<ObjectId>,
        results: &mut Vec<Vec<ObjectId>>,
    ) {
        if current.len() == count {
            results.push(current.clone());
            return;
        }
        for (index, card) in cards.iter().copied().enumerate() {
            if used[index] {
                continue;
            }
            used[index] = true;
            current.push(card);
            visit(cards, count, used, current, results);
            current.pop();
            used[index] = false;
        }
    }

    let mut results = Vec::new();
    let mut used = vec![false; cards.len()];
    visit(
        cards,
        count,
        &mut used,
        &mut Vec::with_capacity(count),
        &mut results,
    );
    results
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn canonical_legal_actions(context: &DecisionContext) -> Vec<AiLegalAction> {
    context
        .options()
        .iter()
        .map(|option| AiLegalAction {
            action_id: option.id().to_string(),
            descriptor_schema_version: 1,
            descriptor: decision_descriptor_value(option.descriptor()),
        })
        .collect()
}

fn decision_descriptor_value(descriptor: &DecisionDescriptor) -> Value {
    match descriptor {
        DecisionDescriptor::PassPriority => json!({"kind": "pass_priority"}),
        DecisionDescriptor::TakeMulligan => json!({"kind": "take_mulligan"}),
        DecisionDescriptor::KeepOpeningHand { bottom } => json!({
            "kind": "keep_opening_hand",
            "bottom_object_ids": bottom.iter().map(|object| object.index()).collect::<Vec<_>>()
        }),
        DecisionDescriptor::PlayLand { object } => {
            json!({"kind": "play_land", "object_id": object.index()})
        }
        DecisionDescriptor::ActivateAbility {
            source,
            ability,
            payment,
        } => json!({
            "kind": "activate_ability",
            "source_object_id": source.index(),
            "ability_id": ability.get(),
            "payment": payment_value(*payment)
        }),
        DecisionDescriptor::CastSpell {
            object,
            payment,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "cast_spell",
            "object_id": object.index(),
            "payment": payment_value(*payment),
            "targets": targets.iter().copied().map(target_value).collect::<Vec<_>>(),
            "modes": modes,
            "optional": optional
        }),
        DecisionDescriptor::DeclareAttackers { attacks } => json!({
            "kind": "declare_attackers",
            "attacks": attacks.iter().map(|attack| json!({
                "attacker_object_id": attack.attacker().index(),
                "defending_seat": attack.defending_player().index()
            })).collect::<Vec<_>>()
        }),
        DecisionDescriptor::DeclareBlockers { blocks } => json!({
            "kind": "declare_blockers",
            "blocks": blocks.iter().map(|block| json!({
                "blocker_object_id": block.blocker().index(),
                "attacker_object_id": block.attacker().index()
            })).collect::<Vec<_>>()
        }),
        DecisionDescriptor::MoveCommanderToCommand { object } => json!({
            "kind": "move_commander_to_command",
            "object_id": object.index()
        }),
        DecisionDescriptor::LeaveCommander { object, zone } => json!({
            "kind": "leave_commander",
            "object_id": object.index(),
            "zone": zone_value(*zone)
        }),
        DecisionDescriptor::ChooseTarget { target } => {
            json!({"kind": "choose_target", "target": target_value(*target)})
        }
        DecisionDescriptor::ChooseMode { mode } => {
            json!({"kind": "choose_mode", "mode": mode})
        }
        DecisionDescriptor::ChooseNumber { value } => {
            json!({"kind": "choose_number", "value": value})
        }
        DecisionDescriptor::ChoosePayment { payment } => {
            json!({"kind": "choose_payment", "payment": payment_value(*payment)})
        }
        DecisionDescriptor::ChooseOptional { prompt, accept } => json!({
            "kind": "choose_optional",
            "prompt": prompt,
            "accept": accept
        }),
        DecisionDescriptor::OrderTriggers { triggers } => json!({
            "kind": "order_triggers",
            "trigger_ids": triggers.iter().map(|trigger| trigger.get()).collect::<Vec<_>>()
        }),
        DecisionDescriptor::ChooseHiddenSlot { zone, slot } => json!({
            "kind": "choose_hidden_slot",
            "zone": zone_value(*zone),
            "slot": slot
        }),
        DecisionDescriptor::ChooseSearchObject { object } => json!({
            "kind": "choose_search_object",
            "object_id": object.index()
        }),
        DecisionDescriptor::AssignCombatDamage {
            source,
            target,
            amount,
        } => json!({
            "kind": "assign_combat_damage",
            "source_object_id": source.index(),
            "target": target_value(*target),
            "amount": amount
        }),
        DecisionDescriptor::Concede => json!({"kind": "concede"}),
    }
}

fn payment_value(payment: PaymentPlan) -> Value {
    json!({
        "paid": mana_pool_value(payment.paid()),
        "generic_paid": mana_pool_value(payment.generic_paid()),
        "generic_required": payment.generic_required(),
        "x_value": payment.x_value(),
        "waste_score": payment.waste_score()
    })
}

fn mana_pool_value(pool: forge_core::ManaPool) -> Value {
    json!({
        "white": pool.get(ManaKind::White),
        "blue": pool.get(ManaKind::Blue),
        "black": pool.get(ManaKind::Black),
        "red": pool.get(ManaKind::Red),
        "green": pool.get(ManaKind::Green),
        "colorless": pool.get(ManaKind::Colorless)
    })
}

fn target_value(target: TargetChoice) -> Value {
    match target {
        TargetChoice::Player(player) => json!({"kind": "player", "seat": player.index()}),
        TargetChoice::Object(object) => {
            json!({"kind": "object", "object_id": object.index()})
        }
        TargetChoice::StackEntry(entry) => {
            json!({"kind": "stack_entry", "stack_entry_id": entry.get()})
        }
    }
}

fn zone_value(zone: ZoneId) -> Value {
    json!({
        "owner_seat": zone.owner().map(PlayerId::index),
        "kind": match zone.kind() {
            ZoneKind::Library => "library",
            ZoneKind::Hand => "hand",
            ZoneKind::Battlefield => "battlefield",
            ZoneKind::Graveyard => "graveyard",
            ZoneKind::Exile => "exile",
            ZoneKind::Stack => "stack",
            ZoneKind::Command => "command",
            ZoneKind::Ceased => "ceased"
        }
    })
}

fn experimental_adaptive_stopping() -> AdaptiveStopping {
    AdaptiveStopping::experimental(
        vec![16, 32, 64, 128, 256, 512, 1_024, 2_048],
        3,
        700_000,
        8,
        500,
        600_000,
    )
}

const fn search_stop_reason(reason: SearchStopReason) -> &'static str {
    match reason {
        SearchStopReason::SingletonLegalAction => "singleton_legal_action",
        SearchStopReason::CertifiedWin => "certified_win",
        SearchStopReason::CertifiedRequiredDefense => "certified_required_defense",
        SearchStopReason::AdaptiveStableLeader => "adaptive_stable_leader_experimental",
        SearchStopReason::FixedIterations => "fixed_iterations",
        SearchStopReason::WallTimeBudget => "wall_time_budget",
        SearchStopReason::Mixed => "mixed",
    }
}

fn ai_decisions_match(left: &[AiDecisionRecord], right: &[AiDecisionRecord]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            let mut left = left.clone();
            let mut right = right.clone();
            left.wall_latency_us = 0;
            right.wall_latency_us = 0;
            left.think_ms = 0;
            right.think_ms = 0;
            left == right
        })
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
    let primary = GameDriver::setup(pod, seed, coverage_target.clone(), primary_mode, None)?
        .run(max_turns)?;
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

    let mut terminal = TerminalDecisionSource::new(input, output);
    let driver = GameDriver::setup_with_human_opening(
        &pod,
        seed,
        None,
        TraceMode::Record(Vec::new()),
        None,
        Some((human_seat, &mut terminal)),
    )?;
    let human = driver.players[human_seat];
    let primary = driver.run_human(max_turns, human, &mut terminal)?;
    let decisions = terminal.into_decisions();
    let actions = primary
        .trace
        .clone()
        .ok_or_else(|| "human game did not retain an action trace".to_owned())?;

    let mut replay_source = ReplayDecisionSource::new(decisions.clone());
    let verify_driver = GameDriver::setup_with_human_opening(
        &pod,
        seed,
        None,
        TraceMode::Verify {
            expected: actions.clone(),
            cursor: 0,
        },
        None,
        Some((human_seat, &mut replay_source)),
    )?;
    let verify_human = verify_driver.players[human_seat];
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

/// Policy selection for one deterministic AI game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiPolicyConfig {
    /// Uniform seeded random legal actions.
    RandomLegal {
        /// Policy RNG seed.
        seed: u64,
    },
    /// One-ply baseline policy.
    Heuristic {
        /// Policy RNG seed.
        seed: u64,
        /// Symmetric deterministic score-noise span.
        noise_span: i64,
    },
    /// Root-parallel determinized UCT under a replayable iteration budget.
    Search {
        /// Search seed.
        seed: u64,
        /// Simulations per determinization tree.
        iterations: u32,
        /// Independent hidden-information samples.
        determinizations: u32,
        /// Maximum local search workers.
        workers: u32,
    },
    /// Product-facing wall-time search used by local calibration campaigns.
    TimedSearch {
        /// Search seed.
        seed: u64,
        /// Wall-time budget for each independent determinization tree.
        think_ms: u32,
        /// Independent hidden-information samples.
        determinizations: u32,
        /// Maximum local search workers.
        workers: u32,
        /// Enables provisional adaptive stopping for paired ablation only.
        adaptive: bool,
        /// Data-only action-prior profile for this product tier.
        guardrail_profile: GuardrailProfile,
    },
}

impl AiPolicyConfig {
    /// Returns the stable policy family name used in arena evidence.
    #[must_use]
    pub const fn policy_name(self) -> &'static str {
        match self {
            Self::RandomLegal { .. } => "random-legal-v1",
            Self::Heuristic { .. } => "heuristic-v1",
            Self::Search { .. } => "determinized-uct-v1-fixed",
            Self::TimedSearch {
                adaptive: false, ..
            } => "determinized-uct-v1-timed-fixed",
            Self::TimedSearch { adaptive: true, .. } => {
                "determinized-uct-v1-timed-adaptive-experimental"
            }
        }
    }
}

/// One headless four-player arena result without replay duplication.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AiArenaSummary {
    /// Deterministic game seed.
    pub seed: u64,
    /// Zero-based winning seat.
    pub winner_seat: usize,
    /// Completed turn count.
    pub turns: u32,
    /// Final deterministic state hash.
    pub final_hash: u64,
    /// Final life totals in seat order.
    pub final_life: [i32; PLAYER_COUNT],
    /// Number of typed engine actions applied.
    pub typed_actions: usize,
    /// Number of canonical policy decisions.
    pub decisions: usize,
    /// Number of decisions entering the search adapter, including forced bypasses.
    pub searched_decisions: usize,
    /// Number of singleton decisions that bypassed full policy work.
    pub singleton_bypasses: usize,
    /// Completed MCTS simulations.
    pub simulations: u64,
    /// Allocated search nodes.
    pub nodes: u64,
    /// Search transposition-table hits.
    pub transposition_hits: u64,
    /// Deepest searched ply.
    pub maximum_depth: u32,
    /// Sum of measured search wall latency.
    pub search_wall_latency_us: u64,
    /// Per-decision search wall latencies for exact local percentile aggregation.
    pub search_wall_latencies_us: Vec<u64>,
    /// Per-decision search latencies grouped by configured wall budget.
    pub search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
    /// Adaptive-search latencies grouped separately for fixed-budget ablation.
    pub adaptive_search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
    /// Sum of measured process CPU time when every search supplied it.
    pub search_cpu_time_us: Option<u64>,
    /// Largest measured resident-memory delta when supplied.
    pub peak_memory_delta_bytes: Option<i64>,
    /// Counts of explicit policy stop reasons.
    pub stop_reasons: BTreeMap<String, u64>,
}

/// Cached compiled pod used by high-throughput local arena campaigns.
#[derive(Clone)]
pub struct AiArena {
    pod: Arc<PodTemplate>,
    weights: AiWeights,
}

impl AiArena {
    /// Loads and compiles one immutable four-deck manifest once.
    pub fn load(manifest: impl AsRef<Path>) -> Result<Self, String> {
        let pod = PodTemplate::load(manifest.as_ref())?;
        let weights =
            AiWeights::bundled().map_err(|error| format!("failed to load AI weights: {error}"))?;
        Ok(Self {
            pod: Arc::new(pod),
            weights,
        })
    }

    /// Runs one mixed-policy game using the cached compiled pod.
    pub fn run_game(
        &self,
        seed: u64,
        max_turns: u32,
        policy_configs: [AiPolicyConfig; AI_ARENA_SEATS],
    ) -> Result<AiArenaSummary, String> {
        run_ai_arena_with_pod(&self.pod, self.weights, seed, max_turns, policy_configs)
    }
}

/// Complete non-file options for one AI game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiGameOptions {
    seed: u64,
    max_turns: u32,
    policy: AiPolicyConfig,
}

impl AiGameOptions {
    /// Creates one AI game configuration.
    #[must_use]
    pub const fn new(seed: u64, max_turns: u32, policy: AiPolicyConfig) -> Self {
        Self {
            seed,
            max_turns,
            policy,
        }
    }
}

fn controller_from_config(
    config: AiPolicyConfig,
    weights: AiWeights,
) -> Result<AiController, String> {
    match config {
        AiPolicyConfig::RandomLegal { seed } => {
            Ok(AiController::Random(RandomLegalPolicy::new(seed)))
        }
        AiPolicyConfig::Heuristic { seed, noise_span } => {
            if noise_span < 0 {
                return Err("heuristic noise_span must be nonnegative".to_owned());
            }
            if noise_span == 0 {
                Ok(AiController::Heuristic(HeuristicPolicy::rollout(
                    weights, seed,
                )))
            } else {
                Ok(AiController::Heuristic(HeuristicPolicy::novice(
                    weights, seed, noise_span,
                )))
            }
        }
        AiPolicyConfig::Search {
            seed,
            iterations,
            determinizations,
            workers,
        } => {
            validate_search_dimensions(iterations, determinizations, workers, "iterations")?;
            Ok(AiController::Search(SearchController {
                weights,
                seed,
                determinizations,
                limit: SearchControllerLimit::Iterations(iterations),
                workers,
                adaptive: false,
                guardrail_profile: GuardrailProfile::Expert,
            }))
        }
        AiPolicyConfig::TimedSearch {
            seed,
            think_ms,
            determinizations,
            workers,
            adaptive,
            guardrail_profile,
        } => {
            validate_search_dimensions(think_ms, determinizations, workers, "think_ms")?;
            Ok(AiController::Search(SearchController {
                weights,
                seed,
                determinizations,
                limit: SearchControllerLimit::WallTimeMs(think_ms),
                workers,
                adaptive,
                guardrail_profile,
            }))
        }
    }
}

fn validate_search_dimensions(
    budget: u32,
    determinizations: u32,
    workers: u32,
    budget_name: &str,
) -> Result<(), String> {
    if budget == 0 {
        return Err(format!("search {budget_name} must be positive"));
    }
    if determinizations == 0 {
        return Err("search determinizations must be positive".to_owned());
    }
    if workers == 0 || workers > MAX_WORKERS as u32 {
        return Err(format!("search workers must be in 1..={MAX_WORKERS}"));
    }
    Ok(())
}

/// Runs one mixed-policy four-player arena game without replay duplication.
///
/// This is the high-throughput primitive for paired seat-rotation campaigns.
/// The game still executes invariant and hidden-information checks internally.
pub fn run_ai_arena_game(
    manifest: impl AsRef<Path>,
    seed: u64,
    max_turns: u32,
    policy_configs: [AiPolicyConfig; AI_ARENA_SEATS],
) -> Result<AiArenaSummary, String> {
    AiArena::load(manifest)?.run_game(seed, max_turns, policy_configs)
}

fn run_ai_arena_with_pod(
    pod: &PodTemplate,
    weights: AiWeights,
    seed: u64,
    max_turns: u32,
    policy_configs: [AiPolicyConfig; AI_ARENA_SEATS],
) -> Result<AiArenaSummary, String> {
    if max_turns < 20 {
        return Err("arena max_turns must be at least 20".to_owned());
    }
    let policies = [
        controller_from_config(policy_configs[0], weights)?,
        controller_from_config(policy_configs[1], weights)?,
        controller_from_config(policy_configs[2], weights)?,
        controller_from_config(policy_configs[3], weights)?,
    ];
    let run = GameDriver::setup(pod, seed, None, TraceMode::Off, Some(policies))?
        .run_ai(max_turns, policies)?;

    let searched = run
        .ai_decisions
        .iter()
        .filter(|decision| decision.policy == "determinized-uct-v1")
        .collect::<Vec<_>>();
    let search_cpu_time_us = (!searched.is_empty()
        && searched
            .iter()
            .all(|decision| decision.actual_cpu_time_us.is_some()))
    .then(|| {
        searched
            .iter()
            .filter_map(|decision| decision.actual_cpu_time_us)
            .sum()
    });
    let mut stop_reasons = BTreeMap::new();
    let mut search_wall_latencies_by_budget_ms = BTreeMap::<u32, Vec<u64>>::new();
    let mut adaptive_search_wall_latencies_by_budget_ms = BTreeMap::<u32, Vec<u64>>::new();
    for decision in &run.ai_decisions {
        *stop_reasons
            .entry(decision.stop_reason.clone())
            .or_insert(0) += 1;
        if decision.policy == "determinized-uct-v1" && decision.configured_wall_ms > 0 {
            let target = if decision.adaptive_search {
                &mut adaptive_search_wall_latencies_by_budget_ms
            } else {
                &mut search_wall_latencies_by_budget_ms
            };
            target
                .entry(decision.configured_wall_ms)
                .or_default()
                .push(decision.wall_latency_us);
        }
    }

    Ok(AiArenaSummary {
        seed,
        winner_seat: run.summary.winner,
        turns: run.summary.turns,
        final_hash: run.summary.final_hash,
        final_life: run.summary.final_life,
        typed_actions: run.actions.len(),
        decisions: run.ai_decisions.len(),
        searched_decisions: searched.len(),
        singleton_bypasses: searched
            .iter()
            .filter(|decision| decision.stop_reason == "singleton_legal_action")
            .count(),
        simulations: searched.iter().map(|decision| decision.simulations).sum(),
        nodes: searched.iter().map(|decision| decision.nodes).sum(),
        transposition_hits: searched
            .iter()
            .map(|decision| decision.transposition_hits)
            .sum(),
        maximum_depth: searched
            .iter()
            .map(|decision| decision.maximum_depth)
            .max()
            .unwrap_or(0),
        search_wall_latency_us: searched
            .iter()
            .map(|decision| decision.wall_latency_us)
            .sum(),
        search_wall_latencies_us: searched
            .iter()
            .map(|decision| decision.wall_latency_us)
            .collect(),
        search_wall_latencies_by_budget_ms,
        adaptive_search_wall_latencies_by_budget_ms,
        search_cpu_time_us,
        peak_memory_delta_bytes: searched
            .iter()
            .filter_map(|decision| decision.memory_delta_bytes)
            .max(),
        stop_reasons,
    })
}

/// Runs one four-player game under a T4 policy and writes an exact replay.
pub fn run_ai_game(
    manifest: impl AsRef<Path>,
    replay_out: impl AsRef<Path>,
    options: AiGameOptions,
) -> Result<String, String> {
    let seed = options.seed;
    let max_turns = options.max_turns;
    let (
        policy_seed,
        noise_span,
        random_legal,
        search_iterations,
        search_determinizations,
        search_workers,
    ) = match options.policy {
        AiPolicyConfig::RandomLegal { seed } => (seed, 0, true, None, 0, 0),
        AiPolicyConfig::Heuristic { seed, noise_span } => (seed, noise_span, false, None, 0, 0),
        AiPolicyConfig::Search {
            seed,
            iterations,
            determinizations,
            workers,
        } => (seed, 0, false, Some(iterations), determinizations, workers),
        AiPolicyConfig::TimedSearch { .. } => {
            return Err(
                "wall-time AI games are arena-only because exact replay requires a deterministic iteration budget"
                    .to_owned(),
            );
        }
    };
    if max_turns < 20 {
        return Err("--max-turns must be at least 20".to_owned());
    }
    if noise_span < 0 {
        return Err("--noise-span must be nonnegative".to_owned());
    }
    if random_legal && search_iterations.is_some() {
        return Err("--random-legal and --search cannot be combined".to_owned());
    }
    if search_iterations == Some(0) {
        return Err("--search-iterations must be positive".to_owned());
    }
    if search_iterations.is_some()
        && (search_determinizations == 0 || search_workers == 0 || search_workers > 24)
    {
        return Err(
            "search determinizations must be positive and workers must be in 1..=24".to_owned(),
        );
    }
    let manifest = manifest.as_ref();
    let replay_out = replay_out.as_ref();
    let pod = PodTemplate::load(manifest)?;
    let pilot_intents_path = Path::new(PILOT_INTENTS_PATH);
    let pilot_intents = PilotIntentRegistry::load(pilot_intents_path, &pod)?;
    let weights =
        AiWeights::bundled().map_err(|error| format!("failed to load AI weights: {error}"))?;
    let (policy, policy_kind) = if let Some(iterations) = search_iterations {
        (
            AiController::Search(SearchController {
                weights,
                seed: policy_seed,
                determinizations: search_determinizations,
                limit: SearchControllerLimit::Iterations(iterations),
                workers: search_workers,
                adaptive: false,
                guardrail_profile: GuardrailProfile::Expert,
            }),
            "determinized-uct-v1",
        )
    } else if random_legal {
        (
            AiController::Random(RandomLegalPolicy::new(policy_seed)),
            "random-legal-v1",
        )
    } else if noise_span == 0 {
        (
            AiController::Heuristic(HeuristicPolicy::rollout(weights, policy_seed)),
            "heuristic-v1",
        )
    } else {
        (
            AiController::Heuristic(HeuristicPolicy::novice(weights, policy_seed, noise_span)),
            "heuristic-v1",
        )
    };
    let policies = [policy; PLAYER_COUNT];
    let primary = GameDriver::setup(&pod, seed, None, TraceMode::Off, Some(policies))?
        .run_ai(max_turns, policies)?;
    let direct_state = replay_captured_actions(&primary.actions, None)?;
    if direct_state.deterministic_hash().get() != primary.summary.final_hash {
        return Err("direct AI typed-action playback produced a different final hash".to_owned());
    }

    let verified = GameDriver::setup(&pod, seed, None, TraceMode::Off, Some(policies))?
        .run_ai(max_turns, policies)?;
    if verified.summary != primary.summary
        || !ai_decisions_match(&verified.ai_decisions, &primary.ai_decisions)
    {
        return Err("AI replay diverged from canonical decisions or final summary".to_owned());
    }
    let replay = AiPlayReplay {
        format: AI_REPLAY_MAGIC.to_owned(),
        manifest: manifest.to_path_buf(),
        seed,
        max_turns,
        policy_seed,
        policy_kind: policy_kind.to_owned(),
        noise_span,
        search_iterations: search_iterations.unwrap_or(0),
        search_determinizations: search_iterations.map_or(0, |_| search_determinizations),
        search_workers: search_iterations.map_or(0, |_| search_workers),
        pilot_intents: pilot_intents_path.to_path_buf(),
        decisions: primary.ai_decisions,
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
        .map_err(|error| format!("failed to serialize AI replay: {error}"))?;
    fs::write(replay_out, payload)
        .map_err(|error| format!("failed to write {}: {error}", replay_out.display()))?;
    Ok(format!(
        "AI baseline game complete\nseed: {}\nturns: {}\nwinner_seat: {}\nactions: {}\ndecisions: {}\npilot_intents_limited: {}\nfinal_hash: {}\nreplay: {}\n",
        seed,
        primary.summary.turns,
        primary.summary.winner + 1,
        primary.actions.len(),
        replay.decisions.len(),
        pilot_intents.limited_count(),
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
        Some(AI_REPLAY_MAGIC) => replay_ai_file(path),
        Some(format) => Err(format!("unsupported JSON replay format `{format}`")),
        None => Err(format!("{} has no replay format", path.display())),
    }
}

/// Replays a T4.3 AI game from canonical decision IDs and seeded policy state.
pub fn replay_ai_file(path: impl AsRef<Path>) -> Result<String, String> {
    let path = path.as_ref();
    let payload =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let replay: AiPlayReplay = serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if replay.format != AI_REPLAY_MAGIC {
        return Err(format!(
            "{} is not a {AI_REPLAY_MAGIC} artifact",
            path.display()
        ));
    }
    let pod = PodTemplate::load(&replay.manifest)?;
    let _pilot_intents = PilotIntentRegistry::load(&replay.pilot_intents, &pod)?;
    let weights =
        AiWeights::bundled().map_err(|error| format!("failed to load AI weights: {error}"))?;
    let policy = match replay.policy_kind.as_str() {
        "random-legal-v1" => AiController::Random(RandomLegalPolicy::new(replay.policy_seed)),
        "heuristic-v1" if replay.noise_span == 0 => {
            AiController::Heuristic(HeuristicPolicy::rollout(weights, replay.policy_seed))
        }
        "heuristic-v1" => AiController::Heuristic(HeuristicPolicy::novice(
            weights,
            replay.policy_seed,
            replay.noise_span,
        )),
        "determinized-uct-v1"
            if replay.search_iterations > 0
                && replay.search_determinizations > 0
                && replay.search_workers > 0 =>
        {
            AiController::Search(SearchController {
                weights,
                seed: replay.policy_seed,
                determinizations: replay.search_determinizations,
                limit: SearchControllerLimit::Iterations(replay.search_iterations),
                workers: replay.search_workers,
                adaptive: false,
                guardrail_profile: GuardrailProfile::Expert,
            })
        }
        other => return Err(format!("unsupported AI replay policy `{other}`")),
    };
    let policies = [policy; PLAYER_COUNT];
    let run = GameDriver::setup(&pod, replay.seed, None, TraceMode::Off, Some(policies))?
        .run_ai(replay.max_turns, policies)?;
    if run.summary != replay.expected || !ai_decisions_match(&run.ai_decisions, &replay.decisions) {
        return Err("AI replay diverged from canonical decisions or final summary".to_owned());
    }
    let direct_state = replay_captured_actions(&run.actions, None)?;
    if direct_state.deterministic_hash().get() != replay.expected.final_hash {
        return Err("direct AI typed-action playback produced a different final hash".to_owned());
    }
    Ok(format!(
        "AI replay complete (canonical decisions and typed actions verified)\nseed: {}\ndecisions: {}\nactions: {}\nfinal_hash: {}\nwinner_seat: {}\n",
        replay.seed,
        replay.decisions.len(),
        run.actions.len(),
        run.summary.final_hash,
        run.summary.winner + 1
    ))
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
    let mut decisions = ReplayDecisionSource::new(replay.decisions.clone());
    let legacy = decisions.is_legacy_replay();
    let trace = TraceMode::Verify {
        expected: replay.actions.clone(),
        cursor: 0,
    };
    let driver = if replay
        .decisions
        .first()
        .is_some_and(|record| record.prompt == "Choose opening hand")
    {
        GameDriver::setup_with_human_opening(
            &pod,
            replay.seed,
            None,
            trace,
            None,
            Some((replay.human_seat, &mut decisions)),
        )?
    } else {
        GameDriver::setup(&pod, replay.seed, None, trace, None)?
    };
    let human = driver.players[replay.human_seat];
    let run = driver.run_human(replay.max_turns, human, &mut decisions)?;
    decisions.finish()?;
    let summary_matches = if legacy {
        legacy_human_summary_matches(&replay.expected, &run.summary)
    } else {
        run.summary == replay.expected
    };
    if !summary_matches {
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

fn legacy_human_summary_matches(expected: &GameSummary, actual: &GameSummary) -> bool {
    if actual.metrics.hidden_information_checks < expected.metrics.hidden_information_checks {
        return false;
    }
    let mut normalized = actual.clone();
    normalized.metrics.hidden_information_checks = expected.metrics.hidden_information_checks;
    &normalized == expected
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
        None,
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
        campaign_seed, legacy_human_summary_matches, player_view_fingerprint,
        replay_captured_actions, replay_human_file, run_prompted_game, snapshot_prompt,
        AiController, DecisionPrompt, DecisionSource, GameDriver, GameMetrics, GameSummary,
        HeuristicPolicy, IdentityExercise, ReplayDecisionSource, TraceMode, TraceRecord,
        PLAYER_COUNT,
    };
    use forge_ai::{AiWeights, GuardrailTable};
    use forge_core::{
        apply, Action, AttackDeclaration, BaseCreatureCharacteristics, CardId, DecisionContext,
        DecisionDescriptor, DecisionKind, DecisionOption, GameState, ObjectColors, ObjectId,
        Outcome, PlayerId, PlayerView, Step, ZoneId, ZoneKind,
    };
    use std::{
        collections::{BTreeSet, HashMap, HashSet},
        env,
        io::Cursor,
        path::Path,
        sync::Arc,
    };

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
        let context = DecisionContext::new(
            DecisionKind::Optional,
            view.observer(),
            &view,
            vec![
                DecisionOption::new(
                    DecisionDescriptor::ChooseOptional {
                        prompt: 0,
                        accept: false,
                    },
                    Vec::new(),
                ),
                DecisionOption::new(
                    DecisionDescriptor::ChooseOptional {
                        prompt: 0,
                        accept: true,
                    },
                    Vec::new(),
                ),
            ],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("decision context should exist: {error}"));
        let options = vec!["First legal action".to_owned(), "Pass".to_owned()];
        let prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            context: &context,
            options: &options,
        };
        let record = snapshot_prompt(0, &prompt, 1);
        assert_eq!(record.context_id, context.id().to_string());
        assert_eq!(record.decision_state_key, context.state_key().to_string());
        assert_eq!(record.canonical_legal_actions.len(), 2);
        assert_eq!(
            record.selected_action_id,
            context.options()[1].id().to_string()
        );
        let mut replay = ReplayDecisionSource::new(vec![record]);
        assert_eq!(replay.choose(&prompt), Ok(1));
        assert!(replay.finish().is_ok());

        let changed_options = vec!["Different action".to_owned(), "Pass".to_owned()];
        let changed_prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            context: &context,
            options: &changed_options,
        };
        let record = snapshot_prompt(0, &prompt, 1);
        let mut replay = ReplayDecisionSource::new(vec![record]);
        assert!(replay.choose(&changed_prompt).is_err());
    }

    #[test]
    fn legacy_human_decision_record_remains_replayable() {
        let view = hidden_card_view(7, false);
        let context = DecisionContext::new(
            DecisionKind::Concession,
            view.observer(),
            &view,
            vec![DecisionOption::new(
                DecisionDescriptor::Concede,
                vec![Action::Concede {
                    player: view.observer(),
                }],
            )],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("decision context should exist: {error}"));
        let labels = vec!["Concede".to_owned()];
        let prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            context: &context,
            options: &labels,
        };
        let mut legacy = snapshot_prompt(0, &prompt, 0);
        legacy.decision_context_schema = 0;
        legacy.context_id.clear();
        legacy.decision_state_key.clear();
        legacy.player_view_hash.clear();
        legacy.canonical_legal_actions.clear();
        legacy.selected_action_id.clear();

        let mut replay = ReplayDecisionSource::new(vec![legacy]);
        assert_eq!(replay.choose(&prompt), Ok(0));
        assert!(replay.finish().is_ok());
    }

    #[test]
    fn legacy_summary_allows_only_monotonic_instrumentation_growth() {
        let expected = GameSummary {
            seed: 17,
            winner: 2,
            turns: 40,
            final_hash: 99,
            final_life: [0, 0, 12, 0],
            metrics: GameMetrics {
                hidden_information_checks: 100,
                ..GameMetrics::default()
            },
        };
        let mut instrumented = expected.clone();
        instrumented.metrics.hidden_information_checks = 104;
        assert!(legacy_human_summary_matches(&expected, &instrumented));

        instrumented.final_hash = 100;
        assert!(!legacy_human_summary_matches(&expected, &instrumented));
        instrumented.final_hash = expected.final_hash;
        instrumented.metrics.hidden_information_checks = 99;
        assert!(!legacy_human_summary_matches(&expected, &instrumented));
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
    fn attack_context_exposes_split_defender_assignments() {
        let (driver, active, defenders, pieces) = combat_decision_driver();
        let candidates = driver
            .diagnostic_attack_candidates(active)
            .unwrap_or_else(|error| panic!("split attack candidates should exist: {error}"));

        assert_eq!(candidates.len(), 16);
        assert!(candidates.iter().any(|attacks| {
            attacks
                == &vec![
                    AttackDeclaration::new(pieces[0], defenders[0]),
                    AttackDeclaration::new(pieces[1], defenders[1]),
                ]
        }));
        assert!(candidates.iter().all(|attacks| {
            attacks
                .iter()
                .map(|attack| attack.attacker())
                .collect::<BTreeSet<_>>()
                .len()
                == attacks.len()
        }));
    }

    #[test]
    fn driver_preserves_blocks_from_every_attacked_defender() {
        let (mut driver, active, defenders, pieces) = combat_decision_driver();
        let attacks = vec![
            AttackDeclaration::new(pieces[0], defenders[0]),
            AttackDeclaration::new(pieces[1], defenders[1]),
        ];
        driver
            .dispatch(Action::DeclareAttackers {
                player: active,
                attacks: attacks.clone(),
            })
            .unwrap_or_else(|error| panic!("split attacks should apply: {error}"));
        driver.current_attacks = attacks;
        assert!(matches!(
            driver.dispatch(Action::AdvanceStep),
            Ok(Outcome::StepAdvanced(Step::DeclareBlockers))
        ));

        let declaration_order = driver.current_defending_players(active);
        assert_eq!(declaration_order, defenders[..2]);
        for defender in declaration_order {
            driver
                .declare_blocks(defender)
                .unwrap_or_else(|error| panic!("defender blocks should apply: {error}"));
        }

        let combat = driver.state.combat_state();
        assert_eq!(combat.blockers_declared_by(), &defenders[..2]);
        assert_eq!(combat.blockers().len(), 2);
        assert!(combat
            .blockers()
            .iter()
            .any(|block| block.object() == pieces[2] && block.attacker() == pieces[0]));
        assert!(combat
            .blockers()
            .iter()
            .any(|block| block.object() == pieces[3] && block.attacker() == pieces[1]));
        assert!(matches!(
            driver.dispatch(Action::AdvanceStep),
            Ok(Outcome::StepAdvanced(Step::CombatDamage))
        ));
    }

    #[test]
    fn commander_zone_adapter_exposes_both_legal_choices_and_ai_records_membership() {
        let (mut driver, owner, commander, graveyard) = commander_decision_driver();
        let context = driver
            .commander_zone_context(owner, commander, graveyard)
            .unwrap_or_else(|error| panic!("commander context should exist: {error}"));
        assert_eq!(context.kind(), DecisionKind::CommanderZone);
        assert_eq!(context.options().len(), 2);
        assert!(matches!(
            context.options()[0].descriptor(),
            DecisionDescriptor::MoveCommanderToCommand { object } if *object == commander
        ));
        assert!(matches!(
            context.options()[1].descriptor(),
            DecisionDescriptor::LeaveCommander { object, zone }
                if *object == commander && *zone == graveyard
        ));
        for option in context.options() {
            assert_eq!(
                context
                    .select(option.id())
                    .map(|selected| selected.descriptor().clone()),
                Ok(option.descriptor().clone())
            );
        }

        let policy = AiController::Heuristic(HeuristicPolicy::rollout(
            AiWeights::bundled().unwrap_or_else(|error| panic!("AI weights should load: {error}")),
            17,
        ));
        driver
            .choose_dead_commanders_with_ai(&[policy; PLAYER_COUNT])
            .unwrap_or_else(|error| panic!("AI commander choice should succeed: {error}"));
        assert_eq!(
            driver.state.object_zone(commander),
            Some(ZoneId::new(None, ZoneKind::Command))
        );
        assert_eq!(driver.metrics.commander_zone_returns, 1);
        let record = driver
            .ai_decisions
            .last()
            .unwrap_or_else(|| panic!("AI commander choice should emit telemetry"));
        assert_eq!(record.kind, "commander_zone");
        assert_eq!(record.context_id, context.id().to_string());
        assert_eq!(record.action_id, context.options()[0].id().to_string());
        assert_eq!(record.legal_actions, 2);
        assert_eq!(record.evaluated_candidates, 2);
        assert_eq!(record.canonical_legal_actions.len(), 2);
        assert_eq!(
            record.canonical_legal_actions[0].descriptor["kind"],
            "move_commander_to_command"
        );
        assert_eq!(
            record.canonical_legal_actions[1].descriptor["kind"],
            "leave_commander"
        );
    }

    #[test]
    fn human_commander_zone_choice_can_legally_leave_the_card() {
        let (mut driver, owner, commander, graveyard) = commander_decision_driver();
        let mut source = PickSecondChoice;
        driver
            .choose_dead_commanders_with_human(owner, &mut source)
            .unwrap_or_else(|error| panic!("human commander choice should succeed: {error}"));
        assert_eq!(driver.state.object_zone(commander), Some(graveyard));
        assert_eq!(driver.metrics.commander_zone_returns, 0);
        assert!(driver.actions.iter().all(|action| !matches!(
            action,
            Action::ChooseCommanderZone { object, .. } if *object == commander
        )));
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
        let mut input = Cursor::new(format!("2\n{}", "1\n".repeat(20_000)));
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

    struct PickSecondChoice;

    impl DecisionSource for PickSecondChoice {
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String> {
            (prompt.options.len() == 2)
                .then_some(1)
                .ok_or_else(|| "expected exactly two commander-zone options".to_owned())
        }
    }

    fn commander_decision_driver() -> (GameDriver, PlayerId, ObjectId, ZoneId) {
        let mut state = GameState::new();
        let owner = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player setup outcome: {other:?}"),
        };
        let graveyard = ZoneId::new(Some(owner), ZoneKind::Graveyard);
        let commander = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(77),
                owner,
                controller: owner,
                zone: graveyard,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected commander setup outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                &mut state,
                Action::DesignateCommander {
                    object: commander,
                    color_identity: ObjectColors::none(),
                },
            ),
            Outcome::Applied
        );
        let driver = GameDriver {
            state,
            players: vec![owner],
            programs: Arc::new(HashMap::new()),
            deck_models: Arc::new(Vec::new()),
            card_definitions: Arc::new(HashMap::new()),
            guardrails: Arc::new(
                GuardrailTable::bundled()
                    .unwrap_or_else(|error| panic!("guardrails should load: {error}")),
            ),
            commanders: vec![commander],
            trigger_programs: HashMap::new(),
            mana_abilities: Vec::new(),
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: 17,
        };
        (driver, owner, commander, graveyard)
    }

    fn combat_decision_driver() -> (GameDriver, PlayerId, [PlayerId; 3], [ObjectId; 4]) {
        let mut state = GameState::new();
        let players = (0..PLAYER_COUNT)
            .map(|_| match apply(&mut state, Action::AddPlayer) {
                Outcome::PlayerAdded(player) => player,
                other => panic!("unexpected player setup outcome: {other:?}"),
            })
            .collect::<Vec<_>>();
        let active = players[0];
        let defenders = [players[1], players[2], players[3]];
        let first_attacker = create_test_creature(&mut state, active, 800, 2, 2);
        let second_attacker = create_test_creature(&mut state, active, 801, 2, 2);
        let first_blocker = create_test_creature(&mut state, defenders[0], 802, 3, 3);
        let second_blocker = create_test_creature(&mut state, defenders[1], 803, 3, 3);
        assert!(matches!(
            apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(899),
                    owner: active,
                    controller: active,
                    zone: ZoneId::new(Some(active), ZoneKind::Library),
                }
            ),
            Outcome::ObjectCreated(_)
        ));
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: active
                }
            ),
            Outcome::Applied
        );
        while state.current_step() != Some(Step::DeclareAttackers) {
            assert!(matches!(
                apply(&mut state, Action::AdvanceStep),
                Outcome::StepAdvanced(_)
            ));
        }
        let driver = GameDriver {
            state,
            players,
            programs: Arc::new(HashMap::new()),
            deck_models: Arc::new(Vec::new()),
            card_definitions: Arc::new(HashMap::new()),
            guardrails: Arc::new(
                GuardrailTable::bundled()
                    .unwrap_or_else(|error| panic!("guardrails should load: {error}")),
            ),
            commanders: vec![first_attacker, second_attacker],
            trigger_programs: HashMap::new(),
            mana_abilities: Vec::new(),
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: 17,
        };
        (
            driver,
            active,
            defenders,
            [
                first_attacker,
                second_attacker,
                first_blocker,
                second_blocker,
            ],
        )
    }

    fn create_test_creature(
        state: &mut GameState,
        controller: PlayerId,
        card: u32,
        power: i32,
        toughness: i32,
    ) -> ObjectId {
        let object = match apply(
            state,
            Action::CreateObject {
                card: CardId::new(card),
                owner: controller,
                controller,
                zone: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected creature setup outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                state,
                Action::SetBaseCreatureCharacteristics {
                    object,
                    base: BaseCreatureCharacteristics::new(power, toughness),
                }
            ),
            Outcome::Applied
        );
        object
    }
}
