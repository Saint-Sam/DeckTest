#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Stable catalog and validated card-mechanics data for Forge 2.0.

use serde::{Deserialize, Serialize};

/// Magic bytes at the start of every compiled Forge card database.
pub const CARD_DATABASE_MAGIC: [u8; 8] = *b"FORGECDB";

/// Current compiled card database schema.
pub const CARD_DATABASE_SCHEMA_VERSION: u32 = 1;

/// Source provenance embedded in catalog and database artifacts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceProvenance {
    /// Human-readable source name.
    pub source: String,
    /// Repo-relative source path or stable source locator.
    pub source_path: String,
    /// Timestamp supplied by the source.
    pub source_updated_at: String,
    /// SHA-256 of the exact local source snapshot.
    pub source_sha256: String,
    /// Generator and schema version string.
    pub generator: String,
}

/// Compiled catalog plus mechanics definitions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CardDatabase {
    /// Database schema, duplicated in the binary header for defensive loading.
    pub schema_version: u32,
    /// Exact source provenance.
    pub provenance: SourceProvenance,
    /// One row per source Oracle identity.
    pub identities: Vec<IdentityRecord>,
    /// One row per English printing.
    pub printings: Vec<PrintingRecord>,
    /// Validated mechanics keyed by Oracle identity.
    pub definitions: Vec<CardDefinition>,
}

/// Compact source catalog before mechanics definitions are compiled.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CardCatalog {
    /// Catalog schema version.
    pub schema_version: u32,
    /// Exact source provenance.
    pub provenance: SourceProvenance,
    /// One row per source Oracle identity.
    pub identities: Vec<IdentityRecord>,
    /// One row per English printing.
    pub printings: Vec<PrintingRecord>,
}

impl CardCatalog {
    /// Creates an empty catalog with the current schema.
    #[must_use]
    pub fn empty(provenance: SourceProvenance) -> Self {
        Self {
            schema_version: CARD_DATABASE_SCHEMA_VERSION,
            provenance,
            identities: Vec::new(),
            printings: Vec::new(),
        }
    }
}

impl CardDatabase {
    /// Creates an empty database with the current schema.
    #[must_use]
    pub fn empty(provenance: SourceProvenance) -> Self {
        Self {
            schema_version: CARD_DATABASE_SCHEMA_VERSION,
            provenance,
            identities: Vec::new(),
            printings: Vec::new(),
            definitions: Vec::new(),
        }
    }
}

macro_rules! string_id {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            /// Creates an id when it is non-empty and contains only portable id characters.
            #[must_use]
            pub fn parse(value: impl Into<String>) -> Option<Self> {
                let value = value.into();
                let valid = !value.is_empty()
                    && value.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '-' | '_' | ':' | '.')
                    });
                valid.then_some(Self(value))
            }

            /// Returns the id text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(
    OracleId,
    "Stable mechanics identity for one Oracle card object."
);
string_id!(
    PrintingId,
    "Stable identity for one physical or digital printing."
);

/// Source layout for a card or catalog record.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum CardLayout {
    /// Ordinary one-face card.
    Normal,
    /// Split or fuse card.
    Split,
    /// Kamigawa-style flip card.
    Flip,
    /// Transforming double-faced card.
    Transform,
    /// Modal double-faced card.
    ModalDfc,
    /// Meld component or meld result.
    Meld,
    /// Adventure card.
    Adventure,
    /// Level-up card.
    Leveler,
    /// Class enchantment.
    Class,
    /// Case enchantment.
    Case,
    /// Saga.
    Saga,
    /// Mutate card.
    Mutate,
    /// Prototype card.
    Prototype,
    /// Reversible card.
    ReversibleCard,
    /// Host card.
    Host,
    /// Augment card.
    Augment,
    /// Prepare/assemble card.
    Prepare,
    /// Plane or phenomenon.
    Planar,
    /// Archenemy scheme.
    Scheme,
    /// Vanguard card.
    Vanguard,
    /// Token card.
    Token,
    /// Double-faced token card.
    DoubleFacedToken,
    /// Emblem record.
    Emblem,
    /// Art-series/non-game record.
    ArtSeries,
}

impl CardLayout {
    /// Parses the canonical source layout name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "split" => Some(Self::Split),
            "flip" => Some(Self::Flip),
            "transform" => Some(Self::Transform),
            "modal_dfc" => Some(Self::ModalDfc),
            "meld" => Some(Self::Meld),
            "adventure" => Some(Self::Adventure),
            "leveler" => Some(Self::Leveler),
            "class" => Some(Self::Class),
            "case" => Some(Self::Case),
            "saga" => Some(Self::Saga),
            "mutate" => Some(Self::Mutate),
            "prototype" => Some(Self::Prototype),
            "reversible_card" => Some(Self::ReversibleCard),
            "host" => Some(Self::Host),
            "augment" => Some(Self::Augment),
            "prepare" => Some(Self::Prepare),
            "planar" => Some(Self::Planar),
            "scheme" => Some(Self::Scheme),
            "vanguard" => Some(Self::Vanguard),
            "token" => Some(Self::Token),
            "double_faced_token" => Some(Self::DoubleFacedToken),
            "emblem" => Some(Self::Emblem),
            "art_series" => Some(Self::ArtSeries),
            _ => None,
        }
    }

    /// Returns the canonical source layout name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Split => "split",
            Self::Flip => "flip",
            Self::Transform => "transform",
            Self::ModalDfc => "modal_dfc",
            Self::Meld => "meld",
            Self::Adventure => "adventure",
            Self::Leveler => "leveler",
            Self::Class => "class",
            Self::Case => "case",
            Self::Saga => "saga",
            Self::Mutate => "mutate",
            Self::Prototype => "prototype",
            Self::ReversibleCard => "reversible_card",
            Self::Host => "host",
            Self::Augment => "augment",
            Self::Prepare => "prepare",
            Self::Planar => "planar",
            Self::Scheme => "scheme",
            Self::Vanguard => "vanguard",
            Self::Token => "token",
            Self::DoubleFacedToken => "double_faced_token",
            Self::Emblem => "emblem",
            Self::ArtSeries => "art_series",
        }
    }

    /// Returns whether the layout is catalog-only by default.
    #[must_use]
    pub const fn catalog_only_by_default(self) -> bool {
        matches!(
            self,
            Self::Token | Self::DoubleFacedToken | Self::Emblem | Self::ArtSeries
        )
    }
}

