use forge_core::{CanonicalActionId, DecisionContext};
use std::{
    collections::{BTreeSet, HashMap},
    error::Error,
    fmt, thread,
    time::{Duration, Instant},
};

const VALUE_LIMIT: i64 = 1_000_000_000;
const PPM: f64 = 1_000_000.0;

/// Search work limit for one determinization tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchLimit {
    /// Deterministic development/replay limit.
    Iterations(u32),
    /// Product-facing wall-time limit.
    WallTime(Duration),
}

/// Deterministic progressive-widening schedule for large action sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressiveWidening {
    initial_actions: u32,
    actions_per_sqrt_visit: u32,
}

impl ProgressiveWidening {
    /// Creates a widening schedule ordered by [`SearchDomain::action_prior`].
    #[must_use]
    pub const fn new(initial_actions: u32, actions_per_sqrt_visit: u32) -> Self {
        Self {
            initial_actions,
            actions_per_sqrt_visit,
        }
    }

    /// Returns the initial active action count.
    #[must_use]
    pub const fn initial_actions(self) -> u32 {
        self.initial_actions
    }

    /// Returns additional active actions per integer square-root visit.
    #[must_use]
    pub const fn actions_per_sqrt_visit(self) -> u32 {
        self.actions_per_sqrt_visit
    }
}

/// Experimental adaptive-stop thresholds.
///
/// These thresholds are not promotion-authoritative until they pass the
/// paired fixed-budget ablation required by `T4_SEARCH_KNEE.md`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdaptiveStopping {
    enabled: bool,
    checkpoints: Vec<u32>,
    stable_checkpoints: u32,
    minimum_leader_share_ppm: u32,
    minimum_visit_gap: u32,
    minimum_value_gap: i64,
    maximum_entropy_ppm: u32,
}

impl AdaptiveStopping {
    /// Disables all non-forced adaptive stopping.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            checkpoints: Vec::new(),
            stable_checkpoints: 0,
            minimum_leader_share_ppm: 0,
            minimum_visit_gap: 0,
            minimum_value_gap: 0,
            maximum_entropy_ppm: 1_000_000,
        }
    }

    /// Creates experimental thresholds for local ablation.
    #[must_use]
    pub fn experimental(
        checkpoints: Vec<u32>,
        stable_checkpoints: u32,
        minimum_leader_share_ppm: u32,
        minimum_visit_gap: u32,
        minimum_value_gap: i64,
        maximum_entropy_ppm: u32,
    ) -> Self {
        Self {
            enabled: true,
            checkpoints,
            stable_checkpoints,
            minimum_leader_share_ppm,
            minimum_visit_gap,
            minimum_value_gap,
            maximum_entropy_ppm,
        }
    }
}

/// Root-parallel determinized UCT configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchConfig {
    seed: u64,
    determinizations: u32,
    workers: u32,
    limit: SearchLimit,
    rollout_depth: u32,
    exploration_milli: u32,
    progressive_widening: Option<ProgressiveWidening>,
    adaptive: AdaptiveStopping,
}

impl SearchConfig {
    /// Creates a deterministic fixed-iteration configuration.
    #[must_use]
    pub fn fixed_iterations(seed: u64, determinizations: u32, iterations: u32) -> Self {
        Self {
            seed,
            determinizations,
            workers: determinizations.max(1),
            limit: SearchLimit::Iterations(iterations),
            rollout_depth: 24,
            exploration_milli: 1_414,
            progressive_widening: Some(ProgressiveWidening::new(8, 2)),
            adaptive: AdaptiveStopping::disabled(),
        }
    }

    /// Creates a wall-time configuration for product latency experiments.
    #[must_use]
    pub fn wall_time(seed: u64, determinizations: u32, think_ms: u64) -> Self {
        Self {
            seed,
            determinizations,
            workers: determinizations.max(1),
            limit: SearchLimit::WallTime(Duration::from_millis(think_ms)),
            rollout_depth: 24,
            exploration_milli: 1_414,
            progressive_widening: Some(ProgressiveWidening::new(8, 2)),
            adaptive: AdaptiveStopping::disabled(),
        }
    }

    /// Sets the maximum local worker count.
    #[must_use]
    pub const fn with_workers(mut self, workers: u32) -> Self {
        self.workers = workers;
        self
    }

    /// Sets the maximum rollout depth after expansion.
    #[must_use]
    pub const fn with_rollout_depth(mut self, depth: u32) -> Self {
        self.rollout_depth = depth;
        self
    }

    /// Sets the UCT exploration constant multiplied by 1,000.
    #[must_use]
    pub const fn with_exploration_milli(mut self, exploration_milli: u32) -> Self {
        self.exploration_milli = exploration_milli;
        self
    }

    /// Sets or disables deterministic progressive widening.
    #[must_use]
    pub const fn with_progressive_widening(
        mut self,
        progressive_widening: Option<ProgressiveWidening>,
    ) -> Self {
        self.progressive_widening = progressive_widening;
        self
    }

    /// Enables an experimental adaptive-stop configuration.
    #[must_use]
    pub fn with_adaptive_stopping(mut self, adaptive: AdaptiveStopping) -> Self {
        self.adaptive = adaptive;
        self
    }

    /// Returns the configured search seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the number of independent hidden-information samples.
    #[must_use]
    pub const fn determinizations(&self) -> u32 {
        self.determinizations
    }

    /// Returns the configured local worker ceiling.
    #[must_use]
    pub const fn workers(&self) -> u32 {
        self.workers
    }

    /// Returns the work limit applied to each tree.
    #[must_use]
    pub const fn limit(&self) -> SearchLimit {
        self.limit
    }
}

/// Certified bounded-solver result supplied by a search domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedSolution {
    /// One action is a certified win.
    Win(CanonicalActionId),
    /// One action is a certified required defense.
    RequiredDefense(CanonicalActionId),
}

/// Why one determinization tree or aggregate search stopped.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SearchStopReason {
    /// No search was needed because only one legal action existed.
    SingletonLegalAction,
    /// A bounded solver certified a winning action.
    CertifiedWin,
    /// A bounded solver certified a required defensive action.
    CertifiedRequiredDefense,
    /// Experimental stability, gap, and uncertainty thresholds all passed.
    AdaptiveStableLeader,
    /// The fixed simulation count was exhausted.
    FixedIterations,
    /// The wall-time budget expired.
    WallTimeBudget,
    /// Independent trees stopped for different reasons.
    Mixed,
}

