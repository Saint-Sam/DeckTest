use forge_ai::{AiPolicyFamily, AiTierSet, DifficultyTier, GuardrailProfile};
use forge_game_runner::{AiArena, AiArenaSummary, AiPolicyConfig};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Instant,
};

const DEFAULT_MANIFEST: &str = "assets/t3_9/integration_decks.json";
const DEFAULT_OUTPUT: &str = "metrics/ai/arena_results.json";
const DEFAULT_CALIBRATION_OUTPUT: &str = "metrics/ai/calibration_results.json";
const DEFAULT_KNEE_OUTPUT: &str = "metrics/ai/search_budget_knee_results.json";
const DEFAULT_GAMES_PER_RUNG: u32 = 400;
const DEFAULT_MAX_TURNS: u32 = 160;
const MAX_JOBS: u32 = 24;
const SEED_BASE: u64 = 0x4d34_0000_0000_0001;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tier {
    Random,
    Novice,
    Standard,
    Expert,
    Master,
}

#[derive(Clone, Copy)]
enum PolicySpec {
    Tier(Tier),
    TimedTrial {
        label: &'static str,
        think_ms: u32,
        determinizations: u32,
        workers: u32,
        profile: GuardrailProfile,
        adaptive: bool,
    },
}

impl PolicySpec {
    fn name(self) -> String {
        match self {
            Self::Tier(tier) => tier.name().to_owned(),
            Self::TimedTrial {
                label,
                think_ms,
                adaptive,
                ..
            } => format!(
                "{label}-{think_ms}ms-{}",
                if adaptive { "adaptive" } else { "fixed" }
            ),
        }
    }

    fn policy(self, seed: u64, tiers: &AiTierSet) -> AiPolicyConfig {
        match self {
            Self::Tier(tier) => tier.policy(seed, tiers),
            Self::TimedTrial {
                think_ms,
                determinizations,
                workers,
                profile,
                adaptive,
                ..
            } => AiPolicyConfig::TimedSearch {
                seed,
                think_ms,
                determinizations,
                workers,
                adaptive,
                guardrail_profile: profile,
            },
        }
    }

    fn search_workers(self, tiers: &AiTierSet) -> u32 {
        match self {
            Self::Tier(tier) => tier.search_workers(tiers),
            Self::TimedTrial { workers, .. } => workers,
        }
    }
}

impl Tier {
    const fn name(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Novice => "novice",
            Self::Standard => "standard",
            Self::Expert => "expert",
            Self::Master => "master",
        }
    }

    const fn difficulty(self) -> DifficultyTier {
        match self {
            Self::Random => DifficultyTier::Random,
            Self::Novice => DifficultyTier::Novice,
            Self::Standard => DifficultyTier::Standard,
            Self::Expert => DifficultyTier::Expert,
            Self::Master => DifficultyTier::Master,
        }
    }

    fn policy(self, seed: u64, tiers: &AiTierSet) -> AiPolicyConfig {
        let definition = tiers.tier(self.difficulty());
        match definition.policy() {
            AiPolicyFamily::RandomLegal => AiPolicyConfig::RandomLegal { seed },
            AiPolicyFamily::Heuristic => AiPolicyConfig::Heuristic {
                seed,
                noise_span: definition.noise_span(),
            },
            AiPolicyFamily::TimedSearch => AiPolicyConfig::TimedSearch {
                seed,
                think_ms: definition.think_ms(),
                determinizations: definition.determinizations(),
                workers: definition.workers(),
                adaptive: false,
                guardrail_profile: self.guardrail_profile(),
            },
        }
    }

    const fn guardrail_profile(self) -> GuardrailProfile {
        match self {
            Self::Random | Self::Novice => GuardrailProfile::Novice,
            Self::Standard => GuardrailProfile::Standard,
            Self::Expert => GuardrailProfile::Expert,
            Self::Master => GuardrailProfile::Master,
        }
    }

    fn search_workers(self, tiers: &AiTierSet) -> u32 {
        tiers.tier(self.difficulty()).workers()
    }
}

#[derive(Clone, Copy)]
struct Rung {
    lower: PolicySpec,
    upper: PolicySpec,
}

impl Rung {
    fn name(self) -> String {
        format!("{}-vs-{}", self.upper.name(), self.lower.name())
    }
}

const RUNGS: [Rung; 4] = [
    Rung {
        lower: PolicySpec::Tier(Tier::Random),
        upper: PolicySpec::Tier(Tier::Novice),
    },
    Rung {
        lower: PolicySpec::Tier(Tier::Novice),
        upper: PolicySpec::Tier(Tier::Standard),
    },
    Rung {
        lower: PolicySpec::Tier(Tier::Standard),
        upper: PolicySpec::Tier(Tier::Expert),
    },
    Rung {
        lower: PolicySpec::Tier(Tier::Expert),
        upper: PolicySpec::Tier(Tier::Master),
    },
];

#[derive(Clone, Debug)]
struct LadderConfig {
    games_per_rung: u32,
    jobs: u32,
    max_turns: u32,
    manifest: PathBuf,
    output: PathBuf,
    rung: Option<String>,
}

