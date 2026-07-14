use forge_core::{CardId, ObjectView, PlayerId, PlayerView, ZoneId, ZoneKind};
use std::{collections::BTreeMap, error::Error, fmt};

/// Exact legal deck composition supplied to the determinizer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckModel {
    player: PlayerId,
    cards: Vec<CardId>,
}

impl DeckModel {
    /// Creates one exact deck model. Duplicate card IDs are retained.
    #[must_use]
    pub fn new(player: PlayerId, cards: Vec<CardId>) -> Self {
        Self { player, cards }
    }

    /// Returns the player whose hidden zones this model describes.
    #[must_use]
    pub const fn player(&self) -> PlayerId {
        self.player
    }

    /// Returns the exact deck multiset.
    #[must_use]
    pub fn cards(&self) -> &[CardId] {
        &self.cards
    }
}

/// One sampled identity for one redacted hidden-zone slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct HiddenAssignment {
    zone: ZoneId,
    slot: usize,
    card: CardId,
}

impl HiddenAssignment {
    /// Returns the hidden zone containing this slot.
    #[must_use]
    pub const fn zone(self) -> ZoneId {
        self.zone
    }

    /// Returns the zero-based position in the visible zone projection.
    #[must_use]
    pub const fn slot(self) -> usize {
        self.slot
    }

    /// Returns the sampled card identity.
    #[must_use]
    pub const fn card(self) -> CardId {
        self.card
    }
}

/// One complete, deterministic assignment of legal cards to hidden slots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Determinization {
    seed: u64,
    assignments: Vec<HiddenAssignment>,
    fingerprint: u64,
}

impl Determinization {
    /// Returns the seed used for this sample.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns sampled assignments in canonical zone/slot order.
    #[must_use]
    pub fn assignments(&self) -> &[HiddenAssignment] {
        &self.assignments
    }

    /// Returns a stable fingerprint of the sample.
    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Returns the sampled identity at one hidden slot.
    #[must_use]
    pub fn card_at(&self, zone: ZoneId, slot: usize) -> Option<CardId> {
        self.assignments
            .binary_search_by_key(&(zone, slot), |assignment| {
                (assignment.zone, assignment.slot)
            })
            .ok()
            .map(|index| self.assignments[index].card)
    }
}

/// Fail-closed determinization errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeterminizationError {
    /// More than one model was supplied for a player.
    DuplicateDeckModel(PlayerId),
    /// A model refers to a player outside the view.
    UnknownDeckPlayer(PlayerId),
    /// A hidden zone has no exact deck model.
    MissingDeckModel(PlayerId),
    /// A known nontoken card cannot be reconciled with the deck model.
    KnownCardOutsideDeck {
        /// Card owner.
        player: PlayerId,
        /// Unreconciled card identity.
        card: CardId,
    },
    /// The remaining legal cards do not exactly fill the hidden slots.
    HiddenCountMismatch {
        /// Player whose model failed.
        player: PlayerId,
        /// Remaining cards after subtracting known objects.
        cards: usize,
        /// Hidden slots requiring assignments.
        slots: usize,
    },
}

impl fmt::Display for DeterminizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDeckModel(player) => {
                write!(
                    formatter,
                    "duplicate deck model for seat {}",
                    player.index() + 1
                )
            }
            Self::UnknownDeckPlayer(player) => {
                write!(
                    formatter,
                    "deck model references unknown seat {}",
                    player.index() + 1
                )
            }
            Self::MissingDeckModel(player) => {
                write!(
                    formatter,
                    "hidden zones for seat {} have no deck model",
                    player.index() + 1
                )
            }
            Self::KnownCardOutsideDeck { player, card } => write!(
                formatter,
                "known card {} for seat {} is outside the deck model",
                card.get(),
                player.index() + 1
            ),
            Self::HiddenCountMismatch {
                player,
                cards,
                slots,
            } => write!(
                formatter,
                "seat {} has {cards} legal cards for {slots} hidden slots",
                player.index() + 1
            ),
        }
    }
}

