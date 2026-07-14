#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Scenario, oracle, and invariant testing crate for Forge 2.0.
//!
//! T1.9 introduces a small RON-compatible scenario surface for kernel tests.
//! The runner intentionally executes through [`forge_core::apply`] so scenario
//! tests exercise the same public mutation boundary as application code.

pub mod runtime_smoke;

/// Deterministic four-player pod, prompted-human controller, and replay verifier.
#[path = "../../../tests/t3_9/four_player_pod.rs"]
pub mod t3_9_pod;

use forge_core::{
    apply, auto_payment_plan, AbilityPlayer, Action, ActivatedAbilityDefinition,
    ActivatedAbilityEffect, ActivatedAbilityId, ActivationCost, ActivationTiming,
    AttackDeclaration, BaseCreatureCharacteristics, BaseObjectCharacteristics, BlockDeclaration,
    CardId, CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest,
    CombatDamageTarget, CombatRestriction, CombatRestrictionSubject, ContinuousEffectDefinition,
    ContinuousEffectId, ContinuousEffectOperation, ContinuousEffectTarget, CostModifierDefinition,
    CostModifierOperation, CostModifierScope, CounterKind, CreatureKeywords, GameOutcome,
    GameState, ManaCost, ManaKind, ManaPool, ObjectColors, ObjectId, ObjectTargetPredicate,
    ObjectTypes, Outcome, PlayerId, PlayerTargetPredicate, RangeOfInfluence, ReplacementCondition,
    ReplacementDamageTargetFilter, ReplacementDefinition, ReplacementDuration, ReplacementEffectId,
    ReplacementOperation, ReplacementSourceFilter, RestrictionDefinition, RestrictionEffect,
    SpellTiming, StackEntryId, StackObjectKind, StateHash, Step, TargetChoice,
    TargetControllerPredicate, TargetKind, TargetRequirement, TargetRestriction,
    TargetRestrictionSubject, TriggerCondition, TriggerDefinition, TriggerId, TriggerObjectFilter,
    TriggerZoneFilter, ZoneId, ZoneKind,
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
    /// Internal retention zone for tokens and copies that ceased to exist.
    Ceased,
}

impl ZoneSpec {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        match value {
            RonValue::Map(map) => parse_zone_from_map(&map),
            RonValue::String(zone) => match zone.as_str() {
                "Battlefield" | "battlefield" => Ok(Self::Battlefield),
                "Exile" | "exile" => Ok(Self::Exile),
                "Stack" | "stack" => Ok(Self::Stack),
                "Command" | "command" => Ok(Self::Command),
                "Ceased" | "ceased" => Ok(Self::Ceased),
                _ => Err(ScenarioError::schema(format!(
                    "target zone `{zone}` requires a zone map with player when applicable"
                ))),
            },
            _ => Err(ScenarioError::schema(
                "zone must be a string or map".to_owned(),
            )),
        }
    }

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
            Self::Ceased => Ok(ZoneId::new(None, ZoneKind::Ceased)),
        }
    }
}

/// One action in a scenario script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioStep {
    /// Decide the starting player from the deterministic seed stream.
    DecideTurnOrder,
    /// Set an explicit multiplayer turn order.
    SetTurnOrder {
        /// Zero-based scenario player order.
        order: Vec<usize>,
    },
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
    /// Add poison counters to a player.
    AddPoisonCounters {
        /// Zero-based scenario player index.
        player: usize,
        /// Number of poison counters to add.
        amount: u32,
    },
    /// Add mana to a player's mana pool.
    AddMana {
        /// Zero-based scenario player index.
        player: usize,
        /// Mana to add.
        mana: ManaSpec,
    },
    /// Clear a player's mana pool.
    ClearMana {
        /// Zero-based scenario player index.
        player: usize,
    },
    /// Pay mana using the kernel's preferred deterministic payment plan.
    PayManaAuto {
        /// Zero-based scenario player index.
        player: usize,
        /// Cost to pay.
        cost: ManaCostSpec,
    },
    /// Scry and bottom selected cards.
    Scry {
        /// Zero-based scenario player index.
        player: usize,
        /// Number of top library cards to inspect.
        count: u32,
        /// Scenario object indexes to bottom.
        bottom: Vec<usize>,
    },
    /// Surveil and move selected cards to the graveyard.
    Surveil {
        /// Zero-based scenario player index.
        player: usize,
        /// Number of top library cards to inspect.
        count: u32,
        /// Scenario object indexes to move to graveyard.
        graveyard: Vec<usize>,
    },
    /// Cycle a hand card using deterministic payment.
    CycleAuto {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario object index.
        object: usize,
        /// Cycling mana cost.
        cost: ManaCostSpec,
    },
    /// Attach or unattach one object.
    AttachObject {
        /// Scenario attachment object index.
        attachment: usize,
        /// Scenario target object index, or none to unattach.
        target_object: Option<usize>,
    },
    /// Equip to a controlled creature using deterministic payment.
    EquipAuto {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario equipment object index.
        equipment: usize,
        /// Scenario target creature object index.
        target_object: usize,
        /// Equip mana cost.
        cost: ManaCostSpec,
    },
    /// Move a scenario object to another zone.
    MoveObject {
        /// Scenario object index.
        object: usize,
        /// Destination zone.
        zone: ZoneSpec,
    },
    /// Create a token on the battlefield.
    CreateToken {
        /// Card definition ID for the token.
        card: u32,
        /// Zero-based owner index.
        owner: usize,
        /// Zero-based controller index.
        controller: usize,
        /// Optional base power.
        power: Option<i32>,
        /// Optional base toughness.
        toughness: Option<i32>,
        /// Static combat keywords.
        keywords: CreatureKeywordSpec,
    },
    /// Create a permanent copy on the battlefield.
    CreatePermanentCopy {
        /// Scenario source object index.
        source: usize,
        /// Zero-based owner index.
        owner: usize,
        /// Zero-based controller index.
        controller: usize,
        /// Whether the copy is also a token.
        token: bool,
    },
    /// Copy a previously created stack entry.
    CopyStackEntry {
        /// Zero-based controller index.
        player: usize,
        /// Zero-based stack-entry registration index.
        entry: usize,
    },
    /// Set an object's base creature characteristics.
    SetBaseCreature {
        /// Scenario object index.
        object: usize,
        /// Base power.
        power: i32,
        /// Base toughness.
        toughness: i32,
        /// Static combat keywords.
        keywords: CreatureKeywordSpec,
    },
    /// Clear an object's base creature characteristics.
    ClearBaseCreature {
        /// Scenario object index.
        object: usize,
    },
    /// Set an object's tapped status.
    SetObjectTapped {
        /// Scenario object index.
        object: usize,
        /// New tapped status.
        tapped: bool,
    },
    /// Set or clear an object's loyalty value.
    SetObjectLoyalty {
        /// Scenario object index.
        object: usize,
        /// New loyalty value, or none to clear loyalty tracking.
        loyalty: Option<i32>,
    },
    /// Set an object's Commander color identity metadata.
    SetObjectColorIdentity {
        /// Scenario object index.
        object: usize,
        /// Color identity metadata.
        colors: ColorSpec,
    },
    /// Designate an object as a commander.
    DesignateCommander {
        /// Scenario object index.
        object: usize,
        /// Commander color identity.
        colors: ColorSpec,
    },
    /// Record one commander cast.
    RecordCommanderCast {
        /// Scenario object index.
        object: usize,
    },
    /// Validate objects under a player's Commander color identity.
    ValidateCommanderColorIdentity {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario object indexes to validate.
        objects: Vec<usize>,
    },
    /// Add counters to an object.
    AddObjectCounters {
        /// Scenario object index.
        object: usize,
        /// Counter kind.
        kind: CounterKind,
        /// Counter amount.
        amount: u32,
    },
    /// Remove counters from an object.
    RemoveObjectCounters {
        /// Scenario object index.
        object: usize,
        /// Counter kind.
        kind: CounterKind,
        /// Counter amount.
        amount: u32,
    },
    /// Mark damage on a creature object.
    MarkDamage {
        /// Scenario object index.
        object: usize,
        /// Damage amount.
        amount: u32,
    },
    /// Register a damage replacement/prevention effect.
    RegisterDamageReplacement {
        /// Zero-based scenario controller index.
        controller: usize,
        /// Optional scenario source object index.
        source_object: Option<usize>,
        /// Optional zero-based target-player index.
        target_player: Option<usize>,
        /// Optional target-object index.
        target_object: Option<usize>,
        /// Whether only combat damage matches.
        combat_only: bool,
        /// Operation name: prevent_all, prevent, add, double, or set.
        operation: String,
        /// Operation amount for prevent/add/set.
        amount: Option<u32>,
        /// Whether the effect is removed after it applies once.
        once: bool,
        /// Whether this effect is a self-replacement effect.
        self_replacement: bool,
    },
    /// Set a player's replacement effect application order.
    SetReplacementOrder {
        /// Zero-based scenario chooser index.
        chooser: usize,
        /// Replacement registration indexes in preferred order.
        order: Vec<usize>,
    },
    /// Register a continuous effect.
    RegisterContinuousEffect {
        /// Continuous-effect registration spec.
        spec: ContinuousEffectSpec,
    },
    /// Register an activated ability.
    RegisterActivatedAbility {
        /// Activated-ability registration spec.
        spec: ActivatedAbilitySpec,
    },
    /// Register a cost modifier for activated abilities.
    RegisterCostModifier {
        /// Cost-modifier registration spec.
        spec: CostModifierSpec,
    },
    /// Register a targeting or combat restriction.
    RegisterRestriction {
        /// Restriction registration spec.
        spec: RestrictionSpec,
    },
    /// Register a triggered ability.
    RegisterTriggeredAbility {
        /// Triggered-ability registration spec.
        spec: TriggerSpec,
    },
    /// Put all currently pending triggered abilities on the stack.
    PutPendingTriggersOnStack,
    /// Activate a previously registered ability using the deterministic payment planner.
    ActivateAbilityAuto {
        /// Zero-based scenario player index.
        player: usize,
        /// Zero-based activated-ability registration index.
        ability: usize,
    },
    /// Cast a spell using the deterministic payment planner.
    CastSpellAuto {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario object index for the spell.
        object: usize,
        /// Stack kind string.
        kind: String,
        /// Spell timing string.
        timing: String,
        /// Mana cost to pay.
        cost: ManaCostSpec,
        /// Whether flash grants instant-speed timing.
        flash: bool,
        /// Optional kicker cost.
        kicker: Option<ManaCostSpec>,
        /// Optional flashback alternative cost.
        flashback: Option<ManaCostSpec>,
        /// Target slots and choices.
        targets: Vec<TargetSpec>,
    },
    /// Assert whether one target choice is currently legal.
    AssertCanTarget {
        /// Zero-based scenario player index.
        player: usize,
        /// Optional source object index.
        source_object: Option<usize>,
        /// Target requirement.
        requirement: TargetRequirementSpec,
        /// Target choice.
        target: TargetChoiceSpec,
        /// Expected legality.
        expected: bool,
    },
    /// Assert the ward cost observed for one object target.
    AssertWardCost {
        /// Target object index.
        target_object: usize,
        /// Expected ward cost.
        cost: ManaCostSpec,
    },
    /// Assert current derived object characteristics.
    AssertCharacteristics {
        /// Expected effective characteristics.
        expectation: CharacteristicExpectation,
    },
    /// Assert an object's tapped status.
    AssertObjectTapped {
        /// Scenario object index.
        object: usize,
        /// Expected tapped status.
        tapped: bool,
    },
    /// Assert an object's loyalty value.
    AssertObjectLoyalty {
        /// Scenario object index.
        object: usize,
        /// Expected loyalty value, or none if loyalty tracking is absent.
        loyalty: Option<i32>,
    },
    /// Assert an object's counter total.
    AssertObjectCounters {
        /// Scenario object index.
        object: usize,
        /// Counter kind.
        kind: CounterKind,
        /// Expected counter count.
        count: u32,
    },
    /// Assert an object's current zone.
    AssertObjectZone {
        /// Scenario object index.
        object: usize,
        /// Expected zone.
        zone: ZoneSpec,
    },
    /// Assert exact object order in a zone, bottom/front to top/back.
    AssertZoneOrder {
        /// Expected zone.
        zone: ZoneSpec,
        /// Expected scenario object indexes.
        objects: Vec<usize>,
    },
    /// Assert token/copy flags and copy source.
    AssertObjectFlags {
        /// Scenario object index.
        object: usize,
        /// Expected token flag, if asserted.
        token: Option<bool>,
        /// Expected copy flag, if asserted.
        copy: Option<bool>,
        /// Expected source object index, if asserted.
        copy_source: Option<usize>,
    },
    /// Assert an object's attachment target.
    AssertAttachedTo {
        /// Scenario attachment object index.
        attachment: usize,
        /// Expected scenario target object index, or none.
        target_object: Option<usize>,
    },
    /// Assert pending triggered ability count.
    AssertPendingTriggers {
        /// Expected pending trigger count.
        count: usize,
    },
    /// Assert metadata captured on a stack entry.
    AssertStackEntryFlags {
        /// Scenario stack-entry index.
        entry: usize,
        /// Expected kicked flag, if asserted.
        kicked: Option<bool>,
        /// Expected flashback flag, if asserted.
        flashback: Option<bool>,
    },
    /// Assert the explicit multiplayer turn order.
    AssertTurnOrder {
        /// Expected zero-based scenario player order.
        order: Vec<usize>,
    },
    /// Assert the range-of-influence policy.
    AssertRangeOfInfluence {
        /// Expected policy name.
        mode: String,
    },
    /// Assert Commander metadata on an object.
    AssertCommander {
        /// Scenario object index.
        object: usize,
        /// Expected commander flag.
        commander: bool,
        /// Expected color identity, if asserted.
        colors: Option<ColorSpec>,
        /// Expected cast count, if asserted.
        cast_count: Option<u32>,
        /// Expected generic tax, if asserted.
        tax_generic: Option<u32>,
    },
    /// Assert whether one object is legal under a player's Commander identity.
    AssertCommanderIdentityLegal {
        /// Zero-based scenario player index.
        player: usize,
        /// Scenario object index.
        object: usize,
        /// Expected legality.
        expected: bool,
    },
    /// Declare attackers during the declare attackers step.
    DeclareAttackers {
        /// Zero-based scenario attacking-player index.
        player: usize,
        /// Attack declarations.
        attacks: Vec<ScenarioAttackDeclaration>,
    },
    /// Declare blockers during the declare blockers step.
    DeclareBlockers {
        /// Zero-based scenario defending-player index.
        player: usize,
        /// Block declarations.
        blocks: Vec<ScenarioBlockDeclaration>,
    },
    /// Assert whether one attack declaration is currently legal.
    AssertCanAttack {
        /// Zero-based scenario attacking-player index.
        player: usize,
        /// Attack declaration.
        attack: ScenarioAttackDeclaration,
        /// Expected legality.
        expected: bool,
    },
    /// Assert whether one block declaration is currently legal.
    AssertCanBlock {
        /// Zero-based scenario defending-player index.
        player: usize,
        /// Block declaration.
        block: ScenarioBlockDeclaration,
        /// Expected legality.
        expected: bool,
    },
    /// Assign and deal combat damage during the combat damage step.
    AssignCombatDamage {
        /// Damage assignment requests.
        assignments: Vec<ScenarioCombatDamageRequest>,
    },
    /// Request the cleanup-step priority exception.
    RequestCleanupPriority,
}

