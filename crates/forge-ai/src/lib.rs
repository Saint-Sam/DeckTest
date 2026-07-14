#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Hidden-information-safe AI primitives for Forge 2.0.
//!
//! Production APIs consume [`forge_core::PlayerView`] projections and explicit
//! deck models. They never accept a full [`forge_core::GameState`].

mod api;
mod determinization;
mod evaluation;
mod guardrail;
mod mulligan;
mod policy;
mod random;
mod search;
mod target;
mod tier;

pub use api::{ConsideredAction, LastDecisionReport};
pub use determinization::{
    DeckModel, Determinization, DeterminizationError, Determinizer, HiddenAssignment,
};
pub use evaluation::{AiWeights, Evaluation, EvaluationError, FeatureVector};
pub use guardrail::{ActionRisk, ActionRisks, GuardrailError, GuardrailProfile, GuardrailTable};
pub use mulligan::{MulliganDecision, MulliganError, MulliganPolicy};
pub use policy::{HeuristicPolicy, PolicyCandidate, PolicyDecision, PolicyError, PolicyMode};
pub use random::{RandomLegalPolicy, RandomPolicyError};
pub use search::{
    AdaptiveStopping, BoundedSolution, ProgressiveWidening, SearchActionReport, SearchCheckpoint,
    SearchConfig, SearchDomain, SearchEngine, SearchError, SearchLimit, SearchReport,
    SearchStopReason,
};
pub use target::{TargetDecision, TargetIntent, TargetPolicy, TargetPolicyError};
pub use tier::{
    AiPolicyFamily, AiTierDefinition, AiTierError, AiTierSet, DifficultyTier, MulliganQuality,
};

use forge_core::PlayerView;

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

/// Returns true when a redacted player view is structurally readable by AI code.
#[must_use]
pub fn can_read_player_view(view: &PlayerView) -> bool {
    view.players().get(view.observer().index()).is_some()
}

#[cfg(test)]
mod tests {
    use super::{can_read_player_view, crate_ready};
    use forge_core::{apply, Action, GameState, Outcome};

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn ai_entrypoint_accepts_player_view() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player outcome: {other:?}"),
        };
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("unexpected view error: {error:?}"));

        assert!(can_read_player_view(&view));
    }
}
