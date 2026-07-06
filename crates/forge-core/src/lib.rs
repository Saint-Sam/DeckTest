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

/// The five phases that make up a turn under CR 500.1.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    /// The beginning phase: untap, upkeep, and draw.
    Beginning,
    /// The first main phase of the turn.
    PrecombatMain,
    /// The combat phase.
    Combat,
    /// The second main phase of the turn.
    PostcombatMain,
    /// The ending phase: end and cleanup.
    Ending,
}

impl Phase {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Beginning => 0,
            Self::PrecombatMain => 1,
            Self::Combat => 2,
            Self::PostcombatMain => 3,
            Self::Ending => 4,
        }
    }
}

/// Explicit turn step or main-phase segment in CR 500-514 order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Step {
    /// CR 502 untap step.
    Untap,
    /// CR 503 upkeep step.
    Upkeep,
    /// CR 504 draw step.
    Draw,
    /// CR 505 precombat main phase.
    PrecombatMain,
    /// CR 507 beginning of combat step.
    BeginningOfCombat,
    /// CR 508 declare attackers step.
    DeclareAttackers,
    /// CR 509 declare blockers step.
    DeclareBlockers,
    /// CR 510 combat damage step.
    CombatDamage,
    /// CR 511 end of combat step.
    EndOfCombat,
    /// CR 505 postcombat main phase.
    PostcombatMain,
    /// CR 513 end step.
    End,
    /// CR 514 cleanup step.
    Cleanup,
}

impl Step {
    /// Returns the CR 500.1 phase containing this step or main-phase segment.
    #[must_use]
    pub const fn phase(self) -> Phase {
        match self {
            Self::Untap | Self::Upkeep | Self::Draw => Phase::Beginning,
            Self::PrecombatMain => Phase::PrecombatMain,
            Self::BeginningOfCombat
            | Self::DeclareAttackers
            | Self::DeclareBlockers
            | Self::CombatDamage
            | Self::EndOfCombat => Phase::Combat,
            Self::PostcombatMain => Phase::PostcombatMain,
            Self::End | Self::Cleanup => Phase::Ending,
        }
    }

    /// Returns true for steps or main phases where CR 5 normally gives priority.
    ///
    /// Untap never gives priority, and cleanup gives priority only via the
    /// explicit CR 514.3a exception tracked by [`GameState`].
    #[must_use]
    pub const fn receives_priority_normally(self) -> bool {
        !matches!(self, Self::Untap | Self::Cleanup)
    }

    const fn canonical_code(self) -> u8 {
        match self {
            Self::Untap => 0,
            Self::Upkeep => 1,
            Self::Draw => 2,
            Self::PrecombatMain => 3,
            Self::BeginningOfCombat => 4,
            Self::DeclareAttackers => 5,
            Self::DeclareBlockers => 6,
            Self::CombatDamage => 7,
            Self::EndOfCombat => 8,
            Self::PostcombatMain => 9,
            Self::End => 10,
            Self::Cleanup => 11,
        }
    }
}

/// The unskipped CR 5 turn skeleton.
///
/// Runtime turn advancement may skip declare-blockers and combat-damage steps
/// when no attackers exist, and may repeat cleanup under CR 514.3a.
pub const NORMAL_TURN_STEPS: [Step; 12] = [
    Step::Untap,
    Step::Upkeep,
    Step::Draw,
    Step::PrecombatMain,
    Step::BeginningOfCombat,
    Step::DeclareAttackers,
    Step::DeclareBlockers,
    Step::CombatDamage,
    Step::EndOfCombat,
    Step::PostcombatMain,
    Step::End,
    Step::Cleanup,
];

/// Summary of the most recent cleanup step's turn-based actions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CleanupReport {
    discarded: u32,
    expired_until_end_of_turn: u32,
    expired_this_turn: u32,
}

impl CleanupReport {
    /// Returns how many objects were discarded to maximum hand size.
    #[must_use]
    pub const fn discarded(self) -> u32 {
        self.discarded
    }

    /// Returns how many placeholder "until end of turn" effects expired.
    #[must_use]
    pub const fn expired_until_end_of_turn(self) -> u32 {
        self.expired_until_end_of_turn
    }

    /// Returns how many placeholder "this turn" effects expired.
    #[must_use]
    pub const fn expired_this_turn(self) -> u32 {
        self.expired_this_turn
    }
}

/// Deterministic handle for a placeholder duration marker.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DurationMarkerId(u32);

impl DurationMarkerId {
    /// Returns the zero-based arena index for this duration marker.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Duration categories needed by the CR 500-514 turn machine.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EffectDuration {
    /// Expires as the named step or main-phase segment begins.
    UntilStepBegins(Step),
    /// Expires as the named phase ends.
    UntilPhaseEnds(Phase),
    /// Expires at the end of the combat phase, not at beginning of end combat.
    UntilEndOfCombat,
    /// Expires during cleanup under CR 514.2.
    UntilEndOfTurn,
    /// Expires during cleanup under CR 514.2.
    ThisTurn,
}

impl EffectDuration {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::UntilStepBegins(_) => 0,
            Self::UntilPhaseEnds(_) => 1,
            Self::UntilEndOfCombat => 2,
            Self::UntilEndOfTurn => 3,
            Self::ThisTurn => 4,
        }
    }
}

/// Placeholder for a future continuous effect with a CR 5 duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurationMarker {
    id: DurationMarkerId,
    duration: EffectDuration,
}

impl DurationMarker {
    /// Returns the stable marker ID.
    #[must_use]
    pub const fn id(self) -> DurationMarkerId {
        self.id
    }

    /// Returns the marker's current duration.
    #[must_use]
    pub const fn duration(self) -> EffectDuration {
        self.duration
    }
}

/// A stable handle for one spell or ability on the stack.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StackEntryId(u32);

