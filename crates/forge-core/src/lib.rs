#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Pure rules-kernel crate for Forge 2.0.
//!
//! T1 starts with deterministic game-state storage. This crate intentionally
//! contains no card behavior yet; it provides the stable arenas, typed IDs,
//! zones, snapshots, invariants, and hashing that later rules systems build on.

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

/// A stable player handle into [`GameState`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlayerId(u32);

impl PlayerId {
    /// Returns the zero-based arena index for this player.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A stable object handle into the object arena.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(u32);

impl ObjectId {
    /// Returns the zero-based arena index for this object.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identifier for a printed card definition.
///
/// T1.1 does not compile real cards yet. This ID lets tests and future card
/// databases refer to card definitions without embedding names in engine state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CardId(u32);

impl CardId {
    /// Creates a card-definition ID from a deterministic numeric value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw deterministic card-definition value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Zone categories tracked by the T1 state arena.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ZoneKind {
    /// A player's library.
    Library,
    /// A player's hand.
    Hand,
    /// The shared battlefield.
    Battlefield,
    /// A player's graveyard.
    Graveyard,
    /// The shared exile zone.
    Exile,
    /// The shared stack zone.
    Stack,
    /// The shared command zone.
    Command,
}

impl ZoneKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Library => 0,
            Self::Hand => 1,
            Self::Battlefield => 2,
            Self::Graveyard => 3,
            Self::Exile => 4,
            Self::Stack => 5,
            Self::Command => 6,
        }
    }

    const fn requires_owner(self) -> bool {
        matches!(self, Self::Library | Self::Hand | Self::Graveyard)
    }
}

/// A specific zone in a game.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ZoneId {
    owner: Option<PlayerId>,
    kind: ZoneKind,
}

impl ZoneId {
    /// Creates a zone ID.
    #[must_use]
    pub const fn new(owner: Option<PlayerId>, kind: ZoneKind) -> Self {
        Self { owner, kind }
    }

    /// Returns the zone owner, if the zone is player-owned.
    #[must_use]
    pub const fn owner(self) -> Option<PlayerId> {
        self.owner
    }

    /// Returns the kind of this zone.
    #[must_use]
    pub const fn kind(self) -> ZoneKind {
        self.kind
    }
}

/// Scalar state for one player.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerState {
    id: PlayerId,
    life: i32,
    poison: u32,
    max_hand_size: u32,
}

impl PlayerState {
    /// Creates a player state with Magic's default constructed-game scalars.
    #[must_use]
    pub const fn new(id: PlayerId) -> Self {
        Self {
            id,
            life: 20,
            poison: 0,
            max_hand_size: 7,
        }
    }

    /// Returns the player's stable ID.
    #[must_use]
    pub const fn id(self) -> PlayerId {
        self.id
    }

    /// Returns the player's current life total.
    #[must_use]
    pub const fn life(self) -> i32 {
        self.life
    }

    /// Returns the player's poison-counter total.
    #[must_use]
    pub const fn poison(self) -> u32 {
        self.poison
    }

    /// Returns the player's current maximum hand size.
    #[must_use]
    pub const fn max_hand_size(self) -> u32 {
        self.max_hand_size
    }
}

/// Arena record for one game object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectRecord {
    id: ObjectId,
    card: CardId,
    owner: PlayerId,
    controller: PlayerId,
}

impl ObjectRecord {
    /// Returns the stable object ID.
    #[must_use]
    pub const fn id(self) -> ObjectId {
        self.id
    }

    /// Returns the printed card-definition ID.
    #[must_use]
    pub const fn card(self) -> CardId {
        self.card
    }

    /// Returns the object's owner.
    #[must_use]
    pub const fn owner(self) -> PlayerId {
        self.owner
    }

    /// Returns the object's controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }
}

/// Arena storage for game objects.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObjectArena {
    records: Vec<ObjectRecord>,
}

impl ObjectArena {
    /// Returns the number of live object records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true when there are no object records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns an object record by ID.
    #[must_use]
    pub fn get(&self, id: ObjectId) -> Option<ObjectRecord> {
        self.records.get(id.index()).copied()
    }

