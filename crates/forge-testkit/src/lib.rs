#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Scenario, oracle, and invariant testing crate for Forge 2.0.
//!
//! T1.9 introduces a small RON-compatible scenario surface for kernel tests.
//! The runner intentionally executes through [`forge_core::apply`] so scenario
//! tests exercise the same public mutation boundary as application code.

use forge_core::{
    apply, Action, CardId, GameOutcome, GameState, ObjectId, Outcome, PlayerId, StateHash, ZoneId,
    ZoneKind,
};
use std::{fs, path::Path};

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

/// Parses and runs one RON-compatible scenario.
pub fn run_scenario_ron(input: &str) -> Result<ScenarioReport, ScenarioError> {
    let scenario = parse_scenario_ron(input)?;
    Ok(run_scenario(&scenario))
}

/// Reads, parses, and runs one RON-compatible scenario file.
pub fn run_scenario_file(path: impl AsRef<Path>) -> Result<ScenarioReport, ScenarioError> {
    let path = path.as_ref();
    let input = fs::read_to_string(path).map_err(|error| {
        ScenarioError::schema(format!("failed to read {}: {error}", path.display()))
    })?;
    run_scenario_ron(&input)
}

/// Creates a failed report for infrastructure errors such as parse failures.
#[must_use]
pub fn failed_report(
    name: impl Into<String>,
    phase: impl Into<String>,
    message: impl Into<String>,
) -> ScenarioReport {
    ScenarioReport {
        name: name.into(),
        steps: Vec::new(),
        failures: vec![ScenarioFailure::new(phase, message)],
        final_hash: None,
    }
}

/// Serializes multiple reports as one JUnit-style XML testsuite.
#[must_use]
pub fn reports_to_junit_xml(reports: &[ScenarioReport]) -> String {
    let failure_count = reports.iter().filter(|report| !report.passed()).count();
    let mut xml = format!(
        "<testsuite name=\"forge-testkit\" tests=\"{}\" failures=\"{}\">",
        reports.len(),
        failure_count
    );
    for report in reports {
        xml.push_str(&format!(
            "<testcase name=\"{}\">",
            escape_xml(report.name())
        ));
        if !report.passed() {
            let message = report.failures().first().map_or_else(
                || "scenario failed".to_owned(),
                |failure| failure.message().to_owned(),
            );
            xml.push_str(&format!("<failure message=\"{}\">", escape_xml(&message)));
            for failure in report.failures() {
                xml.push_str(&escape_xml(&format!(
                    "{}: {}\n",
                    failure.phase(),
                    failure.message()
                )));
            }
            xml.push_str("</failure>");
        }
        xml.push_str("</testcase>");
    }
    xml.push_str("</testsuite>");
    xml
}

/// Parses one RON-compatible scenario document.
pub fn parse_scenario_ron(input: &str) -> Result<Scenario, ScenarioError> {
    let value = RonParser::new(input).parse()?;
    Scenario::from_ron_value(value)
}

/// Runs one scenario and returns a CI-friendly report.
#[must_use]
pub fn run_scenario(scenario: &Scenario) -> ScenarioReport {
    let mut primary = execute_scenario(scenario, true);
    if scenario.expect.hash_determinism {
        let secondary = execute_scenario(scenario, false);
        match (primary.final_hash, secondary.final_hash) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => primary.failures.push(ScenarioFailure::new(
                "hash_determinism",
                format!(
                    "same scenario produced different final hashes: {} != {}",
                    left.get(),
                    right.get()
                ),
            )),
            _ => primary.failures.push(ScenarioFailure::new(
                "hash_determinism",
                "same scenario did not produce comparable final hashes".to_owned(),
            )),
        }
    }
    primary
}

/// A complete scenario with setup, script, and expectations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scenario {
    name: String,
    setup: ScenarioSetup,
    script: Vec<ScenarioStep>,
    expect: ScenarioExpect,
}

impl Scenario {
    /// Creates a scenario from explicit sections.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        setup: ScenarioSetup,
        script: Vec<ScenarioStep>,
        expect: ScenarioExpect,
    ) -> Self {
        Self {
            name: name.into(),
            setup,
            script,
            expect,
        }
    }

    /// Returns the scenario name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the scenario setup section.
    #[must_use]
    pub const fn setup(&self) -> &ScenarioSetup {
        &self.setup
    }

    /// Returns the scenario action script.
    #[must_use]
    pub fn script(&self) -> &[ScenarioStep] {
        &self.script
    }

    /// Returns the scenario expectation section.
    #[must_use]
    pub const fn expect(&self) -> &ScenarioExpect {
        &self.expect
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("scenario")?;
        let name = map
            .optional_string("name")?
            .unwrap_or_else(|| "scenario".to_owned());
        let setup = ScenarioSetup::from_ron_value(map.required("setup")?)?;
        let script = parse_script(map.required("script")?)?;
        let expect = ScenarioExpect::from_ron_value(map.required("expect")?)?;
        Ok(Self::new(name, setup, script, expect))
    }
}

/// Initial state construction for a scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioSetup {
    seed: Option<u64>,
    players: usize,
    libraries: Vec<LibrarySetup>,
    objects: Vec<ObjectSetup>,
}

impl ScenarioSetup {
    /// Creates setup with a fixed number of players.
    #[must_use]
    pub fn new(players: usize) -> Self {
        Self {
            seed: None,
            players,
            libraries: Vec::new(),
            objects: Vec::new(),
        }
    }

    /// Returns setup with a deterministic seed.
    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Returns setup with a player's library seeded in listed order.
    #[must_use]
    pub fn with_library(mut self, library: LibrarySetup) -> Self {
        self.libraries.push(library);
        self
    }

    /// Returns setup with one explicit object creation.
    #[must_use]
    pub fn with_object(mut self, object: ObjectSetup) -> Self {
        self.objects.push(object);
        self
    }

