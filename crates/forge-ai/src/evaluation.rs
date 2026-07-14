use forge_core::{
    GameOutcome, ObjectCharacteristics, ObjectRecord, ObjectView, PlayerId, PlayerState,
    PlayerView, ZoneId, ZoneKind,
};
use std::{collections::BTreeMap, error::Error, fmt};

const BUNDLED_WEIGHTS: &str = include_str!("../../../assets/ai/ai_weights.ron");

/// Data-driven evaluation weights for the baseline AI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiWeights {
    terminal_win: i64,
    terminal_loss: i64,
    life_delta: i64,
    poison_delta: i64,
    hand_delta: i64,
    creature_power_delta: i64,
    creature_toughness_delta: i64,
    keyword_delta: i64,
    noncreature_permanent_delta: i64,
    noncreature_mana_value_delta: i64,
    pending_permanent_delta: i64,
    pending_mana_value_delta: i64,
    land_delta: i64,
    untapped_land_delta: i64,
    floating_mana_delta: i64,
    tempo_delta: i64,
    attack_power_prior: i64,
    attack_body_prior: i64,
    low_life_pressure_prior: i64,
    block_prevent_prior: i64,
    block_favorable_trade_prior: i64,
    block_losing_trade_prior: i64,
}

impl AiWeights {
    /// Loads the repository's versioned baseline weights.
    pub fn bundled() -> Result<Self, EvaluationError> {
        Self::from_ron_str(BUNDLED_WEIGHTS)
    }

    /// Parses the strict integer-only RON subset used by `ai_weights.ron`.
    pub fn from_ron_str(source: &str) -> Result<Self, EvaluationError> {
        let stripped = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        let document = stripped.trim();
        let body = document
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .ok_or(EvaluationError::InvalidWeightsDocument)?;
        let mut fields = BTreeMap::<String, i64>::new();
        for entry in body
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (name, value) = entry
                .split_once(':')
                .ok_or_else(|| EvaluationError::InvalidWeightEntry(entry.to_owned()))?;
            let name = name.trim().to_owned();
            if !EXPECTED_FIELDS.contains(&name.as_str()) {
                return Err(EvaluationError::UnknownWeight(name));
            }
            let value = value
                .trim()
                .parse::<i64>()
                .map_err(|_| EvaluationError::InvalidWeightValue(name.clone()))?;
            if fields.insert(name.clone(), value).is_some() {
                return Err(EvaluationError::DuplicateWeight(name));
            }
        }