    /// Iterates over object records in canonical arena order.
    pub fn iter(&self) -> impl Iterator<Item = ObjectRecord> + '_ {
        self.records.iter().copied()
    }

    fn push(&mut self, card: CardId, owner: PlayerId, controller: PlayerId) -> ObjectId {
        let id = ObjectId(self.records.len() as u32);
        self.records.push(ObjectRecord {
            id,
            card,
            owner,
            controller,
        });
        id
    }
}

/// Ordered object list for a zone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Zone {
    id: ZoneId,
    objects: Vec<ObjectId>,
}

impl Zone {
    /// Returns this zone's ID.
    #[must_use]
    pub const fn id(&self) -> ZoneId {
        self.id
    }

    /// Returns the objects in zone order.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }
}

/// Deterministic state hash.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateHash(u64);

impl StateHash {
    /// Returns the raw 64-bit FNV hash value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Immutable snapshot of a game state and its deterministic hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameSnapshot {
    state: GameState,
    hash: StateHash,
}

impl GameSnapshot {
    /// Returns the cloned state captured by this snapshot.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// Returns the deterministic hash captured with the snapshot.
    #[must_use]
    pub const fn hash(&self) -> StateHash {
        self.hash
    }
}

/// Successful zone-conservation validation summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZoneConservation {
    object_count: usize,
}

impl ZoneConservation {
    /// Returns the number of objects validated.
    #[must_use]
    pub const fn object_count(self) -> usize {
        self.object_count
    }
}

/// State validation or mutation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    /// The requested player ID does not exist.
    UnknownPlayer(PlayerId),
    /// The requested object ID does not exist.
    UnknownObject(ObjectId),
    /// The requested zone ID does not exist.
    UnknownZone(ZoneId),
    /// A player-owned zone was requested without a valid owner.
    InvalidZoneOwner(ZoneId),
    /// A zone contains an object ID that is not in the object arena.
    InvalidZoneObject {
        /// Zone containing the invalid object reference.
        zone: ZoneId,
        /// Object reference that was not present in the arena.
        object: ObjectId,
    },
    /// An object appears in more than one zone.
    DuplicateZoneMembership(ObjectId),
    /// An object appears in no zone.
    MissingZoneMembership(ObjectId),
}

/// Complete T1 game state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameState {
    seed: u64,
    turn_number: u32,
    active_player: Option<PlayerId>,
    priority_player: Option<PlayerId>,
    players: Vec<PlayerState>,
    objects: ObjectArena,
    zones: Vec<Zone>,
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