    /// Returns the deterministic seed, if present.
    #[must_use]
    pub const fn seed(&self) -> Option<u64> {
        self.seed
    }

    /// Returns the number of players to add.
    #[must_use]
    pub const fn players(&self) -> usize {
        self.players
    }

    /// Returns library setup records.
    #[must_use]
    pub fn libraries(&self) -> &[LibrarySetup] {
        &self.libraries
    }

    /// Returns explicit object setup records.
    #[must_use]
    pub fn objects(&self) -> &[ObjectSetup] {
        &self.objects
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("setup")?;
        let players = map.required_usize("players")?;
        let mut setup = Self::new(players);
        if let Some(seed) = map.optional_u64("seed")? {
            setup = setup.with_seed(seed);
        }
        if let Some(libraries) = map.optional("libraries")? {
            for value in libraries.into_list("setup.libraries")? {
                setup = setup.with_library(LibrarySetup::from_ron_value(value)?);
            }
        }
        if let Some(objects) = map.optional("objects")? {
            for value in objects.into_list("setup.objects")? {
                setup = setup.with_object(ObjectSetup::from_ron_value(value)?);
            }
        }
        Ok(setup)
    }
}

/// Library card setup for one player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibrarySetup {
    player: usize,
    cards: Vec<u32>,
}

impl LibrarySetup {
    /// Creates a library setup record.
    #[must_use]
    pub fn new(player: usize, cards: Vec<u32>) -> Self {
        Self { player, cards }
    }

    /// Returns the zero-based scenario player index.
    #[must_use]
    pub const fn player(&self) -> usize {
        self.player
    }

    /// Returns card IDs to create in that player's library.
    #[must_use]
    pub fn cards(&self) -> &[u32] {
        &self.cards
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("library setup")?;
        Ok(Self::new(
            map.required_usize("player")?,
            parse_u32_list(map.required("cards")?, "library cards")?,
        ))
    }
}

/// Explicit object creation during setup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectSetup {
    card: u32,
    owner: usize,
    controller: usize,
    zone: ZoneSpec,
}

impl ObjectSetup {
    /// Creates an object setup record.
    #[must_use]
    pub const fn new(card: u32, owner: usize, controller: usize, zone: ZoneSpec) -> Self {
        Self {
            card,
            owner,
            controller,
            zone,
        }
    }

    /// Returns the card-definition ID.
    #[must_use]
    pub const fn card(&self) -> u32 {
        self.card
    }

    /// Returns the owner player index.
    #[must_use]
    pub const fn owner(&self) -> usize {
        self.owner
    }

    /// Returns the controller player index.
    #[must_use]
    pub const fn controller(&self) -> usize {
        self.controller
    }

    /// Returns the destination zone.
    #[must_use]
    pub const fn zone(&self) -> ZoneSpec {
        self.zone
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("object setup")?;
        let card = map.required_u32("card")?;
        let owner = map.required_usize("owner")?;
        let controller = map.optional_usize("controller")?.unwrap_or(owner);
        let zone = parse_zone_from_map(&map)?;
        Ok(Self::new(card, owner, controller, zone))
    }
}