impl ScenarioStep {
    fn label(&self) -> String {
        match self {
            Self::DecideTurnOrder => "decide_turn_order".to_owned(),
            Self::SetTurnOrder { order } => format!("set_turn_order[{}]", order.len()),
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
            Self::AddPoisonCounters { player, .. } => {
                format!("add_poison_counters[{player}]")
            }
            Self::AddMana { player, .. } => format!("add_mana[{player}]"),
            Self::ClearMana { player } => format!("clear_mana[{player}]"),
            Self::PayManaAuto { player, .. } => format!("pay_mana_auto[{player}]"),
            Self::Scry { player, .. } => format!("scry[{player}]"),
            Self::Surveil { player, .. } => format!("surveil[{player}]"),
            Self::CycleAuto { player, object, .. } => format!("cycle_auto[{player}:{object}]"),
            Self::AttachObject { attachment, .. } => format!("attach_object[{attachment}]"),
            Self::EquipAuto {
                player, equipment, ..
            } => {
                format!("equip_auto[{player}:{equipment}]")
            }
            Self::MoveObject { object, .. } => format!("move_object[{object}]"),
            Self::CreateToken { controller, .. } => format!("create_token[{controller}]"),
            Self::CreatePermanentCopy { source, .. } => format!("create_permanent_copy[{source}]"),
            Self::CopyStackEntry { player, entry } => {
                format!("copy_stack_entry[{player}:{entry}]")
            }
            Self::SetBaseCreature { object, .. } => format!("set_base_creature[{object}]"),
            Self::ClearBaseCreature { object } => format!("clear_base_creature[{object}]"),
            Self::SetObjectTapped { object, .. } => format!("set_object_tapped[{object}]"),
            Self::SetObjectLoyalty { object, .. } => format!("set_object_loyalty[{object}]"),
            Self::SetObjectColorIdentity { object, .. } => {
                format!("set_object_color_identity[{object}]")
            }
            Self::DesignateCommander { object, .. } => format!("designate_commander[{object}]"),
            Self::RecordCommanderCast { object } => format!("record_commander_cast[{object}]"),
            Self::ValidateCommanderColorIdentity { player, .. } => {
                format!("validate_commander_color_identity[{player}]")
            }
            Self::AddObjectCounters { object, .. } => format!("add_object_counters[{object}]"),
            Self::RemoveObjectCounters { object, .. } => {
                format!("remove_object_counters[{object}]")
            }
            Self::MarkDamage { object, .. } => format!("mark_damage[{object}]"),
            Self::RegisterDamageReplacement { controller, .. } => {
                format!("register_damage_replacement[{controller}]")
            }
            Self::SetReplacementOrder { chooser, .. } => {
                format!("set_replacement_order[{chooser}]")
            }
            Self::RegisterContinuousEffect { spec } => {
                format!("register_continuous_effect[{}]", spec.controller)
            }
            Self::RegisterActivatedAbility { spec } => {
                format!("register_activated_ability[{}]", spec.controller)
            }
            Self::RegisterCostModifier { spec } => {
                format!("register_cost_modifier[{}]", spec.controller)
            }
            Self::RegisterRestriction { spec } => {
                format!("register_restriction[{}]", spec.controller)
            }
            Self::RegisterTriggeredAbility { spec } => {
                format!("register_triggered_ability[{}]", spec.controller)
            }
            Self::PutPendingTriggersOnStack => "put_pending_triggers_on_stack".to_owned(),
            Self::ActivateAbilityAuto { player, ability } => {
                format!("activate_ability_auto[{player}:{ability}]")
            }
            Self::CastSpellAuto { player, object, .. } => {
                format!("cast_spell_auto[{player}:{object}]")
            }
            Self::AssertCanTarget {
                player, expected, ..
            } => {
                format!("assert_can_target[{player}:{expected}]")
            }
            Self::AssertWardCost { target_object, .. } => {
                format!("assert_ward_cost[{target_object}]")
            }
            Self::AssertCharacteristics { expectation } => {
                format!("assert_characteristics[{}]", expectation.object)
            }
            Self::AssertObjectTapped { object, .. } => format!("assert_object_tapped[{object}]"),
            Self::AssertObjectLoyalty { object, .. } => format!("assert_object_loyalty[{object}]"),
            Self::AssertObjectCounters { object, .. } => {
                format!("assert_object_counters[{object}]")
            }
            Self::AssertObjectZone { object, .. } => format!("assert_object_zone[{object}]"),
            Self::AssertZoneOrder { objects, .. } => {
                format!("assert_zone_order[{}]", objects.len())
            }
            Self::AssertObjectFlags { object, .. } => format!("assert_object_flags[{object}]"),
            Self::AssertAttachedTo { attachment, .. } => {
                format!("assert_attached_to[{attachment}]")
            }
            Self::AssertPendingTriggers { count } => format!("assert_pending_triggers[{count}]"),
            Self::AssertStackEntryFlags { entry, .. } => {
                format!("assert_stack_entry_flags[{entry}]")
            }
            Self::AssertTurnOrder { order } => format!("assert_turn_order[{}]", order.len()),
            Self::AssertRangeOfInfluence { mode } => {
                format!("assert_range_of_influence[{mode}]")
            }
            Self::AssertCommander { object, .. } => format!("assert_commander[{object}]"),
            Self::AssertCommanderIdentityLegal {
                player,
                object,
                expected,
            } => {
                format!("assert_commander_identity_legal[{player}:{object}:{expected}]")
            }
            Self::DeclareAttackers { player, .. } => format!("declare_attackers[{player}]"),
            Self::DeclareBlockers { player, .. } => format!("declare_blockers[{player}]"),
            Self::AssertCanAttack { player, .. } => format!("assert_can_attack[{player}]"),
            Self::AssertCanBlock { player, .. } => format!("assert_can_block[{player}]"),
            Self::AssignCombatDamage { .. } => "assign_combat_damage".to_owned(),
            Self::RequestCleanupPriority => "request_cleanup_priority".to_owned(),
        }
    }
}

/// One scenario attack declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioAttackDeclaration {
    attacker: usize,
    defender: usize,
}

impl ScenarioAttackDeclaration {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("attack declaration")?;
        Ok(Self {
            attacker: map.required_usize("attacker")?,
            defender: map.required_usize("defender")?,
        })
    }
}

/// One scenario block declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioBlockDeclaration {
    blocker: usize,
    attacker: usize,
}

impl ScenarioBlockDeclaration {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("block declaration")?;
        Ok(Self {
            blocker: map.required_usize("blocker")?,
            attacker: map.required_usize("attacker")?,
        })
    }
}

/// A scenario combat-damage target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioCombatDamageTarget {
    /// Damage assigned to a player by scenario player index.
    Player(usize),
    /// Damage assigned to an object by scenario object index.
    Object(usize),
}

/// One scenario combat-damage target and amount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioCombatDamageAssignment {
    target: ScenarioCombatDamageTarget,
    amount: u32,
}

impl ScenarioCombatDamageAssignment {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("combat damage assignment")?;
        let player = map.optional_usize("player")?;
        let object = map.optional_usize("object")?;
        let target = match (player, object) {
            (Some(index), None) => ScenarioCombatDamageTarget::Player(index),
            (None, Some(index)) => ScenarioCombatDamageTarget::Object(index),
            (None, None) => {
                return Err(ScenarioError::schema(
                    "combat damage assignment requires `player` or `object`".to_owned(),
                ));
            }
            (Some(_), Some(_)) => {
                return Err(ScenarioError::schema(
                    "combat damage assignment cannot target both player and object".to_owned(),
                ));
            }
        };
        Ok(Self {
            target,
            amount: map.required_u32("amount")?,
        })
    }
}

/// All scenario combat damage assigned by one source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioCombatDamageRequest {
    source: usize,
    assignments: Vec<ScenarioCombatDamageAssignment>,
}

impl ScenarioCombatDamageRequest {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("combat damage request")?;
        let mut assignments = Vec::new();
        for value in map
            .required("assignments")?
            .into_list("combat damage assignments")?
        {
            assignments.push(ScenarioCombatDamageAssignment::from_ron_value(value)?);
        }
        Ok(Self {
            source: map.required_usize("source")?,
            assignments,
        })
    }
}

/// A mana-pool or mana-payment quantity in WUBRG plus colorless order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ManaSpec {
    white: u32,
    blue: u32,
    black: u32,
    red: u32,
    green: u32,
    colorless: u32,
}

impl ManaSpec {
    /// Converts this scenario quantity to the kernel mana-pool type.
    #[must_use]
    pub const fn to_pool(self) -> ManaPool {
        ManaPool::new(
            self.white,
            self.blue,
            self.black,
            self.red,
            self.green,
            self.colorless,
        )
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("mana")?;
        Ok(Self {
            white: map.optional_u32("white")?.unwrap_or(0),
            blue: map.optional_u32("blue")?.unwrap_or(0),
            black: map.optional_u32("black")?.unwrap_or(0),
            red: map.optional_u32("red")?.unwrap_or(0),
            green: map.optional_u32("green")?.unwrap_or(0),
            colorless: map.optional_u32("colorless")?.unwrap_or(0),
        })
    }
}

/// A mana-cost quantity in WUBRG plus generic and X components.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ManaCostSpec {
    white: u32,
    blue: u32,
    black: u32,
    red: u32,
    green: u32,
    generic: u32,
    x_count: u32,
    x_value: u32,
}

impl ManaCostSpec {
    fn to_cost(self) -> ManaCost {
        ManaCost::new(
            self.white,
            self.blue,
            self.black,
            self.red,
            self.green,
            self.generic,
        )
        .with_x(self.x_count, self.x_value)
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("mana cost")?;
        Ok(Self {
            white: map.optional_u32("white")?.unwrap_or(0),
            blue: map.optional_u32("blue")?.unwrap_or(0),
            black: map.optional_u32("black")?.unwrap_or(0),
            red: map.optional_u32("red")?.unwrap_or(0),
            green: map.optional_u32("green")?.unwrap_or(0),
            generic: map.optional_u32("generic")?.unwrap_or(0),
            x_count: map.optional_u32("x_count")?.unwrap_or(0),
            x_value: map.optional_u32("x_value")?.unwrap_or(0),
        })
    }
}

/// Static combat keywords for a scenario creature.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CreatureKeywordSpec {
    first_strike: bool,
    double_strike: bool,
    trample: bool,
    deathtouch: bool,
    lifelink: bool,
    flying: bool,
    reach: bool,
    menace: bool,
    vigilance: bool,
    haste: bool,
    defender: bool,
    indestructible: bool,
    prowess: bool,
}

impl CreatureKeywordSpec {
    fn to_keywords(self) -> CreatureKeywords {
        let mut keywords = CreatureKeywords::none();
        if self.first_strike {
            keywords = keywords.with_first_strike();
        }
        if self.double_strike {
            keywords = keywords.with_double_strike();
        }
        if self.trample {
            keywords = keywords.with_trample();
        }
        if self.deathtouch {
            keywords = keywords.with_deathtouch();
        }
        if self.lifelink {
            keywords = keywords.with_lifelink();
        }
        if self.flying {
            keywords = keywords.with_flying();
        }
        if self.reach {
            keywords = keywords.with_reach();
        }
        if self.menace {
            keywords = keywords.with_menace();
        }
        if self.vigilance {
            keywords = keywords.with_vigilance();
        }
        if self.haste {
            keywords = keywords.with_haste();
        }
        if self.defender {
            keywords = keywords.with_defender();
        }
        if self.indestructible {
            keywords = keywords.with_indestructible();
        }
        if self.prowess {
            keywords = keywords.with_prowess();
        }
        keywords
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let mut spec = Self::default();
        for value in value.into_list("keywords")? {
            match value.into_string("keyword")?.as_str() {
                "first_strike" => spec.first_strike = true,
                "double_strike" => spec.double_strike = true,
                "trample" => spec.trample = true,
                "deathtouch" => spec.deathtouch = true,
                "lifelink" => spec.lifelink = true,
                "flying" => spec.flying = true,
                "reach" => spec.reach = true,
                "menace" => spec.menace = true,
                "vigilance" => spec.vigilance = true,
                "haste" => spec.haste = true,
                "defender" => spec.defender = true,
                "indestructible" => spec.indestructible = true,
                "prowess" => spec.prowess = true,
                other => {
                    return Err(ScenarioError::schema(format!(
                        "unsupported creature keyword `{other}`"
                    )));
                }
            }
        }
        Ok(spec)
    }
}

/// Scenario color set for layer assertions and effects.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ColorSpec {
    white: bool,
    blue: bool,
    black: bool,
    red: bool,
    green: bool,
}

impl ColorSpec {
    fn to_colors(self) -> ObjectColors {
        let mut colors = ObjectColors::none();
        if self.white {
            colors = colors.with_white();
        }
        if self.blue {
            colors = colors.with_blue();
        }
        if self.black {
            colors = colors.with_black();
        }
        if self.red {
            colors = colors.with_red();
        }
        if self.green {
            colors = colors.with_green();
        }
        colors
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let mut spec = Self::default();
        for value in value.into_list("colors")? {
            match value.into_string("color")?.as_str() {
                "white" => spec.white = true,
                "blue" => spec.blue = true,
                "black" => spec.black = true,
                "red" => spec.red = true,
                "green" => spec.green = true,
                other => {
                    return Err(ScenarioError::schema(format!(
                        "unsupported color `{other}`"
                    )));
                }
            }
        }
        Ok(spec)
    }
}

/// Scenario object-type set for layer assertions and effects.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TypeSpec {
    artifact: bool,
    creature: bool,
    enchantment: bool,
    instant: bool,
    land: bool,
    planeswalker: bool,
    sorcery: bool,
}

impl TypeSpec {
    fn to_types(self) -> ObjectTypes {
        let mut types = ObjectTypes::none();
        if self.artifact {
            types = types.with_artifact();
        }
        if self.creature {
            types = types.with_creature();
        }
        if self.enchantment {
            types = types.with_enchantment();
        }
        if self.instant {
            types = types.with_instant();
        }
        if self.land {
            types = types.with_land();
        }
        if self.planeswalker {
            types = types.with_planeswalker();
        }
        if self.sorcery {
            types = types.with_sorcery();
        }
        types
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let mut spec = Self::default();
        for value in value.into_list("types")? {
            match value.into_string("type")?.as_str() {
                "artifact" => spec.artifact = true,
                "creature" => spec.creature = true,
                "enchantment" => spec.enchantment = true,
                "instant" => spec.instant = true,
                "land" => spec.land = true,
                "planeswalker" => spec.planeswalker = true,
                "sorcery" => spec.sorcery = true,
                other => {
                    return Err(ScenarioError::schema(format!(
                        "unsupported object type `{other}`"
                    )));
                }
            }
        }
        Ok(spec)
    }
}

/// Scenario target requirement.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TargetRequirementSpec {
    kind: String,
    zone: Option<ZoneSpec>,
    controller: Option<String>,
    controller_player: Option<usize>,
    player_predicate: Option<String>,
    player: Option<usize>,
    required_types: Option<TypeSpec>,
    forbidden_types: Option<TypeSpec>,
    required_colors: Option<ColorSpec>,
    forbidden_colors: Option<ColorSpec>,
    required_keywords: Option<CreatureKeywordSpec>,
    forbidden_keywords: Option<CreatureKeywordSpec>,
}

impl TargetRequirementSpec {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("target requirement")?;
        Self::from_map(&map)
    }

    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            kind: map.required_string("kind")?,
            zone: match map.optional("zone")? {
                Some(value) => Some(ZoneSpec::from_ron_value(value)?),
                None => None,
            },
            controller: map.optional_string("controller")?,
            controller_player: map.optional_usize("controller_player")?,
            player_predicate: map.optional_string("player_predicate")?,
            player: map.optional_usize("player")?,
            required_types: match map.optional("required_types")? {
                Some(value) => Some(TypeSpec::from_ron_value(value)?),
                None => None,
            },
            forbidden_types: match map.optional("forbidden_types")? {
                Some(value) => Some(TypeSpec::from_ron_value(value)?),
                None => None,
            },
            required_colors: match map.optional("required_colors")? {
                Some(value) => Some(ColorSpec::from_ron_value(value)?),
                None => None,
            },
            forbidden_colors: match map.optional("forbidden_colors")? {
                Some(value) => Some(ColorSpec::from_ron_value(value)?),
                None => None,
            },
            required_keywords: match map.optional("required_keywords")? {
                Some(value) => Some(CreatureKeywordSpec::from_ron_value(value)?),
                None => None,
            },
            forbidden_keywords: match map.optional("forbidden_keywords")? {
                Some(value) => Some(CreatureKeywordSpec::from_ron_value(value)?),
                None => None,
            },
        })
    }

    fn to_requirement(&self, players: &[PlayerId]) -> Result<TargetRequirement, ScenarioError> {
        let kind = match self.kind.as_str() {
            "player" => TargetKind::Player,
            "permanent" => TargetKind::Permanent,
            "object_in_zone" => {
                let zone = self
                    .zone
                    .as_ref()
                    .ok_or_else(|| {
                        ScenarioError::schema("object_in_zone target requires zone".to_owned())
                    })?
                    .zone_id(players)?;
                TargetKind::ObjectInZone(zone)
            }
            other => {
                return Err(ScenarioError::schema(format!(
                    "unsupported target kind `{other}`"
                )));
            }
        };
        let mut requirement = TargetRequirement::new(kind);
        if matches!(kind, TargetKind::Player) {
            if self.player_predicate.is_some() || self.player.is_some() {
                requirement = requirement.with_player_predicate(self.player_predicate(players)?);
            }
        } else if self.has_object_predicate() {
            requirement = requirement.with_object_predicate(self.object_predicate(players)?);
        }
        Ok(requirement)
    }

    fn has_object_predicate(&self) -> bool {
        self.controller.is_some()
            || self.controller_player.is_some()
            || self.required_types.is_some()
            || self.forbidden_types.is_some()
            || self.required_colors.is_some()
            || self.forbidden_colors.is_some()
            || self.required_keywords.is_some()
            || self.forbidden_keywords.is_some()
    }

    fn player_predicate(
        &self,
        players: &[PlayerId],
    ) -> Result<PlayerTargetPredicate, ScenarioError> {
        match self
            .player_predicate
            .as_deref()
            .unwrap_or(if self.player.is_some() {
                "player"
            } else {
                "any"
            }) {
            "any" => Ok(PlayerTargetPredicate::Any),
            "you" => Ok(PlayerTargetPredicate::You),
            "opponent" => Ok(PlayerTargetPredicate::Opponent),
            "player" => Ok(PlayerTargetPredicate::Player(player_id(
                players,
                self.player.ok_or_else(|| {
                    ScenarioError::schema("player target predicate requires player".to_owned())
                })?,
                "target_requirement.player",
            )?)),
            other => Err(ScenarioError::schema(format!(
                "unsupported player target predicate `{other}`"
            ))),
        }
    }

    fn object_predicate(
        &self,
        players: &[PlayerId],
    ) -> Result<ObjectTargetPredicate, ScenarioError> {
        let mut predicate = ObjectTargetPredicate::any();
        predicate = predicate.with_controller(self.controller_predicate(players)?);
        if let Some(types) = self.required_types {
            predicate = predicate.with_required_types(types.to_types());
        }
        if let Some(types) = self.forbidden_types {
            predicate = predicate.with_forbidden_types(types.to_types());
        }
        if let Some(colors) = self.required_colors {
            predicate = predicate.with_required_colors(colors.to_colors());
        }
        if let Some(colors) = self.forbidden_colors {
            predicate = predicate.with_forbidden_colors(colors.to_colors());
        }
        if let Some(keywords) = self.required_keywords {
            predicate = predicate.with_required_keywords(keywords.to_keywords());
        }
        if let Some(keywords) = self.forbidden_keywords {
            predicate = predicate.with_forbidden_keywords(keywords.to_keywords());
        }
        Ok(predicate)
    }

    fn controller_predicate(
        &self,
        players: &[PlayerId],
    ) -> Result<TargetControllerPredicate, ScenarioError> {
        match self
            .controller
            .as_deref()
            .unwrap_or(if self.controller_player.is_some() {
                "player"
            } else {
                "any"
            }) {
            "any" => Ok(TargetControllerPredicate::Any),
            "you" => Ok(TargetControllerPredicate::You),
            "opponent" => Ok(TargetControllerPredicate::Opponent),
            "player" => Ok(TargetControllerPredicate::Player(player_id(
                players,
                self.controller_player.ok_or_else(|| {
                    ScenarioError::schema(
                        "controller player predicate requires controller_player".to_owned(),
                    )
                })?,
                "target_requirement.controller_player",
            )?)),
            other => Err(ScenarioError::schema(format!(
                "unsupported controller predicate `{other}`"
            ))),
        }
    }
}