/// Explicit catalog/playability classification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CardClassification {
    /// Mechanics have semantic verification.
    VerifiedPlayable,
    /// Mechanics are expected to be playable but are not semantically verified.
    UnverifiedPlayable,
    /// Translation is blocked by a named, actionable reason.
    Quarantined(String),
    /// Rules are intentionally outside the current v1 mechanics scope.
    OutOfV1(String),
    /// Record is visible in the catalog but is not a playable game card.
    CatalogOnly(String),
}

/// Catalog row for one Oracle identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentityRecord {
    /// Stable Oracle identity.
    pub id: OracleId,
    /// Canonical full name.
    pub name: String,
    /// Source layout.
    pub layout: CardLayout,
    /// Ordered source face names.
    pub face_names: Vec<String>,
    /// Explicit classification.
    pub classification: CardClassification,
}

/// Catalog row for one English printing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrintingRecord {
    /// Stable printing identity.
    pub id: PrintingId,
    /// Referenced mechanics identity.
    pub oracle_id: OracleId,
    /// Printed full name.
    pub name: String,
    /// Source layout.
    pub layout: CardLayout,
    /// Lowercase set code.
    pub set_code: String,
    /// Collector number as printed by the source.
    pub collector_number: String,
    /// ISO release date when supplied.
    pub released_at: String,
    /// Ordered printed face names.
    pub face_names: Vec<String>,
}

/// Validated mechanics for one Oracle identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CardDefinition {
    /// Stable Oracle identity.
    pub id: OracleId,
    /// Canonical full card name.
    pub name: String,
    /// Source layout.
    pub layout: CardLayout,
    /// Playability status of this mechanics definition.
    pub status: CardClassification,
    /// Ordered faces.
    pub faces: Vec<CardFace>,
}

/// Mechanics and printed characteristics for one face.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CardFace {
    /// Face name.
    pub name: String,
    /// Printed mana cost.
    pub mana_cost: ManaCost,
    /// Printed type line.
    pub type_line: TypeLine,
    /// Oracle text retained for text-first rendering and review.
    pub oracle_text: String,
    /// Printed power expression.
    pub power: Option<String>,
    /// Printed toughness expression.
    pub toughness: Option<String>,
    /// Printed loyalty expression.
    pub loyalty: Option<String>,
    /// Printed defense expression.
    pub defense: Option<String>,
    /// Validated keyword ids.
    pub keywords: Vec<KeywordId>,
    /// Typed abilities in printed order.
    pub abilities: Vec<AbilityDefinition>,
}

/// One printed or derived card ability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbilityDefinition {
    /// Ability kind.
    pub kind: AbilityKind,
    /// Additional/activation costs.
    pub costs: Vec<Expression>,
    /// Trigger or replacement event.
    pub event: Option<Expression>,
    /// Intervening or ordinary condition.
    pub condition: Option<Expression>,
    /// Timing permission/restriction.
    pub timing: Option<Expression>,
    /// Resulting effect expression.
    pub effect: Expression,
    /// Whether an activated ability is a mana ability.
    pub mana_ability: bool,
}

/// Card ability kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AbilityKind {
    /// Spell resolution instructions.
    Spell,
    /// Activated ability.
    Activated,
    /// Triggered ability.
    Triggered,
    /// Static ability.
    Static,
    /// Replacement or prevention ability.
    Replacement,
}

impl AbilityKind {
    /// Parses a canonical ability kind.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "spell" => Some(Self::Spell),
            "activated" => Some(Self::Activated),
            "triggered" => Some(Self::Triggered),
            "static" => Some(Self::Static),
            "replacement" => Some(Self::Replacement),
            _ => None,
        }
    }

    /// Returns the canonical source name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spell => "spell",
            Self::Activated => "activated",
            Self::Triggered => "triggered",
            Self::Static => "static",
            Self::Replacement => "replacement",
        }
    }
}

/// Validated keyword identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct KeywordId(String);

impl KeywordId {
    /// Creates a syntactically valid keyword id. Compiler registry validation is separate.
    #[must_use]
    pub fn parse(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        is_symbol(&value).then_some(Self(value))
    }

    /// Returns the keyword id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mana cost in printed symbol order.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManaCost {
    /// Parsed symbols.
    pub symbols: Vec<ManaSymbol>,
}

/// One mana symbol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ManaSymbol {
    /// One colored mana.
    Color(Color),
    /// Generic mana amount.
    Generic(u16),
    /// Colorless mana.
    Colorless,
    /// Snow mana.
    Snow,
    /// Variable X, Y, or Z.
    Variable(char),
    /// Two-color hybrid mana.
    Hybrid(Color, Color),
    /// Two generic or one colored mana.
    MonoHybrid(Color),
    /// Colored or two-life Phyrexian mana.
    Phyrexian(Color),
    /// Hybrid-Phyrexian mana.
    HybridPhyrexian(Color, Color),
    /// Half-mana symbol used by historical cards.
    Half(Color),
}

/// Magic color.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Color {
    /// White.
    White,
    /// Blue.
    Blue,
    /// Black.
    Black,
    /// Red.
    Red,
    /// Green.
    Green,
}

impl Color {
    /// Parses W/U/B/R/G.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "W" => Some(Self::White),
            "U" => Some(Self::Blue),
            "B" => Some(Self::Black),
            "R" => Some(Self::Red),
            "G" => Some(Self::Green),
            _ => None,
        }
    }

    /// Returns W/U/B/R/G.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::White => "W",
            Self::Blue => "U",
            Self::Black => "B",
            Self::Red => "R",
            Self::Green => "G",
        }
    }
}