impl GameState {
    /// Creates an empty game state with shared public zones.
    #[must_use]
    pub fn new() -> Self {
        Self {
            seed: 0,
            turn_number: 0,
            active_player: None,
            priority_player: None,
            players: Vec::new(),
            objects: ObjectArena::default(),
            zones: vec![
                Zone {
                    id: ZoneId::new(None, ZoneKind::Battlefield),
                    objects: Vec::new(),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Exile),
                    objects: Vec::new(),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Stack),
                    objects: Vec::new(),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Command),
                    objects: Vec::new(),
                },
            ],
        }
    }

    /// Sets the deterministic game seed.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    /// Returns the deterministic game seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the current turn number.
    #[must_use]
    pub const fn turn_number(&self) -> u32 {
        self.turn_number
    }

    /// Returns the active player, if turn structure has selected one.
    #[must_use]
    pub const fn active_player(&self) -> Option<PlayerId> {
        self.active_player
    }

    /// Returns the player with priority, if priority has been assigned.
    #[must_use]
    pub const fn priority_player(&self) -> Option<PlayerId> {
        self.priority_player
    }

    /// Adds a player and that player's owned zones.
    pub fn add_player(&mut self) -> PlayerId {
        let id = PlayerId(self.players.len() as u32);
        self.players.push(PlayerState::new(id));
        self.zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Library),
            objects: Vec::new(),
        });
        self.zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Hand),
            objects: Vec::new(),
        });
        self.zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Graveyard),
            objects: Vec::new(),
        });
        id
    }

    /// Returns the players in arena order.
    #[must_use]
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// Returns object arena storage.
    #[must_use]
    pub const fn objects(&self) -> &ObjectArena {
        &self.objects
    }

    /// Returns all zones in canonical state order.
    #[must_use]
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// Returns one zone by ID.
    #[must_use]
    pub fn zone(&self, id: ZoneId) -> Option<&Zone> {
        let index = self.zone_index(id)?;
        self.zones.get(index)
    }

    /// Creates one object and places it into a zone.
    pub fn create_object(
        &mut self,
        card: CardId,
        owner: PlayerId,
        controller: PlayerId,
        zone: ZoneId,
    ) -> Result<ObjectId, StateError> {
        self.require_player(owner)?;
        self.require_player(controller)?;
        self.require_zone(zone)?;
        let object = self.objects.push(card, owner, controller);
        self.zone_mut(zone)?.objects.push(object);
        Ok(object)
    }

    /// Moves an object from its current zone to another zone.
    pub fn move_object(&mut self, object: ObjectId, to: ZoneId) -> Result<(), StateError> {
        if self.objects.get(object).is_none() {
            return Err(StateError::UnknownObject(object));
        }
        self.require_zone(to)?;
        let from_index = self
            .zones
            .iter()
            .position(|zone| zone.objects.contains(&object))
            .ok_or(StateError::MissingZoneMembership(object))?;
        let from_zone_id = self.zones[from_index].id;
        if from_zone_id == to {
            return Ok(());
        }
        let from_position = self.zones[from_index]
            .objects
            .iter()
            .position(|candidate| *candidate == object)
            .ok_or(StateError::MissingZoneMembership(object))?;
        self.zones[from_index].objects.remove(from_position);
        self.zone_mut(to)?.objects.push(object);
        Ok(())
    }

    /// Returns the zone currently containing an object.
    #[must_use]
    pub fn object_zone(&self, object: ObjectId) -> Option<ZoneId> {
        self.zones
            .iter()
            .find(|zone| zone.objects.contains(&object))
            .map(Zone::id)
    }

    /// Validates that every object appears in exactly one zone.
    pub fn validate_zone_conservation(&self) -> Result<ZoneConservation, StateError> {
        let mut memberships = vec![0_u8; self.objects.len()];
        for zone in &self.zones {
            self.validate_zone_id(zone.id)?;
            for object in &zone.objects {
                if self.objects.get(*object).is_none() {
                    return Err(StateError::InvalidZoneObject {
                        zone: zone.id,
                        object: *object,
                    });
                }
                let Some(count) = memberships.get_mut(object.index()) else {
                    return Err(StateError::InvalidZoneObject {
                        zone: zone.id,
                        object: *object,
                    });
                };
                *count = count.saturating_add(1);
                if *count > 1 {
                    return Err(StateError::DuplicateZoneMembership(*object));
                }
            }
        }
        for object in self.objects.iter() {
            if memberships[object.id().index()] == 0 {
                return Err(StateError::MissingZoneMembership(object.id()));
            }
        }
        Ok(ZoneConservation {
            object_count: self.objects.len(),
        })
    }

    /// Captures a cloned state snapshot with its current deterministic hash.
    #[must_use]
    pub fn snapshot(&self) -> GameSnapshot {
        GameSnapshot {
            state: self.clone(),
            hash: self.deterministic_hash(),
        }
    }

    /// Computes the canonical FNV-1a state hash.
    #[must_use]
    pub fn deterministic_hash(&self) -> StateHash {
        let mut hash = Fnva64::new();
        for byte in self.canonical_bytes() {
            hash.write_u8(byte);
        }
        StateHash(hash.finish())
    }

    /// Returns the full-information canonical byte representation.
    ///
    /// This is for deterministic replay, tests, and diagnostics. Player-facing
    /// views must use a redacted projection once hidden information exists.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = CanonicalBytes::default();
        bytes.write_u64(self.seed);
        bytes.write_u32(self.turn_number);
        bytes.write_optional_player(self.active_player);
        bytes.write_optional_player(self.priority_player);
        bytes.write_u32(self.players.len() as u32);
        for player in &self.players {
            bytes.write_u32(player.id.0);
            bytes.write_i32(player.life);
            bytes.write_u32(player.poison);
            bytes.write_u32(player.max_hand_size);
        }

        bytes.write_u32(self.objects.len() as u32);
        for object in self.objects.iter() {
            bytes.write_u32(object.id.0);
            bytes.write_u32(object.card.0);
            bytes.write_u32(object.owner.0);
            bytes.write_u32(object.controller.0);
        }

        bytes.write_u32(self.zones.len() as u32);
        for zone in &self.zones {
            bytes.write_zone_id(zone.id);
            bytes.write_u32(zone.objects.len() as u32);
            for object in &zone.objects {
                bytes.write_u32(object.0);
            }
        }
        bytes.finish()
    }

    /// Computes the canonical FNV-1a state hash without allocating bytes.
    #[must_use]
    pub fn deterministic_hash_streaming(&self) -> StateHash {
        let mut hash = Fnva64::new();
        hash.write_u64(self.seed);
        hash.write_u32(self.turn_number);
        hash.write_optional_player(self.active_player);
        hash.write_optional_player(self.priority_player);
        hash.write_u32(self.players.len() as u32);
        for player in &self.players {
            hash.write_u32(player.id.0);
            hash.write_i32(player.life);
            hash.write_u32(player.poison);
            hash.write_u32(player.max_hand_size);
        }

        hash.write_u32(self.objects.len() as u32);
        for object in self.objects.iter() {
            hash.write_u32(object.id.0);
            hash.write_u32(object.card.0);
            hash.write_u32(object.owner.0);
            hash.write_u32(object.controller.0);
        }

        hash.write_u32(self.zones.len() as u32);
        for zone in &self.zones {
            hash.write_zone_id(zone.id);
            hash.write_u32(zone.objects.len() as u32);
            for object in &zone.objects {
                hash.write_u32(object.0);
            }
        }

        StateHash(hash.finish())
    }

    fn require_player(&self, id: PlayerId) -> Result<(), StateError> {
        if self.players.get(id.index()).is_some() {
            Ok(())
        } else {
            Err(StateError::UnknownPlayer(id))
        }
    }

    fn require_zone(&self, id: ZoneId) -> Result<(), StateError> {
        self.validate_zone_id(id)?;
        if self.zone_index(id).is_some() {
            Ok(())
        } else {
            Err(StateError::UnknownZone(id))
        }
    }

    fn validate_zone_id(&self, id: ZoneId) -> Result<(), StateError> {
        match (id.kind.requires_owner(), id.owner) {
            (true, Some(owner)) => self.require_player(owner),
            (true, None) | (false, Some(_)) => Err(StateError::InvalidZoneOwner(id)),
            (false, None) => Ok(()),
        }
    }

    fn zone_index(&self, id: ZoneId) -> Option<usize> {
        self.zones.iter().position(|zone| zone.id == id)
    }

    fn zone_mut(&mut self, id: ZoneId) -> Result<&mut Zone, StateError> {
        let index = self.zone_index(id).ok_or(StateError::UnknownZone(id))?;
        Ok(&mut self.zones[index])
    }
}

