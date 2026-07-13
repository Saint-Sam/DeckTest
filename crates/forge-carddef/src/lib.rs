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
    CreateToken => ("create_token", Effect, 1, Some(4)),
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
    EventCastTargeting => ("event_cast_targeting", Event, 4, Some(4)),
    RegenerateShield => ("regenerate_shield", Effect, 1, Some(1)),
    HiddenInformation => ("hidden_information", Effect, 1, Some(1)),
    TargetRange => ("target_range", Selector, 3, Some(3)),
    EventPhase => ("event_phase", Event, 2, Some(2)),
    TargetAllocation => ("target_allocation", Selector, 2, Some(2)),
    Negate => ("negate", Value, 1, Some(1)),
    TriggeredAmount => ("triggered_amount", Value, 1, Some(1)),
    OpponentCount => ("opponent_count", Value, 0, Some(0)),
    Crew => ("crew", Effect, 2, Some(2)),
    EffectResult => ("effect_result", Selector, 0, Some(0)),
    Targets => ("targets", Predicate, 1, Some(1)),
    CannotUntap => ("cannot_untap", Effect, 1, Some(2)),
    EventActiveZone => ("event_active_zone", Event, 2, Some(2)),
    ParentTarget => ("parent_target", Selector, 0, Some(0)),
    TriggeredPlayer => ("triggered_player", Selector, 0, Some(0)),
    TriggeredTarget => ("triggered_target", Selector, 0, Some(0)),
    TriggeredActivator => ("triggered_activator", Selector, 0, Some(0)),
    TriggeredStackAbility => ("triggered_stack_ability", Selector, 0, Some(0)),
    ParentStackAbility => ("parent_stack_ability", Selector, 0, Some(0)),
    ChooseBetween => ("choose_between", Effect, 4, None),
    EventTapped => ("event_tapped", Event, 1, Some(1)),
    EventLifeGained => ("event_life_gained", Event, 1, Some(1)),
    EventCycled => ("event_cycled", Event, 1, Some(2)),
    EventSacrificed => ("event_sacrificed", Event, 1, Some(2)),
    EventZoneChangeAll => ("event_zone_change_all", Event, 3, Some(3)),
    PreventAllCombatDamage => ("prevent_all_combat_damage", Effect, 0, Some(0)),
    Fight => ("fight", Effect, 2, Some(2)),
    MustAttack => ("must_attack", Effect, 1, Some(2)),
    Explore => ("explore", Effect, 1, Some(2)),
    Connive => ("connive", Effect, 1, Some(2)),
    MinimumBlockers => ("minimum_blockers", Effect, 1, Some(1)),
    MaximumBlockers => ("maximum_blockers", Effect, 1, Some(1)),
    CastWithFlash => ("cast_with_flash", Effect, 1, Some(1)),
    AffinityCostReduction => ("affinity_cost_reduction", Effect, 1, Some(1)),
    Unearth => ("unearth", Effect, 1, Some(1)),
    Morph => ("morph", Effect, 1, Some(1)),
    WardCost => ("ward_cost", Effect, 2, None),
    EchoCost => ("echo_cost", Effect, 2, None),
    CumulativeUpkeepCost => ("cumulative_upkeep_cost", Effect, 2, None),
    Suspend => ("suspend", Effect, 3, None),
    CostBundle => ("cost_bundle", Cost, 1, None),
    KickerCost => ("kicker_cost", Effect, 2, None),
    MultikickerCost => ("multikicker_cost", Effect, 2, None),
    BuybackCost => ("buyback_cost", Effect, 2, None),
    AlternateAdditionalCost => ("alternate_additional_cost", Effect, 2, None),
    EventTurnedFaceUp => ("event_turned_face_up", Event, 1, Some(1)),
    ProtectionFrom => ("protection_from", Effect, 2, Some(2)),
    Disguise => ("disguise", Effect, 2, None),
    Megamorph => ("megamorph", Effect, 2, None),
    EntwineCost => ("entwine_cost", Effect, 2, None),
    Toxic => ("toxic", Effect, 2, Some(2)),
    Bushido => ("bushido", Effect, 2, Some(2)),
    Soulshift => ("soulshift", Effect, 2, Some(2)),
    Ninjutsu => ("ninjutsu", Effect, 1, Some(1)),
    Saddle => ("saddle", Effect, 2, Some(2)),
    Encore => ("encore", Effect, 1, Some(1)),
    Embalm => ("embalm", Effect, 1, Some(1)),
    Eternalize => ("eternalize", Effect, 1, Some(1)),
    Plot => ("plot", Effect, 1, Some(1)),
    WarpCost => ("warp_cost", Effect, 2, None),
    SneakCost => ("sneak_cost", Effect, 2, None),
    StriveCost => ("strive_cost", Effect, 2, None),
    ReplicateCost => ("replicate_cost", Effect, 2, None),
    MiracleCost => ("miracle_cost", Effect, 2, None),
    OffspringCost => ("offspring_cost", Effect, 2, None),
    Firebending => ("firebending", Effect, 2, Some(2)),
    Vanishing => ("vanishing", Effect, 2, Some(2)),
    Fading => ("fading", Effect, 2, Some(2)),
    Prototype => ("prototype", Effect, 4, Some(4)),
    Station => ("station", Effect, 2, Some(2)),
    Renown => ("renown", Effect, 2, Some(2)),
    Bloodthirst => ("bloodthirst", Effect, 2, Some(2)),
    Fabricate => ("fabricate", Effect, 2, Some(2)),
    Modular => ("modular", Effect, 2, Some(2)),
    Devour => ("devour", Effect, 2, Some(2)),
    Teamwork => ("teamwork", Effect, 2, Some(2)),
    StartingIntensity => ("starting_intensity", Effect, 2, Some(2)),
    Casualty => ("casualty", Effect, 2, Some(2)),
    Reconfigure => ("reconfigure", Effect, 1, Some(1)),
    MutateCost => ("mutate_cost", Effect, 2, None),
    EmergeCost => ("emerge_cost", Effect, 2, None),
    SpliceCost => ("splice_cost", Effect, 3, None),
    AwakenCost => ("awaken_cost", Effect, 3, None),
    StartYourEngines => ("start_your_engines", Effect, 1, Some(1)),
    ChooseBackground => ("choose_background", Effect, 1, Some(1)),
    DoctorsCompanion => ("doctors_companion", Effect, 1, Some(1)),
    Bargain => ("bargain", Effect, 1, Some(1)),
    PartnerWith => ("partner_with", Effect, 2, None),
    PartnerGroup => ("partner_group", Effect, 2, Some(2)),
    GrantProtection => ("grant_protection", Effect, 2, None),
    ChooseType => ("choose_type", Effect, 2, None),
    TriggeredDefendingPlayer => ("triggered_defending_player", Selector, 0, Some(0)),
    EventLimit => ("event_limit", Event, 3, Some(3)),
    ChooseObjects => ("choose_objects", Effect, 3, Some(4)),
    Aggregate => ("aggregate", Value, 2, Some(2)),
    CannotBeCountered => ("cannot_be_countered", Effect, 1, Some(1)),
    EventCounterAttempt => ("event_counter_attempt", Event, 1, Some(1)),
    CostIncrease => ("cost_increase", Effect, 2, Some(2)),
    TimesKicked => ("times_kicked", Value, 0, Some(0)),
    AddReflectedMana => ("add_reflected_mana", Effect, 5, Some(5)),
    PlayerCount => ("player_count", Value, 0, Some(0)),
    ReorderLibraryTop => ("reorder_library_top", Effect, 3, Some(3)),
    ConjureCard => ("conjure_card", Effect, 3, Some(3)),
    AlterAttribute => ("alter_attribute", Effect, 3, Some(3)),
    PlayerAggregate => ("player_aggregate", Value, 1, Some(1)),
    Amass => ("amass", Effect, 3, Some(3)),
    ChosenTypeIs => ("chosen_type_is", Predicate, 0, Some(0)),
    LifeTotal => ("life_total", Value, 1, Some(1)),
    PayToApply => ("pay_to_apply", Effect, 3, None),
    PlayPermission => ("play_permission", Effect, 5, Some(5)),
    GrantActivatedAbility => ("grant_activated_ability", Effect, 3, None),
    GrantTriggeredAbility => ("grant_triggered_ability", Effect, 3, Some(3)),
    RegisterEffectTrigger => ("register_effect_trigger", Effect, 6, Some(7)),
    Perpetual => ("perpetual", Effect, 1, Some(1)),
    BindTargets => ("bind_targets", Effect, 1, Some(1)),
    CannotHaveKeyword => ("cannot_have_keyword", Effect, 2, Some(2)),
    MustBeBlocked => ("must_be_blocked", Effect, 1, Some(1)),
    RegisterEffectReplacement => ("register_effect_replacement", Effect, 6, Some(7)),
    RollDice => ("roll_dice", Effect, 4, Some(4)),
    RollResult => ("roll_result", Value, 1, Some(1)),
    RollDiceTable => ("roll_dice_table", Effect, 5, None),
    PeekLibrary => ("peek_library", Effect, 7, Some(7)),
    EventStatic => ("event_static", Event, 1, Some(1)),
    RegisterEffectStatic => ("register_effect_static", Effect, 5, Some(6)),
    GrantStaticAbility => ("grant_static_ability", Effect, 2, Some(2)),
    GrantReplacementAbility => ("grant_replacement_ability", Effect, 3, Some(3)),
    ApplyInZones => ("apply_in_zones", Effect, 3, None),
    RememberedLki => ("remembered_lki", Selector, 0, Some(0)),
    LibraryDigUntil => ("library_dig_until", Effect, 13, Some(13)),
    ChoosePlayer => ("choose_player", Effect, 5, Some(5)),
    PlayerChooseEffect => ("player_choose_effect", Effect, 7, None),
    SeekLibrary => ("seek_library", Effect, 5, Some(5)),
    ReplacementValue => ("replacement_value", Value, 1, Some(1)),
    ScaleValue => ("scale_value", Value, 2, Some(2)),
    AddValue => ("add_value", Value, 2, Some(2)),
    UpdateReplacementAmount => ("update_replacement_amount", Effect, 2, Some(2)),
    CloneCharacteristics => ("clone_characteristics", Effect, 2, Some(2)),
    TriggeredAttacker => ("triggered_attacker", Selector, 0, Some(0)),
    TriggeredBlocker => ("triggered_blocker", Selector, 0, Some(0)),
    RememberOn => ("remember_on", Effect, 2, Some(2)),
    BranchEffect => ("branch_effect", Effect, 3, Some(3)),
    FlipCoin => ("flip_coin", Effect, 5, Some(5)),
    GrantSVar => ("grant_svar", Effect, 3, Some(3)),
    RegisterDelayedTriggerRemembering => ("register_delayed_trigger_remembering", Effect, 3, Some(4)),
    EventImmediate => ("event_immediate", Event, 0, Some(0)),
    EventController => ("event_controller", Event, 2, Some(2)),
    ForEachImprinted => ("for_each_imprinted", Effect, 2, Some(2)),
    OrderByPlayer => ("order_by_player", Selector, 2, Some(2)),
    BatchEvents => ("batch_events", Effect, 2, Some(2)),
    Imprinted => ("imprinted", Selector, 1, Some(1)),
    Bolster => ("bolster", Effect, 2, Some(2)),
    Support => ("support", Effect, 2, Some(2)),
    Adapt => ("adapt", Effect, 2, Some(2)),
    Monstrosity => ("monstrosity", Effect, 2, Some(2)),
    CopyTriggeredCounters => ("copy_triggered_counters", Effect, 2, Some(3)),
    LookPermission => ("look_permission", Effect, 3, Some(3)),
    PlayFromZone => ("play_from_zone", Effect, 3, Some(3)),
    EventChaosEnsues => ("event_chaos_ensues", Event, 0, Some(0)),
    EventSetInMotion => ("event_set_in_motion", Event, 1, Some(1)),
    ChooseCardName => ("choose_card_name", Effect, 3, Some(4)),
    PutCreatedAttacking => ("put_created_attacking", Effect, 2, Some(2)),
}