impl StackEntryId {
    /// Returns the zero-based arena index for this stack entry.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic stack-entry value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The coarse kind of object represented by a stack entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StackObjectKind {
    /// Instant spell. T1.3 resolution moves it to its owner's graveyard.
    InstantSpell,
    /// Sorcery spell. T1.3 resolution moves it to its owner's graveyard.
    SorcerySpell,
    /// Permanent spell. T1.3 resolution moves it to the battlefield.
    PermanentSpell,
    /// Activated ability with no physical card object on the stack.
    ActivatedAbility,
    /// Triggered ability with no physical card object on the stack.
    TriggeredAbility,
}

impl StackObjectKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::InstantSpell => 0,
            Self::SorcerySpell => 1,
            Self::PermanentSpell => 2,
            Self::ActivatedAbility => 3,
            Self::TriggeredAbility => 4,
        }
    }
}

/// One spell or ability waiting on the stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StackEntry {
    id: StackEntryId,
    controller: PlayerId,
    object: Option<ObjectId>,
    kind: StackObjectKind,
}

impl StackEntry {
    /// Returns the stable stack-entry ID.
    #[must_use]
    pub const fn id(self) -> StackEntryId {
        self.id
    }

    /// Returns the controller of the spell or ability on the stack.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the physical object on the stack, if this entry is a spell.
    #[must_use]
    pub const fn object(self) -> Option<ObjectId> {
        self.object
    }

    /// Returns the coarse stack-object kind.
    #[must_use]
    pub const fn kind(self) -> StackObjectKind {
        self.kind
    }
}

/// Record of a stack object that resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolutionRecord {
    stack_entry: StackEntryId,
    controller: PlayerId,
    object: Option<ObjectId>,
    kind: StackObjectKind,
}

impl ResolutionRecord {
    /// Returns the stack-entry ID that resolved.
    #[must_use]
    pub const fn stack_entry(self) -> StackEntryId {
        self.stack_entry
    }

    /// Returns the controller of the resolved entry.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the physical object that resolved, if any.
    #[must_use]
    pub const fn object(self) -> Option<ObjectId> {
        self.object
    }

    /// Returns the resolved stack-object kind.
    #[must_use]
    pub const fn kind(self) -> StackObjectKind {
        self.kind
    }
}

/// Result of one priority pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriorityOutcome {
    /// Priority moved to the next player in turn order.
    PassedTo(PlayerId),
    /// All players passed and one stack entry resolved.
    Resolved(StackEntryId),
    /// All players passed with an empty stack, so the step or phase can end.
    StepComplete,
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
    /// A turn is already in progress.
    TurnAlreadyStarted,
    /// No turn is currently in progress.
    TurnNotStarted,
    /// Turn advancement requires at least one player.
    NoPlayers,
    /// The deterministic turn counter overflowed.
    TurnNumberOverflow,
    /// No player currently has priority.
    NoPriority,
    /// A player tried to act while another player had priority.
    PriorityPlayerMismatch {
        /// The player who currently has priority.
        expected: PlayerId,
        /// The player who tried to act.
        actual: PlayerId,
    },
    /// A stack resolution was requested while the stack was empty.
    EmptyStack,
    /// A stack entry refers to a spell object that is no longer on the stack.
    StackObjectNotOnStack(ObjectId),
}