/// Scenario target choice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TargetChoiceSpec {
    player: Option<usize>,
    object: Option<usize>,
}

impl TargetChoiceSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            player: map.optional_usize("target_player")?,
            object: map.optional_usize("target_object")?,
        })
    }

    fn to_choice(
        self,
        players: &[PlayerId],
        objects: &[ObjectId],
        phase: &str,
    ) -> Result<TargetChoice, ScenarioError> {
        match (self.player, self.object) {
            (Some(player), None) => Ok(TargetChoice::Player(player_id(players, player, phase)?)),
            (None, Some(object)) => Ok(TargetChoice::Object(object_id(objects, object, phase)?)),
            (Some(_), Some(_)) => Err(ScenarioError::schema(format!(
                "{phase} cannot target both player and object"
            ))),
            (None, None) => Err(ScenarioError::schema(format!(
                "{phase} requires target_player or target_object"
            ))),
        }
    }
}

/// Scenario spell target slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetSpec {
    requirement: TargetRequirementSpec,
    choice: TargetChoiceSpec,
}

impl TargetSpec {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("target")?;
        Ok(Self {
            requirement: TargetRequirementSpec::from_ron_value(map.required("requirement")?)?,
            choice: TargetChoiceSpec::from_map(&map)?,
        })
    }
}

/// Scenario targeting or combat restriction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RestrictionSpec {
    controller: usize,
    source_object: Option<usize>,
    effect: String,
    subject_object: Option<usize>,
    all_objects: bool,
    controlled_by: Option<usize>,
    colors: Option<ColorSpec>,
    cost: Option<ManaCostSpec>,
}

impl RestrictionSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            controller: map.required_usize("controller")?,
            source_object: map.optional_usize("source_object")?,
            effect: map.required_string("effect")?,
            subject_object: map.optional_usize("subject_object")?,
            all_objects: map.optional_bool("all_objects")?.unwrap_or(false),
            controlled_by: map.optional_usize("controlled_by")?,
            colors: match map.optional("colors")? {
                Some(value) => Some(ColorSpec::from_ron_value(value)?),
                None => None,
            },
            cost: match map.optional("cost")? {
                Some(value) => Some(ManaCostSpec::from_ron_value(value)?),
                None => None,
            },
        })
    }
}

/// Expected effective object characteristics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CharacteristicExpectation {
    object: usize,
    controller: Option<usize>,
    is_creature: Option<bool>,
    power: Option<i32>,
    toughness: Option<i32>,
    keywords: Option<CreatureKeywordSpec>,
    colors: Option<ColorSpec>,
    types: Option<TypeSpec>,
    text_marker: Option<u32>,
}

impl CharacteristicExpectation {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("characteristics expectation")?;
        Ok(Self {
            object: map.required_usize("object")?,
            controller: map.optional_usize("controller")?,
            is_creature: map.optional_bool("is_creature")?,
            power: map.optional_i32("power")?,
            toughness: map.optional_i32("toughness")?,
            keywords: match map.optional("keywords")? {
                Some(value) => Some(CreatureKeywordSpec::from_ron_value(value)?),
                None => None,
            },
            colors: match map.optional("colors")? {
                Some(value) => Some(ColorSpec::from_ron_value(value)?),
                None => None,
            },
            types: match map.optional("types")? {
                Some(value) => Some(TypeSpec::from_ron_value(value)?),
                None => None,
            },
            text_marker: map.optional_u32("text_marker")?,
        })
    }
}

/// Scenario continuous-effect registration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContinuousEffectSpec {
    controller: usize,
    source_object: Option<usize>,
    target_object: Option<usize>,
    all_objects: bool,
    operation: String,
    from_object: Option<usize>,
    player: Option<usize>,
    marker: Option<u32>,
    types: Option<TypeSpec>,
    colors: Option<ColorSpec>,
    keywords: Option<CreatureKeywordSpec>,
    power: Option<i32>,
    toughness: Option<i32>,
    timestamp: Option<u64>,
    dependencies: Vec<usize>,
}

impl ContinuousEffectSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            controller: map.required_usize("controller")?,
            source_object: map.optional_usize("source_object")?,
            target_object: map.optional_usize("target_object")?,
            all_objects: map.optional_bool("all_objects")?.unwrap_or(false),
            operation: map.required_string("operation")?,
            from_object: map.optional_usize("from_object")?,
            player: map.optional_usize("player")?,
            marker: map.optional_u32("marker")?,
            types: match map.optional("types")? {
                Some(value) => Some(TypeSpec::from_ron_value(value)?),
                None => None,
            },
            colors: match map.optional("colors")? {
                Some(value) => Some(ColorSpec::from_ron_value(value)?),
                None => None,
            },
            keywords: match map.optional("keywords")? {
                Some(value) => Some(CreatureKeywordSpec::from_ron_value(value)?),
                None => None,
            },
            power: map.optional_i32("power")?,
            toughness: map.optional_i32("toughness")?,
            timestamp: map.optional_u64("timestamp")?,
            dependencies: parse_usize_list(
                map.optional("dependencies")?
                    .unwrap_or_else(|| RonValue::List(Vec::new())),
                "register_continuous_effect.dependencies",
            )?,
        })
    }
}

/// Scenario activated-ability cost registration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivationCostSpec {
    mana: ManaCostSpec,
    tap_source: bool,
    sacrifice_source: bool,
    loyalty_delta: Option<i32>,
}

impl ActivationCostSpec {
    fn from_optional(value: Option<RonValue>) -> Result<Self, ScenarioError> {
        match value {
            Some(value) => Self::from_ron_value(value),
            None => Ok(Self::default()),
        }
    }

    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("activation cost")?;
        Ok(Self {
            mana: match map.optional("mana")? {
                Some(value) => ManaCostSpec::from_ron_value(value)?,
                None => ManaCostSpec::default(),
            },
            tap_source: map.optional_bool("tap_source")?.unwrap_or(false),
            sacrifice_source: map.optional_bool("sacrifice_source")?.unwrap_or(false),
            loyalty_delta: map.optional_i32("loyalty_delta")?,
        })
    }

    fn to_cost(&self) -> ActivationCost {
        let mut cost = ActivationCost::new(self.mana.to_cost());
        if self.tap_source {
            cost = cost.with_tap_source();
        }
        if self.sacrifice_source {
            cost = cost.with_sacrifice_source();
        }
        if let Some(delta) = self.loyalty_delta {
            cost = cost.with_loyalty_delta(delta);
        }
        cost
    }
}

/// Scenario activated-ability registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatedAbilitySpec {
    controller: usize,
    source_object: Option<usize>,
    timing: ActivationTiming,
    cost: ActivationCostSpec,
    effect: AbilityEffectSpec,
    mana_ability: bool,
}

impl ActivatedAbilitySpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            controller: map.required_usize("controller")?,
            source_object: map.optional_usize("source_object")?,
            timing: match map.optional_string("timing")? {
                Some(timing) => parse_activation_timing(&timing)?,
                None => ActivationTiming::Instant,
            },
            cost: ActivationCostSpec::from_optional(map.optional("cost")?)?,
            effect: AbilityEffectSpec::from_ron_value(map.required("effect")?)?,
            mana_ability: map.optional_bool("mana_ability")?.unwrap_or(false),
        })
    }
}

/// Scenario triggered-ability registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerSpec {
    controller: usize,
    source_object: Option<usize>,
    condition: String,
    object: Option<usize>,
    once: bool,
}

impl TriggerSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            controller: map.required_usize("controller")?,
            source_object: map.optional_usize("source_object")?,
            condition: map.required_string("condition")?,
            object: map.optional_usize("object")?,
            once: map.optional_bool("once")?.unwrap_or(false),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AbilityEffectSpec {
    operation: String,
    player: Option<usize>,
    mana: Option<ManaSpec>,
    amount: Option<u32>,
}

impl AbilityEffectSpec {
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        let map = value.into_map("activated ability effect")?;
        Ok(Self {
            operation: map.required_string("operation")?,
            player: map.optional_usize("player")?,
            mana: match map.optional("mana")? {
                Some(value) => Some(ManaSpec::from_ron_value(value)?),
                None => None,
            },
            amount: map.optional_u32("amount")?,
        })
    }
}

/// Scenario activated-ability cost modifier registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostModifierSpec {
    controller: usize,
    source_object: Option<usize>,
    scope: CostModifierScopeSpec,
    operation: CostModifierOperationSpec,
}