/// A scenario zone reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZoneSpec {
    /// A player's library.
    Library {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// A player's hand.
    Hand {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// A player's graveyard.
    Graveyard {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// The shared battlefield.
    Battlefield,
    /// The shared exile zone.
    Exile,
    /// The shared stack zone.
    Stack,
    /// The shared command zone.
    Command,
}

impl ZoneSpec {
    fn zone_id(self, players: &[PlayerId]) -> Result<ZoneId, ScenarioError> {
        match self {
            Self::Library { player } => Ok(ZoneId::new(
                Some(player_id(players, player, "zone player")?),
                ZoneKind::Library,
            )),
            Self::Hand { player } => Ok(ZoneId::new(
                Some(player_id(players, player, "zone player")?),
                ZoneKind::Hand,
            )),
            Self::Graveyard { player } => Ok(ZoneId::new(
                Some(player_id(players, player, "zone player")?),
                ZoneKind::Graveyard,
            )),
            Self::Battlefield => Ok(ZoneId::new(None, ZoneKind::Battlefield)),
            Self::Exile => Ok(ZoneId::new(None, ZoneKind::Exile)),
            Self::Stack => Ok(ZoneId::new(None, ZoneKind::Stack)),
            Self::Command => Ok(ZoneId::new(None, ZoneKind::Command)),
        }
    }
}

/// One action in a scenario script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioStep {
    /// Decide the starting player from the deterministic seed stream.
    DecideTurnOrder,
    /// Draw all opening hands.
    DrawOpeningHands,
    /// Take one London mulligan.
    TakeMulligan {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// Keep an opening hand.
    KeepOpeningHand {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario object indexes to place on the library bottom.
        bottom: Vec<usize>,
    },
    /// Start a turn.
    StartTurn {
        /// Zero-based scenario active-player index.
        player: usize,
    },
    /// Advance the current step.
    AdvanceStep,
    /// Pass priority.
    PassPriority {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// Check state-based actions.
    CheckStateBasedActions,
    /// Set a player's life total.
    SetLife {
        /// Zero-based scenario player index.
        player: usize,
        /// New life total.
        life: i32,
    },
    /// Make a player lose life.
    LoseLife {
        /// Zero-based scenario player index.
        player: usize,
        /// Life amount to lose.
        amount: u32,
    },
    /// Make a player gain life.
    GainLife {
        /// Zero-based scenario player index.
        player: usize,
        /// Life amount to gain.
        amount: u32,
    },
}

impl ScenarioStep {
    fn label(&self) -> String {
        match self {
            Self::DecideTurnOrder => "decide_turn_order".to_owned(),
            Self::DrawOpeningHands => "draw_opening_hands".to_owned(),
            Self::TakeMulligan { player } => format!("take_mulligan[{player}]"),
            Self::KeepOpeningHand { player, .. } => format!("keep_opening_hand[{player}]"),
            Self::StartTurn { player } => format!("start_turn[{player}]"),
            Self::AdvanceStep => "advance_step".to_owned(),
            Self::PassPriority { player } => format!("pass_priority[{player}]"),
            Self::CheckStateBasedActions => "check_state_based_actions".to_owned(),
            Self::SetLife { player, .. } => format!("set_life[{player}]"),
            Self::LoseLife { player, .. } => format!("lose_life[{player}]"),
            Self::GainLife { player, .. } => format!("gain_life[{player}]"),
        }
    }
}

/// Expected final scenario facts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScenarioExpect {
    zone_counts: Vec<ZoneCountExpectation>,
    players: Vec<PlayerExpectation>,
    outcome: Option<OutcomeExpectation>,
    invariants: Vec<Invariant>,
    hash_determinism: bool,
}

impl ScenarioExpect {
    /// Creates an empty expectation set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns expectations with one zone-count assertion appended.
    #[must_use]
    pub fn with_zone_count(mut self, expectation: ZoneCountExpectation) -> Self {
        self.zone_counts.push(expectation);
        self
    }

    /// Returns expectations with one player assertion appended.
    #[must_use]
    pub fn with_player(mut self, expectation: PlayerExpectation) -> Self {
        self.players.push(expectation);
        self
    }

    /// Returns expectations with a final game outcome assertion.
    #[must_use]
    pub const fn with_outcome(mut self, outcome: OutcomeExpectation) -> Self {
        self.outcome = Some(outcome);
        self
    }

    /// Returns expectations with one invariant enabled.
    #[must_use]
    pub fn with_invariant(mut self, invariant: Invariant) -> Self {
        if invariant == Invariant::HashDeterminism {
            self.hash_determinism = true;
        } else {
            self.invariants.push(invariant);
        }
        self
    }

    /// Returns expectations with final replay hash determinism enabled.
    #[must_use]
    pub const fn with_hash_determinism(mut self) -> Self {
        self.hash_determinism = true;
        self
    }

    /// Returns zone-count expectations.
    #[must_use]
    pub fn zone_counts(&self) -> &[ZoneCountExpectation] {
        &self.zone_counts
    }

    /// Returns player expectations.
    #[must_use]
    pub fn players(&self) -> &[PlayerExpectation] {
        &self.players
    }

    /// Returns the final outcome expectation, if any.
    #[must_use]
    pub const fn outcome(&self) -> Option<OutcomeExpectation> {
        self.outcome
    }

    /// Returns invariants checked after setup and after every action.
    #[must_use]
    pub fn invariants(&self) -> &[Invariant] {
        &self.invariants
    }

    /// Returns whether the runner replays this scenario to compare final hashes.
    #[must_use]
    pub const fn hash_determinism(&self) -> bool {
        self.hash_determinism
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("expect")?;
        let mut expect = Self::new();
        if let Some(zone_counts) = map.optional("zone_counts")? {
            for value in zone_counts.into_list("expect.zone_counts")? {
                expect = expect.with_zone_count(ZoneCountExpectation::from_ron_value(value)?);
            }
        }
        if let Some(players) = map.optional("players")? {
            for value in players.into_list("expect.players")? {
                expect = expect.with_player(PlayerExpectation::from_ron_value(value)?);
            }
        }
        if let Some(outcome) = map.optional_string("outcome")? {
            expect = expect.with_outcome(OutcomeExpectation::parse(&outcome)?);
        }
        if let Some(invariants) = map.optional("invariants")? {
            for value in invariants.into_list("expect.invariants")? {
                expect = expect.with_invariant(Invariant::parse(&value.into_string("invariant")?)?);
            }
        }
        if map.optional_bool("hash_determinism")?.unwrap_or(false) {
            expect = expect.with_hash_determinism();
        }
        Ok(expect)
    }
}

/// A final zone-size expectation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZoneCountExpectation {
    zone: ZoneSpec,
    count: usize,
}

impl ZoneCountExpectation {
    /// Creates a zone-count expectation.
    #[must_use]
    pub const fn new(zone: ZoneSpec, count: usize) -> Self {
        Self { zone, count }
    }

    /// Returns the expected zone.
    #[must_use]
    pub const fn zone(self) -> ZoneSpec {
        self.zone
    }

    /// Returns the expected object count.
    #[must_use]
    pub const fn count(self) -> usize {
        self.count
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("zone count")?;
        let zone = parse_zone_from_map(&map)?;
        Ok(Self::new(zone, map.required_usize("count")?))
    }
}

/// A final player scalar expectation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerExpectation {
    player: usize,
    life: Option<i32>,
    poison: Option<u32>,
}

impl PlayerExpectation {
    /// Creates a player expectation.
    #[must_use]
    pub const fn new(player: usize) -> Self {
        Self {
            player,
            life: None,
            poison: None,
        }
    }

    /// Returns this expectation with a life-total assertion.
    #[must_use]
    pub const fn with_life(mut self, life: i32) -> Self {
        self.life = Some(life);
        self
    }

    /// Returns this expectation with a poison-counter assertion.
    #[must_use]
    pub const fn with_poison(mut self, poison: u32) -> Self {
        self.poison = Some(poison);
        self
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("player expectation")?;
        let mut expectation = Self::new(map.required_usize("player")?);
        if let Some(life) = map.optional_i32("life")? {
            expectation = expectation.with_life(life);
        }
        if let Some(poison) = map.optional_u32("poison")? {
            expectation = expectation.with_poison(poison);
        }
        Ok(expectation)
    }
}

/// A final game outcome expectation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeExpectation {
    /// The game remains in progress.
    InProgress,
    /// A player has won.
    Won {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// The game ended in a draw.
    Draw,
}

impl OutcomeExpectation {
    fn parse(input: &str) -> Result<Self, ScenarioError> {
        match input {
            "in_progress" | "InProgress" => Ok(Self::InProgress),
            "draw" | "Draw" => Ok(Self::Draw),
            _ => Err(ScenarioError::schema(format!(
                "unsupported outcome expectation `{input}`"
            ))),
        }
    }
}

