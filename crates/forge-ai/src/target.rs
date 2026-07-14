use crate::AiWeights;
use forge_core::{
    CanonicalActionId, DecisionContext, DecisionDescriptor, DecisionKind, ObjectId, ObjectView,
    PlayerId, PlayerView, TargetChoice,
};
use std::{error::Error, fmt};

/// Semantic direction of one target prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetIntent {
    /// Prefer visible threats controlled by another player.
    Harmful,
    /// Prefer valuable visible targets controlled by the actor.
    Beneficial,
    /// Preserve canonical order without a value assumption.
    Neutral,
}

/// Inspectable result from the baseline target selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetDecision {
    action_id: CanonicalActionId,
    score: i64,
}

impl TargetDecision {
    /// Returns the selected canonical target action.
    #[must_use]
    pub const fn action_id(self) -> CanonicalActionId {
        self.action_id
    }

    /// Returns the visible, card-agnostic target score.
    #[must_use]
    pub const fn score(self) -> i64 {
        self.score
    }
}

/// Baseline typed threat and target policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetPolicy {
    weights: AiWeights,
}

impl TargetPolicy {
    /// Creates a target policy from the versioned evaluation weights.
    #[must_use]
    pub const fn new(weights: AiWeights) -> Self {
        Self { weights }
    }

    /// Selects one target from a complete canonical target context.
    pub fn select(
        self,
        context: &DecisionContext,
        view: &PlayerView,
        intent: TargetIntent,
    ) -> Result<TargetDecision, TargetPolicyError> {
        if context.kind() != DecisionKind::Target {
            return Err(TargetPolicyError::WrongDecisionKind(context.kind()));
        }
        if context.actor() != view.observer() {
            return Err(TargetPolicyError::ObserverMismatch {
                actor: context.actor(),
                observer: view.observer(),
            });
        }
        let mut best = None::<TargetDecision>;
        for option in context.options() {
            let DecisionDescriptor::ChooseTarget { target } = option.descriptor() else {
                return Err(TargetPolicyError::UnexpectedDescriptor(format!(
                    "{:?}",
                    option.descriptor()
                )));
            };
            let (value, friendly) = target_value(self.weights, view, *target)?;
            let score = match intent {
                TargetIntent::Harmful if friendly => value.saturating_neg(),
                TargetIntent::Harmful => value,
                TargetIntent::Beneficial if friendly => value,
                TargetIntent::Beneficial => value.saturating_neg(),
                TargetIntent::Neutral => 0,
            };
            let candidate = TargetDecision {
                action_id: option.id(),
                score,
            };
            if best.map_or(true, |current| {
                candidate.score > current.score
                    || (candidate.score == current.score && candidate.action_id < current.action_id)
            }) {
                best = Some(candidate);
            }
        }
        best.ok_or(TargetPolicyError::NoOptions)
    }
}

fn target_value(
    weights: AiWeights,
    view: &PlayerView,
    target: TargetChoice,
) -> Result<(i64, bool), TargetPolicyError> {
    match target {
        TargetChoice::Player(player) => {
            let state = view
                .players()
                .get(player.index())
                .copied()
                .ok_or(TargetPolicyError::UnknownPlayer(player))?;
            Ok((weights.player_threat(state), player == view.observer()))
        }
        TargetChoice::Object(object) => {
            let (record, characteristics) =
                find_object(view, object).ok_or(TargetPolicyError::ObjectNotVisible(object))?;
            Ok((
                weights.object_threat(record, characteristics),
                characteristics.controller() == view.observer(),
            ))
        }
        TargetChoice::StackEntry(_) => Ok((0, false)),
    }
}

fn find_object(
    view: &PlayerView,
    object: ObjectId,
) -> Option<(forge_core::ObjectRecord, forge_core::ObjectCharacteristics)> {
    view.zones()
        .iter()
        .flat_map(|zone| zone.objects())
        .find_map(|candidate| match candidate {
            ObjectView::Known {
                object: record,
                characteristics,
            } if record.id() == object => Some((*record, *characteristics)),
            ObjectView::Known { .. } | ObjectView::Hidden => None,
        })
}