impl CostModifierSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        Ok(Self {
            controller: map.required_usize("controller")?,
            source_object: map.optional_usize("source_object")?,
            scope: CostModifierScopeSpec::from_map(map)?,
            operation: CostModifierOperationSpec::from_map(map)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CostModifierScopeSpec {
    All,
    Ability(usize),
    Source(usize),
    Controller(usize),
}

impl CostModifierScopeSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        match map
            .optional_string("scope")?
            .unwrap_or_else(|| "all".to_owned())
            .as_str()
        {
            "all" => Ok(Self::All),
            "ability" => Ok(Self::Ability(map.required_usize("ability")?)),
            "source" => Ok(Self::Source(map.required_usize("scope_source_object")?)),
            "controller" => Ok(Self::Controller(map.required_usize("player")?)),
            other => Err(ScenarioError::schema(format!(
                "unsupported cost modifier scope `{other}`"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CostModifierOperationSpec {
    AddManaCost(ManaCostSpec),
    AddGeneric(u32),
    ReduceGeneric(u32),
}

impl CostModifierOperationSpec {
    fn from_map(map: &RonMap) -> Result<Self, ScenarioError> {
        match map.required_string("operation")?.as_str() {
            "add_mana_cost" => Ok(Self::AddManaCost(ManaCostSpec::from_ron_value(
                map.required("cost")?,
            )?)),
            "add_generic" => Ok(Self::AddGeneric(map.required_u32("amount")?)),
            "reduce_generic" => Ok(Self::ReduceGeneric(map.required_u32("amount")?)),
            other => Err(ScenarioError::schema(format!(
                "unsupported cost modifier operation `{other}`"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaybePlayerExpectation {
    None,
    Player(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaybeStepExpectation {
    None,
    Step(Step),
}

/// Expected final scenario facts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScenarioExpect {
    zone_counts: Vec<ZoneCountExpectation>,
    players: Vec<PlayerExpectation>,
    characteristics: Vec<CharacteristicExpectation>,
    outcome: Option<OutcomeExpectation>,
    active_player: Option<MaybePlayerExpectation>,
    priority_player: Option<MaybePlayerExpectation>,
    current_step: Option<MaybeStepExpectation>,
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

    /// Returns expectations with one object-characteristics assertion appended.
    #[must_use]
    pub fn with_characteristics(mut self, expectation: CharacteristicExpectation) -> Self {
        self.characteristics.push(expectation);
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

    /// Returns final object-characteristics expectations.
    #[must_use]
    pub fn characteristics(&self) -> &[CharacteristicExpectation] {
        &self.characteristics
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
        if let Some(characteristics) = map.optional("characteristics")? {
            for value in characteristics.into_list("expect.characteristics")? {
                expect =
                    expect.with_characteristics(CharacteristicExpectation::from_ron_value(value)?);
            }
        }
        if let Some(outcome) = map.optional("outcome")? {
            expect = expect.with_outcome(OutcomeExpectation::from_ron_value(outcome)?);
        }
        if let Some(active_player) = map.optional("active_player")? {
            expect.active_player = Some(parse_maybe_player(active_player, "active_player")?);
        }
        if let Some(priority_player) = map.optional("priority_player")? {
            expect.priority_player = Some(parse_maybe_player(priority_player, "priority_player")?);
        }
        if let Some(current_step) = map.optional("current_step")? {
            expect.current_step = Some(parse_maybe_step(current_step)?);
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
    mana: Option<ManaSpec>,
}

impl PlayerExpectation {
    /// Creates a player expectation.
    #[must_use]
    pub const fn new(player: usize) -> Self {
        Self {
            player,
            life: None,
            poison: None,
            mana: None,
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

    /// Returns this expectation with a mana-pool assertion.
    #[must_use]
    pub const fn with_mana(mut self, mana: ManaSpec) -> Self {
        self.mana = Some(mana);
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
        if let Some(mana) = map.optional("mana")? {
            expectation = expectation.with_mana(ManaSpec::from_ron_value(mana)?);
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
    fn from_ron_value(value: RonValue) -> Result<Self, ScenarioError> {
        match value {
            RonValue::String(input) => Self::parse(&input),
            RonValue::Map(map) => {
                let status = map.required_string("status")?;
                match status.as_str() {
                    "won" | "Won" => Ok(Self::Won {
                        player: map.required_usize("player")?,
                    }),
                    _ => Self::parse(&status),
                }
            }
            _ => Err(ScenarioError::schema(
                "outcome must be a string or map".to_owned(),
            )),
        }
    }

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
    replacements: Vec<ReplacementEffectId>,
    continuous_effects: Vec<ContinuousEffectId>,
    activated_abilities: Vec<ActivatedAbilityId>,
    triggers: Vec<TriggerId>,
    stack_entries: Vec<StackEntryId>,
    failures: Vec<ScenarioFailure>,
    steps: Vec<StepRecord>,
}

fn execute_scenario(scenario: &Scenario, check_expectations: bool) -> ScenarioReport {
    let mut context = RunContext {
        state: GameState::new(),
        players: Vec::new(),
        objects: Vec::new(),
        replacements: Vec::new(),
        continuous_effects: Vec::new(),
        activated_abilities: Vec::new(),
        triggers: Vec::new(),
        stack_entries: Vec::new(),
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
    match step {
        ScenarioStep::AssertCharacteristics { expectation } => {
            check_characteristic_expectation(&label, expectation, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertObjectTapped { object, tapped } => {
            check_object_tapped_expectation(&label, *object, *tapped, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertObjectLoyalty { object, loyalty } => {
            check_object_loyalty_expectation(&label, *object, *loyalty, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertObjectCounters {
            object,
            kind,
            count,
        } => {
            check_object_counter_expectation(&label, *object, *kind, *count, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertObjectZone { object, zone } => {
            check_object_zone_expectation(&label, *object, *zone, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertZoneOrder { zone, objects } => {
            check_zone_order_expectation(&label, *zone, objects, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertObjectFlags {
            object,
            token,
            copy,
            copy_source,
        } => {
            check_object_flags_expectation(&label, *object, *token, *copy, *copy_source, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertAttachedTo {
            attachment,
            target_object,
        } => {
            check_attached_to_expectation(&label, *attachment, *target_object, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertPendingTriggers { count } => {
            check_pending_triggers_expectation(&label, *count, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertStackEntryFlags {
            entry,
            kicked,
            flashback,
        } => {
            check_stack_entry_flags_expectation(&label, *entry, *kicked, *flashback, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertTurnOrder { order } => {
            check_turn_order_expectation(&label, order, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertRangeOfInfluence { mode } => {
            check_range_of_influence_expectation(&label, mode, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertCommander {
            object,
            commander,
            colors,
            cast_count,
            tax_generic,
        } => {
            check_commander_expectation(
                &label,
                *object,
                *commander,
                *colors,
                *cast_count,
                *tax_generic,
                context,
            );
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertCommanderIdentityLegal {
            player,
            object,
            expected,
        } => {
            check_commander_identity_legal_expectation(
                &label, *player, *object, *expected, context,
            );
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertCanTarget {
            player,
            source_object,
            requirement,
            target,
            expected,
        } => {
            check_can_target(
                &label,
                *player,
                *source_object,
                requirement,
                *target,
                *expected,
                context,
            );
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertWardCost {
            target_object,
            cost,
        } => {
            check_ward_cost(&label, *target_object, cost, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertCanAttack {
            player,
            attack,
            expected,
        } => {
            check_can_attack(&label, *player, *attack, *expected, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        ScenarioStep::AssertCanBlock {
            player,
            block,
            expected,
        } => {
            check_can_block(&label, *player, *block, *expected, context);
            record_outcome(&label, Outcome::Applied, context);
            return;
        }
        _ => {}
    }
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
    match &outcome {
        Outcome::Failed(error) => {
            context.failures.push(ScenarioFailure::new(
                label.clone(),
                format!("action failed: {error:?}"),
            ));
        }
        Outcome::ReplacementEffectRegistered(replacement) => {
            context.replacements.push(*replacement);
        }
        Outcome::ContinuousEffectRegistered(effect) => {
            context.continuous_effects.push(*effect);
        }
        Outcome::ActivatedAbilityRegistered(ability) => {
            context.activated_abilities.push(*ability);
        }
        Outcome::TriggerRegistered(trigger) => {
            context.triggers.push(*trigger);
        }
        Outcome::ObjectCreated(object) => {
            context.objects.push(*object);
        }
        Outcome::StackEntryAdded(entry) => {
            context.stack_entries.push(*entry);
        }
        Outcome::StackEntriesAdded(entries) => {
            context.stack_entries.extend(entries.iter().copied());
        }
        _ => {}
    }
    record_outcome(&label, outcome, context);
}

fn action_for_step(step: &ScenarioStep, context: &RunContext) -> Result<Action, ScenarioError> {
    match step {
        ScenarioStep::DecideTurnOrder => Ok(Action::DecideTurnOrder),
        ScenarioStep::SetTurnOrder { order } => {
            let mut players = Vec::with_capacity(order.len());
            for index in order {
                players.push(player_id(&context.players, *index, "set_turn_order")?);
            }
            Ok(Action::SetTurnOrder { order: players })
        }
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
        ScenarioStep::AddPoisonCounters { player, amount } => Ok(Action::AddPoisonCounters {
            player: player_id(&context.players, *player, "add_poison_counters")?,
            amount: *amount,
        }),
        ScenarioStep::AddMana { player, mana } => Ok(Action::AddManaToPool {
            player: player_id(&context.players, *player, "add_mana")?,
            mana: mana.to_pool(),
        }),
        ScenarioStep::ClearMana { player } => Ok(Action::ClearManaPool {
            player: player_id(&context.players, *player, "clear_mana")?,
        }),
        ScenarioStep::PayManaAuto { player, cost } => {
            let player_id = player_id(&context.players, *player, "pay_mana_auto")?;
            let cost = cost.to_cost();
            let available = context
                .state
                .mana_pool(player_id)
                .map_err(|error| ScenarioError::schema(format!("mana pool failed: {error:?}")))?;
            let plan = auto_payment_plan(available, cost)
                .map_err(|error| {
                    ScenarioError::schema(format!("payment planning failed: {error:?}"))
                })?
                .ok_or_else(|| {
                    ScenarioError::schema("pay_mana_auto has no valid payment plan".to_owned())
                })?;
            Ok(Action::PayMana {
                player: player_id,
                cost,
                plan,
            })
        }
        ScenarioStep::Scry {
            player,
            count,
            bottom,
        } => {
            let mut objects = Vec::with_capacity(bottom.len());
            for index in bottom {
                objects.push(object_id(&context.objects, *index, "scry.bottom")?);
            }
            Ok(Action::Scry {
                player: player_id(&context.players, *player, "scry.player")?,
                count: *count,
                bottom: objects,
            })
        }
        ScenarioStep::Surveil {
            player,
            count,
            graveyard,
        } => {
            let mut objects = Vec::with_capacity(graveyard.len());
            for index in graveyard {
                objects.push(object_id(&context.objects, *index, "surveil.graveyard")?);
            }
            Ok(Action::Surveil {
                player: player_id(&context.players, *player, "surveil.player")?,
                count: *count,
                graveyard: objects,
            })
        }
        ScenarioStep::CycleAuto {
            player,
            object,
            cost,
        } => {
            let player = player_id(&context.players, *player, "cycle_auto.player")?;
            let object = object_id(&context.objects, *object, "cycle_auto.object")?;
            let cost = cost.to_cost();
            let available = context.state.mana_pool(player).map_err(|error| {
                ScenarioError::schema(format!("cycle mana pool failed: {error:?}"))
            })?;
            let payment = auto_payment_plan(available, cost)
                .map_err(|error| {
                    ScenarioError::schema(format!("cycle payment planning failed: {error:?}"))
                })?
                .ok_or_else(|| {
                    ScenarioError::schema("cycle_auto has no valid payment plan".to_owned())
                })?;
            Ok(Action::Cycle {
                player,
                object,
                cost,
                payment,
            })
        }
        ScenarioStep::AttachObject {
            attachment,
            target_object,
        } => Ok(Action::AttachObject {
            attachment: object_id(&context.objects, *attachment, "attach_object.attachment")?,
            target: match target_object {
                Some(index) => Some(object_id(
                    &context.objects,
                    *index,
                    "attach_object.target_object",
                )?),
                None => None,
            },
        }),
        ScenarioStep::EquipAuto {
            player,
            equipment,
            target_object,
            cost,
        } => {
            let player = player_id(&context.players, *player, "equip_auto.player")?;
            let cost = cost.to_cost();
            let available = context.state.mana_pool(player).map_err(|error| {
                ScenarioError::schema(format!("equip mana pool failed: {error:?}"))
            })?;
            let payment = auto_payment_plan(available, cost)
                .map_err(|error| {
                    ScenarioError::schema(format!("equip payment planning failed: {error:?}"))
                })?
                .ok_or_else(|| {
                    ScenarioError::schema("equip_auto has no valid payment plan".to_owned())
                })?;
            Ok(Action::Equip {
                player,
                equipment: object_id(&context.objects, *equipment, "equip_auto.equipment")?,
                target: object_id(&context.objects, *target_object, "equip_auto.target_object")?,
                cost,
                payment,
            })
        }
        ScenarioStep::MoveObject { object, zone } => Ok(Action::MoveObject {
            object: object_id(&context.objects, *object, "move_object")?,
            to: zone.zone_id(&context.players)?,
        }),
        ScenarioStep::CreateToken {
            card,
            owner,
            controller,
            power,
            toughness,
            keywords,
        } => {
            let base = match (*power, *toughness) {
                (Some(power), Some(toughness)) => Some(
                    BaseCreatureCharacteristics::new(power, toughness)
                        .with_keywords(keywords.to_keywords()),
                ),
                (None, None) => None,
                _ => {
                    return Err(ScenarioError::schema(
                        "create_token requires both power and toughness or neither".to_owned(),
                    ));
                }
            };
            Ok(Action::CreateToken {
                card: CardId::new(*card),
                owner: player_id(&context.players, *owner, "create_token.owner")?,
                controller: player_id(&context.players, *controller, "create_token.controller")?,
                base_object: BaseObjectCharacteristics::new(
                    if base.is_some() {
                        ObjectTypes::none().with_creature()
                    } else {
                        ObjectTypes::none()
                    },
                    ObjectColors::none(),
                ),
                base,
            })
        }
        ScenarioStep::CreatePermanentCopy {
            source,
            owner,
            controller,
            token,
        } => Ok(Action::CreatePermanentCopy {
            source: object_id(&context.objects, *source, "create_permanent_copy.source")?,
            owner: player_id(&context.players, *owner, "create_permanent_copy.owner")?,
            controller: player_id(
                &context.players,
                *controller,
                "create_permanent_copy.controller",
            )?,
            token: *token,
        }),
        ScenarioStep::CopyStackEntry { player, entry } => Ok(Action::CopyStackEntry {
            player: player_id(&context.players, *player, "copy_stack_entry.player")?,
            entry: stack_entry_id(&context.stack_entries, *entry, "copy_stack_entry.entry")?,
        }),
        ScenarioStep::SetBaseCreature {
            object,
            power,
            toughness,
            keywords,
        } => Ok(Action::SetBaseCreatureCharacteristics {
            object: object_id(&context.objects, *object, "set_base_creature")?,
            base: BaseCreatureCharacteristics::new(*power, *toughness)
                .with_keywords(keywords.to_keywords()),
        }),
        ScenarioStep::ClearBaseCreature { object } => {
            Ok(Action::ClearBaseCreatureCharacteristics {
                object: object_id(&context.objects, *object, "clear_base_creature")?,
            })
        }
        ScenarioStep::SetObjectTapped { object, tapped } => Ok(Action::SetObjectTapped {
            object: object_id(&context.objects, *object, "set_object_tapped")?,
            tapped: *tapped,
        }),
        ScenarioStep::SetObjectLoyalty { object, loyalty } => Ok(Action::SetObjectLoyalty {
            object: object_id(&context.objects, *object, "set_object_loyalty")?,
            loyalty: *loyalty,
        }),
        ScenarioStep::SetObjectColorIdentity { object, colors } => {
            Ok(Action::SetObjectColorIdentity {
                object: object_id(&context.objects, *object, "set_object_color_identity")?,
                colors: colors.to_colors(),
            })
        }
        ScenarioStep::DesignateCommander { object, colors } => Ok(Action::DesignateCommander {
            object: object_id(&context.objects, *object, "designate_commander")?,
            color_identity: colors.to_colors(),
        }),
        ScenarioStep::RecordCommanderCast { object } => Ok(Action::RecordCommanderCast {
            object: object_id(&context.objects, *object, "record_commander_cast")?,
        }),
        ScenarioStep::ValidateCommanderColorIdentity { player, objects } => {
            let mut object_ids = Vec::with_capacity(objects.len());
            for object in objects {
                object_ids.push(object_id(
                    &context.objects,
                    *object,
                    "validate_commander_color_identity.object",
                )?);
            }
            Ok(Action::ValidateCommanderColorIdentity {
                player: player_id(
                    &context.players,
                    *player,
                    "validate_commander_color_identity.player",
                )?,
                objects: object_ids,
            })
        }
        ScenarioStep::AddObjectCounters {
            object,
            kind,
            amount,
        } => Ok(Action::AddObjectCounters {
            object: object_id(&context.objects, *object, "add_object_counters")?,
            kind: *kind,
            amount: *amount,
        }),
        ScenarioStep::RemoveObjectCounters {
            object,
            kind,
            amount,
        } => Ok(Action::RemoveObjectCounters {
            object: object_id(&context.objects, *object, "remove_object_counters")?,
            kind: *kind,
            amount: *amount,
        }),
        ScenarioStep::MarkDamage { object, amount } => Ok(Action::MarkDamageOnObject {
            object: object_id(&context.objects, *object, "mark_damage")?,
            amount: *amount,
        }),
        ScenarioStep::RegisterDamageReplacement {
            controller,
            source_object,
            target_player,
            target_object,
            combat_only,
            operation,
            amount,
            once,
            self_replacement,
        } => {
            let controller = player_id(&context.players, *controller, "register_replacement")?;
            let source_filter = match source_object {
                Some(index) => ReplacementSourceFilter::Object(object_id(
                    &context.objects,
                    *index,
                    "register_replacement.source_object",
                )?),
                None => ReplacementSourceFilter::Any,
            };
            let target = replacement_target_filter(
                &context.players,
                &context.objects,
                *target_player,
                *target_object,
            )?;
            let condition = ReplacementCondition::DamageWouldBeDealt {
                source: source_filter,
                target,
                combat_only: *combat_only,
            };
            let operation = replacement_operation(operation, *amount)?;
            let mut definition = ReplacementDefinition::new(controller, condition, operation);
            if let Some(index) = source_object {
                definition = definition.with_source(object_id(
                    &context.objects,
                    *index,
                    "register_replacement.source_object",
                )?);
            }
            if *once {
                definition = definition.with_duration(ReplacementDuration::Once);
            }
            if *self_replacement {
                definition = definition.with_self_replacement();
            }
            Ok(Action::RegisterReplacementEffect { definition })
        }
        ScenarioStep::SetReplacementOrder { chooser, order } => {
            let mut ids = Vec::with_capacity(order.len());
            for index in order {
                ids.push(replacement_id(
                    &context.replacements,
                    *index,
                    "set_replacement_order",
                )?);
            }
            Ok(Action::SetReplacementChoiceOrder {
                chooser: player_id(&context.players, *chooser, "set_replacement_order")?,
                order: ids,
            })
        }
        ScenarioStep::RegisterContinuousEffect { spec } => {
            let definition = continuous_effect_definition(spec, context)?;
            Ok(Action::RegisterContinuousEffect { definition })
        }
        ScenarioStep::RegisterActivatedAbility { spec } => {
            let definition = activated_ability_definition(spec, context)?;
            Ok(Action::RegisterActivatedAbility { definition })
        }
        ScenarioStep::RegisterCostModifier { spec } => {
            let definition = cost_modifier_definition(spec, context)?;
            Ok(Action::RegisterCostModifier { definition })
        }
        ScenarioStep::RegisterRestriction { spec } => {
            let definition = restriction_definition(spec, context)?;
            Ok(Action::RegisterRestriction { definition })
        }
        ScenarioStep::RegisterTriggeredAbility { spec } => {
            let definition = trigger_definition(spec, context)?;
            Ok(Action::RegisterTriggeredAbility { definition })
        }
        ScenarioStep::PutPendingTriggersOnStack => Ok(Action::PutPendingTriggeredAbilitiesOnStack),
        ScenarioStep::ActivateAbilityAuto { player, ability } => {
            let player = player_id(&context.players, *player, "activate_ability_auto.player")?;
            let ability = activated_ability_id(
                &context.activated_abilities,
                *ability,
                "activate_ability_auto.ability",
            )?;
            let cost = context
                .state
                .effective_activation_cost(ability)
                .map_err(|error| {
                    ScenarioError::schema(format!("activation cost failed: {error:?}"))
                })?;
            let available = context.state.mana_pool(player).map_err(|error| {
                ScenarioError::schema(format!("activation mana pool failed: {error:?}"))
            })?;
            let payment = auto_payment_plan(available, cost.mana())
                .map_err(|error| {
                    ScenarioError::schema(format!("activation payment planning failed: {error:?}"))
                })?
                .ok_or_else(|| {
                    ScenarioError::schema(
                        "activate_ability_auto has no valid payment plan".to_owned(),
                    )
                })?;
            Ok(Action::ActivateAbility {
                player,
                ability,
                payment,
            })
        }
        ScenarioStep::CastSpellAuto {
            player,
            object,
            kind,
            timing,
            cost,
            flash,
            kicker,
            flashback,
            targets,
        } => {
            let player = player_id(&context.players, *player, "cast_spell_auto.player")?;
            let object = object_id(&context.objects, *object, "cast_spell_auto.object")?;
            let cost = cost.to_cost();
            let mut requirements = Vec::with_capacity(targets.len());
            let mut choices = Vec::with_capacity(targets.len());
            for target in targets {
                requirements.push(target.requirement.to_requirement(&context.players)?);
                choices.push(target.choice.to_choice(
                    &context.players,
                    &context.objects,
                    "cast_spell_auto.target",
                )?);
            }
            let placeholder_payment = auto_payment_plan(
                context.state.mana_pool(player).map_err(|error| {
                    ScenarioError::schema(format!("cast mana pool failed: {error:?}"))
                })?,
                ManaCostSpec::default().to_cost(),
            )
            .map_err(|error| {
                ScenarioError::schema(format!("cast payment planning failed: {error:?}"))
            })?
            .ok_or_else(|| {
                ScenarioError::schema("cast_spell_auto has no zero-cost payment plan".to_owned())
            })?;
            let mut request = CastSpellRequest::new(
                parse_stack_object_kind(kind)?,
                parse_spell_timing(timing)?,
                cost,
                placeholder_payment,
            );
            if *flash {
                request = request.with_flash();
            }
            if let Some(kicker) = kicker {
                request = request.with_kicker(kicker.to_cost());
            }
            if let Some(flashback) = flashback {
                request = request.with_flashback(flashback.to_cost());
            }
            let effective_cost = context
                .state
                .effective_spell_request_cost(player, object, &request)
                .map_err(|error| ScenarioError::schema(format!("spell cost failed: {error:?}")))?;
            let available = context.state.mana_pool(player).map_err(|error| {
                ScenarioError::schema(format!("cast mana pool failed: {error:?}"))
            })?;
            let payment = auto_payment_plan(available, effective_cost)
                .map_err(|error| {
                    ScenarioError::schema(format!("cast payment planning failed: {error:?}"))
                })?
                .ok_or_else(|| {
                    ScenarioError::schema("cast_spell_auto has no valid payment plan".to_owned())
                })?;
            request = CastSpellRequest::new(
                parse_stack_object_kind(kind)?,
                parse_spell_timing(timing)?,
                cost,
                payment,
            );
            if *flash {
                request = request.with_flash();
            }
            if let Some(kicker) = kicker {
                request = request.with_kicker(kicker.to_cost());
            }
            if let Some(flashback) = flashback {
                request = request.with_flashback(flashback.to_cost());
            }
            request = request.with_targets(requirements, choices);
            Ok(Action::CastSpell {
                player,
                object,
                request,
            })
        }
        ScenarioStep::AssertCharacteristics { .. }
        | ScenarioStep::AssertObjectTapped { .. }
        | ScenarioStep::AssertObjectLoyalty { .. }
        | ScenarioStep::AssertObjectCounters { .. }
        | ScenarioStep::AssertObjectZone { .. }
        | ScenarioStep::AssertZoneOrder { .. }
        | ScenarioStep::AssertObjectFlags { .. }
        | ScenarioStep::AssertAttachedTo { .. }
        | ScenarioStep::AssertPendingTriggers { .. }
        | ScenarioStep::AssertStackEntryFlags { .. }
        | ScenarioStep::AssertTurnOrder { .. }
        | ScenarioStep::AssertRangeOfInfluence { .. }
        | ScenarioStep::AssertCommander { .. }
        | ScenarioStep::AssertCommanderIdentityLegal { .. }
        | ScenarioStep::AssertCanTarget { .. }
        | ScenarioStep::AssertWardCost { .. }
        | ScenarioStep::AssertCanAttack { .. }
        | ScenarioStep::AssertCanBlock { .. } => Ok(Action::CheckStateBasedActions),
        ScenarioStep::DeclareAttackers { player, attacks } => {
            let mut declarations = Vec::with_capacity(attacks.len());
            for attack in attacks {
                declarations.push(AttackDeclaration::new(
                    object_id(
                        &context.objects,
                        attack.attacker,
                        "declare_attackers.attacker",
                    )?,
                    player_id(
                        &context.players,
                        attack.defender,
                        "declare_attackers.defender",
                    )?,
                ));
            }
            Ok(Action::DeclareAttackers {
                player: player_id(&context.players, *player, "declare_attackers.player")?,
                attacks: declarations,
            })
        }
        ScenarioStep::DeclareBlockers { player, blocks } => {
            let mut declarations = Vec::with_capacity(blocks.len());
            for block in blocks {
                declarations.push(BlockDeclaration::new(
                    object_id(&context.objects, block.blocker, "declare_blockers.blocker")?,
                    object_id(
                        &context.objects,
                        block.attacker,
                        "declare_blockers.attacker",
                    )?,
                ));
            }
            Ok(Action::DeclareBlockers {
                defending_player: player_id(&context.players, *player, "declare_blockers.player")?,
                blocks: declarations,
            })
        }
        ScenarioStep::AssignCombatDamage { assignments } => {
            let mut requests = Vec::with_capacity(assignments.len());
            for request in assignments {
                let mut damage_assignments = Vec::with_capacity(request.assignments.len());
                for assignment in &request.assignments {
                    let target = match assignment.target {
                        ScenarioCombatDamageTarget::Player(index) => CombatDamageTarget::Player(
                            player_id(&context.players, index, "assign_combat_damage.player")?,
                        ),
                        ScenarioCombatDamageTarget::Object(index) => CombatDamageTarget::Object(
                            object_id(&context.objects, index, "assign_combat_damage.object")?,
                        ),
                    };
                    damage_assignments.push(CombatDamageAssignment::new(target, assignment.amount));
                }
                requests.push(CombatDamageAssignmentRequest::new(
                    object_id(
                        &context.objects,
                        request.source,
                        "assign_combat_damage.source",
                    )?,
                    damage_assignments,
                ));
            }
            Ok(Action::AssignCombatDamage {
                assignments: requests,
            })
        }
        ScenarioStep::RequestCleanupPriority => Ok(Action::RequestCleanupPriority),
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
                if let Some(mana) = expectation.mana {
                    match context.state.mana_pool(player) {
                        Ok(actual) if actual == mana.to_pool() => {}
                        Ok(actual) => context.failures.push(ScenarioFailure::new(
                            "expect.players",
                            format!(
                                "player {} expected mana {}, found {}",
                                expectation.player,
                                format_mana_pool(mana.to_pool()),
                                format_mana_pool(actual)
                            ),
                        )),
                        Err(error) => context.failures.push(ScenarioFailure::new(
                            "expect.players",
                            format!("player {} mana check failed: {error:?}", expectation.player),
                        )),
                    }
                }
            }
            Err(error) => context
                .failures
                .push(ScenarioFailure::new("expect.players", error.to_string())),
        }
    }
    for expectation in &scenario.expect.characteristics {
        check_characteristic_expectation("expect.characteristics", expectation, context);
    }
    if let Some(outcome) = scenario.expect.outcome {
        check_outcome(outcome, context);
    }
    if let Some(active_player) = scenario.expect.active_player {
        check_maybe_player(
            "expect.active_player",
            active_player,
            context.state.active_player(),
            context,
        );
    }
    if let Some(priority_player) = scenario.expect.priority_player {
        check_maybe_player(
            "expect.priority_player",
            priority_player,
            context.state.priority_player(),
            context,
        );
    }
    if let Some(current_step) = scenario.expect.current_step {
        check_maybe_step(current_step, context);
    }
}

fn check_maybe_player(
    phase: &str,
    expectation: MaybePlayerExpectation,
    actual: Option<PlayerId>,
    context: &mut RunContext,
) {
    match expectation {
        MaybePlayerExpectation::None if actual.is_none() => {}
        MaybePlayerExpectation::None => context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected no player, found {actual:?}"),
        )),
        MaybePlayerExpectation::Player(player) => {
            match player_id(&context.players, player, phase) {
                Ok(expected) if actual == Some(expected) => {}
                Ok(expected) => context.failures.push(ScenarioFailure::new(
                    phase,
                    format!("expected player {}, found {actual:?}", expected.index()),
                )),
                Err(error) => context
                    .failures
                    .push(ScenarioFailure::new(phase, error.to_string())),
            }
        }
    }
}

fn check_maybe_step(expectation: MaybeStepExpectation, context: &mut RunContext) {
    match expectation {
        MaybeStepExpectation::None if context.state.current_step().is_none() => {}
        MaybeStepExpectation::None => context.failures.push(ScenarioFailure::new(
            "expect.current_step",
            format!(
                "expected no current step, found {:?}",
                context.state.current_step()
            ),
        )),
        MaybeStepExpectation::Step(step) if context.state.current_step() == Some(step) => {}
        MaybeStepExpectation::Step(step) => context.failures.push(ScenarioFailure::new(
            "expect.current_step",
            format!(
                "expected current step {step:?}, found {:?}",
                context.state.current_step()
            ),
        )),
    }
}

fn check_characteristic_expectation(
    phase: &str,
    expectation: &CharacteristicExpectation,
    context: &mut RunContext,
) {
    let object = match object_id(&context.objects, expectation.object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = match context.state.object_characteristics(object) {
        Ok(actual) => actual,
        Err(error) => {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} characteristics failed: {error:?}",
                    expectation.object
                ),
            ));
            return;
        }
    };
    if let Some(controller) = expectation.controller {
        match player_id(&context.players, controller, phase) {
            Ok(expected) if actual.controller() == expected => {}
            Ok(expected) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected controller {}, found {}",
                    expectation.object,
                    expected.index(),
                    actual.controller().index()
                ),
            )),
            Err(error) => context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string())),
        }
    }
    if let Some(is_creature) = expectation.is_creature {
        if actual.is_creature() != is_creature {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected is_creature {}, found {}",
                    expectation.object,
                    is_creature,
                    actual.is_creature()
                ),
            ));
        }
    }
    if let Some(power) = expectation.power {
        match actual.creature() {
            Some(creature) if creature.power() == power => {}
            Some(creature) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected power {}, found {}",
                    expectation.object,
                    power,
                    creature.power()
                ),
            )),
            None => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected power {}, found noncreature",
                    expectation.object, power
                ),
            )),
        }
    }
    if let Some(toughness) = expectation.toughness {
        match actual.creature() {
            Some(creature) if creature.toughness() == toughness => {}
            Some(creature) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected toughness {}, found {}",
                    expectation.object,
                    toughness,
                    creature.toughness()
                ),
            )),
            None => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected toughness {}, found noncreature",
                    expectation.object, toughness
                ),
            )),
        }
    }
    if let Some(keywords) = expectation.keywords {
        match actual.creature() {
            Some(creature) if creature.keywords() == keywords.to_keywords() => {}
            Some(creature) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected keywords {:?}, found {:?}",
                    expectation.object,
                    keywords.to_keywords(),
                    creature.keywords()
                ),
            )),
            None => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected creature keywords, found noncreature",
                    expectation.object
                ),
            )),
        }
    }
    if let Some(colors) = expectation.colors {
        if actual.colors() != colors.to_colors() {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected colors {:?}, found {:?}",
                    expectation.object,
                    colors.to_colors(),
                    actual.colors()
                ),
            ));
        }
    }
    if let Some(types) = expectation.types {
        if actual.types() != types.to_types() {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected types {:?}, found {:?}",
                    expectation.object,
                    types.to_types(),
                    actual.types()
                ),
            ));
        }
    }
    if let Some(text_marker) = expectation.text_marker {
        if actual.text_marker() != text_marker {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {} expected text marker {}, found {}",
                    expectation.object,
                    text_marker,
                    actual.text_marker()
                ),
            ));
        }
    }
}

fn check_object_tapped_expectation(
    phase: &str,
    object: usize,
    expected: bool,
    context: &mut RunContext,
) {
    let checked_object = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let Some(record) = context.state.object(checked_object) else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} is missing from state"),
        ));
        return;
    };
    if record.tapped() != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "object {object} expected tapped {}, found {}",
                expected,
                record.tapped()
            ),
        ));
    }
}

fn check_object_loyalty_expectation(
    phase: &str,
    object: usize,
    expected: Option<i32>,
    context: &mut RunContext,
) {
    let checked_object = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let Some(record) = context.state.object(checked_object) else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} is missing from state"),
        ));
        return;
    };
    if record.loyalty() != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "object {object} expected loyalty {:?}, found {:?}",
                expected,
                record.loyalty()
            ),
        ));
    }
}

