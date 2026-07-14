use forge_core::{
    CanonicalActionId, DecisionContext, DecisionDescriptor, DecisionKind, ObjectId, ObjectView,
    PlayerId, PlayerView, ZoneId, ZoneKind,
};
use std::{collections::BTreeSet, error::Error, fmt};

/// Inspectable result from the baseline London mulligan policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MulliganDecision {
    action_id: CanonicalActionId,
    score: i64,
    keep: bool,
}

impl MulliganDecision {
    /// Returns the selected canonical action ID.
    #[must_use]
    pub const fn action_id(self) -> CanonicalActionId {
        self.action_id
    }

    /// Returns the card-agnostic opening-hand score.
    #[must_use]
    pub const fn score(self) -> i64 {
        self.score
    }

    /// Returns whether the selected action keeps the hand.
    #[must_use]
    pub const fn keeps(self) -> bool {
        self.keep
    }
}

/// Deterministic baseline London mulligan policy.
///
/// This policy deliberately uses only public card characteristics in the
/// acting player's redacted hand. It has no card-name or deck-specific branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MulliganPolicy {
    max_mulligans: u32,
}

impl MulliganPolicy {
    /// Creates the baseline policy, which will accept the best hand after at
    /// most three mulligans.
    #[must_use]
    pub const fn baseline() -> Self {
        Self { max_mulligans: 3 }
    }

    /// Selects a keep/bottom or mulligan action from the complete legal set.
    pub fn select(
        self,
        context: &DecisionContext,
        view: &PlayerView,
    ) -> Result<MulliganDecision, MulliganError> {
        if context.kind() != DecisionKind::OpeningHand {
            return Err(MulliganError::WrongDecisionKind(context.kind()));
        }
        if context.actor() != view.observer() {
            return Err(MulliganError::ObserverMismatch {
                actor: context.actor(),
                observer: view.observer(),
            });
        }
        let player = view
            .players()
            .get(context.actor().index())
            .copied()
            .ok_or(MulliganError::MissingActor(context.actor()))?;
        let hand = visible_hand(view, context.actor())?;
        let should_mulligan =
            player.mulligans_taken() < self.max_mulligans && !opening_hand_is_acceptable(&hand);

        let mut best = None::<MulliganDecision>;
        for option in context.options() {
            let (score, keep) = match option.descriptor() {
                DecisionDescriptor::TakeMulligan => (
                    if should_mulligan {
                        1_000_000
                    } else {
                        -1_000_000
                    },
                    false,
                ),
                DecisionDescriptor::KeepOpeningHand { bottom } => {
                    (score_keep(&hand, bottom)?, true)
                }
                descriptor => {
                    return Err(MulliganError::UnexpectedDescriptor(format!(
                        "{descriptor:?}"
                    )))
                }
            };
            let candidate = MulliganDecision {
                action_id: option.id(),
                score,
                keep,
            };
            if best.map_or(true, |current| {
                candidate.score > current.score
                    || (candidate.score == current.score && candidate.action_id < current.action_id)
            }) {
                best = Some(candidate);
            }
        }
        best.ok_or(MulliganError::NoOptions)
    }
}

#[derive(Clone, Copy)]
struct HandCard {
    object: ObjectId,
    land: bool,
    mana_value: u32,
}

fn visible_hand(view: &PlayerView, player: PlayerId) -> Result<Vec<HandCard>, MulliganError> {
    let hand = view
        .zone(ZoneId::new(Some(player), ZoneKind::Hand))
        .ok_or(MulliganError::MissingHand(player))?;
    hand.objects()
        .iter()
        .map(|object| match object {
            ObjectView::Known { object, .. } => Ok(HandCard {
                object: object.id(),
                land: object.base_object().types().land(),
                mana_value: object.base_object().mana_value(),
            }),
            ObjectView::Hidden => Err(MulliganError::ActorHandHidden(player)),
        })
        .collect()
}

fn opening_hand_is_acceptable(hand: &[HandCard]) -> bool {
    let lands = hand.iter().filter(|card| card.land).count();
    let low_curve = hand.iter().any(|card| !card.land && card.mana_value <= 3);
    (2..=5).contains(&lands) && low_curve
}

fn score_keep(hand: &[HandCard], bottom: &[ObjectId]) -> Result<i64, MulliganError> {
    let mut bottomed = BTreeSet::new();
    for object in bottom {
        if !bottomed.insert(*object) {
            return Err(MulliganError::DuplicateBottomCard(*object));
        }
        if !hand.iter().any(|card| card.object == *object) {
            return Err(MulliganError::BottomCardOutsideHand(*object));
        }
    }
    let retained = hand
        .iter()
        .copied()
        .filter(|card| !bottomed.contains(&card.object))
        .collect::<Vec<_>>();
    let desired_lands = match retained.len() {
        0 => 0,
        1..=3 => 1,
        4..=5 => 2,
        _ => 3,
    };
    let lands = retained.iter().filter(|card| card.land).count();
    let land_error = lands.abs_diff(desired_lands) as i64;
    let low_curve = retained
        .iter()
        .filter(|card| !card.land && card.mana_value <= 3)
        .count() as i64;
    let high_curve = retained
        .iter()
        .filter(|card| !card.land)
        .map(|card| i64::from(card.mana_value.saturating_sub(4)))
        .sum::<i64>();
    Ok(10_000_i64
        .saturating_sub(land_error.saturating_mul(2_000))
        .saturating_add(low_curve.saturating_mul(100))
        .saturating_sub(high_curve.saturating_mul(25)))
}