/// An invariant checked by the runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Invariant {
    /// Every object appears in exactly one zone.
    ZoneConservation,
    /// Player scalar values remain inside conservative sanity limits.
    LifePoisonSanity,
    /// Allocated and streaming deterministic hashes agree.
    HashConsistency,
    /// Replaying the full scenario produces the same final hash.
    HashDeterminism,
}

impl Invariant {
    fn parse(input: &str) -> Result<Self, ScenarioError> {
        match input {
            "zone_conservation" => Ok(Self::ZoneConservation),
            "life_poison_sanity" => Ok(Self::LifePoisonSanity),
            "hash_consistency" => Ok(Self::HashConsistency),
            "hash_determinism" => Ok(Self::HashDeterminism),
            _ => Err(ScenarioError::schema(format!(
                "unsupported invariant `{input}`"
            ))),
        }
    }
}

/// Scenario execution report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioReport {
    name: String,
    steps: Vec<StepRecord>,
    failures: Vec<ScenarioFailure>,
    final_hash: Option<StateHash>,
}

impl ScenarioReport {
    /// Returns the scenario name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns per-step execution records.
    #[must_use]
    pub fn steps(&self) -> &[StepRecord] {
        &self.steps
    }

    /// Returns failures collected during execution and expectation checks.
    #[must_use]
    pub fn failures(&self) -> &[ScenarioFailure] {
        &self.failures
    }

    /// Returns true if the scenario passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// Returns the final deterministic hash, when execution reached a final state.
    #[must_use]
    pub const fn final_hash(&self) -> Option<StateHash> {
        self.final_hash
    }

    /// Serializes this report as small JUnit-style XML.
    #[must_use]
    pub fn to_junit_xml(&self) -> String {
        let failures = usize::from(!self.passed());
        let mut xml = format!(
            "<testsuite name=\"forge-testkit\" tests=\"1\" failures=\"{failures}\">\
             <testcase name=\"{}\">",
            escape_xml(&self.name)
        );
        if !self.passed() {
            let message = self.failures.first().map_or_else(
                || "scenario failed".to_owned(),
                |failure| failure.message.clone(),
            );
            xml.push_str(&format!("<failure message=\"{}\">", escape_xml(&message)));
            for failure in &self.failures {
                xml.push_str(&escape_xml(&format!(
                    "{}: {}\n",
                    failure.phase, failure.message
                )));
            }
            xml.push_str("</failure>");
        }
        xml.push_str("</testcase></testsuite>");
        xml
    }
}

/// One script-step execution record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepRecord {
    label: String,
    outcome: String,
}

impl StepRecord {
    /// Returns the step label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the formatted outcome.
    #[must_use]
    pub fn outcome(&self) -> &str {
        &self.outcome
    }
}

/// A scenario failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioFailure {
    phase: String,
    message: String,
}

impl ScenarioFailure {
    /// Creates a failure record.
    #[must_use]
    pub fn new(phase: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            message: message.into(),
        }
    }

    /// Returns the phase that produced the failure.
    #[must_use]
    pub fn phase(&self) -> &str {
        &self.phase
    }

    /// Returns the failure message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Scenario parse or schema error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioError {
    message: String,
}