fn check_object_counter_expectation(
    phase: &str,
    object: usize,
    kind: CounterKind,
    expected: u32,
    context: &mut RunContext,
) {
    let object_id = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = context.state.object_counter_count(object_id, kind);
    if actual != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} expected {kind:?} counters {expected}, found {actual}"),
        ));
    }
}

fn check_object_zone_expectation(
    phase: &str,
    object: usize,
    expected: ZoneSpec,
    context: &mut RunContext,
) {
    let object_id = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let expected = match expected.zone_id(&context.players) {
        Ok(zone) => zone,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = context.state.object_zone(object_id);
    if actual != Some(expected) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} expected zone {expected:?}, found {actual:?}"),
        ));
    }
}

fn check_zone_order_expectation(
    phase: &str,
    zone: ZoneSpec,
    objects: &[usize],
    context: &mut RunContext,
) {
    let expected_zone = match zone.zone_id(&context.players) {
        Ok(zone) => zone,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let mut expected = Vec::with_capacity(objects.len());
    for object in objects {
        match object_id(&context.objects, *object, phase) {
            Ok(object) => expected.push(object),
            Err(error) => {
                context
                    .failures
                    .push(ScenarioFailure::new(phase, error.to_string()));
                return;
            }
        }
    }
    let actual = context.state.zone_objects(expected_zone);
    if actual != Some(expected.as_slice()) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "zone {expected_zone:?} expected objects {:?}, found {:?}",
                expected
                    .iter()
                    .map(|object| object.index())
                    .collect::<Vec<_>>(),
                actual.map(|objects| objects
                    .iter()
                    .map(|object| object.index())
                    .collect::<Vec<_>>())
            ),
        ));
    }
}

fn check_object_flags_expectation(
    phase: &str,
    object: usize,
    token: Option<bool>,
    copy: Option<bool>,
    copy_source: Option<usize>,
    context: &mut RunContext,
) {
    let checked_object = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let Some(record) = context.state.object(checked_object) else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} is missing from state"),
        ));
        return;
    };
    if token.is_some_and(|expected| expected != record.is_token()) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "object {object} expected token {:?}, found {}",
                token,
                record.is_token()
            ),
        ));
    }
    if copy.is_some_and(|expected| expected != record.is_copy()) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "object {object} expected copy {:?}, found {}",
                copy,
                record.is_copy()
            ),
        ));
    }
    if let Some(source_index) = copy_source {
        match object_id(&context.objects, source_index, phase) {
            Ok(expected) if record.copy_source() == Some(expected) => {}
            Ok(expected) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {object} expected copy source {}, found {:?}",
                    expected.index(),
                    record.copy_source().map(ObjectId::index)
                ),
            )),
            Err(error) => context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string())),
        }
    }
}

fn check_attached_to_expectation(
    phase: &str,
    attachment: usize,
    target_object: Option<usize>,
    context: &mut RunContext,
) {
    let attachment_id = match object_id(&context.objects, attachment, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let expected = match target_object {
        Some(index) => match object_id(&context.objects, index, phase) {
            Ok(object) => Some(object),
            Err(error) => {
                context
                    .failures
                    .push(ScenarioFailure::new(phase, error.to_string()));
                return;
            }
        },
        None => None,
    };
    let Some(record) = context.state.object(attachment_id) else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("attachment object {attachment} is missing from state"),
        ));
        return;
    };
    if record.attached_to() != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "attachment {attachment} expected target {:?}, found {:?}",
                expected.map(ObjectId::index),
                record.attached_to().map(ObjectId::index)
            ),
        ));
    }
}

fn check_pending_triggers_expectation(phase: &str, expected: usize, context: &mut RunContext) {
    let actual = context.state.pending_triggers().len();
    if actual != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected {expected} pending triggers, found {actual}"),
        ));
    }
}

fn check_stack_entry_flags_expectation(
    phase: &str,
    entry: usize,
    kicked: Option<bool>,
    flashback: Option<bool>,
    context: &mut RunContext,
) {
    let Some(entry_id) = context.stack_entries.get(entry).copied() else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("unknown stack entry index {entry}"),
        ));
        return;
    };
    let Some(stack_entry) = context
        .state
        .stack_entries()
        .iter()
        .find(|candidate| candidate.id() == entry_id)
    else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "stack entry {} is not currently on the stack",
                entry_id.index()
            ),
        ));
        return;
    };
    if kicked.is_some_and(|expected| expected != stack_entry.kicked()) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "stack entry {entry} expected kicked {:?}, found {}",
                kicked,
                stack_entry.kicked()
            ),
        ));
    }
    if flashback.is_some_and(|expected| expected != stack_entry.flashback()) {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "stack entry {entry} expected flashback {:?}, found {}",
                flashback,
                stack_entry.flashback()
            ),
        ));
    }
}

fn check_turn_order_expectation(phase: &str, order: &[usize], context: &mut RunContext) {
    let mut expected = Vec::with_capacity(order.len());
    for player in order {
        match player_id(&context.players, *player, phase) {
            Ok(player) => expected.push(player),
            Err(error) => {
                context
                    .failures
                    .push(ScenarioFailure::new(phase, error.to_string()));
                return;
            }
        }
    }
    if context.state.turn_order() != expected.as_slice() {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "expected turn order {:?}, found {:?}",
                expected
                    .iter()
                    .map(|player| player.index())
                    .collect::<Vec<_>>(),
                context
                    .state
                    .turn_order()
                    .iter()
                    .map(|player| player.index())
                    .collect::<Vec<_>>()
            ),
        ));
    }
}

fn check_range_of_influence_expectation(phase: &str, mode: &str, context: &mut RunContext) {
    let expected = match mode {
        "off" | "Off" => RangeOfInfluence::Off,
        other => {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!("unsupported range-of-influence mode `{other}`"),
            ));
            return;
        }
    };
    if context.state.range_of_influence() != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "expected range of influence {:?}, found {:?}",
                expected,
                context.state.range_of_influence()
            ),
        ));
    }
}

