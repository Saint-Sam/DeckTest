#![forbid(unsafe_code)]

//! Production four-player game runner, human controller, AI adapter, and replay verifier.
//!
//! The controller is deliberately generic: deck contents come from a
//! deterministic manifest, card behavior comes from `forge-cards::runtime`,
//! and every mutation crosses `forge_core::apply`.

use forge_ai::{
    ActionRisk, ActionRisks, AdaptiveStopping, AiWeights, DeckModel, Determinizer,
    GuardrailProfile, GuardrailTable, HeuristicPolicy, LastDecisionReport, MulliganPolicy,
    PolicyCandidate, PolicyDecision, PolicyMode, RandomLegalPolicy, ResourceSnapshot, SearchConfig,
    SearchDomain, SearchEngine, SearchLimit, SearchReport, SearchStateKey, SearchStopReason,
};
use forge_cards::runtime::{
    bind_activated_effect_actions, bind_program_actions, bind_triggered_ability_actions,
    compile_card_program, object_satisfies_choice_requirement, CardProgram, ExecutionBindings,
    ObjectChoiceRequirement, PlayerBinding, ProgramKind, TriggeredAbilityProgram,
};
use forge_core::{
    apply, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect, ActivatedAbilityId,
    ActivationCost, AttackDeclaration, BlockDeclaration, CanonicalActionId, CardId,
    CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest, CombatDamageStepKind,
    CombatDamageTarget, DecisionContext, DecisionDescriptor, DecisionKind, DecisionOption,
    GameOutcome, GameState, HiddenCardDefinition, HiddenSlotDefinition, ManaKind, ObjectColors,
    ObjectId, ObjectView, Outcome, PaymentPlan, PlayerId, PlayerView, PriorityOutcome,
    ResolutionOutcome, SpellTiming, StackDecisionBindings, StackEntryId, StackObjectKind,
    StateError, Step, TargetChoice, TargetKind, TargetRequirement, TriggerId, ZoneId, ZoneKind,
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
const MAX_CANONICAL_SPELL_OPTIONS: usize = 65_536;
const MAX_DIRECT_NUMERIC_VALUES: u32 = 64;
const MAX_DIRECT_COMBAT_DAMAGE_AMOUNTS: u32 = 64;
const CONCESSION_PROMPT: &str = "Concede the game";

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
    path_discriminator: Option<u64>,
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
    path_discriminator: Option<u64>,
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
    allow_concession: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecisionSelection {
    Option(usize),
    RequestConcession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalPromptSelection {
    Option(CanonicalActionId),
    RequestConcession,
}

struct LegacyDecisionPrompt<'a> {
    kind: &'static str,
    view: &'a PlayerView,
    options: &'a [String],
}

trait DecisionSource {
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String>;

    fn choose_concession(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String> {
        match self.choose(prompt)? {
            DecisionSelection::Option(selected) => Ok(selected),
            DecisionSelection::RequestConcession => {
                Err("concession prompt recursively requested concession".to_owned())
            }
        }
    }

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
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
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
            let input = line.trim();
            if input.eq_ignore_ascii_case("q") {
                return Err("human game aborted by owner".to_owned());
            }
            if prompt.allow_concession
                && (input.eq_ignore_ascii_case("c") || input.eq_ignore_ascii_case("concede"))
            {
                return Ok(DecisionSelection::RequestConcession);
            }
            let Ok(choice) = input.parse::<usize>() else {
                let help = if prompt.allow_concession {
                    "Enter an option number, c to concede, or q to stop."
                } else {
                    "Enter an option number, or q to stop."
                };
                writeln!(self.output, "{help}")
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
        Ok(DecisionSelection::Option(selected))
    }

    fn choose_concession(&mut self, prompt: &DecisionPrompt<'_>) -> Result<usize, String> {
        validate_decision_prompt(prompt)?;
        if prompt.context.kind() != DecisionKind::Concession
            || prompt.options.len() != 1
            || !matches!(
                prompt.context.options()[0].descriptor(),
                DecisionDescriptor::Concede
            )
        {
            return Err("canonical concession prompt is malformed".to_owned());
        }
        self.decisions
            .push(snapshot_prompt(self.decisions.len() as u64, prompt, 0));
        writeln!(
            self.output,
            "Seat {} conceded.",
            prompt.view.observer().index() + 1
        )
        .map_err(|error| format!("failed to write concession result: {error}"))?;
        Ok(0)
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
    fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
        validate_decision_prompt(prompt)?;
        let expected = self
            .decisions
            .get(self.cursor)
            .ok_or_else(|| format!("unexpected replay prompt `{}`", prompt.kind))?;
        if !self.legacy
            && prompt.allow_concession
            && expected.prompt == CONCESSION_PROMPT
            && prompt.kind != CONCESSION_PROMPT
        {
            return Ok(DecisionSelection::RequestConcession);
        }
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
        Ok(DecisionSelection::Option(expected.selected))
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
        path_discriminator: prompt.context.path_discriminator(),
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

/// Builds the explicit singleton concession context used by human, AI, and
/// benchmark consumers. It is deliberately absent from ordinary policy action
/// sets so random or search controllers cannot concede accidentally.
pub fn concession_decision_context(
    state: &GameState,
    player: PlayerId,
) -> Result<DecisionContext, String> {
    let view = state
        .player_view(player)
        .map_err(|error| format!("concession player view failed: {error:?}"))?;
    concession_decision_context_from_view(&view, player)
}

fn concession_decision_context_from_view(
    view: &PlayerView,
    player: PlayerId,
) -> Result<DecisionContext, String> {
    DecisionContext::new(
        DecisionKind::Concession,
        player,
        view,
        vec![DecisionOption::new(
            DecisionDescriptor::Concede,
            vec![Action::Concede { player }],
        )],
        Vec::new(),
    )
    .map_err(|error| format!("concession decision context failed: {error}"))
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

#[derive(Clone)]
struct ActivatedRuntime {
    program: Arc<CardProgram>,
    ability_index: usize,
    source: ObjectId,
}

#[derive(Clone)]
struct RegisteredAbility {
    source: ObjectId,
    controller: PlayerId,
    id: ActivatedAbilityId,
    runtime: Option<ActivatedRuntime>,
}

#[derive(Clone)]
struct PendingActivatedResolution {
    controller: PlayerId,
    runtime: ActivatedRuntime,
    targets: Vec<TargetChoice>,
    decisions: StackDecisionBindings,
}

#[derive(Clone)]
struct PendingTriggeredResolution {
    controller: PlayerId,
    runtime: TriggerRuntime,
}

#[derive(Clone)]
enum MainChoice {
    PlayLand(ObjectId),
    ActivateAll,
    Activate {
        source: ObjectId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
    },
    ActivateProgram {
        source: ObjectId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
        targets: Vec<TargetChoice>,
        optional: Vec<bool>,
    },
    BeginCast {
        object: ObjectId,
        targets: Vec<TargetChoice>,
        mode: Option<u32>,
        optional: Vec<bool>,
    },
    NarrowCastX {
        object: ObjectId,
        targets: Vec<TargetChoice>,
        mode: Option<u32>,
        optional: Vec<bool>,
        minimum: u32,
        maximum: u32,
    },
    ChooseCastX {
        object: ObjectId,
        targets: Vec<TargetChoice>,
        mode: Option<u32>,
        optional: Vec<bool>,
        x_value: u32,
    },
    Cast {
        object: ObjectId,
        payment: PaymentPlan,
        targets: Vec<TargetChoice>,
        mode: Option<u32>,
        optional: Vec<bool>,
    },
    Finish,
}

type MainDecisionAdapter = (DecisionContext, Vec<(CanonicalActionId, MainChoice)>);

#[derive(Clone)]
struct SpellChoiceBinding {
    targets: Vec<TargetChoice>,
    mode: Option<u32>,
    optional: Vec<bool>,
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
    activated_abilities: Vec<RegisteredAbility>,
    pending_activated_resolution: Option<PendingActivatedResolution>,
    pending_triggered_resolution: Option<PendingTriggeredResolution>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum CombatSearchProgress {
    Attackers {
        active: PlayerId,
        objects: Arc<Vec<ObjectId>>,
        cursor: usize,
        declarations: Vec<AttackDeclaration>,
    },
    Blockers {
        defending: PlayerId,
        objects: Arc<Vec<ObjectId>>,
        cursor: usize,
        declarations: Vec<BlockDeclaration>,
    },
}

#[derive(Clone)]
struct CombatSearchState {
    driver: GameDriver,
    finished: bool,
    terminal_prior: i64,
    progress: CombatSearchProgress,
    context: Option<Arc<DecisionContext>>,
}

struct CombatSearchDomain<'a> {
    root: &'a GameDriver,
    actor: PlayerId,
    weights: AiWeights,
    progress: CombatSearchProgress,
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
            activated_abilities: Vec::new(),
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
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

    fn search_clone_with_state(&self, state: GameState) -> Self {
        Self {
            state,
            players: self.players.clone(),
            programs: Arc::clone(&self.programs),
            deck_models: Arc::clone(&self.deck_models),
            card_definitions: Arc::clone(&self.card_definitions),
            guardrails: Arc::clone(&self.guardrails),
            commanders: self.commanders.clone(),
            trigger_programs: self.trigger_programs.clone(),
            activated_abilities: self.activated_abilities.clone(),
            pending_activated_resolution: self.pending_activated_resolution.clone(),
            pending_triggered_resolution: self.pending_triggered_resolution.clone(),
            triggers_registered_for: self.triggers_registered_for.clone(),
            permanent_runtime_registered_for: self.permanent_runtime_registered_for.clone(),
            commander_zone_decisions: self.commander_zone_decisions.clone(),
            current_attacks: self.current_attacks.clone(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: self.seed,
        }
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

        let state = self
            .state
            .determinized_clone(observer, &slots)
            .map_err(|error| {
                format!(
                    "seed {} failed to bind determinized state: {error:?}",
                    self.seed
                )
            })?;
        let mut clone = self.search_clone_with_state(state);
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
        let repeat_main_actions = ai_policies.is_some()
            || (human.is_some()
                && decisions
                    .as_ref()
                    .is_some_and(|source| !source.is_legacy_replay()));

        while self.state.game_outcome() == GameOutcome::InProgress {
            if self.pending_activated_resolution.is_some() {
                self.complete_pending_activated_resolution(
                    human,
                    &mut decisions,
                    ai_policies.as_ref(),
                )?;
                continue;
            }
            if self.pending_triggered_resolution.is_some() {
                self.complete_pending_triggered_resolution(
                    human,
                    &mut decisions,
                    ai_policies.as_ref(),
                )?;
                continue;
            }
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
                let outcome = self.put_pending_triggers_on_stack(
                    human,
                    &mut decisions,
                    ai_policies.as_ref(),
                )?;
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
                Step::PrecombatMain
                    if active_has_priority && (repeat_main_actions || main_done.insert(turn)) =>
                {
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
                        self.assign_combat_damage(human, &mut decisions, ai_policies.as_ref())?;
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
                        self.take_controlled_priority_action(
                            human,
                            &mut decisions,
                            ai_policies.as_ref(),
                        )?;
                    }
                }
                _ => self.take_controlled_priority_action(
                    human,
                    &mut decisions,
                    ai_policies.as_ref(),
                )?,
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
        match self.prompt_context_selection(source, kind, context, options, false)? {
            CanonicalPromptSelection::Option(selected) => Ok(selected),
            CanonicalPromptSelection::RequestConcession => Err(format!(
                "seed {} prompt `{kind}` accepted concession outside a main or priority window",
                self.seed
            )),
        }
    }

    fn prompt_context_choice_allowing_concession(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
    ) -> Result<CanonicalPromptSelection, String> {
        self.prompt_context_selection(source, kind, context, options, true)
    }

    fn prompt_context_selection(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
        allow_concession: bool,
    ) -> Result<CanonicalPromptSelection, String> {
        let view = self
            .state
            .player_view(context.actor())
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        let selected = source.choose(&DecisionPrompt {
            kind,
            view: &view,
            context,
            options,
            allow_concession,
        })?;
        let DecisionSelection::Option(selected) = selected else {
            return Ok(CanonicalPromptSelection::RequestConcession);
        };
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
        Ok(CanonicalPromptSelection::Option(selected.id()))
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
            let selected_id = match self.prompt_context_choice_allowing_concession(
                source,
                "Choose a main-phase action",
                &context,
                &labels,
            )? {
                CanonicalPromptSelection::Option(selected) => selected,
                CanonicalPromptSelection::RequestConcession => {
                    self.take_human_concession(player, source)?;
                    return Ok(());
                }
            };
            let choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} human main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.finish_human_main_choice(player, source, choice)? {
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
            if self.apply_main_choice(player, choices[selected].clone())? {
                return Ok(());
            }
        }
    }

    fn take_human_priority_action(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        let (context, mappings) = self.priority_decision_context(player)?;
        let labels = context
            .options()
            .iter()
            .map(|option| match option.descriptor() {
                DecisionDescriptor::PassPriority => Ok("Pass priority".to_owned()),
                descriptor => self.main_choice_label(descriptor),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selected_id = match self.prompt_context_choice_allowing_concession(
            source,
            "Choose a priority action",
            &context,
            &labels,
        )? {
            CanonicalPromptSelection::Option(selected) => selected,
            CanonicalPromptSelection::RequestConcession => {
                self.take_human_concession(player, source)?;
                return Ok(());
            }
        };
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
            .ok_or_else(|| {
                format!(
                    "seed {} human priority action {selected_id} has no typed adapter",
                    self.seed
                )
            })?;
        self.finish_human_main_choice(player, source, choice)?;
        Ok(())
    }

    fn finish_human_main_choice(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
        mut choice: MainChoice,
    ) -> Result<bool, String> {
        while let Some((context, mappings)) = self.hierarchical_cast_context(player, &choice)? {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::ChooseNumber { value } => Ok(format!("Choose X = {value}")),
                    DecisionDescriptor::ChooseNumberRange { minimum, maximum } => {
                        Ok(format!("Narrow X to {minimum}-{maximum}"))
                    }
                    DecisionDescriptor::ChoosePayment { payment } => Ok(format!(
                        "Pay for X={} (payment waste {})",
                        payment.x_value(),
                        payment.waste_score()
                    )),
                    descriptor => Err(format!(
                        "seed {} cannot label hierarchical cast descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let prompt = match context.kind() {
                DecisionKind::NumericValue => "Choose X",
                DecisionKind::Payment => "Choose a mana payment",
                other => {
                    return Err(format!(
                        "seed {} unexpected hierarchical cast context {other:?}",
                        self.seed
                    ));
                }
            };
            let selected_id = self.prompt_context_choice(source, prompt, &context, &labels)?;
            choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} hierarchical cast action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
        }
        self.apply_main_choice(player, choice)
    }

    fn take_human_concession(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        let context = concession_decision_context(&self.state, player)?;
        let labels = vec!["Concede the game".to_owned()];
        let view = self
            .state
            .player_view(player)
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        let selected = source.choose_concession(&DecisionPrompt {
            kind: CONCESSION_PROMPT,
            view: &view,
            context: &context,
            options: &labels,
            allow_concession: false,
        })?;
        let option = context.options().get(selected).ok_or_else(|| {
            format!(
                "seed {} concession prompt selected option {selected} outside {} choices",
                self.seed,
                context.options().len()
            )
        })?;
        context.select(option.id()).map_err(|error| {
            format!(
                "seed {} concession prompt selected an illegal action: {error}",
                self.seed
            )
        })?;
        for action in option.actions().to_vec() {
            self.dispatch(action)?;
        }
        Ok(())
    }

    fn take_ai_priority_action(
        &mut self,
        player: PlayerId,
        policy: AiController,
    ) -> Result<(), String> {
        let decision_started = Instant::now();
        let (context, mappings) = self.priority_decision_context(player)?;
        let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
            (context.options()[0].id(), None, "forced-v1", Vec::new())
        } else {
            let profile = policy
                .guardrail_profile()
                .unwrap_or(GuardrailProfile::Standard);
            let candidates = if matches!(policy, AiController::Random(_)) {
                Vec::new()
            } else {
                self.policy_candidates(&context, player, |option| {
                    self.main_action_prior(&context, option.descriptor(), profile)
                })?
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, &context, &candidates, "priority")?;
            (selected_id, decision, policy_name, candidates)
        };
        context.select(selected_id).map_err(|error| {
            format!(
                "seed {} AI selected illegal priority action: {error}",
                self.seed
            )
        })?;
        self.record_ai_decision(AiDecisionTelemetry {
            kind: "priority",
            policy: policy_name,
            context: &context,
            action_id: selected_id,
            decision,
            evaluated_candidates: candidates.len(),
            wall_latency_us: elapsed_us(decision_started),
            score_override: None,
            stop_reason: if context.options().len() == 1 {
                "single_legal_action"
            } else if decision.is_some() {
                "one_ply_complete"
            } else {
                "random_legal_selection"
            },
        });
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
            .ok_or_else(|| {
                format!(
                    "seed {} AI priority action {selected_id} has no typed adapter",
                    self.seed
                )
            })?;
        self.finish_ai_main_choice(player, policy, choice)?;
        Ok(())
    }

    fn take_controlled_priority_action(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(), String> {
        let player = self
            .state
            .priority_player()
            .ok_or_else(|| format!("seed {} cannot choose without priority", self.seed))?;
        if human == Some(player) {
            let source = decisions
                .as_deref_mut()
                .ok_or_else(|| "human game is missing a decision source".to_owned())?;
            if source.is_legacy_replay() {
                self.pass_priority()
            } else {
                self.take_human_priority_action(player, source)
            }
        } else if let Some(policies) = ai_policies {
            let policy = self.policy_for(player, policies)?;
            self.take_ai_priority_action(player, policy)
        } else {
            self.pass_priority()
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
            DecisionDescriptor::ActivateProgramAbility {
                source,
                payment,
                targets,
                optional,
                ..
            } => {
                let mut details = vec![format!("payment waste {}", payment.waste_score())];
                if !targets.is_empty() {
                    details.push(format!(
                        "targets {}",
                        targets
                            .iter()
                            .copied()
                            .map(|target| self.target_choice_label(target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !optional.is_empty() {
                    details.push(format!(
                        "optional {}",
                        optional
                            .iter()
                            .map(|accept| if *accept { "yes" } else { "no" })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                Ok(format!(
                    "Activate ability: {} ({})",
                    self.object_name(*source),
                    details.join("; ")
                ))
            }
            DecisionDescriptor::CastSpell {
                object,
                payment,
                targets,
                modes,
                optional,
            } => {
                let mut details = vec![format!("payment waste {}", payment.waste_score())];
                if !targets.is_empty() {
                    details.push(format!(
                        "targets {}",
                        targets
                            .iter()
                            .copied()
                            .map(|target| self.target_choice_label(target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !modes.is_empty() {
                    details.push(format!(
                        "mode {}",
                        modes
                            .iter()
                            .map(|mode| mode.saturating_add(1).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !optional.is_empty() {
                    details.push(format!(
                        "optional {}",
                        optional
                            .iter()
                            .map(|accept| if *accept { "yes" } else { "no" })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                Ok(format!(
                    "Cast: {} ({})",
                    self.object_name(*object),
                    details.join("; ")
                ))
            }
            DecisionDescriptor::BeginCastSpell {
                object,
                targets,
                modes,
                optional,
            } => {
                let mut details = vec!["choose X and payment".to_owned()];
                if !targets.is_empty() {
                    details.push(format!(
                        "targets {}",
                        targets
                            .iter()
                            .copied()
                            .map(|target| self.target_choice_label(target))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !modes.is_empty() {
                    details.push(format!(
                        "mode {}",
                        modes
                            .iter()
                            .map(|mode| mode.saturating_add(1).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !optional.is_empty() {
                    details.push(format!(
                        "optional {}",
                        optional
                            .iter()
                            .map(|accept| if *accept { "yes" } else { "no" })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                Ok(format!(
                    "Cast: {} ({})",
                    self.object_name(*object),
                    details.join("; ")
                ))
            }
            DecisionDescriptor::PassPriority => Ok("Finish main phase".to_owned()),
            other => Err(format!(
                "seed {} main prompt cannot label descriptor {other:?}",
                self.seed
            )),
        }
    }

    fn target_choice_label(&self, target: TargetChoice) -> String {
        match target {
            TargetChoice::Player(player) => self
                .players
                .iter()
                .position(|candidate| *candidate == player)
                .map_or_else(
                    || format!("player {}", player.index()),
                    |seat| format!("seat {}", seat + 1),
                ),
            TargetChoice::Object(object) => self.object_name(object),
            TargetChoice::StackEntry(entry) => format!("stack entry {}", entry.index()),
        }
    }

    fn activated_runtime(&self, ability: ActivatedAbilityId) -> Result<&ActivatedRuntime, String> {
        self.activated_abilities
            .iter()
            .find(|registered| registered.id == ability)
            .and_then(|registered| registered.runtime.as_ref())
            .ok_or_else(|| {
                format!(
                    "seed {} registered ability {} has no card runtime",
                    self.seed,
                    ability.index()
                )
            })
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
            MainChoice::ActivateProgram {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                let runtime = self.activated_runtime(ability)?.clone();
                let effect = runtime
                    .program
                    .activated_effects()
                    .get(runtime.ability_index)
                    .ok_or_else(|| {
                        format!(
                            "seed {} missing activated runtime {} on {}",
                            self.seed,
                            runtime.ability_index,
                            runtime.program.name()
                        )
                    })?;
                let decisions = StackDecisionBindings::new(None, &optional).map_err(|error| {
                    format!("seed {} activation choices failed: {error:?}", self.seed)
                })?;
                let outcome = self.dispatch(Action::ActivateProgramAbility {
                    player,
                    ability,
                    payment,
                    target_requirements: effect.target_requirements().to_vec(),
                    target_choices: targets,
                    decisions,
                })?;
                if !matches!(outcome, Outcome::StackEntryAdded(_)) {
                    return Err(format!(
                        "seed {} program activation on {} returned {outcome:?}",
                        self.seed,
                        self.object_name(source)
                    ));
                }
                Ok(true)
            }
            MainChoice::BeginCast { .. }
            | MainChoice::NarrowCastX { .. }
            | MainChoice::ChooseCastX { .. } => Err(format!(
                "seed {} attempted to dispatch an incomplete hierarchical cast",
                self.seed
            )),
            MainChoice::Cast {
                object,
                payment,
                targets,
                mode,
                optional,
            } => {
                self.cast_program_with_choices(player, object, payment, targets, mode, optional)?;
                Ok(true)
            }
            MainChoice::Finish => {
                self.pass_priority()?;
                Ok(true)
            }
        }
    }

    fn spell_choice_bindings(
        &self,
        player: PlayerId,
        object: ObjectId,
        program: &CardProgram,
    ) -> Result<Vec<SpellChoiceBinding>, String> {
        if !program.additional_costs().is_empty() {
            return Err(format!(
                "seed {} spell {} requires an additional-cost adapter",
                self.seed,
                program.name()
            ));
        }
        let mut bindings = Vec::new();
        if program.spell_modes().is_empty() {
            self.extend_spell_branch_bindings(
                player,
                object,
                program.name(),
                None,
                program.target_requirements(),
                program.object_choice_requirements().len(),
                program.optional_choice_count(),
                &mut bindings,
            )?;
        } else {
            for (mode_index, mode) in program.spell_modes().iter().enumerate() {
                let mode_index = u32::try_from(mode_index)
                    .map_err(|_| format!("seed {} spell mode index overflow", self.seed))?;
                self.extend_spell_branch_bindings(
                    player,
                    object,
                    program.name(),
                    Some(mode_index),
                    mode.target_requirements(),
                    mode.object_choice_requirements().len(),
                    mode.optional_choice_count(),
                    &mut bindings,
                )?;
            }
        }
        Ok(bindings)
    }

    #[allow(clippy::too_many_arguments)]
    fn extend_spell_branch_bindings(
        &self,
        player: PlayerId,
        object: ObjectId,
        card_name: &str,
        mode: Option<u32>,
        requirements: &[TargetRequirement],
        object_choice_count: usize,
        optional_count: usize,
        output: &mut Vec<SpellChoiceBinding>,
    ) -> Result<(), String> {
        if object_choice_count != 0 {
            return Err(format!(
                "seed {} spell {card_name} requires {object_choice_count} resolution-time object choice(s)",
                self.seed
            ));
        }
        let targets = self.target_bindings(player, object, requirements)?;
        let optionals = self.optional_bindings(card_name, optional_count)?;
        let branch_count = targets
            .len()
            .checked_mul(optionals.len())
            .ok_or_else(|| format!("seed {} spell option count overflow", self.seed))?;
        if output.len().saturating_add(branch_count) > MAX_CANONICAL_SPELL_OPTIONS {
            return Err(format!(
                "seed {} spell {card_name} exceeds the {}-option canonical cap",
                self.seed, MAX_CANONICAL_SPELL_OPTIONS
            ));
        }
        for target_binding in targets {
            for optional in &optionals {
                output.push(SpellChoiceBinding {
                    targets: target_binding.clone(),
                    mode,
                    optional: optional.clone(),
                });
            }
        }
        Ok(())
    }

    fn target_bindings(
        &self,
        player: PlayerId,
        source: ObjectId,
        requirements: &[TargetRequirement],
    ) -> Result<Vec<Vec<TargetChoice>>, String> {
        let mut bindings = vec![Vec::new()];
        for requirement in requirements {
            let choices = self.legal_targets_for(player, source, *requirement);
            if choices.is_empty() {
                return Ok(Vec::new());
            }
            let next_len = bindings
                .len()
                .checked_mul(choices.len())
                .ok_or_else(|| format!("seed {} target option count overflow", self.seed))?;
            if next_len > MAX_CANONICAL_SPELL_OPTIONS {
                return Err(format!(
                    "seed {} target choices exceed the {}-option canonical cap",
                    self.seed, MAX_CANONICAL_SPELL_OPTIONS
                ));
            }
            let mut next = Vec::with_capacity(next_len);
            for prefix in &bindings {
                for choice in &choices {
                    let mut binding = prefix.clone();
                    binding.push(*choice);
                    next.push(binding);
                }
            }
            bindings = next;
        }
        Ok(bindings)
    }

    fn legal_targets_for(
        &self,
        player: PlayerId,
        source: ObjectId,
        requirement: TargetRequirement,
    ) -> Vec<TargetChoice> {
        let mut choices = Vec::new();
        if matches!(
            requirement.kind(),
            TargetKind::Player | TargetKind::PlayerOrPermanent
        ) {
            choices.extend(
                self.state
                    .players()
                    .iter()
                    .copied()
                    .map(|candidate| TargetChoice::Player(candidate.id())),
            );
        }
        match requirement.kind() {
            TargetKind::Permanent | TargetKind::PlayerOrPermanent => {
                self.extend_object_targets(ZoneId::new(None, ZoneKind::Battlefield), &mut choices);
            }
            TargetKind::ObjectInZone(zone) => self.extend_object_targets(zone, &mut choices),
            TargetKind::ObjectInZoneKind(kind) => {
                self.extend_object_targets(ZoneId::new(None, kind), &mut choices);
                for candidate in self.state.players().iter().copied() {
                    self.extend_object_targets(
                        ZoneId::new(Some(candidate.id()), kind),
                        &mut choices,
                    );
                }
            }
            TargetKind::StackEntry => choices.extend(
                self.state
                    .stack_entries()
                    .iter()
                    .map(|entry| TargetChoice::StackEntry(entry.id())),
            ),
            TargetKind::Player => {}
        }
        choices.retain(|choice| {
            self.state
                .can_target(player, Some(source), requirement, *choice)
        });
        choices.sort_by_key(|choice| match choice {
            TargetChoice::Player(target) => (0_u8, target.index()),
            TargetChoice::Object(target) => (1_u8, target.index()),
            TargetChoice::StackEntry(target) => (2_u8, target.index()),
        });
        choices.dedup();
        choices
    }

    fn extend_object_targets(&self, zone: ZoneId, output: &mut Vec<TargetChoice>) {
        if let Some(objects) = self.state.zone_objects(zone) {
            output.extend(objects.iter().copied().map(TargetChoice::Object));
        }
    }

    fn optional_bindings(
        &self,
        card_name: &str,
        optional_count: usize,
    ) -> Result<Vec<Vec<bool>>, String> {
        let shift = u32::try_from(optional_count)
            .map_err(|_| format!("seed {} optional choice count overflow", self.seed))?;
        let count = 1_usize.checked_shl(shift).ok_or_else(|| {
            format!(
                "seed {} spell {card_name} has too many optional choices",
                self.seed
            )
        })?;
        if count > MAX_CANONICAL_SPELL_OPTIONS {
            return Err(format!(
                "seed {} spell {card_name} optional choices exceed the {}-option canonical cap",
                self.seed, MAX_CANONICAL_SPELL_OPTIONS
            ));
        }
        Ok((0..count)
            .map(|mask| {
                (0..optional_count)
                    .map(|index| (mask & (1_usize << index)) != 0)
                    .collect()
            })
            .collect())
    }

    fn cast_payment_plans(
        &self,
        player: PlayerId,
        object: ObjectId,
        x_value: u32,
    ) -> Result<Vec<PaymentPlan>, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing cast program", self.seed))?;
        let printed_cost = program.mana_cost();
        let announced_cost = printed_cost.with_x(printed_cost.x_count(), x_value);
        let effective_cost = match self
            .state
            .effective_spell_cost(player, object, announced_cost)
        {
            Ok(cost) => cost,
            Err(StateError::ManaValueOverflow | StateError::CommanderTaxOverflow(_)) => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(format!("seed {} spell cost failed: {error:?}", self.seed));
            }
        };
        let payments = match self.state.payment_plans_for_player(player, effective_cost) {
            Ok(payments) => payments,
            Err(StateError::ManaValueOverflow) => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "seed {} payment enumeration failed: {error:?}",
                    self.seed
                ));
            }
        };
        Ok(payments
            .plans()
            .iter()
            .copied()
            .map(|payment| payment.with_x_value(x_value))
            .collect())
    }

    fn maximum_affordable_x(
        &self,
        player: PlayerId,
        object: ObjectId,
    ) -> Result<Option<u32>, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing X spell program", self.seed))?;
        let x_count = program.mana_cost().x_count();
        if x_count == 0 {
            return Err(format!(
                "seed {} requested X range for a fixed cost",
                self.seed
            ));
        }
        if self.cast_payment_plans(player, object, 0)?.is_empty() {
            return Ok(None);
        }

        let mut minimum = 0_u32;
        let mut maximum = u32::MAX / x_count;
        while minimum < maximum {
            let midpoint = minimum + (maximum - minimum).div_ceil(2);
            if self
                .cast_payment_plans(player, object, midpoint)?
                .is_empty()
            {
                maximum = midpoint - 1;
            } else {
                minimum = midpoint;
            }
        }
        Ok(Some(minimum))
    }

    fn legal_cast_choices(&self, player: PlayerId) -> Result<Vec<MainChoice>, String> {
        let seat = self
            .players
            .iter()
            .position(|candidate| *candidate == player)
            .ok_or_else(|| format!("seed {} unknown casting player", self.seed))?;
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
        hand_objects.sort_by_key(|object| object.index());
        candidates.extend(hand_objects);

        let mut choices = Vec::new();
        for object in candidates {
            let Some(program) = self.programs.get(&object) else {
                continue;
            };
            if !self.normal_spell_timing_available(player, program.kind()) {
                continue;
            }
            let spell_bindings = self.spell_choice_bindings(player, object, program)?;
            if program.mana_cost().x_count() != 0 {
                let payments = self.cast_payment_plans(player, object, 0)?;
                if payments.is_empty() {
                    continue;
                }
                for binding in spell_bindings {
                    let is_legal = payments.iter().copied().any(|payment| {
                        self.spell_request(
                            program,
                            payment,
                            &binding.targets,
                            binding.mode,
                            &binding.optional,
                        )
                        .is_ok_and(|request| {
                            self.action_is_legal(&Action::CastSpell {
                                player,
                                object,
                                request,
                            })
                        })
                    });
                    if is_legal {
                        choices.push(MainChoice::BeginCast {
                            object,
                            targets: binding.targets,
                            mode: binding.mode,
                            optional: binding.optional,
                        });
                    }
                }
                continue;
            }

            let payment_options = self.cast_payment_plans(player, object, 0)?;
            let option_count = payment_options
                .len()
                .checked_mul(spell_bindings.len())
                .ok_or_else(|| format!("seed {} spell option count overflow", self.seed))?;
            if option_count > MAX_CANONICAL_SPELL_OPTIONS {
                return Err(format!(
                    "seed {} spell {} exceeds the {}-option canonical cap after payments",
                    self.seed,
                    program.name(),
                    MAX_CANONICAL_SPELL_OPTIONS
                ));
            }
            for binding in spell_bindings {
                for payment in payment_options.iter().copied() {
                    let request = self.spell_request(
                        program,
                        payment,
                        &binding.targets,
                        binding.mode,
                        &binding.optional,
                    )?;
                    let action = Action::CastSpell {
                        player,
                        object,
                        request,
                    };
                    if self.action_is_legal(&action) {
                        choices.push(MainChoice::Cast {
                            object,
                            payment,
                            targets: binding.targets.clone(),
                            mode: binding.mode,
                            optional: binding.optional.clone(),
                        });
                    }
                }
            }
        }
        Ok(choices)
    }

    fn legal_activation_choices(&self, player: PlayerId) -> Result<Vec<MainChoice>, String> {
        let mut choices = Vec::new();
        for registered in &self.activated_abilities {
            if registered.controller != player
                || self.state.object_zone(registered.source)
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
            {
                continue;
            }
            let cost = self
                .state
                .effective_activation_cost(registered.id)
                .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
            let payments = self
                .state
                .payment_plans_for_player(player, cost.mana())
                .map_err(|error| {
                    format!("seed {} payment enumeration failed: {error:?}", self.seed)
                })?;
            let Some(runtime) = registered.runtime.as_ref() else {
                for payment in payments.plans().iter().copied() {
                    let action = Action::ActivateAbility {
                        player,
                        ability: registered.id,
                        payment,
                    };
                    if self.action_is_legal(&action) {
                        choices.push(MainChoice::Activate {
                            source: registered.source,
                            ability: registered.id,
                            payment,
                        });
                    }
                }
                continue;
            };
            let ability = runtime
                .program
                .activated_effects()
                .get(runtime.ability_index)
                .ok_or_else(|| {
                    format!(
                        "seed {} missing activated runtime {} on {}",
                        self.seed,
                        runtime.ability_index,
                        runtime.program.name()
                    )
                })?;
            let targets =
                self.target_bindings(player, runtime.source, ability.target_requirements())?;
            let optionals =
                self.optional_bindings(runtime.program.name(), ability.optional_choice_count())?;
            let branch_count = targets
                .len()
                .checked_mul(optionals.len())
                .and_then(|count| count.checked_mul(payments.plans().len()))
                .ok_or_else(|| format!("seed {} activation option count overflow", self.seed))?;
            if branch_count > MAX_CANONICAL_SPELL_OPTIONS {
                return Err(format!(
                    "seed {} activated ability on {} exceeds the {}-option canonical cap",
                    self.seed,
                    runtime.program.name(),
                    MAX_CANONICAL_SPELL_OPTIONS
                ));
            }
            for target_binding in targets {
                for optional in &optionals {
                    let decisions =
                        StackDecisionBindings::new(None, optional).map_err(|error| {
                            format!("seed {} activation choices failed: {error:?}", self.seed)
                        })?;
                    for payment in payments.plans().iter().copied() {
                        let action = Action::ActivateProgramAbility {
                            player,
                            ability: registered.id,
                            payment,
                            target_requirements: ability.target_requirements().to_vec(),
                            target_choices: target_binding.clone(),
                            decisions,
                        };
                        if self.action_is_legal(&action) {
                            choices.push(MainChoice::ActivateProgram {
                                source: registered.source,
                                ability: registered.id,
                                payment,
                                targets: target_binding.clone(),
                                optional: optional.clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(choices)
    }

    fn normal_spell_timing_available(&self, player: PlayerId, kind: ProgramKind) -> bool {
        if self.state.priority_player() != Some(player) {
            return false;
        }
        match kind {
            ProgramKind::Instant => true,
            ProgramKind::Permanent | ProgramKind::Sorcery => {
                self.state.active_player() == Some(player)
                    && matches!(
                        self.state.current_step(),
                        Some(Step::PrecombatMain | Step::PostcombatMain)
                    )
                    && self.state.stack_entries().is_empty()
            }
            ProgramKind::Land => false,
        }
    }

    fn spell_request(
        &self,
        program: &CardProgram,
        payment: PaymentPlan,
        targets: &[TargetChoice],
        mode: Option<u32>,
        optional: &[bool],
    ) -> Result<CastSpellRequest, String> {
        let (kind, timing) = match program.kind() {
            ProgramKind::Permanent => (StackObjectKind::PermanentSpell, SpellTiming::Sorcery),
            ProgramKind::Instant => (StackObjectKind::InstantSpell, SpellTiming::Instant),
            ProgramKind::Sorcery => (StackObjectKind::SorcerySpell, SpellTiming::Sorcery),
            ProgramKind::Land => {
                return Err(format!("seed {} cannot cast a land as a spell", self.seed));
            }
        };
        let requirements = match mode {
            Some(mode) => program
                .spell_modes()
                .get(mode as usize)
                .ok_or_else(|| format!("seed {} invalid spell mode {mode}", self.seed))?
                .target_requirements(),
            None if program.spell_modes().is_empty() => program.target_requirements(),
            None => {
                return Err(format!(
                    "seed {} modal spell {} has no mode binding",
                    self.seed,
                    program.name()
                ));
            }
        };
        let decisions = StackDecisionBindings::new(mode, optional)
            .map_err(|error| format!("seed {} stack choices failed: {error:?}", self.seed))?;
        let announced_cost = program
            .mana_cost()
            .with_x(program.mana_cost().x_count(), payment.x_value());
        let mut request = CastSpellRequest::new(kind, timing, announced_cost, payment)
            .with_targets(requirements.to_vec(), targets.to_vec())
            .with_decisions(decisions);
        if program.split_second() {
            request = request.with_split_second();
        }
        Ok(request)
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

        let activations = self.legal_activation_choices(player)?;
        if activations
            .iter()
            .any(|choice| matches!(choice, MainChoice::Activate { .. }))
        {
            labels.push("Activate all available mana sources".to_owned());
            choices.push(MainChoice::ActivateAll);
        }
        for choice in activations {
            let source = match &choice {
                MainChoice::Activate { source, .. }
                | MainChoice::ActivateProgram { source, .. } => *source,
                _ => unreachable!("activation enumeration returned a non-activation choice"),
            };
            labels.push(format!("Activate ability: {}", self.object_name(source)));
            choices.push(choice);
        }

        for choice in self.legal_cast_choices(player)? {
            let object = match &choice {
                MainChoice::Cast { object, .. } | MainChoice::BeginCast { object, .. } => *object,
                _ => unreachable!("legal cast enumeration returned a non-cast choice"),
            };
            labels.push(format!("Cast: {}", self.object_name(object)));
            choices.push(choice);
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
        for registered in &self.activated_abilities {
            if registered.runtime.is_some()
                || registered.controller != player
                || self.state.object_zone(registered.source)
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
                || self
                    .state
                    .object(registered.source)
                    .map_or(true, |record| record.tapped())
            {
                continue;
            }
            let cost = self
                .state
                .effective_activation_cost(registered.id)
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
                ability: registered.id,
                payment,
            };
            if self.action_is_legal(&action) {
                activations.push((registered.source, registered.id, payment));
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
                choices.push(MainChoice::Cast {
                    object,
                    payment,
                    targets: Vec::new(),
                    mode: None,
                    optional: Vec::new(),
                });
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
            let resource_started =
                matches!(policy, AiController::Search(_)).then(ResourceSnapshot::capture);
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
                    let mut config = controller
                        .config(decision_index)
                        .with_decision_started(decision_started);
                    if let Some(resource_started) = resource_started {
                        config = config.with_resource_started(resource_started);
                    }
                    let report =
                        SearchEngine::search(&domain, &context, &config).map_err(|error| {
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
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} AI main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.finish_ai_main_choice(player, policy, choice)? {
                return Ok(());
            }
        }
    }

    fn finish_ai_main_choice(
        &mut self,
        player: PlayerId,
        policy: AiController,
        mut choice: MainChoice,
    ) -> Result<bool, String> {
        while let Some((context, mappings)) = self.hierarchical_cast_context(player, &choice)? {
            let decision_started = Instant::now();
            let kind = match context.kind() {
                DecisionKind::NumericValue => "numeric_value",
                DecisionKind::Payment => "payment",
                other => {
                    return Err(format!(
                        "seed {} unexpected AI cast context {other:?}",
                        self.seed
                    ));
                }
            };
            let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
                (context.options()[0].id(), None, "forced-v1", Vec::new())
            } else {
                let candidates = if matches!(policy, AiController::Random(_)) {
                    Vec::new()
                } else {
                    self.policy_candidates(&context, player, |option| match option.descriptor() {
                        DecisionDescriptor::ChooseNumber { value } => i64::from(*value),
                        DecisionDescriptor::ChooseNumberRange { maximum, .. } => {
                            i64::from(*maximum)
                        }
                        DecisionDescriptor::ChoosePayment { payment } => {
                            -i64::from(payment.waste_score())
                        }
                        _ => 0,
                    })?
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, &context, &candidates, kind)?;
                (selected_id, decision, policy_name, candidates)
            };
            context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} AI selected illegal hierarchical cast action: {error}",
                    self.seed
                )
            })?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind,
                policy: policy_name,
                context: &context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if context.options().len() == 1 {
                    "single_legal_action"
                } else if decision.is_some() {
                    "one_ply_complete"
                } else {
                    "random_legal_selection"
                },
            });
            choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} AI hierarchical cast action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
        }
        self.apply_main_choice(player, choice)
    }

    fn main_decision_context(
        &self,
        player: PlayerId,
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        let (_, choices) = self.human_main_choices(player)?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for choice in choices {
            if matches!(&choice, MainChoice::ActivateAll) {
                continue;
            }
            let option = self.main_choice_option(player, choice.clone())?;
            mappings.push((option.id(), choice));
            options.push(option);
        }
        let context = self.decision_context(DecisionKind::MainPhase, player, options)?;
        Ok((context, mappings))
    }

    fn priority_decision_context(
        &self,
        player: PlayerId,
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        let mut choices = self.legal_activation_choices(player)?;
        choices.extend(self.legal_cast_choices(player)?);
        choices.push(MainChoice::Finish);
        let mut mappings = Vec::with_capacity(choices.len());
        let mut options = Vec::with_capacity(choices.len());
        for choice in choices {
            let option = self.main_choice_option(player, choice.clone())?;
            mappings.push((option.id(), choice));
            options.push(option);
        }
        let context = self.decision_context(DecisionKind::Priority, player, options)?;
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
            MainChoice::ActivateProgram {
                source,
                ability,
                payment,
                targets,
                optional,
            } => {
                let runtime = self.activated_runtime(ability)?;
                let effect = runtime
                    .program
                    .activated_effects()
                    .get(runtime.ability_index)
                    .ok_or_else(|| {
                        format!(
                            "seed {} missing activated runtime {} on {}",
                            self.seed,
                            runtime.ability_index,
                            runtime.program.name()
                        )
                    })?;
                let decisions = StackDecisionBindings::new(None, &optional).map_err(|error| {
                    format!("seed {} activation choices failed: {error:?}", self.seed)
                })?;
                Ok(DecisionOption::new(
                    DecisionDescriptor::ActivateProgramAbility {
                        source,
                        ability,
                        payment,
                        targets: targets.clone(),
                        optional,
                    },
                    vec![Action::ActivateProgramAbility {
                        player,
                        ability,
                        payment,
                        target_requirements: effect.target_requirements().to_vec(),
                        target_choices: targets,
                        decisions,
                    }],
                ))
            }
            MainChoice::BeginCast {
                object,
                targets,
                mode,
                optional,
            } => Ok(DecisionOption::new(
                DecisionDescriptor::BeginCastSpell {
                    object,
                    targets,
                    modes: mode.into_iter().collect(),
                    optional,
                },
                Vec::new(),
            )),
            MainChoice::NarrowCastX { .. } | MainChoice::ChooseCastX { .. } => Err(format!(
                "seed {} hierarchical cast stage cannot enter a root context",
                self.seed
            )),
            MainChoice::Cast {
                object,
                payment,
                targets,
                mode,
                optional,
            } => {
                let program = self.programs.get(&object).ok_or_else(|| {
                    format!("seed {} missing program for AI cast option", self.seed)
                })?;
                let request = self.spell_request(program, payment, &targets, mode, &optional)?;
                Ok(DecisionOption::new(
                    DecisionDescriptor::CastSpell {
                        object,
                        payment,
                        targets,
                        modes: mode.into_iter().collect(),
                        optional,
                    },
                    vec![Action::CastSpell {
                        player,
                        object,
                        request,
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

    #[allow(clippy::too_many_arguments)]
    fn variable_cast_numeric_context(
        &self,
        player: PlayerId,
        object: ObjectId,
        targets: &[TargetChoice],
        mode: Option<u32>,
        optional: &[bool],
        bounds: (u32, u32),
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        if bounds.0 > bounds.1 {
            return Err(format!("seed {} invalid X bounds {bounds:?}", self.seed));
        }
        let direct = bounds.1 - bounds.0 < MAX_DIRECT_NUMERIC_VALUES;
        let ranges = if direct {
            Vec::new()
        } else {
            let midpoint = bounds.0 + (bounds.1 - bounds.0) / 2;
            vec![(bounds.0, midpoint), (midpoint + 1, bounds.1)]
        };
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        if direct {
            for x_value in bounds.0..=bounds.1 {
                let option = DecisionOption::new(
                    DecisionDescriptor::ChooseNumber { value: x_value },
                    Vec::new(),
                );
                mappings.push((
                    option.id(),
                    MainChoice::ChooseCastX {
                        object,
                        targets: targets.to_vec(),
                        mode,
                        optional: optional.to_vec(),
                        x_value,
                    },
                ));
                options.push(option);
            }
        } else {
            for (minimum, maximum) in ranges {
                let option = DecisionOption::new(
                    DecisionDescriptor::ChooseNumberRange { minimum, maximum },
                    Vec::new(),
                );
                mappings.push((
                    option.id(),
                    MainChoice::NarrowCastX {
                        object,
                        targets: targets.to_vec(),
                        mode,
                        optional: optional.to_vec(),
                        minimum,
                        maximum,
                    },
                ));
                options.push(option);
            }
        }
        let context = self.scoped_decision_context(
            DecisionKind::NumericValue,
            player,
            options,
            variable_cast_path_discriminator(
                player, object, targets, mode, optional, 0, bounds.0, bounds.1,
            ),
        )?;
        Ok((context, mappings))
    }

    #[allow(clippy::too_many_arguments)]
    fn variable_cast_payment_context(
        &self,
        player: PlayerId,
        object: ObjectId,
        targets: &[TargetChoice],
        mode: Option<u32>,
        optional: &[bool],
        x_value: u32,
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing X spell program", self.seed))?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for payment in self.cast_payment_plans(player, object, x_value)? {
            let request = self.spell_request(program, payment, targets, mode, optional)?;
            let action = Action::CastSpell {
                player,
                object,
                request,
            };
            if !self.action_is_legal(&action) {
                continue;
            }
            let option =
                DecisionOption::new(DecisionDescriptor::ChoosePayment { payment }, vec![action]);
            mappings.push((
                option.id(),
                MainChoice::Cast {
                    object,
                    payment,
                    targets: targets.to_vec(),
                    mode,
                    optional: optional.to_vec(),
                },
            ));
            options.push(option);
        }
        if options.is_empty() {
            return Err(format!(
                "seed {} selected unaffordable X={x_value} for {}",
                self.seed,
                program.name()
            ));
        }
        let context = self.scoped_decision_context(
            DecisionKind::Payment,
            player,
            options,
            variable_cast_path_discriminator(
                player, object, targets, mode, optional, 1, x_value, x_value,
            ),
        )?;
        Ok((context, mappings))
    }

    fn hierarchical_cast_context(
        &self,
        player: PlayerId,
        choice: &MainChoice,
    ) -> Result<Option<MainDecisionAdapter>, String> {
        match choice {
            MainChoice::BeginCast {
                object,
                targets,
                mode,
                optional,
            } => {
                let maximum = self.maximum_affordable_x(player, *object)?.ok_or_else(|| {
                    format!("seed {} selected an unaffordable X spell", self.seed)
                })?;
                self.variable_cast_numeric_context(
                    player,
                    *object,
                    targets,
                    *mode,
                    optional,
                    (0, maximum),
                )
                .map(Some)
            }
            MainChoice::NarrowCastX {
                object,
                targets,
                mode,
                optional,
                minimum,
                maximum,
            } => self
                .variable_cast_numeric_context(
                    player,
                    *object,
                    targets,
                    *mode,
                    optional,
                    (*minimum, *maximum),
                )
                .map(Some),
            MainChoice::ChooseCastX {
                object,
                targets,
                mode,
                optional,
                x_value,
            } => self
                .variable_cast_payment_context(player, *object, targets, *mode, optional, *x_value)
                .map(Some),
            _ => Ok(None),
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

    fn scoped_decision_context(
        &self,
        kind: DecisionKind,
        actor: PlayerId,
        options: Vec<DecisionOption>,
        path_discriminator: u64,
    ) -> Result<DecisionContext, String> {
        let view = self
            .state
            .player_view(actor)
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        DecisionContext::new_scoped(kind, actor, &view, options, Vec::new(), path_discriminator)
            .map_err(|error| format!("seed {} scoped decision context failed: {error}", self.seed))
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
            path_discriminator: context.path_discriminator(),
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
            path_discriminator: context.path_discriminator(),
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
        let abilities = self.activated_abilities.clone();
        for registered in abilities {
            if registered.runtime.is_some()
                || registered.controller != player
                || self.state.object_zone(registered.source)
                    != Some(ZoneId::new(None, ZoneKind::Battlefield))
                || self
                    .state
                    .object(registered.source)
                    .map_or(true, |record| record.tapped())
            {
                continue;
            }
            let cost = self
                .state
                .effective_activation_cost(registered.id)
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
                ability: registered.id,
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
            return self.cast_program_with_choices(
                player,
                object,
                payment,
                Vec::new(),
                None,
                Vec::new(),
            );
        }
        Ok(())
    }

    fn cast_program_with_choices(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        payment: PaymentPlan,
        targets: Vec<TargetChoice>,
        mode: Option<u32>,
        optional: Vec<bool>,
    ) -> Result<(), String> {
        let program = self
            .programs
            .get(&object)
            .cloned()
            .ok_or_else(|| format!("seed {} missing program for cast object", self.seed))?;
        let request = self.spell_request(&program, payment, &targets, mode, &optional)?;
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
            self.activated_abilities.push(RegisteredAbility {
                source,
                controller,
                id: ability_id,
                runtime: None,
            });
        }
        for (ability_index, ability) in program.activated_effects().iter().enumerate() {
            if ability.pay_life() != 0 || ability.sacrifice_cost().is_some() {
                return Err(format!(
                    "seed {} activated ability {} on {} requires an unsupported extra-cost adapter",
                    self.seed,
                    ability_index,
                    program.name()
                ));
            }
            let mut cost = ActivationCost::new(ability.mana_cost());
            if ability.tap_source() {
                cost = cost.with_tap_source();
            }
            if ability.sacrifice_source() {
                cost = cost.with_sacrifice_source();
            }
            let outcome = self.dispatch(Action::RegisterActivatedAbility {
                definition: ActivatedAbilityDefinition::new(
                    controller,
                    Some(source),
                    ability.timing(),
                    cost,
                    ActivatedAbilityEffect::ProgramBound,
                ),
            })?;
            let Outcome::ActivatedAbilityRegistered(ability_id) = outcome else {
                return Err(format!(
                    "seed {} program ability registration returned {outcome:?}",
                    self.seed
                ));
            };
            self.activated_abilities.push(RegisteredAbility {
                source,
                controller,
                id: ability_id,
                runtime: Some(ActivatedRuntime {
                    program: Arc::clone(&program),
                    ability_index,
                    source,
                }),
            });
        }
        Ok(())
    }

    fn object_choice_selections(
        &self,
        controller: PlayerId,
        choice_index: usize,
        requirement: ObjectChoiceRequirement,
    ) -> Result<Vec<Vec<ObjectId>>, String> {
        if requirement.player() != PlayerBinding::Controller {
            return Err(format!(
                "seed {} object choice {choice_index} has an unsupported player binding",
                self.seed
            ));
        }
        let zone = ZoneId::new(Some(controller), requirement.zone());
        let mut candidates = Vec::new();
        for object in self.state.zone_objects(zone).unwrap_or_default() {
            if object_satisfies_choice_requirement(&self.state, requirement, controller, *object)
                .map_err(|error| {
                    format!(
                        "seed {} object choice {choice_index} predicate failed: {error}",
                        self.seed
                    )
                })?
            {
                candidates.push(*object);
            }
        }
        candidates.sort_by_key(|object| object.index());
        let selections = bounded_object_combinations(
            &candidates,
            requirement.minimum() as usize,
            requirement.maximum() as usize,
            MAX_CANONICAL_SPELL_OPTIONS,
        )?;
        if selections.is_empty() {
            return Err(format!(
                "seed {} object choice {choice_index} has no legal binding",
                self.seed
            ));
        }
        Ok(selections)
    }

    fn pending_activated_actions(
        &self,
        pending: &PendingActivatedResolution,
        object_choices: Vec<Vec<ObjectId>>,
    ) -> Result<Vec<Action>, String> {
        let effect = pending
            .runtime
            .program
            .activated_effects()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing activated runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?;
        let bindings =
            ExecutionBindings::new(pending.controller, self.live_opponents(pending.controller))
                .with_source(pending.runtime.source)
                .with_targets(pending.targets.clone())
                .with_object_choices(object_choices)
                .with_optional_effect_choices(pending.decisions.optional_choices().collect());
        bind_activated_effect_actions(&self.state, effect, &bindings)
            .map(|actions| {
                actions
                    .into_iter()
                    .map(|action| action.action().clone())
                    .collect()
            })
            .map_err(|error| {
                format!(
                    "seed {} activated interpreter binding failed: {error}",
                    self.seed
                )
            })
    }

    #[cfg(test)]
    fn pending_activated_context(
        &self,
        pending: &PendingActivatedResolution,
    ) -> Result<DecisionContext, String> {
        let effect = pending
            .runtime
            .program
            .activated_effects()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing activated runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?;
        self.pending_activated_choice_context(pending, effect.object_choice_requirements(), 0, &[])
    }

    fn pending_activated_choice_context(
        &self,
        pending: &PendingActivatedResolution,
        requirements: &[ObjectChoiceRequirement],
        cursor: usize,
        prior: &[Vec<ObjectId>],
    ) -> Result<DecisionContext, String> {
        let requirement = requirements.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} activated resolution has no object choice at slot {cursor}",
                self.seed
            )
        })?;
        if prior.len() != cursor {
            return Err(format!(
                "seed {} activated resolution path has {} choices at slot {cursor}",
                self.seed,
                prior.len()
            ));
        }
        let selections = self.object_choice_selections(pending.controller, cursor, requirement)?;
        let final_slot = cursor + 1 == requirements.len();
        let mut options = Vec::with_capacity(selections.len());
        for selection in selections {
            let mut choices = prior.to_vec();
            choices.push(selection);
            let actions = if final_slot {
                self.pending_activated_actions(pending, choices.clone())?
            } else {
                Vec::new()
            };
            options.push(DecisionOption::new(
                DecisionDescriptor::ChooseResolutionObjects { choices },
                actions,
            ));
        }
        let kind = if requirement.zone() == ZoneKind::Library {
            DecisionKind::Search
        } else {
            DecisionKind::HiddenChoice
        };
        self.scoped_decision_context(
            kind,
            pending.controller,
            options,
            resolution_choice_path_discriminator(pending.controller, cursor, prior),
        )
    }

    fn resolution_choice_label(&self, descriptor: &DecisionDescriptor) -> Result<String, String> {
        let DecisionDescriptor::ChooseResolutionObjects { choices } = descriptor else {
            return Err(format!(
                "seed {} cannot label non-resolution descriptor {descriptor:?}",
                self.seed
            ));
        };
        if choices.iter().all(Vec::is_empty) {
            return Ok("Find no matching cards".to_owned());
        }
        Ok(choices
            .iter()
            .enumerate()
            .map(|(index, objects)| {
                if objects.is_empty() {
                    format!("choice {}: no card", index + 1)
                } else {
                    format!(
                        "choice {}: {}",
                        index + 1,
                        objects
                            .iter()
                            .map(|object| self.object_name(*object))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(" | "))
    }

    fn select_resolution_choice(
        &mut self,
        context: &DecisionContext,
        chooser: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        telemetry_kind: &'static str,
    ) -> Result<CanonicalActionId, String> {
        if human == Some(chooser) {
            let labels = context
                .options()
                .iter()
                .map(|option| self.resolution_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for a resolution choice".to_owned()
            })?;
            return self.prompt_context_choice(
                source,
                "Choose cards while resolving",
                context,
                &labels,
            );
        }
        if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(chooser, policies)?;
            let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
                (context.options()[0].id(), None, "forced-v1", Vec::new())
            } else {
                let candidates = if matches!(policy, AiController::Random(_)) {
                    Vec::new()
                } else {
                    self.policy_candidates(context, chooser, |_| 0)?
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, context, &candidates, telemetry_kind)?;
                (selected_id, decision, policy_name, candidates)
            };
            context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} AI selected an illegal resolution choice: {error}",
                    self.seed
                )
            })?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind: telemetry_kind,
                policy: policy_name,
                context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if context.options().len() == 1 {
                    "single_legal_action"
                } else if decision.is_some() {
                    "one_ply_complete"
                } else {
                    "random_legal_selection"
                },
            });
            return Ok(selected_id);
        }
        context
            .options()
            .iter()
            .max_by_key(|option| match option.descriptor() {
                DecisionDescriptor::ChooseResolutionObjects { choices } => {
                    choices.iter().map(Vec::len).sum::<usize>()
                }
                _ => 0,
            })
            .map(DecisionOption::id)
            .ok_or_else(|| format!("seed {} has no autonomous resolution choice", self.seed))
    }

    fn complete_pending_activated_resolution(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(), String> {
        let pending = self
            .pending_activated_resolution
            .take()
            .ok_or_else(|| format!("seed {} has no pending activated resolution", self.seed))?;
        let requirements = pending
            .runtime
            .program
            .activated_effects()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing activated runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?
            .object_choice_requirements()
            .to_vec();
        if requirements.is_empty() {
            return Err(format!(
                "seed {} pending activated resolution has no object choices",
                self.seed
            ));
        }
        let mut choices = Vec::with_capacity(requirements.len());
        let mut actions = Vec::new();
        for cursor in 0..requirements.len() {
            let context =
                self.pending_activated_choice_context(&pending, &requirements, cursor, &choices)?;
            let selected_id = self.select_resolution_choice(
                &context,
                pending.controller,
                human,
                decisions,
                ai_policies,
                "resolution_object_choice",
            )?;
            let selected = context
                .select(selected_id)
                .map_err(|error| format!("seed {} resolution choice failed: {error}", self.seed))?;
            let DecisionDescriptor::ChooseResolutionObjects {
                choices: selected_choices,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} resolution choice returned a non-object descriptor",
                    self.seed
                ));
            };
            choices.clone_from(selected_choices);
            if cursor + 1 == requirements.len() {
                actions = selected.actions().to_vec();
            }
        }
        let action_count = actions.len() as u64;
        for action in actions {
            self.dispatch(action)?;
        }
        self.metrics.interpreter_actions = self
            .metrics
            .interpreter_actions
            .saturating_add(action_count);
        if let Some(exercise) = self.identity_exercise_mut(pending.runtime.source) {
            exercise.effect_actions = exercise.effect_actions.saturating_add(action_count);
        }
        Ok(())
    }

    fn pending_triggered_actions(
        &self,
        pending: &PendingTriggeredResolution,
        object_choices: Vec<Vec<ObjectId>>,
    ) -> Result<Vec<Action>, String> {
        let ability = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing triggered runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?;
        let mut bindings =
            ExecutionBindings::new(pending.controller, self.live_opponents(pending.controller))
                .with_source(pending.runtime.source)
                .with_object_choices(object_choices)
                .with_optional_effect_choices(vec![true; ability.optional_choice_count()]);
        if ability.unless_paid().is_some() {
            bindings = bindings.with_unless_payment(false);
        }
        bind_triggered_ability_actions(&self.state, ability, &bindings)
            .map(|actions| {
                actions
                    .into_iter()
                    .map(|action| action.action().clone())
                    .collect()
            })
            .map_err(|error| format!("seed {} trigger binding failed: {error}", self.seed))
    }

    #[cfg(test)]
    fn pending_triggered_context(
        &self,
        pending: &PendingTriggeredResolution,
    ) -> Result<DecisionContext, String> {
        let ability = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing triggered runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?;
        self.pending_triggered_choice_context(pending, ability.object_choice_requirements(), 0, &[])
    }

    fn pending_triggered_choice_context(
        &self,
        pending: &PendingTriggeredResolution,
        requirements: &[ObjectChoiceRequirement],
        cursor: usize,
        prior: &[Vec<ObjectId>],
    ) -> Result<DecisionContext, String> {
        let requirement = requirements.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} triggered resolution has no object choice at slot {cursor}",
                self.seed
            )
        })?;
        if prior.len() != cursor {
            return Err(format!(
                "seed {} triggered resolution path has {} choices at slot {cursor}",
                self.seed,
                prior.len()
            ));
        }
        let selections = self.object_choice_selections(pending.controller, cursor, requirement)?;
        let final_slot = cursor + 1 == requirements.len();
        let mut options = Vec::with_capacity(selections.len());
        for selection in selections {
            let mut choices = prior.to_vec();
            choices.push(selection);
            let actions = if final_slot {
                self.pending_triggered_actions(pending, choices.clone())?
            } else {
                Vec::new()
            };
            options.push(DecisionOption::new(
                DecisionDescriptor::ChooseResolutionObjects { choices },
                actions,
            ));
        }
        let kind = if requirement.zone() == ZoneKind::Library {
            DecisionKind::Search
        } else {
            DecisionKind::HiddenChoice
        };
        self.scoped_decision_context(
            kind,
            pending.controller,
            options,
            resolution_choice_path_discriminator(pending.controller, cursor, prior),
        )
    }

    fn complete_pending_triggered_resolution(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(), String> {
        let pending = self
            .pending_triggered_resolution
            .take()
            .ok_or_else(|| format!("seed {} has no pending triggered resolution", self.seed))?;
        let requirements = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing triggered runtime {} on {}",
                    self.seed,
                    pending.runtime.ability_index,
                    pending.runtime.program.name()
                )
            })?
            .object_choice_requirements()
            .to_vec();
        if requirements.is_empty() {
            return Err(format!(
                "seed {} pending triggered resolution has no object choices",
                self.seed
            ));
        }
        let mut choices = Vec::with_capacity(requirements.len());
        let mut actions = Vec::new();
        for cursor in 0..requirements.len() {
            let context =
                self.pending_triggered_choice_context(&pending, &requirements, cursor, &choices)?;
            let selected_id = self.select_resolution_choice(
                &context,
                pending.controller,
                human,
                decisions,
                ai_policies,
                "trigger_resolution_object_choice",
            )?;
            let selected = context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} trigger resolution choice failed: {error}",
                    self.seed
                )
            })?;
            let DecisionDescriptor::ChooseResolutionObjects {
                choices: selected_choices,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} trigger resolution returned a non-object descriptor",
                    self.seed
                ));
            };
            choices.clone_from(selected_choices);
            if cursor + 1 == requirements.len() {
                actions = selected.actions().to_vec();
            }
        }
        let action_count = actions.len() as u64;
        for action in actions {
            self.dispatch(action)?;
        }
        self.metrics.interpreter_actions = self
            .metrics
            .interpreter_actions
            .saturating_add(action_count);
        self.metrics.triggers_resolved = self.metrics.triggers_resolved.saturating_add(1);
        if let Some(exercise) = self.identity_exercise_mut(pending.runtime.source) {
            exercise.trigger_resolutions = exercise.trigger_resolutions.saturating_add(1);
        }
        Ok(())
    }

    fn apnap_players(&self, active: PlayerId) -> Result<Vec<PlayerId>, String> {
        let order = if self.state.turn_order().is_empty() {
            self.state
                .players()
                .iter()
                .map(|player| player.id())
                .collect::<Vec<_>>()
        } else {
            self.state.turn_order().to_vec()
        };
        let start = order
            .iter()
            .position(|player| *player == active)
            .ok_or_else(|| format!("seed {} active player is outside turn order", self.seed))?;
        Ok((0..order.len())
            .map(|offset| order[(start + offset) % order.len()])
            .collect())
    }

    fn trigger_order_context(
        &self,
        controller: PlayerId,
        remaining: &[TriggerId],
        controller_prefix: &[TriggerId],
        global_prefix: &[TriggerId],
    ) -> Result<DecisionContext, String> {
        let candidates = remaining.iter().copied().collect::<BTreeSet<_>>();
        let options = candidates
            .into_iter()
            .map(|trigger| {
                let mut triggers = controller_prefix.to_vec();
                triggers.push(trigger);
                DecisionOption::new(DecisionDescriptor::OrderTriggers { triggers }, Vec::new())
            })
            .collect();
        self.scoped_decision_context(
            DecisionKind::TriggerOrder,
            controller,
            options,
            trigger_order_path_discriminator(controller, remaining, global_prefix),
        )
    }

    fn trigger_order_label(&self, trigger: TriggerId, position: usize) -> String {
        self.trigger_programs.get(&trigger).map_or_else(
            || {
                format!(
                    "Put trigger {} in stack position {}",
                    trigger.index(),
                    position
                )
            },
            |runtime| {
                format!(
                    "Put {} ability {} in stack position {}",
                    runtime.program.name(),
                    runtime.ability_index + 1,
                    position
                )
            },
        )
    }

    fn select_trigger_order_choice(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        position: usize,
    ) -> Result<TriggerId, String> {
        let selected_id = if human == Some(controller) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::OrderTriggers { triggers } => triggers
                        .last()
                        .copied()
                        .map(|trigger| self.trigger_order_label(trigger, position))
                        .ok_or_else(|| format!("seed {} trigger-order option is empty", self.seed)),
                    descriptor => Err(format!(
                        "seed {} cannot label trigger-order descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for trigger ordering".to_owned()
            })?;
            self.prompt_context_choice(source, "Order simultaneous triggers", context, &labels)?
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(controller, policies)?;
            let candidates = if matches!(policy, AiController::Random(_)) {
                Vec::new()
            } else {
                self.policy_candidates(context, controller, |_| 0)?
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, context, &candidates, "trigger_order")?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "trigger_order",
                policy: policy_name,
                context,
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
            selected_id
        } else {
            return Err(format!(
                "seed {} trigger-order prompt has no controller",
                self.seed
            ));
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal trigger-order action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::OrderTriggers { triggers } = selected.descriptor() else {
            return Err(format!(
                "seed {} trigger-order context returned a non-order descriptor",
                self.seed
            ));
        };
        triggers
            .last()
            .copied()
            .ok_or_else(|| format!("seed {} trigger-order selection is empty", self.seed))
    }

    fn put_pending_triggers_on_stack(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<Outcome, String> {
        let pending = self.state.pending_triggers().to_vec();
        let active = self
            .state
            .active_player()
            .ok_or_else(|| format!("seed {} cannot order triggers before a turn", self.seed))?;
        let apnap = self.apnap_players(active)?;
        let has_controlled_choice = apnap.iter().copied().any(|controller| {
            let ids = pending
                .iter()
                .filter(|trigger| trigger.controller() == controller)
                .map(|trigger| trigger.trigger())
                .collect::<BTreeSet<_>>();
            ids.len() > 1 && (human == Some(controller) || ai_policies.is_some())
        });
        if !has_controlled_choice {
            return self.dispatch(Action::PutPendingTriggeredAbilitiesOnStack);
        }

        let mut order = Vec::with_capacity(pending.len());
        for controller in apnap {
            let mut remaining = pending
                .iter()
                .filter(|trigger| trigger.controller() == controller)
                .map(|trigger| trigger.trigger())
                .collect::<Vec<_>>();
            if human != Some(controller) && ai_policies.is_none() {
                order.extend(remaining);
                continue;
            }
            let mut controller_prefix = Vec::with_capacity(remaining.len());
            while remaining.iter().copied().collect::<BTreeSet<_>>().len() > 1 {
                let context =
                    self.trigger_order_context(controller, &remaining, &controller_prefix, &order)?;
                let selected = self.select_trigger_order_choice(
                    &context,
                    controller,
                    human,
                    decisions,
                    ai_policies,
                    order.len() + 1,
                )?;
                let index = remaining
                    .iter()
                    .position(|trigger| *trigger == selected)
                    .ok_or_else(|| {
                        format!(
                            "seed {} selected trigger {} outside the remaining order",
                            self.seed,
                            selected.index()
                        )
                    })?;
                remaining.remove(index);
                controller_prefix.push(selected);
                order.push(selected);
            }
            order.extend(remaining);
        }
        self.dispatch(Action::PutPendingTriggeredAbilitiesOnStackInOrder { order })
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
        let activated_ability = record.activated_ability();
        let outcome = record.outcome();
        let targets = record
            .targets()
            .iter()
            .map(|target| target.choice())
            .collect::<Vec<_>>();
        let decisions = record.decisions();
        if outcome != ResolutionOutcome::Resolved {
            return Ok(());
        }
        if let Some(trigger) = trigger {
            return self.execute_trigger(controller, trigger);
        }
        if let Some(ability) = activated_ability {
            let Some(runtime) = self
                .activated_abilities
                .iter()
                .find(|registered| registered.id == ability)
                .and_then(|registered| registered.runtime.clone())
            else {
                return Ok(());
            };
            let effect = runtime
                .program
                .activated_effects()
                .get(runtime.ability_index)
                .ok_or_else(|| {
                    format!(
                        "seed {} missing activated runtime {} on {}",
                        self.seed,
                        runtime.ability_index,
                        runtime.program.name()
                    )
                })?;
            let requires_object_choices = !effect.object_choice_requirements().is_empty();
            let pending = PendingActivatedResolution {
                controller,
                runtime,
                targets,
                decisions,
            };
            if requires_object_choices {
                if self.pending_activated_resolution.is_some()
                    || self.pending_triggered_resolution.is_some()
                {
                    return Err(format!(
                        "seed {} attempted to overlap deferred resolution choices",
                        self.seed
                    ));
                }
                self.pending_activated_resolution = Some(pending);
                return Ok(());
            }
            let actions = self.pending_activated_actions(&pending, Vec::new())?;
            let action_count = actions.len() as u64;
            for action in actions {
                self.dispatch(action)?;
            }
            self.metrics.interpreter_actions = self
                .metrics
                .interpreter_actions
                .saturating_add(action_count);
            if let Some(exercise) = self.identity_exercise_mut(pending.runtime.source) {
                exercise.effect_actions = exercise.effect_actions.saturating_add(action_count);
            }
            return Ok(());
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
        if !program.effects().is_empty() || !program.spell_modes().is_empty() {
            let mut bindings = ExecutionBindings::new(controller, self.live_opponents(controller))
                .with_source(object)
                .with_targets(targets)
                .with_optional_effect_choices(decisions.optional_choices().collect());
            if let Some(mode) = decisions.mode() {
                bindings = bindings.with_spell_mode(mode as usize);
            }
            let actions =
                bind_program_actions(&self.state, &program, &bindings).map_err(|error| {
                    format!("seed {} interpreter binding failed: {error}", self.seed)
                })?;
            let action_count = actions.len() as u64;
            for action in actions {
                self.dispatch(action.action().clone())?;
            }
            self.metrics.interpreter_actions = self
                .metrics
                .interpreter_actions
                .saturating_add(action_count);
            if let Some(exercise) = self.identity_exercise_mut(object) {
                exercise.effect_actions = exercise.effect_actions.saturating_add(action_count);
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
        ensure_trigger_targets_are_autonomous(ability, runtime.program.name())?;
        let requires_object_choices = !ability.object_choice_requirements().is_empty();
        let pending = PendingTriggeredResolution {
            controller,
            runtime,
        };
        if requires_object_choices {
            if self.pending_activated_resolution.is_some()
                || self.pending_triggered_resolution.is_some()
            {
                return Err(format!(
                    "seed {} attempted to overlap deferred resolution choices",
                    self.seed
                ));
            }
            self.pending_triggered_resolution = Some(pending);
            return Ok(());
        }
        let actions = self.pending_triggered_actions(&pending, Vec::new())?;
        let action_count = actions.len() as u64;
        for action in actions {
            self.dispatch(action)?;
        }
        self.metrics.interpreter_actions = self
            .metrics
            .interpreter_actions
            .saturating_add(action_count);
        self.metrics.triggers_resolved = self.metrics.triggers_resolved.saturating_add(1);
        if let Some(exercise) = self.identity_exercise_mut(pending.runtime.source) {
            exercise.trigger_resolutions = exercise.trigger_resolutions.saturating_add(1);
        }
        Ok(())
    }

    fn attack_assignment_objects(&self, active: PlayerId) -> Result<Vec<ObjectId>, String> {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let defenders = self.live_opponents(active);
        let mut objects = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(active))
            .filter(|object| {
                defenders.iter().copied().any(|defender| {
                    self.state
                        .can_attack(active, AttackDeclaration::new(*object, defender))
                })
            })
            .collect::<Vec<_>>();
        objects.sort_by_key(|object| object.index());
        Ok(objects)
    }

    fn attack_assignment_context(
        &self,
        active: PlayerId,
        objects: &[ObjectId],
        cursor: usize,
        declarations: &[AttackDeclaration],
    ) -> Result<DecisionContext, String> {
        let attacker = objects.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} attack assignment cursor is out of range",
                self.seed
            )
        })?;
        let mut options = vec![DecisionOption::new(
            DecisionDescriptor::AssignAttacker {
                attacker,
                defender: None,
            },
            Vec::new(),
        )];
        options.extend(
            self.live_opponents(active)
                .into_iter()
                .filter(|defender| {
                    self.state
                        .can_attack(active, AttackDeclaration::new(attacker, *defender))
                })
                .map(|defender| {
                    DecisionOption::new(
                        DecisionDescriptor::AssignAttacker {
                            attacker,
                            defender: Some(defender),
                        },
                        Vec::new(),
                    )
                }),
        );
        self.scoped_decision_context(
            DecisionKind::DeclareAttackers,
            active,
            options,
            attack_path_discriminator(active, cursor, declarations),
        )
    }

    fn block_assignment_objects(
        &self,
        defending_player: PlayerId,
    ) -> Result<Vec<ObjectId>, String> {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let mut objects = self
            .state
            .zone_objects(battlefield)
            .ok_or_else(|| format!("seed {} missing battlefield", self.seed))?
            .iter()
            .copied()
            .filter(|object| self.state.object_controller(*object) == Ok(defending_player))
            .filter(|object| {
                self.current_attacks
                    .iter()
                    .filter(|attack| attack.defending_player() == defending_player)
                    .any(|attack| {
                        self.state.can_block(
                            defending_player,
                            BlockDeclaration::new(*object, attack.attacker()),
                        )
                    })
            })
            .collect::<Vec<_>>();
        objects.sort_by_key(|object| object.index());
        Ok(objects)
    }

    fn block_assignment_context(
        &self,
        defending_player: PlayerId,
        objects: &[ObjectId],
        cursor: usize,
        declarations: &[BlockDeclaration],
    ) -> Result<DecisionContext, String> {
        let blocker = objects
            .get(cursor)
            .copied()
            .ok_or_else(|| format!("seed {} block assignment cursor is out of range", self.seed))?;
        let remaining = objects.get(cursor + 1..).unwrap_or_default();
        let candidates = std::iter::once(None).chain(
            self.current_attacks
                .iter()
                .filter(|attack| attack.defending_player() == defending_player)
                .map(|attack| attack.attacker())
                .filter(|attacker| {
                    self.state
                        .can_block(defending_player, BlockDeclaration::new(blocker, *attacker))
                })
                .map(Some),
        );
        let options = candidates
            .filter(|attacker| {
                let mut next = declarations.to_vec();
                if let Some(attacker) = attacker {
                    next.push(BlockDeclaration::new(blocker, *attacker));
                }
                self.block_prefix_has_completion(defending_player, &next, remaining)
            })
            .map(|attacker| {
                DecisionOption::new(
                    DecisionDescriptor::AssignBlocker { blocker, attacker },
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        self.scoped_decision_context(
            DecisionKind::DeclareBlockers,
            defending_player,
            options,
            block_path_discriminator(defending_player, cursor, declarations),
        )
    }

    fn block_prefix_has_completion(
        &self,
        defending_player: PlayerId,
        declarations: &[BlockDeclaration],
        remaining_blockers: &[ObjectId],
    ) -> bool {
        fn match_needs(
            driver: &GameDriver,
            defending_player: PlayerId,
            needs: &[ObjectId],
            remaining_blockers: &[ObjectId],
            need_index: usize,
            used: &mut [bool],
        ) -> bool {
            if need_index == needs.len() {
                return true;
            }
            remaining_blockers
                .iter()
                .copied()
                .enumerate()
                .any(|(index, blocker)| {
                    if used[index]
                        || !driver.state.can_block(
                            defending_player,
                            BlockDeclaration::new(blocker, needs[need_index]),
                        )
                    {
                        return false;
                    }
                    used[index] = true;
                    let matched = match_needs(
                        driver,
                        defending_player,
                        needs,
                        remaining_blockers,
                        need_index + 1,
                        used,
                    );
                    used[index] = false;
                    matched
                })
        }

        let needs = self
            .current_attacks
            .iter()
            .filter(|attack| attack.defending_player() == defending_player)
            .filter(|attack| {
                self.state
                    .creature_characteristics(attack.attacker())
                    .is_ok_and(|creature| creature.keywords().menace())
            })
            .filter(|attack| {
                declarations
                    .iter()
                    .filter(|block| block.attacker() == attack.attacker())
                    .count()
                    == 1
            })
            .map(|attack| attack.attacker())
            .collect::<Vec<_>>();
        let mut used = vec![false; remaining_blockers.len()];
        match_needs(
            self,
            defending_player,
            &needs,
            remaining_blockers,
            0,
            &mut used,
        )
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
            DecisionDescriptor::AssignAttacker { attacker, defender } => {
                let attacks = (*defender)
                    .map(|defender| vec![AttackDeclaration::new(*attacker, defender)])
                    .unwrap_or_default();
                return self.combat_action_prior(
                    &DecisionDescriptor::DeclareAttackers { attacks },
                    weights,
                    profile,
                );
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
            DecisionDescriptor::AssignBlocker { blocker, attacker } => {
                let blocks = (*attacker)
                    .map(|attacker| vec![BlockDeclaration::new(*blocker, attacker)])
                    .unwrap_or_default();
                return self.combat_action_prior(
                    &DecisionDescriptor::DeclareBlockers { blocks },
                    weights,
                    profile,
                );
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
        let objects = Arc::new(self.attack_assignment_objects(active)?);
        let mut attacks = Vec::new();
        for (cursor, attacker) in objects.iter().copied().enumerate() {
            let decision_started = Instant::now();
            let resource_started =
                matches!(policy, AiController::Search(_)).then(ResourceSnapshot::capture);
            let context = self.attack_assignment_context(active, &objects, cursor, &attacks)?;
            let (selected_id, decision, policy_name, candidates, search_report) = match policy {
                AiController::Search(controller) => {
                    let decision_index = self.ai_decisions.len() as u64;
                    let domain = CombatSearchDomain {
                        root: self,
                        actor: active,
                        weights: controller.weights,
                        progress: CombatSearchProgress::Attackers {
                            active,
                            objects: Arc::clone(&objects),
                            cursor,
                            declarations: attacks.clone(),
                        },
                        guardrail_profile: controller.guardrail_profile,
                    };
                    let mut config = controller
                        .config(decision_index)
                        .with_decision_started(decision_started);
                    if let Some(resource_started) = resource_started {
                        config = config.with_resource_started(resource_started);
                    }
                    let report =
                        SearchEngine::search(&domain, &context, &config).map_err(|error| {
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
                    "seed {} AI selected illegal attacker assignment: {error}",
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
            let DecisionDescriptor::AssignAttacker {
                attacker: selected_attacker,
                defender,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} attack subcontext returned a non-assignment descriptor",
                    self.seed
                ));
            };
            if *selected_attacker != attacker {
                return Err(format!(
                    "seed {} attack subcontext assigned the wrong object",
                    self.seed
                ));
            }
            if let Some(defender) = defender {
                attacks.push(AttackDeclaration::new(attacker, *defender));
            }
        }
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
        self.current_attacks = attacks;
        self.metrics.combat_declarations = self.metrics.combat_declarations.saturating_add(1);
        Ok(())
    }

    fn declare_human_attackers(
        &mut self,
        active: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        if source.is_legacy_replay() {
            return self.declare_legacy_human_attackers(active, source);
        }
        let objects = self.attack_assignment_objects(active)?;
        let mut attacks = Vec::new();
        for (cursor, attacker) in objects.iter().copied().enumerate() {
            let context = self.attack_assignment_context(active, &objects, cursor, &attacks)?;
            let labels = context
                .options()
                .iter()
                .map(|option| self.attack_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id =
                self.prompt_context_choice(source, "Assign attacker", &context, &labels)?;
            let selected = context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} human selected illegal attacker assignment: {error}",
                    self.seed
                )
            })?;
            let DecisionDescriptor::AssignAttacker {
                attacker: selected_attacker,
                defender,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} attack subcontext returned a non-assignment descriptor",
                    self.seed
                ));
            };
            if *selected_attacker != attacker {
                return Err(format!(
                    "seed {} attack subcontext assigned the wrong object",
                    self.seed
                ));
            }
            if let Some(defender) = defender {
                attacks.push(AttackDeclaration::new(attacker, *defender));
            }
        }
        self.dispatch(Action::DeclareAttackers {
            player: active,
            attacks: attacks.clone(),
        })?;
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
        if let DecisionDescriptor::AssignAttacker { attacker, defender } = descriptor {
            return Ok(defender.map_or_else(
                || format!("Do not attack with {}", self.object_name(*attacker)),
                |defender| {
                    format!(
                        "Attack seat {} with {}",
                        defender.index() + 1,
                        self.object_name(*attacker)
                    )
                },
            ));
        }
        if let DecisionDescriptor::DeclareAttackers { attacks } = descriptor {
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
            return Ok(format!("Attack with {declarations}"));
        }
        Err(format!(
            "seed {} attack prompt cannot label descriptor {descriptor:?}",
            self.seed
        ))
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
        let objects = Arc::new(self.block_assignment_objects(defending_player)?);
        let mut blocks = Vec::new();
        for (cursor, blocker) in objects.iter().copied().enumerate() {
            let decision_started = Instant::now();
            let resource_started =
                matches!(policy, AiController::Search(_)).then(ResourceSnapshot::capture);
            let context =
                self.block_assignment_context(defending_player, &objects, cursor, &blocks)?;
            let (selected_id, decision, policy_name, candidates, search_report) = match policy {
                AiController::Search(controller) => {
                    let decision_index = self.ai_decisions.len() as u64;
                    let domain = CombatSearchDomain {
                        root: self,
                        actor: defending_player,
                        weights: controller.weights,
                        progress: CombatSearchProgress::Blockers {
                            defending: defending_player,
                            objects: Arc::clone(&objects),
                            cursor,
                            declarations: blocks.clone(),
                        },
                        guardrail_profile: controller.guardrail_profile,
                    };
                    let mut config = controller
                        .config(decision_index)
                        .with_decision_started(decision_started);
                    if let Some(resource_started) = resource_started {
                        config = config.with_resource_started(resource_started);
                    }
                    let report =
                        SearchEngine::search(&domain, &context, &config).map_err(|error| {
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
                    "seed {} AI selected illegal blocker assignment: {error}",
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
            let DecisionDescriptor::AssignBlocker {
                blocker: selected_blocker,
                attacker,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} block subcontext returned a non-assignment descriptor",
                    self.seed
                ));
            };
            if *selected_blocker != blocker {
                return Err(format!(
                    "seed {} block subcontext assigned the wrong object",
                    self.seed
                ));
            }
            if let Some(attacker) = attacker {
                blocks.push(BlockDeclaration::new(blocker, *attacker));
            }
        }
        self.dispatch(Action::DeclareBlockers {
            defending_player,
            blocks,
        })?;
        Ok(())
    }

    fn declare_human_blocks(
        &mut self,
        defending_player: PlayerId,
        source: &mut dyn DecisionSource,
    ) -> Result<(), String> {
        if source.is_legacy_replay() {
            return self.declare_legacy_human_blocks(defending_player, source);
        }
        let objects = self.block_assignment_objects(defending_player)?;
        let mut blocks = Vec::new();
        for (cursor, blocker) in objects.iter().copied().enumerate() {
            let context =
                self.block_assignment_context(defending_player, &objects, cursor, &blocks)?;
            let labels = context
                .options()
                .iter()
                .map(|option| self.block_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id =
                self.prompt_context_choice(source, "Assign blocker", &context, &labels)?;
            let selected = context.select(selected_id).map_err(|error| {
                format!(
                    "seed {} human selected illegal blocker assignment: {error}",
                    self.seed
                )
            })?;
            let DecisionDescriptor::AssignBlocker {
                blocker: selected_blocker,
                attacker,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} block subcontext returned a non-assignment descriptor",
                    self.seed
                ));
            };
            if *selected_blocker != blocker {
                return Err(format!(
                    "seed {} block subcontext assigned the wrong object",
                    self.seed
                ));
            }
            if let Some(attacker) = attacker {
                blocks.push(BlockDeclaration::new(blocker, *attacker));
            }
        }
        self.dispatch(Action::DeclareBlockers {
            defending_player,
            blocks,
        })?;
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
        if let DecisionDescriptor::AssignBlocker { blocker, attacker } = descriptor {
            return Ok(attacker.map_or_else(
                || format!("Do not block with {}", self.object_name(*blocker)),
                |attacker| {
                    format!(
                        "Block {} with {}",
                        self.object_name(attacker),
                        self.object_name(*blocker)
                    )
                },
            ));
        }
        if let DecisionDescriptor::DeclareBlockers { blocks } = descriptor {
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
            return Ok(format!("Block with {declarations}"));
        }
        Err(format!(
            "seed {} block prompt cannot label descriptor {descriptor:?}",
            self.seed
        ))
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

    fn combat_damage_order_context(
        &self,
        controller: PlayerId,
        source: ObjectId,
        remaining: &[CombatDamageTarget],
        prefix: &[CombatDamageTarget],
    ) -> Result<DecisionContext, String> {
        let options = remaining
            .iter()
            .copied()
            .map(|target| {
                let mut targets = prefix
                    .iter()
                    .copied()
                    .map(combat_damage_target_choice)
                    .collect::<Vec<_>>();
                targets.push(combat_damage_target_choice(target));
                DecisionOption::new(
                    DecisionDescriptor::OrderCombatDamage { source, targets },
                    Vec::new(),
                )
            })
            .collect();
        self.scoped_decision_context(
            DecisionKind::CombatDamage,
            controller,
            options,
            combat_damage_order_path_discriminator(controller, source, remaining, prefix),
        )
    }

    fn combat_damage_amount_context(
        &self,
        controller: PlayerId,
        source: ObjectId,
        targets: &[CombatDamageTarget],
        assignments: &[CombatDamageAssignment],
        cursor: usize,
        bounds: (u32, u32),
    ) -> Result<DecisionContext, String> {
        let target = targets.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} combat-damage amount cursor {cursor} is out of bounds",
                self.seed
            )
        })?;
        let options = (bounds.0..=bounds.1)
            .map(|amount| {
                DecisionOption::new(
                    DecisionDescriptor::AssignCombatDamage {
                        source,
                        target: combat_damage_target_choice(target),
                        amount,
                    },
                    Vec::new(),
                )
            })
            .collect();
        self.scoped_decision_context(
            DecisionKind::CombatDamage,
            controller,
            options,
            combat_damage_amount_path_discriminator(
                controller,
                source,
                targets,
                assignments,
                cursor,
                bounds,
            ),
        )
    }

    fn combat_damage_amount_range_context(
        &self,
        controller: PlayerId,
        source: ObjectId,
        targets: &[CombatDamageTarget],
        assignments: &[CombatDamageAssignment],
        cursor: usize,
        bounds: (u32, u32),
    ) -> Result<DecisionContext, String> {
        if bounds.0 >= bounds.1 {
            return Err(format!(
                "seed {} combat-damage range is not divisible: {bounds:?}",
                self.seed
            ));
        }
        let target = targets.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} combat-damage range cursor {cursor} is out of bounds",
                self.seed
            )
        })?;
        let midpoint = bounds.0 + (bounds.1 - bounds.0) / 2;
        let options = [(bounds.0, midpoint), (midpoint + 1, bounds.1)]
            .into_iter()
            .map(|(minimum, maximum)| {
                DecisionOption::new(
                    DecisionDescriptor::ChooseCombatDamageRange {
                        source,
                        target: combat_damage_target_choice(target),
                        minimum,
                        maximum,
                    },
                    Vec::new(),
                )
            })
            .collect();
        self.scoped_decision_context(
            DecisionKind::CombatDamage,
            controller,
            options,
            combat_damage_amount_path_discriminator(
                controller,
                source,
                targets,
                assignments,
                cursor,
                bounds,
            ),
        )
    }

    fn combat_damage_target_label(&self, target: CombatDamageTarget) -> String {
        match target {
            CombatDamageTarget::Object(object) => self.object_name(object),
            CombatDamageTarget::Player(player) => {
                format!("player {}", player.index().saturating_add(1))
            }
        }
    }

    fn select_combat_damage_order_choice(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<CombatDamageTarget, String> {
        let selected_id = if human == Some(controller) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::OrderCombatDamage { source, targets } => targets
                        .last()
                        .copied()
                        .and_then(combat_damage_choice_target)
                        .map(|target| {
                            format!(
                                "Assign damage from {} to {} in position {}",
                                self.object_name(*source),
                                self.combat_damage_target_label(target),
                                targets.len()
                            )
                        })
                        .ok_or_else(|| {
                            format!("seed {} combat-damage order option is empty", self.seed)
                        }),
                    descriptor => Err(format!(
                        "seed {} cannot label combat-damage order descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for combat-damage ordering".to_owned()
            })?;
            self.prompt_context_choice(source, "Order combat-damage targets", context, &labels)?
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(controller, policies)?;
            let candidates = if matches!(policy, AiController::Random(_)) {
                Vec::new()
            } else {
                self.policy_candidates(context, controller, |_| 0)?
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, context, &candidates, "combat_damage_order")?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "combat_damage_order",
                policy: policy_name,
                context,
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
            selected_id
        } else {
            return Err(format!(
                "seed {} combat-damage order prompt has no controller",
                self.seed
            ));
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal combat-damage order action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::OrderCombatDamage { targets, .. } = selected.descriptor() else {
            return Err(format!(
                "seed {} combat-damage order returned a non-order descriptor",
                self.seed
            ));
        };
        targets
            .last()
            .copied()
            .and_then(combat_damage_choice_target)
            .ok_or_else(|| format!("seed {} combat-damage order selection is empty", self.seed))
    }

    fn select_combat_damage_amount(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<u32, String> {
        let selected_id = if human == Some(controller) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::AssignCombatDamage {
                        source,
                        target,
                        amount,
                    } => combat_damage_choice_target(*target)
                        .map(|target| {
                            format!(
                                "Assign {amount} damage from {} to {}",
                                self.object_name(*source),
                                self.combat_damage_target_label(target)
                            )
                        })
                        .ok_or_else(|| {
                            format!(
                                "seed {} combat-damage target cannot be a stack entry",
                                self.seed
                            )
                        }),
                    descriptor => Err(format!(
                        "seed {} cannot label combat-damage amount descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for combat-damage assignment".to_owned()
            })?;
            self.prompt_context_choice(source, "Assign combat damage", context, &labels)?
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(controller, policies)?;
            let candidates = if matches!(policy, AiController::Random(_)) {
                Vec::new()
            } else {
                self.policy_candidates(context, controller, |option| {
                    if let DecisionDescriptor::AssignCombatDamage { amount, .. } =
                        option.descriptor()
                    {
                        -i64::from(*amount)
                    } else {
                        0
                    }
                })?
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, context, &candidates, "combat_damage_amount")?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "combat_damage_amount",
                policy: policy_name,
                context,
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
            selected_id
        } else {
            return Err(format!(
                "seed {} combat-damage amount prompt has no controller",
                self.seed
            ));
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal combat-damage amount action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::AssignCombatDamage { amount, .. } = selected.descriptor() else {
            return Err(format!(
                "seed {} combat-damage amount returned a non-assignment descriptor",
                self.seed
            ));
        };
        Ok(*amount)
    }

    fn select_combat_damage_amount_range(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(u32, u32), String> {
        let selected_id = if human == Some(controller) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::ChooseCombatDamageRange {
                        source,
                        target,
                        minimum,
                        maximum,
                    } => combat_damage_choice_target(*target)
                        .map(|target| {
                            format!(
                                "Assign {minimum}-{maximum} damage from {} to {}",
                                self.object_name(*source),
                                self.combat_damage_target_label(target)
                            )
                        })
                        .ok_or_else(|| {
                            format!(
                                "seed {} combat-damage target cannot be a stack entry",
                                self.seed
                            )
                        }),
                    descriptor => Err(format!(
                        "seed {} cannot label combat-damage range descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for combat-damage assignment".to_owned()
            })?;
            self.prompt_context_choice(source, "Narrow combat damage", context, &labels)?
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(controller, policies)?;
            let candidates = if matches!(policy, AiController::Random(_)) {
                Vec::new()
            } else {
                self.policy_candidates(context, controller, |option| {
                    if let DecisionDescriptor::ChooseCombatDamageRange { minimum, .. } =
                        option.descriptor()
                    {
                        -i64::from(*minimum)
                    } else {
                        0
                    }
                })?
            };
            let (selected_id, decision, policy_name) =
                self.select_ai_action(policy, context, &candidates, "combat_damage_amount")?;
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "combat_damage_amount",
                policy: policy_name,
                context,
                action_id: selected_id,
                decision,
                evaluated_candidates: candidates.len(),
                wall_latency_us: elapsed_us(decision_started),
                score_override: None,
                stop_reason: if decision.is_some() {
                    "range_narrowed"
                } else {
                    "random_range_selection"
                },
            });
            selected_id
        } else {
            return Err(format!(
                "seed {} combat-damage range prompt has no controller",
                self.seed
            ));
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal combat-damage range action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::ChooseCombatDamageRange {
            minimum, maximum, ..
        } = selected.descriptor()
        else {
            return Err(format!(
                "seed {} combat-damage range returned a non-range descriptor",
                self.seed
            ));
        };
        Ok((*minimum, *maximum))
    }

    fn assign_combat_damage(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(), String> {
        let sources = self
            .state
            .eligible_combat_damage_sources()
            .map_err(|error| {
                format!(
                    "seed {} combat-damage source discovery failed: {error:?}",
                    self.seed
                )
            })?;
        let mut assignments = Vec::new();
        let legacy_human_replay = decisions
            .as_ref()
            .is_some_and(|source| source.is_legacy_replay());
        for source in sources {
            let canonical = self
                .state
                .combat_damage_choice_profile(source)
                .map_err(|error| {
                    format!(
                        "seed {} combat-damage profile failed for {}: {error:?}",
                        self.seed,
                        source.index()
                    )
                })?;
            if canonical.required_total() == 0 {
                continue;
            }
            let controller = self.state.object_controller(source).map_err(|error| {
                format!(
                    "seed {} combat-damage controller failed for {}: {error:?}",
                    self.seed,
                    source.index()
                )
            })?;
            let controlled =
                (human == Some(controller) && !legacy_human_replay) || ai_policies.is_some();
            let mut remaining = canonical
                .targets()
                .iter()
                .copied()
                .filter(|target| matches!(target, CombatDamageTarget::Object(_)))
                .collect::<Vec<_>>();
            let mut ordered = Vec::with_capacity(canonical.targets().len());
            if controlled {
                while remaining.len() > 1 {
                    let context =
                        self.combat_damage_order_context(controller, source, &remaining, &ordered)?;
                    let selected = self.select_combat_damage_order_choice(
                        &context,
                        controller,
                        human,
                        decisions,
                        ai_policies,
                    )?;
                    let index = remaining
                        .iter()
                        .position(|target| *target == selected)
                        .ok_or_else(|| {
                            format!(
                                "seed {} selected combat-damage target outside the remaining order",
                                self.seed
                            )
                        })?;
                    ordered.push(remaining.remove(index));
                }
            }
            ordered.extend(remaining);
            ordered.extend(
                canonical
                    .targets()
                    .iter()
                    .copied()
                    .filter(|target| matches!(target, CombatDamageTarget::Player(_))),
            );
            if ordered.is_empty() {
                return Err(format!(
                    "seed {} damage source {} has positive power but no legal target",
                    self.seed,
                    source.index()
                ));
            }
            let profile = self
                .state
                .combat_damage_choice_profile_for_order(source, &ordered)
                .map_err(|error| {
                    format!(
                        "seed {} selected combat-damage order failed validation: {error:?}",
                        self.seed
                    )
                })?;
            let mut remaining_damage = profile.required_total();
            let mut source_assignments = Vec::with_capacity(ordered.len());
            for (cursor, target) in ordered.iter().copied().enumerate() {
                let amount = if cursor + 1 == ordered.len() {
                    remaining_damage
                } else if !controlled {
                    if cursor == 0 {
                        remaining_damage
                    } else {
                        0
                    }
                } else {
                    let minimum = profile.minimum_to_advance()[cursor].min(remaining_damage);
                    if minimum == remaining_damage {
                        remaining_damage
                    } else {
                        let mut bounds = (minimum, remaining_damage);
                        while u64::from(bounds.1) - u64::from(bounds.0) + 1
                            > u64::from(MAX_DIRECT_COMBAT_DAMAGE_AMOUNTS)
                        {
                            let context = self.combat_damage_amount_range_context(
                                controller,
                                source,
                                &ordered,
                                &source_assignments,
                                cursor,
                                bounds,
                            )?;
                            bounds = self.select_combat_damage_amount_range(
                                &context,
                                controller,
                                human,
                                decisions,
                                ai_policies,
                            )?;
                        }
                        let context = self.combat_damage_amount_context(
                            controller,
                            source,
                            &ordered,
                            &source_assignments,
                            cursor,
                            bounds,
                        )?;
                        self.select_combat_damage_amount(
                            &context,
                            controller,
                            human,
                            decisions,
                            ai_policies,
                        )?
                    }
                };
                remaining_damage = remaining_damage.checked_sub(amount).ok_or_else(|| {
                    format!(
                        "seed {} selected {amount} damage with only {remaining_damage} remaining",
                        self.seed
                    )
                })?;
                if controlled || amount > 0 {
                    source_assignments.push(CombatDamageAssignment::new(target, amount));
                }
            }
            assignments.push(CombatDamageAssignmentRequest::new(
                source,
                source_assignments,
            ));
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
        self.state_with_context(driver, context, mappings)
    }

    fn state_with_context(
        &self,
        driver: GameDriver,
        context: DecisionContext,
        mappings: Vec<(CanonicalActionId, MainChoice)>,
    ) -> Result<MainSearchState, String> {
        let priors = context
            .options()
            .iter()
            .map(|option| {
                (
                    option.id(),
                    match option.descriptor() {
                        DecisionDescriptor::ChooseNumber { value } => i64::from(*value),
                        DecisionDescriptor::ChooseNumberRange { maximum, .. } => {
                            i64::from(*maximum)
                        }
                        DecisionDescriptor::ChoosePayment { payment } => {
                            -i64::from(payment.waste_score())
                        }
                        descriptor => {
                            driver.main_action_prior(&context, descriptor, self.guardrail_profile)
                        }
                    },
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
            .find_map(|(id, choice)| (*id == action).then(|| choice.clone()))
            .ok_or_else(|| format!("search action {action} has no typed main-phase adapter"))?;
        let mut next = state.clone();
        if let Some((context, mappings)) =
            next.driver.hierarchical_cast_context(self.actor, &choice)?
        {
            return self.state_with_context(next.driver, context, mappings);
        }
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

    fn action_group(&self, state: &Self::State, action: CanonicalActionId) -> u64 {
        state.context.select(action).map_or_else(
            |_| {
                let value = action.get();
                value as u64 ^ (value >> 64) as u64
            },
            DecisionOption::widening_group,
        )
    }

    fn state_key(&self, state: &Self::State) -> Option<SearchStateKey> {
        let context = state.context.id().get();
        let discriminator = (context as u64)
            ^ ((context >> 64) as u64)
            ^ if state.finished {
                0xa076_1d64_78bd_642f
            } else {
                0
            };
        Some(SearchStateKey::new(
            state.driver.state.deterministic_hash().get(),
            discriminator,
        ))
    }

    fn transposition_equivalent(&self, left: &Self::State, right: &Self::State) -> bool {
        left.finished == right.finished
            && left.context.id() == right.context.id()
            && left
                .driver
                .state
                .canonically_equivalent(&right.driver.state)
    }
}

fn bounded_object_combinations(
    candidates: &[ObjectId],
    minimum: usize,
    maximum: usize,
    limit: usize,
) -> Result<Vec<Vec<ObjectId>>, String> {
    fn extend(
        candidates: &[ObjectId],
        start: usize,
        remaining: usize,
        limit: usize,
        current: &mut Vec<ObjectId>,
        output: &mut Vec<Vec<ObjectId>>,
    ) -> Result<(), String> {
        if remaining == 0 {
            if output.len() >= limit {
                return Err(format!(
                    "object-choice combinations exceed the {limit}-option canonical cap"
                ));
            }
            output.push(current.clone());
            return Ok(());
        }
        if candidates.len().saturating_sub(start) < remaining {
            return Ok(());
        }
        let final_start = candidates.len() - remaining;
        for index in start..=final_start {
            current.push(candidates[index]);
            extend(candidates, index + 1, remaining - 1, limit, current, output)?;
            current.pop();
        }
        Ok(())
    }

    let maximum = maximum.min(candidates.len());
    if minimum > maximum {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    let mut current = Vec::new();
    for count in minimum..=maximum {
        extend(candidates, 0, count, limit, &mut current, &mut output)?;
    }
    Ok(output)
}

impl CombatSearchProgress {
    fn context(&self, driver: &GameDriver) -> Result<DecisionContext, String> {
        match self {
            Self::Attackers {
                active,
                objects,
                cursor,
                declarations,
            } => driver.attack_assignment_context(*active, objects, *cursor, declarations),
            Self::Blockers {
                defending,
                objects,
                cursor,
                declarations,
            } => driver.block_assignment_context(*defending, objects, *cursor, declarations),
        }
    }

    fn apply_descriptor(&mut self, descriptor: &DecisionDescriptor) -> Result<bool, String> {
        match self {
            Self::Attackers {
                objects,
                cursor,
                declarations,
                ..
            } => {
                let expected = objects
                    .get(*cursor)
                    .copied()
                    .ok_or_else(|| "attack search progress is already complete".to_owned())?;
                let DecisionDescriptor::AssignAttacker { attacker, defender } = descriptor else {
                    return Err("attack search received a non-assignment descriptor".to_owned());
                };
                if *attacker != expected {
                    return Err("attack search assignment object does not match cursor".to_owned());
                }
                if let Some(defender) = defender {
                    declarations.push(AttackDeclaration::new(expected, *defender));
                }
                *cursor += 1;
                Ok(*cursor == objects.len())
            }
            Self::Blockers {
                objects,
                cursor,
                declarations,
                ..
            } => {
                let expected = objects
                    .get(*cursor)
                    .copied()
                    .ok_or_else(|| "block search progress is already complete".to_owned())?;
                let DecisionDescriptor::AssignBlocker { blocker, attacker } = descriptor else {
                    return Err("block search received a non-assignment descriptor".to_owned());
                };
                if *blocker != expected {
                    return Err("block search assignment object does not match cursor".to_owned());
                }
                if let Some(attacker) = attacker {
                    declarations.push(BlockDeclaration::new(expected, *attacker));
                }
                *cursor += 1;
                Ok(*cursor == objects.len())
            }
        }
    }

    fn commit(&self, driver: &mut GameDriver) -> Result<(), String> {
        match self {
            Self::Attackers {
                active,
                objects,
                cursor,
                declarations,
            } => {
                if *cursor != objects.len() {
                    return Err("attack search committed an incomplete declaration".to_owned());
                }
                driver.dispatch(Action::DeclareAttackers {
                    player: *active,
                    attacks: declarations.clone(),
                })?;
                driver.current_attacks.clone_from(declarations);
            }
            Self::Blockers {
                defending,
                objects,
                cursor,
                declarations,
            } => {
                if *cursor != objects.len() {
                    return Err("block search committed an incomplete declaration".to_owned());
                }
                driver.dispatch(Action::DeclareBlockers {
                    defending_player: *defending,
                    blocks: declarations.clone(),
                })?;
            }
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        match self {
            Self::Attackers {
                active,
                cursor,
                declarations,
                ..
            } => attack_path_discriminator(*active, *cursor, declarations),
            Self::Blockers {
                defending,
                cursor,
                declarations,
                ..
            } => block_path_discriminator(*defending, *cursor, declarations),
        }
    }
}

impl CombatSearchDomain<'_> {
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
        let progress = self.progress.clone();
        let context = progress.context(&driver)?;
        Ok(CombatSearchState {
            driver,
            finished: false,
            terminal_prior: 0,
            progress,
            context: Some(Arc::new(context)),
        })
    }

    fn legal_actions(&self, state: &Self::State) -> Result<Vec<CanonicalActionId>, String> {
        if state.finished {
            return Ok(Vec::new());
        }
        let context = state
            .context
            .as_ref()
            .ok_or_else(|| "unfinished combat search state has no context".to_owned())?;
        Ok(context.options().iter().map(DecisionOption::id).collect())
    }

    fn apply_action(
        &self,
        state: &Self::State,
        action: CanonicalActionId,
    ) -> Result<Self::State, String> {
        let mut next = state.clone();
        let context = next
            .context
            .as_ref()
            .ok_or_else(|| "combat search selected an action after completion".to_owned())?;
        let selected = context
            .select(action)
            .map_err(|error| format!("combat search selected illegal action: {error}"))?;
        let prior = next.driver.combat_action_prior(
            selected.descriptor(),
            self.weights,
            self.guardrail_profile,
        );
        let descriptor = selected.descriptor().clone();
        let complete = next.progress.apply_descriptor(&descriptor)?;
        next.terminal_prior = next.terminal_prior.saturating_add(prior);
        if complete {
            let progress = next.progress.clone();
            progress.commit(&mut next.driver)?;
            next.finished = true;
            next.context = None;
        } else {
            next.context = Some(Arc::new(next.progress.context(&next.driver)?));
        }
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
        state: &Self::State,
        actions: &[CanonicalActionId],
        _seed: u64,
    ) -> Result<CanonicalActionId, String> {
        actions
            .iter()
            .copied()
            .max_by_key(|action| {
                (
                    self.action_prior(state, *action),
                    std::cmp::Reverse(*action),
                )
            })
            .ok_or_else(|| "combat search rollout received no legal actions".to_owned())
    }

    fn action_prior(&self, state: &Self::State, action: CanonicalActionId) -> i64 {
        state
            .context
            .as_ref()
            .and_then(|context| context.select(action).ok())
            .map_or(0, |option| {
                state.driver.combat_action_prior(
                    option.descriptor(),
                    self.weights,
                    self.guardrail_profile,
                )
            })
    }

    fn state_key(&self, state: &Self::State) -> Option<SearchStateKey> {
        let context = state.context.as_ref().map_or(0, |context| {
            let id = context.id().get();
            (id as u64) ^ ((id >> 64) as u64)
        });
        let discriminator = context
            ^ state.progress.fingerprint()
            ^ state.terminal_prior as u64
            ^ if state.finished {
                0xe703_7ed1_a0b4_28db
            } else {
                0
            };
        Some(SearchStateKey::new(
            state.driver.state.deterministic_hash().get(),
            discriminator,
        ))
    }

    fn transposition_equivalent(&self, left: &Self::State, right: &Self::State) -> bool {
        left.finished == right.finished
            && left.terminal_prior == right.terminal_prior
            && left.progress == right.progress
            && left.context.as_ref().map(|context| context.id())
                == right.context.as_ref().map(|context| context.id())
            && left
                .driver
                .state
                .canonically_equivalent(&right.driver.state)
    }
}

fn ensure_trigger_targets_are_autonomous(
    ability: &TriggeredAbilityProgram,
    card_name: &str,
) -> Result<(), String> {
    if !ability.target_requirements().is_empty() {
        return Err(format!(
            "trigger on {card_name} requires target prompts not supplied by this canonical controller"
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

fn combat_path_mix(state: u64, value: u64) -> u64 {
    (state ^ value)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .rotate_left(27)
}

fn attack_path_discriminator(
    active: PlayerId,
    cursor: usize,
    declarations: &[AttackDeclaration],
) -> u64 {
    let mut state = combat_path_mix(0x6174_7461_636b_0001, active.index() as u64);
    state = combat_path_mix(state, cursor as u64);
    for attack in declarations {
        state = combat_path_mix(state, attack.attacker().index() as u64);
        state = combat_path_mix(state, attack.defending_player().index() as u64);
    }
    state
}

fn block_path_discriminator(
    defending: PlayerId,
    cursor: usize,
    declarations: &[BlockDeclaration],
) -> u64 {
    let mut state = combat_path_mix(0x626c_6f63_6b00_0001, defending.index() as u64);
    state = combat_path_mix(state, cursor as u64);
    for block in declarations {
        state = combat_path_mix(state, block.blocker().index() as u64);
        state = combat_path_mix(state, block.attacker().index() as u64);
    }
    state
}

fn resolution_choice_path_discriminator(
    controller: PlayerId,
    cursor: usize,
    choices: &[Vec<ObjectId>],
) -> u64 {
    let mut state = combat_path_mix(0x7265_736f_6c76_0001, controller.index() as u64);
    state = combat_path_mix(state, cursor as u64);
    for choice in choices {
        state = combat_path_mix(state, choice.len() as u64);
        for object in choice {
            state = combat_path_mix(state, object.index() as u64);
        }
    }
    state
}

#[allow(clippy::too_many_arguments)]
fn variable_cast_path_discriminator(
    player: PlayerId,
    object: ObjectId,
    targets: &[TargetChoice],
    mode: Option<u32>,
    optional: &[bool],
    stage: u64,
    minimum: u32,
    maximum: u32,
) -> u64 {
    let mut state = combat_path_mix(0x7661_7263_6173_0001, player.index() as u64);
    state = combat_path_mix(state, object.index() as u64);
    state = combat_path_mix(state, stage);
    state = combat_path_mix(state, u64::from(minimum));
    state = combat_path_mix(state, u64::from(maximum));
    state = combat_path_mix(state, mode.map_or(u64::MAX, u64::from));
    for target in targets {
        state = match target {
            TargetChoice::Player(player) => {
                combat_path_mix(combat_path_mix(state, 0), player.index() as u64)
            }
            TargetChoice::Object(object) => {
                combat_path_mix(combat_path_mix(state, 1), object.index() as u64)
            }
            TargetChoice::StackEntry(entry) => {
                combat_path_mix(combat_path_mix(state, 2), entry.index() as u64)
            }
        };
    }
    for accept in optional {
        state = combat_path_mix(state, u64::from(*accept));
    }
    state
}

fn trigger_order_path_discriminator(
    controller: PlayerId,
    remaining: &[TriggerId],
    global_prefix: &[TriggerId],
) -> u64 {
    let mut state = combat_path_mix(0x7472_6967_6765_0001, controller.index() as u64);
    state = combat_path_mix(state, global_prefix.len() as u64);
    for trigger in global_prefix {
        state = combat_path_mix(state, trigger.index() as u64);
    }
    state = combat_path_mix(state, remaining.len() as u64);
    for trigger in remaining {
        state = combat_path_mix(state, trigger.index() as u64);
    }
    state
}

const fn combat_damage_target_choice(target: CombatDamageTarget) -> TargetChoice {
    match target {
        CombatDamageTarget::Object(object) => TargetChoice::Object(object),
        CombatDamageTarget::Player(player) => TargetChoice::Player(player),
    }
}

const fn combat_damage_choice_target(target: TargetChoice) -> Option<CombatDamageTarget> {
    match target {
        TargetChoice::Object(object) => Some(CombatDamageTarget::Object(object)),
        TargetChoice::Player(player) => Some(CombatDamageTarget::Player(player)),
        TargetChoice::StackEntry(_) => None,
    }
}

fn mix_combat_damage_target(state: u64, target: CombatDamageTarget) -> u64 {
    match target {
        CombatDamageTarget::Object(object) => {
            combat_path_mix(combat_path_mix(state, 0), object.index() as u64)
        }
        CombatDamageTarget::Player(player) => {
            combat_path_mix(combat_path_mix(state, 1), player.index() as u64)
        }
    }
}

fn combat_damage_order_path_discriminator(
    controller: PlayerId,
    source: ObjectId,
    remaining: &[CombatDamageTarget],
    prefix: &[CombatDamageTarget],
) -> u64 {
    let mut state = combat_path_mix(0x6461_6d61_6765_0001, controller.index() as u64);
    state = combat_path_mix(state, source.index() as u64);
    state = combat_path_mix(state, prefix.len() as u64);
    for target in prefix {
        state = mix_combat_damage_target(state, *target);
    }
    state = combat_path_mix(state, remaining.len() as u64);
    for target in remaining {
        state = mix_combat_damage_target(state, *target);
    }
    state
}

fn combat_damage_amount_path_discriminator(
    controller: PlayerId,
    source: ObjectId,
    targets: &[CombatDamageTarget],
    assignments: &[CombatDamageAssignment],
    cursor: usize,
    bounds: (u32, u32),
) -> u64 {
    let mut state = combat_path_mix(0x6461_6d61_6765_0002, controller.index() as u64);
    state = combat_path_mix(state, source.index() as u64);
    state = combat_path_mix(state, cursor as u64);
    state = combat_path_mix(state, u64::from(bounds.0));
    state = combat_path_mix(state, u64::from(bounds.1));
    for target in targets {
        state = mix_combat_damage_target(state, *target);
    }
    for assignment in assignments {
        state = mix_combat_damage_target(state, assignment.target());
        state = combat_path_mix(state, u64::from(assignment.amount()));
    }
    state
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
        DecisionDescriptor::ActivateProgramAbility {
            source,
            ability,
            payment,
            targets,
            optional,
        } => json!({
            "kind": "activate_program_ability",
            "source_object_id": source.index(),
            "ability_id": ability.get(),
            "payment": payment_value(*payment),
            "targets": targets.iter().copied().map(target_value).collect::<Vec<_>>(),
            "optional": optional
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
        DecisionDescriptor::BeginCastSpell {
            object,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "begin_cast_spell",
            "object_id": object.index(),
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
        DecisionDescriptor::AssignAttacker { attacker, defender } => json!({
            "kind": "assign_attacker",
            "attacker_object_id": attacker.index(),
            "defending_seat": defender.map(PlayerId::index)
        }),
        DecisionDescriptor::DeclareBlockers { blocks } => json!({
            "kind": "declare_blockers",
            "blocks": blocks.iter().map(|block| json!({
                "blocker_object_id": block.blocker().index(),
                "attacker_object_id": block.attacker().index()
            })).collect::<Vec<_>>()
        }),
        DecisionDescriptor::AssignBlocker { blocker, attacker } => json!({
            "kind": "assign_blocker",
            "blocker_object_id": blocker.index(),
            "attacker_object_id": attacker.map(ObjectId::index)
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
        DecisionDescriptor::ChooseNumberRange { minimum, maximum } => json!({
            "kind": "choose_number_range",
            "minimum": minimum,
            "maximum": maximum
        }),
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
        DecisionDescriptor::ChooseResolutionObjects { choices } => json!({
            "kind": "choose_resolution_objects",
            "choice_object_ids": choices.iter().map(|choice| {
                choice.iter().map(|object| object.index()).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        }),
        DecisionDescriptor::OrderCombatDamage { source, targets } => json!({
            "kind": "order_combat_damage",
            "source_object_id": source.index(),
            "targets": targets.iter().copied().map(target_value).collect::<Vec<_>>()
        }),
        DecisionDescriptor::ChooseCombatDamageRange {
            source,
            target,
            minimum,
            maximum,
        } => json!({
            "kind": "choose_combat_damage_range",
            "source_object_id": source.index(),
            "target": target_value(*target),
            "minimum": minimum,
            "maximum": maximum
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
        campaign_seed, concession_decision_context, concession_decision_context_from_view,
        legacy_human_summary_matches, player_view_fingerprint, replay_captured_actions,
        replay_human_file, run_prompted_game, snapshot_prompt, ActivatedRuntime, AiController,
        DecisionPrompt, DecisionSelection, DecisionSource, GameDriver, GameMetrics, GameSummary,
        HeuristicPolicy, IdentityExercise, MainChoice, MainSearchDomain,
        PendingActivatedResolution, RandomLegalPolicy, ReplayDecisionSource,
        TerminalDecisionSource, TraceMode, TraceRecord, CONCESSION_PROMPT, PLAYER_COUNT,
    };
    use forge_ai::{AiWeights, GuardrailProfile, GuardrailTable, SearchDomain};
    use forge_cards::runtime::compile_card_program;
    use forge_core::{
        apply, Action, AttackDeclaration, BaseCreatureCharacteristics, BaseObjectCharacteristics,
        BasicLandTypes, BlockDeclaration, CardId, CombatDamageTarget, CreatureKeywords,
        DecisionContext, DecisionDescriptor, DecisionKind, DecisionOption, GameState, ManaPool,
        ObjectColors, ObjectId, ObjectSupertypes, ObjectTypes, Outcome, PaymentPlan, PlayerId,
        PlayerView, ResolutionOutcome, StackDecisionBindings, Step, TargetChoice, TriggerCondition,
        TriggerDefinition, TriggerPlayerFilter, ZoneId, ZoneKind,
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
            allow_concession: false,
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
        assert_eq!(replay.choose(&prompt), Ok(DecisionSelection::Option(1)));
        assert!(replay.finish().is_ok());

        let changed_options = vec!["Different action".to_owned(), "Pass".to_owned()];
        let changed_prompt = DecisionPrompt {
            kind: "Choose",
            view: &view,
            context: &context,
            options: &changed_options,
            allow_concession: false,
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
            allow_concession: false,
        };
        let mut legacy = snapshot_prompt(0, &prompt, 0);
        legacy.decision_context_schema = 0;
        legacy.context_id.clear();
        legacy.decision_state_key.clear();
        legacy.player_view_hash.clear();
        legacy.canonical_legal_actions.clear();
        legacy.selected_action_id.clear();

        let mut replay = ReplayDecisionSource::new(vec![legacy]);
        assert_eq!(replay.choose(&prompt), Ok(DecisionSelection::Option(0)));
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
    fn attack_subcontexts_expose_split_defenders_without_a_cartesian_product() {
        let (driver, active, defenders, pieces) = combat_decision_driver();
        let objects = driver
            .attack_assignment_objects(active)
            .unwrap_or_else(|error| panic!("attack assignments should exist: {error}"));
        assert_eq!(objects, pieces[..2]);

        let first = driver
            .attack_assignment_context(active, &objects, 0, &[])
            .unwrap_or_else(|error| panic!("first attack subcontext should exist: {error}"));
        assert_eq!(first.options().len(), defenders.len() + 1);
        assert!(first.options().iter().any(|option| {
            option.descriptor()
                == &DecisionDescriptor::AssignAttacker {
                    attacker: pieces[0],
                    defender: Some(defenders[0]),
                }
        }));

        let partial = vec![AttackDeclaration::new(pieces[0], defenders[0])];
        let second = driver
            .attack_assignment_context(active, &objects, 1, &partial)
            .unwrap_or_else(|error| panic!("second attack subcontext should exist: {error}"));
        assert!(second.options().iter().any(|option| {
            option.descriptor()
                == &DecisionDescriptor::AssignAttacker {
                    attacker: pieces[1],
                    defender: Some(defenders[1]),
                }
        }));
        let alternate = driver
            .attack_assignment_context(
                active,
                &objects,
                1,
                &[AttackDeclaration::new(pieces[0], defenders[2])],
            )
            .unwrap_or_else(|error| panic!("alternate attack path should exist: {error}"));
        assert_ne!(second.state_key(), alternate.state_key());

        let split = vec![
            AttackDeclaration::new(pieces[0], defenders[0]),
            AttackDeclaration::new(pieces[1], defenders[1]),
        ];
        assert!(driver.action_is_legal(&Action::DeclareAttackers {
            player: active,
            attacks: split,
        }));
    }

    #[test]
    fn block_subcontexts_preserve_menace_completion_legality() {
        let (mut driver, active, defenders, pieces) = combat_decision_driver();
        assert_eq!(
            apply(
                &mut driver.state,
                Action::SetBaseCreatureCharacteristics {
                    object: pieces[0],
                    base: BaseCreatureCharacteristics::new(2, 2)
                        .with_keywords(CreatureKeywords::none().with_menace()),
                },
            ),
            Outcome::Applied
        );
        let second_blocker = create_test_creature(&mut driver.state, defenders[0], 804, 2, 2);
        let attacks = vec![AttackDeclaration::new(pieces[0], defenders[0])];
        driver
            .dispatch(Action::DeclareAttackers {
                player: active,
                attacks: attacks.clone(),
            })
            .unwrap_or_else(|error| panic!("menace attack should apply: {error}"));
        driver.current_attacks = attacks;
        assert!(matches!(
            driver.dispatch(Action::AdvanceStep),
            Ok(Outcome::StepAdvanced(Step::DeclareBlockers))
        ));

        let blockers = driver
            .block_assignment_objects(defenders[0])
            .unwrap_or_else(|error| panic!("block assignments should exist: {error}"));
        assert_eq!(blockers, vec![pieces[2], second_blocker]);
        let first = driver
            .block_assignment_context(defenders[0], &blockers, 0, &[])
            .unwrap_or_else(|error| panic!("first block subcontext should exist: {error}"));
        assert!(first.options().iter().any(|option| {
            option.descriptor()
                == &DecisionDescriptor::AssignBlocker {
                    blocker: pieces[2],
                    attacker: Some(pieces[0]),
                }
        }));

        let partial = vec![BlockDeclaration::new(pieces[2], pieces[0])];
        let second = driver
            .block_assignment_context(defenders[0], &blockers, 1, &partial)
            .unwrap_or_else(|error| panic!("second block subcontext should exist: {error}"));
        assert_eq!(second.options().len(), 1);
        assert_eq!(
            second.options()[0].descriptor(),
            &DecisionDescriptor::AssignBlocker {
                blocker: second_blocker,
                attacker: Some(pieces[0]),
            }
        );
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
    fn combat_damage_order_and_amount_use_hierarchical_shared_contexts() {
        let (mut driver, active, defenders, pieces) = combat_decision_driver();
        assert_eq!(
            apply(
                &mut driver.state,
                Action::SetBaseCreatureCharacteristics {
                    object: pieces[0],
                    base: BaseCreatureCharacteristics::new(4, 4),
                },
            ),
            Outcome::Applied
        );
        let second_blocker = create_test_creature(&mut driver.state, defenders[0], 804, 3, 3);
        let attacks = vec![AttackDeclaration::new(pieces[0], defenders[0])];
        driver
            .dispatch(Action::DeclareAttackers {
                player: active,
                attacks: attacks.clone(),
            })
            .unwrap_or_else(|error| panic!("attack should apply: {error}"));
        driver.current_attacks = attacks;
        assert!(matches!(
            driver.dispatch(Action::AdvanceStep),
            Ok(Outcome::StepAdvanced(Step::DeclareBlockers))
        ));
        driver
            .dispatch(Action::DeclareBlockers {
                defending_player: defenders[0],
                blocks: vec![
                    BlockDeclaration::new(pieces[2], pieces[0]),
                    BlockDeclaration::new(second_blocker, pieces[0]),
                ],
            })
            .unwrap_or_else(|error| panic!("double block should apply: {error}"));
        assert!(matches!(
            driver.dispatch(Action::AdvanceStep),
            Ok(Outcome::StepAdvanced(Step::CombatDamage))
        ));

        let targets = vec![
            CombatDamageTarget::Object(pieces[2]),
            CombatDamageTarget::Object(second_blocker),
        ];
        let order = driver
            .combat_damage_order_context(active, pieces[0], &targets, &[])
            .unwrap_or_else(|error| panic!("order context should exist: {error}"));
        assert_eq!(order.kind(), DecisionKind::CombatDamage);
        assert_eq!(order.options().len(), 2);
        assert!(order.options().iter().all(|option| {
            option.actions().is_empty()
                && matches!(
                    option.descriptor(),
                    DecisionDescriptor::OrderCombatDamage { source, targets }
                        if *source == pieces[0] && targets.len() == 1
                )
        }));
        let reversed = vec![targets[1], targets[0]];
        let profile = driver
            .state
            .combat_damage_choice_profile_for_order(pieces[0], &reversed)
            .unwrap_or_else(|error| panic!("reversed profile should exist: {error:?}"));
        assert_eq!(profile.minimum_to_advance(), &[3, 0]);
        let amount = driver
            .combat_damage_amount_context(active, pieces[0], &reversed, &[], 0, (3, 4))
            .unwrap_or_else(|error| panic!("amount context should exist: {error}"));
        assert_eq!(amount.options().len(), 2);
        assert!(amount.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::AssignCombatDamage { source, target, amount: 3 | 4 }
                if *source == pieces[0]
                    && *target == TargetChoice::Object(second_blocker)
        )));
        let large = driver
            .combat_damage_amount_range_context(active, pieces[0], &reversed, &[], 0, (0, u32::MAX))
            .unwrap_or_else(|error| panic!("large range context should exist: {error}"));
        assert_eq!(large.options().len(), 2);
        let mut ranges = large
            .options()
            .iter()
            .map(|option| match option.descriptor() {
                DecisionDescriptor::ChooseCombatDamageRange {
                    minimum, maximum, ..
                } => (*minimum, *maximum),
                descriptor => panic!("unexpected large-range descriptor: {descriptor:?}"),
            })
            .collect::<Vec<_>>();
        ranges.sort_unstable();
        assert_eq!(
            ranges,
            vec![(0, u32::MAX / 2), (u32::MAX / 2 + 1, u32::MAX)]
        );

        let mut human_driver = driver.clone();
        let mut source = PickSecondChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        human_driver
            .assign_combat_damage(Some(active), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human damage assignment should succeed: {error}"));
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::AssignCombatDamage { assignments })
                if assignments.iter().any(|request| {
                    request.source() == pieces[0] && request.assignments().len() == 2
                })
        ));

        let mut legacy_driver = driver.clone();
        let mut legacy_source = LegacyNoPrompt;
        let mut legacy_decisions = Some(&mut legacy_source as &mut dyn DecisionSource);
        legacy_driver
            .assign_combat_damage(Some(active), &mut legacy_decisions, None)
            .unwrap_or_else(|error| panic!("legacy automatic damage should succeed: {error}"));
        assert!(matches!(
            legacy_driver.actions.last(),
            Some(Action::AssignCombatDamage { assignments })
                if assignments.iter().any(|request| {
                    request.source() == pieces[0] && request.assignments().len() == 1
                })
        ));

        let policies = [AiController::Random(RandomLegalPolicy::new(17)); PLAYER_COUNT];
        let mut no_decisions = None;
        driver
            .assign_combat_damage(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI damage assignment should succeed: {error}"));
        assert!(driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "combat_damage_order"));
        assert!(driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "combat_damage_amount"));
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
    fn modal_targeted_spell_choices_survive_canonical_cast_and_resolution() {
        let (mut driver, caster, opponent, spell) = modal_spell_driver();
        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("modal main context should exist: {error}"));
        let selected = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell {
                        object,
                        targets,
                        modes,
                        optional,
                        ..
                    } if *object == spell
                        && targets == &vec![TargetChoice::Player(opponent)]
                        && modes == &vec![0]
                        && optional.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("opponent damage mode should be canonical"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("selected modal option should have a typed adapter"));

        assert!(matches!(
            &choice,
            MainChoice::Cast {
                mode: Some(0),
                targets,
                optional,
                ..
            } if targets == &vec![TargetChoice::Player(opponent)] && optional.is_empty()
        ));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("modal spell should be on the stack"));
        assert_eq!(stack.decisions().mode(), Some(0));
        assert_eq!(
            stack
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::Player(opponent)]
        );

        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("caster pass should succeed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve: {error}"));
        assert_eq!(
            driver
                .state
                .resolution_log()
                .last()
                .map(|record| record.outcome()),
            Some(ResolutionOutcome::Resolved)
        );
        assert_eq!(
            driver
                .state
                .resolution_log()
                .last()
                .map(|record| record.decisions().mode()),
            Some(Some(0))
        );
        assert!(
            driver.actions.iter().any(|action| matches!(
                action,
                Action::DealDamage {
                    target: forge_core::CombatDamageTarget::Player(player),
                    amount: 4,
                    ..
                } if *player == opponent
            )),
            "resolved modal action missing from {:?}",
            driver.actions
        );
        assert_eq!(driver.state.players()[opponent.index()].life(), 16);
    }

    #[test]
    fn optional_spell_choices_survive_canonical_cast_and_resolution() {
        let source = r#"card "Healing Choice" {
  id: "f16f5077-392e-4606-8e48-b1ec2350cdb1"
  layout: normal
  status: unverified_playable
  face "Healing Choice" {
    cost: "{R}{W}"
    types: "Instant"
    oracle: "You may gain 3 life."
    keywords: []
    ability spell {
      effect: choose_up_to(1, gain_life(3, you()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("healing_choice.frs", source)
            .unwrap_or_else(|error| panic!("optional fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("optional fixture should compile: {error}")),
        );
        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, program);

        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("optional main context should exist: {error}"));
        let optional_values = context
            .options()
            .iter()
            .filter_map(|option| match option.descriptor() {
                DecisionDescriptor::CastSpell {
                    object, optional, ..
                } if *object == spell => Some(optional.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(optional_values, BTreeSet::from([vec![false], vec![true]]));
        let selected = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell {
                        object,
                        optional,
                        modes,
                        targets,
                        ..
                    } if *object == spell
                        && optional == &vec![true]
                        && modes.is_empty()
                        && targets.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("accepted optional effect should be canonical"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("optional option should have a typed adapter"));

        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("optional spell should be on the stack"));
        assert_eq!(
            stack.decisions().optional_choices().collect::<Vec<_>>(),
            vec![true]
        );

        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("caster pass should succeed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve: {error}"));
        assert_eq!(driver.state.players()[caster.index()].life(), 23);
        assert_eq!(
            driver
                .state
                .resolution_log()
                .last()
                .map(|record| record.decisions().optional_choices().collect::<Vec<_>>()),
            Some(vec![true])
        );
        assert!(driver.actions.iter().any(|action| matches!(
            action,
            Action::GainLife { player, amount } if *player == caster && *amount == 3
        )));
    }

    #[test]
    fn x_spell_choices_bind_the_announced_value_to_payment_and_stack() {
        let source = r#"card "X Choice" {
  id: "forge:test:x-choice"
  layout: normal
  status: unverified_playable
  face "X Choice" {
    cost: "{X}{R}"
    types: "Instant"
    oracle: "You gain 1 life."
    keywords: []
    ability spell {
      effect: gain_life(1, you())
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("x_choice.frs", source)
            .unwrap_or_else(|error| panic!("X fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("X fixture should compile: {error}")),
        );
        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, program);

        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("X main context should exist: {error}"));
        let begin = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginCastSpell {
                        object,
                        targets,
                        modes,
                        optional,
                    } if *object == spell
                        && targets.is_empty()
                        && modes.is_empty()
                        && optional.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("X spell should expose one deferred cast option"));
        assert!(begin.actions().is_empty());
        let begin_choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == begin.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("deferred X cast should have a typed adapter"));
        let replay_base = driver.clone();
        let replay_begin = begin_choice.clone();

        let (numeric, numeric_mappings) = driver
            .hierarchical_cast_context(caster, &begin_choice)
            .unwrap_or_else(|error| panic!("X numeric context should build: {error}"))
            .unwrap_or_else(|| panic!("X cast should require a numeric context"));
        assert_eq!(numeric.kind(), DecisionKind::NumericValue);
        assert!(numeric.path_discriminator().is_some());
        let x_values = numeric
            .options()
            .iter()
            .filter_map(|option| match option.descriptor() {
                DecisionDescriptor::ChooseNumber { value } => Some(*value),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(x_values, BTreeSet::from([0, 1]));
        let x_one = numeric
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ChooseNumber { value: 1 }
                )
            })
            .unwrap_or_else(|| panic!("X=1 should be canonical"));
        let x_one_terminal_choice = numeric
            .options()
            .iter()
            .position(|option| option.id() == x_one.id())
            .unwrap_or_else(|| panic!("X=1 should have a prompt position"))
            + 1;
        let payment_stage = numeric_mappings
            .iter()
            .find_map(|(id, choice)| (*id == x_one.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("X=1 should have a typed adapter"));
        let (payments, payment_mappings) = driver
            .hierarchical_cast_context(caster, &payment_stage)
            .unwrap_or_else(|error| panic!("X payment context should build: {error}"))
            .unwrap_or_else(|| panic!("selected X should require a payment context"));
        assert_eq!(payments.kind(), DecisionKind::Payment);
        assert!(payments.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChoosePayment { payment } if payment.x_value() == 1
        )));
        let selected = payments
            .options()
            .first()
            .unwrap_or_else(|| panic!("X=1 should have a legal payment"));
        let choice = payment_mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("X payment should have a final typed adapter"));

        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("X spell should be on the stack"));
        assert_eq!(stack.payment().map(PaymentPlan::x_value), Some(1));
        assert_eq!(
            driver.state.mana_pool(caster),
            Ok(forge_core::ManaPool::empty())
        );

        let mut human_driver = replay_base.clone();
        let scripted_input = format!("{x_one_terminal_choice}\n1\n");
        let mut input = Cursor::new(scripted_input.as_bytes());
        let mut output = Vec::new();
        let mut terminal = TerminalDecisionSource::new(&mut input, &mut output);
        assert_eq!(
            human_driver.finish_human_main_choice(caster, &mut terminal, replay_begin.clone()),
            Ok(true)
        );
        let decisions = terminal.into_decisions();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].prompt, "Choose X");
        assert_eq!(decisions[1].prompt, "Choose a mana payment");
        assert_eq!(
            decisions[0].canonical_legal_actions[decisions[0].selected].descriptor,
            serde_json::json!({"kind": "choose_number", "value": 1})
        );
        assert_eq!(
            decisions[1].canonical_legal_actions[decisions[1].selected].descriptor["kind"],
            "choose_payment"
        );

        let mut replay_driver = replay_base;
        let mut replay = ReplayDecisionSource::new(decisions);
        assert_eq!(
            replay_driver.finish_human_main_choice(caster, &mut replay, replay_begin),
            Ok(true)
        );
        assert!(replay.finish().is_ok());
        assert_eq!(replay_driver.actions, human_driver.actions);
        assert_eq!(
            replay_driver.state.deterministic_hash(),
            human_driver.state.deterministic_hash()
        );
    }

    #[test]
    fn large_x_ranges_split_logarithmically_and_ai_records_both_stages() {
        let source = r#"card "X Choice" {
  id: "forge:test:x-choice-ai"
  layout: normal
  status: unverified_playable
  face "X Choice" {
    cost: "{X}{R}"
    types: "Instant"
    oracle: "You gain 1 life."
    keywords: []
    ability spell {
      effect: gain_life(1, you())
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("x_choice_ai.frs", source)
            .unwrap_or_else(|error| panic!("X fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("X fixture should compile: {error}")),
        );
        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, program);

        let (large, _) = driver
            .variable_cast_numeric_context(caster, spell, &[], None, &[], (0, u32::MAX))
            .unwrap_or_else(|error| panic!("large X range should build: {error}"));
        assert_eq!(large.options().len(), 2);
        assert!(large.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseNumberRange { .. }
        )));
        let first_hidden = hidden_card_view(41, false);
        let second_hidden = hidden_card_view(99, false);
        let path = large
            .path_discriminator()
            .unwrap_or_else(|| panic!("large X context should be scoped"));
        let first_hidden_context = DecisionContext::new_scoped(
            DecisionKind::NumericValue,
            first_hidden.observer(),
            &first_hidden,
            large.options().to_vec(),
            Vec::new(),
            path,
        )
        .unwrap_or_else(|error| panic!("first hidden X context should build: {error}"));
        let second_hidden_context = DecisionContext::new_scoped(
            DecisionKind::NumericValue,
            second_hidden.observer(),
            &second_hidden,
            large.options().to_vec(),
            Vec::new(),
            path,
        )
        .unwrap_or_else(|error| panic!("second hidden X context should build: {error}"));
        assert_eq!(first_hidden_context.id(), second_hidden_context.id());
        assert_eq!(
            first_hidden_context.state_key(),
            second_hidden_context.state_key()
        );

        let (root, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("AI X root should build: {error}"));
        let begin = root
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginCastSpell { object, .. } if *object == spell
                )
            })
            .unwrap_or_else(|| panic!("AI X root should contain the spell"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == begin.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("AI X root should have a typed adapter"));
        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights should load: {error}"));
        {
            let domain = MainSearchDomain {
                root: &driver,
                actor: caster,
                weights,
                rollout_seed: 31,
                guardrail_profile: GuardrailProfile::Standard,
            };
            let search_root = domain
                .state(driver.clone())
                .unwrap_or_else(|error| panic!("search root should build: {error}"));
            let numeric_state = domain
                .apply_action(&search_root, begin.id())
                .unwrap_or_else(|error| panic!("search should enter X context: {error}"));
            assert_eq!(numeric_state.context.kind(), DecisionKind::NumericValue);
            let x_one = numeric_state
                .context
                .options()
                .iter()
                .find(|option| {
                    matches!(
                        option.descriptor(),
                        DecisionDescriptor::ChooseNumber { value: 1 }
                    )
                })
                .unwrap_or_else(|| panic!("search X context should contain one"));
            let payment_state = domain
                .apply_action(&numeric_state, x_one.id())
                .unwrap_or_else(|error| panic!("search should enter payment context: {error}"));
            assert_eq!(payment_state.context.kind(), DecisionKind::Payment);
            let payment = payment_state
                .context
                .options()
                .first()
                .unwrap_or_else(|| panic!("search payment context should not be empty"));
            let cast_state = domain
                .apply_action(&payment_state, payment.id())
                .unwrap_or_else(|error| panic!("search should apply final cast: {error}"));
            assert!(cast_state.finished);
            assert_eq!(
                cast_state
                    .driver
                    .state
                    .stack_top()
                    .and_then(|entry| entry.payment())
                    .map(PaymentPlan::x_value),
                Some(1)
            );
        }
        let policy = AiController::Heuristic(HeuristicPolicy::rollout(weights, 29));
        assert_eq!(
            driver.finish_ai_main_choice(caster, policy, choice),
            Ok(true)
        );
        assert!(driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "numeric_value"));
        assert!(driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "payment"));
        assert_eq!(
            driver
                .state
                .stack_top()
                .and_then(|entry| entry.payment())
                .map(PaymentPlan::x_value),
            Some(1)
        );
    }

    #[test]
    fn priority_adapter_exposes_exact_instant_choices_without_duplicate_labels() {
        let (driver, caster, opponent, spell) = modal_spell_driver();
        let (context, mappings) = driver
            .priority_decision_context(caster)
            .unwrap_or_else(|error| panic!("priority context should exist: {error}"));
        assert_eq!(context.kind(), DecisionKind::Priority);
        assert_eq!(context.options().len(), 5);
        assert_eq!(mappings.len(), context.options().len());
        assert_eq!(
            context
                .options()
                .iter()
                .filter(|option| matches!(option.descriptor(), DecisionDescriptor::PassPriority))
                .count(),
            1
        );
        assert!(context.options().iter().any(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::CastSpell {
                object,
                targets,
                modes,
                ..
            } if *object == spell
                && targets == &vec![TargetChoice::Player(opponent)]
                && modes == &vec![0]
        )));

        let labels = context
            .options()
            .iter()
            .map(|option| match option.descriptor() {
                DecisionDescriptor::PassPriority => "Pass priority".to_owned(),
                descriptor => driver
                    .main_choice_label(descriptor)
                    .unwrap_or_else(|error| panic!("priority label should exist: {error}")),
            })
            .collect::<Vec<_>>();
        assert_eq!(labels.iter().collect::<BTreeSet<_>>().len(), labels.len());
        assert!(labels.iter().any(|label| {
            label.contains("Boros Charm")
                && label.contains("targets seat 2")
                && label.contains("mode 1")
        }));
    }

    #[test]
    fn explicit_concession_uses_one_human_ai_and_replay_context() {
        let (driver, caster, opponent, _spell) = modal_spell_driver();
        let mut human_driver = driver.clone();
        let mut input = Cursor::new(b"concede\n".as_slice());
        let mut output = Vec::new();
        let mut terminal = TerminalDecisionSource::new(&mut input, &mut output);
        human_driver
            .take_human_priority_action(caster, &mut terminal)
            .unwrap_or_else(|error| panic!("human concession should succeed: {error}"));
        let decisions = terminal.into_decisions();

        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].prompt, CONCESSION_PROMPT);
        assert_eq!(decisions[0].canonical_legal_actions.len(), 1);
        assert_eq!(
            decisions[0].canonical_legal_actions[0].descriptor,
            serde_json::json!({"kind": "concede"})
        );
        assert!(String::from_utf8(output)
            .unwrap_or_else(|error| panic!("terminal output should be UTF-8: {error}"))
            .contains("Seat 1 conceded."));
        assert!(human_driver.state.players()[caster.index()].lost());
        assert_eq!(
            human_driver.state.game_outcome(),
            forge_core::GameOutcome::Won(opponent)
        );
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::Concede { player }) if *player == caster
        ));

        let mut replay_driver = driver.clone();
        let mut replay = ReplayDecisionSource::new(decisions);
        replay_driver
            .take_human_priority_action(caster, &mut replay)
            .unwrap_or_else(|error| panic!("concession replay should succeed: {error}"));
        assert!(replay.finish().is_ok());
        assert_eq!(replay_driver.actions, human_driver.actions);
        assert_eq!(
            replay_driver.state.deterministic_hash(),
            human_driver.state.deterministic_hash()
        );

        let mut ai_driver = driver;
        let context = concession_decision_context(&ai_driver.state, caster)
            .unwrap_or_else(|error| panic!("AI concession context should exist: {error}"));
        assert_eq!(context.kind(), DecisionKind::Concession);
        assert_eq!(context.options().len(), 1);
        let selected_id = RandomLegalPolicy::new(91)
            .select(&context, 0)
            .unwrap_or_else(|error| panic!("AI should select the singleton context: {error}"));
        let selected = context
            .select(selected_id)
            .unwrap_or_else(|error| panic!("AI selection should be canonical: {error}"));
        for action in selected.actions().to_vec() {
            ai_driver
                .dispatch(action)
                .unwrap_or_else(|error| panic!("AI concession action should apply: {error}"));
        }
        assert_eq!(
            ai_driver.state.deterministic_hash(),
            human_driver.state.deterministic_hash()
        );

        let first_hidden = hidden_card_view(7, false);
        let second_hidden = hidden_card_view(8, false);
        let first_context =
            concession_decision_context_from_view(&first_hidden, first_hidden.observer())
                .unwrap_or_else(|error| panic!("first hidden context should exist: {error}"));
        let second_context =
            concession_decision_context_from_view(&second_hidden, second_hidden.observer())
                .unwrap_or_else(|error| panic!("second hidden context should exist: {error}"));
        assert_eq!(first_context.id(), second_context.id());
        assert_eq!(first_context.state_key(), second_context.state_key());
    }

    #[test]
    fn program_activated_choices_survive_priority_stack_and_resolution() {
        let source = r#"card "Choice Pinger" {
  id: "f46a6fa5-df77-4da9-bd2c-733c86174cf9"
  layout: normal
  status: unverified_playable
  face "Choice Pinger" {
    cost: "{1}{R}"
    types: "Enchantment"
    oracle: "{R}: You may deal 2 damage to any target."
    keywords: []
    ability activated {
      costs: [mana_cost("{R}")]
      effect: choose_up_to(1, deal_damage(target(any()), 2))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("choice_pinger.frs", source)
            .unwrap_or_else(|error| panic!("activated fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("activated fixture should compile: {error}")),
        );
        assert_eq!(program.activated_effects().len(), 1);
        let (mut driver, caster, opponent, permanent) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(permanent, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: permanent,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("activated base setup should succeed: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: permanent,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("activated source setup should succeed: {error}"));
        driver
            .register_permanent_runtime(caster, permanent)
            .unwrap_or_else(|error| panic!("activated runtime should register: {error}"));

        let (context, mappings) = driver
            .priority_decision_context(caster)
            .unwrap_or_else(|error| panic!("activated priority context should exist: {error}"));
        let selected = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ActivateProgramAbility {
                        source,
                        targets,
                        optional,
                        ..
                    } if *source == permanent
                        && targets == &vec![TargetChoice::Player(opponent)]
                        && optional == &vec![true]
                )
            })
            .unwrap_or_else(|| panic!("accepted opponent activation should be canonical"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("activated option should have a typed adapter"));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("activated ability should be on the stack"));
        assert!(stack.activated_ability().is_some());
        assert_eq!(
            stack.decisions().optional_choices().collect::<Vec<_>>(),
            vec![true]
        );
        assert_eq!(
            stack
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::Player(opponent)]
        );

        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("caster pass should succeed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve: {error}"));
        assert_eq!(driver.state.players()[opponent.index()].life(), 18);
        assert!(driver.actions.iter().any(|action| matches!(
            action,
            Action::DealDamage {
                target: CombatDamageTarget::Player(player),
                amount: 2,
                ..
            } if *player == opponent
        )));
    }

    #[test]
    fn simultaneous_trigger_order_uses_shared_human_and_ai_contexts() {
        let (mut driver, controller, opponent, _source) = modal_spell_driver();
        let mut triggers = Vec::new();
        for _ in 0..2 {
            let trigger = match driver
                .dispatch(Action::RegisterTriggeredAbility {
                    definition: TriggerDefinition::new(
                        controller,
                        TriggerCondition::LifeLost {
                            player: TriggerPlayerFilter::Any,
                        },
                    ),
                })
                .unwrap_or_else(|error| panic!("trigger registration should succeed: {error}"))
            {
                Outcome::TriggerRegistered(trigger) => trigger,
                other => panic!("unexpected trigger registration outcome: {other:?}"),
            };
            triggers.push(trigger);
        }
        driver
            .dispatch(Action::LoseLife {
                player: opponent,
                amount: 1,
            })
            .unwrap_or_else(|error| panic!("trigger event should succeed: {error}"));
        let context = driver
            .trigger_order_context(controller, &triggers, &[], &[])
            .unwrap_or_else(|error| panic!("trigger order context should exist: {error}"));
        assert_eq!(context.kind(), DecisionKind::TriggerOrder);
        assert_eq!(context.options().len(), 2);
        assert!(context.options().iter().all(|option| {
            option.actions().is_empty()
                && matches!(
                    option.descriptor(),
                    DecisionDescriptor::OrderTriggers { triggers } if triggers.len() == 1
                )
        }));

        let mut human_driver = driver.clone();
        let mut source = PickSecondChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        let human_outcome = human_driver
            .put_pending_triggers_on_stack(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human trigger order should succeed: {error}"));
        assert!(matches!(human_outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 2));
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::PutPendingTriggeredAbilitiesOnStackInOrder { order }) if order.len() == 2
        ));

        let policies = [AiController::Random(RandomLegalPolicy::new(17)); PLAYER_COUNT];
        let mut ai_driver = driver;
        let mut no_decisions = None;
        let ai_outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI trigger order should succeed: {error}"));
        assert!(matches!(ai_outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 2));
        assert_eq!(ai_driver.ai_decisions.len(), 1);
        assert_eq!(ai_driver.ai_decisions[0].kind, "trigger_order");
        assert!(ai_driver.ai_decisions[0]
            .canonical_legal_actions
            .iter()
            .any(|action| action.action_id == ai_driver.ai_decisions[0].action_id));
    }

    #[test]
    fn evolving_wilds_search_is_chosen_at_resolution_and_filters_nonbasic_lands() {
        let source = r#"card "Evolving Wilds" {
  id: "a75445d3-1303-4bb5-89ad-26ea93fecd48"
  layout: normal
  status: unverified_playable
  face "Evolving Wilds" {
    cost: ""
    types: "Land"
    oracle: "{T}, Sacrifice Evolving Wilds: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle."
    keywords: []
    ability activated {
      costs: [tap_self(), sacrifice_self()]
      effect: sequence(search_library(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library")))), "battlefield", 1), tap(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))))), shuffle(you()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("evolving_wilds.frs", source)
            .unwrap_or_else(|error| panic!("Evolving Wilds should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("Evolving Wilds should compile: {error}")),
        );
        let (mut driver, caster, _opponent, wilds) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(wilds, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: wilds,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("Wilds characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: wilds,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("Wilds should enter the battlefield: {error}"));

        let library = ZoneId::new(Some(caster), ZoneKind::Library);
        let nonbasic = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(903),
                owner: caster,
                controller: caster,
                zone: library,
            })
            .unwrap_or_else(|error| panic!("nonbasic setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected nonbasic setup outcome: {other:?}"),
        };
        let basic = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(904),
                owner: caster,
                controller: caster,
                zone: library,
            })
            .unwrap_or_else(|error| panic!("basic setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected basic setup outcome: {other:?}"),
        };
        let forest = BasicLandTypes::none().with_forest();
        for (object, supertypes) in [
            (nonbasic, ObjectSupertypes::none()),
            (basic, ObjectSupertypes::none().with_basic()),
        ] {
            driver
                .dispatch(Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_land(),
                        ObjectColors::none(),
                    )
                    .with_supertypes(supertypes)
                    .with_basic_land_types(forest),
                })
                .unwrap_or_else(|error| panic!("land characteristics should apply: {error}"));
        }
        driver
            .register_permanent_runtime(caster, wilds)
            .unwrap_or_else(|error| panic!("Wilds runtime should register: {error}"));

        let (context, mappings) = driver
            .priority_decision_context(caster)
            .unwrap_or_else(|error| panic!("Wilds priority context should exist: {error}"));
        let selected = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ActivateProgramAbility {
                        source,
                        targets,
                        optional,
                        ..
                    } if *source == wilds && targets.is_empty() && optional.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("Wilds activation should be canonical"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("Wilds activation should have a typed adapter"));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("caster pass should succeed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve Wilds: {error}"));
        assert!(driver.pending_activated_resolution.is_some());
        assert_eq!(
            driver.state.object_zone(wilds),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );

        let pending = driver
            .pending_activated_resolution
            .as_ref()
            .unwrap_or_else(|| panic!("Wilds should await its search choice"));
        let search = driver
            .pending_activated_context(pending)
            .unwrap_or_else(|error| panic!("Wilds search context should exist: {error}"));
        assert_eq!(search.kind(), DecisionKind::Search);
        assert_eq!(search.options().len(), 2);
        assert!(search
            .options()
            .iter()
            .all(|option| match option.descriptor() {
                DecisionDescriptor::ChooseResolutionObjects { choices } => {
                    !choices.iter().flatten().any(|object| *object == nonbasic)
                }
                _ => false,
            }));
        assert!(search.options().iter().any(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseResolutionObjects { choices }
                if choices == &vec![vec![basic]]
        )));

        let mut source = PickNonEmptyResolutionChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        driver
            .complete_pending_activated_resolution(Some(caster), &mut decisions, None)
            .unwrap_or_else(|error| panic!("Wilds search should complete: {error}"));
        assert!(driver.pending_activated_resolution.is_none());
        assert_eq!(
            driver.state.object_zone(basic),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert!(driver
            .state
            .object(basic)
            .is_some_and(|record| record.tapped()));
        assert_eq!(driver.state.object_zone(nonbasic), Some(library));
        assert!(driver.actions.iter().any(|action| matches!(
            action,
            Action::ShuffleLibrary { player } if *player == caster
        )));
    }

    #[test]
    fn resolution_choice_slots_do_not_materialize_a_cartesian_product() {
        let source = r#"card "Two Searches" {
  id: "88e24adb-2fb2-49db-8e55-f5bbeb18e2f7"
  layout: normal
  status: unverified_playable
  face "Two Searches" {
    cost: "{1}"
    types: "Artifact"
    oracle: "{1}: Search your library twice."
    keywords: []
    ability activated {
      costs: [mana_cost("{1}")]
      effect: sequence(search_library(cards(and(type_is("land"), zone_is("library"))), you(), 1), search_library(cards(and(type_is("creature"), zone_is("library"))), you(), 1))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("two_searches.frs", source)
            .unwrap_or_else(|error| panic!("two-search fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("two-search fixture should compile: {error}")),
        );
        let requirements = program.activated_effects()[0]
            .object_choice_requirements()
            .to_vec();
        assert_eq!(requirements.len(), 2);

        let (mut driver, controller, _opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        let library = ZoneId::new(Some(controller), ZoneKind::Library);
        for index in 0..8 {
            let object = match driver
                .dispatch(Action::CreateObject {
                    card: CardId::new(1_100 + index),
                    owner: controller,
                    controller,
                    zone: library,
                })
                .unwrap_or_else(|error| panic!("search candidate setup should succeed: {error}"))
            {
                Outcome::ObjectCreated(object) => object,
                other => panic!("unexpected search candidate outcome: {other:?}"),
            };
            driver
                .dispatch(Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_land(),
                        ObjectColors::none(),
                    ),
                })
                .unwrap_or_else(|error| panic!("search candidate types should apply: {error}"));
        }
        for index in 0..8 {
            let object = match driver
                .dispatch(Action::CreateObject {
                    card: CardId::new(1_200 + index),
                    owner: controller,
                    controller,
                    zone: library,
                })
                .unwrap_or_else(|error| panic!("creature candidate setup should succeed: {error}"))
            {
                Outcome::ObjectCreated(object) => object,
                other => panic!("unexpected creature candidate outcome: {other:?}"),
            };
            driver
                .dispatch(Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature(),
                        ObjectColors::none(),
                    ),
                })
                .unwrap_or_else(|error| panic!("creature candidate types should apply: {error}"));
        }
        let pending = PendingActivatedResolution {
            controller,
            runtime: ActivatedRuntime {
                program,
                ability_index: 0,
                source: source_object,
            },
            targets: Vec::new(),
            decisions: StackDecisionBindings::default(),
        };
        let first = driver
            .pending_activated_choice_context(&pending, &requirements, 0, &[])
            .unwrap_or_else(|error| panic!("first search slot should exist: {error}"));
        assert_eq!(first.kind(), DecisionKind::Search);
        assert_eq!(first.options().len(), 9);
        let first_choice = first
            .options()
            .iter()
            .find_map(|option| match option.descriptor() {
                DecisionDescriptor::ChooseResolutionObjects { choices }
                    if choices.first().is_some_and(|choice| !choice.is_empty()) =>
                {
                    Some(choices.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("first search should expose a non-empty choice"));
        let second = driver
            .pending_activated_choice_context(&pending, &requirements, 1, &first_choice)
            .unwrap_or_else(|error| panic!("second search slot should exist: {error}"));
        assert_eq!(second.options().len(), 9);
        assert_ne!(first.state_key(), second.state_key());
        assert!(second.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseResolutionObjects { choices } if choices.len() == 2
        )));
    }

    #[test]
    fn sword_of_the_animist_trigger_search_uses_the_canonical_resolution_choice() {
        let source = r#"card "Sword of the Animist" {
  id: "d79cbc61-6c15-48ea-bbba-3cffb819ccba"
  layout: normal
  status: unverified_playable
  face "Sword of the Animist" {
    cost: "{2}"
    types: "Legendary Artifact - Equipment"
    oracle: "Whenever equipped creature attacks, you may search your library for a basic land card, put it onto the battlefield tapped, then shuffle."
    keywords: [equip]
    ability activated {
      costs: [mana_cost("{2}")]
      timing: timing_sorcery()
      effect: attach(source(), target(permanents(and(type_is("creature"), controlled_by(you())))))
    }
    ability static {
      effect: continuous(equipped_object(source()), modify_pt(any(), 1, 1))
    }
    ability triggered {
      event: event_attacks(equipped_object(source()))
      effect: choose_up_to(1, sequence(search_library(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library")))), "battlefield", 1), tap(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))))), shuffle(you())))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("sword_of_the_animist.frs", source)
            .unwrap_or_else(|error| panic!("Sword should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("Sword should compile: {error}")),
        );
        let (mut driver, controller, _opponent, sword) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(sword, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: sword,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("Sword characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: sword,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("Sword should enter the battlefield: {error}"));

        let library = ZoneId::new(Some(controller), ZoneKind::Library);
        let nonbasic = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(905),
                owner: controller,
                controller,
                zone: library,
            })
            .unwrap_or_else(|error| panic!("nonbasic setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected nonbasic setup outcome: {other:?}"),
        };
        let basic = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(906),
                owner: controller,
                controller,
                zone: library,
            })
            .unwrap_or_else(|error| panic!("basic setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected basic setup outcome: {other:?}"),
        };
        let forest = BasicLandTypes::none().with_forest();
        for (object, supertypes) in [
            (nonbasic, ObjectSupertypes::none()),
            (basic, ObjectSupertypes::none().with_basic()),
        ] {
            driver
                .dispatch(Action::SetBaseObjectCharacteristics {
                    object,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_land(),
                        ObjectColors::none(),
                    )
                    .with_supertypes(supertypes)
                    .with_basic_land_types(forest),
                })
                .unwrap_or_else(|error| panic!("land characteristics should apply: {error}"));
        }
        driver
            .register_triggers(controller, sword, &program)
            .unwrap_or_else(|error| panic!("Sword trigger should register: {error}"));
        driver
            .register_permanent_runtime(controller, sword)
            .unwrap_or_else(|error| panic!("Sword runtime should register: {error}"));
        let trigger = driver
            .trigger_programs
            .keys()
            .copied()
            .next()
            .unwrap_or_else(|| panic!("Sword trigger should register"));
        driver
            .execute_trigger(controller, trigger)
            .unwrap_or_else(|error| panic!("Sword trigger should await a search: {error}"));
        let pending = driver
            .pending_triggered_resolution
            .as_ref()
            .unwrap_or_else(|| panic!("Sword should await its search choice"));
        let search = driver
            .pending_triggered_context(pending)
            .unwrap_or_else(|error| panic!("Sword search context should exist: {error}"));
        assert_eq!(search.kind(), DecisionKind::Search);
        assert_eq!(search.options().len(), 2);
        assert!(search.options().iter().any(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseResolutionObjects { choices }
                if choices == &vec![vec![basic]]
        )));
        assert!(search
            .options()
            .iter()
            .all(|option| match option.descriptor() {
                DecisionDescriptor::ChooseResolutionObjects { choices } => {
                    !choices.iter().flatten().any(|object| *object == nonbasic)
                }
                _ => false,
            }));

        let mut source = PickNonEmptyResolutionChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        driver
            .complete_pending_triggered_resolution(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("Sword search should complete: {error}"));
        assert!(driver.pending_triggered_resolution.is_none());
        assert_eq!(
            driver.state.object_zone(basic),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert!(driver
            .state
            .object(basic)
            .is_some_and(|record| record.tapped()));
        assert_eq!(driver.state.object_zone(nonbasic), Some(library));
        assert!(driver.actions.iter().any(|action| matches!(
            action,
            Action::ShuffleLibrary { player } if *player == controller
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
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            (prompt.options.len() == 2)
                .then_some(DecisionSelection::Option(1))
                .ok_or_else(|| "expected exactly two commander-zone options".to_owned())
        }
    }

    struct LegacyNoPrompt;

    impl DecisionSource for LegacyNoPrompt {
        fn choose(&mut self, _prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            Err("legacy combat damage must remain automatic".to_owned())
        }

        fn is_legacy_replay(&self) -> bool {
            true
        }
    }

    struct PickNonEmptyResolutionChoice;

    impl DecisionSource for PickNonEmptyResolutionChoice {
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            prompt
                .options
                .iter()
                .position(|label| !label.starts_with("Find no matching"))
                .map(DecisionSelection::Option)
                .ok_or_else(|| "expected a non-empty resolution choice".to_owned())
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
            activated_abilities: Vec::new(),
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
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

    fn modal_spell_driver() -> (GameDriver, PlayerId, PlayerId, ObjectId) {
        let source = r#"card "Boros Charm" {
  id: "2679d0dd-ba30-4a1c-b6a0-b3ac6c790496"
  layout: normal
  status: unverified_playable
  face "Boros Charm" {
    cost: "{R}{W}"
    types: "Instant"
    oracle: "Choose one"
    keywords: []
    ability spell {
      effect: choose_one(deal_damage(target(all(any(), permanents(type_is("planeswalker")))), 4), grant_keyword(permanents(controlled_by(you())), "indestructible", "until_end_of_turn"), grant_keyword(target(permanents(type_is("creature"))), "double_strike", "until_end_of_turn"))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("boros_charm.frs", source)
            .unwrap_or_else(|error| panic!("modal fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("modal fixture should compile: {error}")),
        );
        let mut state = GameState::new();
        let caster = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected caster setup outcome: {other:?}"),
        };
        let opponent = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected opponent setup outcome: {other:?}"),
        };
        let spell = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(900),
                owner: caster,
                controller: caster,
                zone: ZoneId::new(Some(caster), ZoneKind::Hand),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected spell setup outcome: {other:?}"),
        };
        let creature = create_test_creature(&mut state, opponent, 901, 2, 2);
        assert!(matches!(
            apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(902),
                    owner: caster,
                    controller: caster,
                    zone: ZoneId::new(Some(caster), ZoneKind::Library),
                }
            ),
            Outcome::ObjectCreated(_)
        ));
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: caster
                }
            ),
            Outcome::Applied
        );
        while state.current_step() != Some(Step::PrecombatMain) {
            assert!(matches!(
                apply(&mut state, Action::AdvanceStep),
                Outcome::StepAdvanced(_)
            ));
        }
        assert_eq!(
            apply(
                &mut state,
                Action::AddManaToPool {
                    player: caster,
                    mana: ManaPool::new(1, 0, 0, 1, 0, 0),
                }
            ),
            Outcome::Applied
        );
        let mut programs = HashMap::new();
        programs.insert(spell, program);
        let driver = GameDriver {
            state,
            players: vec![caster, opponent],
            programs: Arc::new(programs),
            deck_models: Arc::new(Vec::new()),
            card_definitions: Arc::new(HashMap::new()),
            guardrails: Arc::new(
                GuardrailTable::bundled()
                    .unwrap_or_else(|error| panic!("guardrails should load: {error}")),
            ),
            commanders: vec![spell, creature],
            trigger_programs: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
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
        (driver, caster, opponent, spell)
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
            activated_abilities: Vec::new(),
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
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