/// Parsed card type line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeLine {
    /// Supertypes before card types.
    pub supertypes: Vec<Supertype>,
    /// One or more card types.
    pub card_types: Vec<CardType>,
    /// Source subtypes after the dash.
    pub subtypes: Vec<String>,
}

/// Closed Magic supertype set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Supertype {
    /// Basic.
    Basic,
    /// Legendary.
    Legendary,
    /// Ongoing.
    Ongoing,
    /// Snow.
    Snow,
    /// World.
    World,
}

/// Closed top-level card type set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CardType {
    /// Artifact.
    Artifact,
    /// Battle.
    Battle,
    /// Creature.
    Creature,
    /// Dungeon.
    Dungeon,
    /// Enchantment.
    Enchantment,
    /// Instant.
    Instant,
    /// Kindred.
    Kindred,
    /// Land.
    Land,
    /// Phenomenon.
    Phenomenon,
    /// Plane.
    Plane,
    /// Planeswalker.
    Planeswalker,
    /// Scheme.
    Scheme,
    /// Sorcery.
    Sorcery,
    /// Vanguard.
    Vanguard,
}

/// Recursive typed expression used by abilities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    /// Signed integer literal.
    Integer(i64),
    /// Boolean literal.
    Boolean(bool),
    /// Quoted text literal.
    Text(String),
    /// Validated symbolic literal.
    Symbol(String),
    /// Operation call.
    Call {
        /// Closed operation id.
        operation: Operation,
        /// Ordered arguments.
        arguments: Vec<Expression>,
    },
    /// Ordered expression list.
    List(Vec<Expression>),
}

/// Semantic category for an expression operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationCategory {
    /// Selects objects or players.
    Selector,
    /// Produces a boolean predicate.
    Predicate,
    /// Produces a payment cost.
    Cost,
    /// Describes an observed game event.
    Event,
    /// Mutates game state or creates a choice.
    Effect,
    /// Restricts timing.
    Timing,
    /// Produces a calculated value.
    Value,
}

/// Closed type accepted by one operation argument position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentKind {
    /// An integer literal.
    Integer,
    /// A boolean literal.
    Boolean,
    /// A quoted text literal.
    Text,
    /// A selector operation.
    Selector,
    /// A predicate operation.
    Predicate,
    /// A cost operation.
    Cost,
    /// An event operation.
    Event,
    /// An effect operation.
    Effect,
    /// A timing operation.
    Timing,
    /// A value operation.
    Value,
    /// An integer literal or value operation.
    Number,
    /// A selector operation or quoted text literal.
    SelectorOrText,
    /// A selector or predicate operation.
    SelectorOrPredicate,
    /// A selector, quoted text, integer, or calculated value.
    SelectorTextOrNumber,
    /// A selector, integer, or calculated value.
    SelectorOrNumber,
    /// A predicate operation or quoted text literal.
    PredicateOrText,
    /// A selector or event operation.
    SelectorOrEvent,
    /// A scalar literal or calculated value.
    Scalar,
    /// A scalar, selector, predicate, or event used in a comparison.
    Comparable,
    /// A closed value that can be retained under a remembered key.
    RememberedValue,
}

impl ArgumentKind {
    /// Returns whether an expression belongs to this argument kind.
    #[must_use]
    pub const fn accepts(self, expression: &Expression) -> bool {
        let category = match expression {
            Expression::Call { operation, .. } => Some(operation.category()),
            _ => None,
        };
        match self {
            Self::Integer => matches!(expression, Expression::Integer(_)),
            Self::Boolean => matches!(expression, Expression::Boolean(_)),
            Self::Text => matches!(expression, Expression::Text(_)),
            Self::Selector => matches!(category, Some(OperationCategory::Selector)),
            Self::Predicate => matches!(category, Some(OperationCategory::Predicate)),
            Self::Cost => matches!(category, Some(OperationCategory::Cost)),
            Self::Event => matches!(category, Some(OperationCategory::Event)),
            Self::Effect => matches!(category, Some(OperationCategory::Effect)),
            Self::Timing => matches!(category, Some(OperationCategory::Timing)),
            Self::Value => matches!(category, Some(OperationCategory::Value)),
            Self::Number => {
                matches!(expression, Expression::Integer(_))
                    || matches!(category, Some(OperationCategory::Value))
            }
            Self::SelectorOrText => {
                matches!(expression, Expression::Text(_))
                    || matches!(category, Some(OperationCategory::Selector))
            }
            Self::SelectorOrPredicate => matches!(
                category,
                Some(OperationCategory::Selector | OperationCategory::Predicate)
            ),
            Self::SelectorTextOrNumber => {
                matches!(expression, Expression::Integer(_) | Expression::Text(_))
                    || matches!(
                        category,
                        Some(OperationCategory::Selector | OperationCategory::Value)
                    )
            }
            Self::SelectorOrNumber => {
                matches!(expression, Expression::Integer(_))
                    || matches!(
                        category,
                        Some(OperationCategory::Selector | OperationCategory::Value)
                    )
            }
            Self::PredicateOrText => {
                matches!(expression, Expression::Text(_))
                    || matches!(category, Some(OperationCategory::Predicate))
            }
            Self::SelectorOrEvent => matches!(
                category,
                Some(OperationCategory::Selector | OperationCategory::Event)
            ),
            Self::Scalar => {
                matches!(
                    expression,
                    Expression::Integer(_) | Expression::Boolean(_) | Expression::Text(_)
                ) || matches!(category, Some(OperationCategory::Value))
            }
            Self::Comparable => {
                matches!(
                    expression,
                    Expression::Integer(_) | Expression::Boolean(_) | Expression::Text(_)
                ) || matches!(
                    category,
                    Some(
                        OperationCategory::Selector
                            | OperationCategory::Predicate
                            | OperationCategory::Event
                            | OperationCategory::Value
                    )
                )
            }
            Self::RememberedValue => {
                matches!(
                    expression,
                    Expression::Integer(_) | Expression::Boolean(_) | Expression::Text(_)
                ) || matches!(
                    category,
                    Some(OperationCategory::Selector | OperationCategory::Value)
                )
            }
        }
    }