impl LadderConfig {
    fn parse(args: &[String]) -> Result<Self, String> {
        let games_per_rung = value_u32(args, "--games")?.unwrap_or(DEFAULT_GAMES_PER_RUNG);
        if games_per_rung < 2 || games_per_rung % 2 != 0 {
            return Err("--games must be an even integer of at least 2 per rung".to_owned());
        }
        let available = thread::available_parallelism()
            .map_or(1, |value| value.get())
            .min(MAX_JOBS as usize) as u32;
        let jobs = value_u32(args, "--jobs")?
            .unwrap_or(available)
            .min(MAX_JOBS);
        if jobs == 0 {
            return Err("--jobs must be positive".to_owned());
        }
        let max_turns = value_u32(args, "--max-turns")?.unwrap_or(DEFAULT_MAX_TURNS);
        if max_turns < 20 {
            return Err("--max-turns must be at least 20".to_owned());
        }
        Ok(Self {
            games_per_rung,
            jobs,
            max_turns,
            manifest: value_string(args, "--manifest")?
                .map_or_else(|| PathBuf::from(DEFAULT_MANIFEST), PathBuf::from),
            output: value_string(args, "--output")?
                .map_or_else(|| PathBuf::from(DEFAULT_OUTPUT), PathBuf::from),
            rung: value_string(args, "--rung")?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct GameEvidence {
    pair_index: u32,
    rotation: u8,
    seed: u64,
    winner_seat: usize,
    candidate_won: bool,
    turns: u32,
    final_hash: u64,
    typed_actions: usize,
    decisions: usize,
    searched_decisions: usize,
    singleton_bypasses: usize,
    simulations: u64,
    nodes: u64,
    transposition_hits: u64,
    maximum_depth: u32,
    search_wall_latency_us: u64,
    search_wall_latency_p95_us: u64,
    wall_runtime_us: u64,
    #[serde(skip)]
    search_wall_latencies_us: Vec<u64>,
    #[serde(skip)]
    search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
    #[serde(skip)]
    adaptive_search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
}

#[derive(Clone, Debug)]
struct PairEvidence {
    pair_index: u32,
    games: [GameEvidence; 2],
}

#[derive(Clone, Debug, Serialize)]
struct RungReport {
    rung: String,
    lower: String,
    upper: String,
    games: u32,
    pairs: u32,
    parallel_game_workers: u32,
    maximum_search_workers_per_game: u32,
    candidate_wins: u32,
    candidate_win_rate: f64,
    wilson_95: [f64; 2],
    controller_team_elo_estimate: f64,
    target_band: [f64; 2],
    target_band_met: bool,
    candidate_sweeps: u32,
    split_pairs: u32,
    candidate_losses_both: u32,
    totals: CampaignTotals,
    evidence: Vec<GameEvidence>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct CampaignTotals {
    typed_actions: u64,
    decisions: u64,
    searched_decisions: u64,
    singleton_bypasses: u64,
    simulations: u64,
    nodes: u64,
    transposition_hits: u64,
    search_wall_latency_us: u64,
    search_wall_latency_mean_us: u64,
    search_wall_latency_p50_us: u64,
    search_wall_latency_p95_us: u64,
    search_wall_latency_p99_us: u64,
    search_wall_latency_p95_by_budget_ms: BTreeMap<u32, u64>,
    searched_decisions_by_budget_ms: BTreeMap<u32, u64>,
    adaptive_search_wall_latency_p95_by_budget_ms: BTreeMap<u32, u64>,
    adaptive_searched_decisions_by_budget_ms: BTreeMap<u32, u64>,
    wall_runtime_us: u64,
    #[serde(skip)]
    search_wall_latencies_us: Vec<u64>,
    #[serde(skip)]
    search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
    #[serde(skip)]
    adaptive_search_wall_latencies_by_budget_ms: BTreeMap<u32, Vec<u64>>,
}

impl CampaignTotals {
    fn add(&mut self, game: &GameEvidence) {
        self.typed_actions = self.typed_actions.saturating_add(game.typed_actions as u64);
        self.decisions = self.decisions.saturating_add(game.decisions as u64);
        self.searched_decisions = self
            .searched_decisions
            .saturating_add(game.searched_decisions as u64);
        self.singleton_bypasses = self
            .singleton_bypasses
            .saturating_add(game.singleton_bypasses as u64);
        self.simulations = self.simulations.saturating_add(game.simulations);
        self.nodes = self.nodes.saturating_add(game.nodes);
        self.transposition_hits = self
            .transposition_hits
            .saturating_add(game.transposition_hits);
        self.search_wall_latency_us = self
            .search_wall_latency_us
            .saturating_add(game.search_wall_latency_us);
        self.search_wall_latencies_us
            .extend_from_slice(&game.search_wall_latencies_us);
        for (budget, samples) in &game.search_wall_latencies_by_budget_ms {
            self.search_wall_latencies_by_budget_ms
                .entry(*budget)
                .or_default()
                .extend_from_slice(samples);
        }
        for (budget, samples) in &game.adaptive_search_wall_latencies_by_budget_ms {
            self.adaptive_search_wall_latencies_by_budget_ms
                .entry(*budget)
                .or_default()
                .extend_from_slice(samples);
        }
        self.wall_runtime_us = self.wall_runtime_us.saturating_add(game.wall_runtime_us);
    }

    fn finalize(&mut self) {
        self.search_wall_latency_mean_us = if self.search_wall_latencies_us.is_empty() {
            0
        } else {
            self.search_wall_latency_us
                / u64::try_from(self.search_wall_latencies_us.len()).unwrap_or(u64::MAX)
        };
        self.search_wall_latencies_us.sort_unstable();
        self.search_wall_latency_p50_us = percentile(&self.search_wall_latencies_us, 50);
        self.search_wall_latency_p95_us = percentile(&self.search_wall_latencies_us, 95);
        self.search_wall_latency_p99_us = percentile(&self.search_wall_latencies_us, 99);
        for (budget, samples) in &self.search_wall_latencies_by_budget_ms {
            self.search_wall_latency_p95_by_budget_ms
                .insert(*budget, percentile(samples, 95));
            self.searched_decisions_by_budget_ms
                .insert(*budget, u64::try_from(samples.len()).unwrap_or(u64::MAX));
        }
        for (budget, samples) in &self.adaptive_search_wall_latencies_by_budget_ms {
            self.adaptive_search_wall_latency_p95_by_budget_ms
                .insert(*budget, percentile(samples, 95));
            self.adaptive_searched_decisions_by_budget_ms
                .insert(*budget, u64::try_from(samples.len()).unwrap_or(u64::MAX));
        }
    }
}

#[derive(Debug, Serialize)]
struct LadderReport {
    schema_version: u32,
    status: &'static str,
    protocol: &'static str,
    manifest: String,
    games_per_rung: u32,
    jobs: u32,
    max_turns: u32,
    seed_base: u64,
    tier_registry_status: String,
    fixed_hardware_and_thread_configuration: bool,
    rung_reports: Vec<RungReport>,
    all_measured_rungs_in_target_band: bool,
    promotion_eligible: bool,
    promotion_blockers: [&'static str; 4],
}

#[derive(Debug, Serialize)]
struct CalibrationReport {
    schema_version: u32,
    status: &'static str,
    target_tier: String,
    baseline_tier: String,
    target_win_rate_band: [f64; 2],
    tested_budgets_ms: Vec<u32>,
    selected_candidate_budget_ms: Option<u32>,
    games_per_budget: u32,
    selection_rule: &'static str,
    tier_registry_updated: bool,
    promotion_eligible: bool,
    promotion_blockers: [&'static str; 3],
    trials: Vec<RungReport>,
}

#[derive(Debug, Serialize)]
struct SearchKneeReport {
    schema_version: u32,
    status: &'static str,
    threshold_status: &'static str,
    target_tier: String,
    budgets_ms: Vec<u32>,
    games_per_comparison: u32,
    jobs: u32,
    max_turns: u32,
    manifest: String,
    matched_protocol: &'static str,
    provisional_thresholds: SearchKneeThresholds,
    comparisons: Vec<BudgetComparisonReport>,
    adaptive_ablations: Vec<AdaptiveAblationReport>,
    two_consecutive_complete_plateaus: bool,
    provisional_knee_budget_ms: Option<u32>,
    selected_standard_budget_ms: Option<u32>,
    promotion_eligible: bool,
    promotion_blockers: [&'static str; 5],
}

#[derive(Debug, Serialize)]
struct SearchKneeThresholds {
    paired_win_rate_gain_lt_percentage_points: f64,
    estimated_elo_gain_lt_range: [f64; 2],
    ordinary_state_acceptable_action_agreement_at_least: f64,
    consecutive_budget_increases: u32,
    material_latency_and_cost_threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
struct BudgetComparisonReport {
    lower_budget_ms: u32,
    upper_budget_ms: u32,
    paired_win_rate_improvement_percentage_points: f64,
    estimated_elo_improvement: f64,
    confidence_interval_improvement_percentage_points: [f64; 2],
    gain_below_one_percentage_point_or_fifteen_elo: bool,
    confidence_interval_includes_no_product_useful_improvement: bool,
    catastrophic_blunder_rate_meaningfully_decreased: Option<bool>,
    missed_win_or_required_defense_rate_meaningfully_decreased: Option<bool>,
    ordinary_state_acceptable_action_agreement: Option<f64>,
    lower_p95_wall_latency_us: Option<u64>,
    upper_p95_wall_latency_us: Option<u64>,
    p95_wall_latency_increased: Option<bool>,
    measured_cpu_cost_increased_materially: Option<bool>,
    all_acceptance_criteria_complete: bool,
    plateau_acceptance_passed: bool,
    arena: RungReport,
}

#[derive(Debug, Serialize)]
struct AdaptiveAblationReport {
    budget_ms: u32,
    adaptive_win_rate_delta_percentage_points: f64,
    adaptive_elo_delta: f64,
    confidence_interval_delta_percentage_points: [f64; 2],
    fixed_p95_wall_latency_us: Option<u64>,
    adaptive_p95_wall_latency_us: Option<u64>,
    p95_wall_latency_reduction_fraction: Option<f64>,
    measured_cpu_cost_reduction_fraction: Option<f64>,
    no_practical_strength_decline_proven: bool,
    tracks_a_b_c_complete: bool,
    shipping_eligible: bool,
    arena: RungReport,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let config = LadderConfig::parse(args)?;
    let arena = Arc::new(AiArena::load(&config.manifest)?);
    let tiers = Arc::new(
        AiTierSet::bundled().map_err(|error| format!("failed to load AI tiers: {error}"))?,
    );
    let selected = RUNGS
        .into_iter()
        .filter(|rung| {
            config
                .rung
                .as_ref()
                .map_or(true, |name| name == &rung.name())
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(format!(
            "unknown --rung {}; expected one of {}",
            config.rung.as_deref().unwrap_or_default(),
            RUNGS.map(Rung::name).join(", ")
        ));
    }

    let mut rung_reports = Vec::with_capacity(selected.len());
    for rung in selected {
        let report = run_rung(Arc::clone(&arena), Arc::clone(&tiers), &config, rung)?;
        println!(
            "{}: {:.2}% [{:.2}%, {:.2}%], Elo {:.1}, {} games",
            report.rung,
            report.candidate_win_rate * 100.0,
            report.wilson_95[0] * 100.0,
            report.wilson_95[1] * 100.0,
            report.controller_team_elo_estimate,
            report.games
        );
        rung_reports.push(report);
    }

    let all_measured_rungs_in_target_band =
        rung_reports.iter().all(|report| report.target_band_met);
    let report = LadderReport {
        schema_version: 1,
        status: "diagnostics_only",
        protocol: "paired four-player 2v2 controller attribution; same seed, decks, physical seats, and seat-tied policy RNG; controller ownership swaps on the second game",
        manifest: config.manifest.display().to_string(),
        games_per_rung: config.games_per_rung,
        jobs: config.jobs,
        max_turns: config.max_turns,
        seed_base: SEED_BASE,
        tier_registry_status: tiers.calibration_status().to_owned(),
        fixed_hardware_and_thread_configuration: true,
        rung_reports,
        all_measured_rungs_in_target_band,
        promotion_eligible: false,
        promotion_blockers: [
            "one mixed diagnostics pod is not the required aggro/midrange/control campaign",
            "PilotIntent classifies all four current integration decks as Limited",
            "Track B acceptable-action and catastrophic-blunder labels are not sealed",
            "reference Android and WASM latency evidence is not present",
        ],
    };
    write_report(&config.output, &report)?;
    println!("wrote {}", config.output.display());
    Ok(())
}

pub(crate) fn calibrate(args: &[String]) -> Result<(), String> {
    let mut config = LadderConfig::parse(args)?;
    if value_string(args, "--output")?.is_none() {
        config.output = PathBuf::from(DEFAULT_CALIBRATION_OUTPUT);
    }
    let target = match value_string(args, "--tier")?.as_deref() {
        Some("expert") | None => Tier::Expert,
        Some("master") => Tier::Master,
        Some(other) => {
            return Err(format!(
                "unsupported calibration tier `{other}`; expected expert or master"
            ));
        }
    };
    let baseline = match target {
        Tier::Expert => Tier::Standard,
        Tier::Master => Tier::Expert,
        _ => unreachable!("calibration target is validated above"),
    };
    let budgets = parse_budgets(args)?.unwrap_or_else(|| vec![10, 25, 50, 100, 250, 500, 1_000]);
    let arena = Arc::new(AiArena::load(&config.manifest)?);
    let tiers = Arc::new(
        AiTierSet::bundled().map_err(|error| format!("failed to load AI tiers: {error}"))?,
    );
    let target_definition = tiers.tier(target.difficulty());
    let mut low = 0_usize;
    let mut high = budgets.len();
    let mut selected = None;
    let mut trials = Vec::new();
    while low < high {
        let middle = low + (high - low) / 2;
        let budget = budgets[middle];
        let candidate = PolicySpec::TimedTrial {
            label: target.name(),
            think_ms: budget,
            determinizations: target_definition.determinizations(),
            workers: target_definition.workers(),
            profile: target.guardrail_profile(),
            adaptive: false,
        };
        let rung = Rung {
            lower: PolicySpec::Tier(baseline),
            upper: candidate,
        };
        let trial = run_rung(Arc::clone(&arena), Arc::clone(&tiers), &config, rung)?;
        println!(
            "calibration {}ms: {:.2}% [{:.2}%, {:.2}%]",
            budget,
            trial.candidate_win_rate * 100.0,
            trial.wilson_95[0] * 100.0,
            trial.wilson_95[1] * 100.0
        );
        let rate = trial.candidate_win_rate;
        trials.push(trial);
        if (0.65..=0.75).contains(&rate) {
            selected = Some(budget);
            high = middle;
        } else if rate < 0.65 {
            low = middle.saturating_add(1);
        } else {
            high = middle;
        }
    }
    trials.sort_by_key(|trial| {
        trial
            .upper
            .split('-')
            .find_map(|piece| piece.strip_suffix("ms"))
            .and_then(|piece| piece.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    let report = CalibrationReport {
        schema_version: 1,
        status: "diagnostics_only_unpromoted",
        target_tier: target.name().to_owned(),
        baseline_tier: baseline.name().to_owned(),
        target_win_rate_band: [0.65, 0.75],
        tested_budgets_ms: trials
            .iter()
            .filter_map(|trial| {
                trial
                    .upper
                    .split('-')
                    .find_map(|piece| piece.strip_suffix("ms"))
                    .and_then(|piece| piece.parse::<u32>().ok())
            })
            .collect(),
        selected_candidate_budget_ms: selected,
        games_per_budget: config.games_per_rung,
        selection_rule: "Discrete binary search for the smallest measured budget in the 65-75% controller-team win-rate band; every trial reuses paired seeds and seat rotations.",
        tier_registry_updated: false,
        promotion_eligible: false,
        promotion_blockers: [
            "a one-pod diagnostic cannot replace the three-archetype calibration campaign",
            "the confidence interval and sealed validation evidence are not promotion-authoritative",
            "ai_tiers.ron remains provisional until exact gate evidence passes",
        ],
        trials,
    };
    write_report(&config.output, &report)?;
    println!("wrote {}", config.output.display());
    Ok(())
}

pub(crate) fn search_knee(args: &[String]) -> Result<(), String> {
    let mut config = LadderConfig::parse(args)?;
    if value_string(args, "--output")?.is_none() {
        config.output = PathBuf::from(DEFAULT_KNEE_OUTPUT);
    }
    let target = match value_string(args, "--tier")?.as_deref() {
        Some("standard") | None => Tier::Standard,
        Some("expert") => Tier::Expert,
        Some("master") => Tier::Master,
        Some(other) => {
            return Err(format!(
                "unsupported search-knee tier `{other}`; expected standard, expert, or master"
            ));
        }
    };
    let budgets = parse_budgets(args)?.unwrap_or_else(|| vec![10, 20, 40, 80, 160, 320, 640]);
    let budget_pairs = budgets
        .windows(2)
        .filter_map(|pair| (pair[1] == pair[0].saturating_mul(2)).then_some((pair[0], pair[1])))
        .collect::<Vec<_>>();
    if budget_pairs.len() < 2 {
        return Err(
            "--search-knee needs at least two adjacent B/2B increases, such as 10,20,40".to_owned(),
        );
    }

    let arena = Arc::new(AiArena::load(&config.manifest)?);
    let tiers = Arc::new(
        AiTierSet::bundled().map_err(|error| format!("failed to load AI tiers: {error}"))?,
    );
    let definition = tiers.tier(target.difficulty());
    let trial = |budget, adaptive| PolicySpec::TimedTrial {
        label: target.name(),
        think_ms: budget,
        determinizations: definition.determinizations(),
        workers: definition.workers(),
        profile: target.guardrail_profile(),
        adaptive,
    };

    let mut comparisons = Vec::with_capacity(budget_pairs.len());
    for (lower_budget, upper_budget) in budget_pairs {
        let rung = Rung {
            lower: trial(lower_budget, false),
            upper: trial(upper_budget, false),
        };
        let arena_report = run_rung(Arc::clone(&arena), Arc::clone(&tiers), &config, rung)?;
        let report = budget_comparison(lower_budget, upper_budget, arena_report);
        println!(
            "knee {}ms->{}ms: {:+.2}pp, {:+.1} Elo",
            lower_budget,
            upper_budget,
            report.paired_win_rate_improvement_percentage_points,
            report.estimated_elo_improvement
        );
        comparisons.push(report);
    }

    let mut adaptive_ablations = Vec::new();
    if !args.iter().any(|arg| arg == "--skip-adaptive-ablation") {
        for budget in budgets.iter().copied() {
            let rung = Rung {
                lower: trial(budget, false),
                upper: trial(budget, true),
            };
            let arena_report = run_rung(Arc::clone(&arena), Arc::clone(&tiers), &config, rung)?;
            let report = adaptive_ablation(budget, arena_report);
            println!(
                "adaptive {}ms: {:+.2}pp, fixed/adaptive p95 {:?}/{:?}us",
                budget,
                report.adaptive_win_rate_delta_percentage_points,
                report.fixed_p95_wall_latency_us,
                report.adaptive_p95_wall_latency_us
            );
            adaptive_ablations.push(report);
        }
    }

    let (two_consecutive_complete_plateaus, provisional_knee_budget_ms) =
        confirmed_knee(&comparisons);
    let report = SearchKneeReport {
        schema_version: 1,
        status: "diagnostics_only_unpromoted",
        threshold_status: "owner_approved_provisional_2026-07-14",
        target_tier: target.name().to_owned(),
        budgets_ms: budgets,
        games_per_comparison: config.games_per_rung,
        jobs: config.jobs,
        max_turns: config.max_turns,
        manifest: config.manifest.display().to_string(),
        matched_protocol: "Every B/2B and fixed/adaptive comparison reuses the same pod, physical seats, controller rotations, hidden-information seed derivation, legal-action generator, hardware, and worker configuration.",
        provisional_thresholds: SearchKneeThresholds {
            paired_win_rate_gain_lt_percentage_points: 1.0,
            estimated_elo_gain_lt_range: [10.0, 15.0],
            ordinary_state_acceptable_action_agreement_at_least: 0.95,
            consecutive_budget_increases: 2,
            material_latency_and_cost_threshold: None,
        },
        comparisons,
        adaptive_ablations,
        two_consecutive_complete_plateaus,
        provisional_knee_budget_ms,
        selected_standard_budget_ms: None,
        promotion_eligible: false,
        promotion_blockers: [
            "Track B catastrophic-blunder and missed-win/required-defense labels are not sealed",
            "ordinary benchmark-state acceptable-action agreement is not yet measured",
            "the material CPU-cost threshold and process CPU adapter are not yet approved",
            "the competence bar must pass before selecting the smallest pre-plateau Standard budget",
            "three archetype tracks plus reference Android/WASM latency evidence remain required",
        ],
    };
    write_report(&config.output, &report)?;
    println!("wrote {}", config.output.display());
    Ok(())
}

fn budget_comparison(
    lower_budget_ms: u32,
    upper_budget_ms: u32,
    arena: RungReport,
) -> BudgetComparisonReport {
    let gain_pp = (arena.candidate_win_rate - 0.5) * 100.0;
    let elo = arena.controller_team_elo_estimate;
    let confidence_interval_pp = [
        (arena.wilson_95[0] - 0.5) * 100.0,
        (arena.wilson_95[1] - 0.5) * 100.0,
    ];
    let useful_score = score_from_elo(10.0).max(0.51);
    let lower_p95 = arena
        .totals
        .search_wall_latency_p95_by_budget_ms
        .get(&lower_budget_ms)
        .copied();
    let upper_p95 = arena
        .totals
        .search_wall_latency_p95_by_budget_ms
        .get(&upper_budget_ms)
        .copied();
    BudgetComparisonReport {
        lower_budget_ms,
        upper_budget_ms,
        paired_win_rate_improvement_percentage_points: gain_pp,
        estimated_elo_improvement: elo,
        confidence_interval_improvement_percentage_points: confidence_interval_pp,
        gain_below_one_percentage_point_or_fifteen_elo: gain_pp < 1.0 || elo < 15.0,
        confidence_interval_includes_no_product_useful_improvement: arena.wilson_95[0]
            <= useful_score,
        catastrophic_blunder_rate_meaningfully_decreased: None,
        missed_win_or_required_defense_rate_meaningfully_decreased: None,
        ordinary_state_acceptable_action_agreement: None,
        lower_p95_wall_latency_us: lower_p95,
        upper_p95_wall_latency_us: upper_p95,
        p95_wall_latency_increased: lower_p95.zip(upper_p95).map(|(lower, upper)| upper > lower),
        measured_cpu_cost_increased_materially: None,
        all_acceptance_criteria_complete: false,
        plateau_acceptance_passed: false,
        arena,
    }
}

fn adaptive_ablation(budget_ms: u32, arena: RungReport) -> AdaptiveAblationReport {
    let fixed_p95 = arena
        .totals
        .search_wall_latency_p95_by_budget_ms
        .get(&budget_ms)
        .copied();
    let adaptive_p95 = arena
        .totals
        .adaptive_search_wall_latency_p95_by_budget_ms
        .get(&budget_ms)
        .copied();
    let reduction = fixed_p95.zip(adaptive_p95).and_then(|(fixed, adaptive)| {
        (fixed > 0).then_some((fixed as f64 - adaptive as f64) / fixed as f64)
    });
    AdaptiveAblationReport {
        budget_ms,
        adaptive_win_rate_delta_percentage_points: (arena.candidate_win_rate - 0.5) * 100.0,
        adaptive_elo_delta: arena.controller_team_elo_estimate,
        confidence_interval_delta_percentage_points: [
            (arena.wilson_95[0] - 0.5) * 100.0,
            (arena.wilson_95[1] - 0.5) * 100.0,
        ],
        fixed_p95_wall_latency_us: fixed_p95,
        adaptive_p95_wall_latency_us: adaptive_p95,
        p95_wall_latency_reduction_fraction: reduction,
        measured_cpu_cost_reduction_fraction: None,
        no_practical_strength_decline_proven: arena.wilson_95[0] >= 0.49,
        tracks_a_b_c_complete: false,
        shipping_eligible: false,
        arena,
    }
}

fn confirmed_knee(comparisons: &[BudgetComparisonReport]) -> (bool, Option<u32>) {
    for pair in comparisons.windows(2) {
        let consecutive = pair[0].upper_budget_ms == pair[1].lower_budget_ms;
        if consecutive && pair[0].plateau_acceptance_passed && pair[1].plateau_acceptance_passed {
            return (true, Some(pair[0].lower_budget_ms));
        }
    }
    (false, None)
}

fn run_rung(
    arena: Arc<AiArena>,
    tiers: Arc<AiTierSet>,
    config: &LadderConfig,
    rung: Rung,
) -> Result<RungReport, String> {
    let pair_count = config.games_per_rung / 2;
    let next_pair = Arc::new(AtomicU32::new(0));
    let results = Arc::new(Mutex::new(Vec::<Result<PairEvidence, String>>::new()));
    let maximum_search_workers_per_game = rung
        .lower
        .search_workers(&tiers)
        .max(rung.upper.search_workers(&tiers));
    let worker_count = config
        .jobs
        .min(pair_count)
        .min((MAX_JOBS / maximum_search_workers_per_game.saturating_mul(2).max(1)).max(1))
        .max(1);
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let arena = Arc::clone(&arena);
            let tiers = Arc::clone(&tiers);
            let next_pair = Arc::clone(&next_pair);
            let results = Arc::clone(&results);
            scope.spawn(move || loop {
                let pair_index = next_pair.fetch_add(1, Ordering::Relaxed);
                if pair_index >= pair_count {
                    break;
                }
                let result = run_pair(&arena, &tiers, config.max_turns, rung, pair_index);
                match results.lock() {
                    Ok(mut results) => results.push(result),
                    Err(poisoned) => {
                        poisoned
                            .into_inner()
                            .push(Err("ladder result mutex poisoned".to_owned()));
                        break;
                    }
                }
            });
        }
    });

    let mut pairs = Vec::with_capacity(pair_count as usize);
    for result in Arc::into_inner(results)
        .ok_or_else(|| "ladder workers retained the result buffer".to_owned())?
        .into_inner()
        .map_err(|_| "ladder result mutex was poisoned".to_owned())?
    {
        pairs.push(result?);
    }
    pairs.sort_by_key(|pair| pair.pair_index);
    if pairs.len() != pair_count as usize {
        return Err(format!(
            "{} produced {} of {pair_count} pairs",
            rung.name(),
            pairs.len()
        ));
    }

    let mut candidate_wins = 0_u32;
    let mut candidate_sweeps = 0_u32;
    let mut split_pairs = 0_u32;
    let mut candidate_losses_both = 0_u32;
    let mut totals = CampaignTotals::default();
    let mut evidence = Vec::with_capacity(config.games_per_rung as usize);
    for pair in pairs {
        let pair_wins = pair.games.iter().filter(|game| game.candidate_won).count() as u32;
        candidate_wins = candidate_wins.saturating_add(pair_wins);
        match pair_wins {
            2 => candidate_sweeps = candidate_sweeps.saturating_add(1),
            1 => split_pairs = split_pairs.saturating_add(1),
            _ => candidate_losses_both = candidate_losses_both.saturating_add(1),
        }
        for game in pair.games {
            totals.add(&game);
            evidence.push(game);
        }
    }
    totals.finalize();
    let rate = f64::from(candidate_wins) / f64::from(config.games_per_rung);
    let wilson = wilson_interval(candidate_wins, config.games_per_rung);
    Ok(RungReport {
        rung: rung.name(),
        lower: rung.lower.name(),
        upper: rung.upper.name(),
        games: config.games_per_rung,
        pairs: pair_count,
        parallel_game_workers: worker_count,
        maximum_search_workers_per_game,
        candidate_wins,
        candidate_win_rate: rate,
        wilson_95: wilson,
        controller_team_elo_estimate: elo_from_score(rate),
        target_band: [0.65, 0.75],
        target_band_met: (0.65..=0.75).contains(&rate),
        candidate_sweeps,
        split_pairs,
        candidate_losses_both,
        totals,
        evidence,
    })
}

fn run_pair(
    arena: &AiArena,
    tiers: &AiTierSet,
    max_turns: u32,
    rung: Rung,
    pair_index: u32,
) -> Result<PairEvidence, String> {
    let seed = splitmix64(SEED_BASE ^ u64::from(pair_index));
    let (first, second) = thread::scope(|scope| {
        let first =
            scope.spawn(|| run_rotation(arena, tiers, max_turns, rung, pair_index, seed, 0));
        let second =
            scope.spawn(|| run_rotation(arena, tiers, max_turns, rung, pair_index, seed, 1));
        Ok::<_, String>((
            first
                .join()
                .map_err(|_| "first seat rotation panicked".to_owned())?,
            second
                .join()
                .map_err(|_| "second seat rotation panicked".to_owned())?,
        ))
    })?;
    Ok(PairEvidence {
        pair_index,
        games: [first?, second?],
    })
}

fn run_rotation(
    arena: &AiArena,
    tiers: &AiTierSet,
    max_turns: u32,
    rung: Rung,
    pair_index: u32,
    seed: u64,
    rotation: u8,
) -> Result<GameEvidence, String> {
    let policies = std::array::from_fn(|seat| {
        let policy_seed = splitmix64(seed ^ (seat as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let candidate = seat % 2 == usize::from(rotation);
        if candidate {
            rung.upper.policy(policy_seed, tiers)
        } else {
            rung.lower.policy(policy_seed, tiers)
        }
    });
    let started = Instant::now();
    let summary = arena.run_game(seed, max_turns, policies).map_err(|error| {
        format!(
            "{} pair {pair_index} rotation {rotation} seed {seed}: {error}",
            rung.name()
        )
    })?;
    Ok(game_evidence(
        pair_index,
        rotation,
        summary,
        started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    ))
}

fn game_evidence(
    pair_index: u32,
    rotation: u8,
    summary: AiArenaSummary,
    wall_runtime_us: u64,
) -> GameEvidence {
    GameEvidence {
        pair_index,
        rotation,
        seed: summary.seed,
        winner_seat: summary.winner_seat,
        candidate_won: summary.winner_seat % 2 == usize::from(rotation),
        turns: summary.turns,
        final_hash: summary.final_hash,
        typed_actions: summary.typed_actions,
        decisions: summary.decisions,
        searched_decisions: summary.searched_decisions,
        singleton_bypasses: summary.singleton_bypasses,
        simulations: summary.simulations,
        nodes: summary.nodes,
        transposition_hits: summary.transposition_hits,
        maximum_depth: summary.maximum_depth,
        search_wall_latency_us: summary.search_wall_latency_us,
        search_wall_latency_p95_us: percentile(&summary.search_wall_latencies_us, 95),
        wall_runtime_us,
        search_wall_latencies_us: summary.search_wall_latencies_us,
        search_wall_latencies_by_budget_ms: summary.search_wall_latencies_by_budget_ms,
        adaptive_search_wall_latencies_by_budget_ms: summary
            .adaptive_search_wall_latencies_by_budget_ms,
    }
}

fn percentile(samples: &[u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let numerator = percentile
        .saturating_mul(ordered.len().saturating_sub(1))
        .saturating_add(99);
    ordered[numerator / 100]
}

fn wilson_interval(successes: u32, trials: u32) -> [f64; 2] {
    if trials == 0 {
        return [0.0, 1.0];
    }
    let n = f64::from(trials);
    let p = f64::from(successes) / n;
    let z = 1.959_963_984_540_054_f64;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let radius = z * ((p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / denominator;
    [(center - radius).max(0.0), (center + radius).min(1.0)]
}

fn elo_from_score(score: f64) -> f64 {
    let bounded = score.clamp(0.000_001, 0.999_999);
    400.0 * (bounded / (1.0 - bounded)).log10()
}

fn score_from_elo(elo: f64) -> f64 {
    1.0 / (1.0 + 10.0_f64.powf(-elo / 400.0))
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn value_u32(args: &[String], flag: &str) -> Result<Option<u32>, String> {
    value_string(args, flag)?
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("invalid integer after {flag}: {value}"))
        })
        .transpose()
}

fn value_string(args: &[String], flag: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.get(index + 1)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("missing value after {flag}"))
}

fn parse_budgets(args: &[String]) -> Result<Option<Vec<u32>>, String> {
    let Some(raw) = value_string(args, "--budgets")? else {
        return Ok(None);
    };
    let mut budgets = raw
        .split(',')
        .map(|value| {
            value
                .trim()
                .parse::<u32>()
                .map_err(|_| format!("invalid --budgets value `{value}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    budgets.sort_unstable();
    budgets.dedup();
    if budgets.is_empty() || budgets[0] == 0 {
        return Err("--budgets must contain positive comma-separated milliseconds".to_owned());
    }
    Ok(Some(budgets))
}

fn write_report(path: &Path, report: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("failed to serialize ladder report: {error}"))?;
    fs::write(path, payload).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{elo_from_score, percentile, score_from_elo, wilson_interval};
    use forge_core::DecisionKind;
    use serde::Deserialize;
    use serde_json::Value;
    use std::collections::BTreeSet;

    #[derive(Deserialize)]
    struct DecisionSurface {
        schema_version: u32,
        decision_context_schema: u32,
        gate_status: String,
        allowed_statuses: Vec<String>,
        families: Vec<DecisionFamily>,
    }

    #[derive(Deserialize)]
    struct DecisionFamily {
        kind: String,
        human: String,
        ai: String,
        benchmark: String,
        note: String,
    }

    #[test]
    fn statistical_helpers_are_bounded_and_centered() {
        let interval = wilson_interval(200, 400);
        assert!(interval[0] < 0.5 && interval[1] > 0.5);
        assert_eq!(elo_from_score(0.5), 0.0);
        assert!(elo_from_score(0.7) > 140.0);
        assert!((score_from_elo(elo_from_score(0.7)) - 0.7).abs() < 0.000_001);
        assert_eq!(percentile(&[40, 10, 20, 30], 50), 30);
        assert_eq!(percentile(&[40, 10, 20, 30], 95), 40);
    }

    #[test]
    fn decision_surface_registry_covers_every_schema_kind_and_fails_closed() {
        let registry: DecisionSurface =
            serde_json::from_str(include_str!("../../../assets/ai/decision_surface.json"))
                .unwrap_or_else(|error| panic!("decision surface should parse: {error}"));
        assert_eq!(registry.schema_version, 1);
        assert_eq!(registry.decision_context_schema, 1);
        let expected = DecisionKind::ALL
            .into_iter()
            .map(DecisionKind::registry_key)
            .collect::<BTreeSet<_>>();
        let actual = registry
            .families
            .iter()
            .map(|family| family.kind.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), registry.families.len());
        let allowed = registry
            .allowed_statuses
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert!(registry.families.iter().all(|family| {
            !family.note.trim().is_empty()
                && [&family.human, &family.ai, &family.benchmark]
                    .into_iter()
                    .all(|status| allowed.contains(status.as_str()))
        }));
        let complete = registry.families.iter().all(|family| {
            family.human == "complete" && family.ai == "complete" && family.benchmark == "complete"
        });
        assert_eq!(
            registry.gate_status,
            if complete {
                "ready"
            } else {
                "blocked_incomplete_adapters"
            }
        );
    }

    #[test]
    fn learning_protocol_schemas_retain_the_durable_contract() {
        let manifest = parse_schema(include_str!(
            "../../../schemas/learning/v1/game_manifest.schema.json"
        ));
        assert_required(
            &manifest,
            &[
                "session_id",
                "episode_id",
                "engine",
                "rules",
                "card_database",
                "decks",
                "controllers",
                "pilot_intent",
                "participants",
                "capture_mode",
                "provenance",
                "timestamps",
                "terminal",
                "replay_sha256",
            ],
        );

        let decision = parse_schema(include_str!(
            "../../../schemas/learning/v1/decision_record.schema.json"
        ));
        assert_required(
            &decision,
            &[
                "episode_id",
                "decision_index",
                "context",
                "public_state_hash",
                "player_view",
                "known_information_mask_sha256",
                "legal_actions",
                "selected_action_id",
                "choice_latency_us",
                "takeback_or_misclick",
                "shadow_ai",
                "bounded_solver",
                "compute",
                "resulting_state_hash",
            ],
        );
        let context = &decision["properties"]["context"];
        assert_required(
            context,
            &[
                "id",
                "state_key",
                "kind",
                "turn",
                "phase",
                "step",
                "priority_seat",
                "stack_depth",
            ],
        );
        let compute = &decision["properties"]["compute"];
        assert_required(
            compute,
            &[
                "configured_wall_ms",
                "configured_cpu_ms",
                "actual_wall_us",
                "actual_cpu_us",
                "nodes",
                "determinizations",
                "legal_action_count",
                "maximum_depth",
                "transposition_hits",
                "value_gap",
                "visit_gap",
                "uncertainty_ppm",
                "memory_delta_bytes",
                "stop_reason",
                "checkpoints",
            ],
        );
        let checkpoint = &compute["properties"]["checkpoints"]["items"];
        assert_required(
            checkpoint,
            &[
                "simulations",
                "leading_action_id",
                "leading_visit_share_ppm",
                "value_gap",
                "visit_gap",
                "ranking_stable",
                "uncertainty_ppm",
                "bounded_solver_state",
                "stop_reason",
            ],
        );

        let review = parse_schema(include_str!(
            "../../../schemas/learning/v1/post_game_review.schema.json"
        ));
        let reviewed_decision = &review["properties"]["decision_reviews"]["items"];
        assert_required(
            reviewed_decision,
            &[
                "assessment",
                "preferred_action_ids",
                "acceptable_action_ids",
                "questionable_action_ids",
                "catastrophic_action_ids",
                "reason_tags",
                "confidence",
                "hindsight_used",
                "intended_deck_line",
                "social_constraint",
                "rationale",
            ],
        );
    }

    fn parse_schema(source: &str) -> Value {
        let schema: Value = serde_json::from_str(source)
            .unwrap_or_else(|error| panic!("learning schema should parse: {error}"));
        assert_eq!(schema["additionalProperties"], Value::Bool(false));
        schema
    }

    fn assert_required(schema: &Value, expected: &[&str]) {
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("schema must have a required array"))
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("required entry must be a string"))
            })
            .collect::<BTreeSet<_>>();
        for field in expected {
            assert!(
                required.contains(field),
                "required field `{field}` is missing"
            );
        }
    }
}