        macro_rules! take {
            ($name:literal) => {
                fields
                    .remove($name)
                    .ok_or(EvaluationError::MissingWeight($name))?
            };
        }
        Ok(Self {
            terminal_win: take!("terminal_win"),
            terminal_loss: take!("terminal_loss"),
            life_delta: take!("life_delta"),
            poison_delta: take!("poison_delta"),
            hand_delta: take!("hand_delta"),
            creature_power_delta: take!("creature_power_delta"),
            creature_toughness_delta: take!("creature_toughness_delta"),
            keyword_delta: take!("keyword_delta"),
            noncreature_permanent_delta: take!("noncreature_permanent_delta"),
            noncreature_mana_value_delta: take!("noncreature_mana_value_delta"),
            pending_permanent_delta: take!("pending_permanent_delta"),
            pending_mana_value_delta: take!("pending_mana_value_delta"),
            land_delta: take!("land_delta"),
            untapped_land_delta: take!("untapped_land_delta"),
            floating_mana_delta: take!("floating_mana_delta"),
            tempo_delta: take!("tempo_delta"),
            attack_power_prior: take!("attack_power_prior"),
            attack_body_prior: take!("attack_body_prior"),
            low_life_pressure_prior: take!("low_life_pressure_prior"),
            block_prevent_prior: take!("block_prevent_prior"),
            block_favorable_trade_prior: take!("block_favorable_trade_prior"),
            block_losing_trade_prior: take!("block_losing_trade_prior"),
        })
    }

    /// Scores a card-agnostic attack declaration for move ordering.
    #[must_use]
    pub const fn attack_prior(
        self,
        total_power: i64,
        attacker_count: i64,
        defender_life: i32,
    ) -> i64 {
        let raw_pressure = 40_i64.saturating_sub(defender_life as i64);
        let pressure = if raw_pressure < 0 { 0 } else { raw_pressure };
        weighted(total_power, self.attack_power_prior)
            .saturating_add(weighted(attacker_count, self.attack_body_prior))
            .saturating_add(weighted(pressure, self.low_life_pressure_prior))
    }

    /// Scores a card-agnostic block declaration for move ordering.
    #[must_use]
    pub const fn block_prior(
        self,
        prevented_power: i64,
        favorable_trades: i64,
        losing_trades: i64,
    ) -> i64 {
        weighted(prevented_power, self.block_prevent_prior)
            .saturating_add(weighted(favorable_trades, self.block_favorable_trade_prior))
            .saturating_add(weighted(losing_trades, self.block_losing_trade_prior))
    }

    /// Scores the visible tactical importance of one targetable object.
    #[must_use]
    pub fn object_threat(
        self,
        object: ObjectRecord,
        characteristics: ObjectCharacteristics,
    ) -> i64 {
        let mana_value = i64::from(object.base_object().mana_value());
        let commander = if object.is_commander() { 1 } else { 0 };
        if let Some(creature) = characteristics.creature() {
            let keywords = i64::from(positive_keyword_count(creature.keywords()));
            weighted(
                i64::from(creature.power().max(0)),
                self.creature_power_delta,
            )
            .saturating_add(weighted(
                i64::from(creature.toughness().max(0)),
                self.creature_toughness_delta,
            ))
            .saturating_add(weighted(keywords, self.keyword_delta))
            .saturating_add(weighted(mana_value, self.noncreature_mana_value_delta))
            .saturating_add(weighted(commander, self.noncreature_permanent_delta))
        } else {
            self.noncreature_permanent_delta
                .saturating_add(weighted(mana_value, self.noncreature_mana_value_delta))
                .saturating_add(weighted(commander, self.noncreature_permanent_delta))
        }
    }

    /// Scores one visible player as a harmful-target priority.
    #[must_use]
    pub fn player_threat(self, player: PlayerState) -> i64 {
        let life_pressure = 40_i64.saturating_sub(player.life() as i64).max(0);
        weighted(life_pressure, self.life_delta)
            .saturating_add(weighted(i64::from(player.poison()), self.poison_delta))
    }

    /// Evaluates one redacted view from its observer's perspective.
    pub fn evaluate(self, view: &PlayerView) -> Result<Evaluation, EvaluationError> {
        let observer = view.observer();
        let observer_state = view
            .players()
            .get(observer.index())
            .ok_or(EvaluationError::ObserverMissing(observer))?;
        match view.game_outcome() {
            GameOutcome::Won(winner) if winner == observer => {
                return Ok(Evaluation::terminal(self.terminal_win));
            }
            GameOutcome::Won(_) => return Ok(Evaluation::terminal(self.terminal_loss)),
            GameOutcome::Draw => return Ok(Evaluation::terminal(0)),
            GameOutcome::InProgress => {}
        }

        let own = player_features(view, observer);
        let opponents = view
            .players()
            .iter()
            .copied()
            .filter(|player| player.id() != observer && !player.lost())
            .map(|player| player_features(view, player.id()))
            .collect::<Vec<_>>();
        let opponent = average_features(&opponents);
        let features = FeatureVector {
            life_delta: i64::from(observer_state.life()) - opponent.life,
            poison_delta: opponent.poison - i64::from(observer_state.poison()),
            hand_delta: own.hand - opponent.hand,
            creature_power_delta: own.creature_power - opponent.creature_power,
            creature_toughness_delta: own.creature_toughness - opponent.creature_toughness,
            keyword_delta: own.keywords - opponent.keywords,
            noncreature_permanent_delta: own.noncreature_permanents
                - opponent.noncreature_permanents,
            noncreature_mana_value_delta: own.noncreature_mana_value
                - opponent.noncreature_mana_value,
            pending_permanent_delta: own.pending_permanents - opponent.pending_permanents,
            pending_mana_value_delta: own.pending_mana_value - opponent.pending_mana_value,
            land_delta: own.lands - opponent.lands,
            untapped_land_delta: own.untapped_lands - opponent.untapped_lands,
            floating_mana_delta: own.floating_mana - opponent.floating_mana,
            tempo_delta: own.tempo - opponent.tempo,
        };
        let total = weighted(features.life_delta, self.life_delta)
            .saturating_add(weighted(features.poison_delta, self.poison_delta))
            .saturating_add(weighted(features.hand_delta, self.hand_delta))
            .saturating_add(weighted(
                features.creature_power_delta,
                self.creature_power_delta,
            ))
            .saturating_add(weighted(
                features.creature_toughness_delta,
                self.creature_toughness_delta,
            ))
            .saturating_add(weighted(features.keyword_delta, self.keyword_delta))
            .saturating_add(weighted(
                features.noncreature_permanent_delta,
                self.noncreature_permanent_delta,
            ))
            .saturating_add(weighted(
                features.noncreature_mana_value_delta,
                self.noncreature_mana_value_delta,
            ))
            .saturating_add(weighted(
                features.pending_permanent_delta,
                self.pending_permanent_delta,
            ))
            .saturating_add(weighted(
                features.pending_mana_value_delta,
                self.pending_mana_value_delta,
            ))
            .saturating_add(weighted(features.land_delta, self.land_delta))
            .saturating_add(weighted(
                features.untapped_land_delta,
                self.untapped_land_delta,
            ))
            .saturating_add(weighted(
                features.floating_mana_delta,
                self.floating_mana_delta,
            ))
            .saturating_add(weighted(features.tempo_delta, self.tempo_delta));
        Ok(Evaluation { features, total })
    }
}