/// Opaque game adapter consumed by root-parallel search.
///
/// Implementations own determinization and transition details. Search never
/// receives a production `GameState` or display label. Values are always from
/// the root actor's perspective and are clamped to +/-1e9.
pub trait SearchDomain: Sync {
    /// Opaque determinized search state.
    type State: Clone + Send;

    /// Builds one legal hidden-information sample from a deterministic seed.
    fn determinize(&self, seed: u64) -> Result<Self::State, String>;

    /// Enumerates canonical legal action IDs for one state.
    fn legal_actions(&self, state: &Self::State) -> Result<Vec<CanonicalActionId>, String>;

    /// Applies one canonical legal action to a cloned state.
    fn apply_action(
        &self,
        state: &Self::State,
        action: CanonicalActionId,
    ) -> Result<Self::State, String>;

    /// Returns terminal utility from the root actor's perspective.
    fn terminal_value(&self, state: &Self::State) -> Option<i64>;

    /// Evaluates a nonterminal leaf from the root actor's perspective.
    fn evaluate(&self, state: &Self::State) -> i64;

    /// Chooses one rollout action from the supplied complete legal set.
    fn rollout_action(
        &self,
        state: &Self::State,
        actions: &[CanonicalActionId],
        seed: u64,
    ) -> Result<CanonicalActionId, String>;

    /// Returns +1 when the node actor maximizes root value, -1 when it minimizes.
    fn selection_sign(&self, _state: &Self::State) -> i8 {
        1
    }

    /// Returns a card-agnostic move-ordering prior.
    fn action_prior(&self, _state: &Self::State, _action: CanonicalActionId) -> i64 {
        0
    }

    /// Returns a deterministic equivalence group for widening order.
    ///
    /// The default gives every action its own group. Domain adapters may group
    /// actions only when they preserve the complete concrete legal set and can
    /// justify equivalent transition semantics.
    fn action_group(&self, _state: &Self::State, action: CanonicalActionId) -> u64 {
        let value = action.get();
        value as u64 ^ (value >> 64) as u64
    }

    /// Returns a deterministic state key for transposition sharing.
    ///
    /// Domains that cannot provide a complete key return `None`; search then
    /// retains tree semantics and reports zero transposition hits.
    fn state_key(&self, _state: &Self::State) -> Option<u64> {
        None
    }

    /// Runs an optional exact bounded win/defense solver at the root.
    fn bounded_solution(
        &self,
        _state: &Self::State,
        _actions: &[CanonicalActionId],
    ) -> Option<BoundedSolution> {
        None
    }
}

/// One fixed-visit adaptive checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchCheckpoint {
    determinization: u32,
    simulations: u32,
    leading_action: CanonicalActionId,
    leading_visit_share_ppm: u32,
    value_gap: i64,
    visit_gap: u32,
    ranking_stable: bool,
    uncertainty_ppm: u32,
    bounded_solver_state: &'static str,
    stop_reason: Option<SearchStopReason>,
}

impl SearchCheckpoint {
    /// Returns the determinization ordinal.
    #[must_use]
    pub const fn determinization(&self) -> u32 {
        self.determinization
    }

    /// Returns completed simulations at this checkpoint.
    #[must_use]
    pub const fn simulations(&self) -> u32 {
        self.simulations
    }

    /// Returns the current leading canonical action.
    #[must_use]
    pub const fn leading_action(&self) -> CanonicalActionId {
        self.leading_action
    }

    /// Returns the leader's root-visit share in parts per million.
    #[must_use]
    pub const fn leading_visit_share_ppm(&self) -> u32 {
        self.leading_visit_share_ppm
    }

    /// Returns the leader-minus-runner-up mean-value gap.
    #[must_use]
    pub const fn value_gap(&self) -> i64 {
        self.value_gap
    }

    /// Returns the leader-minus-runner-up visit gap.
    #[must_use]
    pub const fn visit_gap(&self) -> u32 {
        self.visit_gap
    }

    /// Returns whether the full action ranking matches the prior checkpoint.
    #[must_use]
    pub const fn ranking_stable(&self) -> bool {
        self.ranking_stable
    }

    /// Returns normalized visit entropy in parts per million.
    #[must_use]
    pub const fn uncertainty_ppm(&self) -> u32 {
        self.uncertainty_ppm
    }

    /// Returns the bounded-solver state recorded at the checkpoint.
    #[must_use]
    pub const fn bounded_solver_state(&self) -> &'static str {
        self.bounded_solver_state
    }

    /// Returns the stop reason when this checkpoint ended the tree.
    #[must_use]
    pub const fn stop_reason(&self) -> Option<SearchStopReason> {
        self.stop_reason
    }
}

/// Aggregated root statistics for one legal action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchActionReport {
    action: CanonicalActionId,
    visits: u64,
    total_value: i128,
    mean_value: i64,
}

impl SearchActionReport {
    /// Returns the canonical action ID.
    #[must_use]
    pub const fn action(self) -> CanonicalActionId {
        self.action
    }

    /// Returns summed visits across determinization trees.
    #[must_use]
    pub const fn visits(self) -> u64 {
        self.visits
    }

    /// Returns summed root utility across visits.
    #[must_use]
    pub const fn total_value(self) -> i128 {
        self.total_value
    }

    /// Returns integer mean root utility.
    #[must_use]
    pub const fn mean_value(self) -> i64 {
        self.mean_value
    }
}

/// Complete inspectable report for one searched decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchReport {
    selected_action: CanonicalActionId,
    actions: Vec<SearchActionReport>,
    configured_limit: SearchLimit,
    determinizations: u32,
    workers: u32,
    simulations: u64,
    nodes: u64,
    maximum_depth: u32,
    transposition_hits: u64,
    actual_wall_time_us: u64,
    actual_cpu_time_us: Option<u64>,
    memory_delta_bytes: Option<i64>,
    value_gap: i64,
    visit_gap: u64,
    uncertainty_ppm: u32,
    stop_reason: SearchStopReason,
    checkpoints: Vec<SearchCheckpoint>,
}

impl SearchReport {
    /// Returns the selected visit-sum action.
    #[must_use]
    pub const fn selected_action(&self) -> CanonicalActionId {
        self.selected_action
    }

    /// Returns canonical root reports sorted by action ID.
    #[must_use]
    pub fn actions(&self) -> &[SearchActionReport] {
        &self.actions
    }