impl Operation {
    /// Returns the first variadic position and its closed repeated type.
    #[must_use]
    pub const fn variadic_signature(self) -> Option<(usize, ArgumentKind)> {
        use ArgumentKind::{
            Cost, Effect, Integer, Predicate, PredicateOrText, Scalar, Selector, Text, Timing,
        };

        match self {
            Self::All => Some((0, Selector)),
            Self::Cards | Self::Permanents | Self::Spells => Some((0, PredicateOrText)),
            Self::And | Self::Or => Some((0, Predicate)),
            Self::Sequence | Self::ChooseOne => Some((0, Effect)),
            Self::ChooseExactly | Self::ChooseUpTo => Some((1, Effect)),
            Self::ChooseBetween => Some((2, Effect)),
            Self::SearchLibrary => Some((2, Scalar)),
            Self::LayerEffect => Some((3, Integer)),
            Self::AlternateCost => Some((1, Cost)),
            Self::UnlessPaid => Some((2, Cost)),
            Self::PayToApply => Some((2, Cost)),
            Self::GrantActivatedAbility => Some((3, Cost)),
            Self::RollDiceTable => Some((4, Effect)),
            Self::ApplyInZones => Some((2, Text)),
            Self::PlayerChooseEffect => Some((5, Effect)),
            Self::WardCost | Self::EchoCost => Some((1, Cost)),
            Self::CumulativeUpkeepCost => Some((1, Cost)),
            Self::Suspend => Some((2, Cost)),
            Self::CostBundle => Some((0, Cost)),
            Self::KickerCost
            | Self::MultikickerCost
            | Self::BuybackCost
            | Self::AlternateAdditionalCost
            | Self::Disguise
            | Self::Megamorph
            | Self::EntwineCost
            | Self::WarpCost
            | Self::SneakCost
            | Self::StriveCost
            | Self::ReplicateCost
            | Self::MiracleCost
            | Self::OffspringCost
            | Self::MutateCost
            | Self::EmergeCost => Some((1, Cost)),
            Self::SpliceCost | Self::AwakenCost => Some((2, Cost)),
            Self::PartnerWith | Self::GrantProtection | Self::ChooseType => {
                Some((1, ArgumentKind::Text))
            }
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
            Self::Any
            | Self::You
            | Self::Source
            | Self::EffectResult
            | Self::RememberedLki
            | Self::TriggeredAttacker
            | Self::TriggeredBlocker
            | Self::ParentTarget
            | Self::TriggeredPlayer
            | Self::TriggeredTarget
            | Self::TriggeredActivator
            | Self::TriggeredDefendingPlayer
            | Self::TriggeredStackAbility
            | Self::ParentStackAbility => None,
            Self::Opponent => Some(Selector),
            Self::Chosen | Self::Target => Some(SelectorOrPredicate),
            Self::TargetRange => match index {
                0 => Some(SelectorOrPredicate),
                1 | 2 => Some(Number),
                _ => None,
            },
            Self::TargetAllocation => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
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
            Self::ControlledBy | Self::OwnedBy | Self::AttachedTo | Self::Targets => Some(Selector),
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
            | Self::EventDiscard
            | Self::EventTargeted => match index {
                0 => Some(Selector),
                1 => Some(SelectorOrText),
                _ => None,
            },
            Self::EventDraw => match index {
                0 => Some(Selector),
                1 => Some(SelectorTextOrNumber),
                _ => None,
            },
            Self::EventTapped | Self::EventLifeGained => Some(Selector),
            Self::EventCycled | Self::EventSacrificed => Some(Selector),
            Self::EventTurnedFaceUp => Some(Selector),
            Self::EventCastTargeting => match index {
                0..=2 => Some(Selector),
                3 => Some(Text),
                _ => None,
            },
            Self::EventDamage => match index {
                0 | 1 => Some(Selector),
                2 => Some(Text),
                _ => None,
            },
            Self::EventUpkeep => Some(Selector),
            Self::EventPhase => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::EventCounterAdded | Self::EventZoneChange => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::EventZoneChangeAll => match index {
                0 => Some(Selector),
                1 | 2 => Some(Text),
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
            Self::EventActiveZone => match index {
                0 => Some(Event),
                1 => Some(Text),
                _ => None,
            },
            Self::EventLimit => match index {
                0 => Some(Event),
                1 => Some(Selector),
                2 => Some(Number),
                _ => None,
            },
            Self::ChooseObjects => match index {
                0 | 2 => Some(Selector),
                1 => Some(Number),
                3 => Some(Text),
                _ => None,
            },
            Self::Aggregate => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::CannotBeCountered | Self::EventCounterAttempt => Some(Selector),

            Self::Sequence | Self::ChooseOne => Some(Effect),
            Self::ChooseExactly | Self::ChooseUpTo => {
                if index == 0 {
                    Some(Number)
                } else {
                    Some(Effect)
                }
            }
            Self::ChooseBetween => {
                if index < 2 {
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
            Self::SacrificeEffect | Self::RegenerateShield => Some(Selector),
            Self::MustAttack => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::Fight => Some(Selector),
            Self::Explore | Self::Connive => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::MinimumBlockers | Self::MaximumBlockers => Some(Number),
            Self::CastWithFlash => Some(Selector),
            Self::ProtectionFrom => match index {
                0 => Some(Selector),
                1 => Some(SelectorOrText),
                _ => None,
            },
            Self::Disguise | Self::Megamorph | Self::EntwineCost => Some(Selector),
            Self::Toxic | Self::Bushido | Self::Soulshift => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::Ninjutsu => Some(Selector),
            Self::Saddle => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::Encore | Self::Embalm | Self::Eternalize | Self::Plot => Some(Selector),
            Self::WarpCost
            | Self::SneakCost
            | Self::StriveCost
            | Self::ReplicateCost
            | Self::MiracleCost
            | Self::OffspringCost => Some(Selector),
            Self::Firebending | Self::Vanishing | Self::Fading => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::Prototype => match index {
                0 => Some(Selector),
                1 => Some(ArgumentKind::Cost),
                2 | 3 => Some(Number),
                _ => None,
            },
            Self::Station
            | Self::Renown
            | Self::Bloodthirst
            | Self::Fabricate
            | Self::Modular
            | Self::Devour
            | Self::Teamwork
            | Self::StartingIntensity
            | Self::Casualty => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::Reconfigure => Some(Selector),
            Self::MutateCost | Self::EmergeCost => Some(Selector),
            Self::SpliceCost => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::AwakenCost => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::StartYourEngines
            | Self::ChooseBackground
            | Self::DoctorsCompanion
            | Self::Bargain => Some(Selector),
            Self::PartnerWith | Self::PartnerGroup => match index {
                0 => Some(Selector),
                _ => Some(Text),
            },
            Self::GrantProtection => match index {
                0 => Some(Selector),
                _ => Some(Text),
            },
            Self::ChooseType => match index {
                0 => Some(Selector),
                _ => Some(Text),
            },
            Self::AffinityCostReduction => Some(Selector),
            Self::Unearth => Some(Selector),
            Self::Morph => Some(Selector),
            Self::WardCost | Self::EchoCost => Some(Selector),
            Self::CumulativeUpkeepCost => Some(Selector),
            Self::Suspend => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::CostBundle => None,
            Self::KickerCost
            | Self::MultikickerCost
            | Self::BuybackCost
            | Self::AlternateAdditionalCost => Some(Selector),
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
            Self::AddReflectedMana => match index {
                0 | 3 => Some(Selector),
                1 | 2 => Some(Text),
                4 => Some(Number),
                _ => None,
            },
            Self::ReorderLibraryTop => match index {
                0 => Some(Selector),
                1 => Some(Number),
                2 => Some(Boolean),
                _ => None,
            },
            Self::ConjureCard => match index {
                0 | 1 => Some(Text),
                2 => Some(Selector),
                _ => None,
            },
            Self::AlterAttribute => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2 => Some(Boolean),
                _ => None,
            },
            Self::Amass => match index {
                0 => Some(Text),
                1 => Some(Number),
                2 => Some(Selector),
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
                3 => Some(Text),
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
            Self::HiddenInformation => Some(Effect),
            Self::PreventAllCombatDamage => None,
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
            Self::CannotUntap => match index {
                0 => Some(Selector),
                1 => Some(Text),
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
            Self::AdditionalLandPlays | Self::CostReduction | Self::CostIncrease => match index {
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
            Self::PaidX | Self::TimesKicked | Self::ChosenTypeIs => None,
            Self::OpponentCount | Self::PlayerCount => None,
            Self::PlayerAggregate => Some(Text),
            Self::CounterCount | Self::Devotion | Self::DistinctCount | Self::HistoryCount => {
                match index {
                    0 => Some(Selector),
                    1 => Some(Text),
                    _ => None,
                }
            }
            Self::ManaValue | Self::Power | Self::Toughness => Some(Selector),
            Self::LifeTotal => Some(Selector),
            Self::Negate => Some(Number),
            Self::TriggeredAmount => Some(Text),
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
            Self::Crew => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::UnlessPaid => match index {
                0 => Some(Effect),
                1 => Some(Selector),
                _ => None,
            },
            Self::PayToApply => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                _ => None,
            },
            Self::PlayPermission => match index {
                0 | 2 => Some(Selector),
                1 | 3 | 4 => Some(Text),
                _ => None,
            },
            Self::GrantActivatedAbility => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                2 => Some(Timing),
                _ => None,
            },
            Self::GrantTriggeredAbility => match index {
                0 => Some(Selector),
                1 => Some(Event),
                2 => Some(Effect),
                _ => None,
            },
            Self::RegisterEffectTrigger => match index {
                0 => Some(Selector),
                1 => Some(Event),
                2 => Some(Effect),
                3 | 5 => Some(Text),
                4 => Some(Boolean),
                6 => Some(Selector),
                _ => None,
            },
            Self::Perpetual => Some(Effect),
            Self::BindTargets => Some(Selector),
            Self::CannotHaveKeyword => match index {
                0 => Some(Selector),
                1 => Some(Text),
                _ => None,
            },
            Self::MustBeBlocked => Some(Selector),
            Self::RegisterEffectReplacement => match index {
                0 => Some(Selector),
                1 => Some(Event),
                2 => Some(Effect),
                3 | 5 => Some(Text),
                4 => Some(Boolean),
                6 => Some(Selector),
                _ => None,
            },
            Self::RollDice => match index {
                0 => Some(Selector),
                1 | 2 => Some(Number),
                3 => Some(Text),
                _ => None,
            },
            Self::RollResult => Some(Text),
            Self::RollDiceTable => match index {
                0 => Some(Selector),
                1 | 2 => Some(Number),
                3 => Some(Text),
                _ => None,
            },
            Self::PeekLibrary => match index {
                0 | 1 | 4 => Some(Selector),
                2 => Some(Number),
                3 | 5 | 6 => Some(Text),
                _ => None,
            },
            Self::EventStatic => Some(Event),
            Self::RegisterEffectStatic => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                2 | 4 => Some(Text),
                3 => Some(Boolean),
                5 => Some(Selector),
                _ => None,
            },
            Self::GrantStaticAbility => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                _ => None,
            },
            Self::GrantReplacementAbility => match index {
                0 => Some(Selector),
                1 => Some(Event),
                2 => Some(Effect),
                _ => None,
            },
            Self::ApplyInZones => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                _ => Some(Text),
            },
            Self::LibraryDigUntil => match index {
                0 | 1 => Some(Selector),
                2 | 5 | 7 => Some(Number),
                3 | 4 | 6 => Some(Text),
                8..=12 => Some(Boolean),
                _ => None,
            },
            Self::ChoosePlayer => match index {
                0 | 1 => Some(Selector),
                2..=4 => Some(Boolean),
                _ => None,
            },
            Self::PlayerChooseEffect => match index {
                0 => Some(Selector),
                1 => Some(Text),
                2..=4 => Some(Boolean),
                _ => Some(Effect),
            },
            Self::SeekLibrary => match index {
                0 | 1 => Some(Selector),
                2 => Some(Number),
                3 | 4 => Some(Boolean),
                _ => None,
            },
            Self::ReplacementValue => Some(Text),
            Self::ScaleValue | Self::AddValue => match index {
                0 => Some(Value),
                1 => Some(Number),
                _ => None,
            },
            Self::UpdateReplacementAmount => match index {
                0 => Some(Text),
                1 => Some(Value),
                _ => None,
            },
            Self::CloneCharacteristics => Some(Selector),
            Self::RememberOn => Some(Selector),
            Self::BranchEffect => match index {
                0 => Some(Predicate),
                1 | 2 => Some(Effect),
                _ => None,
            },
            Self::FlipCoin => match index {
                0 => Some(Selector),
                1 => Some(Number),
                2 | 3 => Some(Effect),
                4 => Some(Text),
                _ => None,
            },
            Self::GrantSVar => match index {
                0 => Some(Selector),
                1 | 2 => Some(Text),
                _ => None,
            },
            Self::RegisterDelayedTriggerRemembering => match index {
                0 => Some(Event),
                1 => Some(Effect),
                2 => Some(Selector),
                3 => Some(Text),
                _ => None,
            },
            Self::EventImmediate => None,
            Self::EventController => match index {
                0 => Some(Event),
                1 => Some(Selector),
                _ => None,
            },
            Self::ForEachImprinted => match index {
                0 => Some(Selector),
                1 => Some(Effect),
                _ => None,
            },
            Self::OrderByPlayer => Some(Selector),
            Self::BatchEvents => match index {
                0 => Some(Effect),
                1 => Some(Text),
                _ => None,
            },
            Self::Imprinted => Some(Selector),
            Self::Bolster | Self::Support | Self::Adapt | Self::Monstrosity => match index {
                0 => Some(Selector),
                1 => Some(Number),
                _ => None,
            },
            Self::CopyTriggeredCounters => match index {
                0 | 1 => Some(Selector),
                2 => Some(Number),
                _ => None,
            },
            Self::LookPermission => match index {
                0 | 1 => Some(Selector),
                2 => Some(Text),
                _ => None,
            },
            Self::PlayFromZone => match index {
                0 | 1 => Some(Selector),
                2 => Some(Text),
                _ => None,
            },
            Self::EventChaosEnsues => None,
            Self::EventSetInMotion => Some(Selector),
            Self::ChooseCardName => match index {
                0 | 1 => Some(Selector),
                2 | 3 => Some(Text),
                _ => None,
            },
            Self::PutCreatedAttacking => match index {
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
        assert_eq!(
            Operation::TargetRange.argument_kind(1),
            Some(ArgumentKind::Number)
        );
        assert_eq!(
            Operation::TargetRange.argument_kind(2),
            Some(ArgumentKind::Number)
        );
        assert_eq!(
            Operation::CreateToken.argument_kind(3),
            Some(ArgumentKind::Text)
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
        assert_eq!(Operation::EventCastTargeting as u32, 170);
        assert_eq!(Operation::RegenerateShield as u32, 171);
        assert_eq!(Operation::HiddenInformation as u32, 172);
        assert_eq!(Operation::TargetRange as u32, 173);
        assert_eq!(Operation::EventPhase as u32, 174);
        assert_eq!(Operation::TargetAllocation as u32, 175);
        assert_eq!(Operation::Negate as u32, 176);
        assert_eq!(Operation::TriggeredAmount as u32, 177);
        assert_eq!(Operation::OpponentCount as u32, 178);
        assert_eq!(Operation::EventActiveZone as u32, 183);
        assert_eq!(Operation::ParentTarget as u32, 184);
        assert_eq!(Operation::TriggeredPlayer as u32, 185);
        assert_eq!(Operation::TriggeredTarget as u32, 186);
        assert_eq!(Operation::TriggeredActivator as u32, 187);
        assert_eq!(Operation::TriggeredStackAbility as u32, 188);
        assert_eq!(Operation::ParentStackAbility as u32, 189);
        assert_eq!(Operation::ChooseBetween as u32, 190);
        assert_eq!(Operation::EventTapped as u32, 191);
        assert_eq!(Operation::EventLifeGained as u32, 192);
        assert_eq!(Operation::EventCycled as u32, 193);
        assert_eq!(Operation::EventSacrificed as u32, 194);
        assert_eq!(Operation::EventZoneChangeAll as u32, 195);
        assert_eq!(Operation::PreventAllCombatDamage as u32, 196);
        assert_eq!(Operation::Fight as u32, 197);
        assert_eq!(Operation::MustAttack as u32, 198);
        assert_eq!(Operation::Explore as u32, 199);
        assert_eq!(Operation::Connive as u32, 200);
        assert_eq!(Operation::MinimumBlockers as u32, 201);
        assert_eq!(Operation::MaximumBlockers as u32, 202);
        assert_eq!(Operation::CastWithFlash as u32, 203);
        assert_eq!(Operation::AffinityCostReduction as u32, 204);
        assert_eq!(Operation::Unearth as u32, 205);
        assert_eq!(Operation::Morph as u32, 206);
        assert_eq!(Operation::WardCost as u32, 207);
        assert_eq!(Operation::EchoCost as u32, 208);
        assert_eq!(Operation::CumulativeUpkeepCost as u32, 209);
        assert_eq!(Operation::Suspend as u32, 210);
        assert_eq!(Operation::CostBundle as u32, 211);
        assert_eq!(Operation::KickerCost as u32, 212);
        assert_eq!(Operation::MultikickerCost as u32, 213);
        assert_eq!(Operation::BuybackCost as u32, 214);
        assert_eq!(Operation::AlternateAdditionalCost as u32, 215);
        assert_eq!(Operation::EventTurnedFaceUp as u32, 216);
        assert_eq!(Operation::ProtectionFrom as u32, 217);
        assert_eq!(Operation::Disguise as u32, 218);
        assert_eq!(Operation::Megamorph as u32, 219);
        assert_eq!(Operation::EntwineCost as u32, 220);
        assert_eq!(Operation::Toxic as u32, 221);
        assert_eq!(Operation::Bushido as u32, 222);
        assert_eq!(Operation::Soulshift as u32, 223);
        assert_eq!(Operation::Ninjutsu as u32, 224);
        assert_eq!(Operation::Saddle as u32, 225);
        assert_eq!(Operation::Encore as u32, 226);
        assert_eq!(Operation::Embalm as u32, 227);
        assert_eq!(Operation::Eternalize as u32, 228);
        assert_eq!(Operation::Plot as u32, 229);
        assert_eq!(Operation::WarpCost as u32, 230);
        assert_eq!(Operation::SneakCost as u32, 231);
        assert_eq!(Operation::StriveCost as u32, 232);
        assert_eq!(Operation::ReplicateCost as u32, 233);
        assert_eq!(Operation::MiracleCost as u32, 234);
        assert_eq!(Operation::OffspringCost as u32, 235);
        assert_eq!(Operation::Firebending as u32, 236);
        assert_eq!(Operation::Vanishing as u32, 237);
        assert_eq!(Operation::Fading as u32, 238);
        assert_eq!(Operation::Prototype as u32, 239);
        assert_eq!(Operation::Station as u32, 240);
        assert_eq!(Operation::Renown as u32, 241);
        assert_eq!(Operation::Bloodthirst as u32, 242);
        assert_eq!(Operation::Fabricate as u32, 243);
        assert_eq!(Operation::Modular as u32, 244);
        assert_eq!(Operation::Devour as u32, 245);
        assert_eq!(Operation::Teamwork as u32, 246);
        assert_eq!(Operation::StartingIntensity as u32, 247);
        assert_eq!(Operation::Casualty as u32, 248);
        assert_eq!(Operation::Reconfigure as u32, 249);
        assert_eq!(Operation::MutateCost as u32, 250);
        assert_eq!(Operation::EmergeCost as u32, 251);
        assert_eq!(Operation::SpliceCost as u32, 252);
        assert_eq!(Operation::AwakenCost as u32, 253);
        assert_eq!(Operation::StartYourEngines as u32, 254);
        assert_eq!(Operation::ChooseBackground as u32, 255);
        assert_eq!(Operation::DoctorsCompanion as u32, 256);
        assert_eq!(Operation::Bargain as u32, 257);
        assert_eq!(Operation::PartnerWith as u32, 258);
        assert_eq!(Operation::PartnerGroup as u32, 259);
        assert_eq!(Operation::GrantProtection as u32, 260);
        assert_eq!(Operation::ChooseType as u32, 261);
        assert_eq!(Operation::TriggeredDefendingPlayer as u32, 262);
        assert_eq!(Operation::EventLimit as u32, 263);
        assert_eq!(Operation::ChooseObjects as u32, 264);
        assert_eq!(Operation::Aggregate as u32, 265);
        assert_eq!(Operation::CannotBeCountered as u32, 266);
        assert_eq!(Operation::EventCounterAttempt as u32, 267);
        assert_eq!(Operation::CostIncrease as u32, 268);
        assert_eq!(Operation::TimesKicked as u32, 269);
        assert_eq!(Operation::AddReflectedMana as u32, 270);
        assert_eq!(Operation::PlayerCount as u32, 271);
        assert_eq!(Operation::ReorderLibraryTop as u32, 272);
        assert_eq!(Operation::ConjureCard as u32, 273);
        assert_eq!(Operation::AlterAttribute as u32, 274);
        assert_eq!(Operation::PlayerAggregate as u32, 275);
        assert_eq!(Operation::Amass as u32, 276);
        assert_eq!(Operation::ChosenTypeIs as u32, 277);
        assert_eq!(Operation::LifeTotal as u32, 278);
        assert_eq!(Operation::PayToApply as u32, 279);
        assert_eq!(Operation::PlayPermission as u32, 280);
        assert_eq!(Operation::GrantActivatedAbility as u32, 281);
        assert_eq!(Operation::GrantTriggeredAbility as u32, 282);
        assert_eq!(Operation::RegisterEffectTrigger as u32, 283);
        assert_eq!(Operation::Perpetual as u32, 284);
        assert_eq!(Operation::BindTargets as u32, 285);
        assert_eq!(Operation::CannotHaveKeyword as u32, 286);
        assert_eq!(Operation::MustBeBlocked as u32, 287);
        assert_eq!(Operation::RegisterEffectReplacement as u32, 288);
        assert_eq!(Operation::RollDice as u32, 289);
        assert_eq!(Operation::RollResult as u32, 290);
        assert_eq!(Operation::RollDiceTable as u32, 291);
        assert_eq!(Operation::PeekLibrary as u32, 292);
        assert_eq!(Operation::EventStatic as u32, 293);
        assert_eq!(Operation::RegisterEffectStatic as u32, 294);
        assert_eq!(Operation::GrantStaticAbility as u32, 295);
        assert_eq!(Operation::GrantReplacementAbility as u32, 296);
        assert_eq!(Operation::ApplyInZones as u32, 297);
        assert_eq!(Operation::RememberedLki as u32, 298);
        assert_eq!(Operation::LibraryDigUntil as u32, 299);
        assert_eq!(Operation::ChoosePlayer as u32, 300);
        assert_eq!(Operation::PlayerChooseEffect as u32, 301);
        assert_eq!(Operation::SeekLibrary as u32, 302);
        assert_eq!(Operation::ReplacementValue as u32, 303);
        assert_eq!(Operation::ScaleValue as u32, 304);
        assert_eq!(Operation::AddValue as u32, 305);
        assert_eq!(Operation::UpdateReplacementAmount as u32, 306);
        assert_eq!(Operation::CloneCharacteristics as u32, 307);
        assert_eq!(Operation::TriggeredAttacker as u32, 308);
        assert_eq!(Operation::TriggeredBlocker as u32, 309);
        assert_eq!(Operation::RememberOn as u32, 310);
        assert_eq!(Operation::BranchEffect as u32, 311);
        assert_eq!(Operation::FlipCoin as u32, 312);
        assert_eq!(Operation::GrantSVar as u32, 313);
        assert_eq!(Operation::RegisterDelayedTriggerRemembering as u32, 314);
        assert_eq!(Operation::EventImmediate as u32, 315);
        assert_eq!(Operation::EventController as u32, 316);
        assert_eq!(Operation::ForEachImprinted as u32, 317);
        assert_eq!(Operation::OrderByPlayer as u32, 318);
        assert_eq!(Operation::BatchEvents as u32, 319);
        assert_eq!(Operation::Imprinted as u32, 320);
        assert_eq!(Operation::Bolster as u32, 321);
        assert_eq!(Operation::Support as u32, 322);
        assert_eq!(Operation::Adapt as u32, 323);
        assert_eq!(Operation::Monstrosity as u32, 324);
        assert_eq!(Operation::CopyTriggeredCounters as u32, 325);
        assert_eq!(Operation::LookPermission as u32, 326);
        assert_eq!(Operation::PlayFromZone as u32, 327);
        assert_eq!(Operation::EventChaosEnsues as u32, 328);
        assert_eq!(Operation::EventSetInMotion as u32, 329);
        assert_eq!(Operation::ChooseCardName as u32, 330);
        assert_eq!(Operation::PutCreatedAttacking as u32, 331);

        let config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_little_endian();
        let hidden_bytes = bincode::serde::encode_to_vec(Operation::HiddenInformation, config)
            .unwrap_or_else(|error| panic!("operation encoding must succeed: {error}"));
        assert_eq!(hidden_bytes, 172_u32.to_le_bytes());
        let (decoded, consumed): (Operation, usize) =
            bincode::serde::decode_from_slice(&172_u32.to_le_bytes(), config)
                .unwrap_or_else(|error| panic!("legacy operation bytes must decode: {error}"));
        assert_eq!(decoded, Operation::HiddenInformation);
        assert_eq!(consumed, 4);
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