    /// Returns a stable human-readable type name for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Text => "text",
            Self::Selector => "selector",
            Self::Predicate => "predicate",
            Self::Cost => "cost",
            Self::Event => "event",
            Self::Effect => "effect",
            Self::Timing => "timing",
            Self::Value => "value",
            Self::Number => "integer or value",
            Self::SelectorOrText => "selector or text",
            Self::SelectorOrPredicate => "selector or predicate",
            Self::SelectorTextOrNumber => "selector, text, integer, or value",
            Self::SelectorOrNumber => "selector, integer, or value",
            Self::PredicateOrText => "predicate or text",
            Self::SelectorOrEvent => "selector or event",
            Self::Scalar => "scalar literal or value",
            Self::Comparable => "comparable expression",
            Self::RememberedValue => "rememberable value",
        }
    }
}

macro_rules! operations {
    ($($variant:ident => ($name:literal, $category:ident, $min:expr, $max:expr)),+ $(,)?) => {
        /// Closed card-expression operation registry.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub enum Operation {
            $(
                #[doc = concat!("Card expression operation `", $name, "`.")]
                $variant,
            )+
        }

        impl Operation {
            /// Every operation in stable declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Parses a canonical operation name.
            #[must_use]
            pub fn parse(value: &str) -> Option<Self> {
                match value { $($name => Some(Self::$variant),)+ _ => None }
            }

            /// Returns the canonical operation name.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            /// Returns the operation category.
            #[must_use]
            pub const fn category(self) -> OperationCategory {
                match self { $(Self::$variant => OperationCategory::$category),+ }
            }

            /// Returns the minimum accepted argument count.
            #[must_use]
            pub const fn min_args(self) -> usize {
                match self { $(Self::$variant => $min),+ }
            }

            /// Returns the maximum accepted argument count, or no limit.
            #[must_use]
            pub const fn max_args(self) -> Option<usize> {
                match self { $(Self::$variant => $max),+ }
            }
        }
    };
}