    /// Returns the configured per-tree work limit.
    #[must_use]
    pub const fn configured_limit(&self) -> SearchLimit {
        self.configured_limit
    }

    /// Returns completed hidden-information samples.
    #[must_use]
    pub const fn determinizations(&self) -> u32 {
        self.determinizations
    }

    /// Returns the worker count used.
    #[must_use]
    pub const fn workers(&self) -> u32 {
        self.workers
    }

    /// Returns total simulations across all trees.
    #[must_use]
    pub const fn simulations(&self) -> u64 {
        self.simulations
    }

    /// Returns total allocated nodes across all trees.
    #[must_use]
    pub const fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Returns maximum reached tree depth.
    #[must_use]
    pub const fn maximum_depth(&self) -> u32 {
        self.maximum_depth
    }

    /// Returns transposition hits. T4.4 reports zero until T4.5 adds the table.
    #[must_use]
    pub const fn transposition_hits(&self) -> u64 {
        self.transposition_hits
    }

    /// Returns measured decision wall time in microseconds.
    #[must_use]
    pub const fn actual_wall_time_us(&self) -> u64 {
        self.actual_wall_time_us
    }

    /// Returns measured process CPU time when the platform adapter supplies it.
    #[must_use]
    pub const fn actual_cpu_time_us(&self) -> Option<u64> {
        self.actual_cpu_time_us
    }

    /// Returns measured resident-memory delta when available.
    #[must_use]
    pub const fn memory_delta_bytes(&self) -> Option<i64> {
        self.memory_delta_bytes
    }

    /// Returns the leading mean-value gap.
    #[must_use]
    pub const fn value_gap(&self) -> i64 {
        self.value_gap
    }

    /// Returns the leading visit gap.
    #[must_use]
    pub const fn visit_gap(&self) -> u64 {
        self.visit_gap
    }

    /// Returns normalized visit entropy in parts per million.
    #[must_use]
    pub const fn uncertainty_ppm(&self) -> u32 {
        self.uncertainty_ppm
    }

    /// Returns the aggregate stop reason.
    #[must_use]
    pub const fn stop_reason(&self) -> SearchStopReason {
        self.stop_reason
    }

    /// Returns all per-determinization fixed-visit checkpoints.
    #[must_use]
    pub fn checkpoints(&self) -> &[SearchCheckpoint] {
        &self.checkpoints
    }
}

/// Root-parallel UCT implementation.
pub struct SearchEngine;