impl ScenarioError {
    /// Creates a schema error.
    #[must_use]
    pub fn schema(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for ScenarioError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ScenarioError {}

struct RunContext {
    state: GameState,
    players: Vec<PlayerId>,
    objects: Vec<ObjectId>,
    failures: Vec<ScenarioFailure>,
    steps: Vec<StepRecord>,
}

fn execute_scenario(scenario: &Scenario, check_expectations: bool) -> ScenarioReport {
    let mut context = RunContext {
        state: GameState::new(),
        players: Vec::new(),
        objects: Vec::new(),
        failures: Vec::new(),
        steps: Vec::new(),
    };

    setup_scenario(scenario, &mut context);
    check_invariants("setup", &scenario.expect.invariants, &mut context);
    for step in &scenario.script {
        execute_step(step, &mut context);
        let label = step.label();
        check_invariants(&label, &scenario.expect.invariants, &mut context);
    }
    if check_expectations {
        check_expectations_for(scenario, &mut context);
    }

    ScenarioReport {
        name: scenario.name.clone(),
        steps: context.steps,
        failures: context.failures,
        final_hash: Some(context.state.deterministic_hash()),
    }
}

fn setup_scenario(scenario: &Scenario, context: &mut RunContext) {
    if let Some(seed) = scenario.setup.seed {
        record_outcome(
            "setup.set_seed",
            apply(&mut context.state, Action::SetSeed { seed }),
            context,
        );
    }
    for _ in 0..scenario.setup.players {
        match apply(&mut context.state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => context.players.push(player),
            other => context.failures.push(ScenarioFailure::new(
                "setup.add_player",
                format!("unexpected add-player outcome: {other:?}"),
            )),
        }
    }
    for library in &scenario.setup.libraries {
        let Ok(player) = player_id(&context.players, library.player, "library setup") else {
            context.failures.push(ScenarioFailure::new(
                "library setup",
                format!("unknown player index {}", library.player),
            ));
            continue;
        };
        let zone = ZoneId::new(Some(player), ZoneKind::Library);
        for card in &library.cards {
            create_object(
                context,
                *card,
                library.player,
                library.player,
                zone,
                "library setup",
            );
        }
    }
    for object in &scenario.setup.objects {
        match object.zone.zone_id(&context.players) {
            Ok(zone) => create_object(
                context,
                object.card,
                object.owner,
                object.controller,
                zone,
                "object setup",
            ),
            Err(error) => context
                .failures
                .push(ScenarioFailure::new("object setup", error.to_string())),
        }
    }
}

fn create_object(
    context: &mut RunContext,
    card: u32,
    owner: usize,
    controller: usize,
    zone: ZoneId,
    phase: &str,
) {
    let owner = match player_id(&context.players, owner, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let controller = match player_id(&context.players, controller, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    match apply(
        &mut context.state,
        Action::CreateObject {
            card: CardId::new(card),
            owner,
            controller,
            zone,
        },
    ) {
        Outcome::ObjectCreated(object) => context.objects.push(object),
        other => context.failures.push(ScenarioFailure::new(
            phase,
            format!("unexpected create-object outcome: {other:?}"),
        )),
    }
}

fn execute_step(step: &ScenarioStep, context: &mut RunContext) {
    let label = step.label();
    let action = match action_for_step(step, context) {
        Ok(action) => action,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(label, error.to_string()));
            return;
        }
    };
    let outcome = apply(&mut context.state, action);
    if let Outcome::Failed(error) = outcome {
        context.failures.push(ScenarioFailure::new(
            label.clone(),
            format!("action failed: {error:?}"),
        ));
    }
    record_outcome(&label, outcome, context);
}

fn action_for_step(step: &ScenarioStep, context: &RunContext) -> Result<Action, ScenarioError> {
    match step {
        ScenarioStep::DecideTurnOrder => Ok(Action::DecideTurnOrder),
        ScenarioStep::DrawOpeningHands => Ok(Action::DrawOpeningHands),
        ScenarioStep::TakeMulligan { player } => Ok(Action::TakeMulligan {
            player: player_id(&context.players, *player, "take_mulligan")?,
        }),
        ScenarioStep::KeepOpeningHand { player, bottom } => {
            let mut objects = Vec::with_capacity(bottom.len());
            for index in bottom {
                objects.push(object_id(&context.objects, *index, "keep_opening_hand")?);
            }
            Ok(Action::KeepOpeningHand {
                player: player_id(&context.players, *player, "keep_opening_hand")?,
                bottom: objects,
            })
        }
        ScenarioStep::StartTurn { player } => Ok(Action::StartTurn {
            active_player: player_id(&context.players, *player, "start_turn")?,
        }),
        ScenarioStep::AdvanceStep => Ok(Action::AdvanceStep),
        ScenarioStep::PassPriority { player } => Ok(Action::PassPriority {
            player: player_id(&context.players, *player, "pass_priority")?,
        }),
        ScenarioStep::CheckStateBasedActions => Ok(Action::CheckStateBasedActions),
        ScenarioStep::SetLife { player, life } => Ok(Action::SetPlayerLife {
            player: player_id(&context.players, *player, "set_life")?,
            life: *life,
        }),
        ScenarioStep::LoseLife { player, amount } => Ok(Action::LoseLife {
            player: player_id(&context.players, *player, "lose_life")?,
            amount: *amount,
        }),
        ScenarioStep::GainLife { player, amount } => Ok(Action::GainLife {
            player: player_id(&context.players, *player, "gain_life")?,
            amount: *amount,
        }),
    }
}

fn record_outcome(label: &str, outcome: Outcome, context: &mut RunContext) {
    context.steps.push(StepRecord {
        label: label.to_owned(),
        outcome: format!("{outcome:?}"),
    });
}

fn check_invariants(phase: &str, invariants: &[Invariant], context: &mut RunContext) {
    for invariant in invariants {
        match invariant {
            Invariant::ZoneConservation => {
                if let Err(error) = context.state.validate_zone_conservation() {
                    context.failures.push(ScenarioFailure::new(
                        phase,
                        format!("zone conservation failed: {error:?}"),
                    ));
                }
            }
            Invariant::LifePoisonSanity => {
                for player in context.state.players() {
                    if !(-1_000_000..=1_000_000).contains(&player.life()) {
                        context.failures.push(ScenarioFailure::new(
                            phase,
                            format!(
                                "player {} life is outside sanity bounds",
                                player.id().index()
                            ),
                        ));
                    }
                    if player.poison() > 1_000 {
                        context.failures.push(ScenarioFailure::new(
                            phase,
                            format!(
                                "player {} poison is outside sanity bounds",
                                player.id().index()
                            ),
                        ));
                    }
                    if player.max_hand_size() > 1_000 {
                        context.failures.push(ScenarioFailure::new(
                            phase,
                            format!(
                                "player {} max hand size is outside sanity bounds",
                                player.id().index()
                            ),
                        ));
                    }
                }
            }
            Invariant::HashConsistency => {
                if context.state.deterministic_hash()
                    != context.state.deterministic_hash_streaming()
                {
                    context.failures.push(ScenarioFailure::new(
                        phase,
                        "allocated and streaming deterministic hashes differ".to_owned(),
                    ));
                }
            }
            Invariant::HashDeterminism => {}
        }
    }
}

fn check_expectations_for(scenario: &Scenario, context: &mut RunContext) {
    for expectation in &scenario.expect.zone_counts {
        match zone_count(context, expectation.zone) {
            Ok(actual) if actual == expectation.count => {}
            Ok(actual) => context.failures.push(ScenarioFailure::new(
                "expect.zone_counts",
                format!(
                    "zone {:?} expected {} objects, found {}",
                    expectation.zone, expectation.count, actual
                ),
            )),
            Err(error) => context.failures.push(ScenarioFailure::new(
                "expect.zone_counts",
                error.to_string(),
            )),
        }
    }
    for expectation in &scenario.expect.players {
        match player_id(&context.players, expectation.player, "expect.players") {
            Ok(player) => {
                let Some(state) = context.state.players().get(player.index()) else {
                    context.failures.push(ScenarioFailure::new(
                        "expect.players",
                        format!("missing player state {}", player.index()),
                    ));
                    continue;
                };
                if let Some(life) = expectation.life {
                    if state.life() != life {
                        context.failures.push(ScenarioFailure::new(
                            "expect.players",
                            format!(
                                "player {} expected life {}, found {}",
                                expectation.player,
                                life,
                                state.life()
                            ),
                        ));
                    }
                }
                if let Some(poison) = expectation.poison {
                    if state.poison() != poison {
                        context.failures.push(ScenarioFailure::new(
                            "expect.players",
                            format!(
                                "player {} expected poison {}, found {}",
                                expectation.player,
                                poison,
                                state.poison()
                            ),
                        ));
                    }
                }
            }
            Err(error) => context
                .failures
                .push(ScenarioFailure::new("expect.players", error.to_string())),
        }
    }
    if let Some(outcome) = scenario.expect.outcome {
        check_outcome(outcome, context);
    }
}

fn check_outcome(expectation: OutcomeExpectation, context: &mut RunContext) {
    match (expectation, context.state.game_outcome()) {
        (OutcomeExpectation::InProgress, GameOutcome::InProgress)
        | (OutcomeExpectation::Draw, GameOutcome::Draw) => {}
        (OutcomeExpectation::Won { player }, GameOutcome::Won(winner)) => {
            match player_id(&context.players, player, "expect.outcome") {
                Ok(expected) if expected == winner => {}
                Ok(expected) => context.failures.push(ScenarioFailure::new(
                    "expect.outcome",
                    format!(
                        "expected winner {}, found {}",
                        expected.index(),
                        winner.index()
                    ),
                )),
                Err(error) => context
                    .failures
                    .push(ScenarioFailure::new("expect.outcome", error.to_string())),
            }
        }
        (_, actual) => context.failures.push(ScenarioFailure::new(
            "expect.outcome",
            format!("unexpected outcome: {actual:?}"),
        )),
    }
}

fn zone_count(context: &RunContext, zone: ZoneSpec) -> Result<usize, ScenarioError> {
    let zone_id = zone.zone_id(&context.players)?;
    let observer = context
        .players
        .first()
        .copied()
        .ok_or_else(|| ScenarioError::schema("zone count requires at least one player"))?;
    let view = context
        .state
        .player_view(observer)
        .map_err(|error| ScenarioError::schema(format!("player view failed: {error:?}")))?;
    Ok(view
        .zone(zone_id)
        .ok_or_else(|| ScenarioError::schema(format!("missing zone {zone_id:?}")))?
        .objects()
        .len())
}

fn player_id(players: &[PlayerId], index: usize, phase: &str) -> Result<PlayerId, ScenarioError> {
    players
        .get(index)
        .copied()
        .ok_or_else(|| ScenarioError::schema(format!("{phase}: unknown player index {index}")))
}

fn object_id(objects: &[ObjectId], index: usize, phase: &str) -> Result<ObjectId, ScenarioError> {
    objects
        .get(index)
        .copied()
        .ok_or_else(|| ScenarioError::schema(format!("{phase}: unknown object index {index}")))
}

fn parse_script(value: RonValue) -> Result<Vec<ScenarioStep>, ScenarioError> {
    let mut script = Vec::new();
    for value in value.into_list("script")? {
        let map = value.into_map("script step")?;
        let action = map.required_string("action")?;
        script.push(match action.as_str() {
            "decide_turn_order" => ScenarioStep::DecideTurnOrder,
            "draw_opening_hands" => ScenarioStep::DrawOpeningHands,
            "take_mulligan" => ScenarioStep::TakeMulligan {
                player: map.required_usize("player")?,
            },
            "keep_opening_hand" => ScenarioStep::KeepOpeningHand {
                player: map.required_usize("player")?,
                bottom: parse_usize_list(
                    map.optional("bottom")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                    "keep_opening_hand.bottom",
                )?,
            },
            "start_turn" => ScenarioStep::StartTurn {
                player: map.required_usize("player")?,
            },
            "advance_step" => ScenarioStep::AdvanceStep,
            "pass_priority" => ScenarioStep::PassPriority {
                player: map.required_usize("player")?,
            },
            "check_state_based_actions" => ScenarioStep::CheckStateBasedActions,
            "set_life" => ScenarioStep::SetLife {
                player: map.required_usize("player")?,
                life: map.required_i32("life")?,
            },
            "lose_life" => ScenarioStep::LoseLife {
                player: map.required_usize("player")?,
                amount: map.required_u32("amount")?,
            },
            "gain_life" => ScenarioStep::GainLife {
                player: map.required_usize("player")?,
                amount: map.required_u32("amount")?,
            },
            _ => {
                return Err(ScenarioError::schema(format!(
                    "unsupported script action `{action}`"
                )));
            }
        });
    }
    Ok(script)
}

fn parse_zone_from_map(map: &RonMap) -> Result<ZoneSpec, ScenarioError> {
    let zone = map.required_string("zone")?;
    let player = map.optional_usize("player")?;
    match zone.as_str() {
        "Library" | "library" => Ok(ZoneSpec::Library {
            player: player.ok_or_else(|| {
                ScenarioError::schema("library zone requires `player`".to_owned())
            })?,
        }),
        "Hand" | "hand" => Ok(ZoneSpec::Hand {
            player: player
                .ok_or_else(|| ScenarioError::schema("hand zone requires `player`".to_owned()))?,
        }),
        "Graveyard" | "graveyard" => Ok(ZoneSpec::Graveyard {
            player: player.ok_or_else(|| {
                ScenarioError::schema("graveyard zone requires `player`".to_owned())
            })?,
        }),
        "Battlefield" | "battlefield" => Ok(ZoneSpec::Battlefield),
        "Exile" | "exile" => Ok(ZoneSpec::Exile),
        "Stack" | "stack" => Ok(ZoneSpec::Stack),
        "Command" | "command" => Ok(ZoneSpec::Command),
        _ => Err(ScenarioError::schema(format!("unsupported zone `{zone}`"))),
    }
}

fn parse_u32_list(value: RonValue, label: &str) -> Result<Vec<u32>, ScenarioError> {
    value
        .into_list(label)?
        .into_iter()
        .map(|value| value.into_u32(label))
        .collect()
}

fn parse_usize_list(value: RonValue, label: &str) -> Result<Vec<usize>, ScenarioError> {
    value
        .into_list(label)?
        .into_iter()
        .map(|value| value.into_usize(label))
        .collect()
}

fn escape_xml(input: &str) -> String {
    let mut escaped = String::new();
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RonValue {
    Map(RonMap),
    List(Vec<RonValue>),
    String(String),
    Integer(i64),
    Bool(bool),
}

impl RonValue {
    fn into_map(self, label: &str) -> Result<RonMap, ScenarioError> {
        match self {
            Self::Map(map) => Ok(map),
            _ => Err(ScenarioError::schema(format!("{label} must be a map"))),
        }
    }

    fn into_list(self, label: &str) -> Result<Vec<RonValue>, ScenarioError> {
        match self {
            Self::List(list) => Ok(list),
            _ => Err(ScenarioError::schema(format!("{label} must be a list"))),
        }
    }

    fn into_string(self, label: &str) -> Result<String, ScenarioError> {
        match self {
            Self::String(value) => Ok(value),
            _ => Err(ScenarioError::schema(format!("{label} must be a string"))),
        }
    }

    fn into_usize(self, label: &str) -> Result<usize, ScenarioError> {
        let value = self.into_integer(label)?;
        usize::try_from(value)
            .map_err(|_| ScenarioError::schema(format!("{label} must be a nonnegative integer")))
    }

    fn into_u32(self, label: &str) -> Result<u32, ScenarioError> {
        let value = self.into_integer(label)?;
        u32::try_from(value).map_err(|_| ScenarioError::schema(format!("{label} must fit in u32")))
    }

    fn into_u64(self, label: &str) -> Result<u64, ScenarioError> {
        let value = self.into_integer(label)?;
        u64::try_from(value).map_err(|_| ScenarioError::schema(format!("{label} must fit in u64")))
    }

    fn into_i32(self, label: &str) -> Result<i32, ScenarioError> {
        let value = self.into_integer(label)?;
        i32::try_from(value).map_err(|_| ScenarioError::schema(format!("{label} must fit in i32")))
    }

    fn into_bool(self, label: &str) -> Result<bool, ScenarioError> {
        match self {
            Self::Bool(value) => Ok(value),
            _ => Err(ScenarioError::schema(format!("{label} must be a bool"))),
        }
    }

    fn into_integer(self, label: &str) -> Result<i64, ScenarioError> {
        match self {
            Self::Integer(value) => Ok(value),
            _ => Err(ScenarioError::schema(format!("{label} must be an integer"))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RonMap(Vec<(String, RonValue)>);

impl RonMap {
    fn required(&self, key: &str) -> Result<RonValue, ScenarioError> {
        self.optional(key)?
            .ok_or_else(|| ScenarioError::schema(format!("missing required key `{key}`")))
    }

    fn optional(&self, key: &str) -> Result<Option<RonValue>, ScenarioError> {
        let mut found = None;
        for (candidate, value) in &self.0 {
            if candidate == key {
                if found.is_some() {
                    return Err(ScenarioError::schema(format!("duplicate key `{key}`")));
                }
                found = Some(value.clone());
            }
        }
        Ok(found)
    }

    fn required_string(&self, key: &str) -> Result<String, ScenarioError> {
        self.required(key)?.into_string(key)
    }

    fn optional_string(&self, key: &str) -> Result<Option<String>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_string(key))
            .transpose()
    }

    fn required_usize(&self, key: &str) -> Result<usize, ScenarioError> {
        self.required(key)?.into_usize(key)
    }

    fn optional_usize(&self, key: &str) -> Result<Option<usize>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_usize(key))
            .transpose()
    }

    fn required_u32(&self, key: &str) -> Result<u32, ScenarioError> {
        self.required(key)?.into_u32(key)
    }

    fn optional_u32(&self, key: &str) -> Result<Option<u32>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_u32(key))
            .transpose()
    }

    fn optional_u64(&self, key: &str) -> Result<Option<u64>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_u64(key))
            .transpose()
    }

    fn required_i32(&self, key: &str) -> Result<i32, ScenarioError> {
        self.required(key)?.into_i32(key)
    }

    fn optional_i32(&self, key: &str) -> Result<Option<i32>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_i32(key))
            .transpose()
    }

    fn optional_bool(&self, key: &str) -> Result<Option<bool>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_bool(key))
            .transpose()
    }
}