fn check_commander_expectation(
    phase: &str,
    object: usize,
    commander: bool,
    colors: Option<ColorSpec>,
    cast_count: Option<u32>,
    tax_generic: Option<u32>,
    context: &mut RunContext,
) {
    let checked_object = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let Some(record) = context.state.object(checked_object) else {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {object} is missing from state"),
        ));
        return;
    };
    if record.is_commander() != commander {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!(
                "object {object} expected commander {}, found {}",
                commander,
                record.is_commander()
            ),
        ));
    }
    if let Some(colors) = colors {
        if record.color_identity() != colors.to_colors() {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {object} expected identity {:?}, found {:?}",
                    colors.to_colors(),
                    record.color_identity()
                ),
            ));
        }
    }
    if let Some(expected) = cast_count {
        if record.commander_cast_count() != expected {
            context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {object} expected cast count {}, found {}",
                    expected,
                    record.commander_cast_count()
                ),
            ));
        }
    }
    if let Some(expected) = tax_generic {
        match context.state.commander_tax(checked_object) {
            Ok(tax) if tax.generic_total().unwrap_or(u32::MAX) == expected => {}
            Ok(tax) => context.failures.push(ScenarioFailure::new(
                phase,
                format!(
                    "object {object} expected commander tax {}, found {:?}",
                    expected, tax
                ),
            )),
            Err(error) => context.failures.push(ScenarioFailure::new(
                phase,
                format!("object {object} commander tax failed: {error:?}"),
            )),
        }
    }
}

fn check_commander_identity_legal_expectation(
    phase: &str,
    player: usize,
    object: usize,
    expected: bool,
    context: &mut RunContext,
) {
    let player = match player_id(&context.players, player, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let object = match object_id(&context.objects, object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    match context.state.commander_color_identity_legal(player, object) {
        Ok(actual) if actual == expected => {}
        Ok(actual) => context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected commander identity legality {expected}, found {actual}"),
        )),
        Err(error) => context.failures.push(ScenarioFailure::new(
            phase,
            format!("commander identity legality failed: {error:?}"),
        )),
    }
}

fn check_can_target(
    phase: &str,
    player: usize,
    source_object: Option<usize>,
    requirement: &TargetRequirementSpec,
    target: TargetChoiceSpec,
    expected: bool,
    context: &mut RunContext,
) {
    let player_id = match player_id(&context.players, player, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let source = match source_object {
        Some(index) => match object_id(&context.objects, index, phase) {
            Ok(object) => Some(object),
            Err(error) => {
                context
                    .failures
                    .push(ScenarioFailure::new(phase, error.to_string()));
                return;
            }
        },
        None => None,
    };
    let requirement = match requirement.to_requirement(&context.players) {
        Ok(requirement) => requirement,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let target = match target.to_choice(&context.players, &context.objects, phase) {
        Ok(target) => target,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = context
        .state
        .can_target(player_id, source, requirement, target);
    if actual != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected target legality {expected}, found {actual}"),
        ));
    }
}

fn check_ward_cost(
    phase: &str,
    target_object: usize,
    cost: &ManaCostSpec,
    context: &mut RunContext,
) {
    let object = match object_id(&context.objects, target_object, phase) {
        Ok(object) => object,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let expected = cost.to_cost();
    match context
        .state
        .ward_cost_for_target(TargetChoice::Object(object))
    {
        Ok(actual) if actual == expected => {}
        Ok(actual) => context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {target_object} expected ward cost {expected:?}, found {actual:?}"),
        )),
        Err(error) => context.failures.push(ScenarioFailure::new(
            phase,
            format!("object {target_object} ward cost failed: {error:?}"),
        )),
    }
}

fn check_can_attack(
    phase: &str,
    player: usize,
    attack: ScenarioAttackDeclaration,
    expected: bool,
    context: &mut RunContext,
) {
    let player_id = match player_id(&context.players, player, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let attack = match attack_declaration(attack, phase, context) {
        Ok(attack) => attack,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = context.state.can_attack(player_id, attack);
    if actual != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected attack legality {expected}, found {actual}"),
        ));
    }
}