impl Error for DeterminizationError {}

/// Deterministically samples hidden identities using only a view and deck models.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Determinizer {
    seed: u64,
}

impl Determinizer {
    /// Creates a determinizer for one sample seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Samples every hidden hand/library slot without reading a full game state.
    pub fn sample(
        self,
        view: &PlayerView,
        decks: &[DeckModel],
    ) -> Result<Determinization, DeterminizationError> {
        let mut models = BTreeMap::new();
        for deck in decks {
            if view.players().get(deck.player.index()).is_none() {
                return Err(DeterminizationError::UnknownDeckPlayer(deck.player));
            }
            if models.insert(deck.player, deck).is_some() {
                return Err(DeterminizationError::DuplicateDeckModel(deck.player));
            }
        }

        for zone in view.zones() {
            let Some(owner) = zone.id().owner() else {
                continue;
            };
            if zone.objects().iter().any(|object| object.is_hidden())
                && !models.contains_key(&owner)
            {
                return Err(DeterminizationError::MissingDeckModel(owner));
            }
        }

        let mut assignments = Vec::new();
        for (player, deck) in models {
            let mut remaining = deck.cards.clone();
            for zone in view.zones() {
                for object in zone.objects() {
                    let Some(record) = object.known() else {
                        continue;
                    };
                    if record.owner() != player || record.is_token() || record.is_copy() {
                        continue;
                    }
                    let Some(index) = remaining.iter().position(|card| *card == record.card())
                    else {
                        return Err(DeterminizationError::KnownCardOutsideDeck {
                            player,
                            card: record.card(),
                        });
                    };
                    remaining.remove(index);
                }
            }

            let hidden_slots = view
                .zones()
                .iter()
                .filter(|zone| {
                    zone.id().owner() == Some(player)
                        && matches!(zone.id().kind(), ZoneKind::Hand | ZoneKind::Library)
                })
                .flat_map(|zone| {
                    zone.objects()
                        .iter()
                        .enumerate()
                        .filter(|(_, object)| matches!(object, ObjectView::Hidden))
                        .map(|(slot, _)| (zone.id(), slot))
                })
                .collect::<Vec<_>>();
            if remaining.len() != hidden_slots.len() {
                return Err(DeterminizationError::HiddenCountMismatch {
                    player,
                    cards: remaining.len(),
                    slots: hidden_slots.len(),
                });
            }

            let player_seed = self.seed
                ^ (player.index() as u64)
                    .wrapping_add(1)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            shuffle(&mut remaining, player_seed);
            assignments.extend(
                hidden_slots
                    .into_iter()
                    .zip(remaining)
                    .map(|((zone, slot), card)| HiddenAssignment { zone, slot, card }),
            );
        }
        assignments.sort();
        let fingerprint = assignment_fingerprint(self.seed, &assignments);
        Ok(Determinization {
            seed: self.seed,
            assignments,
            fingerprint,
        })
    }
}

