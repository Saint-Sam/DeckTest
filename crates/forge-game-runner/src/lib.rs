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
    compile_card_program, expand_announced_targets, object_satisfies_choice_requirement,
    AlternateCostKind, CardProgram, ExecutionBindings, ObjectChoiceRequirement, PlayerBinding,
    ProgramKind, SpellAdditionalCostProgram, SpellModeProgram,
};
use forge_core::{
    apply, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect, ActivatedAbilityId,
    ActivationCost, AnnouncedTarget, AttackDeclaration, BenchmarkRuntimeSemantics,
    BlockDeclaration, CanonicalActionId, CardId, CastSpellRequest, CombatDamageAssignment,
    CombatDamageAssignmentRequest, CombatDamageStepKind, CombatDamageTarget, DecisionContext,
    DecisionDescriptor, DecisionKind, DecisionOption, GameEvent, GameOutcome, GameState,
    HiddenCardDefinition, HiddenSlotDefinition, ManaKind, ObjectColors, ObjectId, ObjectView,
    Outcome, PaymentPlan, PendingTriggeredAbility, PlayerId, PlayerView, PriorityOutcome,
    ResolutionOutcome, SpellAdditionalCostPayment, SpellAlternateCost, SpellTiming,
    StackDecisionBindings, StackEntryId, StackObjectKind, StateError, Step, TargetChoice,
    TargetKind, TargetPredicate, TargetRequirement, TriggerId, TriggerStackBinding,
    TriggerStackDisposition, ZoneId, ZoneKind,
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct DecisionEpisodeMetadata {
    #[serde(default)]
    decision_episode_id: String,
    #[serde(default)]
    root_context_id: String,
    #[serde(default)]
    parent_context_id: Option<String>,
    #[serde(default)]
    path_depth: u32,
    #[serde(default)]
    is_forced: bool,
    #[serde(default)]
    is_strategic_root: bool,
    #[serde(default)]
    is_terminal_subchoice: bool,
    #[serde(default)]
    final_concrete_action_id: String,
}

impl DecisionEpisodeMetadata {
    fn root(seed: u64, ordinal: u64, context: &DecisionContext, terminal: bool) -> Self {
        let root_context_id = context.id().to_string();
        Self {
            decision_episode_id: stable_record_id(
                b"forge-decision-episode-v1",
                seed,
                ordinal,
                [root_context_id.as_str()],
            ),
            root_context_id,
            parent_context_id: None,
            path_depth: 0,
            is_forced: context.options().len() == 1,
            is_strategic_root: context.options().len() > 1,
            is_terminal_subchoice: terminal,
            final_concrete_action_id: String::new(),
        }
    }

    fn child(&self, parent_context_id: String, context: &DecisionContext) -> Self {
        Self {
            decision_episode_id: self.decision_episode_id.clone(),
            root_context_id: self.root_context_id.clone(),
            parent_context_id: Some(parent_context_id),
            path_depth: self.path_depth.saturating_add(1),
            is_forced: context.options().len() == 1,
            is_strategic_root: false,
            is_terminal_subchoice: false,
            final_concrete_action_id: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct DecisionEpisodePath {
    metadata: DecisionEpisodeMetadata,
    last_context_id: String,
    selected_action_ids: Vec<String>,
}

impl DecisionEpisodePath {
    fn root(seed: u64, ordinal: u64, context: &DecisionContext) -> Self {
        let metadata = DecisionEpisodeMetadata::root(seed, ordinal, context, false);
        Self {
            last_context_id: context.id().to_string(),
            metadata,
            selected_action_ids: Vec::new(),
        }
    }

    fn root_metadata(&self) -> DecisionEpisodeMetadata {
        self.metadata.clone()
    }

    fn child_metadata(&self, context: &DecisionContext) -> DecisionEpisodeMetadata {
        self.metadata.child(self.last_context_id.clone(), context)
    }

    fn record_selection(
        &mut self,
        context: &DecisionContext,
        action: CanonicalActionId,
        path_depth: u32,
    ) {
        self.last_context_id = context.id().to_string();
        self.metadata.path_depth = path_depth;
        self.selected_action_ids.push(action.to_string());
    }

    fn final_action_id(&self) -> String {
        if self.selected_action_ids.len() == 1 {
            return self.selected_action_ids[0].clone();
        }
        stable_record_id(
            b"forge-concrete-decision-path-v1",
            0,
            self.selected_action_ids.len() as u64,
            self.selected_action_ids.iter().map(String::as_str),
        )
    }
}

fn stable_record_id<'a>(
    domain: &[u8],
    seed: u64,
    ordinal: u64,
    values: impl IntoIterator<Item = &'a str>,
) -> String {
    let mut low = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    let mut high = 0x8422_2325_cbf2_9ce4_u64 ^ ordinal.rotate_left(29);
    let mut mix = |byte: u8| {
        low ^= u64::from(byte);
        low = low.wrapping_mul(0x0000_0100_0000_01b3);
        high ^= u64::from(byte).rotate_left(1);
        high = high.wrapping_mul(0x9e37_79b1_85eb_ca87);
    };
    for byte in domain {
        mix(*byte);
    }
    for byte in seed.to_le_bytes().into_iter().chain(ordinal.to_le_bytes()) {
        mix(byte);
    }
    for value in values {
        mix(0xff);
        for byte in value.as_bytes() {
            mix(*byte);
        }
    }
    format!("{high:016x}{low:016x}")
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
    normalized_benchmark_key: String,
    #[serde(default)]
    normalized_player_view_hash: String,
    #[serde(default)]
    normalized_legal_action_ids: Vec<String>,
    #[serde(default)]
    benchmark_normalization_complete: bool,
    #[serde(default)]
    path_discriminator: Option<u64>,
    #[serde(default)]
    player_view_hash: String,
    #[serde(default)]
    canonical_legal_actions: Vec<AiLegalAction>,
    #[serde(default)]
    selected_action_id: String,
    #[serde(flatten)]
    episode: DecisionEpisodeMetadata,
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
    normalized_benchmark_key: String,
    #[serde(default)]
    normalized_player_view_hash: String,
    #[serde(default)]
    normalized_legal_action_ids: Vec<String>,
    #[serde(default)]
    benchmark_normalization_complete: bool,
    #[serde(default)]
    path_discriminator: Option<u64>,
    #[serde(default)]
    player_view_hash: String,
    action_id: String,
    #[serde(flatten)]
    episode: DecisionEpisodeMetadata,
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
    episode: Option<DecisionEpisodeMetadata>,
}

struct DecisionPrompt<'a> {
    kind: &'static str,
    view: &'a PlayerView,
    context: &'a DecisionContext,
    options: &'a [String],
    allow_concession: bool,
    episode: DecisionEpisodeMetadata,
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

    fn decision_count(&self) -> u64 {
        0
    }

    fn complete_episode(
        &mut self,
        _decision_episode_id: &str,
        _final_concrete_action_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

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
    fn decision_count(&self) -> u64 {
        self.decisions.len() as u64
    }

    fn complete_episode(
        &mut self,
        decision_episode_id: &str,
        final_concrete_action_id: &str,
    ) -> Result<(), String> {
        let mut matched = self
            .decisions
            .iter_mut()
            .filter(|decision| decision.episode.decision_episode_id == decision_episode_id)
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Err(format!(
                "cannot complete unknown human decision episode {decision_episode_id}"
            ));
        }
        for decision in &mut matched {
            decision.episode.is_terminal_subchoice = false;
            decision.episode.final_concrete_action_id = final_concrete_action_id.to_owned();
        }
        if let Some(last) = matched.last_mut() {
            last.episode.is_terminal_subchoice = true;
        }
        Ok(())
    }

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
    fn decision_count(&self) -> u64 {
        self.cursor as u64
    }

    fn complete_episode(
        &mut self,
        decision_episode_id: &str,
        final_concrete_action_id: &str,
    ) -> Result<(), String> {
        let matched = self.decisions[..self.cursor]
            .iter()
            .filter(|decision| decision.episode.decision_episode_id == decision_episode_id)
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Ok(());
        }
        if matched
            .iter()
            .any(|decision| decision.episode.final_concrete_action_id != final_concrete_action_id)
        {
            return Err(format!(
                "decision replay episode {decision_episode_id} has a different final concrete action"
            ));
        }
        if matched
            .iter()
            .filter(|decision| decision.episode.is_terminal_subchoice)
            .count()
            != 1
            || !matched
                .last()
                .is_some_and(|decision| decision.episode.is_terminal_subchoice)
        {
            return Err(format!(
                "decision replay episode {decision_episode_id} has an invalid terminal subchoice"
            ));
        }
        Ok(())
    }

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
    let mut episode = prompt.episode.clone();
    if episode.is_terminal_subchoice {
        episode
            .final_concrete_action_id
            .clone_from(&selected_action_id);
    }
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
        normalized_benchmark_key: prompt.context.normalized_benchmark_key().to_string(),
        normalized_player_view_hash: format!(
            "{:016x}",
            prompt.context.normalized_player_view_hash().get()
        ),
        normalized_legal_action_ids: prompt
            .context
            .normalized_action_ids()
            .iter()
            .map(ToString::to_string)
            .collect(),
        benchmark_normalization_complete: prompt.context.benchmark_normalization_complete(),
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
        episode,
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
        let mut normalized_actual = actual.clone();
        if expected.normalized_benchmark_key.is_empty() {
            normalized_actual.normalized_benchmark_key.clear();
            normalized_actual.normalized_player_view_hash.clear();
            normalized_actual.normalized_legal_action_ids.clear();
            normalized_actual.benchmark_normalization_complete = false;
        }
        if expected.episode.decision_episode_id.is_empty() {
            normalized_actual.episode = DecisionEpisodeMetadata::default();
        } else {
            normalized_actual.episode.is_terminal_subchoice =
                expected.episode.is_terminal_subchoice;
            normalized_actual
                .episode
                .final_concrete_action_id
                .clone_from(&expected.episode.final_concrete_action_id);
        }
        return expected == &normalized_actual;
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
    #[serde(default)]
    meaningful_actions: u64,
    casts: u64,
    commander_casts: u64,
    taxed_commander_recasts: u64,
    commander_zone_returns: u64,
    lands_played: u64,
    mana_abilities: u64,
    priority_passes: u64,
    #[serde(default)]
    pass_only_priority_cycles: u64,
    #[serde(default)]
    table_damage_to_players: u64,
    #[serde(default)]
    life_total_movement: u64,
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
        self.meaningful_actions += other.meaningful_actions;
        self.casts += other.casts;
        self.commander_casts += other.commander_casts;
        self.taxed_commander_recasts += other.taxed_commander_recasts;
        self.commander_zone_returns += other.commander_zone_returns;
        self.lands_played += other.lands_played;
        self.mana_abilities += other.mana_abilities;
        self.priority_passes += other.priority_passes;
        self.pass_only_priority_cycles += other.pass_only_priority_cycles;
        self.table_damage_to_players += other.table_damage_to_players;
        self.life_total_movement += other.life_total_movement;
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

/// One eliminated seat and the turn on which it left the game.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EliminationDiagnostic {
    /// Zero-based seat index.
    pub seat: usize,
    /// Game turn on which the elimination occurred.
    pub turn: u32,
}

/// Activity recorded across one four-seat table round.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RoundProgressDiagnostic {
    /// One-based table round number.
    pub round: u32,
    /// First game turn included in this round.
    pub first_turn: u32,
    /// Last game turn included in this round.
    pub last_turn: u32,
    /// Damage actually dealt to players during the round.
    pub table_damage_to_players: u64,
    /// Cumulative absolute life-total movement during the round.
    pub life_total_movement: u64,
    /// Spells cast during the round.
    pub casts: u64,
    /// Successful typed actions excluding passes, step advancement, and rules checks.
    pub meaningful_actions: u64,
    /// Empty-stack all-pass priority cycles during the round.
    pub pass_only_priority_cycles: u64,
    /// Active-player turns containing at least one meaningful action.
    pub active_players_with_progress: u32,
    /// Players eliminated during the round.
    pub eliminations: u64,
    /// True only when the round changed no tracked progress signal.
    pub no_progress: bool,
}

/// Diagnostic-only long-game telemetry; it never changes game termination.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct GameProgressDiagnostics {
    /// Distinguishes ordinary wins from future diagnostic termination modes.
    pub termination_reason: String,
    /// True only when a diagnostic run ended at its configured turn cap.
    pub turn_cap_reached: bool,
    /// Number of full-state observations taken at runner boundaries.
    pub state_observations: u64,
    /// Observations whose full-state hash had appeared before.
    pub repeated_full_state_hashes: u64,
    /// Repeated full-state observations in parts per million.
    pub repeated_full_state_hash_rate_ppm: u32,
    /// AI prompt records whose `DecisionStateKey` had appeared before.
    pub repeated_decision_state_keys: u64,
    /// Repeated AI decision-state keys in parts per million.
    pub repeated_decision_state_key_rate_ppm: u32,
    /// Count of rounds with no tracked progress.
    pub no_progress_rounds: u32,
    /// Longest consecutive run of no-progress rounds.
    pub maximum_consecutive_no_progress_rounds: u32,
    /// Turn-stamped eliminations in seat order of occurrence.
    pub eliminations: Vec<EliminationDiagnostic>,
    /// Per-round progress records.
    pub rounds: Vec<RoundProgressDiagnostic>,
}

#[derive(Clone, Debug)]
struct RoundProgressStart {
    round: u32,
    first_turn: u32,
    last_turn: u32,
    metrics: GameMetrics,
    active_players_with_progress: BTreeSet<usize>,
}

#[derive(Clone, Debug, Default)]
struct GameProgressTracker {
    state_observations: u64,
    repeated_full_state_hashes: u64,
    seen_full_state_hashes: HashSet<u64>,
    current_round: Option<RoundProgressStart>,
    rounds: Vec<RoundProgressDiagnostic>,
    eliminations: Vec<EliminationDiagnostic>,
    consecutive_no_progress_rounds: u32,
    maximum_consecutive_no_progress_rounds: u32,
}

impl GameProgressTracker {
    fn observe_state(&mut self, hash: u64) {
        self.state_observations = self.state_observations.saturating_add(1);
        if !self.seen_full_state_hashes.insert(hash) {
            self.repeated_full_state_hashes = self.repeated_full_state_hashes.saturating_add(1);
        }
    }

    fn observe_turn(&mut self, turn: u32, metrics: &GameMetrics) {
        let round = turn.saturating_sub(1) / PLAYER_COUNT as u32 + 1;
        if self
            .current_round
            .as_ref()
            .is_some_and(|current| current.round != round)
        {
            self.finish_current_round(turn.saturating_sub(1), metrics);
        }
        let current = self
            .current_round
            .get_or_insert_with(|| RoundProgressStart {
                round,
                first_turn: turn,
                last_turn: turn,
                metrics: metrics.clone(),
                active_players_with_progress: BTreeSet::new(),
            });
        current.last_turn = turn;
    }

    fn record_meaningful_action(&mut self, active_seat: Option<usize>) {
        let Some(active_seat) = active_seat else {
            return;
        };
        if let Some(current) = self.current_round.as_mut() {
            current.active_players_with_progress.insert(active_seat);
        }
    }

    fn record_elimination(&mut self, seat: usize, turn: u32) {
        self.eliminations.push(EliminationDiagnostic { seat, turn });
    }

    fn finish_current_round(&mut self, last_turn: u32, metrics: &GameMetrics) {
        let Some(current) = self.current_round.take() else {
            return;
        };
        let table_damage_to_players = metrics
            .table_damage_to_players
            .saturating_sub(current.metrics.table_damage_to_players);
        let life_total_movement = metrics
            .life_total_movement
            .saturating_sub(current.metrics.life_total_movement);
        let casts = metrics.casts.saturating_sub(current.metrics.casts);
        let meaningful_actions = metrics
            .meaningful_actions
            .saturating_sub(current.metrics.meaningful_actions);
        let pass_only_priority_cycles = metrics
            .pass_only_priority_cycles
            .saturating_sub(current.metrics.pass_only_priority_cycles);
        let eliminations = metrics
            .eliminations
            .saturating_sub(current.metrics.eliminations);
        let no_progress = table_damage_to_players == 0
            && life_total_movement == 0
            && casts == 0
            && meaningful_actions == 0
            && eliminations == 0;
        if no_progress {
            self.consecutive_no_progress_rounds =
                self.consecutive_no_progress_rounds.saturating_add(1);
            self.maximum_consecutive_no_progress_rounds = self
                .maximum_consecutive_no_progress_rounds
                .max(self.consecutive_no_progress_rounds);
        } else {
            self.consecutive_no_progress_rounds = 0;
        }
        self.rounds.push(RoundProgressDiagnostic {
            round: current.round,
            first_turn: current.first_turn,
            last_turn: last_turn.max(current.last_turn),
            table_damage_to_players,
            life_total_movement,
            casts,
            meaningful_actions,
            pass_only_priority_cycles,
            active_players_with_progress: current.active_players_with_progress.len() as u32,
            eliminations,
            no_progress,
        });
    }

    fn finish(
        mut self,
        final_turn: u32,
        metrics: &GameMetrics,
        decisions: &[AiDecisionRecord],
    ) -> GameProgressDiagnostics {
        self.finish_current_round(final_turn, metrics);
        let mut seen_keys = HashSet::new();
        let mut decision_keys = 0_u64;
        let mut repeated_decision_state_keys = 0_u64;
        for decision in decisions {
            if decision.decision_state_key.is_empty() {
                continue;
            }
            decision_keys = decision_keys.saturating_add(1);
            if !seen_keys.insert(decision.decision_state_key.as_str()) {
                repeated_decision_state_keys = repeated_decision_state_keys.saturating_add(1);
            }
        }
        GameProgressDiagnostics {
            termination_reason: "winner".to_owned(),
            turn_cap_reached: false,
            state_observations: self.state_observations,
            repeated_full_state_hashes: self.repeated_full_state_hashes,
            repeated_full_state_hash_rate_ppm: ratio_ppm(
                self.repeated_full_state_hashes,
                self.state_observations,
            ),
            repeated_decision_state_keys,
            repeated_decision_state_key_rate_ppm: ratio_ppm(
                repeated_decision_state_keys,
                decision_keys,
            ),
            no_progress_rounds: self.rounds.iter().filter(|round| round.no_progress).count() as u32,
            maximum_consecutive_no_progress_rounds: self.maximum_consecutive_no_progress_rounds,
            eliminations: self.eliminations,
            rounds: self.rounds,
        }
    }
}

fn ratio_ppm(numerator: u64, denominator: u64) -> u32 {
    if denominator == 0 {
        return 0;
    }
    numerator
        .saturating_mul(1_000_000)
        .checked_div(denominator)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GameSummary {
    seed: u64,
    winner: usize,
    turns: u32,
    final_hash: u64,
    final_life: [i32; PLAYER_COUNT],
    metrics: GameMetrics,
    #[serde(default)]
    progress: GameProgressDiagnostics,
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
    benchmark_semantic_identity: String,
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
    benchmark_semantic_identity: String,
}

#[derive(Clone)]
struct PendingActivatedResolution {
    controller: PlayerId,
    runtime: ActivatedRuntime,
    target_requirements: Vec<TargetRequirement>,
    targets: Vec<TargetChoice>,
    target_legalities: Vec<bool>,
    decisions: StackDecisionBindings,
}

#[derive(Clone)]
struct PendingSpellResolution {
    controller: PlayerId,
    object: ObjectId,
    program: Arc<CardProgram>,
    target_requirements: Vec<TargetRequirement>,
    targets: Vec<TargetChoice>,
    target_legalities: Vec<bool>,
    decisions: StackDecisionBindings,
}

#[derive(Clone)]
struct PendingTriggeredResolution {
    controller: PlayerId,
    trigger: TriggerId,
    triggering_player: Option<PlayerId>,
    runtime: TriggerRuntime,
    target_requirements: Vec<TargetRequirement>,
    targets: Vec<TargetChoice>,
    target_legalities: Vec<bool>,
    decisions: StackDecisionBindings,
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
        targets: Vec<AnnouncedTarget>,
        optional: Vec<bool>,
    },
    BeginActivateProgramWithCosts {
        source: ObjectId,
        ability: ActivatedAbilityId,
        targets: Vec<AnnouncedTarget>,
        optional: Vec<bool>,
        sacrifice_objects: Option<Vec<ObjectId>>,
    },
    ActivateProgramWithCosts {
        source: ObjectId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
        targets: Vec<AnnouncedTarget>,
        optional: Vec<bool>,
        sacrifice_objects: Vec<ObjectId>,
    },
    BeginCast {
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: Vec<AnnouncedTarget>,
        mode: Option<u32>,
        optional: Vec<bool>,
        additional_costs: Vec<Vec<ObjectId>>,
    },
    NarrowCastX {
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: Vec<AnnouncedTarget>,
        mode: Option<u32>,
        optional: Vec<bool>,
        additional_costs: Vec<Vec<ObjectId>>,
        minimum: u32,
        maximum: u32,
    },
    ChooseCastX {
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: Vec<AnnouncedTarget>,
        mode: Option<u32>,
        optional: Vec<bool>,
        additional_costs: Vec<Vec<ObjectId>>,
        x_value: u32,
    },
    Cast {
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        payment: PaymentPlan,
        targets: Vec<AnnouncedTarget>,
        mode: Option<u32>,
        optional: Vec<bool>,
        additional_costs: Vec<Vec<ObjectId>>,
    },
    Finish,
}

type MainDecisionAdapter = (DecisionContext, Vec<(CanonicalActionId, MainChoice)>);