const EXPECTED_FIELDS: [&str; 22] = [
    "terminal_win",
    "terminal_loss",
    "life_delta",
    "poison_delta",
    "hand_delta",
    "creature_power_delta",
    "creature_toughness_delta",
    "keyword_delta",
    "noncreature_permanent_delta",
    "noncreature_mana_value_delta",
    "pending_permanent_delta",
    "pending_mana_value_delta",
    "land_delta",
    "untapped_land_delta",
    "floating_mana_delta",
    "tempo_delta",
    "attack_power_prior",
    "attack_body_prior",
    "low_life_pressure_prior",
    "block_prevent_prior",
    "block_favorable_trade_prior",
    "block_losing_trade_prior",
];

/// Signed feature differences between the observer and live opponents.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureVector {
    /// Observer life minus average live-opponent life.
    pub life_delta: i64,
    /// Average live-opponent poison minus observer poison.
    pub poison_delta: i64,
    /// Observer hand size minus average live-opponent hand size.
    pub hand_delta: i64,
    /// Effective creature power difference.
    pub creature_power_delta: i64,
    /// Effective creature toughness difference.
    pub creature_toughness_delta: i64,
    /// Positive combat-keyword count difference.
    pub keyword_delta: i64,
    /// Noncreature, nonland permanent count difference.
    pub noncreature_permanent_delta: i64,
    /// Printed mana-value difference across noncreature permanents.
    pub noncreature_mana_value_delta: i64,
    /// Pending permanent-spell count difference.
    pub pending_permanent_delta: i64,
    /// Pending permanent-spell mana-value difference.
    pub pending_mana_value_delta: i64,
    /// Land count difference.
    pub land_delta: i64,
    /// Untapped land count difference.
    pub untapped_land_delta: i64,
    /// Floating mana-pool difference.
    pub floating_mana_delta: i64,
    /// Untapped-creature and turn-initiative difference.
    pub tempo_delta: i64,
}

/// One inspectable evaluation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Evaluation {
    features: FeatureVector,
    total: i64,
}

impl Evaluation {
    const fn terminal(total: i64) -> Self {
        Self {
            features: FeatureVector {
                life_delta: 0,
                poison_delta: 0,
                hand_delta: 0,
                creature_power_delta: 0,
                creature_toughness_delta: 0,
                keyword_delta: 0,
                noncreature_permanent_delta: 0,
                noncreature_mana_value_delta: 0,
                pending_permanent_delta: 0,
                pending_mana_value_delta: 0,
                land_delta: 0,
                untapped_land_delta: 0,
                floating_mana_delta: 0,
                tempo_delta: 0,
            },
            total,
        }
    }