impl SearchEngine {
    /// Searches one canonical decision over independent determinizations.
    pub fn search<D: SearchDomain>(
        domain: &D,
        context: &DecisionContext,
        config: &SearchConfig,
    ) -> Result<SearchReport, SearchError> {
        validate_config(config)?;
        let started = Instant::now();
        let legal = context
            .options()
            .iter()
            .map(|option| option.id())
            .collect::<Vec<_>>();
        if legal.len() == 1 {
            return Ok(SearchReport {
                selected_action: legal[0],
                actions: vec![SearchActionReport {
                    action: legal[0],
                    visits: 0,
                    total_value: 0,
                    mean_value: 0,
                }],
                configured_limit: config.limit,
                determinizations: 0,
                workers: 0,
                simulations: 0,
                nodes: 0,
                maximum_depth: 0,
                transposition_hits: 0,
                actual_wall_time_us: elapsed_us(started),
                actual_cpu_time_us: None,
                memory_delta_bytes: None,
                value_gap: 0,
                visit_gap: 0,
                uncertainty_ppm: 0,
                stop_reason: SearchStopReason::SingletonLegalAction,
                checkpoints: Vec::new(),
            });
        }

        let worker_count = config.workers.min(config.determinizations).max(1);
        let batches = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(worker_count as usize);
            for worker in 0..worker_count {
                let legal = &legal;
                handles.push(scope.spawn(move || {
                    let mut results = Vec::new();
                    for index in (worker..config.determinizations).step_by(worker_count as usize) {
                        let seed = tree_seed(config.seed, index);
                        results.push((index, run_tree(domain, legal, seed, config)));
                    }
                    results
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().map_err(|_| SearchError::WorkerPanicked))
                .collect::<Result<Vec<_>, _>>()
        })?;
        let mut trees = Vec::<Option<TreeReport>>::new();
        trees.resize_with(config.determinizations as usize, || None);
        for batch in batches {
            for (index, result) in batch {
                trees[index as usize] =
                    Some(result.map_err(|message| SearchError::DomainFailure {
                        determinization: index,
                        message,
                    })?);
            }
        }
        let trees = trees
            .into_iter()
            .enumerate()
            .map(|(index, tree)| tree.ok_or(SearchError::MissingTree(index as u32)))
            .collect::<Result<Vec<_>, _>>()?;
        aggregate(legal, trees, config, worker_count, elapsed_us(started))
    }
}

struct Node<S> {
    state: S,
    terminal: Option<i64>,
    actions: Vec<CanonicalActionId>,
    expansion_order: Vec<usize>,
    children: Vec<Option<usize>>,
    visits: u32,
    total_value: i128,
    depth: u32,
}

#[derive(Clone, Copy)]
struct TreeActionReport {
    action: CanonicalActionId,
    visits: u32,
    total_value: i128,
}

struct TreeReport {
    actions: Vec<TreeActionReport>,
    fallback_action: CanonicalActionId,
    simulations: u32,
    nodes: u32,
    maximum_depth: u32,
    transposition_hits: u64,
    stop_reason: SearchStopReason,
    checkpoints: Vec<SearchCheckpoint>,
}

fn run_tree<D: SearchDomain>(
    domain: &D,
    expected_legal: &[CanonicalActionId],
    seed: u64,
    config: &SearchConfig,
) -> Result<TreeReport, String> {
    let started = Instant::now();
    let deadline = match config.limit {
        SearchLimit::WallTime(duration) => started.checked_add(duration),
        SearchLimit::Iterations(_) => None,
    };
    let root = domain.determinize(seed)?;
    let root_actions = canonical_actions(domain.legal_actions(&root)?)?;
    if root_actions != expected_legal {
        return Err("determinized root legal-action set differs from DecisionContext".to_owned());
    }
    if let Some(solution) = domain.bounded_solution(&root, &root_actions) {
        let (selected, stop_reason) = match solution {
            BoundedSolution::Win(action) => (action, SearchStopReason::CertifiedWin),
            BoundedSolution::RequiredDefense(action) => {
                (action, SearchStopReason::CertifiedRequiredDefense)
            }
        };
        if !root_actions.contains(&selected) {
            return Err("bounded solver returned an action outside the legal set".to_owned());
        }
        return Ok(TreeReport {
            actions: root_actions
                .into_iter()
                .map(|action| TreeActionReport {
                    action,
                    visits: u32::from(action == selected),
                    total_value: if action == selected {
                        i128::from(VALUE_LIMIT)
                    } else {
                        0
                    },
                })
                .collect(),
            fallback_action: selected,
            simulations: 0,
            nodes: 1,
            maximum_depth: 0,
            transposition_hits: 0,
            stop_reason,
            checkpoints: Vec::new(),
        });
    }

    let mut arena = vec![new_node(
        domain,
        root,
        0,
        config,
        deadline,
        Some(root_actions),
    )?];
    let mut transpositions = HashMap::new();
    if let Some(key) = domain.state_key(&arena[0].state) {
        transpositions.insert(key, 0_usize);
    }
    let mut transposition_hits = 0_u64;
    let mut simulations = 0_u32;
    let mut checkpoints = Vec::new();
    let mut prior_ranking = Vec::new();
    let mut stable_count = 0_u32;
    let mut stop_reason = match config.limit {
        SearchLimit::Iterations(_) => SearchStopReason::FixedIterations,
        SearchLimit::WallTime(_) => SearchStopReason::WallTimeBudget,
    };
    loop {
        let limit_reached = match config.limit {
            SearchLimit::Iterations(iterations) => simulations >= iterations,
            SearchLimit::WallTime(duration) => started.elapsed() >= duration,
        };
        if limit_reached {
            break;
        }
        simulate(
            domain,
            &mut arena,
            &mut transpositions,
            &mut transposition_hits,
            seed ^ u64::from(simulations),
            config,
            deadline,
        )?;
        simulations = simulations.saturating_add(1);
        if config.adaptive.enabled && config.adaptive.checkpoints.contains(&simulations) {
            let stats = tree_root_stats(&arena);
            let ranking = stats.iter().map(|item| item.action).collect::<Vec<_>>();
            let stable = ranking == prior_ranking && !prior_ranking.is_empty();
            stable_count = if stable {
                stable_count.saturating_add(1)
            } else {
                0
            };
            prior_ranking = ranking;
            let summary = summarize_tree_stats(&stats);
            let adaptive_stop = stable_count >= config.adaptive.stable_checkpoints
                && summary.leader_share_ppm >= config.adaptive.minimum_leader_share_ppm
                && summary.visit_gap >= config.adaptive.minimum_visit_gap
                && summary.value_gap >= config.adaptive.minimum_value_gap
                && summary.uncertainty_ppm <= config.adaptive.maximum_entropy_ppm;
            let checkpoint_stop = adaptive_stop.then_some(SearchStopReason::AdaptiveStableLeader);
            checkpoints.push(SearchCheckpoint {
                determinization: 0,
                simulations,
                leading_action: summary.leader,
                leading_visit_share_ppm: summary.leader_share_ppm,
                value_gap: summary.value_gap,
                visit_gap: summary.visit_gap,
                ranking_stable: stable,
                uncertainty_ppm: summary.uncertainty_ppm,
                bounded_solver_state: "not_certified",
                stop_reason: checkpoint_stop,
            });
            if adaptive_stop {
                stop_reason = SearchStopReason::AdaptiveStableLeader;
                break;
            }
        }
    }
    let fallback_action = arena[0]
        .expansion_order
        .first()
        .map_or(arena[0].actions[0], |index| arena[0].actions[*index]);
    let actions = arena[0]
        .actions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, action)| {
            arena[0].children[index].map_or(
                TreeActionReport {
                    action,
                    visits: 0,
                    total_value: 0,
                },
                |child| TreeActionReport {
                    action,
                    visits: arena[child].visits,
                    total_value: arena[child].total_value,
                },
            )
        })
        .collect();
    let maximum_depth = arena.iter().map(|node| node.depth).max().unwrap_or(0);
    Ok(TreeReport {
        actions,
        fallback_action,
        simulations,
        nodes: arena.len() as u32,
        maximum_depth,
        transposition_hits,
        stop_reason,
        checkpoints,
    })
}

fn new_node<D: SearchDomain>(
    domain: &D,
    state: D::State,
    depth: u32,
    config: &SearchConfig,
    deadline: Option<Instant>,
    known_actions: Option<Vec<CanonicalActionId>>,
) -> Result<Node<D::State>, String> {
    let terminal = domain.terminal_value(&state).map(clamp_value);
    let actions = if terminal.is_some() {
        Vec::new()
    } else if let Some(actions) = known_actions {
        actions
    } else {
        canonical_actions(domain.legal_actions(&state)?)?
    };
    let children = vec![None; actions.len()];
    let mut prior_order = (0..actions.len())
        .map(|index| (domain.action_prior(&state, actions[index]), index))
        .collect::<Vec<_>>();
    prior_order.sort_by(|(left_prior, left), (right_prior, right)| {
        right_prior
            .cmp(left_prior)
            .then_with(|| actions[*left].cmp(&actions[*right]))
    });
    let use_action_groups = !deadline.is_some_and(|deadline| Instant::now() >= deadline)
        && config
            .progressive_widening
            .is_some_and(|schedule| actions.len() > schedule.initial_actions as usize);
    let mut group_depths = HashMap::<u64, u32>::new();
    let mut expansion_order = prior_order
        .into_iter()
        .enumerate()
        .map(|(prior_rank, (_, index))| {
            let group = if use_action_groups {
                domain.action_group(&state, actions[index])
            } else {
                index as u64
            };
            let group_depth = group_depths.entry(group).or_default();
            let result = (*group_depth, prior_rank, index);
            *group_depth = group_depth.saturating_add(1);
            result
        })
        .collect::<Vec<_>>();
    expansion_order.sort_by_key(|(group_depth, prior_rank, _)| (*group_depth, *prior_rank));
    let expansion_order = expansion_order
        .into_iter()
        .map(|(_, _, index)| index)
        .collect();
    Ok(Node {
        state,
        terminal,
        actions,
        expansion_order,
        children,
        visits: 0,
        total_value: 0,
        depth,
    })
}