fn check_can_block(
    phase: &str,
    player: usize,
    block: ScenarioBlockDeclaration,
    expected: bool,
    context: &mut RunContext,
) {
    let player_id = match player_id(&context.players, player, phase) {
        Ok(player) => player,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let block = match block_declaration(block, phase, context) {
        Ok(block) => block,
        Err(error) => {
            context
                .failures
                .push(ScenarioFailure::new(phase, error.to_string()));
            return;
        }
    };
    let actual = context.state.can_block(player_id, block);
    if actual != expected {
        context.failures.push(ScenarioFailure::new(
            phase,
            format!("expected block legality {expected}, found {actual}"),
        ));
    }
}

fn attack_declaration(
    attack: ScenarioAttackDeclaration,
    phase: &str,
    context: &RunContext,
) -> Result<AttackDeclaration, ScenarioError> {
    Ok(AttackDeclaration::new(
        object_id(&context.objects, attack.attacker, phase)?,
        player_id(&context.players, attack.defender, phase)?,
    ))
}

fn block_declaration(
    block: ScenarioBlockDeclaration,
    phase: &str,
    context: &RunContext,
) -> Result<BlockDeclaration, ScenarioError> {
    Ok(BlockDeclaration::new(
        object_id(&context.objects, block.blocker, phase)?,
        object_id(&context.objects, block.attacker, phase)?,
    ))
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

fn stack_entry_id(
    entries: &[StackEntryId],
    index: usize,
    phase: &str,
) -> Result<StackEntryId, ScenarioError> {
    entries
        .get(index)
        .copied()
        .ok_or_else(|| ScenarioError::schema(format!("{phase}: unknown stack-entry index {index}")))
}

fn replacement_id(
    replacements: &[ReplacementEffectId],
    index: usize,
    phase: &str,
) -> Result<ReplacementEffectId, ScenarioError> {
    replacements
        .get(index)
        .copied()
        .ok_or_else(|| ScenarioError::schema(format!("{phase}: unknown replacement index {index}")))
}

fn continuous_effect_id(
    effects: &[ContinuousEffectId],
    index: usize,
    phase: &str,
) -> Result<ContinuousEffectId, ScenarioError> {
    effects.get(index).copied().ok_or_else(|| {
        ScenarioError::schema(format!("{phase}: unknown continuous effect index {index}"))
    })
}

fn activated_ability_id(
    abilities: &[ActivatedAbilityId],
    index: usize,
    phase: &str,
) -> Result<ActivatedAbilityId, ScenarioError> {
    abilities.get(index).copied().ok_or_else(|| {
        ScenarioError::schema(format!("{phase}: unknown activated ability index {index}"))
    })
}

fn replacement_target_filter(
    players: &[PlayerId],
    objects: &[ObjectId],
    target_player: Option<usize>,
    target_object: Option<usize>,
) -> Result<ReplacementDamageTargetFilter, ScenarioError> {
    match (target_player, target_object) {
        (Some(_), Some(_)) => Err(ScenarioError::schema(
            "register_replacement cannot target both player and object".to_owned(),
        )),
        (Some(index), None) => Ok(ReplacementDamageTargetFilter::Player(player_id(
            players,
            index,
            "register_replacement.target_player",
        )?)),
        (None, Some(index)) => Ok(ReplacementDamageTargetFilter::Object(object_id(
            objects,
            index,
            "register_replacement.target_object",
        )?)),
        (None, None) => Ok(ReplacementDamageTargetFilter::Any),
    }
}

fn replacement_operation(
    operation: &str,
    amount: Option<u32>,
) -> Result<ReplacementOperation, ScenarioError> {
    match operation {
        "prevent_all" => Ok(ReplacementOperation::PreventAllDamage),
        "prevent" => Ok(ReplacementOperation::PreventDamage(amount.ok_or_else(
            || ScenarioError::schema("prevent replacement requires `amount`".to_owned()),
        )?)),
        "add" => Ok(ReplacementOperation::AddDamage(amount.ok_or_else(
            || ScenarioError::schema("add replacement requires `amount`".to_owned()),
        )?)),
        "double" => Ok(ReplacementOperation::DoubleDamage),
        "set" => Ok(ReplacementOperation::SetDamage(amount.ok_or_else(
            || ScenarioError::schema("set replacement requires `amount`".to_owned()),
        )?)),
        other => Err(ScenarioError::schema(format!(
            "unsupported replacement operation `{other}`"
        ))),
    }
}

fn continuous_effect_definition(
    spec: &ContinuousEffectSpec,
    context: &RunContext,
) -> Result<ContinuousEffectDefinition, ScenarioError> {
    let controller = player_id(
        &context.players,
        spec.controller,
        "register_continuous_effect.controller",
    )?;
    let target = match (spec.all_objects, spec.target_object) {
        (true, None) => ContinuousEffectTarget::AllObjects,
        (false, Some(index)) => ContinuousEffectTarget::Object(object_id(
            &context.objects,
            index,
            "register_continuous_effect.target_object",
        )?),
        (true, Some(_)) => {
            return Err(ScenarioError::schema(
                "register_continuous_effect cannot set both all_objects and target_object"
                    .to_owned(),
            ));
        }
        (false, None) => {
            return Err(ScenarioError::schema(
                "register_continuous_effect requires target_object or all_objects".to_owned(),
            ));
        }
    };
    let operation = continuous_effect_operation(spec, context)?;
    let mut definition = ContinuousEffectDefinition::new(controller, target, operation);
    if let Some(index) = spec.source_object {
        definition = definition.with_source(object_id(
            &context.objects,
            index,
            "register_continuous_effect.source_object",
        )?);
    }
    if let Some(timestamp) = spec.timestamp {
        definition = definition.with_timestamp(timestamp);
    }
    if !spec.dependencies.is_empty() {
        let mut dependencies = Vec::with_capacity(spec.dependencies.len());
        for index in &spec.dependencies {
            dependencies.push(continuous_effect_id(
                &context.continuous_effects,
                *index,
                "register_continuous_effect.dependencies",
            )?);
        }
        definition = definition.with_dependencies(dependencies);
    }
    Ok(definition)
}

fn continuous_effect_operation(
    spec: &ContinuousEffectSpec,
    context: &RunContext,
) -> Result<ContinuousEffectOperation, ScenarioError> {
    match spec.operation.as_str() {
        "copy_base_creature" => Ok(ContinuousEffectOperation::CopyBaseCreature {
            from: object_id(
                &context.objects,
                spec.from_object.ok_or_else(|| {
                    ScenarioError::schema("copy_base_creature requires from_object".to_owned())
                })?,
                "register_continuous_effect.from_object",
            )?,
        }),
        "change_controller" => Ok(ContinuousEffectOperation::ChangeController {
            controller: player_id(
                &context.players,
                spec.player.ok_or_else(|| {
                    ScenarioError::schema("change_controller requires player".to_owned())
                })?,
                "register_continuous_effect.player",
            )?,
        }),
        "set_text_marker" => Ok(ContinuousEffectOperation::SetTextMarker {
            marker: spec.marker.ok_or_else(|| {
                ScenarioError::schema("set_text_marker requires marker".to_owned())
            })?,
        }),
        "set_types" => Ok(ContinuousEffectOperation::SetTypes {
            types: spec
                .types
                .ok_or_else(|| ScenarioError::schema("set_types requires types".to_owned()))?
                .to_types(),
        }),
        "add_types" => Ok(ContinuousEffectOperation::AddTypes {
            types: spec
                .types
                .ok_or_else(|| ScenarioError::schema("add_types requires types".to_owned()))?
                .to_types(),
        }),
        "remove_types" => Ok(ContinuousEffectOperation::RemoveTypes {
            types: spec
                .types
                .ok_or_else(|| ScenarioError::schema("remove_types requires types".to_owned()))?
                .to_types(),
        }),
        "set_colors" => Ok(ContinuousEffectOperation::SetColors {
            colors: spec
                .colors
                .ok_or_else(|| ScenarioError::schema("set_colors requires colors".to_owned()))?
                .to_colors(),
        }),
        "add_keywords" => Ok(ContinuousEffectOperation::AddKeywords {
            keywords: spec
                .keywords
                .ok_or_else(|| ScenarioError::schema("add_keywords requires keywords".to_owned()))?
                .to_keywords(),
        }),
        "remove_keywords" => Ok(ContinuousEffectOperation::RemoveKeywords {
            keywords: spec
                .keywords
                .ok_or_else(|| {
                    ScenarioError::schema("remove_keywords requires keywords".to_owned())
                })?
                .to_keywords(),
        }),
        "set_base_pt" => Ok(ContinuousEffectOperation::SetBasePowerToughness {
            power: spec
                .power
                .ok_or_else(|| ScenarioError::schema("set_base_pt requires power".to_owned()))?,
            toughness: spec.toughness.ok_or_else(|| {
                ScenarioError::schema("set_base_pt requires toughness".to_owned())
            })?,
        }),
        "set_pt" => Ok(ContinuousEffectOperation::SetPowerToughness {
            power: spec
                .power
                .ok_or_else(|| ScenarioError::schema("set_pt requires power".to_owned()))?,
            toughness: spec
                .toughness
                .ok_or_else(|| ScenarioError::schema("set_pt requires toughness".to_owned()))?,
        }),
        "modify_pt" => Ok(ContinuousEffectOperation::ModifyPowerToughness {
            power: spec
                .power
                .ok_or_else(|| ScenarioError::schema("modify_pt requires power".to_owned()))?,
            toughness: spec
                .toughness
                .ok_or_else(|| ScenarioError::schema("modify_pt requires toughness".to_owned()))?,
        }),
        "switch_pt" => Ok(ContinuousEffectOperation::SwitchPowerToughness),
        other => Err(ScenarioError::schema(format!(
            "unsupported continuous effect operation `{other}`"
        ))),
    }
}

fn activated_ability_definition(
    spec: &ActivatedAbilitySpec,
    context: &RunContext,
) -> Result<ActivatedAbilityDefinition, ScenarioError> {
    let controller = player_id(
        &context.players,
        spec.controller,
        "register_activated_ability.controller",
    )?;
    let source = match spec.source_object {
        Some(index) => Some(object_id(
            &context.objects,
            index,
            "register_activated_ability.source_object",
        )?),
        None => None,
    };
    let effect = activated_ability_effect(&spec.effect, &context.players)?;
    let mut definition = ActivatedAbilityDefinition::new(
        controller,
        source,
        spec.timing,
        spec.cost.to_cost(),
        effect,
    );
    if spec.mana_ability {
        definition = definition.as_mana_ability();
    }
    Ok(definition)
}

fn trigger_definition(
    spec: &TriggerSpec,
    context: &RunContext,
) -> Result<TriggerDefinition, ScenarioError> {
    let controller = player_id(
        &context.players,
        spec.controller,
        "register_triggered_ability.controller",
    )?;
    let source = match spec.source_object {
        Some(index) => Some(object_id(
            &context.objects,
            index,
            "register_triggered_ability.source_object",
        )?),
        None => None,
    };
    let object_filter = match spec.object {
        Some(index) => TriggerObjectFilter::Object(object_id(
            &context.objects,
            index,
            "register_triggered_ability.object",
        )?),
        None if source.is_some() => TriggerObjectFilter::Source,
        None => TriggerObjectFilter::Any,
    };
    let condition = match spec.condition.as_str() {
        "enters_battlefield" | "etb" => TriggerCondition::ObjectMoved {
            object: object_filter,
            from: TriggerZoneFilter::Any,
            to: TriggerZoneFilter::Exact(ZoneId::new(None, ZoneKind::Battlefield)),
        },
        "dies" => TriggerCondition::ObjectMoved {
            object: object_filter,
            from: TriggerZoneFilter::Exact(ZoneId::new(None, ZoneKind::Battlefield)),
            to: TriggerZoneFilter::Kind(ZoneKind::Graveyard),
        },
        "object_moved" => TriggerCondition::ObjectMoved {
            object: object_filter,
            from: TriggerZoneFilter::Any,
            to: TriggerZoneFilter::Any,
        },
        "stack_entry_added" => {
            TriggerCondition::EventKind(forge_core::GameEventKind::StackEntryAdded)
        }
        other => {
            return Err(ScenarioError::schema(format!(
                "unsupported trigger condition `{other}`"
            )));
        }
    };
    let mut definition = TriggerDefinition::new(controller, condition);
    if let Some(source) = source {
        definition = definition.with_source(source);
    }
    if spec.once {
        definition = definition.delayed_once();
    }
    Ok(definition)
}

fn activated_ability_effect(
    spec: &AbilityEffectSpec,
    players: &[PlayerId],
) -> Result<ActivatedAbilityEffect, ScenarioError> {
    let player = ability_player(spec.player, players, "activated ability effect.player")?;
    match spec.operation.as_str() {
        "add_mana" => Ok(ActivatedAbilityEffect::AddMana {
            player,
            mana: spec
                .mana
                .ok_or_else(|| ScenarioError::schema("add_mana effect requires mana".to_owned()))?
                .to_pool(),
        }),
        "gain_life" => Ok(ActivatedAbilityEffect::GainLife {
            player,
            amount: spec.amount.ok_or_else(|| {
                ScenarioError::schema("gain_life effect requires amount".to_owned())
            })?,
        }),
        "lose_life" => Ok(ActivatedAbilityEffect::LoseLife {
            player,
            amount: spec.amount.ok_or_else(|| {
                ScenarioError::schema("lose_life effect requires amount".to_owned())
            })?,
        }),
        other => Err(ScenarioError::schema(format!(
            "unsupported activated ability effect `{other}`"
        ))),
    }
}

fn cost_modifier_definition(
    spec: &CostModifierSpec,
    context: &RunContext,
) -> Result<CostModifierDefinition, ScenarioError> {
    let controller = player_id(
        &context.players,
        spec.controller,
        "register_cost_modifier.controller",
    )?;
    let source = match spec.source_object {
        Some(index) => Some(object_id(
            &context.objects,
            index,
            "register_cost_modifier.source_object",
        )?),
        None => None,
    };
    Ok(CostModifierDefinition::new(
        controller,
        source,
        cost_modifier_scope(&spec.scope, context)?,
        cost_modifier_operation(&spec.operation),
    ))
}

fn cost_modifier_scope(
    spec: &CostModifierScopeSpec,
    context: &RunContext,
) -> Result<CostModifierScope, ScenarioError> {
    match spec {
        CostModifierScopeSpec::All => Ok(CostModifierScope::AllActivatedAbilities),
        CostModifierScopeSpec::Ability(index) => {
            Ok(CostModifierScope::Ability(activated_ability_id(
                &context.activated_abilities,
                *index,
                "register_cost_modifier.ability",
            )?))
        }
        CostModifierScopeSpec::Source(index) => Ok(CostModifierScope::Source(object_id(
            &context.objects,
            *index,
            "register_cost_modifier.scope_source_object",
        )?)),
        CostModifierScopeSpec::Controller(index) => Ok(CostModifierScope::Controller(player_id(
            &context.players,
            *index,
            "register_cost_modifier.player",
        )?)),
    }
}

fn cost_modifier_operation(spec: &CostModifierOperationSpec) -> CostModifierOperation {
    match spec {
        CostModifierOperationSpec::AddManaCost(cost) => {
            CostModifierOperation::AddManaCost(cost.to_cost())
        }
        CostModifierOperationSpec::AddGeneric(amount) => CostModifierOperation::AddGeneric(*amount),
        CostModifierOperationSpec::ReduceGeneric(amount) => {
            CostModifierOperation::ReduceGeneric(*amount)
        }
    }
}

fn restriction_definition(
    spec: &RestrictionSpec,
    context: &RunContext,
) -> Result<RestrictionDefinition, ScenarioError> {
    let controller = player_id(
        &context.players,
        spec.controller,
        "register_restriction.controller",
    )?;
    let source = match spec.source_object {
        Some(index) => Some(object_id(
            &context.objects,
            index,
            "register_restriction.source_object",
        )?),
        None => None,
    };
    let mut definition = RestrictionDefinition::new(controller, restriction_effect(spec, context)?);
    if let Some(source) = source {
        definition = definition.with_source(source);
    }
    Ok(definition)
}

fn restriction_effect(
    spec: &RestrictionSpec,
    context: &RunContext,
) -> Result<RestrictionEffect, ScenarioError> {
    match spec.effect.as_str() {
        "shroud" => Ok(RestrictionEffect::Targeting {
            subject: target_restriction_subject(spec, context)?,
            restriction: TargetRestriction::Shroud,
        }),
        "hexproof" => Ok(RestrictionEffect::Targeting {
            subject: target_restriction_subject(spec, context)?,
            restriction: TargetRestriction::Hexproof,
        }),
        "protection" => Ok(RestrictionEffect::Targeting {
            subject: target_restriction_subject(spec, context)?,
            restriction: TargetRestriction::ProtectionFromColors {
                colors: spec
                    .colors
                    .ok_or_else(|| {
                        ScenarioError::schema("protection restriction requires colors".to_owned())
                    })?
                    .to_colors(),
            },
        }),
        "ward" => Ok(RestrictionEffect::Targeting {
            subject: target_restriction_subject(spec, context)?,
            restriction: TargetRestriction::Ward {
                cost: spec.cost.unwrap_or_default().to_cost(),
            },
        }),
        "cannot_attack" => Ok(RestrictionEffect::Combat {
            subject: combat_restriction_subject(spec, context)?,
            restriction: CombatRestriction::CannotAttack,
        }),
        "cannot_block" => Ok(RestrictionEffect::Combat {
            subject: combat_restriction_subject(spec, context)?,
            restriction: CombatRestriction::CannotBlock,
        }),
        "cannot_be_blocked" => Ok(RestrictionEffect::Combat {
            subject: combat_restriction_subject(spec, context)?,
            restriction: CombatRestriction::CannotBeBlocked,
        }),
        other => Err(ScenarioError::schema(format!(
            "unsupported restriction effect `{other}`"
        ))),
    }
}

fn target_restriction_subject(
    spec: &RestrictionSpec,
    context: &RunContext,
) -> Result<TargetRestrictionSubject, ScenarioError> {
    match (spec.subject_object, spec.all_objects) {
        (Some(index), false) => Ok(TargetRestrictionSubject::Object(object_id(
            &context.objects,
            index,
            "register_restriction.subject_object",
        )?)),
        (None, true) => Ok(TargetRestrictionSubject::AllObjects),
        (Some(_), true) => Err(ScenarioError::schema(
            "target restriction cannot set both subject_object and all_objects".to_owned(),
        )),
        (None, false) => Err(ScenarioError::schema(
            "target restriction requires subject_object or all_objects".to_owned(),
        )),
    }
}

fn combat_restriction_subject(
    spec: &RestrictionSpec,
    context: &RunContext,
) -> Result<CombatRestrictionSubject, ScenarioError> {
    match (spec.subject_object, spec.controlled_by, spec.all_objects) {
        (Some(index), None, false) => Ok(CombatRestrictionSubject::Object(object_id(
            &context.objects,
            index,
            "register_restriction.subject_object",
        )?)),
        (None, Some(player), false) => Ok(CombatRestrictionSubject::ControlledBy(player_id(
            &context.players,
            player,
            "register_restriction.controlled_by",
        )?)),
        (None, None, true) => Ok(CombatRestrictionSubject::AllObjects),
        _ => Err(ScenarioError::schema(
            "combat restriction requires exactly one of subject_object, controlled_by, all_objects"
                .to_owned(),
        )),
    }
}

fn ability_player(
    player: Option<usize>,
    players: &[PlayerId],
    phase: &str,
) -> Result<AbilityPlayer, ScenarioError> {
    match player {
        Some(index) => Ok(AbilityPlayer::Player(player_id(players, index, phase)?)),
        None => Ok(AbilityPlayer::Controller),
    }
}

fn parse_script(value: RonValue) -> Result<Vec<ScenarioStep>, ScenarioError> {
    let mut script = Vec::new();
    for value in value.into_list("script")? {
        let map = value.into_map("script step")?;
        let action = map.required_string("action")?;
        script.push(match action.as_str() {
            "decide_turn_order" => ScenarioStep::DecideTurnOrder,
            "set_turn_order" => ScenarioStep::SetTurnOrder {
                order: parse_usize_list(map.required("order")?, "set_turn_order.order")?,
            },
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
            "add_poison_counters" => ScenarioStep::AddPoisonCounters {
                player: map.required_usize("player")?,
                amount: map.required_u32("amount")?,
            },
            "add_mana" => ScenarioStep::AddMana {
                player: map.required_usize("player")?,
                mana: ManaSpec::from_ron_value(map.required("mana")?)?,
            },
            "clear_mana" => ScenarioStep::ClearMana {
                player: map.required_usize("player")?,
            },
            "pay_mana_auto" => ScenarioStep::PayManaAuto {
                player: map.required_usize("player")?,
                cost: ManaCostSpec::from_ron_value(map.required("cost")?)?,
            },
            "scry" => ScenarioStep::Scry {
                player: map.required_usize("player")?,
                count: map.required_u32("count")?,
                bottom: parse_usize_list(
                    map.optional("bottom")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                    "scry.bottom",
                )?,
            },
            "surveil" => ScenarioStep::Surveil {
                player: map.required_usize("player")?,
                count: map.required_u32("count")?,
                graveyard: parse_usize_list(
                    map.optional("graveyard")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                    "surveil.graveyard",
                )?,
            },
            "cycle_auto" => ScenarioStep::CycleAuto {
                player: map.required_usize("player")?,
                object: map.required_usize("object")?,
                cost: match map.optional("cost")? {
                    Some(value) => ManaCostSpec::from_ron_value(value)?,
                    None => ManaCostSpec::default(),
                },
            },
            "attach_object" => ScenarioStep::AttachObject {
                attachment: map.required_usize("attachment")?,
                target_object: map.optional_usize("target_object")?,
            },
            "equip_auto" => ScenarioStep::EquipAuto {
                player: map.required_usize("player")?,
                equipment: map.required_usize("equipment")?,
                target_object: map.required_usize("target_object")?,
                cost: match map.optional("cost")? {
                    Some(value) => ManaCostSpec::from_ron_value(value)?,
                    None => ManaCostSpec::default(),
                },
            },
            "move_object" => ScenarioStep::MoveObject {
                object: map.required_usize("object")?,
                zone: parse_zone_from_map(&map)?,
            },
            "create_token" => ScenarioStep::CreateToken {
                card: map.required_u32("card")?,
                owner: map.required_usize("owner")?,
                controller: map
                    .optional_usize("controller")?
                    .unwrap_or(map.required_usize("owner")?),
                power: map.optional_i32("power")?,
                toughness: map.optional_i32("toughness")?,
                keywords: match map.optional("keywords")? {
                    Some(value) => CreatureKeywordSpec::from_ron_value(value)?,
                    None => CreatureKeywordSpec::default(),
                },
            },
            "create_permanent_copy" => ScenarioStep::CreatePermanentCopy {
                source: map.required_usize("source")?,
                owner: map.required_usize("owner")?,
                controller: map
                    .optional_usize("controller")?
                    .unwrap_or(map.required_usize("owner")?),
                token: map.optional_bool("token")?.unwrap_or(false),
            },
            "copy_stack_entry" => ScenarioStep::CopyStackEntry {
                player: map.required_usize("player")?,
                entry: map.required_usize("entry")?,
            },
            "set_base_creature" => ScenarioStep::SetBaseCreature {
                object: map.required_usize("object")?,
                power: map.required_i32("power")?,
                toughness: map.required_i32("toughness")?,
                keywords: match map.optional("keywords")? {
                    Some(value) => CreatureKeywordSpec::from_ron_value(value)?,
                    None => CreatureKeywordSpec::default(),
                },
            },
            "clear_base_creature" => ScenarioStep::ClearBaseCreature {
                object: map.required_usize("object")?,
            },
            "set_object_tapped" => ScenarioStep::SetObjectTapped {
                object: map.required_usize("object")?,
                tapped: map.optional_bool("tapped")?.unwrap_or(true),
            },
            "set_object_loyalty" => ScenarioStep::SetObjectLoyalty {
                object: map.required_usize("object")?,
                loyalty: map.optional_i32("loyalty")?,
            },
            "set_object_color_identity" => ScenarioStep::SetObjectColorIdentity {
                object: map.required_usize("object")?,
                colors: ColorSpec::from_ron_value(map.required("colors")?)?,
            },
            "designate_commander" => ScenarioStep::DesignateCommander {
                object: map.required_usize("object")?,
                colors: ColorSpec::from_ron_value(map.required("colors")?)?,
            },
            "record_commander_cast" => ScenarioStep::RecordCommanderCast {
                object: map.required_usize("object")?,
            },
            "validate_commander_color_identity" => ScenarioStep::ValidateCommanderColorIdentity {
                player: map.required_usize("player")?,
                objects: parse_usize_list(
                    map.optional("objects")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                    "validate_commander_color_identity.objects",
                )?,
            },
            "add_object_counters" => ScenarioStep::AddObjectCounters {
                object: map.required_usize("object")?,
                kind: parse_counter_kind(&map.required_string("kind")?)?,
                amount: map.required_u32("amount")?,
            },
            "remove_object_counters" => ScenarioStep::RemoveObjectCounters {
                object: map.required_usize("object")?,
                kind: parse_counter_kind(&map.required_string("kind")?)?,
                amount: map.required_u32("amount")?,
            },
            "mark_damage" => ScenarioStep::MarkDamage {
                object: map.required_usize("object")?,
                amount: map.required_u32("amount")?,
            },
            "register_damage_replacement" => ScenarioStep::RegisterDamageReplacement {
                controller: map.required_usize("controller")?,
                source_object: map.optional_usize("source_object")?,
                target_player: map.optional_usize("target_player")?,
                target_object: map.optional_usize("target_object")?,
                combat_only: map.optional_bool("combat_only")?.unwrap_or(false),
                operation: map.required_string("operation")?,
                amount: map.optional_u32("amount")?,
                once: map.optional_bool("once")?.unwrap_or(false),
                self_replacement: map.optional_bool("self_replacement")?.unwrap_or(false),
            },
            "set_replacement_order" => ScenarioStep::SetReplacementOrder {
                chooser: map.required_usize("chooser")?,
                order: parse_usize_list(
                    map.optional("order")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                    "set_replacement_order.order",
                )?,
            },
            "register_continuous_effect" => ScenarioStep::RegisterContinuousEffect {
                spec: ContinuousEffectSpec::from_map(&map)?,
            },
            "register_activated_ability" => ScenarioStep::RegisterActivatedAbility {
                spec: ActivatedAbilitySpec::from_map(&map)?,
            },
            "register_cost_modifier" => ScenarioStep::RegisterCostModifier {
                spec: CostModifierSpec::from_map(&map)?,
            },
            "register_restriction" => ScenarioStep::RegisterRestriction {
                spec: RestrictionSpec::from_map(&map)?,
            },
            "register_triggered_ability" => ScenarioStep::RegisterTriggeredAbility {
                spec: TriggerSpec::from_map(&map)?,
            },
            "put_pending_triggers_on_stack" => ScenarioStep::PutPendingTriggersOnStack,
            "activate_ability_auto" => ScenarioStep::ActivateAbilityAuto {
                player: map.required_usize("player")?,
                ability: map.required_usize("ability")?,
            },
            "cast_spell_auto" => ScenarioStep::CastSpellAuto {
                player: map.required_usize("player")?,
                object: map.required_usize("object")?,
                kind: map.required_string("kind")?,
                timing: map
                    .optional_string("timing")?
                    .unwrap_or_else(|| "instant".to_owned()),
                cost: match map.optional("cost")? {
                    Some(value) => ManaCostSpec::from_ron_value(value)?,
                    None => ManaCostSpec::default(),
                },
                flash: map.optional_bool("flash")?.unwrap_or(false),
                kicker: match map.optional("kicker")? {
                    Some(value) => Some(ManaCostSpec::from_ron_value(value)?),
                    None => None,
                },
                flashback: match map.optional("flashback")? {
                    Some(value) => Some(ManaCostSpec::from_ron_value(value)?),
                    None => None,
                },
                targets: parse_targets(
                    map.optional("targets")?
                        .unwrap_or_else(|| RonValue::List(Vec::new())),
                )?,
            },
            "assert_can_target" => ScenarioStep::AssertCanTarget {
                player: map.required_usize("player")?,
                source_object: map.optional_usize("source_object")?,
                requirement: TargetRequirementSpec::from_ron_value(map.required("requirement")?)?,
                target: TargetChoiceSpec::from_map(&map)?,
                expected: map.required_bool("expected")?,
            },
            "assert_ward_cost" => ScenarioStep::AssertWardCost {
                target_object: map.required_usize("target_object")?,
                cost: ManaCostSpec::from_ron_value(map.required("cost")?)?,
            },
            "assert_characteristics" => ScenarioStep::AssertCharacteristics {
                expectation: CharacteristicExpectation::from_ron_value(RonValue::Map(map))?,
            },
            "assert_object_tapped" => ScenarioStep::AssertObjectTapped {
                object: map.required_usize("object")?,
                tapped: map.required_bool("tapped")?,
            },
            "assert_object_loyalty" => ScenarioStep::AssertObjectLoyalty {
                object: map.required_usize("object")?,
                loyalty: map.optional_i32("loyalty")?,
            },
            "assert_object_counters" => ScenarioStep::AssertObjectCounters {
                object: map.required_usize("object")?,
                kind: parse_counter_kind(&map.required_string("kind")?)?,
                count: map.required_u32("count")?,
            },
            "assert_object_zone" => ScenarioStep::AssertObjectZone {
                object: map.required_usize("object")?,
                zone: parse_zone_from_map(&map)?,
            },
            "assert_zone_order" => ScenarioStep::AssertZoneOrder {
                zone: parse_zone_from_map(&map)?,
                objects: parse_usize_list(map.required("objects")?, "assert_zone_order.objects")?,
            },
            "assert_object_flags" => ScenarioStep::AssertObjectFlags {
                object: map.required_usize("object")?,
                token: map.optional_bool("token")?,
                copy: map.optional_bool("copy")?,
                copy_source: map.optional_usize("copy_source")?,
            },
            "assert_attached_to" => ScenarioStep::AssertAttachedTo {
                attachment: map.required_usize("attachment")?,
                target_object: map.optional_usize("target_object")?,
            },
            "assert_pending_triggers" => ScenarioStep::AssertPendingTriggers {
                count: map.required_usize("count")?,
            },
            "assert_stack_entry_flags" => ScenarioStep::AssertStackEntryFlags {
                entry: map.required_usize("entry")?,
                kicked: map.optional_bool("kicked")?,
                flashback: map.optional_bool("flashback")?,
            },
            "assert_turn_order" => ScenarioStep::AssertTurnOrder {
                order: parse_usize_list(map.required("order")?, "assert_turn_order.order")?,
            },
            "assert_range_of_influence" => ScenarioStep::AssertRangeOfInfluence {
                mode: map.required_string("mode")?,
            },
            "assert_commander" => ScenarioStep::AssertCommander {
                object: map.required_usize("object")?,
                commander: map.optional_bool("commander")?.unwrap_or(true),
                colors: match map.optional("colors")? {
                    Some(value) => Some(ColorSpec::from_ron_value(value)?),
                    None => None,
                },
                cast_count: map.optional_u32("cast_count")?,
                tax_generic: map.optional_u32("tax_generic")?,
            },
            "assert_commander_identity_legal" => ScenarioStep::AssertCommanderIdentityLegal {
                player: map.required_usize("player")?,
                object: map.required_usize("object")?,
                expected: map.required_bool("expected")?,
            },
            "declare_attackers" => ScenarioStep::DeclareAttackers {
                player: map.required_usize("player")?,
                attacks: parse_attack_declarations(map.required("attacks")?)?,
            },
            "declare_blockers" => ScenarioStep::DeclareBlockers {
                player: map.required_usize("player")?,
                blocks: parse_block_declarations(map.required("blocks")?)?,
            },
            "assert_can_attack" => ScenarioStep::AssertCanAttack {
                player: map.required_usize("player")?,
                attack: ScenarioAttackDeclaration::from_ron_value(map.required("attack")?)?,
                expected: map.required_bool("expected")?,
            },
            "assert_can_block" => ScenarioStep::AssertCanBlock {
                player: map.required_usize("player")?,
                block: ScenarioBlockDeclaration::from_ron_value(map.required("block")?)?,
                expected: map.required_bool("expected")?,
            },
            "assign_combat_damage" => ScenarioStep::AssignCombatDamage {
                assignments: parse_combat_damage_requests(map.required("assignments")?)?,
            },
            "request_cleanup_priority" => ScenarioStep::RequestCleanupPriority,
            _ => {
                return Err(ScenarioError::schema(format!(
                    "unsupported script action `{action}`"
                )));
            }
        });
    }
    Ok(script)
}

fn parse_attack_declarations(
    value: RonValue,
) -> Result<Vec<ScenarioAttackDeclaration>, ScenarioError> {
    value
        .into_list("attack declarations")?
        .into_iter()
        .map(ScenarioAttackDeclaration::from_ron_value)
        .collect()
}

fn parse_targets(value: RonValue) -> Result<Vec<TargetSpec>, ScenarioError> {
    value
        .into_list("targets")?
        .into_iter()
        .map(TargetSpec::from_ron_value)
        .collect()
}

fn parse_block_declarations(
    value: RonValue,
) -> Result<Vec<ScenarioBlockDeclaration>, ScenarioError> {
    value
        .into_list("block declarations")?
        .into_iter()
        .map(ScenarioBlockDeclaration::from_ron_value)
        .collect()
}

fn parse_combat_damage_requests(
    value: RonValue,
) -> Result<Vec<ScenarioCombatDamageRequest>, ScenarioError> {
    value
        .into_list("combat damage requests")?
        .into_iter()
        .map(ScenarioCombatDamageRequest::from_ron_value)
        .collect()
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
        "Ceased" | "ceased" => Ok(ZoneSpec::Ceased),
        _ => Err(ScenarioError::schema(format!("unsupported zone `{zone}`"))),
    }
}

fn parse_maybe_player(
    value: RonValue,
    label: &str,
) -> Result<MaybePlayerExpectation, ScenarioError> {
    match value {
        RonValue::String(input) if matches!(input.as_str(), "none" | "None") => {
            Ok(MaybePlayerExpectation::None)
        }
        RonValue::String(input) => input
            .parse::<usize>()
            .map(MaybePlayerExpectation::Player)
            .map_err(|_| {
                ScenarioError::schema(format!("{label} must be a player index or string `none`"))
            }),
        other => Ok(MaybePlayerExpectation::Player(other.into_usize(label)?)),
    }
}

fn parse_maybe_step(value: RonValue) -> Result<MaybeStepExpectation, ScenarioError> {
    let input = value.into_string("current_step")?;
    if matches!(input.as_str(), "none" | "None") {
        return Ok(MaybeStepExpectation::None);
    }
    Ok(MaybeStepExpectation::Step(parse_step(&input)?))
}

fn parse_counter_kind(input: &str) -> Result<CounterKind, ScenarioError> {
    match input {
        "+1/+1" | "plus_one_plus_one" | "p1p1" => Ok(CounterKind::PlusOnePlusOne),
        "-1/-1" | "minus_one_minus_one" | "m1m1" => Ok(CounterKind::MinusOneMinusOne),
        "loyalty" | "LOYALTY" => Ok(CounterKind::Loyalty),
        other => {
            if let Some(value) = other.strip_prefix("named:") {
                let id = value.parse::<u32>().map_err(|error| {
                    ScenarioError::schema(format!("invalid named counter id `{value}`: {error}"))
                })?;
                return Ok(CounterKind::named(id));
            }
            Err(ScenarioError::schema(format!(
                "unsupported counter kind `{other}`"
            )))
        }
    }
}

fn parse_step(input: &str) -> Result<Step, ScenarioError> {
    match input {
        "Untap" | "untap" => Ok(Step::Untap),
        "Upkeep" | "upkeep" => Ok(Step::Upkeep),
        "Draw" | "draw" => Ok(Step::Draw),
        "PrecombatMain" | "precombat_main" => Ok(Step::PrecombatMain),
        "BeginningOfCombat" | "beginning_of_combat" => Ok(Step::BeginningOfCombat),
        "DeclareAttackers" | "declare_attackers" => Ok(Step::DeclareAttackers),
        "DeclareBlockers" | "declare_blockers" => Ok(Step::DeclareBlockers),
        "CombatDamage" | "combat_damage" => Ok(Step::CombatDamage),
        "EndOfCombat" | "end_of_combat" => Ok(Step::EndOfCombat),
        "PostcombatMain" | "postcombat_main" => Ok(Step::PostcombatMain),
        "End" | "end" => Ok(Step::End),
        "Cleanup" | "cleanup" => Ok(Step::Cleanup),
        _ => Err(ScenarioError::schema(format!(
            "unsupported current_step `{input}`"
        ))),
    }
}

fn parse_activation_timing(input: &str) -> Result<ActivationTiming, ScenarioError> {
    match input {
        "Instant" | "instant" => Ok(ActivationTiming::Instant),
        "Sorcery" | "sorcery" => Ok(ActivationTiming::Sorcery),
        other => Err(ScenarioError::schema(format!(
            "unsupported activation timing `{other}`"
        ))),
    }
}

fn parse_stack_object_kind(input: &str) -> Result<StackObjectKind, ScenarioError> {
    match input {
        "InstantSpell" | "instant_spell" | "instant" => Ok(StackObjectKind::InstantSpell),
        "SorcerySpell" | "sorcery_spell" | "sorcery" => Ok(StackObjectKind::SorcerySpell),
        "PermanentSpell" | "permanent_spell" | "permanent" => Ok(StackObjectKind::PermanentSpell),
        "ActivatedAbility" | "activated_ability" => Ok(StackObjectKind::ActivatedAbility),
        "TriggeredAbility" | "triggered_ability" => Ok(StackObjectKind::TriggeredAbility),
        other => Err(ScenarioError::schema(format!(
            "unsupported stack object kind `{other}`"
        ))),
    }
}

fn parse_spell_timing(input: &str) -> Result<SpellTiming, ScenarioError> {
    match input {
        "Instant" | "instant" => Ok(SpellTiming::Instant),
        "Sorcery" | "sorcery" => Ok(SpellTiming::Sorcery),
        other => Err(ScenarioError::schema(format!(
            "unsupported spell timing `{other}`"
        ))),
    }
}

fn format_mana_pool(pool: ManaPool) -> String {
    format!(
        "W{} U{} B{} R{} G{} C{}",
        pool.get(ManaKind::White),
        pool.get(ManaKind::Blue),
        pool.get(ManaKind::Black),
        pool.get(ManaKind::Red),
        pool.get(ManaKind::Green),
        pool.get(ManaKind::Colorless)
    )
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

    fn required_bool(&self, key: &str) -> Result<bool, ScenarioError> {
        self.required(key)?.into_bool(key)
    }

    fn optional_bool(&self, key: &str) -> Result<Option<bool>, ScenarioError> {
        self.optional(key)?
            .map(|value| value.into_bool(key))
            .transpose()
    }
}

const MAX_RON_NESTING_DEPTH: usize = 128;

struct RonParser<'a> {
    input: &'a str,
    chars: Vec<char>,
    pos: usize,
    nesting_depth: usize,
}