    /// Returns the unweighted feature vector.
    #[must_use]
    pub const fn features(self) -> FeatureVector {
        self.features
    }

    /// Returns the weighted score from the observer's perspective.
    #[must_use]
    pub const fn total(self) -> i64 {
        self.total
    }
}

/// Fail-closed weight or evaluation errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    /// The weight document lacks outer RON parentheses.
    InvalidWeightsDocument,
    /// One entry is not a `name: integer` pair.
    InvalidWeightEntry(String),
    /// An unknown weight was supplied.
    UnknownWeight(String),
    /// A weight appeared more than once.
    DuplicateWeight(String),
    /// A required weight is absent.
    MissingWeight(&'static str),
    /// A weight value is not an integer.
    InvalidWeightValue(String),
    /// The view does not contain its observer seat.
    ObserverMissing(PlayerId),
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWeightsDocument => write!(formatter, "invalid ai_weights.ron document"),
            Self::InvalidWeightEntry(entry) => write!(formatter, "invalid weight entry `{entry}`"),
            Self::UnknownWeight(name) => write!(formatter, "unknown AI weight `{name}`"),
            Self::DuplicateWeight(name) => write!(formatter, "duplicate AI weight `{name}`"),
            Self::MissingWeight(name) => write!(formatter, "missing AI weight `{name}`"),
            Self::InvalidWeightValue(name) => {
                write!(formatter, "invalid integer for AI weight `{name}`")
            }
            Self::ObserverMissing(player) => {
                write!(
                    formatter,
                    "view is missing observer seat {}",
                    player.index() + 1
                )
            }
        }
    }
}

impl Error for EvaluationError {}

#[derive(Clone, Copy, Debug, Default)]
struct RawFeatures {
    life: i64,
    poison: i64,
    hand: i64,
    creature_power: i64,
    creature_toughness: i64,
    keywords: i64,
    noncreature_permanents: i64,
    noncreature_mana_value: i64,
    pending_permanents: i64,
    pending_mana_value: i64,
    lands: i64,
    untapped_lands: i64,
    floating_mana: i64,
    tempo: i64,
}

fn player_features(view: &PlayerView, player: PlayerId) -> RawFeatures {
    let scalar = view.players()[player.index()];
    let mut raw = RawFeatures {
        life: i64::from(scalar.life()),
        poison: i64::from(scalar.poison()),
        hand: view
            .zone(ZoneId::new(Some(player), ZoneKind::Hand))
            .map_or(0, |zone| zone.objects().len() as i64),
        floating_mana: i64::from(scalar.mana_pool().total()),
        ..RawFeatures::default()
    };
    if view.active_player() == Some(player) {
        raw.tempo += 1;
    }
    if view.priority_player() == Some(player) {
        raw.tempo += 1;
    }
    let Some(battlefield) = view.zone(ZoneId::new(None, ZoneKind::Battlefield)) else {
        return raw;
    };
    for object in battlefield.objects() {
        let ObjectView::Known {
            object,
            characteristics,
        } = object
        else {
            continue;
        };
        if characteristics.controller() != player {
            continue;
        }
        let types = characteristics.types();
        if types.land() {
            raw.lands += 1;
            if !object.tapped() {
                raw.untapped_lands += 1;
            }
        }
        if let Some(creature) = characteristics.creature() {
            raw.creature_power += i64::from(creature.power().max(0));
            raw.creature_toughness += i64::from(creature.toughness().max(0));
            raw.keywords += i64::from(positive_keyword_count(creature.keywords()));
            if !object.tapped() {
                raw.tempo += 1;
            }
        } else if !types.land() {
            raw.noncreature_permanents += 1;
            raw.noncreature_mana_value += i64::from(object.base_object().mana_value());
        }
    }
    if let Some(stack) = view.zone(ZoneId::new(None, ZoneKind::Stack)) {
        for object in stack.objects() {
            let ObjectView::Known {
                object,
                characteristics,
            } = object
            else {
                continue;
            };
            if characteristics.controller() != player {
                continue;
            }
            let types = characteristics.types();
            if !types.instant() && !types.sorcery() {
                raw.pending_permanents += 1;
                raw.pending_mana_value += i64::from(object.base_object().mana_value());
            }
        }
    }
    raw
}

