use forge_core::{CanonicalActionId, DecisionContext};
use std::{error::Error, fmt};

/// Seeded random-legal policy over the canonical production action set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RandomLegalPolicy {
    seed: u64,
}

impl RandomLegalPolicy {
    /// Creates a replayable random-legal policy.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Selects one legal action using the context ID and decision ordinal.
    pub fn select(
        self,
        context: &DecisionContext,
        decision_index: u64,
    ) -> Result<CanonicalActionId, RandomPolicyError> {
        let options = context.options();
        if options.is_empty() {
            return Err(RandomPolicyError::NoLegalActions);
        }
        let context_id = context.id().get();
        let mixed = splitmix64(
            self.seed
                ^ decision_index.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ context_id as u64
                ^ ((context_id >> 64) as u64).rotate_left(17),
        );
        Ok(options[(mixed as usize) % options.len()].id())
    }
}

/// Fail-closed random-policy errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RandomPolicyError {
    /// The supplied context has no legal actions.
    NoLegalActions,
}

impl fmt::Display for RandomPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLegalActions => write!(formatter, "random policy received no legal actions"),
        }
    }
}

impl Error for RandomPolicyError {}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::RandomLegalPolicy;
    use forge_core::{
        apply, Action, DecisionContext, DecisionDescriptor, DecisionKind, DecisionOption,
        GameState, Outcome,
    };

    #[test]
    fn random_legal_selection_is_seeded_and_a_legal_member() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected AddPlayer outcome: {other:?}"),
        };
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let context = DecisionContext::new(
            DecisionKind::NumericValue,
            player,
            &view,
            vec![
                DecisionOption::new(DecisionDescriptor::ChooseNumber { value: 1 }, Vec::new()),
                DecisionOption::new(DecisionDescriptor::ChooseNumber { value: 2 }, Vec::new()),
            ],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("context failed: {error}"));
        let policy = RandomLegalPolicy::new(7);
        let first = policy
            .select(&context, 3)
            .unwrap_or_else(|error| panic!("selection failed: {error}"));
        let second = policy
            .select(&context, 3)
            .unwrap_or_else(|error| panic!("selection failed: {error}"));
        assert_eq!(first, second);
        assert!(context.select(first).is_ok());
    }
}