impl<'a> RonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars().collect(),
            pos: 0,
            nesting_depth: 0,
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
        if self.nesting_depth >= MAX_RON_NESTING_DEPTH {
            return Err(self.error("RON nesting depth exceeds 128"));
        }
        self.nesting_depth += 1;
        let result = match self.peek() {
            Some('(') => self.parse_map(),
            Some('[') => self.parse_list(),
            Some('"') => self.parse_string().map(RonValue::String),
            Some('-' | '0'..='9') => self.parse_integer().map(RonValue::Integer),
            Some(character) if is_ident_start(character) => self.parse_ident_value(),
            Some(_) => Err(self.error("unexpected token")),
            None => Err(self.error("unexpected end of input")),
        };
        self.nesting_depth -= 1;
        result
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
        MAX_RON_NESTING_DEPTH,
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

    #[test]
    fn ron_scenario_asserts_poison_loss_winner() {
        let input = r#"
        (
            name: "poison loss",
            setup: (players: 2),
            script: [
                (action: "add_poison_counters", player: 1, amount: 10),
                (action: "check_state_based_actions"),
            ],
            expect: (
                players: [(player: 1, poison: 10)],
                outcome: (status: "won", player: 0),
                invariants: ["zone_conservation", "hash_consistency"],
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(report.passed(), "{:?}", report.failures());
    }

    #[test]
    fn ron_scenario_covers_mana_and_object_movement() {
        let input = r#"
        (
            name: "mana and move",
            setup: (
                players: 2,
                objects: [
                    (card: 10, owner: 0, controller: 0, zone: "Battlefield"),
                ],
            ),
            script: [
                (action: "add_mana", player: 0, mana: (white: 2, blue: 1, colorless: 2)),
                (action: "pay_mana_auto", player: 0, cost: (white: 1, generic: 1)),
                (action: "add_mana", player: 1, mana: (red: 1)),
                (action: "clear_mana", player: 1),
                (action: "set_base_creature", object: 0, power: 2, toughness: 2, keywords: ["vigilance", "flying"]),
                (action: "set_object_tapped", object: 0, tapped: true),
                (action: "clear_base_creature", object: 0),
                (action: "move_object", object: 0, zone: "Exile"),
            ],
            expect: (
                zone_counts: [
                    (zone: "Battlefield", count: 0),
                    (zone: "Exile", count: 1),
                ],
                players: [
                    (player: 0, mana: (white: 1, blue: 1, colorless: 1)),
                    (player: 1, mana: ()),
                ],
                outcome: "in_progress",
                invariants: ["zone_conservation", "hash_consistency"],
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(report.passed(), "{:?}", report.failures());
    }

    #[test]
    fn ron_scenario_covers_creature_damage_sba() {
        let input = r#"
        (
            name: "creature lethal damage",
            setup: (
                players: 2,
                objects: [
                    (card: 20, owner: 1, controller: 1, zone: "Battlefield"),
                ],
            ),
            script: [
                (action: "set_base_creature", object: 0, power: 3, toughness: 3, keywords: ["reach"]),
                (action: "mark_damage", object: 0, amount: 3),
                (action: "check_state_based_actions"),
            ],
            expect: (
                zone_counts: [
                    (zone: "Battlefield", count: 0),
                    (zone: "Graveyard", player: 1, count: 1),
                ],
                outcome: "in_progress",
                invariants: ["zone_conservation", "hash_consistency"],
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(report.passed(), "{:?}", report.failures());
    }

    #[test]
    fn ron_scenario_covers_combat_actions() {
        let input = r#"
        (
            name: "combat trample deathtouch",
            setup: (
                players: 2,
                libraries: [(player: 0, cards: [41, 42])],
                objects: [
                    (card: 30, owner: 0, controller: 0, zone: "Battlefield"),
                    (card: 31, owner: 1, controller: 1, zone: "Battlefield"),
                ],
            ),
            script: [
                (action: "set_base_creature", object: 0, power: 5, toughness: 5, keywords: ["trample", "deathtouch"]),
                (action: "set_base_creature", object: 1, power: 3, toughness: 3),
                (action: "start_turn", player: 0),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "declare_attackers", player: 0, attacks: [(attacker: 0, defender: 1)]),
                (action: "advance_step"),
                (action: "declare_blockers", player: 1, blocks: [(blocker: 1, attacker: 0)]),
                (action: "advance_step"),
                (action: "assign_combat_damage", assignments: [
                    (source: 0, assignments: [(object: 1, amount: 1), (player: 1, amount: 4)]),
                    (source: 1, assignments: [(object: 0, amount: 3)]),
                ]),
            ],
            expect: (
                zone_counts: [
                    (zone: "Battlefield", count: 1),
                    (zone: "Graveyard", player: 1, count: 1),
                ],
                players: [
                    (player: 0, life: 20),
                    (player: 1, life: 16),
                ],
                outcome: "in_progress",
                invariants: ["zone_conservation", "life_poison_sanity", "hash_consistency"],
                hash_determinism: true,
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(report.passed(), "{:?}", report.failures());
    }

    #[test]
    fn ron_scenario_asserts_turn_priority_and_cleanup_exception() {
        let input = r#"
        (
            name: "cleanup priority expectation",
            setup: (
                players: 2,
                libraries: [(player: 0, cards: [31, 32])],
            ),
            script: [
                (action: "start_turn", player: 0),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "advance_step"),
                (action: "request_cleanup_priority"),
                (action: "advance_step"),
            ],
            expect: (
                active_player: 0,
                priority_player: 0,
                current_step: "Cleanup",
                outcome: "in_progress",
                invariants: ["zone_conservation", "hash_consistency"],
            ),
        )
        "#;

        let report =
            run_scenario_ron(input).unwrap_or_else(|error| panic!("unexpected run error: {error}"));

        assert!(report.passed(), "{:?}", report.failures());
    }

    #[test]
    fn ron_parser_rejects_new_surface_mistakes() {
        let bad_keyword = r#"
        (
            setup: (players: 1, objects: [(card: 1, owner: 0, zone: "Battlefield")]),
            script: [
                (action: "set_base_creature", object: 0, power: 1, toughness: 1, keywords: ["surprise"]),
            ],
            expect: (outcome: "in_progress"),
        )
        "#;
        let bad_step = r#"
        (
            setup: (players: 1),
            script: [],
            expect: (current_step: "TeaBreak"),
        )
        "#;
        let bad_priority_player = r#"
        (
            setup: (players: 1),
            script: [],
            expect: (priority_player: "left"),
        )
        "#;

        assert!(parse_scenario_ron(bad_keyword).is_err());
        assert!(parse_scenario_ron(bad_step).is_err());
        assert!(parse_scenario_ron(bad_priority_player).is_err());
    }

    #[test]
    fn ron_parser_rejects_adversarial_nesting_without_stack_overflow() {
        for depth in [MAX_RON_NESTING_DEPTH, 4_096] {
            let input = format!("{}u{}", "[".repeat(depth), "]".repeat(depth));

            match parse_scenario_ron(&input) {
                Err(error) => {
                    assert!(error.to_string().contains("RON nesting depth exceeds 128"));
                }
                Ok(_) => panic!("deep nesting must fail closed"),
            }
        }
    }
}