fn average_features(values: &[RawFeatures]) -> RawFeatures {
    if values.is_empty() {
        return RawFeatures::default();
    }
    let mut sum = RawFeatures::default();
    for value in values {
        sum.life += value.life;
        sum.poison += value.poison;
        sum.hand += value.hand;
        sum.creature_power += value.creature_power;
        sum.creature_toughness += value.creature_toughness;
        sum.keywords += value.keywords;
        sum.noncreature_permanents += value.noncreature_permanents;
        sum.noncreature_mana_value += value.noncreature_mana_value;
        sum.pending_permanents += value.pending_permanents;
        sum.pending_mana_value += value.pending_mana_value;
        sum.lands += value.lands;
        sum.untapped_lands += value.untapped_lands;
        sum.floating_mana += value.floating_mana;
        sum.tempo += value.tempo;
    }
    let count = values.len() as i64;
    RawFeatures {
        life: sum.life / count,
        poison: sum.poison / count,
        hand: sum.hand / count,
        creature_power: sum.creature_power / count,
        creature_toughness: sum.creature_toughness / count,
        keywords: sum.keywords / count,
        noncreature_permanents: sum.noncreature_permanents / count,
        noncreature_mana_value: sum.noncreature_mana_value / count,
        pending_permanents: sum.pending_permanents / count,
        pending_mana_value: sum.pending_mana_value / count,
        lands: sum.lands / count,
        untapped_lands: sum.untapped_lands / count,
        floating_mana: sum.floating_mana / count,
        tempo: sum.tempo / count,
    }
}

fn positive_keyword_count(keywords: forge_core::CreatureKeywords) -> u32 {
    [
        keywords.first_strike(),
        keywords.double_strike(),
        keywords.trample(),
        keywords.deathtouch(),
        keywords.lifelink(),
        keywords.flying(),
        keywords.reach(),
        keywords.menace(),
        keywords.vigilance(),
        keywords.haste(),
        keywords.indestructible(),
        keywords.prowess(),
    ]
    .into_iter()
    .map(u32::from)
    .sum()
}

const fn weighted(feature: i64, weight: i64) -> i64 {
    feature.saturating_mul(weight)
}

#[cfg(test)]
mod tests {
    use super::AiWeights;
    use forge_core::{
        apply, Action, BaseCreatureCharacteristics, BaseObjectCharacteristics, CardId, GameState,
        ObjectTypes, Outcome, ZoneId, ZoneKind,
    };

    fn add_player(state: &mut GameState) -> forge_core::PlayerId {
        match apply(state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected AddPlayer outcome: {other:?}"),
        }
    }

    #[test]
    fn bundled_weights_parse_and_value_effective_board_material() {
        let weights =
            AiWeights::bundled().unwrap_or_else(|error| panic!("bundled weights failed: {error}"));
        let mut state = GameState::new();
        let observer = add_player(&mut state);
        let opponent = add_player(&mut state);
        assert_eq!(
            apply(
                &mut state,
                Action::SetPlayerLife {
                    player: observer,
                    life: 30,
                }
            ),
            Outcome::Applied
        );
        let creature = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(1),
                owner: observer,
                controller: observer,
                zone: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected creature outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: creature,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature(),
                        forge_core::ObjectColors::none(),
                    ),
                }
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics {
                    object: creature,
                    base: BaseCreatureCharacteristics::new(4, 4),
                }
            ),
            Outcome::Applied
        );
        let view = state
            .player_view(observer)
            .unwrap_or_else(|error| panic!("view failed: {error:?}"));
        let evaluation = weights
            .evaluate(&view)
            .unwrap_or_else(|error| panic!("evaluation failed: {error}"));
        assert!(evaluation.total() > 0);
        assert_eq!(evaluation.features().creature_power_delta, 4);
        assert_eq!(evaluation.features().creature_toughness_delta, 4);
        assert_eq!(view.players()[opponent.index()].life(), 20);
    }

    #[test]
    fn weights_fail_closed_on_unknown_fields() {
        let source = "(terminal_win: 1, surprise: 2,)";
        assert!(AiWeights::from_ron_str(source).is_err());
    }
}