/// Complete T1 game state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameState {
    seed: u64,
    turn_number: u32,
    active_player: Option<PlayerId>,
    priority_player: Option<PlayerId>,
    priority_pass_count: u32,
    current_step: Option<Step>,
    cleanup_iteration: u32,
    cleanup_priority_requested: bool,
    cleanup_repeat_pending: bool,
    attackers_declared_this_combat: bool,
    last_cleanup_report: CleanupReport,
    players: Vec<PlayerState>,
    objects: ObjectArena,
    zones: Vec<Zone>,
    next_duration_marker: u32,
    duration_markers: Vec<DurationMarker>,
    next_stack_entry: u32,
    stack_entries: Vec<StackEntry>,
    resolution_log: Vec<ResolutionRecord>,
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
            priority_pass_count: 0,
            current_step: None,
            cleanup_iteration: 0,
            cleanup_priority_requested: false,
            cleanup_repeat_pending: false,
            attackers_declared_this_combat: false,
            last_cleanup_report: CleanupReport::default(),
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
            next_duration_marker: 0,
            duration_markers: Vec::new(),
            next_stack_entry: 0,
            stack_entries: Vec::new(),
            resolution_log: Vec::new(),
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

    /// Returns the number of consecutive priority passes since the last action.
    #[must_use]
    pub const fn priority_pass_count(&self) -> u32 {
        self.priority_pass_count
    }

    /// Returns the current step or main-phase segment, if a turn has started.
    #[must_use]
    pub const fn current_step(&self) -> Option<Step> {
        self.current_step
    }

    /// Returns the current phase, if a turn has started.
    #[must_use]
    pub const fn current_phase(&self) -> Option<Phase> {
        match self.current_step {
            Some(step) => Some(step.phase()),
            None => None,
        }
    }

    /// Returns how many cleanup steps have begun in the current turn.
    #[must_use]
    pub const fn cleanup_iteration(&self) -> u32 {
        self.cleanup_iteration
    }

    /// Returns the most recent cleanup action summary.
    #[must_use]
    pub const fn last_cleanup_report(&self) -> CleanupReport {
        self.last_cleanup_report
    }

    /// Returns true when the current segment currently has a priority window.
    #[must_use]
    pub const fn has_priority_window(&self) -> bool {
        self.priority_player.is_some()
    }

    /// Returns active stack entries in bottom-to-top order.
    #[must_use]
    pub fn stack_entries(&self) -> &[StackEntry] {
        &self.stack_entries
    }

    /// Returns the current top stack entry.
    #[must_use]
    pub fn stack_top(&self) -> Option<StackEntry> {
        self.stack_entries.last().copied()
    }

    /// Returns resolved stack entries in resolution order.
    #[must_use]
    pub fn resolution_log(&self) -> &[ResolutionRecord] {
        &self.resolution_log
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

    /// Sets a player's maximum hand size.
    pub fn set_player_max_hand_size(
        &mut self,
        player: PlayerId,
        max_hand_size: u32,
    ) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.max_hand_size = max_hand_size;
        Ok(())
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

    /// Starts a turn for the chosen active player at the untap step.
    pub fn start_turn(&mut self, active_player: PlayerId) -> Result<(), StateError> {
        self.require_player(active_player)?;
        if self.current_step.is_some() {
            return Err(StateError::TurnAlreadyStarted);
        }
        self.active_player = Some(active_player);
        self.priority_player = None;
        self.priority_pass_count = 0;
        self.turn_number = self
            .turn_number
            .checked_add(1)
            .ok_or(StateError::TurnNumberOverflow)?;
        self.cleanup_iteration = 0;
        self.attackers_declared_this_combat = false;
        self.begin_step(Step::Untap)
    }

    /// Advances from the current step or main-phase segment to the next one.
    ///
    /// This remains available for no-priority steps and tests. Steps with a
    /// priority window should usually end through [`Self::pass_priority`],
    /// because CR 117.4 requires all players to pass in succession.
    pub fn advance_step(&mut self) -> Result<Step, StateError> {
        self.advance_step_after_empty_stack()
    }

    /// Passes priority for the current priority player.
    ///
    /// If all players pass in succession, this either resolves the top stack
    /// entry or completes the current step when the stack is empty.
    pub fn pass_priority(&mut self, player: PlayerId) -> Result<PriorityOutcome, StateError> {
        let priority_player = self.priority_player.ok_or(StateError::NoPriority)?;
        if priority_player != player {
            return Err(StateError::PriorityPlayerMismatch {
                expected: priority_player,
                actual: player,
            });
        }
        self.priority_pass_count = self.priority_pass_count.saturating_add(1);
        if self.priority_pass_count < self.players.len() as u32 {
            let next = self.next_player_after(player)?;
            self.priority_player = Some(next);
            return Ok(PriorityOutcome::PassedTo(next));
        }

        self.priority_pass_count = 0;
        if self.stack_entries.is_empty() {
            self.advance_step_after_empty_stack()?;
            Ok(PriorityOutcome::StepComplete)
        } else {
            let resolved = self.resolve_top_stack_entry()?;
            self.grant_priority_after_resolution();
            Ok(PriorityOutcome::Resolved(resolved))
        }
    }

    /// Puts a spell object on the stack for the current priority player.
    ///
    /// When `hold_priority` is true, priority remains with the caster as an
    /// explicit full-control choice. T1.3 keeps the same result either way
    /// because CR 117.3c gives priority back after a spell is cast.
    pub fn put_spell_on_stack(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        kind: StackObjectKind,
        hold_priority: bool,
    ) -> Result<StackEntryId, StateError> {
        self.require_priority_player(player)?;
        self.require_player(player)?;
        if self.objects.get(object).is_none() {
            return Err(StateError::UnknownObject(object));
        }
        let stack_zone = ZoneId::new(None, ZoneKind::Stack);
        if self.object_zone(object) != Some(stack_zone) {
            self.move_object(object, stack_zone)?;
        }
        let id = self.push_stack_entry(player, Some(object), kind);
        self.after_priority_action(player, hold_priority);
        Ok(id)
    }

    /// Puts an ability on top of the stack for the current priority player.
    pub fn put_ability_on_stack(
        &mut self,
        player: PlayerId,
        kind: StackObjectKind,
        hold_priority: bool,
    ) -> Result<StackEntryId, StateError> {
        self.require_priority_player(player)?;
        self.require_player(player)?;
        let id = self.push_stack_entry(player, None, kind);
        self.after_priority_action(player, hold_priority);
        Ok(id)
    }

    /// Puts simultaneous triggered abilities on the stack in APNAP order.
    ///
    /// Entries controlled by the active player are placed lowest, followed by
    /// nonactive players in turn order. Within one controller's entries, the
    /// provided order is preserved.
    pub fn put_simultaneous_abilities_apnap(
        &mut self,
        abilities: &[PlayerId],
        kind: StackObjectKind,
    ) -> Result<Vec<StackEntryId>, StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        for player in abilities {
            self.require_player(*player)?;
        }
        let mut ids = Vec::with_capacity(abilities.len());
        for player in self.apnap_players(active)? {
            for ability_controller in abilities {
                if *ability_controller == player {
                    ids.push(self.push_stack_entry(player, None, kind));
                }
            }
        }
        self.priority_pass_count = 0;
        Ok(ids)
    }

    fn advance_step_after_empty_stack(&mut self) -> Result<Step, StateError> {
        let current = self.current_step.ok_or(StateError::TurnNotStarted)?;
        self.end_step(current);
        let next = match current {
            Step::Cleanup if self.cleanup_repeat_pending => {
                self.cleanup_repeat_pending = false;
                Step::Cleanup
            }
            Step::Cleanup => return self.begin_next_turn(),
            Step::DeclareAttackers if !self.attackers_declared_this_combat => Step::EndOfCombat,
            step => Self::next_normal_step(step),
        };
        self.begin_step(next)?;
        Ok(next)
    }

    /// Records whether the current combat has at least one attacker.
    ///
    /// This is the T1.2 hook for CR 508.8. Full attack declaration replaces it
    /// in T1.6, but keeping the flag here makes the step machine testable now.
    pub fn set_attackers_declared_this_combat(&mut self, attackers_declared: bool) {
        self.attackers_declared_this_combat = attackers_declared;
    }

    /// Requests the CR 514.3a cleanup exception after cleanup actions finish.
    pub fn request_cleanup_priority(&mut self) {
        self.cleanup_priority_requested = true;
    }

    /// Adds a placeholder duration marker.
    pub fn add_duration_marker(&mut self, duration: EffectDuration) -> DurationMarkerId {
        let id = DurationMarkerId(self.next_duration_marker);
        self.next_duration_marker = self.next_duration_marker.saturating_add(1);
        self.duration_markers.push(DurationMarker { id, duration });
        id
    }

    /// Returns active duration markers in deterministic arena order.
    #[must_use]
    pub fn duration_markers(&self) -> &[DurationMarker] {
        &self.duration_markers
    }

    /// Counts active duration markers matching one duration exactly.
    #[must_use]
    pub fn duration_marker_count(&self, duration: EffectDuration) -> usize {
        self.duration_markers
            .iter()
            .filter(|marker| marker.duration == duration)
            .count()
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
        bytes.write_u32(self.priority_pass_count);
        bytes.write_optional_step(self.current_step);
        bytes.write_u32(self.cleanup_iteration);
        bytes.write_bool(self.cleanup_priority_requested);
        bytes.write_bool(self.cleanup_repeat_pending);
        bytes.write_bool(self.attackers_declared_this_combat);
        bytes.write_cleanup_report(self.last_cleanup_report);
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

        bytes.write_u32(self.next_duration_marker);
        bytes.write_u32(self.duration_markers.len() as u32);
        for marker in &self.duration_markers {
            bytes.write_u32(marker.id.0);
            bytes.write_effect_duration(marker.duration);
        }
        bytes.write_u32(self.next_stack_entry);
        bytes.write_u32(self.stack_entries.len() as u32);
        for entry in &self.stack_entries {
            bytes.write_stack_entry(*entry);
        }
        bytes.write_u32(self.resolution_log.len() as u32);
        for resolution in &self.resolution_log {
            bytes.write_resolution_record(*resolution);
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
        hash.write_u32(self.priority_pass_count);
        hash.write_optional_step(self.current_step);
        hash.write_u32(self.cleanup_iteration);
        hash.write_bool(self.cleanup_priority_requested);
        hash.write_bool(self.cleanup_repeat_pending);
        hash.write_bool(self.attackers_declared_this_combat);
        hash.write_cleanup_report(self.last_cleanup_report);
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

        hash.write_u32(self.next_duration_marker);
        hash.write_u32(self.duration_markers.len() as u32);
        for marker in &self.duration_markers {
            hash.write_u32(marker.id.0);
            hash.write_effect_duration(marker.duration);
        }

        hash.write_u32(self.next_stack_entry);
        hash.write_u32(self.stack_entries.len() as u32);
        for entry in &self.stack_entries {
            hash.write_stack_entry(*entry);
        }
        hash.write_u32(self.resolution_log.len() as u32);
        for resolution in &self.resolution_log {
            hash.write_resolution_record(*resolution);
        }

        StateHash(hash.finish())
    }

    fn begin_step(&mut self, step: Step) -> Result<(), StateError> {
        self.current_step = Some(step);
        self.expire_step_begin_markers(step);
        match step {
            Step::Untap => self.priority_player = None,
            Step::Draw => {
                self.draw_turn_card()?;
                self.assign_normal_priority(step);
            }
            Step::BeginningOfCombat => {
                self.attackers_declared_this_combat = false;
                self.assign_normal_priority(step);
            }
            Step::Cleanup => self.begin_cleanup_step()?,
            _ => self.assign_normal_priority(step),
        }
        Ok(())
    }

    fn end_step(&mut self, step: Step) {
        self.priority_player = None;
        self.priority_pass_count = 0;
        if step == Step::EndOfCombat {
            self.expire_end_of_combat_markers();
        }
        if self.phase_ends_after_step(step) {
            self.expire_phase_end_markers(step.phase());
        }
    }

    fn begin_next_turn(&mut self) -> Result<Step, StateError> {
        let current_active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let next_active = self.next_player_after(current_active)?;
        self.active_player = Some(next_active);
        self.turn_number = self
            .turn_number
            .checked_add(1)
            .ok_or(StateError::TurnNumberOverflow)?;
        self.cleanup_iteration = 0;
        self.attackers_declared_this_combat = false;
        self.begin_step(Step::Untap)?;
        Ok(Step::Untap)
    }

    const fn next_normal_step(step: Step) -> Step {
        match step {
            Step::Untap => Step::Upkeep,
            Step::Upkeep => Step::Draw,
            Step::Draw => Step::PrecombatMain,
            Step::PrecombatMain => Step::BeginningOfCombat,
            Step::BeginningOfCombat => Step::DeclareAttackers,
            Step::DeclareAttackers => Step::DeclareBlockers,
            Step::DeclareBlockers => Step::CombatDamage,
            Step::CombatDamage => Step::EndOfCombat,
            Step::EndOfCombat => Step::PostcombatMain,
            Step::PostcombatMain => Step::End,
            Step::End => Step::Cleanup,
            Step::Cleanup => Step::Untap,
        }
    }

    const fn phase_ends_after_step(&self, step: Step) -> bool {
        matches!(
            step,
            Step::Draw | Step::PrecombatMain | Step::EndOfCombat | Step::PostcombatMain
        ) || (matches!(step, Step::Cleanup) && !self.cleanup_repeat_pending)
    }

    fn assign_normal_priority(&mut self, step: Step) {
        self.priority_player = if step.receives_priority_normally() {
            self.active_player
        } else {
            None
        };
        self.priority_pass_count = 0;
    }

    fn begin_cleanup_step(&mut self) -> Result<(), StateError> {
        self.cleanup_iteration = self.cleanup_iteration.saturating_add(1);
        self.last_cleanup_report = self.perform_cleanup_actions()?;
        let grant_priority = self.cleanup_priority_requested;
        self.cleanup_priority_requested = false;
        self.cleanup_repeat_pending = grant_priority;
        self.priority_player = if grant_priority {
            self.active_player
        } else {
            None
        };
        self.priority_pass_count = 0;
        Ok(())
    }

    fn require_priority_player(&self, player: PlayerId) -> Result<(), StateError> {
        let priority_player = self.priority_player.ok_or(StateError::NoPriority)?;
        if priority_player == player {
            Ok(())
        } else {
            Err(StateError::PriorityPlayerMismatch {
                expected: priority_player,
                actual: player,
            })
        }
    }

    fn after_priority_action(&mut self, player: PlayerId, _hold_priority: bool) {
        self.priority_player = Some(player);
        self.priority_pass_count = 0;
    }

    fn push_stack_entry(
        &mut self,
        controller: PlayerId,
        object: Option<ObjectId>,
        kind: StackObjectKind,
    ) -> StackEntryId {
        let id = StackEntryId(self.next_stack_entry);
        self.next_stack_entry = self.next_stack_entry.saturating_add(1);
        self.stack_entries.push(StackEntry {
            id,
            controller,
            object,
            kind,
        });
        id
    }

    fn resolve_top_stack_entry(&mut self) -> Result<StackEntryId, StateError> {
        let entry = self.stack_entries.pop().ok_or(StateError::EmptyStack)?;
        if let Some(object) = entry.object {
            if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Stack)) {
                return Err(StateError::StackObjectNotOnStack(object));
            }
            let destination = match entry.kind {
                StackObjectKind::InstantSpell | StackObjectKind::SorcerySpell => {
                    let owner = self
                        .objects
                        .get(object)
                        .ok_or(StateError::UnknownObject(object))?
                        .owner();
                    ZoneId::new(Some(owner), ZoneKind::Graveyard)
                }
                StackObjectKind::PermanentSpell => ZoneId::new(None, ZoneKind::Battlefield),
                StackObjectKind::ActivatedAbility | StackObjectKind::TriggeredAbility => {
                    ZoneId::new(None, ZoneKind::Stack)
                }
            };
            if destination != ZoneId::new(None, ZoneKind::Stack) {
                self.move_object(object, destination)?;
            }
        }
        self.resolution_log.push(ResolutionRecord {
            stack_entry: entry.id,
            controller: entry.controller,
            object: entry.object,
            kind: entry.kind,
        });
        Ok(entry.id)
    }

    fn grant_priority_after_resolution(&mut self) {
        self.priority_player = self.active_player;
        self.priority_pass_count = 0;
    }

    fn perform_cleanup_actions(&mut self) -> Result<CleanupReport, StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let discarded = self.discard_to_max_hand_size(active)?;
        let expired_until_end_of_turn =
            self.expire_duration_markers(EffectDuration::UntilEndOfTurn) as u32;
        let expired_this_turn = self.expire_duration_markers(EffectDuration::ThisTurn) as u32;
        Ok(CleanupReport {
            discarded,
            expired_until_end_of_turn,
            expired_this_turn,
        })
    }

    fn discard_to_max_hand_size(&mut self, player: PlayerId) -> Result<u32, StateError> {
        let max_hand_size = self
            .players
            .get(player.index())
            .ok_or(StateError::UnknownPlayer(player))?
            .max_hand_size as usize;
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let graveyard = ZoneId::new(Some(player), ZoneKind::Graveyard);
        let mut discarded = 0;
        while self.zone_len(hand)? > max_hand_size {
            if self.move_last_between_zones(hand, graveyard)?.is_some() {
                discarded += 1;
            } else {
                break;
            }
        }
        Ok(discarded)
    }

    fn draw_turn_card(&mut self) -> Result<(), StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let library = ZoneId::new(Some(active), ZoneKind::Library);
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let _ = self.move_last_between_zones(library, hand)?;
        Ok(())
    }

    fn move_last_between_zones(
        &mut self,
        from: ZoneId,
        to: ZoneId,
    ) -> Result<Option<ObjectId>, StateError> {
        self.require_zone(from)?;
        self.require_zone(to)?;
        let from_index = self.zone_index(from).ok_or(StateError::UnknownZone(from))?;
        let Some(object) = self.zones[from_index].objects.pop() else {
            return Ok(None);
        };
        let to_index = self.zone_index(to).ok_or(StateError::UnknownZone(to))?;
        self.zones[to_index].objects.push(object);
        Ok(Some(object))
    }

    fn zone_len(&self, id: ZoneId) -> Result<usize, StateError> {
        self.require_zone(id)?;
        let index = self.zone_index(id).ok_or(StateError::UnknownZone(id))?;
        Ok(self.zones[index].objects.len())
    }

    fn next_player_after(&self, player: PlayerId) -> Result<PlayerId, StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        self.require_player(player)?;
        let next_index = (player.index() + 1) % self.players.len();
        Ok(PlayerId(next_index as u32))
    }

    fn apnap_players(&self, active: PlayerId) -> Result<Vec<PlayerId>, StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        self.require_player(active)?;
        let mut order = Vec::with_capacity(self.players.len());
        let mut player = active;
        for _ in 0..self.players.len() {
            order.push(player);
            player = self.next_player_after(player)?;
        }
        Ok(order)
    }

    fn expire_step_begin_markers(&mut self, step: Step) {
        self.duration_markers.retain(|marker| {
            !matches!(
                marker.duration,
                EffectDuration::UntilStepBegins(marker_step) if marker_step == step
            )
        });
    }

    fn expire_phase_end_markers(&mut self, phase: Phase) {
        self.duration_markers.retain(|marker| {
            !matches!(
                marker.duration,
                EffectDuration::UntilPhaseEnds(marker_phase) if marker_phase == phase
            )
        });
    }

    fn expire_end_of_combat_markers(&mut self) {
        self.duration_markers
            .retain(|marker| marker.duration != EffectDuration::UntilEndOfCombat);
    }

    fn expire_duration_markers(&mut self, duration: EffectDuration) -> usize {
        let before = self.duration_markers.len();
        self.duration_markers
            .retain(|marker| marker.duration != duration);
        before - self.duration_markers.len()
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

    fn write_optional_step(&mut self, step: Option<Step>) {
        match step {
            Some(step) => {
                self.write_u8(1);
                self.write_u8(step.canonical_code());
            }
            None => self.write_u8(0),
        }
    }

    fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    fn write_cleanup_report(&mut self, report: CleanupReport) {
        self.write_u32(report.discarded);
        self.write_u32(report.expired_until_end_of_turn);
        self.write_u32(report.expired_this_turn);
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

    fn write_effect_duration(&mut self, duration: EffectDuration) {
        self.write_u8(duration.canonical_code());
        match duration {
            EffectDuration::UntilStepBegins(step) => self.write_u8(step.canonical_code()),
            EffectDuration::UntilPhaseEnds(phase) => self.write_u8(phase.canonical_code()),
            EffectDuration::UntilEndOfCombat
            | EffectDuration::UntilEndOfTurn
            | EffectDuration::ThisTurn => {}
        }
    }

    fn write_optional_object(&mut self, object: Option<ObjectId>) {
        match object {
            Some(object) => {
                self.write_u8(1);
                self.write_u32(object.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_stack_entry(&mut self, entry: StackEntry) {
        self.write_u32(entry.id.0);
        self.write_u32(entry.controller.0);
        self.write_optional_object(entry.object);
        self.write_u8(entry.kind.canonical_code());
    }

    fn write_resolution_record(&mut self, record: ResolutionRecord) {
        self.write_u32(record.stack_entry.0);
        self.write_u32(record.controller.0);
        self.write_optional_object(record.object);
        self.write_u8(record.kind.canonical_code());
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

    fn write_optional_step(&mut self, step: Option<Step>) {
        match step {
            Some(step) => {
                self.write_u8(1);
                self.write_u8(step.canonical_code());
            }
            None => self.write_u8(0),
        }
    }

    fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    fn write_cleanup_report(&mut self, report: CleanupReport) {
        self.write_u32(report.discarded);
        self.write_u32(report.expired_until_end_of_turn);
        self.write_u32(report.expired_this_turn);
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

    fn write_effect_duration(&mut self, duration: EffectDuration) {
        self.write_u8(duration.canonical_code());
        match duration {
            EffectDuration::UntilStepBegins(step) => self.write_u8(step.canonical_code()),
            EffectDuration::UntilPhaseEnds(phase) => self.write_u8(phase.canonical_code()),
            EffectDuration::UntilEndOfCombat
            | EffectDuration::UntilEndOfTurn
            | EffectDuration::ThisTurn => {}
        }
    }

    fn write_optional_object(&mut self, object: Option<ObjectId>) {
        match object {
            Some(object) => {
                self.write_u8(1);
                self.write_u32(object.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_stack_entry(&mut self, entry: StackEntry) {
        self.write_u32(entry.id.0);
        self.write_u32(entry.controller.0);
        self.write_optional_object(entry.object);
        self.write_u8(entry.kind.canonical_code());
    }

    fn write_resolution_record(&mut self, record: ResolutionRecord) {
        self.write_u32(record.stack_entry.0);
        self.write_u32(record.controller.0);
        self.write_optional_object(record.object);
        self.write_u8(record.kind.canonical_code());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        crate_ready, CardId, EffectDuration, GameState, Phase, PlayerId, PriorityOutcome,
        StackEntryId, StackObjectKind, StateError, Step, ZoneConservation, ZoneId, ZoneKind,
        NORMAL_TURN_STEPS,
    };

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

    #[test]
    fn normal_turn_steps_match_cr5_skeleton() {
        assert_eq!(
            NORMAL_TURN_STEPS,
            [
                Step::Untap,
                Step::Upkeep,
                Step::Draw,
                Step::PrecombatMain,
                Step::BeginningOfCombat,
                Step::DeclareAttackers,
                Step::DeclareBlockers,
                Step::CombatDamage,
                Step::EndOfCombat,
                Step::PostcombatMain,
                Step::End,
                Step::Cleanup,
            ]
        );
        assert_eq!(Step::Untap.phase(), Phase::Beginning);
        assert_eq!(Step::PrecombatMain.phase(), Phase::PrecombatMain);
        assert_eq!(Step::EndOfCombat.phase(), Phase::Combat);
        assert_eq!(Step::Cleanup.phase(), Phase::Ending);
    }

    #[test]
    fn untap_has_no_priority_and_upkeep_assigns_active_priority() {
        let mut state = GameState::new();
        let active = state.add_player();

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        assert_eq!(state.turn_number(), 1);
        assert_eq!(state.current_step(), Some(Step::Untap));
        assert_eq!(state.current_phase(), Some(Phase::Beginning));
        assert_eq!(state.priority_player(), None);
        assert!(!state.has_priority_window());

        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        assert_eq!(state.current_step(), Some(Step::Upkeep));
        assert_eq!(state.priority_player(), Some(active));
        assert!(state.has_priority_window());
    }

    #[test]
    fn draw_step_draws_before_active_player_gets_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let library = ZoneId::new(Some(active), ZoneKind::Library);
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let object = state
            .create_object(CardId::new(9), active, active, library)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected upkeep advance error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected draw advance error: {error:?}"));

        assert_eq!(state.current_step(), Some(Step::Draw));
        assert_eq!(state.priority_player(), Some(active));
        assert_eq!(
            state
                .zone(hand)
                .unwrap_or_else(|| panic!("hand zone missing"))
                .objects(),
            &[object]
        );
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects(),
            &[]
        );
    }

    #[test]
    fn combat_without_attackers_skips_blockers_and_damage() {
        let mut state = GameState::new();
        let active = state.add_player();
        state.add_player();

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        for expected in [
            Step::Upkeep,
            Step::Draw,
            Step::PrecombatMain,
            Step::BeginningOfCombat,
            Step::DeclareAttackers,
        ] {
            let step = state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
            assert_eq!(step, expected);
        }

        let next = state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected skip advance error: {error:?}"));
        assert_eq!(next, Step::EndOfCombat);
        assert_eq!(state.current_phase(), Some(Phase::Combat));
    }

    #[test]
    fn combat_with_attackers_visits_blockers_and_damage() {
        let mut state = GameState::new();
        let active = state.add_player();
        state.add_player();

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        for expected in [
            Step::Upkeep,
            Step::Draw,
            Step::PrecombatMain,
            Step::BeginningOfCombat,
            Step::DeclareAttackers,
        ] {
            let step = state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
            assert_eq!(step, expected);
        }

        state.set_attackers_declared_this_combat(true);
        assert_eq!(
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}")),
            Step::DeclareBlockers
        );
        assert_eq!(
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}")),
            Step::CombatDamage
        );
    }

    #[test]
    fn end_of_turn_durations_survive_end_step_and_expire_during_cleanup() {
        let mut state = GameState::new();
        let active = state.add_player();
        state.add_duration_marker(EffectDuration::UntilEndOfTurn);
        state.add_duration_marker(EffectDuration::ThisTurn);

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        while state.current_step() != Some(Step::End) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }
        assert_eq!(
            state.duration_marker_count(EffectDuration::UntilEndOfTurn),
            1
        );
        assert_eq!(state.duration_marker_count(EffectDuration::ThisTurn), 1);

        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected cleanup advance error: {error:?}"));
        assert_eq!(state.current_step(), Some(Step::Cleanup));
        assert_eq!(
            state.duration_marker_count(EffectDuration::UntilEndOfTurn),
            0
        );
        assert_eq!(state.duration_marker_count(EffectDuration::ThisTurn), 0);
        assert_eq!(state.last_cleanup_report().expired_until_end_of_turn(), 1);
        assert_eq!(state.last_cleanup_report().expired_this_turn(), 1);
    }

    #[test]
    fn cleanup_discards_to_max_hand_size_before_next_turn() {
        let mut state = GameState::new();
        let active = state.add_player();
        let next = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        state
            .set_player_max_hand_size(active, 2)
            .unwrap_or_else(|error| panic!("unexpected max hand size error: {error:?}"));
        for card in 0..5 {
            state
                .create_object(CardId::new(card), active, active, hand)
                .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        }
        state.add_duration_marker(EffectDuration::UntilEndOfTurn);

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        while state.current_step() != Some(Step::Cleanup) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }

        assert_eq!(state.last_cleanup_report().discarded(), 3);
        assert_eq!(state.last_cleanup_report().expired_until_end_of_turn(), 1);
        assert_eq!(
            state
                .zone(hand)
                .unwrap_or_else(|| panic!("hand zone missing"))
                .objects()
                .len(),
            2
        );
        assert_eq!(
            state
                .zone(graveyard)
                .unwrap_or_else(|| panic!("graveyard zone missing"))
                .objects()
                .len(),
            3
        );
        assert_eq!(state.priority_player(), None);

        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected next turn advance error: {error:?}"));
        assert_eq!(state.current_step(), Some(Step::Untap));
        assert_eq!(state.turn_number(), 2);
        assert_eq!(state.active_player(), Some(next));
    }

    #[test]
    fn cleanup_priority_exception_repeats_cleanup_step() {
        let mut state = GameState::new();
        let active = state.add_player();
        state.add_player();
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state.request_cleanup_priority();
        while state.current_step() != Some(Step::Cleanup) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }

        assert_eq!(state.current_step(), Some(Step::Cleanup));
        assert_eq!(state.cleanup_iteration(), 1);
        assert_eq!(state.priority_player(), Some(active));

        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected repeated cleanup advance error: {error:?}"));
        assert_eq!(state.current_step(), Some(Step::Cleanup));
        assert_eq!(state.cleanup_iteration(), 2);
        assert_eq!(state.priority_player(), None);
    }

    #[test]
    fn priority_starts_with_active_then_passes_in_turn_order() {
        let mut state = GameState::new();
        let first = state.add_player();
        let active = state.add_player();
        let third = state.add_player();
        start_upkeep(&mut state, active);

        assert_eq!(state.priority_player(), Some(active));
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(third))
        );
        assert_eq!(
            state.pass_priority(third),
            Ok(PriorityOutcome::PassedTo(first))
        );
    }

    #[test]
    fn adding_stack_object_holds_priority_and_resets_passes() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        start_upkeep(&mut state, active);
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(responder))
        );
        assert_eq!(state.priority_pass_count(), 1);

        let ability = state
            .put_ability_on_stack(responder, StackObjectKind::ActivatedAbility, true)
            .unwrap_or_else(|error| panic!("unexpected ability error: {error:?}"));

        assert_eq!(state.priority_player(), Some(responder));
        assert_eq!(state.priority_pass_count(), 0);
        assert_eq!(state.stack_top().map(|entry| entry.id()), Some(ability));
    }

    #[test]
    fn passes_in_succession_on_empty_stack_advance_step() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        start_upkeep(&mut state, active);

        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(responder))
        );
        assert_eq!(
            state.pass_priority(responder),
            Ok(PriorityOutcome::StepComplete)
        );
        assert_eq!(state.current_step(), Some(Step::Draw));
        assert_eq!(state.priority_player(), Some(active));
    }

    #[test]
    fn intervening_action_breaks_pass_succession() {
        let mut state = GameState::new();
        let active = state.add_player();
        let second = state.add_player();
        let third = state.add_player();
        start_upkeep(&mut state, active);

        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(second))
        );
        assert_eq!(
            state.pass_priority(second),
            Ok(PriorityOutcome::PassedTo(third))
        );
        let ability = state
            .put_ability_on_stack(third, StackObjectKind::TriggeredAbility, true)
            .unwrap_or_else(|error| panic!("unexpected ability error: {error:?}"));
        assert_eq!(state.priority_pass_count(), 0);

        assert_eq!(
            state.pass_priority(third),
            Ok(PriorityOutcome::PassedTo(active))
        );
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(second))
        );
        assert_eq!(
            state.pass_priority(second),
            Ok(PriorityOutcome::Resolved(ability))
        );
        assert_eq!(state.resolution_log()[0].stack_entry(), ability);
    }

    #[test]
    fn full_pass_round_resolves_only_one_object() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        let first = state
            .create_object(CardId::new(21), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let second = state
            .create_object(CardId::new(22), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        start_upkeep(&mut state, active);
        let first_entry = state
            .put_spell_on_stack(active, first, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected stack error: {error:?}"));
        let second_entry = state
            .put_spell_on_stack(active, second, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected stack error: {error:?}"));

        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(responder))
        );
        assert_eq!(
            state.pass_priority(responder),
            Ok(PriorityOutcome::Resolved(second_entry))
        );

        assert_eq!(state.stack_entries().len(), 1);
        assert_eq!(state.stack_top().map(|entry| entry.id()), Some(first_entry));
        assert_eq!(state.resolution_log().len(), 1);
        assert_eq!(state.resolution_log()[0].stack_entry(), second_entry);
        assert_eq!(state.object_zone(second), Some(graveyard));
        assert_eq!(state.priority_player(), Some(active));
    }

    #[test]
    fn resolved_object_controller_does_not_receive_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let active_hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let responder_hand = ZoneId::new(Some(responder), ZoneKind::Hand);
        let active_spell = state
            .create_object(CardId::new(30), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let response = state
            .create_object(CardId::new(31), responder, responder, responder_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        start_upkeep(&mut state, active);
        state
            .put_spell_on_stack(active, active_spell, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected stack error: {error:?}"));
        state
            .pass_priority(active)
            .unwrap_or_else(|error| panic!("unexpected pass error: {error:?}"));
        let response_entry = state
            .put_spell_on_stack(responder, response, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected response error: {error:?}"));

        state
            .pass_priority(responder)
            .unwrap_or_else(|error| panic!("unexpected responder pass error: {error:?}"));
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::Resolved(response_entry))
        );
        assert_eq!(state.priority_player(), Some(active));
    }

    #[test]
    fn three_instant_response_chain_resolves_lifo() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let active_hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let responder_hand = ZoneId::new(Some(responder), ZoneKind::Hand);
        let first = state
            .create_object(CardId::new(40), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let second = state
            .create_object(CardId::new(41), responder, responder, responder_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let third = state
            .create_object(CardId::new(42), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        start_upkeep(&mut state, active);
        let first_entry = state
            .put_spell_on_stack(active, first, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected first stack error: {error:?}"));
        state
            .pass_priority(active)
            .unwrap_or_else(|error| panic!("unexpected pass error: {error:?}"));
        let second_entry = state
            .put_spell_on_stack(responder, second, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected second stack error: {error:?}"));
        state
            .pass_priority(responder)
            .unwrap_or_else(|error| panic!("unexpected pass error: {error:?}"));
        let third_entry = state
            .put_spell_on_stack(active, third, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected third stack error: {error:?}"));

        pass_round(&mut state, active, responder, third_entry);
        pass_round(&mut state, active, responder, second_entry);
        pass_round(&mut state, active, responder, first_entry);

        let resolved: Vec<StackEntryId> = state
            .resolution_log()
            .iter()
            .map(|record| record.stack_entry())
            .collect();
        assert_eq!(resolved, vec![third_entry, second_entry, first_entry]);
        assert!(state.stack_entries().is_empty());
    }

    #[test]
    fn simultaneous_stack_objects_use_apnap_low_to_high() {
        let mut state = GameState::new();
        let first = state.add_player();
        let active = state.add_player();
        let third = state.add_player();
        start_upkeep(&mut state, active);

        let ids = state
            .put_simultaneous_abilities_apnap(
                &[active, third, first],
                StackObjectKind::TriggeredAbility,
            )
            .unwrap_or_else(|error| panic!("unexpected APNAP stack error: {error:?}"));

        let controllers: Vec<PlayerId> = state
            .stack_entries()
            .iter()
            .map(|entry| entry.controller())
            .collect();
        assert_eq!(controllers, vec![active, third, first]);
        assert_eq!(ids.len(), 3);
        assert_eq!(
            state.stack_top().map(|entry| entry.controller()),
            Some(first)
        );
    }

    fn start_upkeep(state: &mut GameState, active: PlayerId) {
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected upkeep advance error: {error:?}"));
    }

    fn pass_round(
        state: &mut GameState,
        active: PlayerId,
        responder: PlayerId,
        expected: super::StackEntryId,
    ) {
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(responder))
        );
        assert_eq!(
            state.pass_priority(responder),
            Ok(PriorityOutcome::Resolved(expected))
        );
        assert_eq!(state.priority_player(), Some(active));
    }
}