operations! {
    All => ("all", Selector, 1, None),
    Any => ("any", Selector, 0, Some(0)),
    You => ("you", Selector, 0, Some(0)),
    Opponent => ("opponent", Selector, 0, Some(1)),
    Source => ("source", Selector, 0, Some(0)),
    Chosen => ("chosen", Selector, 1, Some(1)),
    Target => ("target", Selector, 1, Some(1)),
    ControllerOf => ("controller_of", Selector, 1, Some(1)),
    OwnerOf => ("owner_of", Selector, 1, Some(1)),
    Triggered => ("triggered", Selector, 0, Some(1)),
    Remembered => ("remembered", Selector, 1, Some(1)),
    EquippedObject => ("equipped_object", Selector, 1, Some(1)),
    EnchantedObject => ("enchanted_object", Selector, 1, Some(1)),
    Cards => ("cards", Selector, 0, None),
    Permanents => ("permanents", Selector, 0, None),
    Spells => ("spells", Selector, 0, None),
    And => ("and", Predicate, 2, None),
    Or => ("or", Predicate, 2, None),
    Not => ("not", Predicate, 1, Some(1)),
    TypeIs => ("type_is", Predicate, 1, Some(1)),
    SupertypeIs => ("supertype_is", Predicate, 1, Some(1)),
    SubtypeIs => ("subtype_is", Predicate, 1, Some(1)),
    ColorIs => ("color_is", Predicate, 1, Some(1)),
    KeywordIs => ("keyword_is", Predicate, 1, Some(1)),
    ZoneIs => ("zone_is", Predicate, 1, Some(1)),
    During => ("during", Predicate, 1, Some(1)),
    ControlledBy => ("controlled_by", Predicate, 1, Some(1)),
    OwnedBy => ("owned_by", Predicate, 1, Some(1)),
    WithCounter => ("with_counter", Predicate, 1, Some(2)),
    Equals => ("equals", Predicate, 2, Some(2)),
    LessThan => ("less_than", Predicate, 2, Some(2)),
    GreaterThan => ("greater_than", Predicate, 2, Some(2)),
    AtLeast => ("at_least", Predicate, 2, Some(2)),
    ManaCost => ("mana_cost", Cost, 1, Some(1)),
    TapSelf => ("tap_self", Cost, 0, Some(0)),
    UntapSelf => ("untap_self", Cost, 0, Some(0)),
    SacrificeSelf => ("sacrifice_self", Cost, 0, Some(0)),
    Sacrifice => ("sacrifice", Cost, 1, Some(2)),
    DiscardCost => ("discard_cost", Cost, 1, Some(2)),
    PayLife => ("pay_life", Cost, 1, Some(1)),
    LoyaltyCost => ("loyalty_cost", Cost, 1, Some(1)),
    RemoveCounterCost => ("remove_counter_cost", Cost, 2, Some(2)),
    ExileCost => ("exile_cost", Cost, 1, Some(2)),
    EventCast => ("event_cast", Event, 0, Some(2)),
    EventEnters => ("event_enters", Event, 0, Some(2)),
    EventLeaves => ("event_leaves", Event, 0, Some(2)),
    EventDies => ("event_dies", Event, 0, Some(2)),
    EventAttacks => ("event_attacks", Event, 0, Some(2)),
    EventBlocks => ("event_blocks", Event, 0, Some(2)),
    EventDamage => ("event_damage", Event, 0, Some(3)),
    EventUpkeep => ("event_upkeep", Event, 0, Some(1)),
    EventDraw => ("event_draw", Event, 0, Some(2)),
    EventDiscard => ("event_discard", Event, 0, Some(2)),
    EventCounterAdded => ("event_counter_added", Event, 0, Some(2)),
    EventZoneChange => ("event_zone_change", Event, 0, Some(2)),
    EventTargeted => ("event_targeted", Event, 0, Some(2)),
    Sequence => ("sequence", Effect, 1, None),
    ChooseOne => ("choose_one", Effect, 2, None),
    ChooseExactly => ("choose_exactly", Effect, 3, None),
    ChooseUpTo => ("choose_up_to", Effect, 2, None),
    DealDamage => ("deal_damage", Effect, 2, Some(3)),
    Destroy => ("destroy", Effect, 1, Some(2)),
    Exile => ("exile", Effect, 1, Some(2)),
    SacrificeEffect => ("sacrifice_effect", Effect, 1, Some(2)),
    MoveZone => ("move_zone", Effect, 2, Some(3)),
    ReturnToHand => ("return_to_hand", Effect, 1, Some(1)),
    Draw => ("draw", Effect, 1, Some(2)),
    DiscardCards => ("discard_cards", Effect, 1, Some(3)),
    Mill => ("mill", Effect, 1, Some(2)),
    GainLife => ("gain_life", Effect, 1, Some(2)),
    LoseLife => ("lose_life", Effect, 1, Some(2)),
    SetLife => ("set_life", Effect, 1, Some(2)),
    AddMana => ("add_mana", Effect, 1, Some(3)),
    CounterSpell => ("counter_spell", Effect, 1, Some(2)),
    Copy => ("copy", Effect, 1, Some(2)),
    CreateToken => ("create_token", Effect, 1, Some(3)),
    AddCounter => ("add_counter", Effect, 2, Some(3)),
    RemoveCounters => ("remove_counters", Effect, 2, Some(3)),
    ModifyPt => ("modify_pt", Effect, 3, Some(4)),
    SetPt => ("set_pt", Effect, 3, Some(4)),
    SwitchPt => ("switch_pt", Effect, 1, Some(1)),
    GrantKeyword => ("grant_keyword", Effect, 2, Some(3)),
    RemoveKeyword => ("remove_keyword", Effect, 2, Some(3)),
    RemoveAllAbilities => ("remove_all_abilities", Effect, 1, Some(2)),
    AddType => ("add_type", Effect, 2, Some(3)),
    SetType => ("set_type", Effect, 2, Some(3)),
    RemoveType => ("remove_type", Effect, 2, Some(3)),
    SetColor => ("set_color", Effect, 2, Some(3)),
    SetTextMarker => ("set_text_marker", Effect, 2, Some(2)),
    SetBasePt => ("set_base_pt", Effect, 3, Some(3)),
    ChangeControl => ("change_control", Effect, 2, Some(3)),
    ChangeTarget => ("change_target", Effect, 2, Some(2)),
    Attach => ("attach", Effect, 2, Some(2)),
    Detach => ("detach", Effect, 1, Some(1)),
    Tap => ("tap", Effect, 1, Some(1)),
    Untap => ("untap", Effect, 1, Some(1)),
    Scry => ("scry", Effect, 1, Some(2)),
    Surveil => ("surveil", Effect, 1, Some(2)),
    SearchLibrary => ("search_library", Effect, 1, None),
    Shuffle => ("shuffle", Effect, 0, Some(1)),
    Reveal => ("reveal", Effect, 1, Some(2)),
    LookAt => ("look_at", Effect, 1, Some(2)),
    Remember => ("remember", Effect, 2, Some(2)),
    Forget => ("forget", Effect, 1, Some(1)),
    PreventDamage => ("prevent_damage", Effect, 1, Some(3)),
    ReplaceEvent => ("replace_event", Effect, 2, Some(3)),
    DoubleEvent => ("double_event", Effect, 1, Some(2)),
    ExtraTurn => ("extra_turn", Effect, 0, Some(1)),
    ExtraCombat => ("extra_combat", Effect, 0, Some(1)),
    SkipStep => ("skip_step", Effect, 1, Some(2)),
    RegisterDelayedTrigger => ("register_delayed_trigger", Effect, 2, Some(3)),
    LayerEffect => ("layer_effect", Effect, 4, None),
    Continuous => ("continuous", Effect, 2, Some(3)),
    CannotAttack => ("cannot_attack", Effect, 1, Some(2)),
    CannotBlock => ("cannot_block", Effect, 1, Some(2)),
    CanBlockOnly => ("can_block_only", Effect, 2, Some(2)),
    CannotBeBlockedBy => ("cannot_be_blocked_by", Effect, 2, Some(2)),
    CannotCast => ("cannot_cast", Effect, 1, Some(2)),
    DamageCannotBePrevented => ("damage_cannot_be_prevented", Effect, 1, Some(2)),
    SpendManaAsAnyColor => ("spend_mana_as_any_color", Effect, 1, Some(1)),
    NoMaximumHandSize => ("no_maximum_hand_size", Effect, 1, Some(1)),
    AdditionalLandPlays => ("additional_land_plays", Effect, 2, Some(2)),
    CostReduction => ("cost_reduction", Effect, 2, Some(2)),
    AlternateCost => ("alternate_cost", Effect, 2, None),
    DelveCost => ("delve_cost", Effect, 1, Some(1)),
    PlayExiled => ("play_exiled", Effect, 2, Some(2)),
    ActivationLimit => ("activation_limit", Effect, 3, Some(3)),
    UntilEndOfTurn => ("until_end_of_turn", Effect, 1, Some(1)),
    WhileCondition => ("while_condition", Effect, 2, Some(2)),
    Cast => ("cast", Effect, 1, Some(2)),
    Play => ("play", Effect, 1, Some(2)),
    Venture => ("venture", Effect, 0, Some(1)),
    TakeInitiative => ("take_initiative", Effect, 0, Some(1)),
    BecomeMonarch => ("become_monarch", Effect, 0, Some(1)),
    Proliferate => ("proliferate", Effect, 0, Some(1)),
    Populate => ("populate", Effect, 0, Some(1)),
    Transform => ("transform", Effect, 1, Some(1)),
    Meld => ("meld", Effect, 2, Some(3)),
    LevelUp => ("level_up", Effect, 1, Some(2)),
    Vote => ("vote", Effect, 1, Some(2)),
    TimingInstant => ("timing_instant", Timing, 0, Some(0)),
    TimingSorcery => ("timing_sorcery", Timing, 0, Some(0)),
    TimingOnceEachTurn => ("timing_once_each_turn", Timing, 0, Some(1)),
    TimingYourTurn => ("timing_your_turn", Timing, 0, Some(0)),
    Count => ("count", Value, 1, Some(1)),
    Amount => ("amount", Value, 1, Some(2)),
    ManaValue => ("mana_value", Value, 1, Some(1)),
    Power => ("power", Value, 1, Some(1)),
    Toughness => ("toughness", Value, 1, Some(1)),
    IfElse => ("if_else", Value, 3, Some(3)),
    ForEach => ("for_each", Effect, 2, Some(2)),
    BooleanIs => ("boolean_is", Predicate, 1, Some(1)),
    Nonzero => ("nonzero", Predicate, 1, Some(1)),
    AtTiming => ("at_timing", Effect, 2, Some(2)),
    DesignationIs => ("designation_is", Predicate, 1, Some(1)),
    AttachedTo => ("attached_to", Predicate, 1, Some(1)),
    EventWhen => ("event_when", Event, 2, Some(2)),
    TimingCondition => ("timing_condition", Timing, 1, Some(1)),
    PaidX => ("paid_x", Value, 0, Some(0)),
    CounterCount => ("counter_count", Value, 2, Some(2)),
    Devotion => ("devotion", Value, 2, Some(2)),
    DistinctCount => ("distinct_count", Value, 2, Some(2)),
    HistoryCount => ("history_count", Value, 2, Some(2)),
    UnlessPaid => ("unless_paid", Effect, 3, None),
    TimingAll => ("timing_all", Timing, 2, None),
    EventChapter => ("event_chapter", Event, 3, Some(3)),
    MoveZoneFrom => ("move_zone_from", Effect, 3, Some(4)),
    AddRestrictedMana => ("add_restricted_mana", Effect, 4, Some(4)),
    LibraryDig => ("library_dig", Effect, 7, Some(7)),
    Chapter => ("chapter", Effect, 3, Some(3)),
}