fn simulate<D: SearchDomain>(
    domain: &D,
    arena: &mut Vec<Node<D::State>>,
    transpositions: &mut HashMap<u64, usize>,
    transposition_hits: &mut u64,
    seed: u64,
    config: &SearchConfig,
    deadline: Option<Instant>,
) -> Result<(), String> {
    let mut path = Vec::new();
    let mut current = 0_usize;
    let value = loop {
        path.push(current);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break clamp_value(domain.evaluate(&arena[current].state));
        }
        if let Some(value) = arena[current].terminal {
            break value;
        }
        if arena[current].actions.is_empty() {
            break clamp_value(domain.evaluate(&arena[current].state));
        }
        if let Some(action_index) = best_unexpanded(&arena[current], config) {
            let action = arena[current].actions[action_index];
            let next = domain.apply_action(&arena[current].state, action)?;
            let key = domain.state_key(&next);
            let child = key.and_then(|key| transpositions.get(&key).copied());
            let child = if let Some(child) = child {
                *transposition_hits = transposition_hits.saturating_add(1);
                child
            } else {
                let child = arena.len();
                let depth = arena[current].depth.saturating_add(1);
                arena.push(new_node(domain, next, depth, config, deadline, None)?);
                if let Some(key) = key {
                    transpositions.insert(key, child);
                }
                child
            };
            arena[current].children[action_index] = Some(child);
            if path.contains(&child) {
                break clamp_value(domain.evaluate(&arena[child].state));
            }
            path.push(child);
            break rollout(
                domain,
                &arena[child].state,
                seed,
                config.rollout_depth,
                deadline,
            )?;
        }
        let child = select_uct_child(domain, &arena[current], arena, config)?;
        current = child;
    };
    for node in path {
        arena[node].visits = arena[node].visits.saturating_add(1);
        arena[node].total_value = arena[node].total_value.saturating_add(i128::from(value));
    }
    Ok(())
}

fn best_unexpanded<S>(node: &Node<S>, config: &SearchConfig) -> Option<usize> {
    let active = config
        .progressive_widening
        .map_or(node.actions.len(), |schedule| {
            let widening = u64::from(schedule.actions_per_sqrt_visit)
                .saturating_mul(u64::from(integer_sqrt(node.visits)));
            usize::try_from(u64::from(schedule.initial_actions).saturating_add(widening))
                .unwrap_or(usize::MAX)
                .min(node.actions.len())
        });
    node.expansion_order
        .iter()
        .copied()
        .take(active)
        .find(|index| node.children[*index].is_none())
}

const fn integer_sqrt(value: u32) -> u32 {
    if value < 2 {
        return value;
    }
    let mut low = 1_u32;
    let capped = if value > 65_535 { 65_535 } else { value };
    let mut high = capped.saturating_add(1);
    while low.saturating_add(1) < high {
        let middle = low + (high - low) / 2;
        if middle <= value / middle {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

fn select_uct_child<D: SearchDomain>(
    domain: &D,
    node: &Node<D::State>,
    arena: &[Node<D::State>],
    config: &SearchConfig,
) -> Result<usize, String> {
    let sign = f64::from(domain.selection_sign(&node.state).clamp(-1, 1));
    let parent_log = f64::from(node.visits.max(1)).ln();
    let exploration = f64::from(config.exploration_milli) / 1_000.0;
    node.children
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, child)| child.map(|child| (index, child)))
        .max_by(|(left_index, left), (right_index, right)| {
            let score = |index: usize, child: usize| {
                let visits = arena[child].visits.max(1);
                let mean = arena[child].total_value as f64 / f64::from(visits);
                let explore = exploration * (parent_log / f64::from(visits)).sqrt();
                let prior = domain.action_prior(&node.state, node.actions[index]) as f64 / 1_000.0;
                sign * mean + explore + prior
            };
            score(*left_index, *left)
                .total_cmp(&score(*right_index, *right))
                .then_with(|| node.actions[*right_index].cmp(&node.actions[*left_index]))
        })
        .map(|(_, child)| child)
        .ok_or_else(|| "fully expanded node has no child".to_owned())
}

fn rollout<D: SearchDomain>(
    domain: &D,
    start: &D::State,
    seed: u64,
    depth: u32,
    deadline: Option<Instant>,
) -> Result<i64, String> {
    let mut state = start.clone();
    for ply in 0..depth {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        if let Some(value) = domain.terminal_value(&state) {
            return Ok(clamp_value(value));
        }
        let actions = canonical_actions(domain.legal_actions(&state)?)?;
        if actions.is_empty() {
            break;
        }
        let action = domain.rollout_action(&state, &actions, splitmix64(seed ^ u64::from(ply)))?;
        if !actions.contains(&action) {
            return Err("rollout policy returned an action outside the legal set".to_owned());
        }
        state = domain.apply_action(&state, action)?;
    }
    Ok(clamp_value(domain.evaluate(&state)))
}

fn canonical_actions(
    mut actions: Vec<CanonicalActionId>,
) -> Result<Vec<CanonicalActionId>, String> {
    actions.sort_unstable();
    let unique = actions.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != actions.len() {
        return Err("domain emitted duplicate canonical actions".to_owned());
    }
    Ok(actions)
}

fn tree_root_stats<S>(arena: &[Node<S>]) -> Vec<TreeActionReport> {
    let root = &arena[0];
    let mut stats = root
        .actions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, action)| {
            root.children[index].map_or(
                TreeActionReport {
                    action,
                    visits: 0,
                    total_value: 0,
                },
                |child| TreeActionReport {
                    action,
                    visits: arena[child].visits,
                    total_value: arena[child].total_value,
                },
            )
        })
        .collect::<Vec<_>>();
    stats.sort_by(|left, right| {
        right
            .visits
            .cmp(&left.visits)
            .then_with(|| mean_value(*right).cmp(&mean_value(*left)))
            .then_with(|| left.action.cmp(&right.action))
    });
    stats
}

struct StatsSummary {
    leader: CanonicalActionId,
    leader_share_ppm: u32,
    value_gap: i64,
    visit_gap: u32,
    uncertainty_ppm: u32,
}

fn summarize_tree_stats(stats: &[TreeActionReport]) -> StatsSummary {
    let first = stats[0];
    let second = stats.get(1).copied().unwrap_or(TreeActionReport {
        action: first.action,
        visits: 0,
        total_value: 0,
    });
    let total = stats.iter().map(|item| u64::from(item.visits)).sum::<u64>();
    StatsSummary {
        leader: first.action,
        leader_share_ppm: share_ppm(u64::from(first.visits), total),
        value_gap: mean_value(first).saturating_sub(mean_value(second)),
        visit_gap: first.visits.saturating_sub(second.visits),
        uncertainty_ppm: entropy_ppm(
            &stats
                .iter()
                .map(|item| u64::from(item.visits))
                .collect::<Vec<_>>(),
        ),
    }
}