fn shuffle(cards: &mut [CardId], seed: u64) {
    let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    for index in (1..cards.len()).rev() {
        state = splitmix64(state);
        cards.swap(index, (state as usize) % (index + 1));
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn assignment_fingerprint(seed: u64, assignments: &[HiddenAssignment]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    write_hash(&mut hash, seed);
    for assignment in assignments {
        write_hash(
            &mut hash,
            assignment
                .zone
                .owner()
                .map_or(u64::MAX, |owner| owner.index() as u64),
        );
        write_hash(&mut hash, zone_kind_code(assignment.zone.kind()));
        write_hash(&mut hash, assignment.slot as u64);
        write_hash(&mut hash, u64::from(assignment.card.get()));
    }
    hash
}

fn write_hash(hash: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

const fn zone_kind_code(kind: ZoneKind) -> u64 {
    match kind {
        ZoneKind::Library => 0,
        ZoneKind::Hand => 1,
        ZoneKind::Battlefield => 2,
        ZoneKind::Graveyard => 3,
        ZoneKind::Exile => 4,
        ZoneKind::Stack => 5,
        ZoneKind::Command => 6,
        ZoneKind::Ceased => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::{DeckModel, Determinizer};
    use forge_core::{apply, Action, CardId, GameState, Outcome, ZoneId, ZoneKind};

    fn add_player(state: &mut GameState) -> forge_core::PlayerId {
        match apply(state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected AddPlayer outcome: {other:?}"),
        }
    }

    fn create(state: &mut GameState, card: u32, owner: forge_core::PlayerId, zone: ZoneId) {
        match apply(
            state,
            Action::CreateObject {
                card: CardId::new(card),
                owner,
                controller: owner,
                zone,
            },
        ) {
            Outcome::ObjectCreated(_) => {}
            other => panic!("unexpected CreateObject outcome: {other:?}"),
        }
    }

    #[test]
    fn determinization_subtracts_known_cards_and_fills_hidden_slots() {
        let mut state = GameState::new();
        let observer = add_player(&mut state);
        let opponent = add_player(&mut state);
        create(
            &mut state,
            10,
            opponent,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        create(
            &mut state,
            11,
            opponent,
            ZoneId::new(Some(opponent), ZoneKind::Library),
        );
        create(
            &mut state,
            12,
            opponent,
            ZoneId::new(Some(opponent), ZoneKind::Library),
        );
        let view = state
            .player_view(observer)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let model = DeckModel::new(
            opponent,
            vec![CardId::new(10), CardId::new(11), CardId::new(12)],
        );
        let first = Determinizer::new(77)
            .sample(&view, std::slice::from_ref(&model))
            .unwrap_or_else(|error| panic!("determinization failed: {error}"));
        let second = Determinizer::new(77)
            .sample(&view, &[model])
            .unwrap_or_else(|error| panic!("repeat determinization failed: {error}"));
        assert_eq!(first, second);
        let mut cards = first
            .assignments()
            .iter()
            .map(|assignment| assignment.card().get())
            .collect::<Vec<_>>();
        cards.sort_unstable();
        assert_eq!(cards, vec![11, 12]);
    }

    #[test]
    fn hidden_identity_poison_does_not_change_sample() {
        let mut left = GameState::new();
        let left_observer = add_player(&mut left);
        let left_opponent = add_player(&mut left);
        create(
            &mut left,
            21,
            left_opponent,
            ZoneId::new(Some(left_opponent), ZoneKind::Library),
        );
        create(
            &mut left,
            22,
            left_opponent,
            ZoneId::new(Some(left_opponent), ZoneKind::Library),
        );

        let mut right = GameState::new();
        let right_observer = add_player(&mut right);
        let right_opponent = add_player(&mut right);
        create(
            &mut right,
            22,
            right_opponent,
            ZoneId::new(Some(right_opponent), ZoneKind::Library),
        );
        create(
            &mut right,
            21,
            right_opponent,
            ZoneId::new(Some(right_opponent), ZoneKind::Library),
        );

        let left_view = left
            .player_view(left_observer)
            .unwrap_or_else(|error| panic!("left view failed: {error:?}"));
        let right_view = right
            .player_view(right_observer)
            .unwrap_or_else(|error| panic!("right view failed: {error:?}"));
        assert_eq!(left_view, right_view);
        let left_model = DeckModel::new(left_opponent, vec![CardId::new(21), CardId::new(22)]);
        let right_model = DeckModel::new(right_opponent, vec![CardId::new(21), CardId::new(22)]);
        let left_sample = Determinizer::new(99)
            .sample(&left_view, &[left_model])
            .unwrap_or_else(|error| panic!("left sample failed: {error}"));
        let right_sample = Determinizer::new(99)
            .sample(&right_view, &[right_model])
            .unwrap_or_else(|error| panic!("right sample failed: {error}"));
        assert_eq!(left_sample, right_sample);
    }
}