#[derive(Clone)]
struct SpellChoiceBinding {
    targets: Vec<AnnouncedTarget>,
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
    conditional_cast_triggers: HashMap<ObjectId, Vec<TriggerId>>,
    triggering_players_by_stack_entry: HashMap<StackEntryId, PlayerId>,
    activated_abilities: Vec<RegisteredAbility>,
    pending_spell_resolution: Option<PendingSpellResolution>,
    pending_activated_resolution: Option<PendingActivatedResolution>,
    pending_triggered_resolution: Option<PendingTriggeredResolution>,
    triggers_registered_for: HashSet<ObjectId>,
    permanent_runtime_registered_for: HashSet<ObjectId>,
    commander_zone_decisions: HashMap<ObjectId, ZoneId>,
    current_attacks: Vec<AttackDeclaration>,
    coverage_target: Option<String>,
    metrics: GameMetrics,
    progress: GameProgressTracker,
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
    actor: PlayerId,
    window: MainSearchWindow,
    decision_count: u32,
    context: Arc<DecisionContext>,
    mappings: Arc<Vec<(CanonicalActionId, MainChoice)>>,
    priors: Arc<HashMap<CanonicalActionId, i64>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainSearchWindow {
    Main,
    Priority,
}

fn multiplayer_backup_sign(
    root_actor: PlayerId,
    node_actor: PlayerId,
    paranoid_coalition: bool,
) -> i8 {
    if paranoid_coalition && node_actor != root_actor {
        -1
    } else {
        1
    }
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
            conditional_cast_triggers: HashMap::new(),
            triggering_players_by_stack_entry: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_spell_resolution: None,
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
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
                    episode: None,
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
            conditional_cast_triggers: self.conditional_cast_triggers.clone(),
            triggering_players_by_stack_entry: self.triggering_players_by_stack_entry.clone(),
            activated_abilities: self.activated_abilities.clone(),
            pending_spell_resolution: self.pending_spell_resolution.clone(),
            pending_activated_resolution: self.pending_activated_resolution.clone(),
            pending_triggered_resolution: self.pending_triggered_resolution.clone(),
            triggers_registered_for: self.triggers_registered_for.clone(),
            permanent_runtime_registered_for: self.permanent_runtime_registered_for.clone(),
            commander_zone_decisions: self.commander_zone_decisions.clone(),
            current_attacks: self.current_attacks.clone(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
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
            self.progress
                .observe_state(self.state.deterministic_hash().get());
            self.progress
                .observe_turn(self.state.turn_number(), &self.metrics);
            if self.pending_spell_resolution.is_some() {
                self.complete_pending_spell_resolution(
                    human,
                    &mut decisions,
                    ai_policies.as_ref(),
                )?;
                continue;
            }
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
        let progress = std::mem::take(&mut self.progress).finish(
            self.state.turn_number(),
            &self.metrics,
            &self.ai_decisions,
        );
        let summary = GameSummary {
            seed: self.seed,
            winner,
            turns: self.state.turn_number(),
            final_hash: self.state.deterministic_hash().get(),
            final_life,
            metrics: self.metrics,
            progress,
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
        let countered_entry = match &action {
            Action::CounterStackEntry { entry } => Some(*entry),
            _ => None,
        };
        let tracking_progress = self.progress.current_round.is_some();
        let meaningful_action = tracking_progress
            && !matches!(
                &action,
                Action::PassPriority { .. }
                    | Action::AdvanceStep
                    | Action::CheckStateBasedActions
                    | Action::StartTurn { .. }
                    | Action::RequestCleanupPriority
            );
        let noncombat_damage = tracking_progress && matches!(&action, Action::DealDamage { .. });
        let active_seat = self
            .state
            .active_player()
            .and_then(|active| self.players.iter().position(|player| *player == active));
        let life_before = self
            .state
            .players()
            .iter()
            .map(|player| player.life())
            .collect::<Vec<_>>();
        let lost_before = self
            .state
            .players()
            .iter()
            .map(|player| player.lost())
            .collect::<Vec<_>>();
        let trace_header = self
            .trace
            .enabled()
            .then(|| (format!("{action:?}"), self.state.deterministic_hash().get()));
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
        if meaningful_action {
            self.metrics.meaningful_actions = self.metrics.meaningful_actions.saturating_add(1);
            self.progress.record_meaningful_action(active_seat);
        }
        if matches!(outcome, Outcome::Priority(PriorityOutcome::StepComplete)) && tracking_progress
        {
            self.metrics.pass_only_priority_cycles =
                self.metrics.pass_only_priority_cycles.saturating_add(1);
        }
        let life_after = self
            .state
            .players()
            .iter()
            .map(|player| player.life())
            .collect::<Vec<_>>();
        if tracking_progress {
            let life_movement = life_before
                .iter()
                .zip(&life_after)
                .map(|(before, after)| u64::from(before.abs_diff(*after)))
                .sum::<u64>();
            self.metrics.life_total_movement = self
                .metrics
                .life_total_movement
                .saturating_add(life_movement);
            let table_damage = match &outcome {
                Outcome::CombatDamageAssigned(records) => records
                    .iter()
                    .filter(|record| matches!(record.target(), CombatDamageTarget::Player(_)))
                    .map(|record| u64::from(record.amount()))
                    .sum(),
                _ if noncombat_damage => life_before
                    .iter()
                    .zip(&life_after)
                    .map(|(before, after)| u64::from(before.saturating_sub(*after).max(0) as u32))
                    .sum(),
                _ => 0,
            };
            self.metrics.table_damage_to_players = self
                .metrics
                .table_damage_to_players
                .saturating_add(table_damage);
        }
        let mut eliminations = 0_u64;
        for (seat, (was_lost, player)) in lost_before.iter().zip(self.state.players()).enumerate() {
            if !*was_lost && player.lost() {
                eliminations = eliminations.saturating_add(1);
                if tracking_progress {
                    self.progress
                        .record_elimination(seat, self.state.turn_number());
                }
            }
        }
        self.metrics.eliminations = self.metrics.eliminations.saturating_add(eliminations);
        if let Some(entry) = countered_entry {
            self.handle_resolution(entry)?;
        }
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
        match self.prompt_context_selection(source, kind, context, options, false, None)? {
            CanonicalPromptSelection::Option(selected) => Ok(selected),
            CanonicalPromptSelection::RequestConcession => Err(format!(
                "seed {} prompt `{kind}` accepted concession outside a main or priority window",
                self.seed
            )),
        }
    }

    fn prompt_context_choice_in_episode(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
        episode: DecisionEpisodeMetadata,
    ) -> Result<CanonicalActionId, String> {
        match self.prompt_context_selection(source, kind, context, options, false, Some(episode))? {
            CanonicalPromptSelection::Option(selected) => Ok(selected),
            CanonicalPromptSelection::RequestConcession => Err(format!(
                "seed {} prompt `{kind}` accepted concession inside a decision episode",
                self.seed
            )),
        }
    }

    fn prompt_context_selection_in_episode(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
        allow_concession: bool,
        episode: DecisionEpisodeMetadata,
    ) -> Result<CanonicalPromptSelection, String> {
        self.prompt_context_selection(
            source,
            kind,
            context,
            options,
            allow_concession,
            Some(episode),
        )
    }

    fn prompt_context_selection(
        &self,
        source: &mut dyn DecisionSource,
        kind: &'static str,
        context: &DecisionContext,
        options: &[String],
        allow_concession: bool,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<CanonicalPromptSelection, String> {
        let view = self
            .state
            .player_view(context.actor())
            .map_err(|error| format!("seed {} player view failed: {error:?}", self.seed))?;
        let episode = episode.unwrap_or_else(|| {
            DecisionEpisodeMetadata::root(self.seed, source.decision_count(), context, true)
        });
        let selected = source.choose(&DecisionPrompt {
            kind,
            view: &view,
            context,
            options,
            allow_concession,
            episode,
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

    fn begin_controlled_episode(
        &self,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        context: &DecisionContext,
    ) -> Result<Option<DecisionEpisodePath>, String> {
        let ordinal = if human == Some(controller) {
            decisions
                .as_deref()
                .ok_or_else(|| "human game is missing a decision source".to_owned())?
                .decision_count()
        } else if ai_policies.is_some() {
            self.ai_decisions.len() as u64
        } else {
            return Ok(None);
        };
        Ok(Some(DecisionEpisodePath::root(self.seed, ordinal, context)))
    }

    fn complete_controlled_episode(
        &mut self,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        episode: Option<&DecisionEpisodePath>,
    ) -> Result<(), String> {
        let Some(episode) = episode else {
            return Ok(());
        };
        if human == Some(controller) {
            decisions
                .as_deref_mut()
                .ok_or_else(|| "human game is missing a decision source".to_owned())?
                .complete_episode(
                    &episode.metadata.decision_episode_id,
                    &episode.final_action_id(),
                )
        } else if ai_policies.is_some() {
            self.complete_ai_episode(episode)
        } else {
            Ok(())
        }
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
            let mut episode =
                DecisionEpisodePath::root(self.seed, source.decision_count(), &context);
            let labels = context
                .options()
                .iter()
                .map(|option| self.main_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id = match self.prompt_context_selection_in_episode(
                source,
                "Choose a main-phase action",
                &context,
                &labels,
                true,
                episode.root_metadata(),
            )? {
                CanonicalPromptSelection::Option(selected) => selected,
                CanonicalPromptSelection::RequestConcession => {
                    self.take_human_concession(player, source)?;
                    return Ok(());
                }
            };
            episode.record_selection(&context, selected_id, 0);
            let choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} human main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.finish_human_main_choice_in_episode(player, source, choice, Some(episode))? {
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
        let mut episode = DecisionEpisodePath::root(self.seed, source.decision_count(), &context);
        let labels = context
            .options()
            .iter()
            .map(|option| match option.descriptor() {
                DecisionDescriptor::PassPriority => Ok("Pass priority".to_owned()),
                descriptor => self.main_choice_label(descriptor),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selected_id = match self.prompt_context_selection_in_episode(
            source,
            "Choose a priority action",
            &context,
            &labels,
            true,
            episode.root_metadata(),
        )? {
            CanonicalPromptSelection::Option(selected) => selected,
            CanonicalPromptSelection::RequestConcession => {
                self.take_human_concession(player, source)?;
                return Ok(());
            }
        };
        episode.record_selection(&context, selected_id, 0);
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
            .ok_or_else(|| {
                format!(
                    "seed {} human priority action {selected_id} has no typed adapter",
                    self.seed
                )
            })?;
        self.finish_human_main_choice_in_episode(player, source, choice, Some(episode))?;
        Ok(())
    }

    #[cfg(test)]
    fn finish_human_main_choice(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
        choice: MainChoice,
    ) -> Result<bool, String> {
        self.finish_human_main_choice_in_episode(player, source, choice, None)
    }

    fn finish_human_main_choice_in_episode(
        &mut self,
        player: PlayerId,
        source: &mut dyn DecisionSource,
        mut choice: MainChoice,
        mut episode: Option<DecisionEpisodePath>,
    ) -> Result<bool, String> {
        while let Some((context, mappings)) = self.hierarchical_main_context(player, &choice)? {
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    source.decision_count(),
                    &context,
                ));
            }
            let episode_path = episode
                .as_ref()
                .ok_or_else(|| "human decision episode was not initialized".to_owned())?;
            let child_episode = if episode_path.selected_action_ids.is_empty() {
                episode_path.root_metadata()
            } else {
                episode_path.child_metadata(&context)
            };
            let parent_object = match &choice {
                MainChoice::BeginActivateProgramWithCosts { source, .. }
                | MainChoice::ActivateProgramWithCosts { source, .. } => *source,
                MainChoice::BeginCast { object, .. }
                | MainChoice::NarrowCastX { object, .. }
                | MainChoice::ChooseCastX { object, .. }
                | MainChoice::Cast { object, .. } => *object,
                _ => {
                    return Err(format!(
                        "seed {} hierarchical context has an unsupported parent",
                        self.seed
                    ));
                }
            };
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
                    DecisionDescriptor::ChooseAdditionalCost { cost, objects } => {
                        self.additional_cost_choice_label(parent_object, *cost, objects)
                    }
                    DecisionDescriptor::ChooseActivationCostObjects { objects } => Ok(format!(
                        "Sacrifice {}",
                        objects
                            .iter()
                            .map(|object| self.object_name(*object))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    descriptor => Err(format!(
                        "seed {} cannot label hierarchical descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let prompt = match context.kind() {
                DecisionKind::NumericValue => "Choose X",
                DecisionKind::Payment
                    if context.options().iter().all(|option| {
                        matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseAdditionalCost { .. }
                        )
                    }) =>
                {
                    "Choose an additional cost"
                }
                DecisionKind::Payment
                    if context.options().iter().all(|option| {
                        matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseActivationCostObjects { .. }
                        )
                    }) =>
                {
                    "Choose permanents to sacrifice"
                }
                DecisionKind::Payment => "Choose a mana payment",
                other => {
                    return Err(format!(
                        "seed {} unexpected hierarchical context {other:?}",
                        self.seed
                    ));
                }
            };
            let selected_id = self.prompt_context_choice_in_episode(
                source,
                prompt,
                &context,
                &labels,
                child_episode.clone(),
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(&context, selected_id, child_episode.path_depth);
            }
            choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} hierarchical action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
        }
        if let Some(episode) = episode.as_ref() {
            source.complete_episode(
                &episode.metadata.decision_episode_id,
                &episode.final_action_id(),
            )?;
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
            episode: DecisionEpisodeMetadata::root(
                self.seed,
                source.decision_count(),
                &context,
                true,
            ),
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
        let mut episode =
            DecisionEpisodePath::root(self.seed, self.ai_decisions.len() as u64, &context);
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
            episode: Some(episode.root_metadata()),
        });
        episode.record_selection(&context, selected_id, 0);
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
            .ok_or_else(|| {
                format!(
                    "seed {} AI priority action {selected_id} has no typed adapter",
                    self.seed
                )
            })?;
        self.finish_ai_main_choice_in_episode(player, policy, choice, Some(episode))?;
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
            DecisionDescriptor::BeginActivateProgramAbilityWithCosts {
                source,
                targets,
                optional,
                ..
            } => {
                let mut details = Vec::new();
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
                let suffix = if details.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", details.join("; "))
                };
                Ok(format!(
                    "Activate ability: {}{suffix}",
                    self.object_name(*source)
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
            } => self.deferred_cast_choice_label(*object, None, targets, modes, optional),
            DecisionDescriptor::BeginCastSpellAlternate {
                object,
                alternate,
                targets,
                modes,
                optional,
            } => self.deferred_cast_choice_label(
                *object,
                Some(core_alternate_to_runtime(*alternate)),
                targets,
                modes,
                optional,
            ),
            DecisionDescriptor::PassPriority => Ok("Finish main phase".to_owned()),
            other => Err(format!(
                "seed {} main prompt cannot label descriptor {other:?}",
                self.seed
            )),
        }
    }

    fn deferred_cast_choice_label(
        &self,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: &[TargetChoice],
        modes: &[u32],
        optional: &[bool],
    ) -> Result<String, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing deferred spell program", self.seed))?;
        let mut pending = Vec::new();
        if !program.additional_costs().is_empty() {
            pending.push("additional costs");
        }
        if self.cast_mana_cost(program, alternate)?.x_count() != 0 {
            pending.push("X");
        }
        pending.push("mana payment");
        let mut details = vec![format!("choose {}", pending.join(", then "))];
        if let Some(alternate) = alternate {
            details.push(format!("alternate cost {alternate:?}"));
        }
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
            self.object_name(object),
            details.join("; ")
        ))
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
                let (target_requirements, target_choices) =
                    expand_announced_targets(effect.target_requirements(), &targets).map_err(
                        |error| format!("seed {} activation targets failed: {error}", self.seed),
                    )?;
                let outcome = self.dispatch(Action::ActivateProgramAbility {
                    player,
                    ability,
                    payment,
                    target_requirements,
                    target_choices,
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
            MainChoice::ActivateProgramWithCosts {
                source,
                ability,
                payment,
                targets,
                optional,
                sacrifice_objects,
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
                let (target_requirements, target_choices) =
                    expand_announced_targets(effect.target_requirements(), &targets).map_err(
                        |error| format!("seed {} activation targets failed: {error}", self.seed),
                    )?;
                let outcome = self.dispatch(Action::ActivateProgramAbilityWithCosts {
                    player,
                    ability,
                    payment,
                    target_requirements,
                    target_choices,
                    decisions,
                    additional_cost_objects: sacrifice_objects,
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
            MainChoice::BeginActivateProgramWithCosts { .. } => Err(format!(
                "seed {} attempted to dispatch an incomplete hierarchical activation",
                self.seed
            )),
            MainChoice::BeginCast { .. }
            | MainChoice::NarrowCastX { .. }
            | MainChoice::ChooseCastX { .. } => Err(format!(
                "seed {} attempted to dispatch an incomplete hierarchical cast",
                self.seed
            )),
            MainChoice::Cast {
                object,
                alternate,
                payment,
                targets,
                mode,
                optional,
                additional_costs,
            } => {
                self.cast_program_with_choices(
                    player,
                    object,
                    alternate,
                    payment,
                    targets,
                    mode,
                    optional,
                    additional_costs,
                )?;
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
        alternate: Option<AlternateCostKind>,
    ) -> Result<Vec<SpellChoiceBinding>, String> {
        let mut bindings = Vec::new();
        if program.spell_modes().is_empty() {
            self.extend_spell_branch_bindings(
                player,
                object,
                program.name(),
                None,
                program.target_requirements_for_alternate(alternate),
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
        optional_count: usize,
        output: &mut Vec<SpellChoiceBinding>,
    ) -> Result<(), String> {
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

    fn spell_additional_cost_selections(
        &self,
        player: PlayerId,
        spell: ObjectId,
        program: &CardProgram,
        prior: &[Vec<ObjectId>],
    ) -> Result<Vec<Vec<ObjectId>>, String> {
        let Some(cost) = program.additional_costs().get(prior.len()).copied() else {
            return Ok(Vec::new());
        };
        let used = prior.iter().flatten().copied().collect::<HashSet<_>>();
        let (count, mut candidates) = match cost {
            SpellAdditionalCostProgram::DiscardCards { count } => {
                let hand = ZoneId::new(Some(player), ZoneKind::Hand);
                let candidates = self
                    .state
                    .zone_objects(hand)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .filter(|object| *object != spell && !used.contains(object))
                    .collect::<Vec<_>>();
                (count, candidates)
            }
            SpellAdditionalCostProgram::SacrificePermanents { count, predicate } => {
                let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
                let candidates = self
                    .state
                    .zone_objects(battlefield)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .filter(|object| !used.contains(object))
                    .filter(|object| self.state.object_controller(*object) == Ok(player))
                    .filter(|object| {
                        self.state
                            .object_matches_target_predicate(player, predicate, *object)
                    })
                    .collect::<Vec<_>>();
                (count, candidates)
            }
        };
        candidates.sort_by_key(|object| object.index());
        let count = usize::try_from(count)
            .map_err(|_| format!("seed {} additional-cost count overflow", self.seed))?;
        bounded_object_combinations(&candidates, count, count, MAX_CANONICAL_SPELL_OPTIONS)
    }

    fn spell_additional_cost_first_completion(
        &self,
        player: PlayerId,
        spell: ObjectId,
        program: &CardProgram,
        prior: &mut Vec<Vec<ObjectId>>,
    ) -> Result<Option<Vec<Vec<ObjectId>>>, String> {
        if prior.len() == program.additional_costs().len() {
            return Ok(Some(prior.clone()));
        }
        for selection in self.spell_additional_cost_selections(player, spell, program, prior)? {
            prior.push(selection);
            let complete =
                self.spell_additional_cost_first_completion(player, spell, program, prior)?;
            prior.pop();
            if complete.is_some() {
                return Ok(complete);
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn additional_cast_cost_context(
        &self,
        player: PlayerId,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: &[AnnouncedTarget],
        mode: Option<u32>,
        optional: &[bool],
        prior: &[Vec<ObjectId>],
    ) -> Result<MainDecisionAdapter, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing additional-cost spell program", self.seed))?;
        if prior.len() >= program.additional_costs().len() {
            return Err(format!(
                "seed {} spell {} has no additional cost at slot {}",
                self.seed,
                program.name(),
                prior.len()
            ));
        }
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for selection in self.spell_additional_cost_selections(player, object, program, prior)? {
            let mut selections = prior.to_vec();
            selections.push(selection);
            let mut completion = selections.clone();
            if self
                .spell_additional_cost_first_completion(player, object, program, &mut completion)?
                .is_none()
            {
                continue;
            }
            let option = DecisionOption::new(
                DecisionDescriptor::ChooseAdditionalCost {
                    cost: u32::try_from(prior.len()).map_err(|_| {
                        format!("seed {} additional-cost index overflow", self.seed)
                    })?,
                    objects: selections.last().cloned().ok_or_else(|| {
                        format!("seed {} additional-cost selection disappeared", self.seed)
                    })?,
                },
                Vec::new(),
            );
            mappings.push((
                option.id(),
                MainChoice::BeginCast {
                    object,
                    alternate,
                    targets: targets.to_vec(),
                    mode,
                    optional: optional.to_vec(),
                    additional_costs: selections,
                },
            ));
            options.push(option);
        }
        if options.is_empty() {
            return Err(format!(
                "seed {} spell {} has no complete additional-cost payment",
                self.seed,
                program.name()
            ));
        }
        let context = self.scoped_decision_context(
            DecisionKind::Payment,
            player,
            options,
            additional_cast_path_discriminator(
                player, object, alternate, targets, mode, optional, prior,
            ),
        )?;
        Ok((context, mappings))
    }

    fn additional_cost_choice_label(
        &self,
        object: ObjectId,
        index: u32,
        objects: &[ObjectId],
    ) -> Result<String, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing additional-cost spell program", self.seed))?;
        let index = usize::try_from(index)
            .map_err(|_| format!("seed {} additional-cost index overflow", self.seed))?;
        let cost = program.additional_costs().get(index).ok_or_else(|| {
            format!(
                "seed {} additional-cost choice index {index} overflow",
                self.seed
            )
        })?;
        let names = objects
            .iter()
            .map(|selected| self.object_name(*selected))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(match cost {
            SpellAdditionalCostProgram::DiscardCards { .. } => format!("Discard {names}"),
            SpellAdditionalCostProgram::SacrificePermanents { .. } => {
                format!("Sacrifice {names}")
            }
        })
    }

    fn additional_cost_choice_prior(&self, descriptor: &DecisionDescriptor) -> i64 {
        let objects = match descriptor {
            DecisionDescriptor::ChooseAdditionalCost { objects, .. }
            | DecisionDescriptor::ChooseActivationCostObjects { objects } => objects,
            _ => return 0,
        };
        -objects
            .iter()
            .map(|object| {
                self.state.object(*object).map_or(0_i64, |record| {
                    i64::from(record.base_object().mana_value())
                        .saturating_mul(16)
                        .saturating_add(if record.is_commander() { 10_000 } else { 0 })
                })
            })
            .sum::<i64>()
    }

    fn activation_sacrifice_selections(
        &self,
        player: PlayerId,
        source: ObjectId,
        ability: ActivatedAbilityId,
        cost: ActivationCost,
    ) -> Result<Vec<Vec<ObjectId>>, String> {
        let Some((predicate, count)) = self
            .state
            .activation_sacrifice_cost(ability)
            .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?
        else {
            return Ok(vec![Vec::new()]);
        };
        let mut candidates = self
            .state
            .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|object| !cost.sacrifice_source() || *object != source)
            .filter(|object| self.state.object_controller(*object) == Ok(player))
            .filter(|object| {
                self.state
                    .object_matches_target_predicate(player, predicate, *object)
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|object| object.index());
        let count = usize::try_from(count)
            .map_err(|_| format!("seed {} activation sacrifice count overflow", self.seed))?;
        bounded_object_combinations(&candidates, count, count, MAX_CANONICAL_SPELL_OPTIONS)
    }

    fn program_activation_action_with_costs(
        &self,
        player: PlayerId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
        targets: &[AnnouncedTarget],
        optional: &[bool],
        sacrifice_objects: &[ObjectId],
    ) -> Result<Action, String> {
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
        let decisions = StackDecisionBindings::new(None, optional)
            .map_err(|error| format!("seed {} activation choices failed: {error:?}", self.seed))?;
        let (target_requirements, target_choices) =
            expand_announced_targets(effect.target_requirements(), targets).map_err(|error| {
                format!("seed {} activation targets failed: {error}", self.seed)
            })?;
        Ok(Action::ActivateProgramAbilityWithCosts {
            player,
            ability,
            payment,
            target_requirements,
            target_choices,
            decisions,
            additional_cost_objects: sacrifice_objects.to_vec(),
        })
    }

    fn activation_cost_has_completion(
        &self,
        player: PlayerId,
        source: ObjectId,
        ability: ActivatedAbilityId,
        targets: &[AnnouncedTarget],
        optional: &[bool],
    ) -> Result<bool, String> {
        let cost = self
            .state
            .effective_activation_cost(ability)
            .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
        let payments = self
            .state
            .payment_plans_for_player(player, cost.mana())
            .map_err(|error| format!("seed {} payment enumeration failed: {error:?}", self.seed))?;
        for sacrifice_objects in
            self.activation_sacrifice_selections(player, source, ability, cost)?
        {
            for payment in payments.plans().iter().copied() {
                let action = self.program_activation_action_with_costs(
                    player,
                    ability,
                    payment,
                    targets,
                    optional,
                    &sacrifice_objects,
                )?;
                if self.action_is_legal(&action) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn target_bindings(
        &self,
        player: PlayerId,
        source: ObjectId,
        requirements: &[TargetRequirement],
    ) -> Result<Vec<Vec<AnnouncedTarget>>, String> {
        let mut bindings = vec![Vec::new()];
        for (group_index, requirement) in requirements.iter().copied().enumerate() {
            let group = u8::try_from(group_index)
                .map_err(|_| format!("seed {} target group index overflow", self.seed))?;
            if requirement
                .group()
                .is_some_and(|compiled| compiled != group)
            {
                return Err(format!(
                    "seed {} target group {group_index} has inconsistent compiled identity",
                    self.seed
                ));
            }
            let choices = self.legal_targets_for(player, source, requirement);
            let minimum = if requirement.allocation_total().is_some() {
                usize::from(requirement.minimum()).max(1)
            } else {
                usize::from(requirement.minimum())
            };
            let maximum = usize::from(requirement.maximum()).min(choices.len());
            if minimum > maximum {
                return Ok(Vec::new());
            }
            let combinations = bounded_target_combinations(
                &choices,
                minimum,
                maximum,
                MAX_CANONICAL_SPELL_OPTIONS,
            )?;
            let mut group_bindings = Vec::new();
            for combination in combinations {
                if let Some(total) = requirement.allocation_total() {
                    if combination.is_empty() {
                        continue;
                    }
                    for allocation in bounded_positive_allocations(
                        total,
                        combination.len(),
                        MAX_CANONICAL_SPELL_OPTIONS,
                    )? {
                        group_bindings.push(
                            combination
                                .iter()
                                .copied()
                                .zip(allocation)
                                .map(|(target, amount)| {
                                    AnnouncedTarget::new(group, target).with_allocation(amount)
                                })
                                .collect::<Vec<_>>(),
                        );
                    }
                } else {
                    group_bindings.push(
                        combination
                            .into_iter()
                            .map(|target| AnnouncedTarget::new(group, target))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            if group_bindings.is_empty() {
                return Ok(Vec::new());
            }
            let next_len = bindings
                .len()
                .checked_mul(group_bindings.len())
                .ok_or_else(|| format!("seed {} target option count overflow", self.seed))?;
            if next_len > MAX_CANONICAL_SPELL_OPTIONS {
                return Err(format!(
                    "seed {} target choices exceed the {}-option canonical cap",
                    self.seed, MAX_CANONICAL_SPELL_OPTIONS
                ));
            }
            let mut next = Vec::with_capacity(next_len);
            for prefix in &bindings {
                for group_binding in &group_bindings {
                    let mut binding = prefix.clone();
                    binding.extend(group_binding.iter().copied());
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

    fn cast_mana_cost(
        &self,
        program: &CardProgram,
        alternate: Option<AlternateCostKind>,
    ) -> Result<forge_core::ManaCost, String> {
        let Some(alternate) = alternate else {
            return Ok(program.mana_cost());
        };
        program
            .alternate_costs()
            .iter()
            .copied()
            .find(|cost| cost.kind() == alternate)
            .map(|cost| cost.mana_cost())
            .ok_or_else(|| {
                format!(
                    "seed {} spell {} does not compile alternate cost {alternate:?}",
                    self.seed,
                    program.name()
                )
            })
    }

    fn cast_payment_plans(
        &self,
        player: PlayerId,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        x_value: u32,
    ) -> Result<Vec<PaymentPlan>, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing cast program", self.seed))?;
        let printed_cost = self.cast_mana_cost(program, alternate)?;
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
        alternate: Option<AlternateCostKind>,
    ) -> Result<Option<u32>, String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing X spell program", self.seed))?;
        let x_count = self.cast_mana_cost(program, alternate)?.x_count();
        if x_count == 0 {
            return Err(format!(
                "seed {} requested X range for a fixed cost",
                self.seed
            ));
        }
        if self
            .cast_payment_plans(player, object, alternate, 0)?
            .is_empty()
        {
            return Ok(None);
        }

        let mut minimum = 0_u32;
        let mut maximum = u32::MAX / x_count;
        while minimum < maximum {
            let midpoint = minimum + (maximum - minimum).div_ceil(2);
            if self
                .cast_payment_plans(player, object, alternate, midpoint)?
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
            candidates.push((commander, None));
        }
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let mut hand_objects = self
            .state
            .zone_objects(hand)
            .ok_or_else(|| format!("seed {} missing hand zone", self.seed))?
            .to_vec();
        hand_objects.sort_by_key(|object| object.index());
        candidates.extend(hand_objects.iter().copied().map(|object| (object, None)));

        let graveyard = ZoneId::new(Some(player), ZoneKind::Graveyard);
        let mut alternate_objects = hand_objects;
        alternate_objects.extend(
            self.state
                .zone_objects(graveyard)
                .ok_or_else(|| format!("seed {} missing graveyard zone", self.seed))?
                .iter()
                .copied(),
        );
        alternate_objects.sort_by_key(|object| object.index());
        alternate_objects.dedup();
        for object in alternate_objects {
            let Some(program) = self.programs.get(&object) else {
                continue;
            };
            for alternate in program.alternate_costs().iter().copied() {
                let source_zone_is_valid = match alternate.kind() {
                    AlternateCostKind::Flashback => {
                        self.state.object_zone(object) == Some(graveyard)
                    }
                    AlternateCostKind::Commander
                    | AlternateCostKind::Evoke
                    | AlternateCostKind::Overload => self.state.object_zone(object) == Some(hand),
                };
                if source_zone_is_valid && alternate.is_available(&self.state, player, Some(object))
                {
                    candidates.push((object, Some(alternate.kind())));
                }
            }
        }

        let mut choices = Vec::new();
        for (object, alternate) in candidates {
            let Some(program) = self.programs.get(&object) else {
                continue;
            };
            if !self.normal_spell_timing_available(player, program.kind()) {
                continue;
            }
            let spell_bindings = self.spell_choice_bindings(player, object, program, alternate)?;
            let mut no_additional_costs = Vec::new();
            let Some(first_additional_costs) = self.spell_additional_cost_first_completion(
                player,
                object,
                program,
                &mut no_additional_costs,
            )?
            else {
                continue;
            };
            let payment_options = self.cast_payment_plans(player, object, alternate, 0)?;
            if payment_options.is_empty() {
                continue;
            }
            let cast_cost = self.cast_mana_cost(program, alternate)?;
            let requires_hierarchy = alternate.is_some()
                || cast_cost.x_count() != 0
                || !program.additional_costs().is_empty();
            if requires_hierarchy {
                for binding in spell_bindings {
                    let is_legal = payment_options.iter().copied().any(|payment| {
                        self.spell_request(
                            program,
                            alternate,
                            payment,
                            &binding.targets,
                            binding.mode,
                            &binding.optional,
                            &first_additional_costs,
                        )
                        .is_ok_and(|request| {
                            self.action_is_legal(&Action::CastSpell {
                                player,
                                object,
                                request,
                            })
                        })
                    });
                    if !is_legal {
                        continue;
                    }
                    choices.push(MainChoice::BeginCast {
                        object,
                        alternate,
                        targets: binding.targets,
                        mode: binding.mode,
                        optional: binding.optional,
                        additional_costs: Vec::new(),
                    });
                }
                continue;
            }

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
                        alternate,
                        payment,
                        &binding.targets,
                        binding.mode,
                        &binding.optional,
                        &[],
                    )?;
                    let action = Action::CastSpell {
                        player,
                        object,
                        request,
                    };
                    if !self.action_is_legal(&action) {
                        continue;
                    }
                    choices.push(MainChoice::Cast {
                        object,
                        alternate,
                        payment,
                        targets: binding.targets.clone(),
                        mode: binding.mode,
                        optional: binding.optional.clone(),
                        additional_costs: Vec::new(),
                    });
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
            let has_extra_costs = cost.life() != 0
                || self
                    .state
                    .activation_sacrifice_cost(registered.id)
                    .map_err(|error| {
                        format!("seed {} activation cost failed: {error:?}", self.seed)
                    })?
                    .is_some();
            let branch_count = targets
                .len()
                .checked_mul(optionals.len())
                .and_then(|count| {
                    count.checked_mul(if has_extra_costs {
                        1
                    } else {
                        payments.plans().len()
                    })
                })
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
                    if has_extra_costs {
                        if self.activation_cost_has_completion(
                            player,
                            registered.source,
                            registered.id,
                            &target_binding,
                            optional,
                        )? {
                            choices.push(MainChoice::BeginActivateProgramWithCosts {
                                source: registered.source,
                                ability: registered.id,
                                targets: target_binding.clone(),
                                optional: optional.clone(),
                                sacrifice_objects: None,
                            });
                        }
                        continue;
                    }
                    let decisions =
                        StackDecisionBindings::new(None, optional).map_err(|error| {
                            format!("seed {} activation choices failed: {error:?}", self.seed)
                        })?;
                    let (target_requirements, target_choices) =
                        expand_announced_targets(ability.target_requirements(), &target_binding)
                            .map_err(|error| {
                                format!("seed {} activation targets failed: {error}", self.seed)
                            })?;
                    for payment in payments.plans().iter().copied() {
                        let action = Action::ActivateProgramAbility {
                            player,
                            ability: registered.id,
                            payment,
                            target_requirements: target_requirements.clone(),
                            target_choices: target_choices.clone(),
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

    fn spell_additional_cost_payments(
        &self,
        program: &CardProgram,
        selections: &[Vec<ObjectId>],
    ) -> Result<Vec<SpellAdditionalCostPayment>, String> {
        if selections.len() != program.additional_costs().len() {
            return Err(format!(
                "seed {} spell {} supplied {} additional-cost groups for {} requirements",
                self.seed,
                program.name(),
                selections.len(),
                program.additional_costs().len()
            ));
        }
        program
            .additional_costs()
            .iter()
            .copied()
            .zip(selections)
            .enumerate()
            .map(|(index, (cost, objects))| match cost {
                SpellAdditionalCostProgram::DiscardCards { count } => {
                    if objects.len() != count as usize {
                        return Err(format!(
                            "seed {} spell {} additional cost {index} selected {} cards for discard {count}",
                            self.seed,
                            program.name(),
                            objects.len()
                        ));
                    }
                    Ok(SpellAdditionalCostPayment::DiscardCards {
                        objects: objects.clone(),
                    })
                }
                SpellAdditionalCostProgram::SacrificePermanents { count, predicate } => {
                    if objects.len() != count as usize {
                        return Err(format!(
                            "seed {} spell {} additional cost {index} selected {} permanents for sacrifice {count}",
                            self.seed,
                            program.name(),
                            objects.len()
                        ));
                    }
                    Ok(SpellAdditionalCostPayment::SacrificePermanents {
                        objects: objects.clone(),
                        predicate: Box::new(predicate),
                    })
                }
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn spell_request(
        &self,
        program: &CardProgram,
        alternate: Option<AlternateCostKind>,
        payment: PaymentPlan,
        targets: &[AnnouncedTarget],
        mode: Option<u32>,
        optional: &[bool],
        additional_costs: &[Vec<ObjectId>],
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
            None if program.spell_modes().is_empty() => {
                program.target_requirements_for_alternate(alternate)
            }
            None => {
                return Err(format!(
                    "seed {} modal spell {} has no mode binding",
                    self.seed,
                    program.name()
                ));
            }
        };
        let mut decisions = StackDecisionBindings::new(mode, optional)
            .map_err(|error| format!("seed {} stack choices failed: {error:?}", self.seed))?;
        if let Some(alternate) = alternate {
            decisions = decisions.with_alternate_cost(runtime_alternate_to_core(alternate));
        }
        let printed_cost = self.cast_mana_cost(program, alternate)?;
        let announced_cost = printed_cost.with_x(printed_cost.x_count(), payment.x_value());
        let additional_costs = self.spell_additional_cost_payments(program, additional_costs)?;
        let (target_requirements, target_choices) = expand_announced_targets(requirements, targets)
            .map_err(|error| {
                format!(
                    "seed {} spell target announcement failed: {error}",
                    self.seed
                )
            })?;
        let mut request = CastSpellRequest::new(kind, timing, announced_cost, payment)
            .with_targets(target_requirements, target_choices)
            .with_additional_costs(additional_costs)
            .with_decisions(decisions);
        if program.split_second() {
            request = request.with_split_second();
        }
        if alternate == Some(AlternateCostKind::Flashback) {
            request = request.with_flashback(announced_cost);
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
                | MainChoice::ActivateProgram { source, .. }
                | MainChoice::BeginActivateProgramWithCosts { source, .. } => *source,
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
                || !program.additional_costs().is_empty()
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
                    alternate: None,
                    payment,
                    targets: Vec::new(),
                    mode: None,
                    optional: Vec::new(),
                    additional_costs: Vec::new(),
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
            let mut episode =
                DecisionEpisodePath::root(self.seed, self.ai_decisions.len() as u64, &context);
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
                    Some(episode.root_metadata()),
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
                    episode: Some(episode.root_metadata()),
                });
            }
            episode.record_selection(&context, selected_id, 0);
            let choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} AI main action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
            if self.finish_ai_main_choice_in_episode(player, policy, choice, Some(episode))? {
                return Ok(());
            }
        }
    }

    #[cfg(test)]
    fn finish_ai_main_choice(
        &mut self,
        player: PlayerId,
        policy: AiController,
        choice: MainChoice,
    ) -> Result<bool, String> {
        self.finish_ai_main_choice_in_episode(player, policy, choice, None)
    }

    fn finish_ai_main_choice_in_episode(
        &mut self,
        player: PlayerId,
        policy: AiController,
        mut choice: MainChoice,
        mut episode: Option<DecisionEpisodePath>,
    ) -> Result<bool, String> {
        while let Some((context, mappings)) = self.hierarchical_main_context(player, &choice)? {
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    self.ai_decisions.len() as u64,
                    &context,
                ));
            }
            let episode_path = episode
                .as_ref()
                .ok_or_else(|| "AI decision episode was not initialized".to_owned())?;
            let child_episode = if episode_path.selected_action_ids.is_empty() {
                episode_path.root_metadata()
            } else {
                episode_path.child_metadata(&context)
            };
            let decision_started = Instant::now();
            let kind = match context.kind() {
                DecisionKind::NumericValue => "numeric_value",
                DecisionKind::Payment
                    if context.options().iter().all(|option| {
                        matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseAdditionalCost { .. }
                        )
                    }) =>
                {
                    "additional_cost"
                }
                DecisionKind::Payment
                    if context.options().iter().all(|option| {
                        matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseActivationCostObjects { .. }
                        )
                    }) =>
                {
                    "activation_cost"
                }
                DecisionKind::Payment => "payment",
                other => {
                    return Err(format!(
                        "seed {} unexpected AI hierarchical context {other:?}",
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
                        DecisionDescriptor::ChooseAdditionalCost { .. }
                        | DecisionDescriptor::ChooseActivationCostObjects { .. } => {
                            self.additional_cost_choice_prior(option.descriptor())
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
                    "seed {} AI selected illegal hierarchical action: {error}",
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
                episode: Some(child_episode.clone()),
            });
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(&context, selected_id, child_episode.path_depth);
            }
            choice = mappings
                .iter()
                .find_map(|(id, choice)| (*id == selected_id).then(|| choice.clone()))
                .ok_or_else(|| {
                    format!(
                        "seed {} AI hierarchical action {selected_id} has no typed adapter",
                        self.seed
                    )
                })?;
        }
        if let Some(episode) = episode.as_ref() {
            self.complete_ai_episode(episode)?;
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
                let descriptor_targets = announced_target_choices(&targets);
                let (target_requirements, target_choices) =
                    expand_announced_targets(effect.target_requirements(), &targets).map_err(
                        |error| format!("seed {} activation targets failed: {error}", self.seed),
                    )?;
                let descriptor =
                    if has_grouped_target_semantics(effect.target_requirements(), &targets) {
                        DecisionDescriptor::ActivateProgramAbilityTargetGroups {
                            source,
                            ability,
                            payment,
                            targets: targets.clone(),
                            optional: optional.clone(),
                        }
                    } else {
                        DecisionDescriptor::ActivateProgramAbility {
                            source,
                            ability,
                            payment,
                            targets: descriptor_targets,
                            optional: optional.clone(),
                        }
                    };
                Ok(DecisionOption::new(
                    descriptor,
                    vec![Action::ActivateProgramAbility {
                        player,
                        ability,
                        payment,
                        target_requirements,
                        target_choices,
                        decisions,
                    }],
                ))
            }
            MainChoice::BeginActivateProgramWithCosts {
                source,
                ability,
                targets,
                optional,
                sacrifice_objects: None,
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
                Ok(DecisionOption::new(
                    if has_grouped_target_semantics(effect.target_requirements(), &targets) {
                        DecisionDescriptor::BeginActivateProgramAbilityWithCostsTargetGroups {
                            source,
                            ability,
                            targets,
                            optional,
                        }
                    } else {
                        DecisionDescriptor::BeginActivateProgramAbilityWithCosts {
                            source,
                            ability,
                            targets: announced_target_choices(&targets),
                            optional,
                        }
                    },
                    Vec::new(),
                ))
            }
            MainChoice::BeginActivateProgramWithCosts { .. }
            | MainChoice::ActivateProgramWithCosts { .. } => Err(format!(
                "seed {} hierarchical activation stage cannot enter a root context",
                self.seed
            )),
            MainChoice::BeginCast {
                object,
                alternate,
                targets,
                mode,
                optional,
                ..
            } => {
                let program = self.programs.get(&object).ok_or_else(|| {
                    format!("seed {} missing program for AI cast option", self.seed)
                })?;
                let requirements = match mode {
                    Some(mode) => program
                        .spell_modes()
                        .get(mode as usize)
                        .ok_or_else(|| format!("seed {} invalid spell mode {mode}", self.seed))?
                        .target_requirements(),
                    None if program.spell_modes().is_empty() => {
                        program.target_requirements_for_alternate(alternate)
                    }
                    None => {
                        return Err(format!(
                            "seed {} modal spell {} has no mode binding",
                            self.seed,
                            program.name()
                        ));
                    }
                };
                let grouped = has_grouped_target_semantics(requirements, &targets);
                let modes = mode.into_iter().collect();
                let descriptor = match (alternate, grouped) {
                    (None, false) => DecisionDescriptor::BeginCastSpell {
                        object,
                        targets: announced_target_choices(&targets),
                        modes,
                        optional,
                    },
                    (None, true) => DecisionDescriptor::BeginCastSpellTargetGroups {
                        object,
                        targets,
                        modes,
                        optional,
                    },
                    (Some(alternate), false) => DecisionDescriptor::BeginCastSpellAlternate {
                        object,
                        alternate: runtime_alternate_to_core(alternate),
                        targets: announced_target_choices(&targets),
                        modes,
                        optional,
                    },
                    (Some(alternate), true) => {
                        DecisionDescriptor::BeginCastSpellAlternateTargetGroups {
                            object,
                            alternate: runtime_alternate_to_core(alternate),
                            targets,
                            modes,
                            optional,
                        }
                    }
                };
                Ok(DecisionOption::new(descriptor, Vec::new()))
            }
            MainChoice::NarrowCastX { .. } | MainChoice::ChooseCastX { .. } => Err(format!(
                "seed {} hierarchical cast stage cannot enter a root context",
                self.seed
            )),
            MainChoice::Cast {
                object,
                alternate,
                payment,
                targets,
                mode,
                optional,
                additional_costs,
            } => {
                if alternate.is_some() {
                    return Err(format!(
                        "seed {} alternate cast bypassed its canonical root hierarchy",
                        self.seed
                    ));
                }
                let program = self.programs.get(&object).ok_or_else(|| {
                    format!("seed {} missing program for AI cast option", self.seed)
                })?;
                let request = self.spell_request(
                    program,
                    alternate,
                    payment,
                    &targets,
                    mode,
                    &optional,
                    &additional_costs,
                )?;
                let requirements = match mode {
                    Some(mode) => program
                        .spell_modes()
                        .get(mode as usize)
                        .ok_or_else(|| format!("seed {} invalid spell mode {mode}", self.seed))?
                        .target_requirements(),
                    None if program.spell_modes().is_empty() => {
                        program.target_requirements_for_alternate(alternate)
                    }
                    None => {
                        return Err(format!(
                            "seed {} modal spell {} has no mode binding",
                            self.seed,
                            program.name()
                        ));
                    }
                };
                let descriptor = if has_grouped_target_semantics(requirements, &targets) {
                    DecisionDescriptor::CastSpellTargetGroups {
                        object,
                        payment,
                        targets,
                        modes: mode.into_iter().collect(),
                        optional,
                    }
                } else {
                    DecisionDescriptor::CastSpell {
                        object,
                        payment,
                        targets: announced_target_choices(&targets),
                        modes: mode.into_iter().collect(),
                        optional,
                    }
                };
                Ok(DecisionOption::new(
                    descriptor,
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

    fn activation_sacrifice_context(
        &self,
        player: PlayerId,
        source: ObjectId,
        ability: ActivatedAbilityId,
        targets: &[AnnouncedTarget],
        optional: &[bool],
    ) -> Result<MainDecisionAdapter, String> {
        let cost = self
            .state
            .effective_activation_cost(ability)
            .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
        if self
            .state
            .activation_sacrifice_cost(ability)
            .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?
            .is_none()
        {
            return Err(format!(
                "seed {} activation {} has no selected-object cost",
                self.seed,
                ability.get()
            ));
        }
        let payments = self
            .state
            .payment_plans_for_player(player, cost.mana())
            .map_err(|error| format!("seed {} payment enumeration failed: {error:?}", self.seed))?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for sacrifice_objects in
            self.activation_sacrifice_selections(player, source, ability, cost)?
        {
            let mut legal_completion = false;
            for payment in payments.plans().iter().copied() {
                let action = self.program_activation_action_with_costs(
                    player,
                    ability,
                    payment,
                    targets,
                    optional,
                    &sacrifice_objects,
                )?;
                if self.action_is_legal(&action) {
                    legal_completion = true;
                    break;
                }
            }
            if !legal_completion {
                continue;
            }
            let option = DecisionOption::new(
                DecisionDescriptor::ChooseActivationCostObjects {
                    objects: sacrifice_objects.clone(),
                },
                Vec::new(),
            );
            mappings.push((
                option.id(),
                MainChoice::BeginActivateProgramWithCosts {
                    source,
                    ability,
                    targets: targets.to_vec(),
                    optional: optional.to_vec(),
                    sacrifice_objects: Some(sacrifice_objects),
                },
            ));
            options.push(option);
        }
        if options.is_empty() {
            return Err(format!(
                "seed {} activation {} has no legal sacrifice payment",
                self.seed,
                ability.get()
            ));
        }
        let context = self.scoped_decision_context(
            DecisionKind::Payment,
            player,
            options,
            activation_cost_path_discriminator(player, source, ability, targets, optional, None, 0),
        )?;
        Ok((context, mappings))
    }

    fn activation_payment_context(
        &self,
        player: PlayerId,
        source: ObjectId,
        ability: ActivatedAbilityId,
        targets: &[AnnouncedTarget],
        optional: &[bool],
        sacrifice_objects: &[ObjectId],
    ) -> Result<MainDecisionAdapter, String> {
        let cost = self
            .state
            .effective_activation_cost(ability)
            .map_err(|error| format!("seed {} activation cost failed: {error:?}", self.seed))?;
        let payments = self
            .state
            .payment_plans_for_player(player, cost.mana())
            .map_err(|error| format!("seed {} payment enumeration failed: {error:?}", self.seed))?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for payment in payments.plans().iter().copied() {
            let action = self.program_activation_action_with_costs(
                player,
                ability,
                payment,
                targets,
                optional,
                sacrifice_objects,
            )?;
            if !self.action_is_legal(&action) {
                continue;
            }
            let option =
                DecisionOption::new(DecisionDescriptor::ChoosePayment { payment }, vec![action]);
            mappings.push((
                option.id(),
                MainChoice::ActivateProgramWithCosts {
                    source,
                    ability,
                    payment,
                    targets: targets.to_vec(),
                    optional: optional.to_vec(),
                    sacrifice_objects: sacrifice_objects.to_vec(),
                },
            ));
            options.push(option);
        }
        if options.is_empty() {
            return Err(format!(
                "seed {} activation {} has no legal mana payment",
                self.seed,
                ability.get()
            ));
        }
        let context = self.scoped_decision_context(
            DecisionKind::Payment,
            player,
            options,
            activation_cost_path_discriminator(
                player,
                source,
                ability,
                targets,
                optional,
                Some(sacrifice_objects),
                1,
            ),
        )?;
        Ok((context, mappings))
    }

    #[allow(clippy::too_many_arguments)]
    fn variable_cast_numeric_context(
        &self,
        player: PlayerId,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: &[AnnouncedTarget],
        mode: Option<u32>,
        optional: &[bool],
        additional_costs: &[Vec<ObjectId>],
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
                        alternate,
                        targets: targets.to_vec(),
                        mode,
                        optional: optional.to_vec(),
                        additional_costs: additional_costs.to_vec(),
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
                        alternate,
                        targets: targets.to_vec(),
                        mode,
                        optional: optional.to_vec(),
                        additional_costs: additional_costs.to_vec(),
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
                player,
                object,
                alternate,
                targets,
                mode,
                optional,
                additional_costs,
                0,
                bounds.0,
                bounds.1,
            ),
        )?;
        Ok((context, mappings))
    }

    #[allow(clippy::too_many_arguments)]
    fn variable_cast_payment_context(
        &self,
        player: PlayerId,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        targets: &[AnnouncedTarget],
        mode: Option<u32>,
        optional: &[bool],
        additional_costs: &[Vec<ObjectId>],
        x_value: u32,
    ) -> Result<(DecisionContext, Vec<(CanonicalActionId, MainChoice)>), String> {
        let program = self
            .programs
            .get(&object)
            .ok_or_else(|| format!("seed {} missing X spell program", self.seed))?;
        let mut mappings = Vec::new();
        let mut options = Vec::new();
        for payment in self.cast_payment_plans(player, object, alternate, x_value)? {
            let request = self.spell_request(
                program,
                alternate,
                payment,
                targets,
                mode,
                optional,
                additional_costs,
            )?;
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
                    alternate,
                    payment,
                    targets: targets.to_vec(),
                    mode,
                    optional: optional.to_vec(),
                    additional_costs: additional_costs.to_vec(),
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
                player,
                object,
                alternate,
                targets,
                mode,
                optional,
                additional_costs,
                1,
                x_value,
                x_value,
            ),
        )?;
        Ok((context, mappings))
    }

    fn hierarchical_main_context(
        &self,
        player: PlayerId,
        choice: &MainChoice,
    ) -> Result<Option<MainDecisionAdapter>, String> {
        match choice {
            MainChoice::BeginActivateProgramWithCosts {
                source,
                ability,
                targets,
                optional,
                sacrifice_objects,
            } => {
                let sacrifice_cost =
                    self.state
                        .activation_sacrifice_cost(*ability)
                        .map_err(|error| {
                            format!("seed {} activation cost failed: {error:?}", self.seed)
                        })?;
                match (sacrifice_cost, sacrifice_objects) {
                    (Some(_), None) => self
                        .activation_sacrifice_context(player, *source, *ability, targets, optional)
                        .map(Some),
                    (Some(_), Some(objects)) => self
                        .activation_payment_context(
                            player, *source, *ability, targets, optional, objects,
                        )
                        .map(Some),
                    (None, None) => self
                        .activation_payment_context(
                            player,
                            *source,
                            *ability,
                            targets,
                            optional,
                            &[],
                        )
                        .map(Some),
                    (None, Some(_)) => Err(format!(
                        "seed {} activation {} supplied an unexpected sacrifice selection",
                        self.seed,
                        ability.get()
                    )),
                }
            }
            MainChoice::BeginCast {
                object,
                alternate,
                targets,
                mode,
                optional,
                additional_costs,
            } => {
                let program = self
                    .programs
                    .get(object)
                    .ok_or_else(|| format!("seed {} missing deferred spell program", self.seed))?;
                if additional_costs.len() < program.additional_costs().len() {
                    return self
                        .additional_cast_cost_context(
                            player,
                            *object,
                            *alternate,
                            targets,
                            *mode,
                            optional,
                            additional_costs,
                        )
                        .map(Some);
                }
                if additional_costs.len() != program.additional_costs().len() {
                    return Err(format!(
                        "seed {} spell {} has too many additional-cost groups",
                        self.seed,
                        program.name()
                    ));
                }
                if self.cast_mana_cost(program, *alternate)?.x_count() == 0 {
                    return self
                        .variable_cast_payment_context(
                            player,
                            *object,
                            *alternate,
                            targets,
                            *mode,
                            optional,
                            additional_costs,
                            0,
                        )
                        .map(Some);
                }
                let maximum = self
                    .maximum_affordable_x(player, *object, *alternate)?
                    .ok_or_else(|| {
                        format!("seed {} selected an unaffordable X spell", self.seed)
                    })?;
                self.variable_cast_numeric_context(
                    player,
                    *object,
                    *alternate,
                    targets,
                    *mode,
                    optional,
                    additional_costs,
                    (0, maximum),
                )
                .map(Some)
            }
            MainChoice::NarrowCastX {
                object,
                alternate,
                targets,
                mode,
                optional,
                additional_costs,
                minimum,
                maximum,
            } => self
                .variable_cast_numeric_context(
                    player,
                    *object,
                    *alternate,
                    targets,
                    *mode,
                    optional,
                    additional_costs,
                    (*minimum, *maximum),
                )
                .map(Some),
            MainChoice::ChooseCastX {
                object,
                alternate,
                targets,
                mode,
                optional,
                additional_costs,
                x_value,
            } => self
                .variable_cast_payment_context(
                    player,
                    *object,
                    *alternate,
                    targets,
                    *mode,
                    optional,
                    additional_costs,
                    *x_value,
                )
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
        let runtime = self.benchmark_runtime_semantics();
        DecisionContext::new_with_benchmark_semantics(
            kind,
            actor,
            &view,
            options,
            Vec::new(),
            &runtime,
        )
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
        let runtime = self.benchmark_runtime_semantics();
        DecisionContext::new_scoped_with_benchmark_semantics(
            kind,
            actor,
            &view,
            options,
            Vec::new(),
            path_discriminator,
            &runtime,
        )
        .map_err(|error| format!("seed {} scoped decision context failed: {error}", self.seed))
    }

    fn benchmark_runtime_semantics(&self) -> BenchmarkRuntimeSemantics {
        let mut runtime = BenchmarkRuntimeSemantics::default();
        for ability in &self.activated_abilities {
            runtime.bind_ability(
                ability.id,
                ability.source,
                ability.benchmark_semantic_identity.as_bytes(),
            );
        }
        for (trigger, trigger_runtime) in &self.trigger_programs {
            runtime.bind_trigger(
                *trigger,
                trigger_runtime.source,
                trigger_runtime.benchmark_semantic_identity.as_bytes(),
            );
        }
        for (position, entry) in self.state.stack_entries().iter().enumerate() {
            runtime.bind_stack_entry(entry.clone(), position as u32);
        }
        runtime
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
            episode,
        } = telemetry;
        let legal_actions = context.options().len();
        let index = self.ai_decisions.len() as u64;
        let mut episode = episode
            .unwrap_or_else(|| DecisionEpisodeMetadata::root(self.seed, index, context, true));
        if episode.is_terminal_subchoice {
            episode.final_concrete_action_id = action_id.to_string();
        }
        self.ai_decisions.push(AiDecisionRecord {
            index,
            kind: kind.to_owned(),
            policy: policy.to_owned(),
            context_id: context.id().to_string(),
            decision_state_key: context.state_key().to_string(),
            normalized_benchmark_key: context.normalized_benchmark_key().to_string(),
            normalized_player_view_hash: format!(
                "{:016x}",
                context.normalized_player_view_hash().get()
            ),
            normalized_legal_action_ids: context
                .normalized_action_ids()
                .iter()
                .map(ToString::to_string)
                .collect(),
            benchmark_normalization_complete: context.benchmark_normalization_complete(),
            path_discriminator: context.path_discriminator(),
            player_view_hash: format!("{:016x}", context.player_view_hash().get()),
            action_id: action_id.to_string(),
            episode,
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

    #[allow(clippy::too_many_arguments)]
    fn record_search_decision(
        &mut self,
        kind: &'static str,
        policy: &'static str,
        context: &DecisionContext,
        report: &SearchReport,
        adaptive_search: bool,
        wall_latency_us: u64,
        episode: Option<DecisionEpisodeMetadata>,
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
        let index = self.ai_decisions.len() as u64;
        let mut episode = episode
            .unwrap_or_else(|| DecisionEpisodeMetadata::root(self.seed, index, context, true));
        if episode.is_terminal_subchoice {
            episode.final_concrete_action_id = report.selected_action().to_string();
        }
        self.ai_decisions.push(AiDecisionRecord {
            index,
            kind: kind.to_owned(),
            policy: policy.to_owned(),
            context_id: context.id().to_string(),
            decision_state_key: context.state_key().to_string(),
            normalized_benchmark_key: context.normalized_benchmark_key().to_string(),
            normalized_player_view_hash: format!(
                "{:016x}",
                context.normalized_player_view_hash().get()
            ),
            normalized_legal_action_ids: context
                .normalized_action_ids()
                .iter()
                .map(ToString::to_string)
                .collect(),
            benchmark_normalization_complete: context.benchmark_normalization_complete(),
            path_discriminator: context.path_discriminator(),
            player_view_hash: format!("{:016x}", context.player_view_hash().get()),
            action_id: report.selected_action().to_string(),
            episode,
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

    fn complete_ai_episode(&mut self, episode: &DecisionEpisodePath) -> Result<(), String> {
        let final_action_id = episode.final_action_id();
        let mut matched = self
            .ai_decisions
            .iter_mut()
            .filter(|decision| {
                decision.episode.decision_episode_id == episode.metadata.decision_episode_id
            })
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Err(format!(
                "seed {} cannot complete unknown AI decision episode {}",
                self.seed, episode.metadata.decision_episode_id
            ));
        }
        for decision in &mut matched {
            decision.episode.is_terminal_subchoice = false;
            decision
                .episode
                .final_concrete_action_id
                .clone_from(&final_action_id);
        }
        if let Some(last) = matched.last_mut() {
            last.episode.is_terminal_subchoice = true;
        }
        Ok(())
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
                || !program.additional_costs().is_empty()
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
                None,
                payment,
                Vec::new(),
                None,
                Vec::new(),
                Vec::new(),
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn cast_program_with_choices(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        alternate: Option<AlternateCostKind>,
        payment: PaymentPlan,
        targets: Vec<AnnouncedTarget>,
        mode: Option<u32>,
        optional: Vec<bool>,
        additional_costs: Vec<Vec<ObjectId>>,
    ) -> Result<(), String> {
        let program = self
            .programs
            .get(&object)
            .cloned()
            .ok_or_else(|| format!("seed {} missing program for cast object", self.seed))?;
        let request = self.spell_request(
            &program,
            alternate,
            payment,
            &targets,
            mode,
            &optional,
            &additional_costs,
        )?;
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
        self.register_triggers(player, object, &program, alternate)
    }

    fn register_triggers(
        &mut self,
        controller: PlayerId,
        source: ObjectId,
        program: &Arc<CardProgram>,
        alternate: Option<AlternateCostKind>,
    ) -> Result<(), String> {
        let first_registration = self.triggers_registered_for.insert(source);
        for (ability_index, ability) in program.triggered_abilities().iter().enumerate() {
            let conditional = ability.required_alternate_cost();
            if conditional.is_none() && !first_registration {
                continue;
            }
            if conditional.is_some() && conditional != alternate {
                continue;
            }
            let definition = if conditional.is_some() {
                ability.bind(controller, source).delayed_once()
            } else {
                ability.bind(controller, source)
            };
            let outcome = self.dispatch(Action::RegisterTriggeredAbility { definition })?;
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
                    benchmark_semantic_identity: format!(
                        "{}/trigger/{ability_index}",
                        program.oracle_id()
                    ),
                },
            );
            if conditional.is_some() {
                self.conditional_cast_triggers
                    .entry(source)
                    .or_default()
                    .push(trigger);
            }
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
        for (ability_index, ability) in program.activated_abilities().iter().enumerate() {
            if ability.condition().is_some() {
                continue;
            }
            let outcome = self.dispatch(Action::RegisterActivatedAbility {
                definition: Box::new(ability.bind(controller, source)),
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
                benchmark_semantic_identity: format!(
                    "{}/mana/{ability_index}",
                    program.oracle_id()
                ),
            });
        }
        for (ability_index, ability) in program.activated_effects().iter().enumerate() {
            let mut cost = ActivationCost::new(ability.mana_cost());
            if ability.tap_source() {
                cost = cost.with_tap_source();
            }
            if ability.sacrifice_source() {
                cost = cost.with_sacrifice_source();
            }
            if ability.pay_life() != 0 {
                cost = cost.with_life(ability.pay_life());
            }
            let mut definition = ActivatedAbilityDefinition::new(
                controller,
                Some(source),
                ability.timing(),
                cost,
                ActivatedAbilityEffect::ProgramBound,
            );
            if let Some((predicate, count)) = ability.sacrifice_cost() {
                definition = definition.with_sacrifice_permanents(predicate, count);
            }
            let outcome = self.dispatch(Action::RegisterActivatedAbility {
                definition: Box::new(definition),
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
                benchmark_semantic_identity: format!(
                    "{}/activated/{ability_index}",
                    program.oracle_id()
                ),
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

    fn pending_spell_requirements<'a>(
        &self,
        pending: &'a PendingSpellResolution,
    ) -> Result<&'a [ObjectChoiceRequirement], String> {
        match pending.decisions.mode() {
            Some(mode) => pending
                .program
                .spell_modes()
                .get(mode as usize)
                .map(SpellModeProgram::object_choice_requirements)
                .ok_or_else(|| {
                    format!(
                        "seed {} resolved spell {} with invalid mode {mode}",
                        self.seed,
                        pending.program.name()
                    )
                }),
            None if pending.program.spell_modes().is_empty() => {
                Ok(pending.program.object_choice_requirements())
            }
            None => Err(format!(
                "seed {} resolved modal spell {} without a mode binding",
                self.seed,
                pending.program.name()
            )),
        }
    }

    fn pending_spell_actions(
        &self,
        pending: &PendingSpellResolution,
        object_choices: Vec<Vec<ObjectId>>,
    ) -> Result<Vec<Action>, String> {
        let mut bindings =
            ExecutionBindings::new(pending.controller, self.live_opponents(pending.controller))
                .with_source(pending.object)
                .with_announced_targets(
                    pending.target_requirements.clone(),
                    pending.targets.clone(),
                )
                .with_target_legalities(pending.target_legalities.clone())
                .with_object_choices(object_choices)
                .with_optional_effect_choices(pending.decisions.optional_choices().collect());
        if let Some(mode) = pending.decisions.mode() {
            bindings = bindings.with_spell_mode(mode as usize);
        }
        if let Some(alternate) = pending.decisions.alternate_cost() {
            bindings = bindings.with_alternate_cost(core_alternate_to_runtime(alternate));
        }
        bind_program_actions(&self.state, &pending.program, &bindings)
            .map(|actions| {
                actions
                    .into_iter()
                    .map(|action| action.action().clone())
                    .collect()
            })
            .map_err(|error| {
                format!(
                    "seed {} spell interpreter binding failed: {error}",
                    self.seed
                )
            })
    }

    #[cfg(test)]
    fn pending_spell_context(
        &self,
        pending: &PendingSpellResolution,
    ) -> Result<DecisionContext, String> {
        let requirements = self.pending_spell_requirements(pending)?;
        self.pending_spell_choice_context(pending, requirements, 0, &[])
    }

    fn pending_spell_choice_context(
        &self,
        pending: &PendingSpellResolution,
        requirements: &[ObjectChoiceRequirement],
        cursor: usize,
        prior: &[Vec<ObjectId>],
    ) -> Result<DecisionContext, String> {
        let requirement = requirements.get(cursor).copied().ok_or_else(|| {
            format!(
                "seed {} spell resolution has no object choice at slot {cursor}",
                self.seed
            )
        })?;
        if prior.len() != cursor {
            return Err(format!(
                "seed {} spell resolution path has {} choices at slot {cursor}",
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
                self.pending_spell_actions(pending, choices.clone())?
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

    fn complete_pending_spell_resolution(
        &mut self,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
    ) -> Result<(), String> {
        let pending = self
            .pending_spell_resolution
            .take()
            .ok_or_else(|| format!("seed {} has no pending spell resolution", self.seed))?;
        let requirements = self.pending_spell_requirements(&pending)?.to_vec();
        if requirements.is_empty() {
            return Err(format!(
                "seed {} pending spell resolution has no object choices",
                self.seed
            ));
        }
        let mut choices = Vec::with_capacity(requirements.len());
        let mut actions = Vec::new();
        let mut episode = None;
        for cursor in 0..requirements.len() {
            let context =
                self.pending_spell_choice_context(&pending, &requirements, cursor, &choices)?;
            if episode.is_none() {
                episode = self.begin_controlled_episode(
                    pending.controller,
                    human,
                    decisions,
                    ai_policies,
                    &context,
                )?;
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
            let selected_id = self.select_resolution_choice(
                &context,
                pending.controller,
                human,
                decisions,
                ai_policies,
                "spell_resolution_object_choice",
                episode_metadata.clone(),
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
            }
            let selected = context.select(selected_id).map_err(|error| {
                format!("seed {} spell resolution choice failed: {error}", self.seed)
            })?;
            let DecisionDescriptor::ChooseResolutionObjects {
                choices: selected_choices,
            } = selected.descriptor()
            else {
                return Err(format!(
                    "seed {} spell resolution returned a non-object descriptor",
                    self.seed
                ));
            };
            choices.clone_from(selected_choices);
            if cursor + 1 == requirements.len() {
                actions = selected.actions().to_vec();
            }
        }
        self.complete_controlled_episode(
            pending.controller,
            human,
            decisions,
            ai_policies,
            episode.as_ref(),
        )?;
        let action_count = actions.len() as u64;
        for action in actions {
            self.dispatch(action)?;
        }
        self.metrics.interpreter_actions = self
            .metrics
            .interpreter_actions
            .saturating_add(action_count);
        if let Some(exercise) = self.identity_exercise_mut(pending.object) {
            exercise.effect_actions = exercise.effect_actions.saturating_add(action_count);
        }
        Ok(())
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
                .with_announced_targets(
                    pending.target_requirements.clone(),
                    pending.targets.clone(),
                )
                .with_target_legalities(pending.target_legalities.clone())
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

    #[allow(clippy::too_many_arguments)]
    fn select_resolution_choice(
        &mut self,
        context: &DecisionContext,
        chooser: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        telemetry_kind: &'static str,
        episode: Option<DecisionEpisodeMetadata>,
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
            return if let Some(episode) = episode {
                self.prompt_context_choice_in_episode(
                    source,
                    "Choose cards while resolving",
                    context,
                    &labels,
                    episode,
                )
            } else {
                self.prompt_context_choice(source, "Choose cards while resolving", context, &labels)
            };
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
                episode,
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
        let mut episode = None;
        for cursor in 0..requirements.len() {
            let context =
                self.pending_activated_choice_context(&pending, &requirements, cursor, &choices)?;
            if episode.is_none() {
                episode = self.begin_controlled_episode(
                    pending.controller,
                    human,
                    decisions,
                    ai_policies,
                    &context,
                )?;
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
            let selected_id = self.select_resolution_choice(
                &context,
                pending.controller,
                human,
                decisions,
                ai_policies,
                "resolution_object_choice",
                episode_metadata.clone(),
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
            }
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
        self.complete_controlled_episode(
            pending.controller,
            human,
            decisions,
            ai_policies,
            episode.as_ref(),
        )?;
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
        optional_choices: &[bool],
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
        if pending.decisions.optional_choice_count() != 0 {
            return Err(format!(
                "seed {} trigger on {} carried {} announcement-time optional choices; trigger optionals must be chosen at resolution",
                self.seed,
                pending.runtime.program.name(),
                pending.decisions.optional_choice_count()
            ));
        }
        if optional_choices.len() != ability.optional_choice_count() {
            return Err(format!(
                "seed {} trigger on {} has {} resolved optional choices for {} slots",
                self.seed,
                pending.runtime.program.name(),
                optional_choices.len(),
                ability.optional_choice_count()
            ));
        }
        let mut bindings =
            ExecutionBindings::new(pending.controller, self.live_opponents(pending.controller))
                .with_source(pending.runtime.source)
                .with_announced_targets(
                    pending.target_requirements.clone(),
                    pending.targets.clone(),
                )
                .with_target_legalities(pending.target_legalities.clone())
                .with_object_choices(object_choices)
                .with_optional_effect_choices(optional_choices.to_vec());
        if let Some(player) = pending.triggering_player {
            bindings = bindings.with_triggering_player(player);
        }
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
        let optional = vec![true; ability.optional_choice_count()];
        self.pending_triggered_choice_context(
            pending,
            ability.object_choice_requirements(),
            0,
            &[],
            &optional,
        )
    }

    fn pending_triggered_optional_context(
        &self,
        pending: &PendingTriggeredResolution,
        cursor: usize,
        prior: &[bool],
        has_object_choices: bool,
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
        if cursor >= ability.optional_choice_count() || prior.len() != cursor {
            return Err(format!(
                "seed {} trigger optional path has {} choices at slot {cursor} of {}",
                self.seed,
                prior.len(),
                ability.optional_choice_count()
            ));
        }
        let final_slot = cursor + 1 == ability.optional_choice_count();
        let prompt = u32::try_from(cursor)
            .map_err(|_| format!("seed {} trigger optional index overflow", self.seed))?;
        let mut options = Vec::with_capacity(2);
        for accept in [false, true] {
            let mut choices = prior.to_vec();
            choices.push(accept);
            let actions = if final_slot && !has_object_choices {
                self.pending_triggered_actions(pending, Vec::new(), &choices)?
            } else {
                Vec::new()
            };
            options.push(DecisionOption::new(
                DecisionDescriptor::ChooseOptional { prompt, accept },
                actions,
            ));
        }
        self.scoped_decision_context(
            DecisionKind::Optional,
            pending.controller,
            options,
            trigger_optional_path_discriminator(pending.controller, pending.trigger, cursor, prior),
        )
    }

    fn select_trigger_optional_choice(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(bool, Vec<Action>, CanonicalActionId), String> {
        self.select_trigger_boolean_choice(
            context,
            controller,
            human,
            decisions,
            ai_policies,
            "Choose whether to use the triggered effect",
            "Accept optional effect",
            "Decline optional effect",
            "trigger_optional",
            1,
            true,
            episode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn select_trigger_boolean_choice(
        &mut self,
        context: &DecisionContext,
        chooser: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        prompt: &'static str,
        accept_label: &'static str,
        decline_label: &'static str,
        telemetry_kind: &'static str,
        accept_prior: i64,
        autonomous_accept: bool,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(bool, Vec<Action>, CanonicalActionId), String> {
        let selected_id = if context.options().len() == 1 && ai_policies.is_none() {
            context.options()[0].id()
        } else if human == Some(chooser) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::ChooseOptional { accept, .. } => Ok(if *accept {
                        accept_label.to_owned()
                    } else {
                        decline_label.to_owned()
                    }),
                    descriptor => Err(format!(
                        "seed {} cannot label {telemetry_kind} descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                format!("human game is missing a decision source for {telemetry_kind}")
            })?;
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(source, prompt, context, &labels, episode)?
            } else {
                self.prompt_context_choice(source, prompt, context, &labels)?
            }
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(chooser, policies)?;
            let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
                (context.options()[0].id(), None, "forced-v1", Vec::new())
            } else {
                let candidates = if matches!(policy, AiController::Random(_)) {
                    Vec::new()
                } else {
                    self.policy_candidates(context, chooser, |option| {
                        if matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseOptional { accept: true, .. }
                        ) {
                            accept_prior
                        } else {
                            0
                        }
                    })?
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, context, &candidates, telemetry_kind)?;
                (selected_id, decision, policy_name, candidates)
            };
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
                episode,
            });
            selected_id
        } else {
            context
                .options()
                .iter()
                .find(|option| {
                    matches!(
                        option.descriptor(),
                        DecisionDescriptor::ChooseOptional { accept, .. }
                            if *accept == autonomous_accept
                    )
                })
                .or_else(|| context.options().first())
                .map(DecisionOption::id)
                .ok_or_else(|| format!("seed {} {telemetry_kind} has no branch", self.seed))?
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal {telemetry_kind} action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::ChooseOptional { accept, .. } = selected.descriptor() else {
            return Err(format!(
                "seed {} {telemetry_kind} context returned a non-optional descriptor",
                self.seed
            ));
        };
        Ok((*accept, selected.actions().to_vec(), selected_id))
    }

    fn pending_triggered_unless_intent_context(
        &self,
        pending: &PendingTriggeredResolution,
    ) -> Result<(DecisionContext, PlayerId, Vec<PaymentPlan>), String> {
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
        let unless = ability.unless_paid().ok_or_else(|| {
            format!(
                "seed {} trigger on {} has no unless-paid branch",
                self.seed,
                pending.runtime.program.name()
            )
        })?;
        if unless.payer() != PlayerBinding::TriggeringPlayer {
            return Err(format!(
                "seed {} trigger on {} has unsupported unless-paid payer {:?}",
                self.seed,
                pending.runtime.program.name(),
                unless.payer()
            ));
        }
        let payer = pending.triggering_player.ok_or_else(|| {
            format!(
                "seed {} trigger on {} lost its event-bound payer",
                self.seed,
                pending.runtime.program.name()
            )
        })?;
        let plans = self
            .state
            .payment_plans_for_player(payer, unless.mana_cost())
            .map_err(|error| {
                format!(
                    "seed {} unless-payment enumeration for player {} failed: {error:?}",
                    self.seed,
                    payer.index()
                )
            })?
            .plans()
            .to_vec();
        let prompt = u32::try_from(ability.optional_choice_count())
            .map_err(|_| format!("seed {} unless-payment prompt overflow", self.seed))?;
        let has_followup = ability.optional_choice_count() != 0
            || !ability.object_choice_requirements().is_empty();
        let decline_actions = if has_followup {
            Vec::new()
        } else {
            self.pending_triggered_actions(pending, Vec::new(), &[])?
        };
        let mut options = vec![DecisionOption::new(
            DecisionDescriptor::ChooseOptional {
                prompt,
                accept: false,
            },
            decline_actions,
        )];
        if !plans.is_empty() {
            options.push(DecisionOption::new(
                DecisionDescriptor::ChooseOptional {
                    prompt,
                    accept: true,
                },
                Vec::new(),
            ));
        }
        let context = self.scoped_decision_context(
            DecisionKind::Optional,
            payer,
            options,
            trigger_unless_path_discriminator(payer, pending.trigger, 0),
        )?;
        Ok((context, payer, plans))
    }

    fn pending_triggered_unless_payment_context(
        &self,
        pending: &PendingTriggeredResolution,
        payer: PlayerId,
        plans: &[PaymentPlan],
    ) -> Result<DecisionContext, String> {
        let unless = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .and_then(|ability| ability.unless_paid())
            .ok_or_else(|| {
                format!(
                    "seed {} trigger on {} has no unless-paid payment branch",
                    self.seed,
                    pending.runtime.program.name()
                )
            })?;
        if plans.is_empty() {
            return Err(format!(
                "seed {} player {} accepted an unaffordable unless payment",
                self.seed,
                payer.index()
            ));
        }
        let options = plans
            .iter()
            .copied()
            .map(|payment| {
                DecisionOption::new(
                    DecisionDescriptor::ChoosePayment { payment },
                    vec![Action::PayMana {
                        player: payer,
                        cost: unless.mana_cost(),
                        plan: payment,
                    }],
                )
            })
            .collect();
        self.scoped_decision_context(
            DecisionKind::Payment,
            payer,
            options,
            trigger_unless_path_discriminator(payer, pending.trigger, 1),
        )
    }

    fn select_trigger_unless_payment(
        &mut self,
        context: &DecisionContext,
        payer: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(Vec<Action>, CanonicalActionId), String> {
        let selected_id = if context.options().len() == 1
            && ai_policies.is_none()
            && human != Some(payer)
        {
            context.options()[0].id()
        } else if human == Some(payer) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::ChoosePayment { payment } => {
                        Ok(format!("Pay (payment waste {})", payment.waste_score()))
                    }
                    descriptor => Err(format!(
                        "seed {} cannot label trigger payment descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for trigger payment".to_owned()
            })?;
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(
                    source,
                    "Choose a trigger payment",
                    context,
                    &labels,
                    episode,
                )?
            } else {
                self.prompt_context_choice(source, "Choose a trigger payment", context, &labels)?
            }
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(payer, policies)?;
            let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
                (context.options()[0].id(), None, "forced-v1", Vec::new())
            } else {
                let candidates = if matches!(policy, AiController::Random(_)) {
                    Vec::new()
                } else {
                    self.policy_candidates(context, payer, |option| match option.descriptor() {
                        DecisionDescriptor::ChoosePayment { payment } => {
                            -i64::from(payment.waste_score())
                        }
                        _ => 0,
                    })?
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, context, &candidates, "trigger_payment")?;
                (selected_id, decision, policy_name, candidates)
            };
            self.record_ai_decision(AiDecisionTelemetry {
                kind: "trigger_payment",
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
                episode,
            });
            selected_id
        } else {
            context
                .options()
                .iter()
                .min_by_key(|option| match option.descriptor() {
                    DecisionDescriptor::ChoosePayment { payment } => payment.waste_score(),
                    _ => u32::MAX,
                })
                .map(DecisionOption::id)
                .ok_or_else(|| format!("seed {} trigger payment has no options", self.seed))?
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal trigger payment: {error}",
                self.seed
            )
        })?;
        if !matches!(
            selected.descriptor(),
            DecisionDescriptor::ChoosePayment { .. }
        ) {
            return Err(format!(
                "seed {} trigger payment context returned a non-payment descriptor",
                self.seed
            ));
        }
        Ok((selected.actions().to_vec(), selected_id))
    }

    fn pending_triggered_choice_context(
        &self,
        pending: &PendingTriggeredResolution,
        requirements: &[ObjectChoiceRequirement],
        cursor: usize,
        prior: &[Vec<ObjectId>],
        optional_choices: &[bool],
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
                self.pending_triggered_actions(pending, choices.clone(), optional_choices)?
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
        let optional_count = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .map_or(0, |ability| ability.optional_choice_count());
        let has_unless = pending
            .runtime
            .program
            .triggered_abilities()
            .get(pending.runtime.ability_index)
            .is_some_and(|ability| ability.unless_paid().is_some());
        if requirements.is_empty() && optional_count == 0 && !has_unless {
            return Err(format!(
                "seed {} pending triggered resolution has no deferred choices",
                self.seed
            ));
        }
        let mut optional_choices = Vec::with_capacity(optional_count);
        let mut actions = Vec::new();
        let mut unless_paid = false;
        if has_unless {
            let (context, payer, plans) = self.pending_triggered_unless_intent_context(&pending)?;
            let mut episode =
                self.begin_controlled_episode(payer, human, decisions, ai_policies, &context)?;
            let episode_metadata = episode.as_ref().map(DecisionEpisodePath::root_metadata);
            let (pay, selected_actions, selected_id) = self.select_trigger_boolean_choice(
                &context,
                payer,
                human,
                decisions,
                ai_policies,
                "Choose whether to pay for the triggered ability",
                "Pay the unless cost",
                "Do not pay",
                "trigger_unless_payment_intent",
                0,
                false,
                episode_metadata.clone(),
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
            }
            actions = selected_actions;
            if pay {
                let payment_context =
                    self.pending_triggered_unless_payment_context(&pending, payer, &plans)?;
                let payment_episode = episode
                    .as_ref()
                    .map(|episode| episode.child_metadata(&payment_context));
                let (selected_actions, selected_id) = self.select_trigger_unless_payment(
                    &payment_context,
                    payer,
                    human,
                    decisions,
                    ai_policies,
                    payment_episode.clone(),
                )?;
                actions = selected_actions;
                if let Some(episode) = episode.as_mut() {
                    episode.record_selection(
                        &payment_context,
                        selected_id,
                        payment_episode.map_or(0, |metadata| metadata.path_depth),
                    );
                }
                unless_paid = true;
            }
            self.complete_controlled_episode(
                payer,
                human,
                decisions,
                ai_policies,
                episode.as_ref(),
            )?;
        }
        if !unless_paid {
            let mut episode = None;
            for cursor in 0..optional_count {
                let context = self.pending_triggered_optional_context(
                    &pending,
                    cursor,
                    &optional_choices,
                    !requirements.is_empty(),
                )?;
                if episode.is_none() {
                    episode = self.begin_controlled_episode(
                        pending.controller,
                        human,
                        decisions,
                        ai_policies,
                        &context,
                    )?;
                }
                let episode_metadata = episode.as_ref().map(|episode| {
                    if episode.selected_action_ids.is_empty() {
                        episode.root_metadata()
                    } else {
                        episode.child_metadata(&context)
                    }
                });
                let (accept, selected_actions, selected_id) = self.select_trigger_optional_choice(
                    &context,
                    pending.controller,
                    human,
                    decisions,
                    ai_policies,
                    episode_metadata.clone(),
                )?;
                if let Some(episode) = episode.as_mut() {
                    episode.record_selection(
                        &context,
                        selected_id,
                        episode_metadata.map_or(0, |metadata| metadata.path_depth),
                    );
                }
                optional_choices.push(accept);
                if cursor + 1 == optional_count && requirements.is_empty() {
                    actions = selected_actions;
                }
            }
            let mut choices = Vec::with_capacity(requirements.len());
            for cursor in 0..requirements.len() {
                let context = self.pending_triggered_choice_context(
                    &pending,
                    &requirements,
                    cursor,
                    &choices,
                    &optional_choices,
                )?;
                if episode.is_none() {
                    episode = self.begin_controlled_episode(
                        pending.controller,
                        human,
                        decisions,
                        ai_policies,
                        &context,
                    )?;
                }
                let episode_metadata = episode.as_ref().map(|episode| {
                    if episode.selected_action_ids.is_empty() {
                        episode.root_metadata()
                    } else {
                        episode.child_metadata(&context)
                    }
                });
                let selected_id = self.select_resolution_choice(
                    &context,
                    pending.controller,
                    human,
                    decisions,
                    ai_policies,
                    "trigger_resolution_object_choice",
                    episode_metadata.clone(),
                )?;
                if let Some(episode) = episode.as_mut() {
                    episode.record_selection(
                        &context,
                        selected_id,
                        episode_metadata.map_or(0, |metadata| metadata.path_depth),
                    );
                }
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
            self.complete_controlled_episode(
                pending.controller,
                human,
                decisions,
                ai_policies,
                episode.as_ref(),
            )?;
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

    fn ordered_pending_trigger_instances(
        &self,
        pending: &[PendingTriggeredAbility],
        order: &[TriggerId],
    ) -> Result<Vec<PendingTriggeredAbility>, String> {
        if pending.len() != order.len() {
            return Err(format!(
                "seed {} trigger instance order has {} IDs for {} pending instances",
                self.seed,
                order.len(),
                pending.len()
            ));
        }
        let mut used = vec![false; pending.len()];
        let mut ordered = Vec::with_capacity(order.len());
        for trigger in order {
            let index = pending
                .iter()
                .enumerate()
                .find_map(|(index, instance)| {
                    (!used[index] && instance.trigger() == *trigger).then_some(index)
                })
                .ok_or_else(|| {
                    format!(
                        "seed {} trigger {} has no unmatched pending instance",
                        self.seed,
                        trigger.index()
                    )
                })?;
            used[index] = true;
            ordered.push(pending[index]);
        }
        Ok(ordered)
    }

    fn triggering_player_for_pending(
        &self,
        pending: PendingTriggeredAbility,
    ) -> Result<Option<PlayerId>, String> {
        let Some(runtime) = self.trigger_programs.get(&pending.trigger()) else {
            return Ok(None);
        };
        let ability = runtime
            .program
            .triggered_abilities()
            .get(runtime.ability_index)
            .ok_or_else(|| {
                format!(
                    "seed {} missing triggered runtime {} on {}",
                    self.seed,
                    runtime.ability_index,
                    runtime.program.name()
                )
            })?;
        if ability.unless_paid().is_none() {
            return Ok(None);
        }
        let event = self
            .state
            .events_this_turn()
            .iter()
            .copied()
            .find(|record| {
                record.turn() == pending.event_turn()
                    && record.sequence() == pending.event_sequence()
            })
            .ok_or_else(|| {
                format!(
                    "seed {} cannot recover event {}/{} for unless-paid trigger {}",
                    self.seed,
                    pending.event_turn(),
                    pending.event_sequence(),
                    pending.trigger().index()
                )
            })?;
        match event.event() {
            GameEvent::CardDrawn { player, .. } => Ok(Some(player)),
            other => Err(format!(
                "seed {} unless-paid trigger {} was queued by unsupported event {other:?}",
                self.seed,
                pending.trigger().index()
            )),
        }
    }

    fn prepare_trigger_stack_contexts(
        &self,
        ordered: &[PendingTriggeredAbility],
        bindings: Option<&[TriggerStackBinding]>,
    ) -> Result<Vec<(TriggerId, Option<PlayerId>)>, String> {
        if bindings.is_some_and(|bindings| bindings.len() != ordered.len()) {
            return Err(format!(
                "seed {} has a mismatched trigger-binding batch",
                self.seed
            ));
        }
        ordered
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, pending)| {
                let put_on_stack = bindings.map_or(true, |bindings| {
                    bindings[index].disposition() == TriggerStackDisposition::PutOnStack
                });
                put_on_stack.then_some((index, pending))
            })
            .map(|(_, pending)| {
                self.triggering_player_for_pending(pending)
                    .map(|player| (pending.trigger(), player))
            })
            .collect()
    }

    fn dispatch_trigger_stack_action(
        &mut self,
        action: Action,
        contexts: Vec<(TriggerId, Option<PlayerId>)>,
    ) -> Result<Outcome, String> {
        let outcome = self.dispatch(action)?;
        let Outcome::StackEntriesAdded(entries) = &outcome else {
            return Err(format!(
                "seed {} trigger stack action returned {outcome:?}",
                self.seed
            ));
        };
        if entries.len() != contexts.len() {
            return Err(format!(
                "seed {} trigger stack action created {} entries for {} prepared contexts",
                self.seed,
                entries.len(),
                contexts.len()
            ));
        }
        for (entry, (trigger, triggering_player)) in entries.iter().copied().zip(contexts) {
            let actual = self
                .state
                .stack_entries()
                .iter()
                .find(|candidate| candidate.id() == entry)
                .and_then(|candidate| candidate.trigger());
            if actual != Some(trigger) {
                return Err(format!(
                    "seed {} stack entry {} bound trigger {actual:?} instead of {}",
                    self.seed,
                    entry.index(),
                    trigger.index()
                ));
            }
            if let Some(player) = triggering_player {
                self.triggering_players_by_stack_entry.insert(entry, player);
            }
        }
        Ok(outcome)
    }

    fn prospective_trigger_stack_entry_id(&self, offset: usize) -> Result<StackEntryId, String> {
        let offset = u32::try_from(offset)
            .map_err(|_| format!("seed {} trigger stack offset overflow", self.seed))?;
        self.state
            .next_stack_entry_id()
            .checked_offset(offset)
            .ok_or_else(|| format!("seed {} trigger stack ID overflow", self.seed))
    }

    fn trigger_target_choices(
        &self,
        controller: PlayerId,
        source: ObjectId,
        requirement: TargetRequirement,
        prospective_stack_entries: &[StackEntryId],
    ) -> Vec<TargetChoice> {
        let mut choices = self.legal_targets_for(controller, source, requirement);
        if requirement.kind() == TargetKind::StackEntry
            && requirement.predicate() == TargetPredicate::Any
        {
            choices.extend(
                prospective_stack_entries
                    .iter()
                    .copied()
                    .map(TargetChoice::StackEntry),
            );
        }
        choices.sort_by_key(|choice| match choice {
            TargetChoice::Player(target) => (0_u8, target.index()),
            TargetChoice::Object(target) => (1_u8, target.index()),
            TargetChoice::StackEntry(target) => (2_u8, target.index()),
        });
        choices.dedup();
        choices
    }

    #[allow(clippy::too_many_arguments)]
    fn trigger_target_context(
        &self,
        controller: PlayerId,
        source: ObjectId,
        trigger: TriggerId,
        position: (usize, usize),
        prior: &[AnnouncedTarget],
        requirement: TargetRequirement,
        prospective_stack_entries: &[StackEntryId],
    ) -> Result<DecisionContext, String> {
        let (stack_position, group_index) = position;
        let group = u8::try_from(group_index)
            .map_err(|_| format!("seed {} trigger target group overflow", self.seed))?;
        if requirement
            .group()
            .is_some_and(|compiled| compiled != group)
        {
            return Err(format!(
                "seed {} trigger {} target group {group_index} has inconsistent compiled identity",
                self.seed,
                trigger.index()
            ));
        }
        let selected = prior
            .iter()
            .filter(|target| target.group() == group)
            .count();
        if selected >= usize::from(requirement.maximum()) {
            return Err(format!(
                "seed {} trigger {} target group {group_index} is already complete",
                self.seed,
                trigger.index()
            ));
        }
        let mut choices =
            self.trigger_target_choices(controller, source, requirement, prospective_stack_entries);
        choices.retain(|choice| {
            !prior
                .iter()
                .any(|target| target.group() == group && target.target() == *choice)
        });
        let mut options = choices
            .into_iter()
            .map(|target| {
                let mut targets = prior.to_vec();
                targets.push(AnnouncedTarget::new(group, target));
                DecisionOption::new(
                    DecisionDescriptor::ChooseTriggerTargetGroups { trigger, targets },
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let minimum = if requirement.allocation_total().is_some() {
            usize::from(requirement.minimum()).max(1)
        } else {
            usize::from(requirement.minimum())
        };
        if selected >= minimum {
            options.push(DecisionOption::new(
                DecisionDescriptor::ChooseTriggerTargetGroups {
                    trigger,
                    targets: prior.to_vec(),
                },
                Vec::new(),
            ));
        }
        if options.is_empty() {
            return Err(format!(
                "seed {} trigger {} lost every legal target before satisfying group {group_index}",
                self.seed,
                trigger.index()
            ));
        }
        self.scoped_decision_context(
            DecisionKind::Target,
            controller,
            options,
            trigger_target_path_discriminator(
                controller,
                trigger,
                stack_position,
                group_index,
                0,
                prior,
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn trigger_target_allocation_context(
        &self,
        controller: PlayerId,
        trigger: TriggerId,
        stack_position: usize,
        target_index: usize,
        prior: &[AnnouncedTarget],
        minimum: u32,
        maximum: u32,
    ) -> Result<DecisionContext, String> {
        let target = prior.get(target_index).copied().ok_or_else(|| {
            format!(
                "seed {} trigger {} allocation target {target_index} is missing",
                self.seed,
                trigger.index()
            )
        })?;
        if minimum == 0 || minimum > maximum {
            return Err(format!(
                "seed {} trigger {} has invalid target allocation range {minimum}-{maximum}",
                self.seed,
                trigger.index()
            ));
        }
        let options = (minimum..=maximum)
            .map(|amount| {
                let mut targets = prior.to_vec();
                targets[target_index] =
                    AnnouncedTarget::new(target.group(), target.target()).with_allocation(amount);
                DecisionOption::new(
                    DecisionDescriptor::ChooseTriggerTargetGroups { trigger, targets },
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        self.scoped_decision_context(
            DecisionKind::NumericValue,
            controller,
            options,
            trigger_target_path_discriminator(
                controller,
                trigger,
                stack_position,
                target_index,
                1,
                prior,
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn select_trigger_target_groups(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        prior: &[AnnouncedTarget],
        prompt: &'static str,
        telemetry_kind: &'static str,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(Vec<AnnouncedTarget>, CanonicalActionId), String> {
        let selected_id = if human == Some(controller) {
            let labels = context
                .options()
                .iter()
                .map(|option| match option.descriptor() {
                    DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. }
                        if targets == prior =>
                    {
                        Ok("Finish this target group".to_owned())
                    }
                    DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. }
                        if targets.len() == prior.len() + 1 =>
                    {
                        let target = targets.last().ok_or_else(|| {
                            format!("seed {} target announcement is empty", self.seed)
                        })?;
                        Ok(format!(
                            "Target {}",
                            self.target_choice_label(target.target())
                        ))
                    }
                    DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. } => {
                        let changed = targets
                            .iter()
                            .zip(prior)
                            .find(|(next, current)| next.allocation() != current.allocation())
                            .map(|(next, _)| *next)
                            .ok_or_else(|| {
                                format!(
                                    "seed {} target-allocation option does not change its prefix",
                                    self.seed
                                )
                            })?;
                        Ok(format!(
                            "Assign {} to {}",
                            changed.allocation().unwrap_or_default(),
                            self.target_choice_label(changed.target())
                        ))
                    }
                    descriptor => Err(format!(
                        "seed {} cannot label trigger-target descriptor {descriptor:?}",
                        self.seed
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let source = decisions.as_deref_mut().ok_or_else(|| {
                "human game is missing a decision source for trigger targets".to_owned()
            })?;
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(source, prompt, context, &labels, episode)?
            } else {
                self.prompt_context_choice(source, prompt, context, &labels)?
            }
        } else if let Some(policies) = ai_policies {
            let decision_started = Instant::now();
            let policy = self.policy_for(controller, policies)?;
            let (selected_id, decision, policy_name, candidates) = if context.options().len() == 1 {
                (context.options()[0].id(), None, "forced-v1", Vec::new())
            } else {
                let candidates = if matches!(policy, AiController::Random(_)) {
                    Vec::new()
                } else {
                    self.policy_candidates(context, controller, |_| 0)?
                };
                let (selected_id, decision, policy_name) =
                    self.select_ai_action(policy, context, &candidates, telemetry_kind)?;
                (selected_id, decision, policy_name, candidates)
            };
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
                episode,
            });
            selected_id
        } else {
            context
                .options()
                .first()
                .map(DecisionOption::id)
                .ok_or_else(|| format!("seed {} trigger target has no options", self.seed))?
        };
        let selected = context.select(selected_id).map_err(|error| {
            format!(
                "seed {} selected an illegal {telemetry_kind} action: {error}",
                self.seed
            )
        })?;
        let DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. } = selected.descriptor()
        else {
            return Err(format!(
                "seed {} trigger-target context returned a non-target descriptor",
                self.seed
            ));
        };
        Ok((targets.clone(), selected_id))
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

    #[allow(clippy::too_many_arguments)]
    fn select_trigger_order_choice(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        position: usize,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(TriggerId, CanonicalActionId), String> {
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
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(
                    source,
                    "Order simultaneous triggers",
                    context,
                    &labels,
                    episode,
                )?
            } else {
                self.prompt_context_choice(source, "Order simultaneous triggers", context, &labels)?
            }
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
                episode,
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
        let trigger = triggers
            .last()
            .copied()
            .ok_or_else(|| format!("seed {} trigger-order selection is empty", self.seed))?;
        Ok((trigger, selected_id))
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
        let has_target_prompts = pending.iter().try_fold(false, |found, pending| {
            let Some(runtime) = self.trigger_programs.get(&pending.trigger()) else {
                return Ok::<_, String>(found);
            };
            let ability = runtime
                .program
                .triggered_abilities()
                .get(runtime.ability_index)
                .ok_or_else(|| {
                    format!(
                        "seed {} missing triggered runtime {} on {}",
                        self.seed,
                        runtime.ability_index,
                        runtime.program.name()
                    )
                })?;
            Ok(found || !ability.target_requirements().is_empty())
        })?;
        if !has_controlled_choice && !has_target_prompts {
            let order = apnap
                .iter()
                .flat_map(|controller| {
                    pending
                        .iter()
                        .filter(move |trigger| trigger.controller() == *controller)
                        .map(|trigger| trigger.trigger())
                })
                .collect::<Vec<_>>();
            let ordered = self.ordered_pending_trigger_instances(&pending, &order)?;
            let contexts = self.prepare_trigger_stack_contexts(&ordered, None)?;
            return self.dispatch_trigger_stack_action(
                Action::PutPendingTriggeredAbilitiesOnStack,
                contexts,
            );
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
            let mut episode = None;
            while remaining.iter().copied().collect::<BTreeSet<_>>().len() > 1 {
                let context =
                    self.trigger_order_context(controller, &remaining, &controller_prefix, &order)?;
                if episode.is_none() {
                    episode = self.begin_controlled_episode(
                        controller,
                        human,
                        decisions,
                        ai_policies,
                        &context,
                    )?;
                }
                let episode_metadata = episode.as_ref().map(|episode| {
                    if episode.selected_action_ids.is_empty() {
                        episode.root_metadata()
                    } else {
                        episode.child_metadata(&context)
                    }
                });
                let (selected, selected_id) = self.select_trigger_order_choice(
                    &context,
                    controller,
                    human,
                    decisions,
                    ai_policies,
                    order.len() + 1,
                    episode_metadata.clone(),
                )?;
                if let Some(episode) = episode.as_mut() {
                    episode.record_selection(
                        &context,
                        selected_id,
                        episode_metadata.map_or(0, |metadata| metadata.path_depth),
                    );
                }
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
            self.complete_controlled_episode(
                controller,
                human,
                decisions,
                ai_policies,
                episode.as_ref(),
            )?;
            order.extend(remaining);
        }
        if !has_target_prompts {
            let ordered = self.ordered_pending_trigger_instances(&pending, &order)?;
            let contexts = self.prepare_trigger_stack_contexts(&ordered, None)?;
            return self.dispatch_trigger_stack_action(
                Action::PutPendingTriggeredAbilitiesOnStackInOrder { order },
                contexts,
            );
        }

        let mut bindings = Vec::with_capacity(order.len());
        let mut prospective_stack_entries = Vec::with_capacity(order.len());
        for (stack_position, trigger) in order.iter().copied().enumerate() {
            let controller = pending
                .iter()
                .find(|pending| pending.trigger() == trigger)
                .map(|pending| pending.controller())
                .ok_or_else(|| {
                    format!(
                        "seed {} ordered trigger {} is not pending",
                        self.seed,
                        trigger.index()
                    )
                })?;
            let Some(runtime) = self.trigger_programs.get(&trigger) else {
                bindings.push(TriggerStackBinding::new(trigger));
                prospective_stack_entries.push(
                    self.prospective_trigger_stack_entry_id(prospective_stack_entries.len())?,
                );
                continue;
            };
            let ability = runtime
                .program
                .triggered_abilities()
                .get(runtime.ability_index)
                .ok_or_else(|| {
                    format!(
                        "seed {} missing triggered runtime {} on {}",
                        self.seed,
                        runtime.ability_index,
                        runtime.program.name()
                    )
                })?;
            let source = runtime.source;
            let requirements = ability.target_requirements().to_vec();
            if requirements.iter().copied().any(|requirement| {
                let legal = self.trigger_target_choices(
                    controller,
                    source,
                    requirement,
                    &prospective_stack_entries,
                );
                let minimum = if requirement.allocation_total().is_some() {
                    usize::from(requirement.minimum()).max(1)
                } else {
                    usize::from(requirement.minimum())
                };
                legal.len() < minimum
                    || requirement
                        .allocation_total()
                        .is_some_and(|total| total < u32::from(requirement.minimum()))
            }) {
                bindings.push(TriggerStackBinding::no_legal_targets(trigger, requirements));
                continue;
            }
            let mut targets = Vec::new();
            let mut target_episode = None;
            for (group_index, requirement) in requirements.iter().copied().enumerate() {
                let group = u8::try_from(group_index)
                    .map_err(|_| format!("seed {} trigger target group overflow", self.seed))?;
                let legal_count = self
                    .trigger_target_choices(
                        controller,
                        source,
                        requirement,
                        &prospective_stack_entries,
                    )
                    .len();
                let allocation_limit = requirement
                    .allocation_total()
                    .map_or(usize::MAX, |total| total as usize);
                let maximum = usize::from(requirement.maximum())
                    .min(legal_count)
                    .min(allocation_limit);
                let group_start = targets.len();
                while targets.len() - group_start < maximum {
                    let context = self.trigger_target_context(
                        controller,
                        source,
                        trigger,
                        (stack_position, group_index),
                        &targets,
                        requirement,
                        &prospective_stack_entries,
                    )?;
                    if target_episode.is_none() {
                        target_episode = self.begin_controlled_episode(
                            controller,
                            human,
                            decisions,
                            ai_policies,
                            &context,
                        )?;
                    }
                    let episode_metadata = target_episode.as_ref().map(|episode| {
                        if episode.selected_action_ids.is_empty() {
                            episode.root_metadata()
                        } else {
                            episode.child_metadata(&context)
                        }
                    });
                    let (next, selected_id) = self.select_trigger_target_groups(
                        &context,
                        controller,
                        human,
                        decisions,
                        ai_policies,
                        &targets,
                        "Choose triggered ability targets",
                        "trigger_target",
                        episode_metadata.clone(),
                    )?;
                    if let Some(episode) = target_episode.as_mut() {
                        episode.record_selection(
                            &context,
                            selected_id,
                            episode_metadata.map_or(0, |metadata| metadata.path_depth),
                        );
                    }
                    if next == targets {
                        break;
                    }
                    targets = next;
                }
                let selected = targets.len() - group_start;
                let minimum = if requirement.allocation_total().is_some() {
                    usize::from(requirement.minimum()).max(1)
                } else {
                    usize::from(requirement.minimum())
                };
                if selected < minimum {
                    return Err(format!(
                        "seed {} trigger {} selected {selected} targets for group {group_index}, below effective minimum {minimum}",
                        self.seed,
                        trigger.index()
                    ));
                }
                if let Some(total) = requirement.allocation_total() {
                    let group_end = targets.len();
                    let mut remaining = total;
                    for target_index in group_start..group_end {
                        let remaining_members = group_end - target_index - 1;
                        let maximum = remaining
                            .checked_sub(remaining_members as u32)
                            .ok_or_else(|| {
                                format!(
                                    "seed {} trigger {} cannot allocate {total} across {selected} targets",
                                    self.seed,
                                    trigger.index()
                                )
                            })?;
                        let amount = if target_index + 1 == group_end || maximum == 1 {
                            maximum
                        } else {
                            let context = self.trigger_target_allocation_context(
                                controller,
                                trigger,
                                stack_position,
                                target_index,
                                &targets,
                                1,
                                maximum,
                            )?;
                            if target_episode.is_none() {
                                target_episode = self.begin_controlled_episode(
                                    controller,
                                    human,
                                    decisions,
                                    ai_policies,
                                    &context,
                                )?;
                            }
                            let episode_metadata = target_episode.as_ref().map(|episode| {
                                if episode.selected_action_ids.is_empty() {
                                    episode.root_metadata()
                                } else {
                                    episode.child_metadata(&context)
                                }
                            });
                            let (next_targets, selected_id) = self.select_trigger_target_groups(
                                &context,
                                controller,
                                human,
                                decisions,
                                ai_policies,
                                &targets,
                                "Allocate the triggered ability's target amount",
                                "trigger_target_allocation",
                                episode_metadata.clone(),
                            )?;
                            targets = next_targets;
                            if let Some(episode) = target_episode.as_mut() {
                                episode.record_selection(
                                    &context,
                                    selected_id,
                                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                                );
                            }
                            targets[target_index].allocation().ok_or_else(|| {
                                format!(
                                    "seed {} trigger {} allocation choice omitted its amount",
                                    self.seed,
                                    trigger.index()
                                )
                            })?
                        };
                        targets[target_index] =
                            AnnouncedTarget::new(group, targets[target_index].target())
                                .with_allocation(amount);
                        remaining = remaining.checked_sub(amount).ok_or_else(|| {
                            format!(
                                "seed {} trigger {} target allocation exceeded {total}",
                                self.seed,
                                trigger.index()
                            )
                        })?;
                    }
                    if remaining != 0 {
                        return Err(format!(
                            "seed {} trigger {} left {remaining} target allocation unassigned",
                            self.seed,
                            trigger.index()
                        ));
                    }
                }
            }
            self.complete_controlled_episode(
                controller,
                human,
                decisions,
                ai_policies,
                target_episode.as_ref(),
            )?;
            let (target_requirements, target_choices) =
                expand_announced_targets(&requirements, &targets).map_err(|error| {
                    format!(
                        "seed {} trigger {} target expansion failed: {error}",
                        self.seed,
                        trigger.index()
                    )
                })?;
            bindings.push(
                TriggerStackBinding::new(trigger)
                    .with_targets(target_requirements, target_choices)
                    .with_decisions(StackDecisionBindings::default()),
            );
            prospective_stack_entries
                .push(self.prospective_trigger_stack_entry_id(prospective_stack_entries.len())?);
        }
        let ordered = self.ordered_pending_trigger_instances(&pending, &order)?;
        let contexts = self.prepare_trigger_stack_contexts(&ordered, Some(&bindings))?;
        self.dispatch_trigger_stack_action(
            Action::PutPendingTriggeredAbilitiesOnStackWithChoices { bindings },
            contexts,
        )
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
        let target_requirements = record
            .targets()
            .iter()
            .map(|target| target.requirement())
            .collect::<Vec<_>>();
        let targets = record
            .targets()
            .iter()
            .map(|target| target.choice())
            .collect::<Vec<_>>();
        let target_legalities = record.legal_targets().to_vec();
        let decisions = record.decisions();
        let triggering_player = self.triggering_players_by_stack_entry.remove(&entry);
        if let Some(object) = object {
            if let Some(conditional_triggers) = self.conditional_cast_triggers.remove(&object) {
                let entered_battlefield = outcome == ResolutionOutcome::Resolved
                    && self.state.object_zone(object)
                        == Some(ZoneId::new(None, ZoneKind::Battlefield));
                for trigger in conditional_triggers {
                    let queued = entered_battlefield
                        && self
                            .state
                            .pending_triggers()
                            .iter()
                            .any(|pending| pending.trigger() == trigger);
                    if queued {
                        continue;
                    }
                    self.dispatch(Action::UnregisterTriggeredAbility { trigger })?;
                    self.trigger_programs.remove(&trigger);
                }
            }
        }
        if outcome != ResolutionOutcome::Resolved {
            return Ok(());
        }
        if let Some(trigger) = trigger {
            return self.execute_trigger(
                controller,
                trigger,
                triggering_player,
                target_requirements,
                targets,
                target_legalities,
                decisions,
            );
        }
        if triggering_player.is_some() {
            return Err(format!(
                "seed {} non-trigger stack entry {} carried trigger event context",
                self.seed,
                entry.index()
            ));
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
                target_requirements,
                targets,
                target_legalities,
                decisions,
            };
            if requires_object_choices {
                if self.pending_spell_resolution.is_some()
                    || self.pending_activated_resolution.is_some()
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
            let pending = PendingSpellResolution {
                controller,
                object,
                program,
                target_requirements,
                targets,
                target_legalities,
                decisions,
            };
            if !self.pending_spell_requirements(&pending)?.is_empty() {
                if self.pending_spell_resolution.is_some()
                    || self.pending_activated_resolution.is_some()
                    || self.pending_triggered_resolution.is_some()
                {
                    return Err(format!(
                        "seed {} attempted to overlap deferred resolution choices",
                        self.seed
                    ));
                }
                self.pending_spell_resolution = Some(pending);
                return Ok(());
            }
            let actions = self.pending_spell_actions(&pending, Vec::new())?;
            let action_count = actions.len() as u64;
            for action in actions {
                self.dispatch(action)?;
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

    #[allow(clippy::too_many_arguments)]
    fn execute_trigger(
        &mut self,
        controller: PlayerId,
        trigger: TriggerId,
        triggering_player: Option<PlayerId>,
        target_requirements: Vec<TargetRequirement>,
        targets: Vec<TargetChoice>,
        target_legalities: Vec<bool>,
        decisions: StackDecisionBindings,
    ) -> Result<(), String> {
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
        let requires_object_choices = !ability.object_choice_requirements().is_empty();
        let requires_optional_choices = ability.optional_choice_count() != 0;
        let requires_unless_choice = ability.unless_paid().is_some();
        let pending = PendingTriggeredResolution {
            controller,
            trigger,
            triggering_player,
            runtime,
            target_requirements,
            targets,
            target_legalities,
            decisions,
        };
        let requires_deferred_choices =
            requires_object_choices || requires_optional_choices || requires_unless_choice;
        if requires_deferred_choices {
            if self.pending_spell_resolution.is_some()
                || self.pending_activated_resolution.is_some()
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
        let actions = self.pending_triggered_actions(&pending, Vec::new(), &[])?;
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
        let mut episode = None;
        for (cursor, attacker) in objects.iter().copied().enumerate() {
            let decision_started = Instant::now();
            let resource_started =
                matches!(policy, AiController::Search(_)).then(ResourceSnapshot::capture);
            let context = self.attack_assignment_context(active, &objects, cursor, &attacks)?;
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    self.ai_decisions.len() as u64,
                    &context,
                ));
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
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
                    episode_metadata.clone(),
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
                    episode: episode_metadata.clone(),
                });
            }
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
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
        if let Some(episode) = episode.as_ref() {
            self.complete_ai_episode(episode)?;
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
        let mut episode = None;
        for (cursor, attacker) in objects.iter().copied().enumerate() {
            let context = self.attack_assignment_context(active, &objects, cursor, &attacks)?;
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    source.decision_count(),
                    &context,
                ));
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
            let labels = context
                .options()
                .iter()
                .map(|option| self.attack_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id = self.prompt_context_choice_in_episode(
                source,
                "Assign attacker",
                &context,
                &labels,
                episode_metadata
                    .clone()
                    .ok_or_else(|| "attack episode was not initialized".to_owned())?,
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
            }
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
        if let Some(episode) = episode.as_ref() {
            source.complete_episode(
                &episode.metadata.decision_episode_id,
                &episode.final_action_id(),
            )?;
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
        let mut episode = None;
        for (cursor, blocker) in objects.iter().copied().enumerate() {
            let decision_started = Instant::now();
            let resource_started =
                matches!(policy, AiController::Search(_)).then(ResourceSnapshot::capture);
            let context =
                self.block_assignment_context(defending_player, &objects, cursor, &blocks)?;
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    self.ai_decisions.len() as u64,
                    &context,
                ));
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
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
                    episode_metadata.clone(),
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
                    episode: episode_metadata.clone(),
                });
            }
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
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
        if let Some(episode) = episode.as_ref() {
            self.complete_ai_episode(episode)?;
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
        let mut episode = None;
        for (cursor, blocker) in objects.iter().copied().enumerate() {
            let context =
                self.block_assignment_context(defending_player, &objects, cursor, &blocks)?;
            if episode.is_none() {
                episode = Some(DecisionEpisodePath::root(
                    self.seed,
                    source.decision_count(),
                    &context,
                ));
            }
            let episode_metadata = episode.as_ref().map(|episode| {
                if episode.selected_action_ids.is_empty() {
                    episode.root_metadata()
                } else {
                    episode.child_metadata(&context)
                }
            });
            let labels = context
                .options()
                .iter()
                .map(|option| self.block_choice_label(option.descriptor()))
                .collect::<Result<Vec<_>, _>>()?;
            let selected_id = self.prompt_context_choice_in_episode(
                source,
                "Assign blocker",
                &context,
                &labels,
                episode_metadata
                    .clone()
                    .ok_or_else(|| "block episode was not initialized".to_owned())?,
            )?;
            if let Some(episode) = episode.as_mut() {
                episode.record_selection(
                    &context,
                    selected_id,
                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                );
            }
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
        if let Some(episode) = episode.as_ref() {
            source.complete_episode(
                &episode.metadata.decision_episode_id,
                &episode.final_action_id(),
            )?;
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
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(CombatDamageTarget, CanonicalActionId), String> {
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
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(
                    source,
                    "Order combat-damage targets",
                    context,
                    &labels,
                    episode,
                )?
            } else {
                self.prompt_context_choice(source, "Order combat-damage targets", context, &labels)?
            }
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
                episode,
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
        let target = targets
            .last()
            .copied()
            .and_then(combat_damage_choice_target)
            .ok_or_else(|| format!("seed {} combat-damage order selection is empty", self.seed))?;
        Ok((target, selected_id))
    }

    fn select_combat_damage_amount(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<(u32, CanonicalActionId), String> {
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
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(
                    source,
                    "Assign combat damage",
                    context,
                    &labels,
                    episode,
                )?
            } else {
                self.prompt_context_choice(source, "Assign combat damage", context, &labels)?
            }
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
                episode,
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
        Ok((*amount, selected_id))
    }

    fn select_combat_damage_amount_range(
        &mut self,
        context: &DecisionContext,
        controller: PlayerId,
        human: Option<PlayerId>,
        decisions: &mut Option<&mut dyn DecisionSource>,
        ai_policies: Option<&SeatPolicies>,
        episode: Option<DecisionEpisodeMetadata>,
    ) -> Result<((u32, u32), CanonicalActionId), String> {
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
            if let Some(episode) = episode.clone() {
                self.prompt_context_choice_in_episode(
                    source,
                    "Narrow combat damage",
                    context,
                    &labels,
                    episode,
                )?
            } else {
                self.prompt_context_choice(source, "Narrow combat damage", context, &labels)?
            }
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
                episode,
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
        Ok(((*minimum, *maximum), selected_id))
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
            let mut episode = None;
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
                    if episode.is_none() {
                        episode = self.begin_controlled_episode(
                            controller,
                            human,
                            decisions,
                            ai_policies,
                            &context,
                        )?;
                    }
                    let episode_metadata = episode.as_ref().map(|episode| {
                        if episode.selected_action_ids.is_empty() {
                            episode.root_metadata()
                        } else {
                            episode.child_metadata(&context)
                        }
                    });
                    let (selected, selected_id) = self.select_combat_damage_order_choice(
                        &context,
                        controller,
                        human,
                        decisions,
                        ai_policies,
                        episode_metadata.clone(),
                    )?;
                    if let Some(episode) = episode.as_mut() {
                        episode.record_selection(
                            &context,
                            selected_id,
                            episode_metadata.map_or(0, |metadata| metadata.path_depth),
                        );
                    }
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
                            if episode.is_none() {
                                episode = self.begin_controlled_episode(
                                    controller,
                                    human,
                                    decisions,
                                    ai_policies,
                                    &context,
                                )?;
                            }
                            let episode_metadata = episode.as_ref().map(|episode| {
                                if episode.selected_action_ids.is_empty() {
                                    episode.root_metadata()
                                } else {
                                    episode.child_metadata(&context)
                                }
                            });
                            let (next_bounds, selected_id) = self
                                .select_combat_damage_amount_range(
                                    &context,
                                    controller,
                                    human,
                                    decisions,
                                    ai_policies,
                                    episode_metadata.clone(),
                                )?;
                            bounds = next_bounds;
                            if let Some(episode) = episode.as_mut() {
                                episode.record_selection(
                                    &context,
                                    selected_id,
                                    episode_metadata.map_or(0, |metadata| metadata.path_depth),
                                );
                            }
                        }
                        let context = self.combat_damage_amount_context(
                            controller,
                            source,
                            &ordered,
                            &source_assignments,
                            cursor,
                            bounds,
                        )?;
                        if episode.is_none() {
                            episode = self.begin_controlled_episode(
                                controller,
                                human,
                                decisions,
                                ai_policies,
                                &context,
                            )?;
                        }
                        let episode_metadata = episode.as_ref().map(|episode| {
                            if episode.selected_action_ids.is_empty() {
                                episode.root_metadata()
                            } else {
                                episode.child_metadata(&context)
                            }
                        });
                        let (amount, selected_id) = self.select_combat_damage_amount(
                            &context,
                            controller,
                            human,
                            decisions,
                            ai_policies,
                            episode_metadata.clone(),
                        )?;
                        if let Some(episode) = episode.as_mut() {
                            episode.record_selection(
                                &context,
                                selected_id,
                                episode_metadata.map_or(0, |metadata| metadata.path_depth),
                            );
                        }
                        amount
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
            self.complete_controlled_episode(
                controller,
                human,
                decisions,
                ai_policies,
                episode.as_ref(),
            )?;
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
                episode: None,
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
    const MAX_DECISIONS: u32 = 12;

    fn state(&self, driver: GameDriver) -> Result<MainSearchState, String> {
        self.state_for_window(driver, self.actor, MainSearchWindow::Main, 0)
    }

    fn state_for_window(
        &self,
        driver: GameDriver,
        actor: PlayerId,
        window: MainSearchWindow,
        decision_count: u32,
    ) -> Result<MainSearchState, String> {
        let (context, mappings) = match window {
            MainSearchWindow::Main => driver.main_decision_context(actor)?,
            MainSearchWindow::Priority => driver.priority_decision_context(actor)?,
        };
        self.state_with_context(driver, actor, window, decision_count, context, mappings)
    }

    fn state_with_context(
        &self,
        driver: GameDriver,
        actor: PlayerId,
        window: MainSearchWindow,
        decision_count: u32,
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
                        DecisionDescriptor::ChooseAdditionalCost { .. }
                        | DecisionDescriptor::ChooseActivationCostObjects { .. } => {
                            driver.additional_cost_choice_prior(option.descriptor())
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
            actor,
            window,
            decision_count,
            context: Arc::new(context),
            mappings: Arc::new(mappings),
            priors: Arc::new(priors),
        })
    }

    fn finish_state(&self, mut state: MainSearchState, decision_count: u32) -> MainSearchState {
        state.finished = true;
        state.decision_count = decision_count;
        state
    }

    fn trigger_placement_is_forced(&self, driver: &GameDriver) -> Result<bool, String> {
        let pending = driver.state.pending_triggers();
        for controller in driver.state.players().iter().map(|player| player.id()) {
            let distinct = pending
                .iter()
                .filter(|trigger| trigger.controller() == controller)
                .map(|trigger| trigger.trigger())
                .collect::<BTreeSet<_>>();
            if distinct.len() > 1 {
                return Ok(false);
            }
        }
        for pending in pending {
            let Some(runtime) = driver.trigger_programs.get(&pending.trigger()) else {
                return Ok(false);
            };
            let ability = runtime
                .program
                .triggered_abilities()
                .get(runtime.ability_index)
                .ok_or_else(|| {
                    format!(
                        "seed {} missing triggered runtime {} on {}",
                        driver.seed,
                        runtime.ability_index,
                        runtime.program.name()
                    )
                })?;
            if !ability.target_requirements().is_empty() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn advance_after_transition(
        &self,
        mut state: MainSearchState,
        decision_count: u32,
    ) -> Result<MainSearchState, String> {
        loop {
            if decision_count >= Self::MAX_DECISIONS
                || state.driver.state.game_outcome() != GameOutcome::InProgress
            {
                return Ok(self.finish_state(state, decision_count));
            }
            if state.driver.pending_spell_resolution.is_some()
                || state.driver.pending_activated_resolution.is_some()
                || state.driver.pending_triggered_resolution.is_some()
            {
                return Ok(self.finish_state(state, decision_count));
            }
            if !state.driver.state.pending_triggers().is_empty() {
                if !self.trigger_placement_is_forced(&state.driver)? {
                    return Ok(self.finish_state(state, decision_count));
                }
                let mut decisions = None;
                state
                    .driver
                    .put_pending_triggers_on_stack(None, &mut decisions, None)?;
                continue;
            }
            let Some(priority_actor) = state.driver.state.priority_player() else {
                return Ok(self.finish_state(state, decision_count));
            };
            if !matches!(
                state.driver.state.current_step(),
                Some(Step::PrecombatMain | Step::PostcombatMain)
            ) {
                return Ok(self.finish_state(state, decision_count));
            }
            let (context, mappings) = state.driver.priority_decision_context(priority_actor)?;
            let forced_pass = context.options().len() == 1
                && matches!(
                    context.options()[0].descriptor(),
                    DecisionDescriptor::PassPriority
                );
            if forced_pass {
                state.driver.pass_priority()?;
                continue;
            }
            return self.state_with_context(
                state.driver,
                priority_actor,
                MainSearchWindow::Priority,
                decision_count,
                context,
                mappings,
            );
        }
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
        let decision_count = state.decision_count.saturating_add(1);
        if let Some((context, mappings)) = next
            .driver
            .hierarchical_main_context(state.actor, &choice)?
        {
            return self.state_with_context(
                next.driver,
                state.actor,
                state.window,
                decision_count,
                context,
                mappings,
            );
        }
        let transitioned = next.driver.apply_main_choice(state.actor, choice)?;
        if decision_count >= Self::MAX_DECISIONS
            || next.driver.state.game_outcome() != GameOutcome::InProgress
        {
            return Ok(self.finish_state(next, decision_count));
        }
        if transitioned {
            self.advance_after_transition(next, decision_count)
        } else {
            self.state_for_window(next.driver, state.actor, state.window, decision_count)
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
            .policy_candidates(&state.context, state.actor, |option| {
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

    fn selection_sign(&self, state: &Self::State) -> i8 {
        multiplayer_backup_sign(self.actor, state.actor, true)
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
            ^ u64::from(state.decision_count).rotate_left(17)
            ^ (state.actor.index() as u64).rotate_left(31)
            ^ match state.window {
                MainSearchWindow::Main => 0x8ebc_6af0_9c88_c6e3,
                MainSearchWindow::Priority => 0x5899_65cc_7537_4cc3,
            }
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
            && left.actor == right.actor
            && left.window == right.window
            && left.decision_count == right.decision_count
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

fn bounded_target_combinations(
    candidates: &[TargetChoice],
    minimum: usize,
    maximum: usize,
    limit: usize,
) -> Result<Vec<Vec<TargetChoice>>, String> {
    fn extend(
        candidates: &[TargetChoice],
        start: usize,
        remaining: usize,
        limit: usize,
        current: &mut Vec<TargetChoice>,
        output: &mut Vec<Vec<TargetChoice>>,
    ) -> Result<(), String> {
        if remaining == 0 {
            if output.len() >= limit {
                return Err(format!(
                    "target combinations exceed the {limit}-option canonical cap"
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

fn bounded_positive_allocations(
    total: u32,
    count: usize,
    limit: usize,
) -> Result<Vec<Vec<u32>>, String> {
    fn extend(
        remaining: u32,
        slots: usize,
        limit: usize,
        current: &mut Vec<u32>,
        output: &mut Vec<Vec<u32>>,
    ) -> Result<(), String> {
        if slots == 1 {
            if remaining == 0 {
                return Ok(());
            }
            if output.len() >= limit {
                return Err(format!(
                    "target allocations exceed the {limit}-option canonical cap"
                ));
            }
            current.push(remaining);
            output.push(current.clone());
            current.pop();
            return Ok(());
        }
        let remaining_slots = u32::try_from(slots - 1)
            .map_err(|_| "target allocation slot count exceeds u32".to_owned())?;
        if remaining <= remaining_slots {
            return Ok(());
        }
        let maximum = remaining - remaining_slots;
        for amount in 1..=maximum {
            current.push(amount);
            extend(remaining - amount, slots - 1, limit, current, output)?;
            current.pop();
        }
        Ok(())
    }

    if count == 0 || total < count as u32 {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    extend(total, count, limit, &mut Vec::new(), &mut output)?;
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
    fn priority_is_forced_pass(driver: &GameDriver) -> Result<bool, String> {
        let actor = driver
            .state
            .priority_player()
            .ok_or_else(|| format!("seed {} combat search has no priority actor", driver.seed))?;
        let (context, _) = driver.priority_decision_context(actor)?;
        Ok(context.options().len() == 1
            && matches!(
                context.options()[0].descriptor(),
                DecisionDescriptor::PassPriority
            ))
    }

    fn pass_forced_priority_through_step(
        driver: &mut GameDriver,
        step: Step,
    ) -> Result<bool, String> {
        while driver.state.current_step() == Some(step) {
            if !Self::priority_is_forced_pass(driver)? {
                return Ok(false);
            }
            driver.pass_priority()?;
            if !driver.state.pending_triggers().is_empty()
                || driver.pending_spell_resolution.is_some()
                || driver.pending_activated_resolution.is_some()
                || driver.pending_triggered_resolution.is_some()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn advance_unopposed_attack_to_damage(
        &self,
        state: &mut CombatSearchState,
    ) -> Result<(), String> {
        let CombatSearchProgress::Attackers { active, .. } = state.progress else {
            return Ok(());
        };
        if !Self::pass_forced_priority_through_step(&mut state.driver, Step::DeclareAttackers)?
            || state.driver.state.current_step() != Some(Step::DeclareBlockers)
        {
            return Ok(());
        }
        for defender in state.driver.current_defending_players(active) {
            if !state.driver.block_assignment_objects(defender)?.is_empty() {
                return Ok(());
            }
            state.driver.dispatch(Action::DeclareBlockers {
                defending_player: defender,
                blocks: Vec::new(),
            })?;
        }
        if !Self::pass_forced_priority_through_step(&mut state.driver, Step::DeclareBlockers)?
            || state.driver.state.current_step() != Some(Step::CombatDamage)
        {
            return Ok(());
        }
        let mut decisions = None;
        state
            .driver
            .assign_combat_damage(None, &mut decisions, None)?;
        Ok(())
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
            self.advance_unopposed_attack_to_damage(&mut next)?;
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

fn trigger_optional_path_discriminator(
    controller: PlayerId,
    trigger: TriggerId,
    cursor: usize,
    prior: &[bool],
) -> u64 {
    let mut state = combat_path_mix(0x7472_676f_7074_0001, controller.index() as u64);
    state = combat_path_mix(state, trigger.index() as u64);
    state = combat_path_mix(state, cursor as u64);
    for accept in prior {
        state = combat_path_mix(state, u64::from(*accept));
    }
    state
}

fn trigger_unless_path_discriminator(payer: PlayerId, trigger: TriggerId, stage: u64) -> u64 {
    let mut state = combat_path_mix(0x7472_6770_6179_0001, payer.index() as u64);
    state = combat_path_mix(state, trigger.index() as u64);
    combat_path_mix(state, stage)
}

const fn runtime_alternate_to_core(alternate: AlternateCostKind) -> SpellAlternateCost {
    match alternate {
        AlternateCostKind::Commander => SpellAlternateCost::Commander,
        AlternateCostKind::Flashback => SpellAlternateCost::Flashback,
        AlternateCostKind::Evoke => SpellAlternateCost::Evoke,
        AlternateCostKind::Overload => SpellAlternateCost::Overload,
    }
}

const fn core_alternate_to_runtime(alternate: SpellAlternateCost) -> AlternateCostKind {
    match alternate {
        SpellAlternateCost::Commander => AlternateCostKind::Commander,
        SpellAlternateCost::Flashback => AlternateCostKind::Flashback,
        SpellAlternateCost::Evoke => AlternateCostKind::Evoke,
        SpellAlternateCost::Overload => AlternateCostKind::Overload,
    }
}

const fn runtime_alternate_path_code(alternate: Option<AlternateCostKind>) -> u64 {
    match alternate {
        None => 0,
        Some(AlternateCostKind::Commander) => 1,
        Some(AlternateCostKind::Flashback) => 2,
        Some(AlternateCostKind::Evoke) => 3,
        Some(AlternateCostKind::Overload) => 4,
    }
}

fn activation_cost_path_discriminator(
    player: PlayerId,
    source: ObjectId,
    ability: ActivatedAbilityId,
    targets: &[AnnouncedTarget],
    optional: &[bool],
    sacrifice_objects: Option<&[ObjectId]>,
    stage: u64,
) -> u64 {
    let mut state = combat_path_mix(0x6163_7463_6f73_0001, player.index() as u64);
    state = combat_path_mix(state, source.index() as u64);
    state = combat_path_mix(state, u64::from(ability.get()));
    state = combat_path_mix(state, stage);
    for target in targets {
        state = mix_announced_target(state, *target);
    }
    for accept in optional {
        state = combat_path_mix(state, u64::from(*accept));
    }
    match sacrifice_objects {
        None => state = combat_path_mix(state, u64::MAX),
        Some(objects) => {
            state = combat_path_mix(state, objects.len() as u64);
            for object in objects {
                state = combat_path_mix(state, object.index() as u64);
            }
        }
    }
    state
}

fn mix_announced_target(mut state: u64, target: AnnouncedTarget) -> u64 {
    state = combat_path_mix(state, u64::from(target.group()));
    state = combat_path_mix(
        state,
        target
            .allocation()
            .map_or(0, |amount| u64::from(amount) + 1),
    );
    match target.target() {
        TargetChoice::Player(player) => {
            combat_path_mix(combat_path_mix(state, 0), player.index() as u64)
        }
        TargetChoice::Object(object) => {
            combat_path_mix(combat_path_mix(state, 1), object.index() as u64)
        }
        TargetChoice::StackEntry(entry) => {
            combat_path_mix(combat_path_mix(state, 2), entry.index() as u64)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn variable_cast_path_discriminator(
    player: PlayerId,
    object: ObjectId,
    alternate: Option<AlternateCostKind>,
    targets: &[AnnouncedTarget],
    mode: Option<u32>,
    optional: &[bool],
    additional_costs: &[Vec<ObjectId>],
    stage: u64,
    minimum: u32,
    maximum: u32,
) -> u64 {
    let mut state = combat_path_mix(0x7661_7263_6173_0001, player.index() as u64);
    state = combat_path_mix(state, object.index() as u64);
    state = combat_path_mix(state, runtime_alternate_path_code(alternate));
    state = combat_path_mix(state, stage);
    state = combat_path_mix(state, u64::from(minimum));
    state = combat_path_mix(state, u64::from(maximum));
    state = combat_path_mix(state, mode.map_or(u64::MAX, u64::from));
    for target in targets {
        state = mix_announced_target(state, *target);
    }
    for accept in optional {
        state = combat_path_mix(state, u64::from(*accept));
    }
    if !additional_costs.is_empty() {
        state = combat_path_mix(state, 0x6164_6463_6f73_7473);
        for cost in additional_costs {
            state = combat_path_mix(state, cost.len() as u64);
            for object in cost {
                state = combat_path_mix(state, object.index() as u64);
            }
        }
    }
    state
}

fn additional_cast_path_discriminator(
    player: PlayerId,
    object: ObjectId,
    alternate: Option<AlternateCostKind>,
    targets: &[AnnouncedTarget],
    mode: Option<u32>,
    optional: &[bool],
    prior: &[Vec<ObjectId>],
) -> u64 {
    let mut state = combat_path_mix(0x6164_6463_6173_0001, player.index() as u64);
    state = combat_path_mix(state, object.index() as u64);
    state = combat_path_mix(state, runtime_alternate_path_code(alternate));
    state = combat_path_mix(state, prior.len() as u64);
    state = combat_path_mix(state, mode.map_or(u64::MAX, u64::from));
    for target in targets {
        state = mix_announced_target(state, *target);
    }
    for accept in optional {
        state = combat_path_mix(state, u64::from(*accept));
    }
    for cost in prior {
        state = combat_path_mix(state, cost.len() as u64);
        for object in cost {
            state = combat_path_mix(state, object.index() as u64);
        }
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

fn trigger_target_path_discriminator(
    controller: PlayerId,
    trigger: TriggerId,
    stack_position: usize,
    cursor: usize,
    phase: u8,
    prior: &[AnnouncedTarget],
) -> u64 {
    let mut state = combat_path_mix(0x7472_6774_6172_0001, controller.index() as u64);
    state = combat_path_mix(state, trigger.index() as u64);
    state = combat_path_mix(state, stack_position as u64);
    state = combat_path_mix(state, cursor as u64);
    state = combat_path_mix(state, u64::from(phase));
    for target in prior {
        state = mix_announced_target(state, *target);
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
        DecisionDescriptor::BeginActivateProgramAbilityWithCosts {
            source,
            ability,
            targets,
            optional,
        } => json!({
            "kind": "begin_activate_program_ability_with_costs",
            "source_object_id": source.index(),
            "ability_id": ability.get(),
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
        DecisionDescriptor::BeginCastSpellAlternate {
            object,
            alternate,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "begin_cast_spell_alternate",
            "object_id": object.index(),
            "alternate": format!("{alternate:?}"),
            "targets": targets.iter().copied().map(target_value).collect::<Vec<_>>(),
            "modes": modes,
            "optional": optional
        }),
        DecisionDescriptor::ActivateProgramAbilityTargetGroups {
            source,
            ability,
            payment,
            targets,
            optional,
        } => json!({
            "kind": "activate_program_ability_target_groups",
            "source_object_id": source.index(),
            "ability_id": ability.get(),
            "payment": payment_value(*payment),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>(),
            "optional": optional
        }),
        DecisionDescriptor::BeginActivateProgramAbilityWithCostsTargetGroups {
            source,
            ability,
            targets,
            optional,
        } => json!({
            "kind": "begin_activate_program_ability_with_costs_target_groups",
            "source_object_id": source.index(),
            "ability_id": ability.get(),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>(),
            "optional": optional
        }),
        DecisionDescriptor::CastSpellTargetGroups {
            object,
            payment,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "cast_spell_target_groups",
            "object_id": object.index(),
            "payment": payment_value(*payment),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>(),
            "modes": modes,
            "optional": optional
        }),
        DecisionDescriptor::BeginCastSpellTargetGroups {
            object,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "begin_cast_spell_target_groups",
            "object_id": object.index(),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>(),
            "modes": modes,
            "optional": optional
        }),
        DecisionDescriptor::BeginCastSpellAlternateTargetGroups {
            object,
            alternate,
            targets,
            modes,
            optional,
        } => json!({
            "kind": "begin_cast_spell_alternate_target_groups",
            "object_id": object.index(),
            "alternate": format!("{alternate:?}"),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>(),
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
        DecisionDescriptor::ChooseTriggerTargetGroups { trigger, targets } => json!({
            "kind": "choose_trigger_target_groups",
            "trigger_id": trigger.get(),
            "targets": targets.iter().copied().map(announced_target_value).collect::<Vec<_>>()
        }),
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
        DecisionDescriptor::ChooseAdditionalCost { cost, objects } => json!({
            "kind": "choose_additional_cost",
            "cost": cost,
            "object_ids": objects.iter().map(|object| object.index()).collect::<Vec<_>>()
        }),
        DecisionDescriptor::ChooseActivationCostObjects { objects } => json!({
            "kind": "choose_activation_cost_objects",
            "object_ids": objects.iter().map(|object| object.index()).collect::<Vec<_>>()
        }),
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

fn announced_target_value(target: AnnouncedTarget) -> Value {
    json!({
        "group": target.group(),
        "target": target_value(target.target()),
        "allocation": target.allocation()
    })
}

fn announced_target_choices(targets: &[AnnouncedTarget]) -> Vec<TargetChoice> {
    targets.iter().map(|target| target.target()).collect()
}

fn has_grouped_target_semantics(
    requirements: &[TargetRequirement],
    targets: &[AnnouncedTarget],
) -> bool {
    requirements.iter().any(|requirement| {
        requirement.minimum() != 1
            || requirement.maximum() != 1
            || requirement.allocation_total().is_some()
    }) || targets.iter().enumerate().any(|(index, target)| {
        usize::from(target.group()) != index || target.allocation().is_some()
    })
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
            if left.normalized_benchmark_key.is_empty() || right.normalized_benchmark_key.is_empty()
            {
                left.normalized_benchmark_key.clear();
                right.normalized_benchmark_key.clear();
                left.normalized_player_view_hash.clear();
                right.normalized_player_view_hash.clear();
                left.normalized_legal_action_ids.clear();
                right.normalized_legal_action_ids.clear();
                left.benchmark_normalization_complete = false;
                right.benchmark_normalization_complete = false;
            }
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
    /// Diagnostic-only long-game and per-round progress telemetry.
    pub progress: GameProgressDiagnostics,
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
        progress: run.summary.progress,
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
    normalize_additive_progress(expected, &mut normalized);
    &normalized == expected
}

fn additive_progress_summary_matches(expected: &GameSummary, actual: &GameSummary) -> bool {
    let mut normalized = actual.clone();
    normalize_additive_progress(expected, &mut normalized);
    &normalized == expected
}

fn normalize_additive_progress(expected: &GameSummary, actual: &mut GameSummary) {
    if expected.metrics.meaningful_actions == 0 {
        actual.metrics.meaningful_actions = 0;
    }
    if expected.metrics.pass_only_priority_cycles == 0 {
        actual.metrics.pass_only_priority_cycles = 0;
    }
    if expected.metrics.table_damage_to_players == 0 {
        actual.metrics.table_damage_to_players = 0;
    }
    if expected.metrics.life_total_movement == 0 {
        actual.metrics.life_total_movement = 0;
    }
    if expected.progress == GameProgressDiagnostics::default() {
        actual.progress = GameProgressDiagnostics::default();
    }
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
    if !additive_progress_summary_matches(&replay.expected, &run.summary) {
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
        bounded_positive_allocations, bounded_target_combinations, campaign_seed,
        concession_decision_context, concession_decision_context_from_view,
        legacy_human_summary_matches, multiplayer_backup_sign, player_view_fingerprint,
        replay_captured_actions, replay_human_file, run_prompted_game, snapshot_prompt,
        ActivatedRuntime, AiController, AiDecisionRecord, CombatSearchDomain, CombatSearchProgress,
        DecisionEpisodeMetadata, DecisionPrompt, DecisionSelection, DecisionSource, GameDriver,
        GameMetrics, GameProgressDiagnostics, GameProgressTracker, GameSummary, HeuristicPolicy,
        IdentityExercise, MainChoice, MainSearchDomain, MainSearchWindow,
        PendingActivatedResolution, RandomLegalPolicy, RegisteredAbility, ReplayDecisionSource,
        TerminalDecisionSource, TraceMode, TraceRecord, CONCESSION_PROMPT, PLAYER_COUNT,
    };
    use forge_ai::{
        AiWeights, GuardrailProfile, GuardrailTable, SearchConfig, SearchDomain, SearchEngine,
    };
    use forge_cards::runtime::{compile_card_program, AlternateCostKind};
    use forge_core::{
        apply, AbilityPlayer, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect,
        ActivationCost, ActivationTiming, AnnouncedTarget, AttackDeclaration,
        BaseCreatureCharacteristics, BaseObjectCharacteristics, BasicLandTypes, BlockDeclaration,
        CardId, CombatDamageTarget, CreatureKeywords, DecisionContext, DecisionDescriptor,
        DecisionKind, DecisionOption, GameState, ManaCost, ManaPool, ObjectColors, ObjectId,
        ObjectSupertypes, ObjectTypes, Outcome, PaymentPlan, PlayerId, PlayerView,
        ResolutionOutcome, SpellAlternateCost, StackDecisionBindings, Step, TargetChoice,
        TargetKind, TargetRequirement, TriggerCondition, TriggerDefinition, TriggerPlayerFilter,
        TriggerStackDisposition, ZoneId, ZoneKind,
    };

    const FLAWLESS_MANEUVER: &str = r#"
card "Flawless Maneuver" {
  id: "4e183439-17d2-47ff-9d99-5e22821d91e3"
  layout: normal
  status: unverified_playable
  face "Flawless Maneuver" {
    cost: "{2}{W}"
    types: "Instant"
    oracle: "If you control a commander, you may cast this spell without paying its mana cost. Creatures you control gain indestructible until end of turn."
    keywords: []
    ability static {
      effect: while_condition(at_least(count(permanents(and(designation_is("commander"), controlled_by(you())))), 1), alternate_cost(spells(and(equals(any(), source()), controlled_by(you()))), mana_cost("{0}")))
    }
    ability spell {
      effect: grant_keyword(permanents(and(type_is("creature"), controlled_by(you()))), "indestructible", "until_end_of_turn")
    }
  }
}
"#;

    const FAITHLESS_LOOTING: &str = r#"
card "Faithless Looting" {
  id: "3d6fa57a-aa53-4b5c-b8af-a7612c823117"
  layout: normal
  status: unverified_playable
  face "Faithless Looting" {
    cost: "{R}"
    types: "Sorcery"
    oracle: "Draw two cards, then discard two cards. Flashback {2}{R}."
    keywords: [flashback]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{2}{R}")))
    }
    ability spell {
      effect: sequence(draw(2, you()), discard_cards(2, you(), "choose"))
    }
  }
}
"#;

    const CYCLONIC_RIFT: &str = r#"
card "Cyclonic Rift" {
  id: "d75b9c82-1b49-4c3e-a1b5-aeef57d6644b"
  layout: normal
  status: unverified_playable
  face "Cyclonic Rift" {
    cost: "{1}{U}"
    types: "Instant"
    oracle: "Return target nonland permanent you don't control to its owner's hand. Overload {6}{U}."
    keywords: [overload]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{6}{U}")))
    }
    ability spell {
      effect: return_to_hand(target(permanents(and(not(type_is("land")), not(controlled_by(you()))))))
    }
  }
}
"#;

    const MULLDRIFTER: &str = r#"
card "Mulldrifter" {
  id: "24d0f5e7-0d9e-4b76-900e-a7274e80312d"
  layout: normal
  status: unverified_playable
  face "Mulldrifter" {
    cost: "{4}{U}"
    types: "Creature - Elemental"
    oracle: "Flying. When Mulldrifter enters, draw two cards. Evoke {2}{U}."
    power: "2"
    toughness: "2"
    keywords: [evoke, flying]
    ability static {
      effect: continuous(source(), alternate_cost(source(), mana_cost("{2}{U}")))
    }
    ability triggered {
      event: event_enters(source())
      effect: draw(2, you())
    }
  }
}
"#;
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        env,
        io::Cursor,
        path::Path,
        sync::Arc,
    };

    fn assert_ai_episode_integrity(records: &[AiDecisionRecord]) {
        assert!(
            !records.is_empty(),
            "episode fixture must record a decision"
        );
        let mut episodes = BTreeMap::<&str, Vec<&AiDecisionRecord>>::new();
        for record in records {
            assert!(!record.episode.decision_episode_id.is_empty());
            assert!(!record.episode.root_context_id.is_empty());
            assert!(!record.episode.final_concrete_action_id.is_empty());
            assert_eq!(record.episode.is_forced, record.legal_actions == 1);
            episodes
                .entry(record.episode.decision_episode_id.as_str())
                .or_default()
                .push(record);
        }

        for episode in episodes.values() {
            assert_eq!(
                episode
                    .iter()
                    .filter(|record| record.episode.path_depth == 0)
                    .count(),
                1
            );
            assert_eq!(
                episode
                    .iter()
                    .filter(|record| record.episode.is_terminal_subchoice)
                    .count(),
                1
            );
            assert!(episode
                .last()
                .is_some_and(|record| record.episode.is_terminal_subchoice));
            assert_eq!(episode[0].episode.root_context_id, episode[0].context_id);
            assert_eq!(
                episode[0].episode.is_strategic_root,
                episode[0].legal_actions > 1
            );
            let final_id = &episode[0].episode.final_concrete_action_id;
            assert!(episode
                .iter()
                .all(|record| &record.episode.final_concrete_action_id == final_id));
            for (depth, record) in episode.iter().enumerate() {
                assert_eq!(record.episode.path_depth as usize, depth);
                if depth == 0 {
                    assert!(record.episode.parent_context_id.is_none());
                } else {
                    assert_eq!(
                        record.episode.parent_context_id.as_deref(),
                        Some(episode[depth - 1].context_id.as_str())
                    );
                    assert!(!record.episode.is_strategic_root);
                }
            }
        }
    }

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
    fn long_game_progress_tracks_round_activity_and_repetition_without_termination() {
        let mut tracker = GameProgressTracker::default();
        let mut metrics = GameMetrics::default();
        tracker.observe_state(41);
        tracker.observe_state(41);
        tracker.observe_turn(1, &metrics);
        tracker.record_meaningful_action(Some(0));
        tracker.record_elimination(3, 4);
        metrics.meaningful_actions = 2;
        metrics.casts = 1;
        metrics.table_damage_to_players = 7;
        metrics.life_total_movement = 9;
        metrics.pass_only_priority_cycles = 3;
        metrics.eliminations = 1;
        tracker.observe_turn(5, &metrics);

        let diagnostics = tracker.finish(5, &metrics, &[]);
        assert_eq!(diagnostics.termination_reason, "winner");
        assert!(!diagnostics.turn_cap_reached);
        assert_eq!(diagnostics.state_observations, 2);
        assert_eq!(diagnostics.repeated_full_state_hashes, 1);
        assert_eq!(diagnostics.repeated_full_state_hash_rate_ppm, 500_000);
        assert_eq!(diagnostics.no_progress_rounds, 1);
        assert_eq!(diagnostics.maximum_consecutive_no_progress_rounds, 1);
        assert_eq!(diagnostics.eliminations.len(), 1);
        assert_eq!(diagnostics.rounds.len(), 2);
        assert_eq!(diagnostics.rounds[0].last_turn, 4);
        assert_eq!(diagnostics.rounds[0].table_damage_to_players, 7);
        assert_eq!(diagnostics.rounds[0].active_players_with_progress, 1);
        assert!(!diagnostics.rounds[0].no_progress);
        assert!(diagnostics.rounds[1].no_progress);
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
            episode: DecisionEpisodeMetadata::root(7, 0, &context, true),
        };
        let record = snapshot_prompt(0, &prompt, 1);
        assert_eq!(record.context_id, context.id().to_string());
        assert_eq!(record.decision_state_key, context.state_key().to_string());
        assert_eq!(
            record.normalized_benchmark_key,
            context.normalized_benchmark_key().to_string()
        );
        assert_eq!(record.normalized_legal_action_ids.len(), 2);
        assert!(record.benchmark_normalization_complete);
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
            episode: DecisionEpisodeMetadata::root(7, 0, &context, true),
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
            episode: DecisionEpisodeMetadata::root(7, 0, &context, true),
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
            progress: GameProgressDiagnostics::default(),
        };
        let mut instrumented = expected.clone();
        instrumented.metrics.hidden_information_checks = 104;
        instrumented.metrics.meaningful_actions = 9;
        instrumented.metrics.pass_only_priority_cycles = 3;
        instrumented.metrics.table_damage_to_players = 7;
        instrumented.metrics.life_total_movement = 11;
        instrumented.progress.termination_reason = "winner".to_owned();
        instrumented.progress.state_observations = 12;
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
        let (mut driver, active, defenders, pieces) = combat_decision_driver();
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

        driver
            .declare_ai_attackers(active, AiController::Random(RandomLegalPolicy::new(8_040)))
            .unwrap_or_else(|error| panic!("AI attack declaration should succeed: {error}"));
        assert_eq!(driver.ai_decisions.len(), objects.len());
        assert_ai_episode_integrity(&driver.ai_decisions);
    }

    #[test]
    fn combat_search_advances_unopposed_attacks_through_damage() {
        let (mut driver, active, defenders, pieces) = combat_decision_driver();
        let fourth_commander = match driver.dispatch(Action::CreateObject {
            card: CardId::new(8_051),
            owner: defenders[2],
            controller: defenders[2],
            zone: ZoneId::new(None, ZoneKind::Command),
        }) {
            Ok(Outcome::ObjectCreated(object)) => object,
            other => panic!("unexpected fourth commander outcome: {other:?}"),
        };
        driver.commanders = vec![pieces[0], pieces[2], pieces[3], fourth_commander];
        let objects = Arc::new(
            driver
                .attack_assignment_objects(active)
                .unwrap_or_else(|error| panic!("attack objects should exist: {error}")),
        );
        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights should load: {error}"));
        let domain = CombatSearchDomain {
            root: &driver,
            actor: active,
            weights,
            progress: CombatSearchProgress::Attackers {
                active,
                objects,
                cursor: 0,
                declarations: Vec::new(),
            },
            guardrail_profile: GuardrailProfile::Standard,
        };
        let mut state = domain
            .determinize(8_050)
            .unwrap_or_else(|error| panic!("combat search should determinize: {error}"));
        for attacker in [pieces[0], pieces[1]] {
            let context = state
                .context
                .as_ref()
                .unwrap_or_else(|| panic!("incomplete attack path should have a context"));
            let attack = context
                .options()
                .iter()
                .find(|option| {
                    matches!(
                        option.descriptor(),
                        DecisionDescriptor::AssignAttacker {
                            attacker: selected,
                            defender: Some(defender),
                        } if *selected == attacker && *defender == defenders[2]
                    )
                })
                .unwrap_or_else(|| panic!("attacker should reach the unblocked defender"));
            state = domain
                .apply_action(&state, attack.id())
                .unwrap_or_else(|error| panic!("attack path should advance: {error}"));
        }
        assert!(state.finished);
        assert_eq!(state.driver.state.current_step(), Some(Step::CombatDamage));
        assert_eq!(
            state.driver.state.players()[defenders[2].index()].life(),
            16
        );
        assert_eq!(state.driver.metrics.combat_damage_events, 2);
        assert!(state
            .driver
            .actions
            .iter()
            .any(|action| matches!(action, Action::AssignCombatDamage { .. })));
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

        driver
            .declare_ai_blocks(
                defenders[0],
                AiController::Random(RandomLegalPolicy::new(8_041)),
            )
            .unwrap_or_else(|error| panic!("AI block declaration should succeed: {error}"));
        assert_eq!(driver.ai_decisions.len(), blockers.len());
        assert_ai_episode_integrity(&driver.ai_decisions);
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
        assert_ai_episode_integrity(&driver.ai_decisions);
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
            } if targets == &vec![AnnouncedTarget::new(0, TargetChoice::Player(opponent))]
                && optional.is_empty()
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
    fn partially_illegal_targets_skip_only_their_bound_effects() {
        let source = r#"card "Partial Target Legality" {
  id: "forge:test:runner-partial-target-legality"
  layout: normal
  status: unverified_playable
  face "Partial Target Legality" {
    cost: "{R}{W}"
    types: "Instant"
    oracle: "Deal 2 damage to target creature. Target opponent loses 1 life."
    keywords: []
    ability spell {
      effect: sequence(deal_damage(target(permanents(type_is("creature"))), 2), lose_life(1, target(opponent())))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("partial_target_legality.frs", source)
            .unwrap_or_else(|error| panic!("partial-target fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("partial-target fixture should compile: {error}")),
        );
        assert_eq!(program.target_requirements().len(), 2);

        let (mut driver, caster, opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: spell,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("partial-target spell base should apply: {error}"));
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let creature = driver
            .state
            .zone_objects(battlefield)
            .and_then(|objects| objects.first())
            .copied()
            .unwrap_or_else(|| panic!("fixture opponent creature should exist"));
        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("partial-target context should exist: {error}"));
        let selected = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell {
                        object,
                        targets,
                        ..
                    } if *object == spell
                        && targets == &vec![
                            TargetChoice::Object(creature),
                            TargetChoice::Player(opponent),
                        ]
                )
            })
            .unwrap_or_else(|| panic!("two-target cast should be canonical"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("two-target cast should have a typed adapter"));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));

        driver
            .dispatch(Action::MoveObject {
                object: creature,
                to: ZoneId::new(Some(opponent), ZoneKind::Graveyard),
            })
            .unwrap_or_else(|error| panic!("target removal should succeed: {error}"));
        resolve_top_for_two_players(&mut driver);

        let record = driver
            .state
            .resolution_log()
            .last()
            .unwrap_or_else(|| panic!("spell should have a resolution record"));
        assert_eq!(record.outcome(), ResolutionOutcome::Resolved);
        assert_eq!(record.legal_targets(), &[false, true]);
        assert_eq!(driver.state.players()[opponent.index()].life(), 19);
        assert!(driver.actions.iter().all(|action| !matches!(
            action,
            Action::DealDamage {
                target: CombatDamageTarget::Object(object),
                amount: 2,
                ..
            } if *object == creature
        )));
    }

    #[test]
    fn commander_alternate_cost_uses_the_shared_canonical_cast_hierarchy() {
        let (mut driver, caster, _opponent, spell, _) =
            alternate_spell_driver("flawless_maneuver.frs", FLAWLESS_MANEUVER);
        let choice = selected_alternate_cast(&driver, caster, spell, SpellAlternateCost::Commander);
        assert!(matches!(
            choice,
            MainChoice::Cast {
                alternate: Some(AlternateCostKind::Commander),
                ..
            }
        ));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("commander alternate should be on the stack"));
        assert_eq!(
            stack.decisions().alternate_cost(),
            Some(SpellAlternateCost::Commander)
        );
        assert_eq!(
            stack.payment().map(PaymentPlan::paid),
            Some(ManaPool::empty())
        );
    }

    #[test]
    fn flashback_is_offered_from_graveyard_and_exiles_after_resolution() {
        let (mut driver, caster, _opponent, spell, _) =
            alternate_spell_driver("faithless_looting.frs", FAITHLESS_LOOTING);
        driver
            .dispatch(Action::MoveObject {
                object: spell,
                to: ZoneId::new(Some(caster), ZoneKind::Graveyard),
            })
            .unwrap_or_else(|error| panic!("flashback setup move failed: {error}"));
        let choice = selected_alternate_cast(&driver, caster, spell, SpellAlternateCost::Flashback);
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let stack = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("flashback spell should be on the stack"));
        assert!(stack.flashback());
        assert_eq!(
            stack.decisions().alternate_cost(),
            Some(SpellAlternateCost::Flashback)
        );
        resolve_top_for_two_players(&mut driver);
        assert_eq!(
            driver.state.object_zone(spell),
            Some(ZoneId::new(None, ZoneKind::Exile))
        );
        assert!(driver.pending_spell_resolution.is_some());
    }

    #[test]
    fn overload_removes_targets_and_applies_to_each_matching_permanent() {
        let (mut driver, caster, opponent, spell, permanents) =
            alternate_spell_driver("cyclonic_rift.frs", CYCLONIC_RIFT);
        let choice = selected_alternate_cast(&driver, caster, spell, SpellAlternateCost::Overload);
        assert!(matches!(
            &choice,
            MainChoice::Cast {
                alternate: Some(AlternateCostKind::Overload),
                targets,
                ..
            } if targets.is_empty()
        ));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        resolve_top_for_two_players(&mut driver);
        let opponent_hand = ZoneId::new(Some(opponent), ZoneKind::Hand);
        assert_eq!(driver.state.object_zone(permanents[0]), Some(opponent_hand));
        assert_eq!(driver.state.object_zone(permanents[1]), Some(opponent_hand));
    }

    #[test]
    fn evoke_sacrifice_trigger_exists_only_for_an_evoke_cast() {
        let (mut normal, caster, _opponent, spell, _) =
            alternate_spell_driver("mulldrifter.frs", MULLDRIFTER);
        let (context, mappings) = normal
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("normal cast context should exist: {error}"));
        let normal_option = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell { object, .. } if *object == spell
                )
            })
            .unwrap_or_else(|| panic!("normal Mulldrifter cast should be canonical"));
        let normal_choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == normal_option.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("normal cast should have an adapter"));
        assert_eq!(normal.apply_main_choice(caster, normal_choice), Ok(true));
        assert!(normal.conditional_cast_triggers.is_empty());
        resolve_top_for_two_players(&mut normal);
        assert_eq!(
            normal.state.object_zone(spell),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert_eq!(normal.state.pending_triggers().len(), 1);
        let normal_trigger = normal.state.pending_triggers()[0].trigger();
        let runtime = normal
            .trigger_programs
            .get(&normal_trigger)
            .unwrap_or_else(|| panic!("normal trigger runtime should exist"));
        assert_eq!(
            runtime.program.triggered_abilities()[runtime.ability_index].required_alternate_cost(),
            None
        );

        let (mut evoked, caster, _opponent, spell, _) =
            alternate_spell_driver("mulldrifter.frs", MULLDRIFTER);
        let evoke_choice =
            selected_alternate_cast(&evoked, caster, spell, SpellAlternateCost::Evoke);
        assert_eq!(evoked.apply_main_choice(caster, evoke_choice), Ok(true));
        assert_eq!(
            evoked.conditional_cast_triggers.get(&spell).map(Vec::len),
            Some(1)
        );
        resolve_top_for_two_players(&mut evoked);
        assert_eq!(evoked.state.pending_triggers().len(), 2);
        assert!(!evoked.conditional_cast_triggers.contains_key(&spell));
        let outcome = evoked
            .dispatch(Action::PutPendingTriggeredAbilitiesOnStack)
            .unwrap_or_else(|error| panic!("evoke triggers should stack: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 2));
        resolve_top_for_two_players(&mut evoked);
        assert_eq!(
            evoked.state.object_zone(spell),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        resolve_top_for_two_players(&mut evoked);
    }

    #[test]
    fn countered_evoke_cast_retires_its_conditional_trigger() {
        let (mut driver, caster, _opponent, spell, _) =
            alternate_spell_driver("mulldrifter.frs", MULLDRIFTER);
        let choice = selected_alternate_cast(&driver, caster, spell, SpellAlternateCost::Evoke);
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        let entry = driver
            .state
            .stack_top()
            .map(|stack| stack.id())
            .unwrap_or_else(|| panic!("evoke spell should be on the stack"));
        let conditional = driver
            .conditional_cast_triggers
            .get(&spell)
            .and_then(|triggers| triggers.first())
            .copied()
            .unwrap_or_else(|| panic!("evoke trigger should be registered conditionally"));
        assert_eq!(
            driver.dispatch(Action::CounterStackEntry { entry }),
            Ok(Outcome::Applied)
        );
        assert!(!driver.conditional_cast_triggers.contains_key(&spell));
        assert!(!driver.trigger_programs.contains_key(&conditional));

        assert_eq!(
            driver.dispatch(Action::MoveObject {
                object: spell,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            }),
            Ok(Outcome::Applied)
        );
        assert_eq!(driver.state.pending_triggers().len(), 1);
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
    fn additional_spell_costs_use_canonical_human_ai_search_and_replay_paths() {
        let source = r#"card "Costly Insight" {
  id: "forge:test:costly-insight"
  layout: normal
  status: unverified_playable
  face "Costly Insight" {
    cost: "{R}{W}"
    types: "Instant"
    oracle: "As an additional cost to cast this spell, discard a card and sacrifice a creature. You gain 2 life."
    keywords: []
    ability spell {
      costs: [mana_cost("{R}{W}"), discard_cost(1, cards()), sacrifice(permanents(and(type_is("creature"), controlled_by(you()))), 1)]
      effect: gain_life(2, you())
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("costly_insight.frs", source)
            .unwrap_or_else(|error| panic!("additional-cost fixture should parse: {error}"));
        let program = Arc::new(
            forge_cards::runtime::compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("additional-cost fixture should compile: {error}")),
        );
        assert_eq!(program.additional_costs().len(), 2);
        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, program);
        let discarded = match apply(
            &mut driver.state,
            Action::CreateObject {
                card: CardId::new(903),
                owner: caster,
                controller: caster,
                zone: ZoneId::new(Some(caster), ZoneKind::Hand),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected discard setup outcome: {other:?}"),
        };
        let sacrificed = create_test_creature(&mut driver.state, caster, 904, 1, 1);

        let (root, root_mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("additional-cost root should build: {error}"));
        let begin = root
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginCastSpell { object, .. } if *object == spell
                )
            })
            .unwrap_or_else(|| panic!("additional-cost spell should defer its cast"));
        assert!(root.options().iter().all(|option| !matches!(
            option.descriptor(),
            DecisionDescriptor::CastSpell { object, .. } if *object == spell
        )));
        let begin_choice = root_mappings
            .iter()
            .find_map(|(id, choice)| (*id == begin.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("deferred additional-cost cast needs an adapter"));
        let replay_base = driver.clone();

        let (discard_context, discard_mappings) = driver
            .hierarchical_main_context(caster, &begin_choice)
            .unwrap_or_else(|error| panic!("discard context should build: {error}"))
            .unwrap_or_else(|| panic!("discard context should be deferred"));
        assert_eq!(discard_context.kind(), DecisionKind::Payment);
        let discard_option = discard_context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ChooseAdditionalCost { cost: 0, objects }
                        if objects == &vec![discarded]
                )
            })
            .unwrap_or_else(|| panic!("discard selection should be canonical"));
        let sacrifice_stage = discard_mappings
            .iter()
            .find_map(|(id, choice)| (*id == discard_option.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("discard selection needs an adapter"));
        let (sacrifice_context, sacrifice_mappings) = driver
            .hierarchical_main_context(caster, &sacrifice_stage)
            .unwrap_or_else(|error| panic!("sacrifice context should build: {error}"))
            .unwrap_or_else(|| panic!("sacrifice context should be deferred"));
        let sacrifice_option = sacrifice_context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ChooseAdditionalCost { cost: 1, objects }
                        if objects == &vec![sacrificed]
                )
            })
            .unwrap_or_else(|| panic!("sacrifice selection should be canonical"));
        let payment_stage = sacrifice_mappings
            .iter()
            .find_map(|(id, choice)| (*id == sacrifice_option.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("sacrifice selection needs an adapter"));
        let (payment_context, payment_mappings) = driver
            .hierarchical_main_context(caster, &payment_stage)
            .unwrap_or_else(|error| panic!("mana-payment context should build: {error}"))
            .unwrap_or_else(|| panic!("mana payment should be deferred"));
        assert!(payment_context.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChoosePayment { .. }
        )));
        let payment_option = payment_context
            .options()
            .first()
            .unwrap_or_else(|| panic!("additional-cost spell needs a mana payment"));
        let cast = payment_mappings
            .iter()
            .find_map(|(id, choice)| (*id == payment_option.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("mana payment needs a final cast adapter"));
        assert_eq!(driver.apply_main_choice(caster, cast), Ok(true));
        let graveyard = ZoneId::new(Some(caster), ZoneKind::Graveyard);
        assert_eq!(driver.state.object_zone(discarded), Some(graveyard));
        assert_eq!(driver.state.object_zone(sacrificed), Some(graveyard));
        assert_eq!(
            driver.state.object_zone(spell),
            Some(ZoneId::new(None, ZoneKind::Stack))
        );

        let mut human_driver = replay_base.clone();
        let mut input = Cursor::new(b"1\n1\n1\n".as_slice());
        let mut output = Vec::new();
        let mut terminal = TerminalDecisionSource::new(&mut input, &mut output);
        assert_eq!(
            human_driver.finish_human_main_choice(caster, &mut terminal, begin_choice.clone()),
            Ok(true)
        );
        let decisions = terminal.into_decisions();
        assert_eq!(decisions.len(), 3);
        assert!(decisions
            .iter()
            .all(|decision| decision.episode.decision_episode_id
                == decisions[0].episode.decision_episode_id));
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| decision.episode.is_terminal_subchoice)
                .count(),
            1
        );
        assert!(decisions.iter().all(|decision| {
            !decision.episode.final_concrete_action_id.is_empty()
                && decision.episode.final_concrete_action_id
                    == decisions[0].episode.final_concrete_action_id
        }));
        assert_eq!(decisions[0].prompt, "Choose an additional cost");
        assert_eq!(decisions[1].prompt, "Choose an additional cost");
        assert_eq!(decisions[2].prompt, "Choose a mana payment");

        let mut replay_driver = replay_base.clone();
        let mut replay = ReplayDecisionSource::new(decisions);
        assert_eq!(
            replay_driver.finish_human_main_choice(caster, &mut replay, begin_choice.clone()),
            Ok(true)
        );
        assert!(replay.finish().is_ok());
        assert_eq!(replay_driver.actions, human_driver.actions);
        assert_eq!(
            replay_driver.state.deterministic_hash(),
            human_driver.state.deterministic_hash()
        );

        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights should load: {error}"));
        let mut ai_driver = replay_base.clone();
        assert_eq!(
            ai_driver.finish_ai_main_choice(
                caster,
                AiController::Heuristic(HeuristicPolicy::rollout(weights, 905)),
                begin_choice.clone(),
            ),
            Ok(true)
        );
        assert_eq!(
            ai_driver
                .ai_decisions
                .iter()
                .filter(|record| record.kind == "additional_cost")
                .count(),
            2
        );
        assert!(ai_driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "payment"));

        let domain = MainSearchDomain {
            root: &replay_base,
            actor: caster,
            weights,
            rollout_seed: 906,
            guardrail_profile: GuardrailProfile::Standard,
        };
        let search_root = domain
            .state(replay_base.clone())
            .unwrap_or_else(|error| panic!("search root should build: {error}"));
        let discard_state = domain
            .apply_action(&search_root, begin.id())
            .unwrap_or_else(|error| panic!("search should enter discard context: {error}"));
        let search_discard = discard_state
            .context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ChooseAdditionalCost { cost: 0, objects }
                        if objects == &vec![discarded]
                )
            })
            .unwrap_or_else(|| panic!("search discard context should contain the chosen card"));
        let sacrifice_state = domain
            .apply_action(&discard_state, search_discard.id())
            .unwrap_or_else(|error| panic!("search should enter sacrifice context: {error}"));
        let payment_state = domain
            .apply_action(&sacrifice_state, sacrifice_state.context.options()[0].id())
            .unwrap_or_else(|error| panic!("search should enter mana payment: {error}"));
        let cast_state = domain
            .apply_action(&payment_state, payment_state.context.options()[0].id())
            .unwrap_or_else(|error| panic!("search should apply the cast: {error}"));
        assert!(cast_state.finished);
        assert_eq!(
            cast_state.driver.state.object_zone(discarded),
            Some(graveyard)
        );
        assert_eq!(
            cast_state.driver.state.object_zone(sacrificed),
            Some(graveyard)
        );
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
            .hierarchical_main_context(caster, &begin_choice)
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
            .hierarchical_main_context(caster, &payment_stage)
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
        assert_eq!(
            decisions
                .iter()
                .map(|decision| decision.episode.decision_episode_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| decision.episode.is_strategic_root)
                .count(),
            1
        );
        assert!(!decisions[0].episode.is_terminal_subchoice);
        assert!(decisions[1].episode.is_terminal_subchoice);
        assert!(decisions[1].episode.is_forced);
        assert_eq!(decisions[0].episode.path_depth, 0);
        assert_eq!(decisions[1].episode.path_depth, 1);
        assert_eq!(
            decisions[1].episode.parent_context_id.as_deref(),
            Some(decisions[0].context_id.as_str())
        );
        assert!(decisions.iter().all(|decision| {
            !decision.episode.final_concrete_action_id.is_empty()
                && decision.episode.final_concrete_action_id
                    == decisions[0].episode.final_concrete_action_id
        }));
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
            .variable_cast_numeric_context(caster, spell, None, &[], None, &[], &[], (0, u32::MAX))
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
            assert!(cast_state.driver.state.stack_top().is_none());
            assert_eq!(
                cast_state.driver.state.object_zone(spell),
                Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
            );
            assert_eq!(cast_state.driver.state.players()[caster.index()].life(), 21);
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
    fn main_search_crosses_opponent_priority_and_resolves_the_response_stack() {
        let (mut driver, caster, opponent, spell) = modal_spell_driver();
        let response_source = match driver.dispatch(Action::CreateObject {
            card: CardId::new(9_050),
            owner: opponent,
            controller: opponent,
            zone: ZoneId::new(None, ZoneKind::Battlefield),
        }) {
            Ok(Outcome::ObjectCreated(object)) => object,
            other => panic!("unexpected response source outcome: {other:?}"),
        };
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: response_source,
                base: BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_artifact(),
                    ObjectColors::none(),
                ),
            })
            .unwrap_or_else(|error| panic!("response source should be an artifact: {error}"));
        let definition = ActivatedAbilityDefinition::new(
            opponent,
            Some(response_source),
            ActivationTiming::Instant,
            ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_sacrifice_source(),
            ActivatedAbilityEffect::GainLife {
                player: AbilityPlayer::Controller,
                amount: 2,
            },
        );
        let ability = match driver.dispatch(Action::RegisterActivatedAbility {
            definition: Box::new(definition),
        }) {
            Ok(Outcome::ActivatedAbilityRegistered(ability)) => ability,
            other => panic!("unexpected response registration outcome: {other:?}"),
        };
        driver.activated_abilities.push(RegisteredAbility {
            source: response_source,
            controller: opponent,
            id: ability,
            runtime: None,
            benchmark_semantic_identity: "test/gain-life/0".to_owned(),
        });

        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights should load: {error}"));
        let domain = MainSearchDomain {
            root: &driver,
            actor: caster,
            weights,
            rollout_seed: 9_051,
            guardrail_profile: GuardrailProfile::Standard,
        };
        let root = domain
            .state(driver.clone())
            .unwrap_or_else(|error| panic!("search root should build: {error}"));
        let cast = root
            .context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell { object, modes, .. }
                        if *object == spell && modes == &vec![1]
                )
            })
            .unwrap_or_else(|| panic!("root should expose the targetless Charm mode"));
        let response = domain
            .apply_action(&root, cast.id())
            .unwrap_or_else(|error| panic!("cast should reach opponent priority: {error}"));
        assert!(!response.finished);
        assert_eq!(response.actor, opponent);
        assert_eq!(response.window, MainSearchWindow::Priority);
        assert_eq!(domain.selection_sign(&response), -1);
        assert_eq!(multiplayer_backup_sign(caster, opponent, false), 1);
        assert!(response.driver.state.stack_top().is_some());
        let activate = response
            .context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::ActivateAbility { source, .. }
                        if *source == response_source
                )
            })
            .unwrap_or_else(|| panic!("opponent response should be a canonical action"));
        let resolved = domain
            .apply_action(&response, activate.id())
            .unwrap_or_else(|error| panic!("response line should resolve: {error}"));
        assert!(resolved.finished);
        assert!(resolved.driver.state.stack_top().is_none());
        assert_eq!(
            resolved.driver.state.object_zone(response_source),
            Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard))
        );
        assert_eq!(resolved.driver.state.players()[opponent.index()].life(), 22);

        let report = SearchEngine::search(
            &domain,
            &root.context,
            &SearchConfig::fixed_iterations(9_052, 1, 64)
                .with_workers(1)
                .with_rollout_depth(8),
        )
        .unwrap_or_else(|error| panic!("bounded response search should run: {error}"));
        assert!(report.maximum_depth() >= 2);
    }

    #[test]
    fn main_search_places_and_resolves_a_forced_trigger_before_evaluation() {
        let source = r#"card "Search Trigger Fixture" {
  id: "forge:test:search-trigger-fixture"
  layout: normal
  status: unverified_playable
  face "Search Trigger Fixture" {
    cost: "{0}"
    types: "Creature - Human Wizard"
    oracle: "When this enters, you gain 3 life."
    power: "1"
    toughness: "1"
    keywords: []
    ability triggered {
      event: event_enters(source())
      effect: gain_life(3, you())
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("search_trigger_fixture.frs", source)
            .unwrap_or_else(|error| panic!("trigger fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("trigger fixture should compile: {error}")),
        );
        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: spell,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger fixture base should apply: {error}"));
        driver
            .dispatch(Action::SetBaseCreatureCharacteristics {
                object: spell,
                base: program
                    .base_creature()
                    .unwrap_or_else(|| panic!("trigger fixture should be a creature")),
            })
            .unwrap_or_else(|error| panic!("trigger fixture creature base should apply: {error}"));

        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights should load: {error}"));
        let domain = MainSearchDomain {
            root: &driver,
            actor: caster,
            weights,
            rollout_seed: 9_060,
            guardrail_profile: GuardrailProfile::Standard,
        };
        let root = domain
            .state(driver.clone())
            .unwrap_or_else(|error| panic!("trigger search root should build: {error}"));
        let cast = root
            .context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::CastSpell { object, .. } if *object == spell
                )
            })
            .unwrap_or_else(|| panic!("trigger fixture should be castable"));
        let resolved = domain
            .apply_action(&root, cast.id())
            .unwrap_or_else(|error| panic!("forced trigger line should resolve: {error}"));
        assert!(resolved.finished);
        assert!(resolved.driver.state.pending_triggers().is_empty());
        assert!(resolved.driver.state.stack_top().is_none());
        assert_eq!(resolved.driver.state.players()[caster.index()].life(), 23);
        assert_eq!(resolved.driver.metrics.triggers_resolved, 1);
        assert!(resolved
            .driver
            .actions
            .iter()
            .any(|action| matches!(action, Action::PutPendingTriggeredAbilitiesOnStack)));
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
        for _ in 0..3 {
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
        assert_eq!(context.options().len(), 3);
        assert!(context.options().iter().all(|option| {
            option.actions().is_empty()
                && matches!(
                    option.descriptor(),
                    DecisionDescriptor::OrderTriggers { triggers } if triggers.len() == 1
                )
        }));
        let pending = driver.state.pending_triggers().to_vec();
        let mut reversed = triggers.clone();
        reversed.reverse();
        let reordered = driver
            .ordered_pending_trigger_instances(&pending, &reversed)
            .unwrap_or_else(|error| panic!("explicit trigger order should bind: {error}"));
        assert_eq!(
            reordered
                .iter()
                .map(|pending| pending.trigger())
                .collect::<Vec<_>>(),
            reversed
        );

        let mut human_driver = driver.clone();
        let mut source = PickFirstChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        let human_outcome = human_driver
            .put_pending_triggers_on_stack(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human trigger order should succeed: {error}"));
        assert!(matches!(human_outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 3));
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::PutPendingTriggeredAbilitiesOnStackInOrder { order }) if order.len() == 3
        ));

        let policies = [AiController::Random(RandomLegalPolicy::new(17)); PLAYER_COUNT];
        let mut ai_driver = driver;
        let mut no_decisions = None;
        let ai_outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI trigger order should succeed: {error}"));
        assert!(matches!(ai_outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 3));
        assert_eq!(ai_driver.ai_decisions.len(), 2);
        assert_eq!(ai_driver.ai_decisions[0].kind, "trigger_order");
        assert!(ai_driver.ai_decisions[0]
            .canonical_legal_actions
            .iter()
            .any(|action| action.action_id == ai_driver.ai_decisions[0].action_id));
        assert_ai_episode_integrity(&ai_driver.ai_decisions);
    }

    #[test]
    fn triggered_targets_use_shared_human_and_ai_contexts_and_resolve_from_stack() {
        let source = r#"card "Queza Trigger Fixture" {
  id: "forge:test:queza-trigger-fixture"
  layout: normal
  status: unverified_playable
  face "Queza Trigger Fixture" {
    cost: "{1}{W}{U}{B}"
    types: "Legendary Creature - Octopus Advisor"
    oracle: "Whenever an opponent draws a card, target opponent loses 1 life."
    power: "3"
    toughness: "4"
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: lose_life(1, target(opponent()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("queza_trigger_fixture.frs", source)
            .unwrap_or_else(|error| panic!("trigger-target fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("trigger-target fixture should compile: {error}")),
        );
        assert_eq!(program.triggered_abilities().len(), 1);
        assert_eq!(
            program.triggered_abilities()[0].target_requirements().len(),
            1
        );

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtime should register: {error}"));
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_302),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("opponent draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw event should queue the trigger: {error}"));
        let trigger = driver
            .state
            .pending_triggers()
            .first()
            .map(|pending| pending.trigger())
            .unwrap_or_else(|| panic!("draw should queue the targeted trigger"));
        let requirement = program.triggered_abilities()[0].target_requirements()[0];
        let context = driver
            .trigger_target_context(
                controller,
                source_object,
                trigger,
                (0, 0),
                &[],
                requirement,
                &[],
            )
            .unwrap_or_else(|error| panic!("trigger target context should exist: {error}"));
        assert_eq!(context.kind(), DecisionKind::Target);
        assert_eq!(context.options().len(), 1);
        assert!(matches!(
            context.options()[0].descriptor(),
            DecisionDescriptor::ChooseTriggerTargetGroups { trigger: selected, targets }
                if *selected == trigger
                    && targets == &[AnnouncedTarget::new(
                        0,
                        TargetChoice::Player(opponent),
                    )]
        ));

        let mut ai_driver = driver.clone();
        let mut human_driver = driver;
        let mut source = PickFirstChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        let outcome = human_driver
            .put_pending_triggers_on_stack(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human trigger target should stack: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::PutPendingTriggeredAbilitiesOnStackWithChoices { bindings })
                if bindings.len() == 1
        ));
        assert_eq!(
            human_driver
                .state
                .stack_top()
                .unwrap_or_else(|| panic!("targeted trigger should be on the stack"))
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::Player(opponent)]
        );
        human_driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("controller pass should succeed: {error}"));
        human_driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve trigger: {error}"));
        assert_eq!(human_driver.state.players()[opponent.index()].life(), 19);

        let policies = [AiController::Random(RandomLegalPolicy::new(19)); PLAYER_COUNT];
        let mut no_decisions = None;
        let outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI trigger target should stack: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
        assert_eq!(ai_driver.ai_decisions.len(), 1);
        assert_eq!(ai_driver.ai_decisions[0].kind, "trigger_target");
        assert_eq!(
            ai_driver.ai_decisions[0].context_id,
            context.id().to_string()
        );
        assert_eq!(
            ai_driver
                .state
                .stack_top()
                .unwrap_or_else(|| panic!("AI targeted trigger should be on the stack"))
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::Player(opponent)]
        );
    }

    #[test]
    fn triggered_target_ranges_and_divided_amounts_are_announced_hierarchically() {
        let source = r#"card "Divided Trigger Fixture" {
  id: "forge:test:divided-trigger-fixture"
  layout: normal
  status: unverified_playable
  face "Divided Trigger Fixture" {
    cost: "{2}{R}"
    types: "Creature - Human Wizard"
    oracle: "Whenever an opponent draws a card, this deals 4 damage divided as you choose among up to four target creatures."
    power: "2"
    toughness: "3"
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: deal_damage(target_allocation(target_range(permanents(type_is("creature")), 0, 4), 4), 4)
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("divided_trigger_fixture.frs", source)
            .unwrap_or_else(|error| panic!("divided trigger fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("divided trigger fixture should compile: {error}")),
        );
        let requirement = program.triggered_abilities()[0].target_requirements()[0];
        assert_eq!((requirement.minimum(), requirement.maximum()), (0, 4));
        assert_eq!(requirement.allocation_total(), Some(4));

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        let existing = driver
            .state
            .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
            .and_then(|objects| objects.first())
            .copied()
            .unwrap_or_else(|| panic!("fixture should begin with one battlefield creature"));
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtime should register: {error}"));
        let first = create_test_creature(&mut driver.state, opponent, 1_350, 2, 2);
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_352),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw should queue divided trigger: {error}"));
        let trigger = driver.state.pending_triggers()[0].trigger();
        let initial = driver
            .trigger_target_context(
                controller,
                source_object,
                trigger,
                (0, 0),
                &[],
                requirement,
                &[],
            )
            .unwrap_or_else(|error| panic!("range target context should exist: {error}"));
        assert!(!initial.options().iter().any(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. } if targets.is_empty()
        )));
        let first_prefix = initial
            .options()
            .iter()
            .find_map(|option| match option.descriptor() {
                DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. }
                    if targets.len() == 1 =>
                {
                    Some(targets.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("range context should expose a first target"));
        let continuation = driver
            .trigger_target_context(
                controller,
                source_object,
                trigger,
                (0, 0),
                &first_prefix,
                requirement,
                &[],
            )
            .unwrap_or_else(|error| panic!("range continuation should exist: {error}"));
        assert!(continuation.options().iter().any(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChooseTriggerTargetGroups { targets, .. }
                if targets == &first_prefix
        )));

        let mut ai_driver = driver.clone();
        let mut source = MaximizeTriggerTargets;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        let outcome = driver
            .put_pending_triggers_on_stack(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("divided trigger should stack: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
        let stack_entry = driver
            .state
            .stack_top()
            .unwrap_or_else(|| panic!("divided trigger should be on stack"));
        let targets = stack_entry.targets();
        assert_eq!(targets.len(), 3);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.requirement().assigned_allocation())
                .collect::<Vec<_>>(),
            vec![Some(1), Some(1), Some(2)]
        );
        assert_eq!(
            targets
                .iter()
                .map(|target| target.choice())
                .collect::<HashSet<_>>(),
            [
                TargetChoice::Object(source_object),
                TargetChoice::Object(existing),
                TargetChoice::Object(first),
            ]
            .into_iter()
            .collect::<HashSet<_>>()
        );

        let policies = [AiController::Random(RandomLegalPolicy::new(1_351)); PLAYER_COUNT];
        let mut no_decisions = None;
        let ai_outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI divided trigger should stack: {error}"));
        assert!(matches!(ai_outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
        assert!(ai_driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "trigger_target"));
        assert!(ai_driver
            .ai_decisions
            .iter()
            .any(|record| record.kind == "trigger_target_allocation"));
        assert_ai_episode_integrity(&ai_driver.ai_decisions);
    }

    #[test]
    fn same_batch_trigger_targeting_uses_the_staged_stack_for_human_and_ai() {
        let source = r#"card "Trigger Batch Fixture" {
  id: "forge:test:trigger-batch-fixture"
  layout: normal
  status: unverified_playable
  face "Trigger Batch Fixture" {
    cost: "{2}{U}"
    types: "Creature - Human Wizard"
    oracle: "Whenever an opponent draws a card, you gain 1 life. Whenever an opponent draws a card, counter target spell or ability."
    power: "2"
    toughness: "3"
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: gain_life(1, you())
    }
    ability triggered {
      event: event_draw(opponent())
      effect: counter_spell(target(spells()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("trigger_batch_fixture.frs", source)
            .unwrap_or_else(|error| panic!("trigger-batch fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("trigger-batch fixture should compile: {error}")),
        );
        assert_eq!(program.triggered_abilities().len(), 2);
        assert!(program.triggered_abilities()[0]
            .target_requirements()
            .is_empty());
        assert_eq!(
            program.triggered_abilities()[1].target_requirements(),
            &[TargetRequirement::new(TargetKind::StackEntry).with_group(0, 1, 1)]
        );

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtimes should register: {error}"));
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_304),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("opponent draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw event should queue both triggers: {error}"));
        assert_eq!(driver.state.pending_triggers().len(), 2);
        let lower_entry = driver.state.next_stack_entry_id();
        let upper_trigger = driver.state.pending_triggers()[1].trigger();
        let target_context = driver
            .trigger_target_context(
                controller,
                source_object,
                upper_trigger,
                (1, 0),
                &[],
                TargetRequirement::new(TargetKind::StackEntry),
                &[lower_entry],
            )
            .unwrap_or_else(|error| panic!("same-batch target context should exist: {error}"));
        assert_eq!(target_context.options().len(), 1);
        assert!(matches!(
            target_context.options()[0].descriptor(),
            DecisionDescriptor::ChooseTriggerTargetGroups { trigger, targets }
                if *trigger == upper_trigger
                    && targets == &[AnnouncedTarget::new(
                        0,
                        TargetChoice::StackEntry(lower_entry),
                    )]
        ));

        let mut human_driver = driver.clone();
        let mut human_source = PickFirstChoice;
        let mut human_decisions = Some(&mut human_source as &mut dyn DecisionSource);
        let outcome = human_driver
            .put_pending_triggers_on_stack(Some(controller), &mut human_decisions, None)
            .unwrap_or_else(|error| panic!("human trigger batch should stack: {error}"));
        let Outcome::StackEntriesAdded(human_entries) = outcome else {
            panic!("human trigger batch returned {outcome:?}");
        };
        assert_eq!(human_entries.len(), 2);
        assert_eq!(human_entries[0], lower_entry);
        assert_eq!(
            human_driver
                .state
                .stack_top()
                .unwrap_or_else(|| panic!("counter trigger should be on top"))
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::StackEntry(lower_entry)]
        );
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::PutPendingTriggeredAbilitiesOnStackWithChoices { bindings })
                if bindings.len() == 2
        ));
        resolve_top_for_two_players(&mut human_driver);
        assert!(human_driver.state.stack_entries().is_empty());
        assert!(human_driver
            .state
            .resolution_log()
            .iter()
            .any(|record| record.stack_entry() == lower_entry
                && record.outcome() == ResolutionOutcome::CounteredBySpell));

        let weights = AiWeights::bundled()
            .unwrap_or_else(|error| panic!("bundled weights should parse: {error}"));
        let policies =
            [AiController::Heuristic(HeuristicPolicy::rollout(weights, 31)); PLAYER_COUNT];
        let mut ai_driver = driver;
        let mut no_decisions = None;
        let outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI trigger batch should stack: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 2));
        assert_eq!(
            ai_driver
                .state
                .stack_top()
                .unwrap_or_else(|| panic!("AI counter trigger should be on top"))
                .targets()
                .iter()
                .map(|target| target.choice())
                .collect::<Vec<_>>(),
            vec![TargetChoice::StackEntry(lower_entry)]
        );
        assert!(ai_driver
            .ai_decisions
            .iter()
            .any(|decision| decision.kind == "trigger_order"));
        let target_decision = ai_driver
            .ai_decisions
            .iter()
            .find(|decision| decision.kind == "trigger_target")
            .unwrap_or_else(|| panic!("AI should record the same-batch target decision"));
        assert_eq!(target_decision.context_id, target_context.id().to_string());
        assert!(target_decision
            .canonical_legal_actions
            .iter()
            .any(|action| action.action_id == target_decision.action_id));
    }

    #[test]
    fn required_trigger_without_legal_targets_is_removed_without_prompting() {
        let source = r#"card "No Target Trigger Fixture" {
  id: "forge:test:no-target-trigger-fixture"
  layout: normal
  status: unverified_playable
  face "No Target Trigger Fixture" {
    cost: "{2}{U}"
    types: "Creature - Human Wizard"
    oracle: "Whenever an opponent draws a card, this deals 1 damage to target planeswalker."
    power: "2"
    toughness: "2"
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: deal_damage(target(permanents(type_is("planeswalker"))), 1)
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("no_target_trigger_fixture.frs", source)
            .unwrap_or_else(|error| panic!("no-target fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("no-target fixture should compile: {error}")),
        );
        assert_eq!(program.triggered_abilities().len(), 1);
        assert_eq!(
            program.triggered_abilities()[0].target_requirements().len(),
            1
        );

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtime should register: {error}"));
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_303),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("opponent draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw event should queue the trigger: {error}"));
        assert_eq!(driver.state.pending_triggers().len(), 1);

        let mut ai_driver = driver.clone();
        let mut human_driver = driver;
        let mut source = PickFirstChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        let outcome = human_driver
            .put_pending_triggers_on_stack(Some(controller), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human no-target trigger should be removed: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.is_empty()));
        assert!(human_driver.state.pending_triggers().is_empty());
        assert!(human_driver.state.stack_entries().is_empty());
        assert!(matches!(
            human_driver.actions.last(),
            Some(Action::PutPendingTriggeredAbilitiesOnStackWithChoices { bindings })
                if bindings.len() == 1
                    && bindings[0].disposition()
                        == TriggerStackDisposition::RemoveForNoLegalTargets
        ));

        let policies = [AiController::Random(RandomLegalPolicy::new(23)); PLAYER_COUNT];
        let mut no_decisions = None;
        let outcome = ai_driver
            .put_pending_triggers_on_stack(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI no-target trigger should be removed: {error}"));
        assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.is_empty()));
        assert!(ai_driver.state.pending_triggers().is_empty());
        assert!(ai_driver.state.stack_entries().is_empty());
        assert!(ai_driver.ai_decisions.is_empty());
    }

    #[test]
    fn triggered_optionals_use_shared_resolution_contexts_without_silent_acceptance() {
        let source = r#"card "Optional Trigger Fixture" {
  id: "forge:test:optional-trigger-fixture"
  layout: normal
  status: unverified_playable
  face "Optional Trigger Fixture" {
    cost: "{1}{W}"
    types: "Creature - Human Cleric"
    oracle: "Whenever an opponent draws a card, you may gain 3 life."
    power: "2"
    toughness: "2"
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: choose_up_to(1, gain_life(3, you()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("optional_trigger_fixture.frs", source)
            .unwrap_or_else(|error| panic!("optional-trigger fixture should parse: {error}"));
        let program =
            Arc::new(compile_card_program(&definition).unwrap_or_else(|error| {
                panic!("optional-trigger fixture should compile: {error}")
            }));
        assert_eq!(program.triggered_abilities().len(), 1);
        assert_eq!(program.triggered_abilities()[0].optional_choice_count(), 1);

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtime should register: {error}"));
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_304),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("opponent draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw event should queue the trigger: {error}"));

        let mut human_driver = driver.clone();
        let mut ai_driver = driver;
        for candidate in [&mut human_driver, &mut ai_driver] {
            let mut no_decisions = None;
            let outcome = candidate
                .put_pending_triggers_on_stack(None, &mut no_decisions, None)
                .unwrap_or_else(|error| panic!("optional trigger should stack: {error}"));
            assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
            assert_eq!(
                candidate
                    .state
                    .stack_top()
                    .map(|entry| entry.decisions().optional_choice_count()),
                Some(0)
            );
            candidate
                .pass_priority()
                .unwrap_or_else(|error| panic!("controller pass should succeed: {error}"));
            candidate
                .pass_priority()
                .unwrap_or_else(|error| panic!("opponent pass should resolve trigger: {error}"));
            assert!(candidate.pending_triggered_resolution.is_some());
        }

        let expected_context = human_driver
            .pending_triggered_resolution
            .as_ref()
            .and_then(|pending| {
                human_driver
                    .pending_triggered_optional_context(pending, 0, &[], false)
                    .ok()
            })
            .unwrap_or_else(|| panic!("trigger optional context should exist"));
        assert_eq!(expected_context.kind(), DecisionKind::Optional);
        assert_eq!(expected_context.options().len(), 2);
        assert_eq!(
            expected_context
                .options()
                .iter()
                .filter_map(|option| match option.descriptor() {
                    DecisionDescriptor::ChooseOptional { accept, .. } => Some(*accept),
                    _ => None,
                })
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([false, true])
        );

        let mut decline = DeclineOptionalChoice;
        let mut human_decisions = Some(&mut decline as &mut dyn DecisionSource);
        human_driver
            .complete_pending_triggered_resolution(Some(controller), &mut human_decisions, None)
            .unwrap_or_else(|error| panic!("human decline should resolve: {error}"));
        assert_eq!(human_driver.state.players()[controller.index()].life(), 20);
        assert!(human_driver.actions.iter().all(|action| !matches!(
            action,
            Action::GainLife { player, amount } if *player == controller && *amount == 3
        )));

        let policy = AiController::Heuristic(HeuristicPolicy::rollout(
            AiWeights::bundled().unwrap_or_else(|error| panic!("AI weights should load: {error}")),
            31,
        ));
        let policies = [policy; PLAYER_COUNT];
        let mut no_decisions = None;
        ai_driver
            .complete_pending_triggered_resolution(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI optional choice should resolve: {error}"));
        assert_eq!(ai_driver.state.players()[controller.index()].life(), 23);
        let record = ai_driver
            .ai_decisions
            .last()
            .unwrap_or_else(|| panic!("AI trigger optional should emit telemetry"));
        assert_eq!(record.kind, "trigger_optional");
        assert_eq!(record.context_id, expected_context.id().to_string());
        assert!(record
            .canonical_legal_actions
            .iter()
            .any(|action| action.action_id == record.action_id));
    }

    #[test]
    fn unless_paid_triggers_bind_the_event_player_and_canonical_payment_path() {
        let source = r#"card "Smothering Tithe Fixture" {
  id: "forge:test:smothering-tithe-fixture"
  layout: normal
  status: unverified_playable
  face "Smothering Tithe Fixture" {
    cost: "{3}{W}"
    types: "Enchantment"
    oracle: "Whenever an opponent draws a card, that player may pay {2}. If the player doesn't, you create a Treasure token."
    keywords: []
    ability triggered {
      event: event_draw(opponent())
      effect: unless_paid(create_token("c_a_treasure_sac", 1, you()), controller_of(triggered()), mana_cost("{2}"))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("smothering_tithe_fixture.frs", source)
            .unwrap_or_else(|error| panic!("unless-paid fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("unless-paid fixture should compile: {error}")),
        );
        assert!(program.triggered_abilities()[0].unless_paid().is_some());

        let (mut driver, controller, opponent, source_object) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(source_object, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: source_object,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("trigger source characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: source_object,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("trigger source should enter play: {error}"));
        driver
            .register_triggers(controller, source_object, &program, None)
            .unwrap_or_else(|error| panic!("trigger runtime should register: {error}"));
        driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_305),
                owner: opponent,
                controller: opponent,
                zone: ZoneId::new(Some(opponent), ZoneKind::Library),
            })
            .unwrap_or_else(|error| panic!("opponent draw card should be created: {error}"));
        driver
            .dispatch(Action::DrawCards {
                player: opponent,
                count: 1,
            })
            .unwrap_or_else(|error| panic!("draw event should queue the trigger: {error}"));

        let mut pay_driver = driver.clone();
        let mut decline_driver = driver.clone();
        let mut cannot_pay_driver = driver;
        for candidate in [&mut pay_driver, &mut decline_driver] {
            candidate
                .dispatch(Action::AddManaToPool {
                    player: opponent,
                    mana: ManaPool::new(2, 0, 0, 0, 0, 0),
                })
                .unwrap_or_else(|error| panic!("payer mana setup should succeed: {error}"));
        }
        let stage_resolution = |candidate: &mut GameDriver| {
            let mut no_decisions = None;
            let outcome = candidate
                .put_pending_triggers_on_stack(None, &mut no_decisions, None)
                .unwrap_or_else(|error| panic!("unless-paid trigger should stack: {error}"));
            assert!(matches!(outcome, Outcome::StackEntriesAdded(entries) if entries.len() == 1));
            assert_eq!(
                candidate
                    .triggering_players_by_stack_entry
                    .values()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![opponent]
            );
            candidate
                .pass_priority()
                .unwrap_or_else(|error| panic!("controller pass should succeed: {error}"));
            candidate
                .pass_priority()
                .unwrap_or_else(|error| panic!("opponent pass should resolve trigger: {error}"));
            let pending = candidate
                .pending_triggered_resolution
                .as_ref()
                .unwrap_or_else(|| panic!("unless-paid choice should be deferred"));
            assert_eq!(pending.triggering_player, Some(opponent));
            assert!(candidate.triggering_players_by_stack_entry.is_empty());
        };
        stage_resolution(&mut pay_driver);
        stage_resolution(&mut decline_driver);
        stage_resolution(&mut cannot_pay_driver);

        let intent = pay_driver
            .pending_triggered_resolution
            .as_ref()
            .and_then(|pending| {
                pay_driver
                    .pending_triggered_unless_intent_context(pending)
                    .ok()
            })
            .unwrap_or_else(|| panic!("unless-paid intent context should exist"));
        assert_eq!(intent.0.actor(), opponent);
        assert_eq!(intent.0.kind(), DecisionKind::Optional);
        assert_eq!(intent.0.options().len(), 2);

        let (intent_choice, payment_choice) = {
            let pending = pay_driver
                .pending_triggered_resolution
                .as_ref()
                .unwrap_or_else(|| panic!("unless-paid resolution should remain pending"));
            let (intent_context, payer, plans) = pay_driver
                .pending_triggered_unless_intent_context(pending)
                .unwrap_or_else(|error| panic!("unless intent should rebuild: {error}"));
            let intent_choice = intent_context
                .options()
                .iter()
                .position(|option| {
                    matches!(
                        option.descriptor(),
                        DecisionDescriptor::ChooseOptional { accept: true, .. }
                    )
                })
                .unwrap_or_else(|| panic!("pay intent should be canonical"))
                + 1;
            let payment_context = pay_driver
                .pending_triggered_unless_payment_context(pending, payer, &plans)
                .unwrap_or_else(|error| panic!("unless payment should build: {error}"));
            (
                intent_choice,
                usize::from(!payment_context.options().is_empty()),
            )
        };
        let scripted_input = format!("{intent_choice}\n{payment_choice}\n");
        let mut input = Cursor::new(scripted_input.as_bytes());
        let mut output = Vec::new();
        let mut terminal = TerminalDecisionSource::new(&mut input, &mut output);
        {
            let mut pay_decisions = Some(&mut terminal as &mut dyn DecisionSource);
            pay_driver
                .complete_pending_triggered_resolution(Some(opponent), &mut pay_decisions, None)
                .unwrap_or_else(|error| panic!("human payment should resolve: {error}"));
        }
        let pay_records = terminal.into_decisions();
        assert_eq!(pay_records.len(), 2);
        assert!(pay_records.iter().all(|record| {
            record.episode.decision_episode_id == pay_records[0].episode.decision_episode_id
                && record.episode.final_concrete_action_id
                    == pay_records[0].episode.final_concrete_action_id
                && !record.episode.final_concrete_action_id.is_empty()
        }));
        assert_eq!(
            pay_records
                .iter()
                .filter(|record| record.episode.is_terminal_subchoice)
                .count(),
            1
        );
        assert_eq!(pay_records[0].episode.path_depth, 0);
        assert_eq!(pay_records[1].episode.path_depth, 1);
        assert_eq!(
            pay_records[1].episode.parent_context_id.as_deref(),
            Some(pay_records[0].context_id.as_str())
        );
        assert_eq!(pay_driver.state.mana_pool(opponent), Ok(ManaPool::empty()));
        assert!(pay_driver.actions.iter().any(|action| matches!(
            action,
            Action::PayMana { player, cost, .. }
                if *player == opponent && cost.base_generic() == 2
        )));
        assert!(pay_driver
            .actions
            .iter()
            .all(|action| !matches!(action, Action::CreateToken { .. })));

        let mut decline = DeclineOptionalChoice;
        let mut decline_decisions = Some(&mut decline as &mut dyn DecisionSource);
        decline_driver
            .complete_pending_triggered_resolution(Some(opponent), &mut decline_decisions, None)
            .unwrap_or_else(|error| panic!("human decline should resolve: {error}"));
        assert_eq!(
            decline_driver.state.mana_pool(opponent),
            Ok(ManaPool::new(2, 0, 0, 0, 0, 0))
        );
        assert!(decline_driver
            .actions
            .iter()
            .any(|action| matches!(action, Action::CreateToken { controller: player, .. } if *player == controller)));

        let policies = [AiController::Random(RandomLegalPolicy::new(37)); PLAYER_COUNT];
        let mut no_decisions = None;
        cannot_pay_driver
            .complete_pending_triggered_resolution(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("forced decline should resolve: {error}"));
        let forced = cannot_pay_driver
            .ai_decisions
            .last()
            .unwrap_or_else(|| panic!("forced decline should emit AI telemetry"));
        assert_eq!(forced.kind, "trigger_unless_payment_intent");
        assert_eq!(forced.policy, "forced-v1");
        assert_eq!(forced.stop_reason, "singleton_legal_action");
        assert_eq!(forced.legal_actions, 1);
        assert!(cannot_pay_driver
            .actions
            .iter()
            .any(|action| matches!(action, Action::CreateToken { controller: player, .. } if *player == controller)));
        assert_ai_episode_integrity(&cannot_pay_driver.ai_decisions);
    }

    #[test]
    fn spell_search_is_deferred_to_a_shared_resolution_context() {
        let source = r#"card "Spell Search Fixture" {
  id: "c992b25e-224d-4856-a785-56b8e4017590"
  layout: normal
  status: unverified_playable
  face "Spell Search Fixture" {
    cost: "{R}{W}"
    types: "Sorcery"
    oracle: "Search your library for a basic land card, put it into your hand, then shuffle."
    keywords: []
    ability spell {
      effect: sequence(search_library(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(and(type_is("land"), supertype_is("basic")), zone_is("library")))), "hand", 1), shuffle(you()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("spell_search_fixture.frs", source)
            .unwrap_or_else(|error| panic!("spell-search fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("spell-search fixture should compile: {error}")),
        );
        assert_eq!(program.object_choice_requirements().len(), 1);

        let (mut driver, caster, _opponent, spell) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(spell, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: spell,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("spell characteristics should apply: {error}"));

        let library = ZoneId::new(Some(caster), ZoneKind::Library);
        let nonbasic = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(1_300),
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
                card: CardId::new(1_301),
                owner: caster,
                controller: caster,
                zone: library,
            })
            .unwrap_or_else(|error| panic!("basic setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected basic setup outcome: {other:?}"),
        };
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
                    .with_supertypes(supertypes),
                })
                .unwrap_or_else(|error| panic!("land characteristics should apply: {error}"));
        }

        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("spell-search main context should exist: {error}"));
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
                        && targets.is_empty()
                        && modes.is_empty()
                        && optional.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("spell search should have a canonical cast option"));
        let choice = mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("spell search should have a typed cast adapter"));
        assert_eq!(driver.apply_main_choice(caster, choice), Ok(true));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("caster pass should succeed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("opponent pass should resolve spell: {error}"));
        assert!(driver.pending_spell_resolution.is_some());
        assert_eq!(
            driver.state.object_zone(spell),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );

        let pending = driver
            .pending_spell_resolution
            .as_ref()
            .unwrap_or_else(|| panic!("spell should await its search choice"));
        let search = driver
            .pending_spell_context(pending)
            .unwrap_or_else(|error| panic!("spell search context should exist: {error}"));
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

        let mut ai_driver = driver.clone();
        let policies = [AiController::Random(RandomLegalPolicy::new(31)); PLAYER_COUNT];
        let mut no_decisions = None;
        ai_driver
            .complete_pending_spell_resolution(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI spell search should complete: {error}"));
        assert!(ai_driver.pending_spell_resolution.is_none());
        assert_eq!(
            ai_driver
                .ai_decisions
                .last()
                .map(|record| record.kind.as_str()),
            Some("spell_resolution_object_choice")
        );

        let mut source = PickNonEmptyResolutionChoice;
        let mut decisions = Some(&mut source as &mut dyn DecisionSource);
        driver
            .complete_pending_spell_resolution(Some(caster), &mut decisions, None)
            .unwrap_or_else(|error| panic!("human spell search should complete: {error}"));
        assert!(driver.pending_spell_resolution.is_none());
        assert_eq!(
            driver.state.object_zone(basic),
            Some(ZoneId::new(Some(caster), ZoneKind::Hand))
        );
        assert_eq!(driver.state.object_zone(nonbasic), Some(library));
        assert!(driver.actions.iter().any(|action| matches!(
            action,
            Action::ShuffleLibrary { player } if *player == caster
        )));
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
    fn polluted_delta_uses_canonical_life_cost_activation_hierarchy() {
        let source = r#"card "Polluted Delta" {
  id: "ef86989d-ce80-4e55-aece-7d11710eeffa"
  layout: normal
  status: unverified_playable
  face "Polluted Delta" {
    cost: ""
    types: "Land"
    oracle: "{T}, Pay 1 life, Sacrifice Polluted Delta: Search your library for an Island or Swamp card, put it onto the battlefield, then shuffle."
    keywords: []
    ability activated {
      costs: [tap_self(), pay_life(1), sacrifice_self()]
      effect: sequence(search_library(cards(and(or(subtype_is("island"), subtype_is("swamp")), zone_is("library"))), you(), 1), move_zone(chosen(cards(and(or(subtype_is("island"), subtype_is("swamp")), zone_is("library")))), "battlefield", 1), shuffle(you()))
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("polluted_delta.frs", source)
            .unwrap_or_else(|error| panic!("Polluted Delta should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("Polluted Delta should compile: {error}")),
        );
        let (mut driver, caster, _opponent, delta) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(delta, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: delta,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("Delta characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: delta,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("Delta should enter the battlefield: {error}"));
        driver
            .register_permanent_runtime(caster, delta)
            .unwrap_or_else(|error| panic!("Delta runtime should register: {error}"));

        let (context, mappings) = driver
            .priority_decision_context(caster)
            .unwrap_or_else(|error| panic!("Delta priority context should exist: {error}"));
        let root = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginActivateProgramAbilityWithCosts {
                        source,
                        targets,
                        optional,
                        ..
                    } if *source == delta && targets.is_empty() && optional.is_empty()
                )
            })
            .unwrap_or_else(|| panic!("Delta extra-cost activation root should be canonical"));
        let begin = mappings
            .iter()
            .find_map(|(id, choice)| (*id == root.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("Delta root should have a typed adapter"));
        let (payment_context, payment_mappings) = driver
            .hierarchical_main_context(caster, &begin)
            .unwrap_or_else(|error| panic!("Delta payment context should build: {error}"))
            .unwrap_or_else(|| panic!("Delta should defer its exact payment"));
        assert!(payment_context.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChoosePayment { .. }
        )));
        let finish = payment_mappings
            .first()
            .map(|(_, choice)| choice.clone())
            .unwrap_or_else(|| panic!("Delta should have a zero-mana payment"));

        assert_eq!(driver.state.players()[caster.index()].life(), 20);
        assert_eq!(driver.apply_main_choice(caster, finish), Ok(true));
        assert_eq!(driver.state.players()[caster.index()].life(), 19);
        assert_eq!(
            driver.state.object_zone(delta),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        assert!(driver.state.stack_top().is_some());
    }

    #[test]
    fn zuran_orb_uses_canonical_selected_sacrifice_hierarchy() {
        let source = r#"card "Zuran Orb" {
  id: "08cb8a30-9cb4-4517-bee5-8848aa60d1a2"
  layout: normal
  status: unverified_playable
  face "Zuran Orb" {
    cost: "{0}"
    types: "Artifact"
    oracle: "Sacrifice a land: You gain 2 life."
    keywords: []
    ability activated {
      costs: [sacrifice(permanents(and(type_is("land"), controlled_by(you()))), 1)]
      effect: gain_life(2, you())
    }
  }
}"#;
        let definition = forge_cardc::parse_card_named("zuran_orb.frs", source)
            .unwrap_or_else(|error| panic!("Zuran Orb should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("Zuran Orb should compile: {error}")),
        );
        let (mut driver, caster, _opponent, orb) = modal_spell_driver();
        Arc::make_mut(&mut driver.programs).insert(orb, Arc::clone(&program));
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: orb,
                base: program.base_object(),
            })
            .unwrap_or_else(|error| panic!("Orb characteristics should apply: {error}"));
        driver
            .dispatch(Action::MoveObject {
                object: orb,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("Orb should enter the battlefield: {error}"));
        let land = match driver
            .dispatch(Action::CreateObject {
                card: CardId::new(907),
                owner: caster,
                controller: caster,
                zone: ZoneId::new(None, ZoneKind::Battlefield),
            })
            .unwrap_or_else(|error| panic!("land setup should succeed: {error}"))
        {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected land setup outcome: {other:?}"),
        };
        driver
            .dispatch(Action::SetBaseObjectCharacteristics {
                object: land,
                base: BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_land(),
                    ObjectColors::none(),
                ),
            })
            .unwrap_or_else(|error| panic!("land characteristics should apply: {error}"));
        driver
            .register_permanent_runtime(caster, orb)
            .unwrap_or_else(|error| panic!("Orb runtime should register: {error}"));

        let (context, mappings) = driver
            .priority_decision_context(caster)
            .unwrap_or_else(|error| panic!("Orb priority context should exist: {error}"));
        let root = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginActivateProgramAbilityWithCosts { source, .. }
                        if *source == orb
                )
            })
            .unwrap_or_else(|| panic!("Orb extra-cost activation root should be canonical"));
        let begin = mappings
            .iter()
            .find_map(|(id, choice)| (*id == root.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("Orb root should have a typed adapter"));
        let (sacrifice_context, sacrifice_mappings) = driver
            .hierarchical_main_context(caster, &begin)
            .unwrap_or_else(|error| panic!("Orb sacrifice context should build: {error}"))
            .unwrap_or_else(|| panic!("Orb should defer its sacrifice"));
        assert_eq!(sacrifice_context.options().len(), 1);
        assert!(matches!(
            sacrifice_context.options()[0].descriptor(),
            DecisionDescriptor::ChooseActivationCostObjects { objects }
                if objects == &vec![land]
        ));
        let sacrifice_stage = sacrifice_mappings[0].1.clone();
        let (payment_context, payment_mappings) = driver
            .hierarchical_main_context(caster, &sacrifice_stage)
            .unwrap_or_else(|error| panic!("Orb payment context should build: {error}"))
            .unwrap_or_else(|| panic!("Orb should defer its exact payment"));
        assert!(matches!(
            payment_context.options()[0].descriptor(),
            DecisionDescriptor::ChoosePayment { .. }
        ));
        let finish = payment_mappings[0].1.clone();

        assert_eq!(driver.apply_main_choice(caster, finish), Ok(true));
        assert_eq!(
            driver.state.object_zone(orb),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert_eq!(
            driver.state.object_zone(land),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        assert!(driver.state.stack_top().is_some());
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
            target_requirements: Vec::new(),
            targets: Vec::new(),
            target_legalities: Vec::new(),
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

        driver.pending_activated_resolution = Some(pending);
        let policies = [AiController::Random(RandomLegalPolicy::new(1_299)); PLAYER_COUNT];
        let mut no_decisions = None;
        driver
            .complete_pending_activated_resolution(None, &mut no_decisions, Some(&policies))
            .unwrap_or_else(|error| panic!("AI resolution choices should complete: {error}"));
        assert_eq!(
            driver
                .ai_decisions
                .iter()
                .filter(|record| record.kind == "resolution_object_choice")
                .count(),
            2
        );
        assert_ai_episode_integrity(&driver.ai_decisions);
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
            .register_triggers(controller, sword, &program, None)
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
            .execute_trigger(
                controller,
                trigger,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                StackDecisionBindings::default(),
            )
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

    struct PickFirstChoice;

    impl DecisionSource for PickFirstChoice {
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            (!prompt.options.is_empty())
                .then_some(DecisionSelection::Option(0))
                .ok_or_else(|| "expected at least one canonical option".to_owned())
        }
    }

    struct MaximizeTriggerTargets;

    impl DecisionSource for MaximizeTriggerTargets {
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            prompt
                .options
                .iter()
                .position(|label| label != "Finish this target group")
                .map(DecisionSelection::Option)
                .ok_or_else(|| "expected a non-finish trigger target option".to_owned())
        }
    }

    struct DeclineOptionalChoice;

    impl DecisionSource for DeclineOptionalChoice {
        fn choose(&mut self, prompt: &DecisionPrompt<'_>) -> Result<DecisionSelection, String> {
            prompt
                .context
                .options()
                .iter()
                .position(|option| {
                    matches!(
                        option.descriptor(),
                        DecisionDescriptor::ChooseOptional { accept: false, .. }
                    )
                })
                .map(DecisionSelection::Option)
                .ok_or_else(|| "expected a declined optional choice".to_owned())
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
            if prompt.context.kind() == DecisionKind::Optional {
                return prompt
                    .context
                    .options()
                    .iter()
                    .position(|option| {
                        matches!(
                            option.descriptor(),
                            DecisionDescriptor::ChooseOptional { accept: true, .. }
                        )
                    })
                    .map(DecisionSelection::Option)
                    .ok_or_else(|| "expected an accepted optional choice".to_owned());
            }
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
            conditional_cast_triggers: HashMap::new(),
            triggering_players_by_stack_entry: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_spell_resolution: None,
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: 17,
        };
        (driver, owner, commander, graveyard)
    }

    fn alternate_spell_driver(
        file: &str,
        source: &str,
    ) -> (GameDriver, PlayerId, PlayerId, ObjectId, [ObjectId; 2]) {
        let definition = forge_cardc::parse_card_named(file, source)
            .unwrap_or_else(|error| panic!("alternate fixture should parse: {error}"));
        let program = Arc::new(
            compile_card_program(&definition)
                .unwrap_or_else(|error| panic!("alternate fixture should compile: {error}")),
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
                card: CardId::new(9_100),
                owner: caster,
                controller: caster,
                zone: ZoneId::new(Some(caster), ZoneKind::Hand),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected spell setup outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: spell,
                    base: program.base_object(),
                },
            ),
            Outcome::Applied
        );
        if let Some(base) = program.base_creature() {
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseCreatureCharacteristics {
                        object: spell,
                        base,
                    },
                ),
                Outcome::Applied
            );
        }
        let commander = create_test_creature(&mut state, caster, 9_101, 2, 2);
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
        let opponent_permanents = [
            create_test_creature(&mut state, opponent, 9_102, 2, 2),
            create_test_creature(&mut state, opponent, 9_103, 3, 3),
        ];
        for card in 9_200..9_212 {
            assert!(matches!(
                apply(
                    &mut state,
                    Action::CreateObject {
                        card: CardId::new(card),
                        owner: caster,
                        controller: caster,
                        zone: ZoneId::new(Some(caster), ZoneKind::Library),
                    },
                ),
                Outcome::ObjectCreated(_)
            ));
        }
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: caster,
                },
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
                    mana: ManaPool::new(20, 20, 20, 20, 20, 20),
                },
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
            commanders: vec![commander, opponent_permanents[0]],
            trigger_programs: HashMap::new(),
            conditional_cast_triggers: HashMap::new(),
            triggering_players_by_stack_entry: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_spell_resolution: None,
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: 29,
        };
        (driver, caster, opponent, spell, opponent_permanents)
    }

    fn selected_alternate_cast(
        driver: &GameDriver,
        caster: PlayerId,
        spell: ObjectId,
        alternate: SpellAlternateCost,
    ) -> MainChoice {
        let (context, mappings) = driver
            .main_decision_context(caster)
            .unwrap_or_else(|error| panic!("alternate main context should exist: {error}"));
        let root = context
            .options()
            .iter()
            .find(|option| {
                matches!(
                    option.descriptor(),
                    DecisionDescriptor::BeginCastSpellAlternate {
                        object,
                        alternate: selected,
                        ..
                    } if *object == spell && *selected == alternate
                )
            })
            .unwrap_or_else(|| panic!("alternate root should be canonical"));
        let begin = mappings
            .iter()
            .find_map(|(id, choice)| (*id == root.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("alternate root should have a typed adapter"));
        let (payment_context, payment_mappings) = driver
            .hierarchical_main_context(caster, &begin)
            .unwrap_or_else(|error| panic!("alternate payment context should build: {error}"))
            .unwrap_or_else(|| panic!("alternate cast should defer payment"));
        assert!(payment_context.options().iter().all(|option| matches!(
            option.descriptor(),
            DecisionDescriptor::ChoosePayment { .. }
        )));
        let selected = payment_context
            .options()
            .first()
            .unwrap_or_else(|| panic!("alternate cast should have a payment"));
        payment_mappings
            .iter()
            .find_map(|(id, choice)| (*id == selected.id()).then(|| choice.clone()))
            .unwrap_or_else(|| panic!("alternate payment should have a typed adapter"))
    }

    fn resolve_top_for_two_players(driver: &mut GameDriver) {
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("first pass failed: {error}"));
        driver
            .pass_priority()
            .unwrap_or_else(|error| panic!("second pass failed: {error}"));
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
            conditional_cast_triggers: HashMap::new(),
            triggering_players_by_stack_entry: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_spell_resolution: None,
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
            trace: TraceMode::Off,
            actions: Vec::new(),
            ai_decisions: Vec::new(),
            next_hidden_check_action: u64::MAX,
            next_invariant_check_action: u64::MAX,
            seed: 17,
        };
        (driver, caster, opponent, spell)
    }

    #[test]
    fn grouped_target_enumeration_is_distinct_bounded_and_allocation_sensitive() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player setup outcome: {other:?}"),
        };
        let choices = (10..13)
            .map(|card| {
                match apply(
                    &mut state,
                    Action::CreateObject {
                        card: CardId::new(card),
                        owner: player,
                        controller: player,
                        zone: ZoneId::new(Some(player), ZoneKind::Hand),
                    },
                ) {
                    Outcome::ObjectCreated(object) => TargetChoice::Object(object),
                    other => panic!("unexpected object setup outcome: {other:?}"),
                }
            })
            .collect::<Vec<_>>();
        let combinations = bounded_target_combinations(&choices, 0, 2, 16)
            .unwrap_or_else(|error| panic!("target combinations should enumerate: {error}"));
        assert_eq!(combinations.len(), 7);
        assert!(combinations.iter().all(|combination| {
            combination.iter().copied().collect::<HashSet<_>>().len() == combination.len()
        }));
        assert_eq!(
            bounded_positive_allocations(4, 2, 16)
                .unwrap_or_else(|error| panic!("allocations should enumerate: {error}")),
            vec![vec![1, 3], vec![2, 2], vec![3, 1]]
        );

        let first = DecisionOption::new(
            DecisionDescriptor::BeginCastSpellTargetGroups {
                object: match choices[0] {
                    TargetChoice::Object(object) => object,
                    _ => unreachable!("fixture choices are objects"),
                },
                targets: vec![
                    AnnouncedTarget::new(0, choices[0]).with_allocation(1),
                    AnnouncedTarget::new(0, choices[1]).with_allocation(3),
                ],
                modes: Vec::new(),
                optional: Vec::new(),
            },
            Vec::new(),
        );
        let second = DecisionOption::new(
            DecisionDescriptor::BeginCastSpellTargetGroups {
                object: match choices[0] {
                    TargetChoice::Object(object) => object,
                    _ => unreachable!("fixture choices are objects"),
                },
                targets: vec![
                    AnnouncedTarget::new(0, choices[0]).with_allocation(2),
                    AnnouncedTarget::new(0, choices[1]).with_allocation(2),
                ],
                modes: Vec::new(),
                optional: Vec::new(),
            },
            Vec::new(),
        );
        assert_ne!(first.id(), second.id());
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
            conditional_cast_triggers: HashMap::new(),
            triggering_players_by_stack_entry: HashMap::new(),
            activated_abilities: Vec::new(),
            pending_spell_resolution: None,
            pending_activated_resolution: None,
            pending_triggered_resolution: None,
            triggers_registered_for: HashSet::new(),
            permanent_runtime_registered_for: HashSet::new(),
            commander_zone_decisions: HashMap::new(),
            current_attacks: Vec::new(),
            coverage_target: None,
            metrics: GameMetrics::default(),
            progress: GameProgressTracker::default(),
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