/// Fail-closed target-policy errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetPolicyError {
    /// The policy received a non-target prompt.
    WrongDecisionKind(DecisionKind),
    /// The context actor and view observer disagree.
    ObserverMismatch {
        /// Context actor.
        actor: PlayerId,
        /// View observer.
        observer: PlayerId,
    },
    /// A target player is outside the view.
    UnknownPlayer(PlayerId),
    /// A targeted object is not visible to the actor.
    ObjectNotVisible(ObjectId),
    /// A non-target descriptor appeared in the context.
    UnexpectedDescriptor(String),
    /// The context had no options.
    NoOptions,
}

impl fmt::Display for TargetPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongDecisionKind(kind) => {
                write!(formatter, "expected target context, got {kind:?}")
            }
            Self::ObserverMismatch { actor, observer } => write!(
                formatter,
                "target actor seat {} does not match observer seat {}",
                actor.index() + 1,
                observer.index() + 1
            ),
            Self::UnknownPlayer(player) => {
                write!(formatter, "unknown target seat {}", player.index() + 1)
            }
            Self::ObjectNotVisible(object) => {
                write!(formatter, "target object {} is not visible", object.index())
            }
            Self::UnexpectedDescriptor(descriptor) => {
                write!(formatter, "unexpected target descriptor {descriptor}")
            }
            Self::NoOptions => write!(formatter, "target context has no options"),
        }
    }
}

impl Error for TargetPolicyError {}

#[cfg(test)]
mod tests {
    use super::{TargetIntent, TargetPolicy};
    use crate::AiWeights;
    use forge_core::{
        apply, Action, BaseCreatureCharacteristics, BaseObjectCharacteristics, CardId,
        DecisionContext, DecisionDescriptor, DecisionKind, DecisionOption, GameState, ObjectColors,
        ObjectTypes, Outcome, TargetChoice, ZoneId, ZoneKind,
    };

    #[test]
    fn harmful_targeting_prefers_the_larger_visible_opposing_threat() {
        let mut state = GameState::new();
        let actor = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected actor result: {other:?}"),
        };
        let opponent = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected opponent result: {other:?}"),
        };
        let mut objects = Vec::new();
        for (index, size) in [1, 6].into_iter().enumerate() {
            let object = match apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(index as u32 + 1),
                    owner: opponent,
                    controller: opponent,
                    zone: ZoneId::new(None, ZoneKind::Battlefield),
                },
            ) {
                Outcome::ObjectCreated(object) => object,
                other => panic!("unexpected object result: {other:?}"),
            };
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(
                            ObjectTypes::none().with_creature(),
                            ObjectColors::none(),
                        )
                        .with_mana_value(size as u32),
                    },
                ),
                Outcome::Applied
            );
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseCreatureCharacteristics {
                        object,
                        base: BaseCreatureCharacteristics::new(size, size),
                    },
                ),
                Outcome::Applied
            );
            objects.push(object);
        }
        let view = state
            .player_view(actor)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let options = objects
            .iter()
            .copied()
            .map(|object| {
                DecisionOption::new(
                    DecisionDescriptor::ChooseTarget {
                        target: TargetChoice::Object(object),
                    },
                    Vec::new(),
                )
            })
            .collect();
        let context = DecisionContext::new(DecisionKind::Target, actor, &view, options, Vec::new())
            .unwrap_or_else(|error| panic!("context failed: {error}"));
        let decision = TargetPolicy::new(
            AiWeights::bundled().unwrap_or_else(|error| panic!("weights failed: {error}")),
        )
        .select(&context, &view, TargetIntent::Harmful)
        .unwrap_or_else(|error| panic!("target selection failed: {error}"));
        let selected = context
            .select(decision.action_id())
            .unwrap_or_else(|error| panic!("selection failed: {error}"));
        assert!(matches!(
            selected.descriptor(),
            DecisionDescriptor::ChooseTarget {
                target: TargetChoice::Object(object)
            } if *object == objects[1]
        ));
    }
}