fn aggregate(
    legal: Vec<CanonicalActionId>,
    trees: Vec<TreeReport>,
    config: &SearchConfig,
    workers: u32,
    actual_wall_time_us: u64,
) -> Result<SearchReport, SearchError> {
    let mut actions = legal
        .into_iter()
        .map(|action| SearchActionReport {
            action,
            visits: 0,
            total_value: 0,
            mean_value: 0,
        })
        .collect::<Vec<_>>();
    let mut simulations = 0_u64;
    let mut nodes = 0_u64;
    let mut maximum_depth = 0_u32;
    let mut transposition_hits = 0_u64;
    let mut checkpoints = Vec::new();
    let mut stop_reasons = BTreeSet::new();
    let mut fallback_votes = HashMap::<CanonicalActionId, u32>::new();
    for (index, tree) in trees.into_iter().enumerate() {
        simulations = simulations.saturating_add(u64::from(tree.simulations));
        nodes = nodes.saturating_add(u64::from(tree.nodes));
        maximum_depth = maximum_depth.max(tree.maximum_depth);
        transposition_hits = transposition_hits.saturating_add(tree.transposition_hits);
        stop_reasons.insert(tree.stop_reason);
        let votes = fallback_votes.entry(tree.fallback_action).or_default();
        *votes = votes.saturating_add(1);
        for report in tree.actions {
            let target = actions
                .iter_mut()
                .find(|candidate| candidate.action == report.action)
                .ok_or(SearchError::AggregateActionMismatch(report.action))?;
            target.visits = target.visits.saturating_add(u64::from(report.visits));
            target.total_value = target.total_value.saturating_add(report.total_value);
        }
        checkpoints.extend(tree.checkpoints.into_iter().map(|mut checkpoint| {
            checkpoint.determinization = index as u32;
            checkpoint
        }));
    }
    for action in &mut actions {
        action.mean_value = if action.visits == 0 {
            0
        } else {
            clamp_i128(action.total_value / i128::from(action.visits))
        };
    }
    let mut ranked = actions.clone();
    ranked.sort_by(|left, right| {
        right
            .visits
            .cmp(&left.visits)
            .then_with(|| right.mean_value.cmp(&left.mean_value))
            .then_with(|| left.action.cmp(&right.action))
    });
    let first = if simulations == 0 {
        let fallback = fallback_votes
            .into_iter()
            .max_by(|(left_action, left_votes), (right_action, right_votes)| {
                left_votes
                    .cmp(right_votes)
                    .then_with(|| right_action.cmp(left_action))
            })
            .map(|(action, _)| action)
            .ok_or(SearchError::MissingFallbackAction)?;
        actions
            .iter()
            .copied()
            .find(|report| report.action == fallback)
            .ok_or(SearchError::AggregateActionMismatch(fallback))?
    } else {
        ranked[0]
    };
    let second = ranked
        .iter()
        .copied()
        .find(|candidate| candidate.action != first.action)
        .unwrap_or(first);
    let stop_reason = if stop_reasons.len() == 1 {
        stop_reasons
            .iter()
            .next()
            .copied()
            .unwrap_or(SearchStopReason::Mixed)
    } else {
        SearchStopReason::Mixed
    };
    let visit_counts = actions.iter().map(|item| item.visits).collect::<Vec<_>>();
    Ok(SearchReport {
        selected_action: first.action,
        actions,
        configured_limit: config.limit,
        determinizations: config.determinizations,
        workers,
        simulations,
        nodes,
        maximum_depth,
        transposition_hits,
        actual_wall_time_us,
        actual_cpu_time_us: None,
        memory_delta_bytes: None,
        value_gap: first.mean_value.saturating_sub(second.mean_value),
        visit_gap: first.visits.saturating_sub(second.visits),
        uncertainty_ppm: entropy_ppm(&visit_counts),
        stop_reason,
        checkpoints,
    })
}

fn mean_value(report: TreeActionReport) -> i64 {
    if report.visits == 0 {
        0
    } else {
        clamp_i128(report.total_value / i128::from(report.visits))
    }
}

fn validate_config(config: &SearchConfig) -> Result<(), SearchError> {
    if config.determinizations == 0 {
        return Err(SearchError::InvalidConfig(
            "determinizations must be positive".to_owned(),
        ));
    }
    if config.workers == 0 {
        return Err(SearchError::InvalidConfig(
            "workers must be positive".to_owned(),
        ));
    }
    if config.rollout_depth == 0 {
        return Err(SearchError::InvalidConfig(
            "rollout depth must be positive".to_owned(),
        ));
    }
    if config.progressive_widening.is_some_and(|schedule| {
        schedule.initial_actions == 0 || schedule.actions_per_sqrt_visit == 0
    }) {
        return Err(SearchError::InvalidConfig(
            "progressive widening requires positive schedule values".to_owned(),
        ));
    }
    if config.adaptive.enabled {
        let checkpoints_are_strict = config
            .adaptive
            .checkpoints
            .windows(2)
            .all(|pair| pair[0] < pair[1]);
        if config.adaptive.checkpoints.is_empty()
            || config.adaptive.checkpoints[0] == 0
            || !checkpoints_are_strict
        {
            return Err(SearchError::InvalidConfig(
                "adaptive checkpoints must be nonempty, positive, and strictly increasing"
                    .to_owned(),
            ));
        }
        if config.adaptive.stable_checkpoints == 0
            || config.adaptive.stable_checkpoints as usize >= config.adaptive.checkpoints.len()
        {
            return Err(SearchError::InvalidConfig(
                "adaptive stable-checkpoint count must be positive and leave an earlier comparison checkpoint"
                    .to_owned(),
            ));
        }
        if config.adaptive.minimum_leader_share_ppm > 1_000_000
            || config.adaptive.maximum_entropy_ppm > 1_000_000
        {
            return Err(SearchError::InvalidConfig(
                "adaptive share and entropy thresholds must be within 0..=1000000 ppm".to_owned(),
            ));
        }
        if config.adaptive.minimum_value_gap < 0 {
            return Err(SearchError::InvalidConfig(
                "adaptive minimum value gap must be nonnegative".to_owned(),
            ));
        }
    }
    match config.limit {
        SearchLimit::Iterations(0) => Err(SearchError::InvalidConfig(
            "fixed iterations must be positive".to_owned(),
        )),
        SearchLimit::WallTime(duration) if duration.is_zero() => Err(SearchError::InvalidConfig(
            "wall-time search budget must be positive".to_owned(),
        )),
        _ => Ok(()),
    }
}

