use crate::{AiWeights, Evaluation, EvaluationError};
use forge_core::{CanonicalActionId, PlayerId, PlayerView};
use std::{collections::BTreeSet, error::Error, fmt};

/// Baseline policy role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyMode {
    /// Noisy one-ply policy used by the Novice tier.
    Novice,
    /// Deterministic one-ply policy used for rollout and move ordering.
    Rollout,
}

/// One canonical legal action paired with its determinized successor view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyCandidate {
    action_id: CanonicalActionId,
    resulting_view: PlayerView,
    prior: i64,
}

impl PolicyCandidate {
    /// Creates a candidate. `prior` is a card-agnostic tactical ordering score.
    #[must_use]
    pub fn new(action_id: CanonicalActionId, resulting_view: PlayerView, prior: i64) -> Self {
        Self {
            action_id,
            resulting_view,
            prior,
        }
    }

    /// Returns the canonical legal-action ID.
    #[must_use]
    pub const fn action_id(&self) -> CanonicalActionId {
        self.action_id
    }

    /// Returns the determinized successor projection.
    #[must_use]
    pub const fn resulting_view(&self) -> &PlayerView {
        &self.resulting_view
    }

    /// Returns the tactical prior supplied by the action generator.
    #[must_use]
    pub const fn prior(&self) -> i64 {
        self.prior
    }
}

/// One inspectable heuristic decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    index: usize,
    evaluation: Evaluation,
    prior: i64,
    noise: i64,
    score: i64,
}

impl PolicyDecision {
    /// Returns the selected candidate index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    /// Returns the successor-state evaluation.
    #[must_use]
    pub const fn evaluation(self) -> Evaluation {
        self.evaluation
    }

    /// Returns the action-generator prior.
    #[must_use]
    pub const fn prior(self) -> i64 {
        self.prior
    }

    /// Returns deterministic policy noise.
    #[must_use]
    pub const fn noise(self) -> i64 {
        self.noise
    }

    /// Returns evaluation plus prior and noise.
    #[must_use]
    pub const fn score(self) -> i64 {
        self.score
    }
}

/// One-ply greedy policy over redacted successor projections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeuristicPolicy {
    weights: AiWeights,
    mode: PolicyMode,
    seed: u64,
    noise_span: i64,
}

impl HeuristicPolicy {
    /// Creates a noisy Novice policy.
    #[must_use]
    pub const fn novice(weights: AiWeights, seed: u64, noise_span: i64) -> Self {
        Self {
            weights,
            mode: PolicyMode::Novice,
            seed,
            noise_span: if noise_span < 0 { 0 } else { noise_span },
        }
    }

    /// Creates a deterministic rollout and move-ordering policy.
    #[must_use]
    pub const fn rollout(weights: AiWeights, seed: u64) -> Self {
        Self {
            weights,
            mode: PolicyMode::Rollout,
            seed,
            noise_span: 0,
        }
    }

    /// Returns this policy's role.
    #[must_use]
    pub const fn mode(self) -> PolicyMode {
        self.mode
    }

    /// Returns the data-driven weights used by this policy.
    #[must_use]
    pub const fn weights(self) -> AiWeights {
        self.weights
    }

    /// Selects the highest-scoring candidate, preserving canonical order on ties.
    pub fn select(self, candidates: &[PolicyCandidate]) -> Result<PolicyDecision, PolicyError> {
        let Some(first) = candidates.first() else {
            return Err(PolicyError::NoCandidates);
        };
        let observer = first.resulting_view.observer();
        let mut seen = BTreeSet::new();
        seen.insert(first.action_id());
        let mut best = self.score_candidate(0, first)?;
        for (index, candidate) in candidates.iter().enumerate().skip(1) {
            if !seen.insert(candidate.action_id()) {
                return Err(PolicyError::DuplicateActionId(candidate.action_id()));
            }
            let actual = candidate.resulting_view.observer();
            if actual != observer {
                return Err(PolicyError::ObserverMismatch {
                    index,
                    expected: observer,
                    actual,
                });
            }
            let scored = self.score_candidate(index, candidate)?;
            if scored.score > best.score
                || (scored.score == best.score
                    && candidate.action_id() < candidates[best.index].action_id())
            {
                best = scored;
            }
        }
        Ok(best)
    }

    fn score_candidate(
        self,
        index: usize,
        candidate: &PolicyCandidate,
    ) -> Result<PolicyDecision, PolicyError> {
        let evaluation = self
            .weights
            .evaluate(&candidate.resulting_view)
            .map_err(PolicyError::Evaluation)?;
        let noise = self.noise(
            candidate.action_id(),
            candidate.resulting_view.turn_number(),
            candidate.resulting_view.observer(),
        );
        let score = evaluation
            .total()
            .saturating_add(candidate.prior)
            .saturating_add(noise);
        Ok(PolicyDecision {
            index,
            evaluation,
            prior: candidate.prior,
            noise,
            score,
        })
    }