struct RonParser<'a> {
    input: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl<'a> RonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<RonValue, ScenarioError> {
        let value = self.parse_value()?;
        self.skip_ws_and_comments();
        if self.peek().is_some() {
            return Err(self.error("unexpected trailing input"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<RonValue, ScenarioError> {
        self.skip_ws_and_comments();
        match self.peek() {
            Some('(') => self.parse_map(),
            Some('[') => self.parse_list(),
            Some('"') => self.parse_string().map(RonValue::String),
            Some('-' | '0'..='9') => self.parse_integer().map(RonValue::Integer),
            Some(character) if is_ident_start(character) => self.parse_ident_value(),
            Some(_) => Err(self.error("unexpected token")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_map(&mut self) -> Result<RonValue, ScenarioError> {
        self.expect_char('(')?;
        let mut entries = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.consume_char(')') {
                break;
            }
            let key = self.parse_key()?;
            self.skip_ws_and_comments();
            self.expect_char(':')?;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws_and_comments();
            if self.consume_char(',') {
                continue;
            }
            self.expect_char(')')?;
            break;
        }
        Ok(RonValue::Map(RonMap(entries)))
    }

    fn parse_list(&mut self) -> Result<RonValue, ScenarioError> {
        self.expect_char('[')?;
        let mut values = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.consume_char(']') {
                break;
            }
            values.push(self.parse_value()?);
            self.skip_ws_and_comments();
            if self.consume_char(',') {
                continue;
            }
            self.expect_char(']')?;
            break;
        }
        Ok(RonValue::List(values))
    }

    fn parse_key(&mut self) -> Result<String, ScenarioError> {
        self.skip_ws_and_comments();
        match self.peek() {
            Some('"') => self.parse_string(),
            Some(character) if is_ident_start(character) => self.parse_ident(),
            _ => Err(self.error("expected map key")),
        }
    }

    fn parse_ident_value(&mut self) -> Result<RonValue, ScenarioError> {
        let ident = self.parse_ident()?;
        match ident.as_str() {
            "true" => Ok(RonValue::Bool(true)),
            "false" => Ok(RonValue::Bool(false)),
            _ => Ok(RonValue::String(ident)),
        }
    }

    fn parse_ident(&mut self) -> Result<String, ScenarioError> {
        self.skip_ws_and_comments();
        let Some(first) = self.peek() else {
            return Err(self.error("expected identifier"));
        };
        if !is_ident_start(first) {
            return Err(self.error("expected identifier"));
        }
        let mut ident = String::new();
        while let Some(character) = self.peek() {
            if is_ident_continue(character) {
                ident.push(character);
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(ident)
    }

    fn parse_string(&mut self) -> Result<String, ScenarioError> {
        self.expect_char('"')?;
        let mut value = String::new();
        while let Some(character) = self.peek() {
            self.pos += 1;
            match character {
                '"' => return Ok(value),
                '\\' => {
                    let Some(escaped) = self.peek() else {
                        return Err(self.error("unterminated string escape"));
                    };
                    self.pos += 1;
                    match escaped {
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        other => {
                            return Err(
                                self.error(&format!("unsupported string escape `\\{other}`"))
                            );
                        }
                    }
                }
                other => value.push(other),
            }
        }
        Err(self.error("unterminated string"))
    }

    fn parse_integer(&mut self) -> Result<i64, ScenarioError> {
        self.skip_ws_and_comments();
        let start = self.pos;
        if self.consume_char('-')
            && !self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
        {
            return Err(self.error("expected digit after minus sign"));
        }
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_digit())
        {
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse::<i64>()
            .map_err(|_| self.error("integer is out of range"))
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.pos += 1;
            }
            if self.peek() == Some('/') && self.peek_next() == Some('/') {
                while self.peek().is_some_and(|character| character != '\n') {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), ScenarioError> {
        self.skip_ws_and_comments();
        if self.consume_char(expected) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{expected}`")))
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn error(&self, message: &str) -> ScenarioError {
        let byte = self
            .chars
            .iter()
            .take(self.pos)
            .map(|character| character.len_utf8())
            .sum::<usize>();
        let preview = self.input.get(byte..).unwrap_or("");
        ScenarioError::schema(format!("{message} at char {} near `{}`", self.pos, preview))
    }
}

fn is_ident_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

fn is_ident_continue(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::{
        crate_ready, parse_scenario_ron, run_scenario, run_scenario_ron, Invariant, LibrarySetup,
        Scenario, ScenarioExpect, ScenarioSetup, ScenarioStep, ZoneCountExpectation, ZoneSpec,
    };

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn constructed_scenario_runs_opening_hand_setup() {
        let scenario = Scenario::new(
            "opening hand setup",
            ScenarioSetup::new(2)
                .with_seed(17)
                .with_library(LibrarySetup::new(
                    0,
                    vec![100, 101, 102, 103, 104, 105, 106],
                ))
                .with_library(LibrarySetup::new(
                    1,
                    vec![200, 201, 202, 203, 204, 205, 206],
                )),
            vec![
                ScenarioStep::DecideTurnOrder,
                ScenarioStep::DrawOpeningHands,
            ],
            ScenarioExpect::new()
                .with_zone_count(ZoneCountExpectation::new(ZoneSpec::Hand { player: 0 }, 7))
                .with_zone_count(ZoneCountExpectation::new(ZoneSpec::Hand { player: 1 }, 7))
                .with_invariant(Invariant::ZoneConservation)
                .with_invariant(Invariant::HashConsistency)
                .with_hash_determinism(),
        );

        let report = run_scenario(&scenario);

        assert!(report.passed(), "{:?}", report.failures());
        assert!(report.final_hash().is_some());
        assert!(report.to_junit_xml().contains("failures=\"0\""));
    }

    #[test]
    fn ron_scenario_runs_and_reports_junit() {
        let input = r#"
        (
            name: "ron opening hand",
            setup: (
                seed: 23,
                players: 2,
                libraries: [
                    (player: 0, cards: [1, 2, 3, 4, 5, 6, 7]),
                    (player: 1, cards: [11, 12, 13, 14, 15, 16, 17]),
                ],
            ),
            script: [
                (action: "decide_turn_order"),
                (action: "draw_opening_hands"),
            ],
            expect: (
                zone_counts: [
                    (zone: "Hand", player: 0, count: 7),
                    (zone: "Hand", player: 1, count: 7),
                    (zone: "Library", player: 0, count: 0),
                    (zone: "Library", player: 1, count: 0),
                ],
                invariants: [
                    "zone_conservation",
                    "life_poison_sanity",
                    "hash_consistency",
                    "hash_determinism",
                ],
            ),
        )
        "#;

        let scenario = parse_scenario_ron(input)
            .unwrap_or_else(|error| panic!("unexpected parse error: {error}"));
        let report = run_scenario(&scenario);

        assert_eq!(scenario.name(), "ron opening hand");
        assert!(report.passed(), "{:?}", report.failures());
        assert!(report.to_junit_xml().contains("<testcase"));
    }

    #[test]
    fn run_scenario_ron_records_failed_expectation() {
        let input = r#"
        (
            name: "bad count",
            setup: (
                players: 1,
                libraries: [(player: 0, cards: [1])],
            ),
            script: [],
            expect: (
                zone_counts: [(zone: "Library", player: 0, count: 2)],
                invariants: ["zone_conservation"],
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(!report.passed());
        assert_eq!(report.failures().len(), 1);
        assert!(report.to_junit_xml().contains("<failure"));
    }
}