struct Fnva64 {
    value: u64,
}

impl Fnva64 {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    const fn new() -> Self {
        Self { value: Self::BASIS }
    }

    const fn finish(self) -> u64 {
        self.value
    }

    fn write_u8(&mut self, value: u8) {
        self.value ^= u64::from(value);
        self.value = self.value.wrapping_mul(Self::PRIME);
    }

    fn write_u32(&mut self, value: u32) {
        for byte in value.to_le_bytes() {
            self.write_u8(byte);
        }
    }

    fn write_u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.write_u8(byte);
        }
    }

    fn write_i32(&mut self, value: i32) {
        for byte in value.to_le_bytes() {
            self.write_u8(byte);
        }
    }

    fn write_optional_player(&mut self, player: Option<PlayerId>) {
        match player {
            Some(player) => {
                self.write_u8(1);
                self.write_u32(player.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_zone_id(&mut self, zone: ZoneId) {
        self.write_u8(zone.kind.canonical_code());
        match zone.owner {
            Some(owner) => {
                self.write_u8(1);
                self.write_u32(owner.0);
            }
            None => self.write_u8(0),
        }
    }
}

#[derive(Default)]
struct CanonicalBytes {
    bytes: Vec<u8>,
}

impl CanonicalBytes {
    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn write_u32(&mut self, value: u32) {
        self.bytes.extend(value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.bytes.extend(value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.bytes.extend(value.to_le_bytes());
    }

    fn write_optional_player(&mut self, player: Option<PlayerId>) {
        match player {
            Some(player) => {
                self.write_u8(1);
                self.write_u32(player.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_zone_id(&mut self, zone: ZoneId) {
        self.write_u8(zone.kind.canonical_code());
        match zone.owner {
            Some(owner) => {
                self.write_u8(1);
                self.write_u32(owner.0);
            }
            None => self.write_u8(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{crate_ready, CardId, GameState, StateError, ZoneConservation, ZoneId, ZoneKind};

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn players_receive_owned_zones() {
        let mut state = GameState::new();
        let alice = state.add_player();
        let bob = state.add_player();

        assert_eq!(alice.index(), 0);
        assert_eq!(bob.index(), 1);
        assert!(state
            .zone(ZoneId::new(Some(alice), ZoneKind::Library))
            .is_some());
        assert!(state
            .zone(ZoneId::new(None, ZoneKind::Battlefield))
            .is_some());
        assert_eq!(
            state.zone(ZoneId::new(Some(alice), ZoneKind::Battlefield)),
            None
        );
    }

    #[test]
    fn objects_move_between_zones_and_conserve_membership() {
        let mut state = GameState::new();
        let player = state.add_player();
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);

        let object = state
            .create_object(CardId::new(100), player, player, hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        assert_eq!(state.object_zone(object), Some(hand));

        state
            .move_object(object, battlefield)
            .unwrap_or_else(|error| panic!("unexpected move error: {error:?}"));
        assert_eq!(state.object_zone(object), Some(battlefield));
        assert_eq!(
            state.validate_zone_conservation(),
            Ok(ZoneConservation { object_count: 1 })
        );
    }

    #[test]
    fn deterministic_hash_is_canonical_and_sensitive_to_ordered_state() {
        let mut left = GameState::new();
        let left_player = left.add_player();
        let left_hand = ZoneId::new(Some(left_player), ZoneKind::Hand);
        let left_battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let left_object = left
            .create_object(CardId::new(7), left_player, left_player, left_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));

        let mut right = GameState::new();
        let right_player = right.add_player();
        let right_hand = ZoneId::new(Some(right_player), ZoneKind::Hand);
        let right_object = right
            .create_object(CardId::new(7), right_player, right_player, right_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));

        assert_eq!(left_object.index(), right_object.index());
        assert_eq!(left.deterministic_hash(), right.deterministic_hash());
        assert_eq!(
            left.deterministic_hash(),
            left.deterministic_hash_streaming()
        );
        assert_eq!(left.canonical_bytes(), right.canonical_bytes());

        let before = left.deterministic_hash();
        let snapshot = left.snapshot();
        left.move_object(left_object, left_battlefield)
            .unwrap_or_else(|error| panic!("unexpected move error: {error:?}"));

        assert_eq!(snapshot.hash(), before);
        assert_ne!(left.deterministic_hash(), before);
    }

    #[test]
    fn invalid_zone_owner_is_rejected() {
        let mut state = GameState::new();
        let player = state.add_player();
        let result = state.create_object(
            CardId::new(1),
            player,
            player,
            ZoneId::new(None, ZoneKind::Hand),
        );

        assert_eq!(
            result,
            Err(StateError::InvalidZoneOwner(ZoneId::new(
                None,
                ZoneKind::Hand
            )))
        );
    }
}