impl Operation {
    /// Returns the first variadic position and its closed repeated type.
    #[must_use]
    pub const fn variadic_signature(self) -> Option<(usize, ArgumentKind)> {
        use ArgumentKind::{
            Cost, Effect, Integer, Predicate, PredicateOrText, Scalar, Selector, Timing,
        };

        match self {
            Self::All => Some((0, Selector)),
            Self::Cards | Self::Permanents | Self::Spells => Some((0, PredicateOrText)),
            Self::And | Self::Or => Some((0, Predicate)),
            Self::Sequence | Self::ChooseOne => Some((0, Effect)),
            Self::ChooseExactly | Self::ChooseUpTo => Some((1, Effect)),
            Self::SearchLibrary => Some((2, Scalar)),
            Self::LayerEffect => Some((3, Integer)),
            Self::AlternateCost => Some((1, Cost)),
            Self::UnlessPaid => Some((2, Cost)),
            Self::TimingAll => Some((0, Timing)),
            _ => None,
        }
    }

    /// Returns the closed argument type for one accepted position.
    #[must_use]
    pub const fn argument_kind(self, index: usize) -> Option<ArgumentKind> {
        use ArgumentKind::{
            Boolean, Comparable, Effect, Event, Number, Predicate, PredicateOrText,
            RememberedValue, Scalar, Selector, SelectorOrEvent, SelectorOrNumber,
            SelectorOrPredicate, SelectorOrText, SelectorTextOrNumber, Text, Timing, Value,
        };

        if let Some((first, kind)) = self.variadic_signature() {
            if index >= first {
                return Some(kind);
            }
        }

        match self {
            Self::All => Some(Selector),
            Self::Any | Self::You | Self::Source => None,
            Self::Opponent => Some(Selector),
            Self::Chosen | Self::Target => Some(SelectorOrPredicate),
            Self::ControllerOf | Self::OwnerOf => Some(Selector),
            Self::Triggered => Some(Selector),
            Self::Remembered => Some(Text),
            Self::EquippedObject | Self::EnchantedObject => Some(Selector),
            Self::Cards | Self::Permanents | Self::Spells => Some(PredicateOrText),

            Self::And | Self::Or => Some(Predicate),
            Self::Not => Some(Predicate),
            Self::TypeIs
            | Self::SupertypeIs
            | Self::SubtypeIs
            | Self::ColorIs
            | Self::KeywordIs
            | Self::ZoneIs
            | Self::During
            | Self::DesignationIs => Some(Text),
            Self::ControlledBy | Self::OwnedBy | Self::AttachedTo => Some(Selector),
            Self::WithCounter => match index {
                0 => Some(Text),
                1 => Some(Number),
                _ => None,
            },
            Self::Equals | Self::LessThan | Self::GreaterThan | Self::AtLeast => Some(Comparable),
            Self::BooleanIs => Some(Boolean),
            Self::Nonzero => Some(Value),

            Self::ManaCost => Some(Text),
            Self::TapSelf | Self::UntapSelf | Self::SacrificeSelf => None,
            Self::Sacrifice | Self::ExileCost => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::DiscardCost => match index {
                0 => Some(Number),
                1 => Some(Selector),
                _ => None,
            },
            Self::PayLife | Self::LoyaltyCost => Some(Number),
            Self::RemoveCounterCost => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },

            Self::EventCast
            | Self::EventEnters
            | Self::EventLeaves
            | Self::EventDies
            | Self::EventAttacks
            | Self::EventBlocks
            | Self::EventDraw
            | Self::EventDiscard
            | Self::EventTargeted => match index {
                0 => Some(Selector),
                1 => Some(SelectorOrText),
                _ => None,
            },
            Self::EventDamage => match index {
                0 | 1 => Some(Selector),
                2 => Some(Text),
                _ => None,
            },
            Self::EventUpkeep => Some(Selector),
            Self::EventCounterAdded | Self::EventZoneChange => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::EventChapter => match index {
                0 => Some(Selector),
                1 | 2 => Some(Number),
                _ => None,
            },
            Self::EventWhen => match index {
                0 => Some(Event),
                1 => Some(Predicate),
                _ => None,
            },

            Self::Sequence | Self::ChooseOne => Some(Effect),
            Self::ChooseExactly | Self::ChooseUpTo => {
                if index == 0 {
                    Some(Number)
                } else {
                    Some(Effect)
                }
            }
            Self::DealDamage => match index {
                0 => Some(Selector),
                1 => Some(Number),
                2 => Some(SelectorOrText),
                _ => None,
            },
            Self::Destroy => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::Exile => match index {
                0 => Some(Selector),
                1 => Some(SelectorTextOrNumber),
                _ => None,
            },
            Self::SacrificeEffect => Some(Selector),
            Self::MoveZone => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2 => Some(Comparable),
                _ => None,
            },
            Self::MoveZoneFrom => match index {
                0 => Some(Selector),
                1 | 2 => Some(Text),
                3 => Some(Comparable),
                _ => None,
            },
            Self::ReturnToHand | Self::SwitchPt | Self::Detach | Self::Tap | Self::Untap => {
                Some(Selector)
            }
            Self::Draw
            | Self::Mill
            | Self::GainLife
            | Self::LoseLife
            | Self::SetLife
            | Self::Scry
            | Self::Surveil => match index {
                0 => Some(Number),
                1 => Some(Selector),
                _ => None,
            },
            Self::DiscardCards => match index {
                0 => Some(Number),
                1 => Some(Selector),
                2 => Some(Text),
                _ => None,
            },
            Self::AddMana => match index {
                0 => Some(Text),
                1 => Some(Selector),
                2 => Some(Number),
                _ => None,
            },
            Self::AddRestrictedMana => match index {
                0 | 2 => Some(Text),
                1 => Some(Selector),
                3 => Some(Number),
                _ => None,
            },
            Self::CounterSpell | Self::Copy => Some(Selector),
            Self::CreateToken => match index {
                0 => Some(Text),
                1 => Some(Number),
                2 => Some(Selector),
                _ => None,
            },
            Self::AddCounter | Self::RemoveCounters => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2 => Some(Number),
                _ => None,
            },
            Self::ModifyPt | Self::SetPt => match index {
                0 => Some(Selector),
                1 | 2 => Some(Number),
                3 => Some(Text),
                _ => None,
            },
            Self::GrantKeyword | Self::RemoveKeyword => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2 => Some(SelectorOrText),
                _ => None,
            },
            Self::RemoveAllAbilities => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::AddType | Self::SetType | Self::RemoveType | Self::SetColor => match index {
                0 => Some(Selector),
                1 | 2 => Some(Text),
                _ => None,
            },
            Self::SetTextMarker => match index {
                0 => Some(Selector),
                1 => Some(ArgumentKind::Integer),
                _ => None,
            },
            Self::SetBasePt => match index {
                0 => Some(Selector),
                1 | 2 => Some(Number),
                _ => None,
            },
            Self::ChangeControl | Self::ChangeTarget | Self::Attach => Some(Selector),
            Self::SearchLibrary => match index {
                0 | 1 => Some(Selector),
                _ => None,
            },
            Self::Shuffle => Some(Selector),
            Self::Reveal => Some(Selector),
            Self::LookAt => match index {
                0 => Some(Selector),
                1 => Some(SelectorOrNumber),
                _ => None,
            },
            Self::LibraryDig => match index {
                0 | 3 => Some(Selector),
                1 => Some(Number),
                2 => Some(Comparable),
                4..=6 => Some(Text),
                _ => None,
            },
            Self::Remember => match index {
                0 => Some(Text),
                1 => Some(RememberedValue),
                _ => None,
            },
            Self::Forget => Some(Text),
            Self::PreventDamage => match index {
                0 | 1 => Some(SelectorOrText),
                2 => Some(Number),
                _ => None,
            },
            Self::ReplaceEvent => match index {
                0 => Some(SelectorOrEvent),
                1 => Some(Effect),
                2 => Some(Text),
                _ => None,
            },
            Self::DoubleEvent => match index {
                0 => Some(SelectorOrEvent),
                1 => Some(Number),
                _ => None,
            },
            Self::ExtraTurn | Self::ExtraCombat => Some(Selector),
            Self::SkipStep => match index {
                0 => Some(Text),
                1 => Some(Selector),
                _ => None,
            },
            Self::RegisterDelayedTrigger => match index {
                0 => Some(Event),
                1 => Some(Effect),
                2 => Some(Text),
                _ => None,
            },
            Self::Chapter => match index {
                0 | 1 => Some(Number),
                2 => Some(Effect),
                _ => None,
            },
            Self::AtTiming => match index {
                0 => Some(Timing),
                1 => Some(Effect),
                _ => None,
            },
            Self::LayerEffect => match index {
                0 | 1 => Some(Selector),
                2 => Some(Effect),
                _ => None,
            },
            Self::Continuous => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                2 => Some(Text),
                _ => None,
            },
            Self::CannotAttack | Self::CannotBlock => match index {
                0 => Some(Selector),
                1 => Some(Predicate),
                _ => None,
            },
            Self::CanBlockOnly => match index {
                0 => Some(Selector),
                1 => Some(Predicate),
                _ => None,
            },
            Self::CannotBeBlockedBy => Some(Selector),
            Self::CannotCast | Self::DamageCannotBePrevented => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::SpendManaAsAnyColor | Self::NoMaximumHandSize | Self::DelveCost => Some(Selector),
            Self::AdditionalLandPlays | Self::CostReduction => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::AlternateCost => {
                if index == 0 {
                    Some(Selector)
                } else {
                    None
                }
            }
            Self::PlayExiled => Some(Selector),
            Self::ActivationLimit => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2 => Some(Number),
                _ => None,
            },
            Self::UntilEndOfTurn => Some(Effect),
            Self::WhileCondition => {
                if index == 0 {
                    Some(Predicate)
                } else {
                    Some(Effect)
                }
            }
            Self::Cast | Self::Play => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::Venture
            | Self::TakeInitiative
            | Self::BecomeMonarch
            | Self::Proliferate
            | Self::Populate => Some(Selector),
            Self::Transform => Some(Selector),
            Self::Meld => Some(Selector),
            Self::LevelUp => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::Vote => match index {
                0 => Some(Text),
                1 => Some(Selector),
                _ => None,
            },

            Self::TimingInstant | Self::TimingSorcery | Self::TimingYourTurn => None,
            Self::TimingOnceEachTurn => Some(Predicate),
            Self::TimingCondition => Some(Predicate),
            Self::Count => Some(SelectorOrText),
            Self::Amount => Some(Comparable),
            Self::PaidX => None,
            Self::CounterCount | Self::Devotion | Self::DistinctCount | Self::HistoryCount => {
                match index {
                    0 => Some(Selector),
                    1 => Some(Text),
                    _ => None,
                }
            }
            Self::ManaValue | Self::Power | Self::Toughness => Some(Selector),
            Self::IfElse => {
                if index == 0 {
                    Some(Predicate)
                } else {
                    Some(Scalar)
                }
            }
            Self::ForEach => {
                if index == 0 {
                    Some(Selector)
                } else {
                    Some(Effect)
                }
            }
            Self::UnlessPaid => match index {
                0 => Some(Effect),
                1 => Some(Selector),
                _ => None,
            },
            Self::TimingAll => None,
        }
    }
}