/// Fail-closed baseline mulligan errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MulliganError {
    /// The policy received a non-opening-hand prompt.
    WrongDecisionKind(DecisionKind),
    /// The context actor and view observer disagree.
    ObserverMismatch {
        /// Context actor.
        actor: PlayerId,
        /// View observer.
        observer: PlayerId,
    },
    /// The actor does not exist in the supplied view.
    MissingActor(PlayerId),
    /// The actor's hand zone is absent.
    MissingHand(PlayerId),
    /// The actor's own hand was unexpectedly redacted.
    ActorHandHidden(PlayerId),
    /// A keep option named a card outside the hand.
    BottomCardOutsideHand(ObjectId),
    /// A keep option bottomed one card more than once.
    DuplicateBottomCard(ObjectId),
    /// A non-mulligan descriptor appeared in the context.
    UnexpectedDescriptor(String),
    /// The context had no options.
    NoOptions,
}

impl fmt::Display for MulliganError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongDecisionKind(kind) => {
                write!(formatter, "expected opening-hand context, got {kind:?}")
            }
            Self::ObserverMismatch { actor, observer } => write!(
                formatter,
                "opening-hand actor seat {} does not match observer seat {}",
                actor.index() + 1,
                observer.index() + 1
            ),
            Self::MissingActor(player) => {
                write!(formatter, "missing actor seat {}", player.index() + 1)
            }
            Self::MissingHand(player) => {
                write!(formatter, "missing hand for seat {}", player.index() + 1)
            }
            Self::ActorHandHidden(player) => write!(
                formatter,
                "seat {} cannot see its own hand",
                player.index() + 1
            ),
            Self::BottomCardOutsideHand(object) => write!(
                formatter,
                "bottom card {} is outside the hand",
                object.index()
            ),
            Self::DuplicateBottomCard(object) => {
                write!(formatter, "bottom card {} is duplicated", object.index())
            }
            Self::UnexpectedDescriptor(descriptor) => {
                write!(formatter, "unexpected opening-hand descriptor {descriptor}")
            }
            Self::NoOptions => write!(formatter, "opening-hand context has no options"),
        }
    }
}

impl Error for MulliganError {}

#[cfg(test)]
mod tests {
    use super::MulliganPolicy;
    use forge_core::{
        apply, Action, BaseObjectCharacteristics, CardId, DecisionContext, DecisionDescriptor,
        DecisionKind, DecisionOption, GameState, ObjectTypes, Outcome, ZoneId, ZoneKind,
    };

    fn opening_context(land_count: usize) -> (GameState, forge_core::PlayerId, DecisionContext) {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player result: {other:?}"),
        };
        for index in 0..7 {
            let object = match apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(index as u32 + 1),
                    owner: player,
                    controller: player,
                    zone: ZoneId::new(Some(player), ZoneKind::Hand),
                },
            ) {
                Outcome::ObjectCreated(object) => object,
                other => panic!("unexpected object result: {other:?}"),
            };
            let types = if index < land_count {
                ObjectTypes::none().with_land()
            } else {
                ObjectTypes::none().with_creature()
            };
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object,
                        base: BaseObjectCharacteristics::new(
                            types,
                            forge_core::ObjectColors::none()
                        )
                        .with_mana_value(2),
                    },
                ),
                Outcome::Applied
            );
        }
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let context = DecisionContext::new(
            DecisionKind::OpeningHand,
            player,
            &view,
            vec![
                DecisionOption::new(
                    DecisionDescriptor::TakeMulligan,
                    vec![Action::TakeMulligan { player }],
                ),
                DecisionOption::new(
                    DecisionDescriptor::KeepOpeningHand { bottom: Vec::new() },
                    vec![Action::KeepOpeningHand {
                        player,
                        bottom: Vec::new(),
                    }],
                ),
            ],
            Vec::new(),
        )
        .unwrap_or_else(|error| panic!("context failed: {error}"));
        (state, player, context)
    }

    #[test]
    fn baseline_keeps_balanced_hands_and_mulligans_zero_land_hands() {
        let (good_state, player, good_context) = opening_context(3);
        let good_view = good_state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let good = MulliganPolicy::baseline()
            .select(&good_context, &good_view)
            .unwrap_or_else(|error| panic!("mulligan decision failed: {error}"));
        assert!(good.keeps());

        let (bad_state, player, bad_context) = opening_context(0);
        let bad_view = bad_state
            .player_view(player)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let bad = MulliganPolicy::baseline()
            .select(&bad_context, &bad_view)
            .unwrap_or_else(|error| panic!("mulligan decision failed: {error}"));
        assert!(!bad.keeps());
    }
}