    fn noise(self, action_id: CanonicalActionId, turn: u32, observer: PlayerId) -> i64 {
        let span = self.noise_span.min(1_000_000_000);
        if span == 0 {
            return 0;
        }
        let id = action_id.get();
        let mixed = splitmix64(
            self.seed
                ^ id as u64
                ^ ((id >> 64) as u64).rotate_left(23)
                ^ u64::from(turn)
                ^ (observer.index() as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9),
        );
        let width = (span as u64).saturating_mul(2).saturating_add(1);
        (mixed % width) as i64 - span
    }
}

/// Fail-closed heuristic-policy errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    /// No legal candidate was supplied.
    NoCandidates,
    /// The action generator supplied the same canonical action more than once.
    DuplicateActionId(CanonicalActionId),
    /// Candidate views disagree about the acting observer.
    ObserverMismatch {
        /// Candidate index.
        index: usize,
        /// Observer established by the first candidate.
        expected: PlayerId,
        /// Observer found on this candidate.
        actual: PlayerId,
    },
    /// Evaluation failed.
    Evaluation(EvaluationError),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCandidates => write!(formatter, "heuristic policy received no legal actions"),
            Self::DuplicateActionId(id) => {
                write!(formatter, "heuristic policy received duplicate action {id}")
            }
            Self::ObserverMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "candidate {index} uses seat {}, expected seat {}",
                actual.index() + 1,
                expected.index() + 1
            ),
            Self::Evaluation(error) => write!(formatter, "candidate evaluation failed: {error}"),
        }
    }
}

impl Error for PolicyError {}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::{HeuristicPolicy, PolicyCandidate};
    use crate::AiWeights;
    use forge_core::{apply, Action, DecisionDescriptor, DecisionOption, GameState, Outcome};

    fn add_player(state: &mut GameState) -> forge_core::PlayerId {
        match apply(state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected AddPlayer outcome: {other:?}"),
        }
    }

    #[test]
    fn rollout_policy_chooses_the_better_one_ply_view() {
        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights failed: {error}"));
        let mut state = GameState::new();
        let observer = add_player(&mut state);
        let _opponent = add_player(&mut state);

        let gain = Action::GainLife {
            player: observer,
            amount: 3,
        };
        let mut gain_state = state.clone();
        assert_eq!(apply(&mut gain_state, gain.clone()), Outcome::Applied);
        let gain_view = gain_state
            .player_view(observer)
            .unwrap_or_else(|error| panic!("gain view failed: {error:?}"));

        let lose = Action::LoseLife {
            player: observer,
            amount: 3,
        };
        let mut lose_state = state;
        assert_eq!(apply(&mut lose_state, lose.clone()), Outcome::Applied);
        let lose_view = lose_state
            .player_view(observer)
            .unwrap_or_else(|error| panic!("lose view failed: {error:?}"));

        let candidates = vec![
            PolicyCandidate::new(
                DecisionOption::new(DecisionDescriptor::ChooseNumber { value: 0 }, vec![lose]).id(),
                lose_view,
                0,
            ),
            PolicyCandidate::new(
                DecisionOption::new(DecisionDescriptor::ChooseNumber { value: 1 }, vec![gain]).id(),
                gain_view,
                0,
            ),
        ];
        let decision = HeuristicPolicy::rollout(weights, 5)
            .select(&candidates)
            .unwrap_or_else(|error| panic!("policy failed: {error}"));
        assert_eq!(decision.index(), 1);
        assert!(decision.score() > candidates[0].prior());
    }

    #[test]
    fn novice_noise_is_seeded_and_replayable() {
        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights failed: {error}"));
        let mut state = GameState::new();
        let observer = add_player(&mut state);
        let _opponent = add_player(&mut state);
        let action = Action::GainLife {
            player: observer,
            amount: 1,
        };
        let view = state
            .player_view(observer)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let action_id =
            DecisionOption::new(DecisionDescriptor::ChooseNumber { value: 0 }, vec![action]).id();
        let candidates = vec![PolicyCandidate::new(action_id, view, 0)];
        let policy = HeuristicPolicy::novice(weights, 44, 200);
        assert_eq!(
            policy
                .select(&candidates)
                .unwrap_or_else(|error| panic!("first decision failed: {error}")),
            policy
                .select(&candidates)
                .unwrap_or_else(|error| panic!("second decision failed: {error}"))
        );
    }
}