fn is_symbol(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == '_')
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

#[cfg(test)]
mod tests {
    use super::{ArgumentKind, CardLayout, Operation, OperationCategory, OracleId};

    #[test]
    fn ids_are_portable_and_nonempty() {
        assert!(OracleId::parse("b34bb2dc-c1af-4d77-b0b3-a0fb342a5fc6").is_some());
        assert!(OracleId::parse("source:abc_123").is_some());
        assert!(OracleId::parse("").is_none());
        assert!(OracleId::parse("has spaces").is_none());
    }

    #[test]
    fn layout_registry_is_closed() {
        assert_eq!(CardLayout::parse("modal_dfc"), Some(CardLayout::ModalDfc));
        assert_eq!(CardLayout::parse("unknown_layout"), None);
    }

    #[test]
    fn operation_registry_reports_context_and_arity() {
        let operation = Operation::parse("deal_damage");
        assert_eq!(operation, Some(Operation::DealDamage));
        let operation = operation.unwrap_or(Operation::Sequence);
        assert_eq!(operation.category(), OperationCategory::Effect);
        assert_eq!(operation.min_args(), 2);
        assert_eq!(operation.max_args(), Some(3));
        assert_eq!(operation.argument_kind(0), Some(ArgumentKind::Selector));
        assert_eq!(operation.argument_kind(1), Some(ArgumentKind::Number));
        assert_eq!(operation.argument_kind(3), None);
        assert_eq!(
            Operation::LayerEffect.argument_kind(2),
            Some(ArgumentKind::Effect)
        );
        assert_eq!(
            Operation::EventWhen.argument_kind(0),
            Some(ArgumentKind::Event)
        );
        assert_eq!(
            Operation::EventWhen.argument_kind(1),
            Some(ArgumentKind::Predicate)
        );
        assert_eq!(
            Operation::TimingCondition.argument_kind(0),
            Some(ArgumentKind::Predicate)
        );
        assert_eq!(
            Operation::UnlessPaid.argument_kind(0),
            Some(ArgumentKind::Effect)
        );
        assert_eq!(
            Operation::UnlessPaid.argument_kind(1),
            Some(ArgumentKind::Selector)
        );
        assert_eq!(
            Operation::UnlessPaid.argument_kind(2),
            Some(ArgumentKind::Cost)
        );
        assert_eq!(
            Operation::TimingAll.argument_kind(3),
            Some(ArgumentKind::Timing)
        );
        assert_eq!(Operation::parse("card_specific_magic"), None);
    }