fn share_ppm(value: u64, total: u64) -> u32 {
    if total == 0 {
        0
    } else {
        ((value as f64 / total as f64) * PPM).round() as u32
    }
}

fn entropy_ppm(visits: &[u64]) -> u32 {
    let total = visits.iter().sum::<u64>();
    let active = visits.iter().filter(|visits| **visits > 0).count();
    if total == 0 || active <= 1 {
        return 0;
    }
    let entropy = visits
        .iter()
        .filter(|visits| **visits > 0)
        .map(|visits| {
            let probability = *visits as f64 / total as f64;
            -probability * probability.ln()
        })
        .sum::<f64>();
    ((entropy / (active as f64).ln()) * PPM).round() as u32
}

const fn clamp_value(value: i64) -> i64 {
    if value > VALUE_LIMIT {
        VALUE_LIMIT
    } else if value < -VALUE_LIMIT {
        -VALUE_LIMIT
    } else {
        value
    }
}

fn clamp_i128(value: i128) -> i64 {
    i64::try_from(value).unwrap_or_else(|_| {
        if value.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

fn tree_seed(seed: u64, index: u32) -> u64 {
    splitmix64(seed ^ u64::from(index).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

/// Fail-closed search errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    /// Search configuration is invalid.
    InvalidConfig(String),
    /// One opaque determinization or transition failed.
    DomainFailure {
        /// Determinization ordinal.
        determinization: u32,
        /// Domain-supplied failure detail.
        message: String,
    },
    /// A worker thread panicked.
    WorkerPanicked,
    /// A worker omitted one determinization result.
    MissingTree(u32),
    /// A tree emitted a root action absent from the canonical context.
    AggregateActionMismatch(CanonicalActionId),
    /// No determinization supplied a prior-ordered timeout fallback.
    MissingFallbackAction,
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(formatter, "invalid search config: {message}"),
            Self::DomainFailure {
                determinization,
                message,
            } => write!(
                formatter,
                "determinization {determinization} failed: {message}"
            ),
            Self::WorkerPanicked => write!(formatter, "search worker panicked"),
            Self::MissingTree(index) => write!(formatter, "search worker omitted tree {index}"),
            Self::AggregateActionMismatch(action) => {
                write!(formatter, "tree returned unknown root action {action}")
            }
            Self::MissingFallbackAction => write!(formatter, "search produced no fallback action"),
        }
    }
}

impl Error for SearchError {}

#[cfg(test)]
mod tests {
    use super::{
        best_unexpanded, AdaptiveStopping, Node, ProgressiveWidening, SearchConfig, SearchDomain,
        SearchEngine, SearchError, SearchStopReason,
    };
    use crate::LastDecisionReport;
    use forge_core::{
        apply, Action, CanonicalActionId, DecisionContext, DecisionDescriptor, DecisionKind,
        DecisionOption, GameState, Outcome,
    };
    use std::{
        sync::atomic::{AtomicU32, Ordering},
        thread,
        time::Duration,
    };

    #[derive(Clone)]
    struct ToyState {
        depth: u8,
        value: i64,
    }

    struct ToyDomain {
        left: CanonicalActionId,
        right: CanonicalActionId,
        determinizations: AtomicU32,
        converge: bool,
        right_prior: i64,
        determinization_delay_ms: u64,
    }

    impl SearchDomain for ToyDomain {
        type State = ToyState;

        fn determinize(&self, _seed: u64) -> Result<Self::State, String> {
            self.determinizations.fetch_add(1, Ordering::Relaxed);
            if self.determinization_delay_ms > 0 {
                thread::sleep(Duration::from_millis(self.determinization_delay_ms));
            }
            Ok(ToyState { depth: 0, value: 0 })
        }

        fn legal_actions(&self, state: &Self::State) -> Result<Vec<CanonicalActionId>, String> {
            Ok(if state.depth == 0 {
                vec![self.left, self.right]
            } else {
                Vec::new()
            })
        }

        fn apply_action(
            &self,
            state: &Self::State,
            action: CanonicalActionId,
        ) -> Result<Self::State, String> {
            if state.depth != 0 {
                return Err("toy terminal state cannot transition".to_owned());
            }
            Ok(ToyState {
                depth: 1,
                value: if self.converge || action == self.left {
                    100
                } else {
                    -100
                },
            })
        }

        fn terminal_value(&self, state: &Self::State) -> Option<i64> {
            (state.depth == 1).then_some(state.value)
        }

        fn evaluate(&self, state: &Self::State) -> i64 {
            state.value
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
                .ok_or_else(|| "no action".to_owned())
        }

        fn state_key(&self, state: &Self::State) -> Option<u64> {
            Some((u64::from(state.depth) << 56) ^ (state.value as u64))
        }

        fn action_prior(&self, _state: &Self::State, action: CanonicalActionId) -> i64 {
            if action == self.right {
                self.right_prior
            } else {
                0
            }
        }
    }

    fn context(option_count: usize) -> (DecisionContext, Vec<CanonicalActionId>) {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player result: {other:?}"),
        };
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let options = (0..option_count)
            .map(|value| {
                DecisionOption::new(
                    DecisionDescriptor::ChooseNumber {
                        value: value as u32,
                    },
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        let context = DecisionContext::new(
            DecisionKind::NumericValue,
            player,
            &view,
            options,
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("context failed: {error}"));
        let ids = context.options().iter().map(DecisionOption::id).collect();
        (context, ids)
    }

    #[test]
    fn singleton_actions_bypass_determinization_and_search() {
        let (context, ids) = context(1);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[0],
            determinizations: AtomicU32::new(0),
            converge: false,
            right_prior: 0,
            determinization_delay_ms: 0,
        };
        let report =
            SearchEngine::search(&domain, &context, &SearchConfig::fixed_iterations(7, 4, 32))
                .unwrap_or_else(|error| panic!("search failed: {error}"));
        assert_eq!(report.selected_action(), ids[0]);
        assert_eq!(report.stop_reason(), SearchStopReason::SingletonLegalAction);
        assert_eq!(domain.determinizations.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn root_parallel_visit_sum_selects_the_winning_action_replayably() {
        let (context, ids) = context(2);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[1],
            determinizations: AtomicU32::new(0),
            converge: false,
            right_prior: 0,
            determinization_delay_ms: 0,
        };
        let config = SearchConfig::fixed_iterations(11, 4, 64).with_workers(4);
        let first = SearchEngine::search(&domain, &context, &config)
            .unwrap_or_else(|error| panic!("search failed: {error}"));
        let second = SearchEngine::search(&domain, &context, &config)
            .unwrap_or_else(|error| panic!("search failed: {error}"));
        assert_eq!(first.selected_action(), ids[0]);
        assert_eq!(first.actions(), second.actions());
        assert_eq!(first.simulations(), 256);
        assert_eq!(first.determinizations(), 4);
        assert_eq!(first.transposition_hits(), 0);
        let public_report = LastDecisionReport::from_search(&first);
        assert_eq!(public_report.selected_action(), ids[0]);
        assert_eq!(public_report.considered().len(), 2);
        assert_eq!(public_report.considered()[0].action(), ids[0]);
        assert_eq!(public_report.considered()[0].value_delta_from_selected(), 0);
    }

    #[test]
    fn state_keys_share_convergent_children_and_report_hits() {
        let (context, ids) = context(2);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[1],
            determinizations: AtomicU32::new(0),
            converge: true,
            right_prior: 0,
            determinization_delay_ms: 0,
        };
        let report =
            SearchEngine::search(&domain, &context, &SearchConfig::fixed_iterations(19, 1, 4))
                .unwrap_or_else(|error| panic!("search failed: {error}"));

        assert!(report.transposition_hits() >= 1);
        assert!(report.nodes() < report.simulations().saturating_add(1));
    }

    #[test]
    fn expired_wall_budget_uses_prior_ordered_fallback_without_a_simulation() {
        let (context, ids) = context(2);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[1],
            determinizations: AtomicU32::new(0),
            converge: false,
            right_prior: 100,
            determinization_delay_ms: 5,
        };
        let report = SearchEngine::search(
            &domain,
            &context,
            &SearchConfig::wall_time(23, 1, 1).with_workers(1),
        )
        .unwrap_or_else(|error| panic!("search failed: {error}"));

        assert_eq!(report.simulations(), 0);
        assert_eq!(report.selected_action(), ids[1]);
        assert_eq!(report.stop_reason(), SearchStopReason::WallTimeBudget);
    }

    #[test]
    fn progressive_widening_opens_prior_order_by_sqrt_visits() {
        let (_, ids) = context(8);
        let mut node = Node {
            state: (),
            terminal: None,
            actions: ids,
            expansion_order: (0..8).collect(),
            children: vec![None; 8],
            visits: 0,
            total_value: 0,
            depth: 0,
        };
        let config = SearchConfig::fixed_iterations(23, 1, 8)
            .with_progressive_widening(Some(ProgressiveWidening::new(2, 1)));

        assert_eq!(best_unexpanded(&node, &config), Some(0));
        node.children[0] = Some(1);
        assert_eq!(best_unexpanded(&node, &config), Some(1));
        node.children[1] = Some(2);
        assert_eq!(best_unexpanded(&node, &config), None);
        node.visits = 4;
        assert_eq!(best_unexpanded(&node, &config), Some(2));
    }

    #[test]
    fn adaptive_stopping_records_auditable_fixed_visit_checkpoints() {
        let (context, ids) = context(2);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[1],
            determinizations: AtomicU32::new(0),
            converge: false,
            right_prior: 0,
            determinization_delay_ms: 0,
        };
        let config = SearchConfig::fixed_iterations(29, 1, 128).with_adaptive_stopping(
            AdaptiveStopping::experimental(vec![16, 32, 64, 128], 1, 0, 0, 0, 1_000_000),
        );
        let report = SearchEngine::search(&domain, &context, &config)
            .unwrap_or_else(|error| panic!("adaptive search failed: {error}"));

        assert_eq!(report.stop_reason(), SearchStopReason::AdaptiveStableLeader);
        assert!(report.simulations() < 128);
        assert!(report.checkpoints().len() >= 2);
        for checkpoint in report.checkpoints() {
            assert_eq!(checkpoint.determinization(), 0);
            assert!(matches!(checkpoint.simulations(), 16 | 32 | 64 | 128));
            assert!(context.select(checkpoint.leading_action()).is_ok());
            assert!(checkpoint.leading_visit_share_ppm() <= 1_000_000);
            assert!(checkpoint.uncertainty_ppm() <= 1_000_000);
            assert_eq!(checkpoint.bounded_solver_state(), "not_certified");
        }
        let final_checkpoint = report
            .checkpoints()
            .last()
            .unwrap_or_else(|| panic!("adaptive stop should retain its final checkpoint"));
        assert!(final_checkpoint.ranking_stable());
        assert_eq!(
            final_checkpoint.stop_reason(),
            Some(SearchStopReason::AdaptiveStableLeader)
        );
    }

    #[test]
    fn adaptive_stopping_rejects_invalid_threshold_documents() {
        let (context, ids) = context(2);
        let domain = ToyDomain {
            left: ids[0],
            right: ids[1],
            determinizations: AtomicU32::new(0),
            converge: false,
            right_prior: 0,
            determinization_delay_ms: 0,
        };
        let invalid = [
            AdaptiveStopping::experimental(Vec::new(), 1, 0, 0, 0, 1_000_000),
            AdaptiveStopping::experimental(vec![16, 16], 1, 0, 0, 0, 1_000_000),
            AdaptiveStopping::experimental(vec![16, 32], 0, 0, 0, 0, 1_000_000),
            AdaptiveStopping::experimental(vec![16, 32], 1, 1_000_001, 0, 0, 1_000_000),
            AdaptiveStopping::experimental(vec![16, 32], 1, 0, 0, -1, 1_000_000),
        ];
        for adaptive in invalid {
            let error = match SearchEngine::search(
                &domain,
                &context,
                &SearchConfig::fixed_iterations(31, 1, 64).with_adaptive_stopping(adaptive),
            ) {
                Err(error) => error,
                Ok(_) => panic!("invalid adaptive thresholds must fail closed"),
            };
            assert!(matches!(error, SearchError::InvalidConfig(_)));
        }
    }
}