    #[test]
    fn operation_serialization_prefix_remains_stable() {
        assert_eq!(Operation::EventCounterAdded as u32, 53);
        assert_eq!(Operation::EventZoneChange as u32, 54);
        assert_eq!(Operation::MoveZone as u32, 64);
        assert_eq!(Operation::ReturnToHand as u32, 65);
        assert_eq!(Operation::AddMana as u32, 72);
        assert_eq!(Operation::CounterSpell as u32, 73);
        assert_eq!(Operation::LookAt as u32, 101);
        assert_eq!(Operation::Remember as u32, 102);
        assert_eq!(Operation::RegisterDelayedTrigger as u32, 110);
        assert_eq!(Operation::LayerEffect as u32, 111);
        assert_eq!(Operation::TimingAll as u32, 164);
        assert_eq!(Operation::EventChapter as u32, 165);
    }

    #[test]
    fn every_declared_operation_argument_has_a_closed_type() {
        for &operation in Operation::ALL {
            let checked_positions = operation.max_args().unwrap_or_else(|| {
                let Some((first, kind)) = operation.variadic_signature() else {
                    panic!(
                        "{} is unbounded without a variadic signature",
                        operation.as_str()
                    );
                };
                for index in first..first.saturating_add(128) {
                    assert_eq!(
                        operation.argument_kind(index),
                        Some(kind),
                        "{} variadic argument {} changed type",
                        operation.as_str(),
                        index + 1
                    );
                }
                first
            });
            for index in 0..checked_positions.max(operation.min_args()) {
                assert!(
                    operation.argument_kind(index).is_some(),
                    "{} accepts argument {} without a closed type",
                    operation.as_str(),
                    index + 1
                );
            }
        }
    }
}
