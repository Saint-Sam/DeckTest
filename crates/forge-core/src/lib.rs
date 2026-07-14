#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Pure rules-kernel crate for Forge 2.0.
//!
//! T1 starts with deterministic game-state storage. This crate intentionally
//! contains no card behavior yet; it provides the stable arenas, typed IDs,
//! zones, snapshots, invariants, and hashing that later rules systems build on.

use std::sync::Arc;

mod decision;

pub use decision::{
    CanonicalActionId, DecisionContext, DecisionContextError, DecisionContextId,
    DecisionDescriptor, DecisionGroup, DecisionKind, DecisionOption, DecisionStateKey,
    DECISION_CONTEXT_SCHEMA_VERSION,
};

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

/// A counter kind tracked by the T2.7 counter engine.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CounterKind {
    /// A +1/+1 counter.
    PlusOnePlusOne,
    /// A -1/-1 counter.
    MinusOneMinusOne,
    /// A loyalty counter.
    Loyalty,
    /// A deterministic compiler-assigned named counter.
    Named(u32),
}

/// Multiplayer range-of-influence policy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RangeOfInfluence {
    /// All players and objects are in range. This is the v1 Commander policy.
    Off,
}

impl RangeOfInfluence {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Off => 0,
        }
    }
}

impl CounterKind {
    /// Creates an arbitrary named counter from a deterministic ID.
    #[must_use]
    pub const fn named(id: u32) -> Self {
        Self::Named(id)
    }

    const fn canonical_code(self) -> u8 {
        match self {
            Self::PlusOnePlusOne => 0,
            Self::MinusOneMinusOne => 1,
            Self::Loyalty => 2,
            Self::Named(_) => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ObjectCounterEntry {
    object: ObjectId,
    kind: CounterKind,
    count: u32,
}

/// Maximum number of payment plans returned by the T1.4 enumerator.
pub const PAYMENT_PLAN_LIMIT: usize = 64;

/// Maximum number of state-based-action loops before declaring nontermination.
pub const SBA_FIXPOINT_LIMIT: u32 = 64;

/// Number of cards drawn for a normal Magic opening hand and each London mulligan.
pub const OPENING_HAND_SIZE: u32 = 7;

const MANA_KIND_COUNT: usize = 6;
const COLORED_MANA_KINDS: [ManaKind; 5] = [
    ManaKind::White,
    ManaKind::Blue,
    ManaKind::Black,
    ManaKind::Red,
    ManaKind::Green,
];

/// A kind of mana that can exist in a player's mana pool.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManaKind {
    /// White mana.
    White,
    /// Blue mana.
    Blue,
    /// Black mana.
    Black,
    /// Red mana.
    Red,
    /// Green mana.
    Green,
    /// Colorless mana.
    Colorless,
}

impl ManaKind {
    const fn index(self) -> usize {
        match self {
            Self::White => 0,
            Self::Blue => 1,
            Self::Black => 2,
            Self::Red => 3,
            Self::Green => 4,
            Self::Colorless => 5,
        }
    }
}

/// Mana currently available or selected for payment.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ManaPool {
    amounts: [u32; MANA_KIND_COUNT],
}

impl ManaPool {
    /// Creates an empty mana pool.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            amounts: [0; MANA_KIND_COUNT],
        }
    }

    /// Creates a mana pool from WUBRG and colorless amounts.
    #[must_use]
    pub const fn new(
        white: u32,
        blue: u32,
        black: u32,
        red: u32,
        green: u32,
        colorless: u32,
    ) -> Self {
        Self {
            amounts: [white, blue, black, red, green, colorless],
        }
    }

    /// Creates a pool containing one kind of mana.
    #[must_use]
    pub fn of(kind: ManaKind, amount: u32) -> Self {
        let mut pool = Self::empty();
        pool.amounts[kind.index()] = amount;
        pool
    }

    /// Returns the amount of one kind of mana in this pool.
    #[must_use]
    pub const fn get(self, kind: ManaKind) -> u32 {
        self.amounts[kind.index()]
    }

    /// Returns the total mana in this pool.
    #[must_use]
    pub fn total(self) -> u32 {
        self.amounts.iter().copied().sum()
    }

    /// Returns the total colored mana in this pool.
    #[must_use]
    pub fn colored_total(self) -> u32 {
        COLORED_MANA_KINDS.iter().map(|kind| self.get(*kind)).sum()
    }

    /// Returns true when this pool has at least every amount in `required`.
    #[must_use]
    pub fn contains_at_least(self, required: Self) -> bool {
        self.amounts
            .iter()
            .zip(required.amounts.iter())
            .all(|(available, needed)| available >= needed)
    }

    /// Pays a validated payment plan from this pool.
    pub fn pay(self, plan: PaymentPlan) -> Result<Self, PaymentError> {
        self.checked_sub(plan.paid)
            .ok_or(PaymentError::InsufficientMana)
    }

    /// Returns the component-wise sum, or none if any mana component overflows.
    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        let mut amounts = [0_u32; MANA_KIND_COUNT];
        for (index, amount) in amounts.iter_mut().enumerate() {
            *amount = self.amounts[index].checked_add(other.amounts[index])?;
        }
        Some(Self { amounts })
    }

    fn checked_sub(self, other: Self) -> Option<Self> {
        let mut amounts = [0_u32; MANA_KIND_COUNT];
        for (index, amount) in amounts.iter_mut().enumerate() {
            *amount = self.amounts[index].checked_sub(other.amounts[index])?;
        }
        Some(Self { amounts })
    }

    const fn canonical_key(self) -> [u32; MANA_KIND_COUNT] {
        self.amounts
    }
}

/// A resolved mana cost: colored requirements plus generic and optional X.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ManaCost {
    colored: [u32; 5],
    generic: u32,
    x_count: u32,
    x_value: u32,
}

impl ManaCost {
    /// Creates a mana cost from WUBRG colored pips and a generic amount.
    #[must_use]
    pub const fn new(
        white: u32,
        blue: u32,
        black: u32,
        red: u32,
        green: u32,
        generic: u32,
    ) -> Self {
        Self {
            colored: [white, blue, black, red, green],
            generic,
            x_count: 0,
            x_value: 0,
        }
    }

    /// Returns this cost with `x_count` X symbols set to the chosen value.
    #[must_use]
    pub const fn with_x(mut self, x_count: u32, x_value: u32) -> Self {
        self.x_count = x_count;
        self.x_value = x_value;
        self
    }

    /// Returns the colored pips of one mana kind.
    #[must_use]
    pub const fn colored(self, kind: ManaKind) -> u32 {
        match kind {
            ManaKind::White => self.colored[0],
            ManaKind::Blue => self.colored[1],
            ManaKind::Black => self.colored[2],
            ManaKind::Red => self.colored[3],
            ManaKind::Green => self.colored[4],
            ManaKind::Colorless => 0,
        }
    }

    /// Returns the printed generic component before X is added.
    #[must_use]
    pub const fn base_generic(self) -> u32 {
        self.generic
    }

    /// Returns how many X symbols this cost contains.
    #[must_use]
    pub const fn x_count(self) -> u32 {
        self.x_count
    }

    /// Returns the chosen value of X.
    #[must_use]
    pub const fn x_value(self) -> u32 {
        self.x_value
    }

    /// Returns colored requirements as a mana pool.
    #[must_use]
    pub const fn colored_pool(self) -> ManaPool {
        ManaPool::new(
            self.colored[0],
            self.colored[1],
            self.colored[2],
            self.colored[3],
            self.colored[4],
            0,
        )
    }

    /// Returns the total generic amount after adding X.
    pub fn generic_total(self) -> Result<u32, PaymentError> {
        let x_total = self
            .x_count
            .checked_mul(self.x_value)
            .ok_or(PaymentError::ManaValueOverflow)?;
        self.generic
            .checked_add(x_total)
            .ok_or(PaymentError::ManaValueOverflow)
    }

    /// Returns the component-wise sum of two costs, or `None` on overflow.
    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            colored: [
                self.colored[0].checked_add(other.colored[0])?,
                self.colored[1].checked_add(other.colored[1])?,
                self.colored[2].checked_add(other.colored[2])?,
                self.colored[3].checked_add(other.colored[3])?,
                self.colored[4].checked_add(other.colored[4])?,
            ],
            generic: self.generic.checked_add(other.generic)?,
            x_count: self.x_count.checked_add(other.x_count)?,
            x_value: self.x_value.checked_add(other.x_value)?,
        })
    }
}

/// A validated choice of mana used to pay a cost.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PaymentPlan {
    paid: ManaPool,
    generic_paid: ManaPool,
    generic_required: u32,
    x_value: u32,
    waste_score: u32,
}

impl PaymentPlan {
    /// Returns all mana consumed by this plan.
    #[must_use]
    pub const fn paid(self) -> ManaPool {
        self.paid
    }

    /// Returns the part of the payment assigned to generic or X costs.
    #[must_use]
    pub const fn generic_paid(self) -> ManaPool {
        self.generic_paid
    }

    /// Returns the generic amount, including X, that this plan pays.
    #[must_use]
    pub const fn generic_required(self) -> u32 {
        self.generic_required
    }

    /// Returns the chosen X value captured by this plan.
    #[must_use]
    pub const fn x_value(self) -> u32 {
        self.x_value
    }

    /// Returns the ordering score used by auto-payment.
    ///
    /// Lower is better. T1.4 defines waste as colored mana spent on generic or
    /// X costs when colorless mana could otherwise preserve colored resources.
    #[must_use]
    pub const fn waste_score(self) -> u32 {
        self.waste_score
    }
}

/// A bounded set of distinct payment plans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentEnumeration {
    plans: Vec<PaymentPlan>,
    truncated: bool,
}

impl PaymentEnumeration {
    /// Returns payment plans in deterministic auto-payment order.
    #[must_use]
    pub fn plans(&self) -> &[PaymentPlan] {
        &self.plans
    }

    /// Returns true when more than [`PAYMENT_PLAN_LIMIT`] plans exist.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the first and therefore preferred automatic payment plan.
    #[must_use]
    pub fn best(&self) -> Option<PaymentPlan> {
        self.plans.first().copied()
    }
}

/// One object that can be auto-tapped for mana in T1.4 planning.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ManaSource {
    object: ObjectId,
    produces: ManaPool,
}

impl ManaSource {
    /// Creates a mana source from an object and one deterministic output.
    #[must_use]
    pub const fn new(object: ObjectId, produces: ManaPool) -> Self {
        Self { object, produces }
    }

    /// Returns the object that would be tapped.
    #[must_use]
    pub const fn object(self) -> ObjectId {
        self.object
    }

    /// Returns the mana this source produces when tapped.
    #[must_use]
    pub const fn produces(self) -> ManaPool {
        self.produces
    }
}

/// One tap chosen by an auto-tap payment plan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ManaTap {
    source: ObjectId,
    produced: ManaPool,
}

impl ManaTap {
    /// Returns the tapped source object.
    #[must_use]
    pub const fn source(self) -> ObjectId {
        self.source
    }

    /// Returns the mana produced by this tap.
    #[must_use]
    pub const fn produced(self) -> ManaPool {
        self.produced
    }
}

/// A source-level auto-tap choice plus the resulting payment plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoTapPaymentPlan {
    taps: Vec<ManaTap>,
    produced: ManaPool,
    payment: PaymentPlan,
    unspent: ManaPool,
    total_waste_score: u32,
}

impl AutoTapPaymentPlan {
    /// Returns taps in deterministic source order.
    #[must_use]
    pub fn taps(&self) -> &[ManaTap] {
        &self.taps
    }

    /// Returns all mana produced by the taps.
    #[must_use]
    pub const fn produced(&self) -> ManaPool {
        self.produced
    }

    /// Returns the pool-level payment plan chosen from the produced mana.
    #[must_use]
    pub const fn payment(&self) -> PaymentPlan {
        self.payment
    }

    /// Returns mana that would remain floating after the payment.
    #[must_use]
    pub const fn unspent(&self) -> ManaPool {
        self.unspent
    }

    /// Returns the source-level ordering score used by auto-tap.
    #[must_use]
    pub const fn total_waste_score(&self) -> u32 {
        self.total_waste_score
    }
}

/// A bounded set of source-level auto-tap plans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoTapPaymentEnumeration {
    plans: Vec<AutoTapPaymentPlan>,
    truncated: bool,
}

impl AutoTapPaymentEnumeration {
    /// Returns auto-tap plans in deterministic preference order.
    #[must_use]
    pub fn plans(&self) -> &[AutoTapPaymentPlan] {
        &self.plans
    }

    /// Returns true when more than [`PAYMENT_PLAN_LIMIT`] plans exist.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the first and therefore preferred auto-tap plan.
    #[must_use]
    pub fn best(&self) -> Option<&AutoTapPaymentPlan> {
        self.plans.first()
    }
}

/// Errors raised while enumerating or applying mana payments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaymentError {
    /// A mana arithmetic operation overflowed `u32`.
    ManaValueOverflow,
    /// The available pool cannot cover the requested payment.
    InsufficientMana,
    /// The proposed explicit payment does not satisfy the cost.
    InvalidPaymentPlan,
}

/// Returns all distinct payment plans up to [`PAYMENT_PLAN_LIMIT`].
pub fn enumerate_payment_plans(
    available: ManaPool,
    cost: ManaCost,
) -> Result<PaymentEnumeration, PaymentError> {
    let colored_required = cost.colored_pool();
    if !available.contains_at_least(colored_required) {
        return Ok(PaymentEnumeration {
            plans: Vec::new(),
            truncated: false,
        });
    }
    let generic_required = cost.generic_total()?;
    let Some(remaining) = available.checked_sub(colored_required) else {
        return Ok(PaymentEnumeration {
            plans: Vec::new(),
            truncated: false,
        });
    };
    if generic_required > remaining.total() {
        return Ok(PaymentEnumeration {
            plans: Vec::new(),
            truncated: false,
        });
    }

    let max_colored_spend = generic_required.min(remaining.colored_total());
    let mut plans = Vec::new();
    let mut truncated = false;
    for colored_spend in 0..=max_colored_spend {
        let colorless_spend = generic_required - colored_spend;
        if colorless_spend > remaining.get(ManaKind::Colorless) {
            continue;
        }
        let mut colored_generic = [0_u32; 5];
        let mut search = PaymentSearch {
            remaining,
            colorless_spend,
            colored_required,
            generic_required,
            x_value: cost.x_value(),
            plans: &mut plans,
            truncated: &mut truncated,
        };
        let should_continue = enumerate_colored_payment_distributions(
            &mut search,
            colored_spend,
            0,
            &mut colored_generic,
        );
        if !should_continue {
            break;
        }
    }

    Ok(PaymentEnumeration { plans, truncated })
}

/// Returns the preferred automatic payment plan, if the cost can be paid.
pub fn auto_payment_plan(
    available: ManaPool,
    cost: ManaCost,
) -> Result<Option<PaymentPlan>, PaymentError> {
    Ok(enumerate_payment_plans(available, cost)?.best())
}

/// Enumerates source-level auto-tap plans up to [`PAYMENT_PLAN_LIMIT`].
pub fn enumerate_auto_tap_payment_plans(
    sources: &[ManaSource],
    cost: ManaCost,
) -> Result<AutoTapPaymentEnumeration, PaymentError> {
    let mut sorted_sources = sources.to_vec();
    sorted_sources.sort_by_key(|source| (source.object.0, source.produces.canonical_key()));
    let mut candidates = Vec::new();
    let mut child_truncated = false;
    let mut taps = Vec::new();
    let mut search = AutoTapSearch {
        sources: &sorted_sources,
        cost,
        candidates: &mut candidates,
        child_truncated: &mut child_truncated,
    };
    collect_auto_tap_candidates(&mut search, 0, ManaPool::empty(), &mut taps)?;

    candidates.sort_by(compare_auto_tap_plans);
    candidates.dedup();
    let mut truncated = child_truncated;
    if candidates.len() > PAYMENT_PLAN_LIMIT {
        candidates.truncate(PAYMENT_PLAN_LIMIT);
        truncated = true;
    }
    Ok(AutoTapPaymentEnumeration {
        plans: candidates,
        truncated,
    })
}

/// Returns the preferred source-level auto-tap plan, if the cost can be paid.
pub fn auto_tap_payment_plan(
    sources: &[ManaSource],
    cost: ManaCost,
) -> Result<Option<AutoTapPaymentPlan>, PaymentError> {
    Ok(enumerate_auto_tap_payment_plans(sources, cost)?
        .best()
        .cloned())
}

struct PaymentSearch<'a> {
    remaining: ManaPool,
    colorless_spend: u32,
    colored_required: ManaPool,
    generic_required: u32,
    x_value: u32,
    plans: &'a mut Vec<PaymentPlan>,
    truncated: &'a mut bool,
}

struct AutoTapSearch<'a> {
    sources: &'a [ManaSource],
    cost: ManaCost,
    candidates: &'a mut Vec<AutoTapPaymentPlan>,
    child_truncated: &'a mut bool,
}

struct CombatDamageProfile {
    legal_targets: Vec<CombatDamageTarget>,
    required_total: u32,
    trample_blockers: Vec<ObjectId>,
    trample_defender: CombatDamageTarget,
}

/// Validates an explicit pool selection against an available pool and cost.
pub fn validate_payment_plan(
    available: ManaPool,
    cost: ManaCost,
    paid: ManaPool,
) -> Result<PaymentPlan, PaymentError> {
    if !available.contains_at_least(paid) {
        return Err(PaymentError::InsufficientMana);
    }
    let colored_required = cost.colored_pool();
    let Some(generic_paid) = paid.checked_sub(colored_required) else {
        return Err(PaymentError::InvalidPaymentPlan);
    };
    let generic_required = cost.generic_total()?;
    if generic_paid.total() != generic_required {
        return Err(PaymentError::InvalidPaymentPlan);
    }
    Ok(PaymentPlan {
        paid,
        generic_paid,
        generic_required,
        x_value: cost.x_value(),
        waste_score: generic_paid.colored_total(),
    })
}

fn collect_auto_tap_candidates(
    search: &mut AutoTapSearch<'_>,
    source_index: usize,
    produced: ManaPool,
    taps: &mut Vec<ManaTap>,
) -> Result<(), PaymentError> {
    if source_index == search.sources.len() {
        let payment_plans = enumerate_payment_plans(produced, search.cost)?;
        *search.child_truncated |= payment_plans.truncated();
        for payment in payment_plans.plans() {
            let unspent = produced
                .checked_sub(payment.paid())
                .ok_or(PaymentError::InvalidPaymentPlan)?;
            let total_waste_score = payment
                .waste_score()
                .checked_add(unspent.total())
                .ok_or(PaymentError::ManaValueOverflow)?;
            search.candidates.push(AutoTapPaymentPlan {
                taps: taps.clone(),
                produced,
                payment: *payment,
                unspent,
                total_waste_score,
            });
        }
        return Ok(());
    }

    collect_auto_tap_candidates(search, source_index + 1, produced, taps)?;

    let source = search.sources[source_index];
    if let Some(next_produced) = produced.checked_add(source.produces) {
        taps.push(ManaTap {
            source: source.object,
            produced: source.produces,
        });
        collect_auto_tap_candidates(search, source_index + 1, next_produced, taps)?;
        taps.pop();
    } else {
        return Err(PaymentError::ManaValueOverflow);
    }
    Ok(())
}

fn compare_auto_tap_plans(
    left: &AutoTapPaymentPlan,
    right: &AutoTapPaymentPlan,
) -> std::cmp::Ordering {
    left.total_waste_score
        .cmp(&right.total_waste_score)
        .then(left.taps.len().cmp(&right.taps.len()))
        .then(left.payment.waste_score().cmp(&right.payment.waste_score()))
        .then(
            left.unspent
                .canonical_key()
                .cmp(&right.unspent.canonical_key()),
        )
        .then(
            left.payment
                .generic_paid()
                .canonical_key()
                .cmp(&right.payment.generic_paid().canonical_key()),
        )
        .then_with(|| compare_mana_taps(&left.taps, &right.taps))
}

fn compare_mana_taps(left: &[ManaTap], right: &[ManaTap]) -> std::cmp::Ordering {
    for (left_tap, right_tap) in left.iter().zip(right.iter()) {
        let ordering = left_tap.source.0.cmp(&right_tap.source.0).then(
            left_tap
                .produced
                .canonical_key()
                .cmp(&right_tap.produced.canonical_key()),
        );
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

fn enumerate_colored_payment_distributions(
    search: &mut PaymentSearch<'_>,
    target_colored: u32,
    color_index: usize,
    colored_generic: &mut [u32; 5],
) -> bool {
    if color_index == COLORED_MANA_KINDS.len() {
        if target_colored != 0 {
            return true;
        }
        if search.plans.len() == PAYMENT_PLAN_LIMIT {
            *search.truncated = true;
            return false;
        }
        let generic_paid = ManaPool::new(
            colored_generic[0],
            colored_generic[1],
            colored_generic[2],
            colored_generic[3],
            colored_generic[4],
            search.colorless_spend,
        );
        let Some(paid) = search.colored_required.checked_add(generic_paid) else {
            *search.truncated = true;
            return false;
        };
        search.plans.push(PaymentPlan {
            paid,
            generic_paid,
            generic_required: search.generic_required,
            x_value: search.x_value,
            waste_score: generic_paid.colored_total(),
        });
        return true;
    }

    let kind = COLORED_MANA_KINDS[color_index];
    let max_amount = search.remaining.get(kind).min(target_colored);
    for amount in 0..=max_amount {
        colored_generic[color_index] = amount;
        if !enumerate_colored_payment_distributions(
            search,
            target_colored - amount,
            color_index + 1,
            colored_generic,
        ) {
            return false;
        }
    }
    colored_generic[color_index] = 0;
    true
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
    /// Internal retention zone for tokens and copies that ceased to exist.
    Ceased,
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
            Self::Ceased => 7,
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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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

/// Provenance for a copied stack entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StackCopyInfo {
    source_entry: StackEntryId,
    source_object: Option<ObjectId>,
}

impl StackCopyInfo {
    /// Creates stack-copy provenance.
    #[must_use]
    pub const fn new(source_entry: StackEntryId, source_object: Option<ObjectId>) -> Self {
        Self {
            source_entry,
            source_object,
        }
    }

    /// Returns the stack entry that was copied.
    #[must_use]
    pub const fn source_entry(self) -> StackEntryId {
        self.source_entry
    }

    /// Returns the copied spell object, if the copied entry had one.
    #[must_use]
    pub const fn source_object(self) -> Option<ObjectId> {
        self.source_object
    }
}

/// A stable handle for one registered triggered ability definition.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TriggerId(u32);

impl TriggerId {
    /// Returns the zero-based trigger-definition index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic trigger value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A stable handle for one registered activated ability definition.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActivatedAbilityId(u32);

impl ActivatedAbilityId {
    /// Returns the zero-based activated-ability definition index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic activated-ability value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A stable handle for one registered activation cost modifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CostModifierId(u32);

impl CostModifierId {
    /// Returns the zero-based cost-modifier index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic cost-modifier value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A stable handle for one registered targeting or combat restriction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RestrictionId(u32);

impl RestrictionId {
    /// Returns the zero-based restriction-definition index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic restriction value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A stable handle for one registered replacement/prevention effect definition.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ReplacementEffectId(u32);

impl ReplacementEffectId {
    /// Returns the zero-based replacement-effect index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic replacement-effect value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A stable handle for one registered continuous effect definition.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContinuousEffectId(u32);

impl ContinuousEffectId {
    /// Returns the zero-based continuous-effect index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw deterministic continuous-effect value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Magic object colors represented by the layer engine.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ObjectColors {
    white: bool,
    blue: bool,
    black: bool,
    red: bool,
    green: bool,
}

impl ObjectColors {
    /// Creates an empty color set, which represents colorless.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            white: false,
            blue: false,
            black: false,
            red: false,
            green: false,
        }
    }

    /// Returns this set with white enabled.
    #[must_use]
    pub const fn with_white(mut self) -> Self {
        self.white = true;
        self
    }

    /// Returns this set with blue enabled.
    #[must_use]
    pub const fn with_blue(mut self) -> Self {
        self.blue = true;
        self
    }

    /// Returns this set with black enabled.
    #[must_use]
    pub const fn with_black(mut self) -> Self {
        self.black = true;
        self
    }

    /// Returns this set with red enabled.
    #[must_use]
    pub const fn with_red(mut self) -> Self {
        self.red = true;
        self
    }

    /// Returns this set with green enabled.
    #[must_use]
    pub const fn with_green(mut self) -> Self {
        self.green = true;
        self
    }

    /// Returns true if white is present.
    #[must_use]
    pub const fn white(self) -> bool {
        self.white
    }

    /// Returns true if blue is present.
    #[must_use]
    pub const fn blue(self) -> bool {
        self.blue
    }

    /// Returns true if black is present.
    #[must_use]
    pub const fn black(self) -> bool {
        self.black
    }

    /// Returns true if red is present.
    #[must_use]
    pub const fn red(self) -> bool {
        self.red
    }

    /// Returns true if green is present.
    #[must_use]
    pub const fn green(self) -> bool {
        self.green
    }

    /// Returns true when no colors are present.
    #[must_use]
    pub const fn colorless(self) -> bool {
        !self.white && !self.blue && !self.black && !self.red && !self.green
    }

    /// Returns true if every color in `required` is present in this set.
    #[must_use]
    pub const fn contains_all(self, required: Self) -> bool {
        (!required.white || self.white)
            && (!required.blue || self.blue)
            && (!required.black || self.black)
            && (!required.red || self.red)
            && (!required.green || self.green)
    }

    /// Returns true if this set and `other` share at least one color.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.white && other.white)
            || (self.blue && other.blue)
            || (self.black && other.black)
            || (self.red && other.red)
            || (self.green && other.green)
    }

    /// Returns the union of this color set and `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            white: self.white || other.white,
            blue: self.blue || other.blue,
            black: self.black || other.black,
            red: self.red || other.red,
            green: self.green || other.green,
        }
    }

    const fn canonical_bits(self) -> u8 {
        (self.white as u8)
            | ((self.blue as u8) << 1)
            | ((self.black as u8) << 2)
            | ((self.red as u8) << 3)
            | ((self.green as u8) << 4)
    }
}

/// Magic object card types represented by the layer engine.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ObjectTypes {
    artifact: bool,
    creature: bool,
    enchantment: bool,
    instant: bool,
    land: bool,
    planeswalker: bool,
    sorcery: bool,
}

/// Closed Magic supertypes represented by the runtime.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ObjectSupertypes {
    basic: bool,
    legendary: bool,
    ongoing: bool,
    snow: bool,
    world: bool,
}

impl ObjectSupertypes {
    /// Creates an empty supertype set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            basic: false,
            legendary: false,
            ongoing: false,
            snow: false,
            world: false,
        }
    }

    /// Returns this set with Basic enabled.
    #[must_use]
    pub const fn with_basic(mut self) -> Self {
        self.basic = true;
        self
    }

    /// Returns this set with Legendary enabled.
    #[must_use]
    pub const fn with_legendary(mut self) -> Self {
        self.legendary = true;
        self
    }

    /// Returns this set with Ongoing enabled.
    #[must_use]
    pub const fn with_ongoing(mut self) -> Self {
        self.ongoing = true;
        self
    }

    /// Returns this set with Snow enabled.
    #[must_use]
    pub const fn with_snow(mut self) -> Self {
        self.snow = true;
        self
    }

    /// Returns this set with World enabled.
    #[must_use]
    pub const fn with_world(mut self) -> Self {
        self.world = true;
        self
    }

    /// Returns true if Basic is present.
    #[must_use]
    pub const fn basic(self) -> bool {
        self.basic
    }

    /// Returns true if Legendary is present.
    #[must_use]
    pub const fn legendary(self) -> bool {
        self.legendary
    }

    /// Returns true if Ongoing is present.
    #[must_use]
    pub const fn ongoing(self) -> bool {
        self.ongoing
    }

    /// Returns true if Snow is present.
    #[must_use]
    pub const fn snow(self) -> bool {
        self.snow
    }

    /// Returns true if World is present.
    #[must_use]
    pub const fn world(self) -> bool {
        self.world
    }

    /// Returns true if every supertype in `required` is present.
    #[must_use]
    pub const fn contains_all(self, required: Self) -> bool {
        (!required.basic || self.basic)
            && (!required.legendary || self.legendary)
            && (!required.ongoing || self.ongoing)
            && (!required.snow || self.snow)
            && (!required.world || self.world)
    }

    /// Returns the union of this set and `add`.
    #[must_use]
    pub const fn union(mut self, add: Self) -> Self {
        self.basic |= add.basic;
        self.legendary |= add.legendary;
        self.ongoing |= add.ongoing;
        self.snow |= add.snow;
        self.world |= add.world;
        self
    }

    const fn canonical_bits(self) -> u8 {
        (self.basic as u8)
            | ((self.legendary as u8) << 1)
            | ((self.ongoing as u8) << 2)
            | ((self.snow as u8) << 3)
            | ((self.world as u8) << 4)
    }
}

/// The five basic land types represented by the runtime.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct BasicLandTypes {
    plains: bool,
    island: bool,
    swamp: bool,
    mountain: bool,
    forest: bool,
}

impl BasicLandTypes {
    /// Creates an empty basic-land-type set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            plains: false,
            island: false,
            swamp: false,
            mountain: false,
            forest: false,
        }
    }

    /// Returns this set with Plains enabled.
    #[must_use]
    pub const fn with_plains(mut self) -> Self {
        self.plains = true;
        self
    }

    /// Returns this set with Island enabled.
    #[must_use]
    pub const fn with_island(mut self) -> Self {
        self.island = true;
        self
    }

    /// Returns this set with Swamp enabled.
    #[must_use]
    pub const fn with_swamp(mut self) -> Self {
        self.swamp = true;
        self
    }

    /// Returns this set with Mountain enabled.
    #[must_use]
    pub const fn with_mountain(mut self) -> Self {
        self.mountain = true;
        self
    }

    /// Returns this set with Forest enabled.
    #[must_use]
    pub const fn with_forest(mut self) -> Self {
        self.forest = true;
        self
    }

    /// Returns true if Plains is present.
    #[must_use]
    pub const fn plains(self) -> bool {
        self.plains
    }

    /// Returns true if Island is present.
    #[must_use]
    pub const fn island(self) -> bool {
        self.island
    }

    /// Returns true if Swamp is present.
    #[must_use]
    pub const fn swamp(self) -> bool {
        self.swamp
    }

    /// Returns true if Mountain is present.
    #[must_use]
    pub const fn mountain(self) -> bool {
        self.mountain
    }

    /// Returns true if Forest is present.
    #[must_use]
    pub const fn forest(self) -> bool {
        self.forest
    }

    /// Returns true if every land type in `required` is present.
    #[must_use]
    pub const fn contains_all(self, required: Self) -> bool {
        (!required.plains || self.plains)
            && (!required.island || self.island)
            && (!required.swamp || self.swamp)
            && (!required.mountain || self.mountain)
            && (!required.forest || self.forest)
    }

    /// Returns true if this set and `other` share a land type.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.plains && other.plains)
            || (self.island && other.island)
            || (self.swamp && other.swamp)
            || (self.mountain && other.mountain)
            || (self.forest && other.forest)
    }

    /// Returns the union of this set and `add`.
    #[must_use]
    pub const fn union(mut self, add: Self) -> Self {
        self.plains |= add.plains;
        self.island |= add.island;
        self.swamp |= add.swamp;
        self.mountain |= add.mountain;
        self.forest |= add.forest;
        self
    }

    const fn canonical_bits(self) -> u8 {
        (self.plains as u8)
            | ((self.island as u8) << 1)
            | ((self.swamp as u8) << 2)
            | ((self.mountain as u8) << 3)
            | ((self.forest as u8) << 4)
    }
}

const MAX_OBJECT_SUBTYPE_BYTES: usize = 64;
const MAX_OBJECT_SUBTYPES: usize = 8;

/// One exact canonical card subtype stored without allocation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectSubtype {
    len: u8,
    bytes: [u8; MAX_OBJECT_SUBTYPE_BYTES],
}

impl Default for ObjectSubtype {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; MAX_OBJECT_SUBTYPE_BYTES],
        }
    }
}

impl ObjectSubtype {
    /// Parses a nonempty ASCII subtype and canonicalizes it case-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let source = value.as_bytes();
        if source.is_empty() || source.len() > MAX_OBJECT_SUBTYPE_BYTES || !value.is_ascii() {
            return None;
        }
        let mut bytes = [0_u8; MAX_OBJECT_SUBTYPE_BYTES];
        for (index, byte) in source.iter().copied().enumerate() {
            bytes[index] = byte.to_ascii_lowercase();
        }
        Some(Self {
            len: source.len() as u8,
            bytes,
        })
    }

    /// Returns the canonical ASCII bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

/// A bounded exact set of printed card subtypes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObjectSubtypes {
    len: u8,
    values: [ObjectSubtype; MAX_OBJECT_SUBTYPES],
}

impl Default for ObjectSubtypes {
    fn default() -> Self {
        Self::none()
    }
}

impl ObjectSubtypes {
    /// Creates an empty subtype set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            len: 0,
            values: [ObjectSubtype {
                len: 0,
                bytes: [0; MAX_OBJECT_SUBTYPE_BYTES],
            }; MAX_OBJECT_SUBTYPES],
        }
    }

    /// Inserts one subtype in canonical sorted order, or returns none if full.
    #[must_use]
    pub fn try_with(mut self, subtype: ObjectSubtype) -> Option<Self> {
        if self.contains(subtype) {
            return Some(self);
        }
        let len = usize::from(self.len);
        if len == MAX_OBJECT_SUBTYPES {
            return None;
        }
        let insertion = self.values[..len]
            .iter()
            .position(|existing| subtype < *existing)
            .unwrap_or(len);
        for index in (insertion..len).rev() {
            self.values[index + 1] = self.values[index];
        }
        self.values[insertion] = subtype;
        self.len += 1;
        Some(self)
    }

    /// Returns true when this set contains one subtype.
    #[must_use]
    pub fn contains(self, subtype: ObjectSubtype) -> bool {
        self.values[..usize::from(self.len)].contains(&subtype)
    }

    /// Returns true when every required subtype is present.
    #[must_use]
    pub fn contains_all(self, required: Self) -> bool {
        required
            .as_slice()
            .iter()
            .all(|value| self.contains(*value))
    }

    /// Returns the canonical sorted subtype slice.
    #[must_use]
    pub fn as_slice(&self) -> &[ObjectSubtype] {
        &self.values[..usize::from(self.len)]
    }
}

/// Printed type and color characteristics shared by every card object.
///
/// Power, toughness, and combat keywords remain in
/// [`BaseCreatureCharacteristics`]. Keeping these records separate lets the
/// runtime represent noncreature permanents without inventing creature data.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BaseObjectCharacteristics {
    types: ObjectTypes,
    colors: ObjectColors,
    supertypes: ObjectSupertypes,
    basic_land_types: BasicLandTypes,
    subtypes: ObjectSubtypes,
    mana_value: u32,
    printed_mana_symbols: ManaPool,
    enters_tapped: bool,
}

impl BaseObjectCharacteristics {
    /// Creates base printed object characteristics.
    #[must_use]
    pub const fn new(types: ObjectTypes, colors: ObjectColors) -> Self {
        Self {
            types,
            colors,
            supertypes: ObjectSupertypes::none(),
            basic_land_types: BasicLandTypes::none(),
            subtypes: ObjectSubtypes::none(),
            mana_value: 0,
            printed_mana_symbols: ManaPool::empty(),
            enters_tapped: false,
        }
    }

    /// Sets printed supertypes.
    #[must_use]
    pub const fn with_supertypes(mut self, supertypes: ObjectSupertypes) -> Self {
        self.supertypes = supertypes;
        self
    }

    /// Sets printed basic land types.
    #[must_use]
    pub const fn with_basic_land_types(mut self, land_types: BasicLandTypes) -> Self {
        self.basic_land_types = land_types;
        self
    }

    /// Sets exact printed subtypes.
    #[must_use]
    pub const fn with_subtypes(mut self, subtypes: ObjectSubtypes) -> Self {
        self.subtypes = subtypes;
        self
    }

    /// Sets the printed mana value represented by this object.
    #[must_use]
    pub const fn with_mana_value(mut self, mana_value: u32) -> Self {
        self.mana_value = mana_value;
        self
    }

    /// Sets W/U/B/R/G printed mana-symbol counts used by devotion.
    #[must_use]
    pub const fn with_printed_mana_symbols(mut self, symbols: ManaPool) -> Self {
        self.printed_mana_symbols = symbols;
        self
    }

    /// Marks this face as entering the battlefield tapped.
    #[must_use]
    pub const fn with_enters_tapped(mut self) -> Self {
        self.enters_tapped = true;
        self
    }

    /// Returns the printed card types.
    #[must_use]
    pub const fn types(self) -> ObjectTypes {
        self.types
    }

    /// Returns the printed colors.
    #[must_use]
    pub const fn colors(self) -> ObjectColors {
        self.colors
    }

    /// Returns printed supertypes.
    #[must_use]
    pub const fn supertypes(self) -> ObjectSupertypes {
        self.supertypes
    }

    /// Returns printed basic land types.
    #[must_use]
    pub const fn basic_land_types(self) -> BasicLandTypes {
        self.basic_land_types
    }

    /// Returns exact printed subtypes.
    #[must_use]
    pub const fn subtypes(self) -> ObjectSubtypes {
        self.subtypes
    }

    /// Returns the printed mana value represented by this object.
    #[must_use]
    pub const fn mana_value(self) -> u32 {
        self.mana_value
    }

    /// Returns W/U/B/R/G printed mana-symbol counts used by devotion.
    #[must_use]
    pub const fn printed_mana_symbols(self) -> ManaPool {
        self.printed_mana_symbols
    }

    /// Returns whether this face enters the battlefield tapped.
    #[must_use]
    pub const fn enters_tapped(self) -> bool {
        self.enters_tapped
    }
}

impl ObjectTypes {
    /// Creates an empty type set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            artifact: false,
            creature: false,
            enchantment: false,
            instant: false,
            land: false,
            planeswalker: false,
            sorcery: false,
        }
    }

    /// Returns this set with artifact enabled.
    #[must_use]
    pub const fn with_artifact(mut self) -> Self {
        self.artifact = true;
        self
    }

    /// Returns this set with creature enabled.
    #[must_use]
    pub const fn with_creature(mut self) -> Self {
        self.creature = true;
        self
    }

    /// Returns this set with enchantment enabled.
    #[must_use]
    pub const fn with_enchantment(mut self) -> Self {
        self.enchantment = true;
        self
    }

    /// Returns this set with instant enabled.
    #[must_use]
    pub const fn with_instant(mut self) -> Self {
        self.instant = true;
        self
    }

    /// Returns this set with land enabled.
    #[must_use]
    pub const fn with_land(mut self) -> Self {
        self.land = true;
        self
    }

    /// Returns this set with planeswalker enabled.
    #[must_use]
    pub const fn with_planeswalker(mut self) -> Self {
        self.planeswalker = true;
        self
    }

    /// Returns this set with sorcery enabled.
    #[must_use]
    pub const fn with_sorcery(mut self) -> Self {
        self.sorcery = true;
        self
    }

    /// Returns true if artifact is present.
    #[must_use]
    pub const fn artifact(self) -> bool {
        self.artifact
    }

    /// Returns true if creature is present.
    #[must_use]
    pub const fn creature(self) -> bool {
        self.creature
    }

    /// Returns true if enchantment is present.
    #[must_use]
    pub const fn enchantment(self) -> bool {
        self.enchantment
    }

    /// Returns true if instant is present.
    #[must_use]
    pub const fn instant(self) -> bool {
        self.instant
    }

    /// Returns true if land is present.
    #[must_use]
    pub const fn land(self) -> bool {
        self.land
    }

    /// Returns true if planeswalker is present.
    #[must_use]
    pub const fn planeswalker(self) -> bool {
        self.planeswalker
    }

    /// Returns true if sorcery is present.
    #[must_use]
    pub const fn sorcery(self) -> bool {
        self.sorcery
    }

    /// Returns true if every type in `required` is present in this set.
    #[must_use]
    pub const fn contains_all(self, required: Self) -> bool {
        (!required.artifact || self.artifact)
            && (!required.creature || self.creature)
            && (!required.enchantment || self.enchantment)
            && (!required.instant || self.instant)
            && (!required.land || self.land)
            && (!required.planeswalker || self.planeswalker)
            && (!required.sorcery || self.sorcery)
    }

    /// Returns true if this set and `other` share at least one type.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.artifact && other.artifact)
            || (self.creature && other.creature)
            || (self.enchantment && other.enchantment)
            || (self.instant && other.instant)
            || (self.land && other.land)
            || (self.planeswalker && other.planeswalker)
            || (self.sorcery && other.sorcery)
    }

    const fn without(mut self, remove: Self) -> Self {
        self.artifact &= !remove.artifact;
        self.creature &= !remove.creature;
        self.enchantment &= !remove.enchantment;
        self.instant &= !remove.instant;
        self.land &= !remove.land;
        self.planeswalker &= !remove.planeswalker;
        self.sorcery &= !remove.sorcery;
        self
    }

    /// Returns the union of this type set and `add`.
    #[must_use]
    pub const fn union(mut self, add: Self) -> Self {
        self.artifact |= add.artifact;
        self.creature |= add.creature;
        self.enchantment |= add.enchantment;
        self.instant |= add.instant;
        self.land |= add.land;
        self.planeswalker |= add.planeswalker;
        self.sorcery |= add.sorcery;
        self
    }

    const fn canonical_bits(self) -> u8 {
        (self.artifact as u8)
            | ((self.creature as u8) << 1)
            | ((self.enchantment as u8) << 2)
            | ((self.instant as u8) << 3)
            | ((self.land as u8) << 4)
            | ((self.planeswalker as u8) << 5)
            | ((self.sorcery as u8) << 6)
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

/// The coarse variant of a [`GameEvent`] used by trigger subscription tables.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GameEventKind {
    /// The deterministic seed changed.
    SeedSet,
    /// A player and that player's zones were added.
    PlayerAdded,
    /// The starting player was selected.
    TurnOrderDecided,
    /// Opening hands were drawn.
    OpeningHandsDrawn,
    /// A player took a mulligan.
    MulliganTaken,
    /// A player kept an opening hand.
    OpeningHandKept,
    /// One opening-hand card was bottomed.
    OpeningHandCardBottomed,
    /// A player's maximum hand size changed.
    PlayerMaxHandSizeSet,
    /// A player's life total was set directly.
    LifeTotalSet,
    /// A player lost life.
    LifeLost,
    /// A player gained life.
    LifeGained,
    /// Poison counters were added to a player.
    PoisonCountersAdded,
    /// A mana pool changed.
    ManaPoolChanged,
    /// Mana was paid.
    ManaPaid,
    /// An object was created.
    ObjectCreated,
    /// An object moved zones.
    ObjectMoved,
    /// A zone was shuffled.
    ZoneShuffled,
    /// Base creature characteristics were set.
    BaseCreatureCharacteristicsSet,
    /// Base creature characteristics were cleared.
    BaseCreatureCharacteristicsCleared,
    /// An object's tapped status changed.
    ObjectTapped,
    /// Damage was marked on an object.
    DamageMarked,
    /// A turn began.
    TurnStarted,
    /// A step ended.
    StepEnded,
    /// A step began.
    StepBegan,
    /// Priority was passed.
    PriorityPassed,
    /// The priority holder changed.
    PriorityChanged,
    /// A stack entry was added.
    StackEntryAdded,
    /// A stack entry resolved.
    StackEntryResolved,
    /// Attackers were declared.
    AttackersDeclared,
    /// One attacker was declared.
    AttackDeclared,
    /// Blockers were declared.
    BlockersDeclared,
    /// One blocker was declared.
    BlockDeclared,
    /// Combat damage was dealt.
    CombatDamageDealt,
    /// A player lost due to a state-based action.
    PlayerLostByStateBasedAction,
    /// A permanent moved due to a state-based action.
    PermanentMovedByStateBasedAction,
    /// The game outcome changed.
    GameOutcomeChanged,
    /// Cleanup priority was requested.
    CleanupPriorityRequested,
    /// A duration marker was added.
    DurationMarkerAdded,
    /// Duration markers expired.
    DurationMarkersExpired,
    /// Cleanup actions were performed.
    CleanupPerformed,
    /// Mana pools were cleared.
    ManaPoolsCleared,
    /// A player tried to draw from an empty library.
    EmptyLibraryDraw,
    /// A triggered ability definition was registered.
    TriggeredAbilityRegistered,
    /// A triggered ability instance was queued.
    TriggeredAbilityQueued,
    /// A queued triggered ability was put on the stack.
    TriggeredAbilityPutOnStack,
    /// A replacement/prevention effect definition was registered.
    ReplacementEffectRegistered,
    /// A player's deterministic replacement ordering preference changed.
    ReplacementChoiceOrderSet,
    /// A replacement/prevention effect modified an event.
    ReplacementEffectApplied,
    /// A continuous effect definition was registered.
    ContinuousEffectRegistered,
    /// An object's loyalty value changed.
    ObjectLoyaltySet,
    /// An activated ability definition was registered.
    ActivatedAbilityRegistered,
    /// An activation cost modifier was registered.
    CostModifierRegistered,
    /// An activated ability was activated.
    ActivatedAbilityActivated,
    /// An activated ability resolved.
    ActivatedAbilityResolved,
    /// A targeting or combat restriction definition was registered.
    RestrictionRegistered,
    /// An object's counter total changed.
    ObjectCountersChanged,
    /// A token object was created.
    TokenCreated,
    /// A permanent-copy object was created.
    ObjectCopyCreated,
    /// A stack entry was copied.
    StackEntryCopied,
    /// A token or copy ceased to exist.
    ObjectCeasedToExist,
    /// Matching +1/+1 and -1/-1 counters were removed.
    CounterPairCancelled,
    /// An object's Commander color identity metadata changed.
    ObjectColorIdentitySet,
    /// An object was designated as a commander.
    CommanderDesignated,
    /// A commander cast was recorded for tax tracking.
    CommanderCastRecorded,
    /// Commander color identity validation completed.
    CommanderColorIdentityValidated,
    /// A library was inspected and reordered by a keyword action.
    LibraryManipulated,
    /// An object's attachment pointer changed.
    ObjectAttached,
    /// Base printed card types and colors were set.
    BaseObjectCharacteristicsSet,
    /// Noncombat damage was dealt after replacement effects.
    NoncombatDamageDealt,
    /// A card was successfully drawn from a library.
    CardDrawn,
    /// One object's identity was publicly revealed.
    ObjectRevealed,
}

impl GameEventKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::SeedSet => 0,
            Self::PlayerAdded => 1,
            Self::TurnOrderDecided => 2,
            Self::OpeningHandsDrawn => 3,
            Self::MulliganTaken => 4,
            Self::OpeningHandKept => 5,
            Self::OpeningHandCardBottomed => 6,
            Self::PlayerMaxHandSizeSet => 7,
            Self::LifeTotalSet => 8,
            Self::LifeLost => 9,
            Self::LifeGained => 10,
            Self::PoisonCountersAdded => 11,
            Self::ManaPoolChanged => 12,
            Self::ManaPaid => 13,
            Self::ObjectCreated => 14,
            Self::ObjectMoved => 15,
            Self::ZoneShuffled => 16,
            Self::BaseCreatureCharacteristicsSet => 17,
            Self::BaseCreatureCharacteristicsCleared => 18,
            Self::ObjectTapped => 19,
            Self::DamageMarked => 20,
            Self::TurnStarted => 21,
            Self::StepEnded => 22,
            Self::StepBegan => 23,
            Self::PriorityPassed => 24,
            Self::PriorityChanged => 25,
            Self::StackEntryAdded => 26,
            Self::StackEntryResolved => 27,
            Self::AttackersDeclared => 28,
            Self::AttackDeclared => 29,
            Self::BlockersDeclared => 30,
            Self::BlockDeclared => 31,
            Self::CombatDamageDealt => 32,
            Self::PlayerLostByStateBasedAction => 33,
            Self::PermanentMovedByStateBasedAction => 34,
            Self::GameOutcomeChanged => 35,
            Self::CleanupPriorityRequested => 36,
            Self::DurationMarkerAdded => 37,
            Self::DurationMarkersExpired => 38,
            Self::CleanupPerformed => 39,
            Self::ManaPoolsCleared => 40,
            Self::EmptyLibraryDraw => 41,
            Self::TriggeredAbilityRegistered => 42,
            Self::TriggeredAbilityQueued => 43,
            Self::TriggeredAbilityPutOnStack => 44,
            Self::ReplacementEffectRegistered => 45,
            Self::ReplacementChoiceOrderSet => 46,
            Self::ReplacementEffectApplied => 47,
            Self::ContinuousEffectRegistered => 48,
            Self::ObjectLoyaltySet => 49,
            Self::ActivatedAbilityRegistered => 50,
            Self::CostModifierRegistered => 51,
            Self::ActivatedAbilityActivated => 52,
            Self::ActivatedAbilityResolved => 53,
            Self::RestrictionRegistered => 54,
            Self::ObjectCountersChanged => 55,
            Self::TokenCreated => 56,
            Self::ObjectCopyCreated => 57,
            Self::StackEntryCopied => 58,
            Self::ObjectCeasedToExist => 59,
            Self::CounterPairCancelled => 60,
            Self::ObjectColorIdentitySet => 61,
            Self::CommanderDesignated => 62,
            Self::CommanderCastRecorded => 63,
            Self::CommanderColorIdentityValidated => 64,
            Self::LibraryManipulated => 65,
            Self::ObjectAttached => 66,
            Self::BaseObjectCharacteristicsSet => 67,
            Self::NoncombatDamageDealt => 68,
            Self::CardDrawn => 69,
            Self::ObjectRevealed => 70,
        }
    }
}

/// Keyword-driven library manipulation operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LibraryManipulation {
    /// Scry moved selected cards to the library bottom.
    Scry,
    /// Surveil moved selected cards to the graveyard.
    Surveil,
    /// A searched-for card was placed on top of its owner's library.
    TutorToTop,
}

impl LibraryManipulation {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Scry => 0,
            Self::Surveil => 1,
            Self::TutorToTop => 2,
        }
    }
}

/// Timing permission used by the T1.5 casting pipeline.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SpellTiming {
    /// Castable whenever the player has priority.
    Instant,
    /// Castable only during the active player's main phase with an empty stack.
    Sorcery,
}

/// A target category understood by the T1.5 casting pipeline.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetKind {
    /// A player in the game.
    Player,
    /// An object currently on the battlefield.
    Permanent,
    /// An object currently in a specific zone.
    ObjectInZone(ZoneId),
    /// A spell or ability currently on the stack.
    StackEntry,
    /// An object in this zone kind, regardless of which player owns the zone.
    ObjectInZoneKind(ZoneKind),
    /// Either a player or an object currently on the battlefield.
    PlayerOrPermanent,
}

impl TargetKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Player => 0,
            Self::Permanent => 1,
            Self::ObjectInZone(_) => 2,
            Self::StackEntry => 3,
            Self::ObjectInZoneKind(_) => 4,
            Self::PlayerOrPermanent => 5,
        }
    }
}

/// Controller relationship used by target predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetControllerPredicate {
    /// Any controller is accepted.
    Any,
    /// The target must be controlled by the player choosing targets.
    You,
    /// The target must be controlled by a different player.
    Opponent,
    /// The target must be controlled by one exact player.
    Player(PlayerId),
}

impl TargetControllerPredicate {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::You => 1,
            Self::Opponent => 2,
            Self::Player(_) => 3,
        }
    }
}

/// Declarative predicate for object targets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObjectTargetPredicate {
    controller: TargetControllerPredicate,
    owner: TargetControllerPredicate,
    required_types: ObjectTypes,
    required_any_types: ObjectTypes,
    forbidden_types: ObjectTypes,
    required_colors: ObjectColors,
    forbidden_colors: ObjectColors,
    required_keywords: CreatureKeywords,
    forbidden_keywords: CreatureKeywords,
    required_subtypes: ObjectSubtypes,
    minimum_mana_value: Option<u32>,
    maximum_mana_value: Option<u32>,
}

impl ObjectTargetPredicate {
    /// Creates an object predicate that accepts any object matching its target kind.
    #[must_use]
    pub const fn any() -> Self {
        Self {
            controller: TargetControllerPredicate::Any,
            owner: TargetControllerPredicate::Any,
            required_types: ObjectTypes::none(),
            required_any_types: ObjectTypes::none(),
            forbidden_types: ObjectTypes::none(),
            required_colors: ObjectColors::none(),
            forbidden_colors: ObjectColors::none(),
            required_keywords: CreatureKeywords::none(),
            forbidden_keywords: CreatureKeywords::none(),
            required_subtypes: ObjectSubtypes::none(),
            minimum_mana_value: None,
            maximum_mana_value: None,
        }
    }

    /// Returns this predicate constrained to one controller relationship.
    #[must_use]
    pub const fn with_controller(mut self, controller: TargetControllerPredicate) -> Self {
        self.controller = controller;
        self
    }

    /// Returns this predicate constrained to one owner relationship.
    #[must_use]
    pub const fn with_owner(mut self, owner: TargetControllerPredicate) -> Self {
        self.owner = owner;
        self
    }

    /// Returns this predicate with additional required object types.
    #[must_use]
    pub const fn with_required_types(mut self, types: ObjectTypes) -> Self {
        self.required_types = types;
        self
    }

    /// Returns this predicate requiring at least one listed object type.
    #[must_use]
    pub const fn with_required_any_types(mut self, types: ObjectTypes) -> Self {
        self.required_any_types = types;
        self
    }

    /// Returns this predicate with forbidden object types.
    #[must_use]
    pub const fn with_forbidden_types(mut self, types: ObjectTypes) -> Self {
        self.forbidden_types = types;
        self
    }

    /// Returns this predicate with additional required object colors.
    #[must_use]
    pub const fn with_required_colors(mut self, colors: ObjectColors) -> Self {
        self.required_colors = colors;
        self
    }

    /// Returns this predicate with forbidden object colors.
    #[must_use]
    pub const fn with_forbidden_colors(mut self, colors: ObjectColors) -> Self {
        self.forbidden_colors = colors;
        self
    }

    /// Returns this predicate with required combat keywords.
    #[must_use]
    pub const fn with_required_keywords(mut self, keywords: CreatureKeywords) -> Self {
        self.required_keywords = keywords;
        self
    }

    /// Returns this predicate with forbidden combat keywords.
    #[must_use]
    pub const fn with_forbidden_keywords(mut self, keywords: CreatureKeywords) -> Self {
        self.forbidden_keywords = keywords;
        self
    }

    /// Returns this predicate requiring all listed exact subtypes.
    #[must_use]
    pub const fn with_required_subtypes(mut self, subtypes: ObjectSubtypes) -> Self {
        self.required_subtypes = subtypes;
        self
    }

    /// Returns this predicate with an inclusive minimum printed mana value.
    #[must_use]
    pub const fn with_minimum_mana_value(mut self, minimum: u32) -> Self {
        self.minimum_mana_value = Some(minimum);
        self
    }

    /// Returns this predicate with an inclusive maximum printed mana value.
    #[must_use]
    pub const fn with_maximum_mana_value(mut self, maximum: u32) -> Self {
        self.maximum_mana_value = Some(maximum);
        self
    }

    /// Returns the controller relationship constraint.
    #[must_use]
    pub const fn controller(self) -> TargetControllerPredicate {
        self.controller
    }

    /// Returns the owner relationship constraint.
    #[must_use]
    pub const fn owner(self) -> TargetControllerPredicate {
        self.owner
    }

    /// Returns types that must all be present.
    #[must_use]
    pub const fn required_types(self) -> ObjectTypes {
        self.required_types
    }

    /// Returns types of which at least one must be present.
    #[must_use]
    pub const fn required_any_types(self) -> ObjectTypes {
        self.required_any_types
    }

    /// Returns forbidden types.
    #[must_use]
    pub const fn forbidden_types(self) -> ObjectTypes {
        self.forbidden_types
    }

    /// Returns exact subtypes that must all be present.
    #[must_use]
    pub const fn required_subtypes(self) -> ObjectSubtypes {
        self.required_subtypes
    }

    /// Returns the inclusive minimum printed mana value, when constrained.
    #[must_use]
    pub const fn minimum_mana_value(self) -> Option<u32> {
        self.minimum_mana_value
    }

    /// Returns the inclusive maximum printed mana value, when constrained.
    #[must_use]
    pub const fn maximum_mana_value(self) -> Option<u32> {
        self.maximum_mana_value
    }
}

/// Declarative predicate for player targets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlayerTargetPredicate {
    /// Any player.
    Any,
    /// The targeting player.
    You,
    /// A player other than the targeting player.
    Opponent,
    /// One exact player.
    Player(PlayerId),
}

impl PlayerTargetPredicate {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::You => 1,
            Self::Opponent => 2,
            Self::Player(_) => 3,
        }
    }
}

/// Predicate attached to one target slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetPredicate {
    /// Accept any legal target of the required kind.
    Any,
    /// Apply an object-characteristics predicate.
    Object(ObjectTargetPredicate),
    /// Apply a player predicate.
    Player(PlayerTargetPredicate),
    /// Apply distinct predicates to the player and permanent branches of a union target.
    PlayerOrObject {
        /// Predicate for a selected player.
        player: PlayerTargetPredicate,
        /// Predicate for a selected permanent.
        object: ObjectTargetPredicate,
    },
}

impl TargetPredicate {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Object(_) => 1,
            Self::Player(_) => 2,
            Self::PlayerOrObject { .. } => 3,
        }
    }
}

/// One required target slot for a spell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TargetRequirement {
    kind: TargetKind,
    predicate: TargetPredicate,
}

impl TargetRequirement {
    /// Creates a target requirement.
    #[must_use]
    pub const fn new(kind: TargetKind) -> Self {
        Self {
            kind,
            predicate: TargetPredicate::Any,
        }
    }

    /// Returns the required target category.
    #[must_use]
    pub const fn kind(self) -> TargetKind {
        self.kind
    }

    /// Returns this requirement with an object predicate.
    #[must_use]
    pub const fn with_object_predicate(mut self, predicate: ObjectTargetPredicate) -> Self {
        self.predicate = TargetPredicate::Object(predicate);
        self
    }

    /// Returns this requirement with a player predicate.
    #[must_use]
    pub const fn with_player_predicate(mut self, predicate: PlayerTargetPredicate) -> Self {
        self.predicate = TargetPredicate::Player(predicate);
        self
    }

    /// Returns this union requirement with distinct player and permanent predicates.
    #[must_use]
    pub const fn with_player_or_object_predicate(
        mut self,
        player: PlayerTargetPredicate,
        object: ObjectTargetPredicate,
    ) -> Self {
        self.predicate = TargetPredicate::PlayerOrObject { player, object };
        self
    }

    /// Returns the predicate attached to this target slot.
    #[must_use]
    pub const fn predicate(self) -> TargetPredicate {
        self.predicate
    }
}

/// A selected spell target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetChoice {
    /// A targeted player.
    Player(PlayerId),
    /// A targeted game object.
    Object(ObjectId),
    /// A targeted spell or ability on the stack.
    StackEntry(StackEntryId),
}

impl TargetChoice {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Player(_) => 0,
            Self::Object(_) => 1,
            Self::StackEntry(_) => 2,
        }
    }
}

/// Legality snapshot captured as a spell is cast.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TargetSnapshot {
    requirement: TargetRequirement,
    choice: TargetChoice,
    original_zone: Option<ZoneId>,
    ward_cost: ManaCost,
}

impl TargetSnapshot {
    /// Returns the target requirement.
    #[must_use]
    pub const fn requirement(self) -> TargetRequirement {
        self.requirement
    }

    /// Returns the selected target.
    #[must_use]
    pub const fn choice(self) -> TargetChoice {
        self.choice
    }

    /// Returns the object zone captured when the target was selected.
    #[must_use]
    pub const fn original_zone(self) -> Option<ZoneId> {
        self.original_zone
    }

    /// Returns the ward cost observed when this target was chosen.
    #[must_use]
    pub const fn ward_cost(self) -> ManaCost {
        self.ward_cost
    }
}

/// Object selector used by declarative trigger predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerObjectFilter {
    /// Any object.
    Any,
    /// The registered trigger's source object.
    Source,
    /// One exact object.
    Object(ObjectId),
    /// The object currently attached to the registered trigger's source.
    AttachedToSource,
}

impl TriggerObjectFilter {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Source => 1,
            Self::Object(_) => 2,
            Self::AttachedToSource => 3,
        }
    }
}

/// Player selector used by declarative trigger predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerPlayerFilter {
    /// Any player.
    Any,
    /// The registered trigger's controller.
    Controller,
    /// Any opponent of the registered trigger's controller.
    OpponentOfController,
    /// One exact player.
    Player(PlayerId),
}

impl TriggerPlayerFilter {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Controller => 1,
            Self::OpponentOfController => 2,
            Self::Player(_) => 3,
        }
    }
}

/// Zone selector used by declarative trigger predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerZoneFilter {
    /// Any zone.
    Any,
    /// One exact zone.
    Exact(ZoneId),
    /// Any zone with this kind.
    Kind(ZoneKind),
    /// A zone of this kind belonging to a selected player.
    Owned {
        /// Player selector for the zone owner.
        owner: TriggerPlayerFilter,
        /// Required zone kind.
        kind: ZoneKind,
    },
}

impl TriggerZoneFilter {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Exact(_) => 1,
            Self::Kind(_) => 2,
            Self::Owned { .. } => 3,
        }
    }
}

/// Declarative event predicate for T2.2 triggered abilities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerCondition {
    /// Match any event with the given coarse kind.
    EventKind(GameEventKind),
    /// Match an object moving between selected zones.
    ObjectMoved {
        /// Object selector.
        object: TriggerObjectFilter,
        /// Source-zone selector.
        from: TriggerZoneFilter,
        /// Destination-zone selector.
        to: TriggerZoneFilter,
    },
    /// Match the beginning of one step.
    StepBegan {
        /// Step that must begin.
        step: Step,
    },
    /// Match life loss by a selected player.
    LifeLost {
        /// Player selector.
        player: TriggerPlayerFilter,
    },
    /// Match life gain by a selected player.
    LifeGained {
        /// Player selector.
        player: TriggerPlayerFilter,
    },
    /// Match damage marked on a selected object.
    DamageMarked {
        /// Object selector.
        object: TriggerObjectFilter,
    },
    /// Match stack resolution with optional kind/outcome filters.
    StackEntryResolved {
        /// Optional stack-object kind filter.
        kind: Option<StackObjectKind>,
        /// Optional resolution-outcome filter.
        outcome: Option<ResolutionOutcome>,
    },
    /// Match an attack declaration by a selected object.
    AttackDeclared {
        /// Attacking object selector.
        attacker: TriggerObjectFilter,
    },
    /// Match a step beginning for a selected active player.
    StepBeganFor {
        /// Step that must begin.
        step: Step,
        /// Required active player.
        player: TriggerPlayerFilter,
    },
    /// Match a newly cast stack object with closed type constraints.
    StackEntryAdded {
        /// Required stack-entry controller.
        controller: TriggerPlayerFilter,
        /// Types the stack object's source card must contain.
        required_types: ObjectTypes,
        /// Type union from which the source card must match at least one.
        required_any_types: ObjectTypes,
        /// Types the source card must not contain.
        forbidden_types: ObjectTypes,
    },
    /// Match positive combat damage from a matching permanent to a player.
    CombatDamageToPlayer {
        /// Source predicate interpreted relative to the trigger controller.
        source: ObjectTargetPredicate,
    },
    /// Match a successful card draw by a selected player.
    PlayerDrewCard {
        /// Player selector interpreted relative to the trigger controller.
        player: TriggerPlayerFilter,
    },
    /// Match a permanent entering the battlefield with a closed predicate.
    PermanentEnteredBattlefield {
        /// Entering permanent predicate interpreted relative to the trigger controller.
        predicate: ObjectTargetPredicate,
        /// Whether the registered trigger source itself is excluded.
        exclude_source: bool,
    },
    /// Match a newly cast spell or a copied stack object with closed type constraints.
    StackEntryAddedOrCopied {
        /// Required stack-entry controller.
        controller: TriggerPlayerFilter,
        /// Types the stack object's source card must contain.
        required_types: ObjectTypes,
        /// Type union from which the source card must match at least one.
        required_any_types: ObjectTypes,
        /// Types the source card must not contain.
        forbidden_types: ObjectTypes,
    },
}

impl TriggerCondition {
    /// Returns the event kind this condition subscribes to.
    #[must_use]
    pub const fn subscribed_event_kind(self) -> GameEventKind {
        match self {
            Self::EventKind(kind) => kind,
            Self::ObjectMoved { .. } => GameEventKind::ObjectMoved,
            Self::StepBegan { .. } => GameEventKind::StepBegan,
            Self::LifeLost { .. } => GameEventKind::LifeLost,
            Self::LifeGained { .. } => GameEventKind::LifeGained,
            Self::DamageMarked { .. } => GameEventKind::DamageMarked,
            Self::StackEntryResolved { .. } => GameEventKind::StackEntryResolved,
            Self::AttackDeclared { .. } => GameEventKind::AttackDeclared,
            Self::StepBeganFor { .. } => GameEventKind::StepBegan,
            Self::StackEntryAdded { .. } => GameEventKind::StackEntryAdded,
            Self::CombatDamageToPlayer { .. } => GameEventKind::CombatDamageDealt,
            Self::PlayerDrewCard { .. } => GameEventKind::CardDrawn,
            Self::PermanentEnteredBattlefield { .. } => GameEventKind::ObjectMoved,
            Self::StackEntryAddedOrCopied { .. } => GameEventKind::StackEntryAdded,
        }
    }

    const fn canonical_code(self) -> u8 {
        match self {
            Self::EventKind(_) => 0,
            Self::ObjectMoved { .. } => 1,
            Self::StepBegan { .. } => 2,
            Self::LifeLost { .. } => 3,
            Self::LifeGained { .. } => 4,
            Self::DamageMarked { .. } => 5,
            Self::StackEntryResolved { .. } => 6,
            Self::AttackDeclared { .. } => 7,
            Self::StepBeganFor { .. } => 8,
            Self::StackEntryAdded { .. } => 9,
            Self::CombatDamageToPlayer { .. } => 10,
            Self::PlayerDrewCard { .. } => 11,
            Self::PermanentEnteredBattlefield { .. } => 12,
            Self::StackEntryAddedOrCopied { .. } => 13,
        }
    }
}

/// Declarative intervening-if predicate checked when an event would trigger.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerInterveningIf {
    /// Always true.
    Always,
    /// The trigger source must currently be in the selected zone.
    SourceInZone(ZoneId),
    /// The trigger controller must still control the source.
    ControllerControlsSource,
    /// The trigger controller's life total must be at or below this value.
    ControllerLifeAtMost(i32),
}

impl TriggerInterveningIf {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Always => 0,
            Self::SourceInZone(_) => 1,
            Self::ControllerControlsSource => 2,
            Self::ControllerLifeAtMost(_) => 3,
        }
    }
}

/// Lifetime of a registered trigger subscription.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriggerDuration {
    /// The subscription remains until explicitly unsupported future removal.
    Persistent,
    /// The subscription is removed after the first matching event queues it.
    DelayedOnce,
}

impl TriggerDuration {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Persistent => 0,
            Self::DelayedOnce => 1,
        }
    }
}

/// Data-only triggered ability definition produced by card IR compilation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TriggerDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    condition: TriggerCondition,
    intervening_if: TriggerInterveningIf,
    duration: TriggerDuration,
}

impl TriggerDefinition {
    /// Creates a persistent triggered ability definition with no source object.
    #[must_use]
    pub const fn new(controller: PlayerId, condition: TriggerCondition) -> Self {
        Self {
            controller,
            source: None,
            condition,
            intervening_if: TriggerInterveningIf::Always,
            duration: TriggerDuration::Persistent,
        }
    }

    /// Sets the source object for source-relative predicates.
    #[must_use]
    pub const fn with_source(mut self, source: ObjectId) -> Self {
        self.source = Some(source);
        self
    }

    /// Sets the intervening-if predicate.
    #[must_use]
    pub const fn with_intervening_if(mut self, intervening_if: TriggerInterveningIf) -> Self {
        self.intervening_if = intervening_if;
        self
    }

    /// Marks this definition as a delayed trigger that queues only once.
    #[must_use]
    pub const fn delayed_once(mut self) -> Self {
        self.duration = TriggerDuration::DelayedOnce;
        self
    }

    /// Returns the trigger controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the optional trigger source object.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the event predicate.
    #[must_use]
    pub const fn condition(self) -> TriggerCondition {
        self.condition
    }

    /// Returns the intervening-if predicate.
    #[must_use]
    pub const fn intervening_if(self) -> TriggerInterveningIf {
        self.intervening_if
    }

    /// Returns the trigger duration.
    #[must_use]
    pub const fn duration(self) -> TriggerDuration {
        self.duration
    }
}

/// Damage source selector used by declarative replacement predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReplacementSourceFilter {
    /// Any source, including source-less test damage.
    Any,
    /// The registered replacement effect's source object.
    Source,
    /// One exact object.
    Object(ObjectId),
}

impl ReplacementSourceFilter {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Source => 1,
            Self::Object(_) => 2,
        }
    }
}

/// Damage target selector used by declarative replacement predicates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReplacementDamageTargetFilter {
    /// Any player or object.
    Any,
    /// One exact player.
    Player(PlayerId),
    /// One exact object.
    Object(ObjectId),
}

impl ReplacementDamageTargetFilter {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::Player(_) => 1,
            Self::Object(_) => 2,
        }
    }
}

/// Declarative event predicate for replacement/prevention effects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReplacementCondition {
    /// Match damage that would be dealt to a selected target.
    DamageWouldBeDealt {
        /// Damage source selector.
        source: ReplacementSourceFilter,
        /// Damage target selector.
        target: ReplacementDamageTargetFilter,
        /// Whether only combat damage matches.
        combat_only: bool,
    },
}

impl ReplacementCondition {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::DamageWouldBeDealt { .. } => 0,
        }
    }
}

/// Data-only replacement/prevention operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReplacementOperation {
    /// Prevent all matching damage.
    PreventAllDamage,
    /// Prevent up to the given amount of matching damage.
    PreventDamage(u32),
    /// Increase matching damage by the given amount.
    AddDamage(u32),
    /// Double matching damage.
    DoubleDamage,
    /// Set matching damage to the given amount.
    SetDamage(u32),
}

impl ReplacementOperation {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::PreventAllDamage => 0,
            Self::PreventDamage(_) => 1,
            Self::AddDamage(_) => 2,
            Self::DoubleDamage => 3,
            Self::SetDamage(_) => 4,
        }
    }
}

/// Lifetime of a registered replacement/prevention effect.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReplacementDuration {
    /// The effect remains active until explicitly unsupported future removal.
    Persistent,
    /// The effect is removed after it applies once.
    Once,
}

impl ReplacementDuration {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Persistent => 0,
            Self::Once => 1,
        }
    }
}

/// Data-only replacement/prevention definition produced by card IR compilation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReplacementDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    condition: ReplacementCondition,
    operation: ReplacementOperation,
    duration: ReplacementDuration,
    self_replacement: bool,
}

impl ReplacementDefinition {
    /// Creates a persistent replacement/prevention definition with no source object.
    #[must_use]
    pub const fn new(
        controller: PlayerId,
        condition: ReplacementCondition,
        operation: ReplacementOperation,
    ) -> Self {
        Self {
            controller,
            source: None,
            condition,
            operation,
            duration: ReplacementDuration::Persistent,
            self_replacement: false,
        }
    }

    /// Sets the source object for source-relative predicates.
    #[must_use]
    pub const fn with_source(mut self, source: ObjectId) -> Self {
        self.source = Some(source);
        self
    }

    /// Sets the replacement effect duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: ReplacementDuration) -> Self {
        self.duration = duration;
        self
    }

    /// Marks this effect as a self-replacement effect applied before normal choices.
    #[must_use]
    pub const fn with_self_replacement(mut self) -> Self {
        self.self_replacement = true;
        self
    }

    /// Returns the effect controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the optional effect source object.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the event predicate.
    #[must_use]
    pub const fn condition(self) -> ReplacementCondition {
        self.condition
    }

    /// Returns the effect operation.
    #[must_use]
    pub const fn operation(self) -> ReplacementOperation {
        self.operation
    }

    /// Returns the effect duration.
    #[must_use]
    pub const fn duration(self) -> ReplacementDuration {
        self.duration
    }

    /// Returns true when this is a self-replacement effect.
    #[must_use]
    pub const fn self_replacement(self) -> bool {
        self.self_replacement
    }
}

/// Stored deterministic ordering preference for replacement choices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementChoiceOrder {
    chooser: PlayerId,
    order: Vec<ReplacementEffectId>,
}

impl ReplacementChoiceOrder {
    /// Creates a replacement ordering preference for one chooser.
    #[must_use]
    pub fn new(chooser: PlayerId, order: Vec<ReplacementEffectId>) -> Self {
        Self { chooser, order }
    }

    /// Returns the player whose choices are represented.
    #[must_use]
    pub const fn chooser(&self) -> PlayerId {
        self.chooser
    }

    /// Returns effect IDs in preferred application order.
    #[must_use]
    pub fn order(&self) -> &[ReplacementEffectId] {
        &self.order
    }
}

/// Request object for casting one spell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CastSpellRequest {
    kind: StackObjectKind,
    timing: SpellTiming,
    cost: ManaCost,
    payment: PaymentPlan,
    flash: bool,
    kicker: Option<ManaCost>,
    flashback: Option<ManaCost>,
    split_second: bool,
    target_requirements: Vec<TargetRequirement>,
    target_choices: Vec<TargetChoice>,
}

impl CastSpellRequest {
    /// Creates a spell-casting request with no targets.
    #[must_use]
    pub fn new(
        kind: StackObjectKind,
        timing: SpellTiming,
        cost: ManaCost,
        payment: PaymentPlan,
    ) -> Self {
        Self {
            kind,
            timing,
            cost,
            payment,
            flash: false,
            kicker: None,
            flashback: None,
            split_second: false,
            target_requirements: Vec::new(),
            target_choices: Vec::new(),
        }
    }

    /// Adds target requirements and selected targets.
    #[must_use]
    pub fn with_targets(
        mut self,
        target_requirements: Vec<TargetRequirement>,
        target_choices: Vec<TargetChoice>,
    ) -> Self {
        self.target_requirements = target_requirements;
        self.target_choices = target_choices;
        self
    }

    /// Marks this spell as castable at instant timing.
    #[must_use]
    pub const fn with_flash(mut self) -> Self {
        self.flash = true;
        self
    }

    /// Adds an optional kicker cost to this cast request.
    #[must_use]
    pub const fn with_kicker(mut self, cost: ManaCost) -> Self {
        self.kicker = Some(cost);
        self
    }

    /// Marks this request as a flashback cast using an alternative cost.
    #[must_use]
    pub const fn with_flashback(mut self, cost: ManaCost) -> Self {
        self.flashback = Some(cost);
        self
    }

    /// Marks this spell as having split second while it remains on the stack.
    #[must_use]
    pub const fn with_split_second(mut self) -> Self {
        self.split_second = true;
        self
    }

    /// Returns the stack-object kind that will be created.
    #[must_use]
    pub const fn kind(&self) -> StackObjectKind {
        self.kind
    }

    /// Returns the timing permission used for this cast.
    #[must_use]
    pub const fn timing(&self) -> SpellTiming {
        self.timing
    }

    /// Returns the total mana cost to pay.
    #[must_use]
    pub const fn cost(&self) -> ManaCost {
        self.cost
    }

    /// Returns the explicit payment plan.
    #[must_use]
    pub const fn payment(&self) -> PaymentPlan {
        self.payment
    }

    /// Returns true when flash grants instant-speed timing.
    #[must_use]
    pub const fn flash(&self) -> bool {
        self.flash
    }

    /// Returns the optional kicker cost.
    #[must_use]
    pub const fn kicker(&self) -> Option<ManaCost> {
        self.kicker
    }

    /// Returns the optional flashback alternative cost.
    #[must_use]
    pub const fn flashback(&self) -> Option<ManaCost> {
        self.flashback
    }

    /// Returns true when this spell has split second.
    #[must_use]
    pub const fn split_second(&self) -> bool {
        self.split_second
    }

    /// Returns true when this spell was kicked.
    #[must_use]
    pub const fn kicked(&self) -> bool {
        self.kicker.is_some()
    }

    /// Returns target requirements.
    #[must_use]
    pub fn target_requirements(&self) -> &[TargetRequirement] {
        &self.target_requirements
    }

    /// Returns selected targets.
    #[must_use]
    pub fn target_choices(&self) -> &[TargetChoice] {
        &self.target_choices
    }
}

/// Which object(s) a targeting restriction protects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetRestrictionSubject {
    /// One exact object.
    Object(ObjectId),
    /// Every current object.
    AllObjects,
    /// The object currently attached to the restriction's source.
    AttachedToSource,
}

impl TargetRestrictionSubject {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Object(_) => 0,
            Self::AllObjects => 1,
            Self::AttachedToSource => 2,
        }
    }
}

/// Targeting restrictions checked by the T2.6 legality engine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TargetRestriction {
    /// This object cannot be targeted by spells or abilities.
    Shroud,
    /// This object cannot be targeted by opponents' spells or abilities.
    Hexproof,
    /// This object cannot be targeted by sources of any listed color.
    ProtectionFromColors {
        /// Protected colors.
        colors: ObjectColors,
    },
    /// This object has ward. Ward does not make the target illegal; the cost is captured.
    Ward {
        /// Ward cost observed when the target is selected.
        cost: ManaCost,
    },
}

impl TargetRestriction {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Shroud => 0,
            Self::Hexproof => 1,
            Self::ProtectionFromColors { .. } => 2,
            Self::Ward { .. } => 3,
        }
    }
}

/// Which object(s) a combat restriction affects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CombatRestrictionSubject {
    /// One exact object.
    Object(ObjectId),
    /// All objects controlled by one player.
    ControlledBy(PlayerId),
    /// Every current object.
    AllObjects,
}

impl CombatRestrictionSubject {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Object(_) => 0,
            Self::ControlledBy(_) => 1,
            Self::AllObjects => 2,
        }
    }
}

/// Combat declaration restrictions checked by the T2.6 legality engine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CombatRestriction {
    /// A matching creature cannot be declared as an attacker.
    CannotAttack,
    /// A matching creature cannot be declared as a blocker.
    CannotBlock,
    /// A matching attacker cannot be blocked at all.
    CannotBeBlocked,
}

impl CombatRestriction {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::CannotAttack => 0,
            Self::CannotBlock => 1,
            Self::CannotBeBlocked => 2,
        }
    }
}

/// Which player a continuous player-rule effect affects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlayerRuleSubject {
    /// One exact player.
    Player(PlayerId),
    /// The current controller of the source permanent.
    ControllerOfSource,
}

impl PlayerRuleSubject {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Player(_) => 0,
            Self::ControllerOfSource => 1,
        }
    }
}

/// Closed continuous changes to player-level game rules.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlayerRule {
    /// The affected player does not discard for maximum hand size during cleanup.
    NoMaximumHandSize,
}

impl PlayerRule {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::NoMaximumHandSize => 0,
        }
    }
}

/// A targeting or combat restriction emitted by card IR.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RestrictionEffect {
    /// A restriction against choosing a target.
    Targeting {
        /// Object(s) protected by this targeting restriction.
        subject: TargetRestrictionSubject,
        /// Targeting restriction to apply.
        restriction: TargetRestriction,
    },
    /// A restriction against declaring attackers or blockers.
    Combat {
        /// Object(s) affected by this combat restriction.
        subject: CombatRestrictionSubject,
        /// Combat restriction to apply.
        restriction: CombatRestriction,
    },
    /// A source-bound continuous player-rule change.
    PlayerRule {
        /// Player affected by the rule change.
        subject: PlayerRuleSubject,
        /// Rule change to apply.
        rule: PlayerRule,
    },
    /// One exact permanent cannot be destroyed.
    Indestructible {
        /// Permanent protected from destruction.
        object: ObjectId,
    },
    /// One consumable regeneration shield on an exact permanent.
    RegenerationShield {
        /// Permanent protected by the shield.
        object: ObjectId,
    },
}

impl RestrictionEffect {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Targeting { .. } => 0,
            Self::Combat { .. } => 1,
            Self::PlayerRule { .. } => 2,
            Self::Indestructible { .. } => 3,
            Self::RegenerationShield { .. } => 4,
        }
    }
}

/// Data-only targeting/combat restriction definition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RestrictionDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    effect: RestrictionEffect,
    duration: ContinuousEffectDuration,
}

impl RestrictionDefinition {
    /// Creates a restriction definition with no source object.
    #[must_use]
    pub const fn new(controller: PlayerId, effect: RestrictionEffect) -> Self {
        Self {
            controller,
            source: None,
            effect,
            duration: ContinuousEffectDuration::Persistent,
        }
    }

    /// Sets the source object for this restriction.
    #[must_use]
    pub const fn with_source(mut self, source: ObjectId) -> Self {
        self.source = Some(source);
        self
    }

    /// Returns this restriction with an explicit duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: ContinuousEffectDuration) -> Self {
        self.duration = duration;
        self
    }

    /// Returns the restriction controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the optional source object.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the restriction effect.
    #[must_use]
    pub const fn effect(self) -> RestrictionEffect {
        self.effect
    }

    /// Returns the restriction duration.
    #[must_use]
    pub const fn duration(self) -> ContinuousEffectDuration {
        self.duration
    }
}

/// Timing permission for activating an ability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivationTiming {
    /// The ability may be activated whenever its controller has priority.
    Instant,
    /// The ability may be activated only during that player's main phase with an empty stack.
    Sorcery,
}

impl ActivationTiming {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Instant => 0,
            Self::Sorcery => 1,
        }
    }
}

/// Closed condition checked before an activated ability can be paid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivationCondition {
    /// The controller must control at least this many matching battlefield permanents.
    ControllerControlsAtLeast {
        /// Closed permanent predicate interpreted relative to the controller.
        predicate: ObjectTargetPredicate,
        /// Minimum matching permanent count.
        count: u32,
    },
}

impl ActivationCondition {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::ControllerControlsAtLeast { .. } => 0,
        }
    }
}

/// Player selector used by no-target activated ability effects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AbilityPlayer {
    /// The player activating or controlling the ability.
    Controller,
    /// One exact player.
    Player(PlayerId),
}

impl AbilityPlayer {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Controller => 0,
            Self::Player(_) => 1,
        }
    }
}

/// A data-only activated ability effect that can resolve without card-specific code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivatedAbilityEffect {
    /// Add mana to a selected player's mana pool.
    AddMana {
        /// Player receiving mana.
        player: AbilityPlayer,
        /// Mana to add.
        mana: ManaPool,
    },
    /// Gain life for a selected player.
    GainLife {
        /// Player gaining life.
        player: AbilityPlayer,
        /// Life amount to gain.
        amount: u32,
    },
    /// Lose life for a selected player.
    LoseLife {
        /// Player losing life.
        player: AbilityPlayer,
        /// Life amount to lose.
        amount: u32,
    },
    /// Add mana, then deal noncombat damage to the selected player.
    AddManaAndDealDamage {
        /// Player receiving mana and damage.
        player: AbilityPlayer,
        /// Mana to add.
        mana: ManaPool,
        /// Damage amount.
        amount: u32,
    },
}

impl ActivatedAbilityEffect {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::AddMana { .. } => 0,
            Self::GainLife { .. } => 1,
            Self::LoseLife { .. } => 2,
            Self::AddManaAndDealDamage { .. } => 3,
        }
    }

    /// Returns true if this is a mana-producing effect.
    #[must_use]
    pub const fn is_mana_effect(self) -> bool {
        matches!(
            self,
            Self::AddMana { .. } | Self::AddManaAndDealDamage { .. }
        )
    }
}

/// Costs paid to activate one ability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActivationCost {
    mana: ManaCost,
    tap_source: bool,
    sacrifice_source: bool,
    loyalty_delta: Option<i32>,
}

impl ActivationCost {
    /// Creates an activation cost with mana and no non-mana costs.
    #[must_use]
    pub const fn new(mana: ManaCost) -> Self {
        Self {
            mana,
            tap_source: false,
            sacrifice_source: false,
            loyalty_delta: None,
        }
    }

    /// Adds a tap-symbol source cost.
    #[must_use]
    pub const fn with_tap_source(mut self) -> Self {
        self.tap_source = true;
        self
    }

    /// Adds a source-sacrifice cost.
    #[must_use]
    pub const fn with_sacrifice_source(mut self) -> Self {
        self.sacrifice_source = true;
        self
    }

    /// Adds a loyalty cost or loyalty-increase cost.
    #[must_use]
    pub const fn with_loyalty_delta(mut self, delta: i32) -> Self {
        self.loyalty_delta = Some(delta);
        self
    }

    /// Returns the mana portion of the cost.
    #[must_use]
    pub const fn mana(self) -> ManaCost {
        self.mana
    }

    /// Returns whether the source must be tapped.
    #[must_use]
    pub const fn tap_source(self) -> bool {
        self.tap_source
    }

    /// Returns whether the source must be sacrificed.
    #[must_use]
    pub const fn sacrifice_source(self) -> bool {
        self.sacrifice_source
    }

    /// Returns the loyalty change paid as a cost, if any.
    #[must_use]
    pub const fn loyalty_delta(self) -> Option<i32> {
        self.loyalty_delta
    }
}

/// Declarative T2.5 activated ability definition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActivatedAbilityDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    timing: ActivationTiming,
    cost: ActivationCost,
    effect: ActivatedAbilityEffect,
    mana_ability: bool,
    condition: Option<ActivationCondition>,
}

impl ActivatedAbilityDefinition {
    /// Creates one activated ability definition.
    #[must_use]
    pub const fn new(
        controller: PlayerId,
        source: Option<ObjectId>,
        timing: ActivationTiming,
        cost: ActivationCost,
        effect: ActivatedAbilityEffect,
    ) -> Self {
        Self {
            controller,
            source,
            timing,
            cost,
            effect,
            mana_ability: false,
            condition: None,
        }
    }

    /// Marks this ability as a mana ability that resolves without using the stack.
    #[must_use]
    pub const fn as_mana_ability(mut self) -> Self {
        self.mana_ability = true;
        self
    }

    /// Returns this ability with one closed activation condition.
    #[must_use]
    pub const fn with_condition(mut self, condition: ActivationCondition) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Returns the ability controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the source object, if any.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the activation timing restriction.
    #[must_use]
    pub const fn timing(self) -> ActivationTiming {
        self.timing
    }

    /// Returns the base activation cost.
    #[must_use]
    pub const fn cost(self) -> ActivationCost {
        self.cost
    }

    /// Returns the effect to resolve.
    #[must_use]
    pub const fn effect(self) -> ActivatedAbilityEffect {
        self.effect
    }

    /// Returns true if this ability resolves without using the stack.
    #[must_use]
    pub const fn is_mana_ability(self) -> bool {
        self.mana_ability
    }

    /// Returns the activation condition, when present.
    #[must_use]
    pub const fn condition(self) -> Option<ActivationCondition> {
        self.condition
    }
}

/// Scope for a T2.5 activation cost modifier.
// Predicates remain inline so this public value type stays Copy and allocation-free.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CostModifierScope {
    /// Applies to every activated ability.
    AllActivatedAbilities,
    /// Applies to one registered ability.
    Ability(ActivatedAbilityId),
    /// Applies to abilities whose source matches this object.
    Source(ObjectId),
    /// Applies to abilities controlled by this player.
    Controller(PlayerId),
    /// Applies to spells whose object matches a closed predicate.
    Spells(ObjectTargetPredicate),
}

impl CostModifierScope {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::AllActivatedAbilities => 0,
            Self::Ability(_) => 1,
            Self::Source(_) => 2,
            Self::Controller(_) => 3,
            Self::Spells(_) => 4,
        }
    }
}

/// Mana-cost adjustment for activated abilities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CostModifierOperation {
    /// Add a complete mana cost as an additional cost.
    AddManaCost(ManaCost),
    /// Increase the generic portion.
    AddGeneric(u32),
    /// Reduce the generic portion, to a floor of zero.
    ReduceGeneric(u32),
}

impl CostModifierOperation {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::AddManaCost(_) => 0,
            Self::AddGeneric(_) => 1,
            Self::ReduceGeneric(_) => 2,
        }
    }
}

/// Registered cost adjustment for activated abilities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CostModifierDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    scope: CostModifierScope,
    operation: CostModifierOperation,
}

impl CostModifierDefinition {
    /// Creates one data-only activated ability cost modifier.
    #[must_use]
    pub const fn new(
        controller: PlayerId,
        source: Option<ObjectId>,
        scope: CostModifierScope,
        operation: CostModifierOperation,
    ) -> Self {
        Self {
            controller,
            source,
            scope,
            operation,
        }
    }

    /// Returns the modifier controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the source object, if any.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the matching scope.
    #[must_use]
    pub const fn scope(self) -> CostModifierScope {
        self.scope
    }

    /// Returns the cost operation.
    #[must_use]
    pub const fn operation(self) -> CostModifierOperation {
        self.operation
    }
}

/// Outcome recorded when a stack entry leaves the stack.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ResolutionOutcome {
    /// The entry resolved normally.
    Resolved,
    /// The entry had targets and all of them were illegal on resolution.
    CounteredOnResolution,
    /// The entry was countered by a resolving spell or ability.
    CounteredBySpell,
}

impl ResolutionOutcome {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Resolved => 0,
            Self::CounteredOnResolution => 1,
            Self::CounteredBySpell => 2,
        }
    }
}

/// One spell or ability waiting on the stack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackEntry {
    id: StackEntryId,
    controller: PlayerId,
    object: Option<ObjectId>,
    trigger: Option<TriggerId>,
    activated_ability: Option<ActivatedAbilityId>,
    kind: StackObjectKind,
    // clone_surface: target snapshots are Copy records bounded by target requirements.
    targets: Vec<TargetSnapshot>,
    payment: Option<PaymentPlan>,
    copy_info: Option<StackCopyInfo>,
    kicked: bool,
    flashback: bool,
    split_second: bool,
}

impl StackEntry {
    /// Returns the stable stack-entry ID.
    #[must_use]
    pub const fn id(&self) -> StackEntryId {
        self.id
    }

    /// Returns the controller of the spell or ability on the stack.
    #[must_use]
    pub const fn controller(&self) -> PlayerId {
        self.controller
    }

    /// Returns the physical object on the stack, if this entry is a spell.
    #[must_use]
    pub const fn object(&self) -> Option<ObjectId> {
        self.object
    }

    /// Returns the trigger definition that created this entry, if any.
    #[must_use]
    pub const fn trigger(&self) -> Option<TriggerId> {
        self.trigger
    }

    /// Returns the activated ability definition that created this entry, if any.
    #[must_use]
    pub const fn activated_ability(&self) -> Option<ActivatedAbilityId> {
        self.activated_ability
    }

    /// Returns the coarse stack-object kind.
    #[must_use]
    pub const fn kind(&self) -> StackObjectKind {
        self.kind
    }

    /// Returns target snapshots captured as this entry was put on the stack.
    #[must_use]
    pub fn targets(&self) -> &[TargetSnapshot] {
        &self.targets
    }

    /// Returns the payment plan used to cast this spell, if any.
    #[must_use]
    pub const fn payment(&self) -> Option<PaymentPlan> {
        self.payment
    }

    /// Returns provenance when this stack entry is a copy.
    #[must_use]
    pub const fn copy_info(&self) -> Option<StackCopyInfo> {
        self.copy_info
    }

    /// Returns true when this spell entry was cast with kicker.
    #[must_use]
    pub const fn kicked(&self) -> bool {
        self.kicked
    }

    /// Returns true when this spell entry was cast using flashback.
    #[must_use]
    pub const fn flashback(&self) -> bool {
        self.flashback
    }

    /// Returns true when this spell entry has split second.
    #[must_use]
    pub const fn split_second(&self) -> bool {
        self.split_second
    }
}

/// Record of a stack object that resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionRecord {
    stack_entry: StackEntryId,
    controller: PlayerId,
    object: Option<ObjectId>,
    trigger: Option<TriggerId>,
    activated_ability: Option<ActivatedAbilityId>,
    kind: StackObjectKind,
    // clone_surface: copied target snapshots are bounded by the resolving entry.
    targets: Vec<TargetSnapshot>,
    // clone_surface: one bool per target snapshot; paired with `targets`.
    legal_targets: Vec<bool>,
    outcome: ResolutionOutcome,
    copy_info: Option<StackCopyInfo>,
    kicked: bool,
    flashback: bool,
    split_second: bool,
}

impl ResolutionRecord {
    /// Returns the stack-entry ID that resolved.
    #[must_use]
    pub const fn stack_entry(&self) -> StackEntryId {
        self.stack_entry
    }

    /// Returns the controller of the resolved entry.
    #[must_use]
    pub const fn controller(&self) -> PlayerId {
        self.controller
    }

    /// Returns the physical object that resolved, if any.
    #[must_use]
    pub const fn object(&self) -> Option<ObjectId> {
        self.object
    }

    /// Returns the trigger definition that created this entry, if any.
    #[must_use]
    pub const fn trigger(&self) -> Option<TriggerId> {
        self.trigger
    }

    /// Returns the activated ability definition that resolved, if any.
    #[must_use]
    pub const fn activated_ability(&self) -> Option<ActivatedAbilityId> {
        self.activated_ability
    }

    /// Returns the resolved stack-object kind.
    #[must_use]
    pub const fn kind(&self) -> StackObjectKind {
        self.kind
    }

    /// Returns target snapshots captured for the entry.
    #[must_use]
    pub fn targets(&self) -> &[TargetSnapshot] {
        &self.targets
    }

    /// Returns whether each target was legal when the entry resolved.
    #[must_use]
    pub fn legal_targets(&self) -> &[bool] {
        &self.legal_targets
    }

    /// Returns whether the entry resolved or was countered by game rules.
    #[must_use]
    pub const fn outcome(&self) -> ResolutionOutcome {
        self.outcome
    }

    /// Returns provenance when the resolved stack entry was a copy.
    #[must_use]
    pub const fn copy_info(&self) -> Option<StackCopyInfo> {
        self.copy_info
    }

    /// Returns true when the resolved spell entry was cast with kicker.
    #[must_use]
    pub const fn kicked(&self) -> bool {
        self.kicked
    }

    /// Returns true when the resolved spell entry was cast using flashback.
    #[must_use]
    pub const fn flashback(&self) -> bool {
        self.flashback
    }

    /// Returns true when the resolved spell had split second.
    #[must_use]
    pub const fn split_second(&self) -> bool {
        self.split_second
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StackEntryRequest {
    controller: PlayerId,
    object: Option<ObjectId>,
    trigger: Option<TriggerId>,
    activated_ability: Option<ActivatedAbilityId>,
    kind: StackObjectKind,
    targets: Vec<TargetSnapshot>,
    payment: Option<PaymentPlan>,
    copy_info: Option<StackCopyInfo>,
    kicked: bool,
    flashback: bool,
    split_second: bool,
}

/// Combat-relevant static keywords tracked by the T1.6 kernel.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CreatureKeywords {
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

impl CreatureKeywords {
    /// Creates an empty keyword set.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            first_strike: false,
            double_strike: false,
            trample: false,
            deathtouch: false,
            lifelink: false,
            flying: false,
            reach: false,
            menace: false,
            vigilance: false,
            haste: false,
            defender: false,
            indestructible: false,
            prowess: false,
        }
    }

    /// Returns this set with first strike enabled.
    #[must_use]
    pub const fn with_first_strike(mut self) -> Self {
        self.first_strike = true;
        self
    }

    /// Returns this set with double strike enabled.
    #[must_use]
    pub const fn with_double_strike(mut self) -> Self {
        self.double_strike = true;
        self
    }

    /// Returns this set with trample enabled.
    #[must_use]
    pub const fn with_trample(mut self) -> Self {
        self.trample = true;
        self
    }

    /// Returns this set with deathtouch enabled.
    #[must_use]
    pub const fn with_deathtouch(mut self) -> Self {
        self.deathtouch = true;
        self
    }

    /// Returns this set with lifelink enabled.
    #[must_use]
    pub const fn with_lifelink(mut self) -> Self {
        self.lifelink = true;
        self
    }

    /// Returns this set with flying enabled.
    #[must_use]
    pub const fn with_flying(mut self) -> Self {
        self.flying = true;
        self
    }

    /// Returns this set with reach enabled.
    #[must_use]
    pub const fn with_reach(mut self) -> Self {
        self.reach = true;
        self
    }

    /// Returns this set with menace enabled.
    #[must_use]
    pub const fn with_menace(mut self) -> Self {
        self.menace = true;
        self
    }

    /// Returns this set with vigilance enabled.
    #[must_use]
    pub const fn with_vigilance(mut self) -> Self {
        self.vigilance = true;
        self
    }

    /// Returns this set with haste enabled.
    #[must_use]
    pub const fn with_haste(mut self) -> Self {
        self.haste = true;
        self
    }

    /// Returns this set with defender enabled.
    #[must_use]
    pub const fn with_defender(mut self) -> Self {
        self.defender = true;
        self
    }

    /// Returns this set with indestructible enabled.
    #[must_use]
    pub const fn with_indestructible(mut self) -> Self {
        self.indestructible = true;
        self
    }

    /// Returns this set with prowess enabled.
    #[must_use]
    pub const fn with_prowess(mut self) -> Self {
        self.prowess = true;
        self
    }

    /// Returns true if this set has first strike.
    #[must_use]
    pub const fn first_strike(self) -> bool {
        self.first_strike
    }

    /// Returns true if this set has double strike.
    #[must_use]
    pub const fn double_strike(self) -> bool {
        self.double_strike
    }

    /// Returns true if this set has trample.
    #[must_use]
    pub const fn trample(self) -> bool {
        self.trample
    }

    /// Returns true if this set has deathtouch.
    #[must_use]
    pub const fn deathtouch(self) -> bool {
        self.deathtouch
    }

    /// Returns true if this set has lifelink.
    #[must_use]
    pub const fn lifelink(self) -> bool {
        self.lifelink
    }

    /// Returns true if this set has flying.
    #[must_use]
    pub const fn flying(self) -> bool {
        self.flying
    }

    /// Returns true if this set has reach.
    #[must_use]
    pub const fn reach(self) -> bool {
        self.reach
    }

    /// Returns true if this set has menace.
    #[must_use]
    pub const fn menace(self) -> bool {
        self.menace
    }

    /// Returns true if this set has vigilance.
    #[must_use]
    pub const fn vigilance(self) -> bool {
        self.vigilance
    }

    /// Returns true if this set has haste.
    #[must_use]
    pub const fn haste(self) -> bool {
        self.haste
    }

    /// Returns true if this set has defender.
    #[must_use]
    pub const fn defender(self) -> bool {
        self.defender
    }

    /// Returns true if this set has indestructible.
    #[must_use]
    pub const fn indestructible(self) -> bool {
        self.indestructible
    }

    /// Returns true if this set has prowess.
    #[must_use]
    pub const fn prowess(self) -> bool {
        self.prowess
    }

    /// Returns true if every keyword in `required` is present in this set.
    #[must_use]
    pub const fn contains_all(self, required: Self) -> bool {
        (!required.first_strike || self.first_strike)
            && (!required.double_strike || self.double_strike)
            && (!required.trample || self.trample)
            && (!required.deathtouch || self.deathtouch)
            && (!required.lifelink || self.lifelink)
            && (!required.flying || self.flying)
            && (!required.reach || self.reach)
            && (!required.menace || self.menace)
            && (!required.vigilance || self.vigilance)
            && (!required.haste || self.haste)
            && (!required.defender || self.defender)
            && (!required.indestructible || self.indestructible)
            && (!required.prowess || self.prowess)
    }

    /// Returns true if this set and `other` share at least one keyword.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.first_strike && other.first_strike)
            || (self.double_strike && other.double_strike)
            || (self.trample && other.trample)
            || (self.deathtouch && other.deathtouch)
            || (self.lifelink && other.lifelink)
            || (self.flying && other.flying)
            || (self.reach && other.reach)
            || (self.menace && other.menace)
            || (self.vigilance && other.vigilance)
            || (self.haste && other.haste)
            || (self.defender && other.defender)
            || (self.indestructible && other.indestructible)
            || (self.prowess && other.prowess)
    }

    const fn canonical_bits(self) -> u16 {
        (self.first_strike as u16)
            | ((self.double_strike as u16) << 1)
            | ((self.trample as u16) << 2)
            | ((self.deathtouch as u16) << 3)
            | ((self.lifelink as u16) << 4)
            | ((self.flying as u16) << 5)
            | ((self.reach as u16) << 6)
            | ((self.menace as u16) << 7)
            | ((self.vigilance as u16) << 8)
            | ((self.haste as u16) << 9)
            | ((self.defender as u16) << 10)
            | ((self.indestructible as u16) << 11)
            | ((self.prowess as u16) << 12)
    }

    const fn without(mut self, remove: Self) -> Self {
        self.first_strike &= !remove.first_strike;
        self.double_strike &= !remove.double_strike;
        self.trample &= !remove.trample;
        self.deathtouch &= !remove.deathtouch;
        self.lifelink &= !remove.lifelink;
        self.flying &= !remove.flying;
        self.reach &= !remove.reach;
        self.menace &= !remove.menace;
        self.vigilance &= !remove.vigilance;
        self.haste &= !remove.haste;
        self.defender &= !remove.defender;
        self.indestructible &= !remove.indestructible;
        self.prowess &= !remove.prowess;
        self
    }

    const fn union(mut self, add: Self) -> Self {
        self.first_strike |= add.first_strike;
        self.double_strike |= add.double_strike;
        self.trample |= add.trample;
        self.deathtouch |= add.deathtouch;
        self.lifelink |= add.lifelink;
        self.flying |= add.flying;
        self.reach |= add.reach;
        self.menace |= add.menace;
        self.vigilance |= add.vigilance;
        self.haste |= add.haste;
        self.defender |= add.defender;
        self.indestructible |= add.indestructible;
        self.prowess |= add.prowess;
        self
    }
}

/// Derived power, toughness, and combat keywords for a creature object.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CreatureCharacteristics {
    power: i32,
    toughness: i32,
    keywords: CreatureKeywords,
}

impl CreatureCharacteristics {
    /// Creates creature characteristics with no keywords.
    #[must_use]
    pub const fn new(power: i32, toughness: i32) -> Self {
        Self {
            power,
            toughness,
            keywords: CreatureKeywords::none(),
        }
    }

    /// Returns this creature with the provided keyword set.
    #[must_use]
    pub const fn with_keywords(mut self, keywords: CreatureKeywords) -> Self {
        self.keywords = keywords;
        self
    }

    /// Returns this creature's power.
    #[must_use]
    pub const fn power(self) -> i32 {
        self.power
    }

    /// Returns this creature's toughness.
    #[must_use]
    pub const fn toughness(self) -> i32 {
        self.toughness
    }

    /// Returns this creature's combat keywords.
    #[must_use]
    pub const fn keywords(self) -> CreatureKeywords {
        self.keywords
    }
}

/// Base printed creature characteristics before continuous effects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BaseCreatureCharacteristics {
    power: i32,
    toughness: i32,
    keywords: CreatureKeywords,
}

impl BaseCreatureCharacteristics {
    /// Creates base printed creature characteristics with no keywords.
    #[must_use]
    pub const fn new(power: i32, toughness: i32) -> Self {
        Self {
            power,
            toughness,
            keywords: CreatureKeywords::none(),
        }
    }

    /// Returns this base characteristic set with the provided keyword set.
    #[must_use]
    pub const fn with_keywords(mut self, keywords: CreatureKeywords) -> Self {
        self.keywords = keywords;
        self
    }

    /// Returns the base printed power.
    #[must_use]
    pub const fn power(self) -> i32 {
        self.power
    }

    /// Returns the base printed toughness.
    #[must_use]
    pub const fn toughness(self) -> i32 {
        self.toughness
    }

    /// Returns the base printed combat keywords.
    #[must_use]
    pub const fn keywords(self) -> CreatureKeywords {
        self.keywords
    }

    const fn derived(self) -> CreatureCharacteristics {
        CreatureCharacteristics {
            power: self.power,
            toughness: self.toughness,
            keywords: self.keywords,
        }
    }
}

/// Effective object characteristics after CR 613 continuous effects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObjectCharacteristics {
    controller: PlayerId,
    colors: ObjectColors,
    types: ObjectTypes,
    supertypes: ObjectSupertypes,
    basic_land_types: BasicLandTypes,
    subtypes: ObjectSubtypes,
    creature: Option<CreatureCharacteristics>,
    text_marker: u32,
}

impl ObjectCharacteristics {
    /// Creates base characteristics from one stored object record.
    #[must_use]
    pub const fn new(
        controller: PlayerId,
        colors: ObjectColors,
        types: ObjectTypes,
        creature: Option<CreatureCharacteristics>,
    ) -> Self {
        Self {
            controller,
            colors,
            types,
            supertypes: ObjectSupertypes::none(),
            basic_land_types: BasicLandTypes::none(),
            subtypes: ObjectSubtypes::none(),
            creature,
            text_marker: 0,
        }
    }

    /// Returns the effective controller after layer 2.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the effective colors after layer 5.
    #[must_use]
    pub const fn colors(self) -> ObjectColors {
        self.colors
    }

    /// Returns the effective types after layer 4.
    #[must_use]
    pub const fn types(self) -> ObjectTypes {
        self.types
    }

    /// Returns effective supertypes after layer 4.
    #[must_use]
    pub const fn supertypes(self) -> ObjectSupertypes {
        self.supertypes
    }

    /// Returns effective basic land types after layer 4.
    #[must_use]
    pub const fn basic_land_types(self) -> BasicLandTypes {
        self.basic_land_types
    }

    /// Returns effective exact subtypes after type-line effects.
    #[must_use]
    pub const fn subtypes(self) -> ObjectSubtypes {
        self.subtypes
    }

    const fn with_type_line(
        mut self,
        supertypes: ObjectSupertypes,
        basic_land_types: BasicLandTypes,
        subtypes: ObjectSubtypes,
    ) -> Self {
        self.supertypes = supertypes;
        self.basic_land_types = basic_land_types;
        self.subtypes = subtypes;
        self
    }

    /// Returns effective creature characteristics, if this object is a creature.
    #[must_use]
    pub const fn creature(self) -> Option<CreatureCharacteristics> {
        self.creature
    }

    /// Returns the deterministic text-effect marker after layer 3.
    #[must_use]
    pub const fn text_marker(self) -> u32 {
        self.text_marker
    }

    /// Returns true if this object is currently a creature.
    #[must_use]
    pub const fn is_creature(self) -> bool {
        self.types.creature()
    }

    fn sync_creature_type(&mut self) {
        if self.types.creature() && self.creature.is_none() {
            self.creature = Some(CreatureCharacteristics::new(0, 0));
        }
        if !self.types.creature() {
            self.creature = None;
        }
    }
}

/// Which objects a continuous effect can affect.
// Predicates remain inline so this public value type stays Copy and allocation-free.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContinuousEffectTarget {
    /// Only the named object is affected.
    Object(ObjectId),
    /// Every object is affected.
    AllObjects,
    /// Every object matching a closed predicate, except one optional object.
    Objects {
        /// Predicate interpreted relative to the effect controller.
        predicate: ObjectTargetPredicate,
        /// Optional excluded object, normally the source of an "other" effect.
        excluded: Option<ObjectId>,
    },
    /// The object currently attached to the continuous effect's source.
    AttachedToSource,
}

impl ContinuousEffectTarget {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Object(_) => 0,
            Self::AllObjects => 1,
            Self::Objects { .. } => 2,
            Self::AttachedToSource => 3,
        }
    }
}

/// The CR 613 layer or sublayer for a continuous effect operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ContinuousEffectLayer {
    /// Layer 1: copy effects.
    Copy,
    /// Layer 2: control-changing effects.
    Control,
    /// Layer 3: text-changing effects.
    Text,
    /// Layer 4: type-changing effects.
    Type,
    /// Layer 5: color-changing effects.
    Color,
    /// Layer 6: ability-adding/removing effects.
    Ability,
    /// Layer 7a: characteristic-defining power/toughness effects.
    PowerToughnessCda,
    /// Layer 7b: power/toughness set effects.
    PowerToughnessSet,
    /// Layer 7c: power/toughness modify effects.
    PowerToughnessModify,
    /// Layer 7d: power/toughness switch effects.
    PowerToughnessSwitch,
}

impl ContinuousEffectLayer {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Copy => 1,
            Self::Control => 2,
            Self::Text => 3,
            Self::Type => 4,
            Self::Color => 5,
            Self::Ability => 6,
            Self::PowerToughnessCda => 70,
            Self::PowerToughnessSet => 71,
            Self::PowerToughnessModify => 72,
            Self::PowerToughnessSwitch => 73,
        }
    }
}

/// One data-only continuous-effect operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContinuousEffectOperation {
    /// Layer 1: copy base creature copiable values from another object.
    CopyBaseCreature {
        /// Source object whose base creature values are copied.
        from: ObjectId,
    },
    /// Layer 2: change effective controller.
    ChangeController {
        /// New effective controller.
        controller: PlayerId,
    },
    /// Layer 3: set a deterministic text marker for card-compiler tests.
    SetTextMarker {
        /// Deterministic marker standing in for text-changing output.
        marker: u32,
    },
    /// Layer 4: replace all represented object types.
    SetTypes {
        /// Replacement type set.
        types: ObjectTypes,
    },
    /// Layer 4: add represented object types.
    AddTypes {
        /// Types to add.
        types: ObjectTypes,
    },
    /// Layer 4: remove represented object types.
    RemoveTypes {
        /// Types to remove.
        types: ObjectTypes,
    },
    /// Layer 5: replace colors.
    SetColors {
        /// Replacement color set.
        colors: ObjectColors,
    },
    /// Layer 6: add combat keywords.
    AddKeywords {
        /// Keywords to add.
        keywords: CreatureKeywords,
    },
    /// Layer 6: remove combat keywords.
    RemoveKeywords {
        /// Keywords to remove.
        keywords: CreatureKeywords,
    },
    /// Layer 7a: set characteristic-defining power/toughness.
    SetBasePowerToughness {
        /// Characteristic-defining power.
        power: i32,
        /// Characteristic-defining toughness.
        toughness: i32,
    },
    /// Layer 7b: set power/toughness.
    SetPowerToughness {
        /// Set power.
        power: i32,
        /// Set toughness.
        toughness: i32,
    },
    /// Layer 7c: modify power/toughness.
    ModifyPowerToughness {
        /// Power delta.
        power: i32,
        /// Toughness delta.
        toughness: i32,
    },
    /// Layer 7d: switch power and toughness.
    SwitchPowerToughness,
}

impl ContinuousEffectOperation {
    const fn layer(self) -> ContinuousEffectLayer {
        match self {
            Self::CopyBaseCreature { .. } => ContinuousEffectLayer::Copy,
            Self::ChangeController { .. } => ContinuousEffectLayer::Control,
            Self::SetTextMarker { .. } => ContinuousEffectLayer::Text,
            Self::SetTypes { .. } | Self::AddTypes { .. } | Self::RemoveTypes { .. } => {
                ContinuousEffectLayer::Type
            }
            Self::SetColors { .. } => ContinuousEffectLayer::Color,
            Self::AddKeywords { .. } | Self::RemoveKeywords { .. } => {
                ContinuousEffectLayer::Ability
            }
            Self::SetBasePowerToughness { .. } => ContinuousEffectLayer::PowerToughnessCda,
            Self::SetPowerToughness { .. } => ContinuousEffectLayer::PowerToughnessSet,
            Self::ModifyPowerToughness { .. } => ContinuousEffectLayer::PowerToughnessModify,
            Self::SwitchPowerToughness => ContinuousEffectLayer::PowerToughnessSwitch,
        }
    }

    const fn canonical_code(self) -> u8 {
        match self {
            Self::CopyBaseCreature { .. } => 0,
            Self::ChangeController { .. } => 1,
            Self::SetTextMarker { .. } => 2,
            Self::SetTypes { .. } => 3,
            Self::AddTypes { .. } => 4,
            Self::RemoveTypes { .. } => 5,
            Self::SetColors { .. } => 6,
            Self::AddKeywords { .. } => 7,
            Self::RemoveKeywords { .. } => 8,
            Self::SetBasePowerToughness { .. } => 9,
            Self::SetPowerToughness { .. } => 10,
            Self::ModifyPowerToughness { .. } => 11,
            Self::SwitchPowerToughness => 12,
        }
    }
}

/// Duration for a continuous effect.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContinuousEffectDuration {
    /// The effect remains until explicitly absent from state.
    Persistent,
    /// The effect expires during the next cleanup step under CR 514.2.
    UntilEndOfTurn,
    /// The effect applies only while its source remains on the battlefield.
    WhileSourceOnBattlefield,
}

impl ContinuousEffectDuration {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Persistent => 0,
            Self::UntilEndOfTurn => 1,
            Self::WhileSourceOnBattlefield => 2,
        }
    }
}

/// Closed live predicate controlling whether a continuous effect applies.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContinuousEffectCondition {
    /// The continuous effect always applies while its duration remains active.
    Always,
    /// The effect applies while its controller's devotion to one color is below a threshold.
    ControllerDevotionLessThan {
        /// Colored mana symbol counted for devotion.
        color: ManaKind,
        /// Exclusive upper bound.
        threshold: u32,
    },
}

impl ContinuousEffectCondition {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Always => 0,
            Self::ControllerDevotionLessThan { .. } => 1,
        }
    }
}

/// Declarative continuous-effect definition consumed by the CR 613 layer engine.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ContinuousEffectDefinition {
    controller: PlayerId,
    source: Option<ObjectId>,
    target: ContinuousEffectTarget,
    operation: ContinuousEffectOperation,
    duration: ContinuousEffectDuration,
    condition: ContinuousEffectCondition,
    timestamp: u64,
    dependencies: Vec<ContinuousEffectId>,
}

impl ContinuousEffectDefinition {
    /// Creates a persistent continuous effect with no source or dependencies.
    #[must_use]
    pub fn new(
        controller: PlayerId,
        target: ContinuousEffectTarget,
        operation: ContinuousEffectOperation,
    ) -> Self {
        Self {
            controller,
            source: None,
            target,
            operation,
            duration: ContinuousEffectDuration::Persistent,
            condition: ContinuousEffectCondition::Always,
            timestamp: 0,
            dependencies: Vec::new(),
        }
    }

    /// Returns this definition with a source object.
    #[must_use]
    pub const fn with_source(mut self, source: ObjectId) -> Self {
        self.source = Some(source);
        self
    }

    /// Returns this definition with an explicit duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: ContinuousEffectDuration) -> Self {
        self.duration = duration;
        self
    }

    /// Returns this definition with a closed live applicability predicate.
    #[must_use]
    pub const fn with_condition(mut self, condition: ContinuousEffectCondition) -> Self {
        self.condition = condition;
        self
    }

    /// Returns this definition with an explicit CR timestamp.
    #[must_use]
    pub const fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Returns this definition with explicit CR 613.8 dependency edges.
    #[must_use]
    pub fn with_dependencies(mut self, dependencies: Vec<ContinuousEffectId>) -> Self {
        self.dependencies = dependencies;
        self
    }

    /// Returns the controller of the effect.
    #[must_use]
    pub const fn controller(&self) -> PlayerId {
        self.controller
    }

    /// Returns the source object, if any.
    #[must_use]
    pub const fn source(&self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the target filter.
    #[must_use]
    pub const fn target(&self) -> ContinuousEffectTarget {
        self.target
    }

    /// Returns the operation.
    #[must_use]
    pub const fn operation(&self) -> ContinuousEffectOperation {
        self.operation
    }

    /// Returns the duration.
    #[must_use]
    pub const fn duration(&self) -> ContinuousEffectDuration {
        self.duration
    }

    /// Returns the live applicability predicate.
    #[must_use]
    pub const fn condition(&self) -> ContinuousEffectCondition {
        self.condition
    }

    /// Returns the CR timestamp. Zero means registration order assigns it.
    #[must_use]
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Returns explicit CR 613.8 dependencies.
    #[must_use]
    pub fn dependencies(&self) -> &[ContinuousEffectId] {
        &self.dependencies
    }
}

/// Which combat damage step is currently being processed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CombatDamageStepKind {
    /// A single normal combat damage step.
    Normal,
    /// The first combat damage step created by first strike or double strike.
    FirstStrike,
    /// The second combat damage step after a first-strike step.
    Regular,
}

impl CombatDamageStepKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::FirstStrike => 1,
            Self::Regular => 2,
        }
    }
}

/// A player or creature that may receive combat damage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CombatDamageTarget {
    /// Damage assigned to a player.
    Player(PlayerId),
    /// Damage assigned to a creature object.
    Object(ObjectId),
}

impl CombatDamageTarget {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::Player(_) => 0,
            Self::Object(_) => 1,
        }
    }
}

/// One attacker chosen during the declare attackers step.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AttackDeclaration {
    attacker: ObjectId,
    defending_player: PlayerId,
}

impl AttackDeclaration {
    /// Creates an attack declaration.
    #[must_use]
    pub const fn new(attacker: ObjectId, defending_player: PlayerId) -> Self {
        Self {
            attacker,
            defending_player,
        }
    }

    /// Returns the attacking object.
    #[must_use]
    pub const fn attacker(self) -> ObjectId {
        self.attacker
    }

    /// Returns the player this attacker is attacking.
    #[must_use]
    pub const fn defending_player(self) -> PlayerId {
        self.defending_player
    }
}

/// One blocker chosen during the declare blockers step.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockDeclaration {
    blocker: ObjectId,
    attacker: ObjectId,
}

impl BlockDeclaration {
    /// Creates a block declaration.
    #[must_use]
    pub const fn new(blocker: ObjectId, attacker: ObjectId) -> Self {
        Self { blocker, attacker }
    }

    /// Returns the blocking object.
    #[must_use]
    pub const fn blocker(self) -> ObjectId {
        self.blocker
    }

    /// Returns the attacking object being blocked.
    #[must_use]
    pub const fn attacker(self) -> ObjectId {
        self.attacker
    }
}

/// One target and amount in a combat damage assignment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CombatDamageAssignment {
    target: CombatDamageTarget,
    amount: u32,
}

impl CombatDamageAssignment {
    /// Creates one combat damage assignment.
    #[must_use]
    pub const fn new(target: CombatDamageTarget, amount: u32) -> Self {
        Self { target, amount }
    }

    /// Returns the assigned target.
    #[must_use]
    pub const fn target(self) -> CombatDamageTarget {
        self.target
    }

    /// Returns the assigned damage amount.
    #[must_use]
    pub const fn amount(self) -> u32 {
        self.amount
    }
}

/// All combat damage assigned by one source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CombatDamageAssignmentRequest {
    source: ObjectId,
    assignments: Vec<CombatDamageAssignment>,
}

impl CombatDamageAssignmentRequest {
    /// Creates a request for one combat damage source.
    #[must_use]
    pub fn new(source: ObjectId, assignments: Vec<CombatDamageAssignment>) -> Self {
        Self {
            source,
            assignments,
        }
    }

    /// Returns the damage source.
    #[must_use]
    pub const fn source(&self) -> ObjectId {
        self.source
    }

    /// Returns target assignments for this source.
    #[must_use]
    pub fn assignments(&self) -> &[CombatDamageAssignment] {
        &self.assignments
    }
}

/// One combat damage event recorded after damage is dealt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CombatDamageRecord {
    source: ObjectId,
    target: CombatDamageTarget,
    amount: u32,
    step: CombatDamageStepKind,
    source_had_deathtouch: bool,
    source_had_lifelink: bool,
}

impl CombatDamageRecord {
    /// Returns the damage source.
    #[must_use]
    pub const fn source(self) -> ObjectId {
        self.source
    }

    /// Returns the damaged target.
    #[must_use]
    pub const fn target(self) -> CombatDamageTarget {
        self.target
    }

    /// Returns the dealt damage amount.
    #[must_use]
    pub const fn amount(self) -> u32 {
        self.amount
    }

    /// Returns which combat damage step dealt this damage.
    #[must_use]
    pub const fn step(self) -> CombatDamageStepKind {
        self.step
    }

    /// Returns whether the source had deathtouch as damage was dealt.
    #[must_use]
    pub const fn source_had_deathtouch(self) -> bool {
        self.source_had_deathtouch
    }

    /// Returns whether the source had lifelink as damage was dealt.
    #[must_use]
    pub const fn source_had_lifelink(self) -> bool {
        self.source_had_lifelink
    }
}

/// One attacking creature in the current combat.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackingCreature {
    object: ObjectId,
    defending_player: PlayerId,
    blocked: bool,
    // clone_surface: blocker IDs are bounded by current combat declarations.
    blockers: Vec<ObjectId>,
}

impl AttackingCreature {
    /// Returns the attacking object.
    #[must_use]
    pub const fn object(&self) -> ObjectId {
        self.object
    }

    /// Returns the attacked player.
    #[must_use]
    pub const fn defending_player(&self) -> PlayerId {
        self.defending_player
    }

    /// Returns true once this attacker has become blocked this combat.
    #[must_use]
    pub const fn blocked(&self) -> bool {
        self.blocked
    }

    /// Returns blockers declared for this attacker in declaration order.
    #[must_use]
    pub fn blockers(&self) -> &[ObjectId] {
        &self.blockers
    }
}

/// One blocking creature in the current combat.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockingCreature {
    object: ObjectId,
    attacker: ObjectId,
}

impl BlockingCreature {
    /// Returns the blocking object.
    #[must_use]
    pub const fn object(self) -> ObjectId {
        self.object
    }

    /// Returns the attacking creature this object blocks.
    #[must_use]
    pub const fn attacker(self) -> ObjectId {
        self.attacker
    }
}

/// Current combat state, cleared at the beginning and end of combat.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CombatState {
    // clone_surface: current-combat attacker records are cleared between combats.
    attackers: Vec<AttackingCreature>,
    // clone_surface: current-combat blocking records are cleared between combats.
    blockers: Vec<BlockingCreature>,
    // clone_surface: damage records are Copy records for the current combat step.
    damage_records: Vec<CombatDamageRecord>,
    damage_step: Option<CombatDamageStepKind>,
    // clone_surface: object IDs only, bounded by attackers/blockers in combat.
    first_strike_participants: Vec<ObjectId>,
}

impl CombatState {
    /// Creates an empty combat state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            attackers: Vec::new(),
            blockers: Vec::new(),
            damage_records: Vec::new(),
            damage_step: None,
            first_strike_participants: Vec::new(),
        }
    }

    /// Returns current attackers in declaration order.
    #[must_use]
    pub fn attackers(&self) -> &[AttackingCreature] {
        &self.attackers
    }

    /// Returns current blockers in declaration order.
    #[must_use]
    pub fn blockers(&self) -> &[BlockingCreature] {
        &self.blockers
    }

    /// Returns combat damage records in deal order.
    #[must_use]
    pub fn damage_records(&self) -> &[CombatDamageRecord] {
        &self.damage_records
    }

    /// Returns the current combat damage sub-step, if any.
    #[must_use]
    pub const fn damage_step(&self) -> Option<CombatDamageStepKind> {
        self.damage_step
    }
}

/// Current game result derived by state-based actions.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GameOutcome {
    /// The game has not ended.
    InProgress,
    /// Exactly one player remains in the game.
    Won(PlayerId),
    /// No player remains, or all remaining players lost simultaneously.
    Draw,
}

impl GameOutcome {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::InProgress => 0,
            Self::Won(_) => 1,
            Self::Draw => 2,
        }
    }
}

/// One CR 704.5 state-based-action row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StateBasedActionKind {
    /// CR 704.5a: a player with 0 or less life loses the game.
    PlayerZeroOrLessLife,
    /// CR 704.5b: a player who tried to draw from an empty library loses.
    PlayerDrewFromEmptyLibrary,
    /// CR 704.5c: a player with ten or more poison counters loses.
    PlayerTenOrMorePoison,
    /// CR 704.5d: a token outside the battlefield ceases to exist.
    TokenOffBattlefield,
    /// CR 704.5e: a copy in an illegal zone ceases to exist.
    CopyOutOfAllowedZone,
    /// CR 704.5f: a creature with toughness 0 or less goes to its owner's graveyard.
    CreatureZeroOrLessToughness,
    /// CR 704.5g: lethal damage destroys a creature with toughness greater than 0.
    CreatureLethalDamage,
    /// CR 704.5h: deathtouch damage destroys a creature with toughness greater than 0.
    CreatureDeathtouchDamage,
    /// CR 704.5i: a planeswalker with loyalty 0 goes to its owner's graveyard.
    PlaneswalkerZeroLoyalty,
    /// CR 704.5j: the legend rule.
    LegendRule,
    /// CR 704.5k: the world rule.
    WorldRule,
    /// CR 704.5m: illegal or unattached Auras go to their owners' graveyards.
    AuraIllegalOrUnattached,
    /// CR 704.5n: illegal Equipment or Fortification attachments become unattached.
    EquipmentOrFortificationIllegalAttachment,
    /// CR 704.5p: battles, creatures, and other illegal attachments become unattached.
    BattleCreatureOrOtherIllegalAttachment,
    /// CR 704.5q: matching +1/+1 and -1/-1 counters annihilate.
    CounterPairCancellation,
    /// CR 704.5r: counters above a maximum are removed.
    CounterMaximum,
    /// CR 704.5s: completed Sagas are sacrificed.
    SagaFinalChapter,
    /// CR 704.5t: completed dungeons are removed from the game.
    DungeonCompleted,
    /// CR 704.5u: space sculptor sector designations are chosen.
    SpaceSculptorDesignation,
    /// CR 704.5v: a battle with defense 0 goes to its owner's graveyard.
    BattleZeroDefense,
    /// CR 704.5w: a battle without a protector chooses one or goes to graveyard.
    BattleMissingProtector,
    /// CR 704.5x: a Siege whose controller is its protector chooses a new protector.
    SiegeControllerProtector,
    /// CR 704.5y: duplicate Roles controlled by one player are put into graveyards.
    DuplicateRole,
    /// CR 704.5z: start your engines! gives speed 1.
    StartYourEnginesNoSpeed,
}

impl StateBasedActionKind {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::PlayerZeroOrLessLife => 0,
            Self::PlayerDrewFromEmptyLibrary => 1,
            Self::PlayerTenOrMorePoison => 2,
            Self::TokenOffBattlefield => 3,
            Self::CopyOutOfAllowedZone => 4,
            Self::CreatureZeroOrLessToughness => 5,
            Self::CreatureLethalDamage => 6,
            Self::CreatureDeathtouchDamage => 7,
            Self::PlaneswalkerZeroLoyalty => 8,
            Self::LegendRule => 9,
            Self::WorldRule => 10,
            Self::AuraIllegalOrUnattached => 11,
            Self::EquipmentOrFortificationIllegalAttachment => 12,
            Self::BattleCreatureOrOtherIllegalAttachment => 13,
            Self::CounterPairCancellation => 14,
            Self::CounterMaximum => 15,
            Self::SagaFinalChapter => 16,
            Self::DungeonCompleted => 17,
            Self::SpaceSculptorDesignation => 18,
            Self::BattleZeroDefense => 19,
            Self::BattleMissingProtector => 20,
            Self::SiegeControllerProtector => 21,
            Self::DuplicateRole => 22,
            Self::StartYourEnginesNoSpeed => 23,
        }
    }
}

const STATE_BASED_ACTION_TABLE: [StateBasedActionKind; 24] = [
    StateBasedActionKind::PlayerZeroOrLessLife,
    StateBasedActionKind::PlayerDrewFromEmptyLibrary,
    StateBasedActionKind::PlayerTenOrMorePoison,
    StateBasedActionKind::TokenOffBattlefield,
    StateBasedActionKind::CopyOutOfAllowedZone,
    StateBasedActionKind::CreatureZeroOrLessToughness,
    StateBasedActionKind::CreatureLethalDamage,
    StateBasedActionKind::CreatureDeathtouchDamage,
    StateBasedActionKind::PlaneswalkerZeroLoyalty,
    StateBasedActionKind::LegendRule,
    StateBasedActionKind::WorldRule,
    StateBasedActionKind::AuraIllegalOrUnattached,
    StateBasedActionKind::EquipmentOrFortificationIllegalAttachment,
    StateBasedActionKind::BattleCreatureOrOtherIllegalAttachment,
    StateBasedActionKind::CounterPairCancellation,
    StateBasedActionKind::CounterMaximum,
    StateBasedActionKind::SagaFinalChapter,
    StateBasedActionKind::DungeonCompleted,
    StateBasedActionKind::SpaceSculptorDesignation,
    StateBasedActionKind::BattleZeroDefense,
    StateBasedActionKind::BattleMissingProtector,
    StateBasedActionKind::SiegeControllerProtector,
    StateBasedActionKind::DuplicateRole,
    StateBasedActionKind::StartYourEnginesNoSpeed,
];

/// Returns the CR 704.5 table used by the state-based-action runner.
#[must_use]
pub const fn state_based_action_table() -> &'static [StateBasedActionKind] {
    &STATE_BASED_ACTION_TABLE
}

/// Summary returned after checking state-based actions to a fixpoint.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct StateBasedActionReport {
    iterations: u32,
    actions_performed: u32,
    players_lost: u32,
    permanents_moved_to_graveyard: u32,
    empty_library_draw_losses: u32,
    zero_toughness_creatures: u32,
    lethal_damage_creatures: u32,
    deathtouch_damage_creatures: u32,
    tokens_ceased: u32,
    copies_ceased: u32,
    counter_pairs_cancelled: u32,
    counters_removed: u32,
}

impl StateBasedActionReport {
    /// Returns how many nonempty CR 704 passes were needed.
    #[must_use]
    pub const fn iterations(self) -> u32 {
        self.iterations
    }

    /// Returns the total count of player-loss and permanent-movement actions.
    #[must_use]
    pub const fn actions_performed(self) -> u32 {
        self.actions_performed
    }

    /// Returns how many players lost during this check.
    #[must_use]
    pub const fn players_lost(self) -> u32 {
        self.players_lost
    }

    /// Returns how many permanents were moved to graveyards.
    #[must_use]
    pub const fn permanents_moved_to_graveyard(self) -> u32 {
        self.permanents_moved_to_graveyard
    }

    /// Returns how many player losses came from empty-library draw attempts.
    #[must_use]
    pub const fn empty_library_draw_losses(self) -> u32 {
        self.empty_library_draw_losses
    }

    /// Returns how many creatures moved for toughness 0 or less.
    #[must_use]
    pub const fn zero_toughness_creatures(self) -> u32 {
        self.zero_toughness_creatures
    }

    /// Returns how many creatures were destroyed by lethal damage.
    #[must_use]
    pub const fn lethal_damage_creatures(self) -> u32 {
        self.lethal_damage_creatures
    }

    /// Returns how many creatures were destroyed by deathtouch damage.
    #[must_use]
    pub const fn deathtouch_damage_creatures(self) -> u32 {
        self.deathtouch_damage_creatures
    }

    /// Returns how many token objects ceased to exist.
    #[must_use]
    pub const fn tokens_ceased(self) -> u32 {
        self.tokens_ceased
    }

    /// Returns how many copy objects ceased to exist.
    #[must_use]
    pub const fn copies_ceased(self) -> u32 {
        self.copies_ceased
    }

    /// Returns how many +1/+1 and -1/-1 counter pairs were cancelled.
    #[must_use]
    pub const fn counter_pairs_cancelled(self) -> u32 {
        self.counter_pairs_cancelled
    }

    /// Returns the total counters removed by pair-cancellation actions.
    #[must_use]
    pub const fn counters_removed(self) -> u32 {
        self.counters_removed
    }

    fn record_iteration(&mut self) {
        self.iterations = self.iterations.saturating_add(1);
    }

    fn record_player_loss(&mut self, kind: StateBasedActionKind) {
        self.actions_performed = self.actions_performed.saturating_add(1);
        self.players_lost = self.players_lost.saturating_add(1);
        if kind == StateBasedActionKind::PlayerDrewFromEmptyLibrary {
            self.empty_library_draw_losses = self.empty_library_draw_losses.saturating_add(1);
        }
    }

    fn record_permanent_move(&mut self, kind: StateBasedActionKind) {
        self.actions_performed = self.actions_performed.saturating_add(1);
        self.permanents_moved_to_graveyard = self.permanents_moved_to_graveyard.saturating_add(1);
        match kind {
            StateBasedActionKind::CreatureZeroOrLessToughness => {
                self.zero_toughness_creatures = self.zero_toughness_creatures.saturating_add(1);
            }
            StateBasedActionKind::CreatureLethalDamage => {
                self.lethal_damage_creatures = self.lethal_damage_creatures.saturating_add(1);
            }
            StateBasedActionKind::CreatureDeathtouchDamage => {
                self.deathtouch_damage_creatures =
                    self.deathtouch_damage_creatures.saturating_add(1);
            }
            _ => {}
        }
    }

    fn record_ceased_object(&mut self, kind: StateBasedActionKind) {
        self.actions_performed = self.actions_performed.saturating_add(1);
        match kind {
            StateBasedActionKind::TokenOffBattlefield => {
                self.tokens_ceased = self.tokens_ceased.saturating_add(1);
            }
            StateBasedActionKind::CopyOutOfAllowedZone => {
                self.copies_ceased = self.copies_ceased.saturating_add(1);
            }
            _ => {}
        }
    }

    fn record_counter_pair_cancelled(&mut self, amount: u32) {
        self.actions_performed = self.actions_performed.saturating_add(1);
        self.counter_pairs_cancelled = self.counter_pairs_cancelled.saturating_add(1);
        self.counters_removed = self
            .counters_removed
            .saturating_add(amount.saturating_mul(2));
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
    lost: bool,
    max_hand_size: u32,
    mulligans_taken: u32,
    opening_hand_kept: bool,
    mana_pool: ManaPool,
    lands_played_this_turn: u32,
}

impl PlayerState {
    /// Creates a player state with Magic's default constructed-game scalars.
    #[must_use]
    pub const fn new(id: PlayerId) -> Self {
        Self {
            id,
            life: 20,
            poison: 0,
            lost: false,
            max_hand_size: 7,
            mulligans_taken: 0,
            opening_hand_kept: false,
            mana_pool: ManaPool::empty(),
            lands_played_this_turn: 0,
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

    /// Returns whether this player has lost the game.
    #[must_use]
    pub const fn lost(self) -> bool {
        self.lost
    }

    /// Returns the player's current maximum hand size.
    #[must_use]
    pub const fn max_hand_size(self) -> u32 {
        self.max_hand_size
    }

    /// Returns how many London mulligans this player has taken.
    #[must_use]
    pub const fn mulligans_taken(self) -> u32 {
        self.mulligans_taken
    }

    /// Returns whether this player has kept their current opening hand.
    #[must_use]
    pub const fn opening_hand_kept(self) -> bool {
        self.opening_hand_kept
    }

    /// Returns the player's current mana pool.
    #[must_use]
    pub const fn mana_pool(self) -> ManaPool {
        self.mana_pool
    }

    /// Returns how many lands this player has played during the current turn.
    #[must_use]
    pub const fn lands_played_this_turn(self) -> u32 {
        self.lands_played_this_turn
    }
}

/// Arena record for one game object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectRecord {
    id: ObjectId,
    card: CardId,
    owner: PlayerId,
    controller: PlayerId,
    tapped: bool,
    base_object: BaseObjectCharacteristics,
    base_creature: Option<BaseCreatureCharacteristics>,
    damage_marked: u32,
    deathtouch_damage_marked: bool,
    loyalty: Option<i32>,
    token: bool,
    copy_source: Option<ObjectId>,
    attached_to: Option<ObjectId>,
    controlled_since_turn: u32,
    color_identity: ObjectColors,
    commander: bool,
    commander_cast_count: u32,
}

/// Printed definition used to populate one hidden slot in an AI simulation.
///
/// This value contains no object identity, controller state, counters, or zone
/// metadata. [`GameState::determinized_clone`] applies it only to a slot that
/// is redacted from the supplied observer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HiddenCardDefinition {
    card: CardId,
    base_object: BaseObjectCharacteristics,
    base_creature: Option<BaseCreatureCharacteristics>,
    color_identity: ObjectColors,
}

impl HiddenCardDefinition {
    /// Creates one complete printed hidden-card definition.
    #[must_use]
    pub const fn new(
        card: CardId,
        base_object: BaseObjectCharacteristics,
        base_creature: Option<BaseCreatureCharacteristics>,
        color_identity: ObjectColors,
    ) -> Self {
        Self {
            card,
            base_object,
            base_creature,
            color_identity,
        }
    }

    /// Returns the sampled card-definition ID.
    #[must_use]
    pub const fn card(self) -> CardId {
        self.card
    }

    /// Returns the sampled printed object characteristics.
    #[must_use]
    pub const fn base_object(self) -> BaseObjectCharacteristics {
        self.base_object
    }

    /// Returns the sampled printed creature characteristics, when present.
    #[must_use]
    pub const fn base_creature(self) -> Option<BaseCreatureCharacteristics> {
        self.base_creature
    }

    /// Returns the sampled Commander color identity.
    #[must_use]
    pub const fn color_identity(self) -> ObjectColors {
        self.color_identity
    }
}

/// One validated hidden-zone slot assignment for an AI simulation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HiddenSlotDefinition {
    zone: ZoneId,
    slot: u32,
    definition: HiddenCardDefinition,
}

impl HiddenSlotDefinition {
    /// Creates an assignment for a zero-based visible-zone slot.
    #[must_use]
    pub const fn new(zone: ZoneId, slot: u32, definition: HiddenCardDefinition) -> Self {
        Self {
            zone,
            slot,
            definition,
        }
    }

    /// Returns the assigned hidden zone.
    #[must_use]
    pub const fn zone(self) -> ZoneId {
        self.zone
    }

    /// Returns the zero-based slot within the visible zone projection.
    #[must_use]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// Returns the printed definition sampled for this slot.
    #[must_use]
    pub const fn definition(self) -> HiddenCardDefinition {
        self.definition
    }
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

    /// Returns true if this object is tapped.
    #[must_use]
    pub const fn tapped(self) -> bool {
        self.tapped
    }

    /// Returns base printed card types and colors.
    #[must_use]
    pub const fn base_object(self) -> BaseObjectCharacteristics {
        self.base_object
    }

    /// Returns base printed creature characteristics, if this object is a creature.
    #[must_use]
    pub const fn base_creature(self) -> Option<BaseCreatureCharacteristics> {
        self.base_creature
    }

    /// Returns damage currently marked on this object.
    #[must_use]
    pub const fn damage_marked(self) -> u32 {
        self.damage_marked
    }

    /// Returns whether this object has deathtouch damage pending an SBA check.
    #[must_use]
    pub const fn deathtouch_damage_marked(self) -> bool {
        self.deathtouch_damage_marked
    }

    /// Returns the object's loyalty value, if this record is tracking one.
    #[must_use]
    pub const fn loyalty(self) -> Option<i32> {
        self.loyalty
    }

    /// Returns true if this object is a token.
    #[must_use]
    pub const fn is_token(self) -> bool {
        self.token
    }

    /// Returns the permanent copied to create this object, if any.
    #[must_use]
    pub const fn copy_source(self) -> Option<ObjectId> {
        self.copy_source
    }

    /// Returns the object this object is attached to, if any.
    #[must_use]
    pub const fn attached_to(self) -> Option<ObjectId> {
        self.attached_to
    }

    /// Returns true if this object is a copy object.
    #[must_use]
    pub const fn is_copy(self) -> bool {
        self.copy_source.is_some()
    }

    /// Returns the turn number since which this controller has controlled it.
    #[must_use]
    pub const fn controlled_since_turn(self) -> u32 {
        self.controlled_since_turn
    }

    /// Returns this object's Commander color identity metadata.
    #[must_use]
    pub const fn color_identity(self) -> ObjectColors {
        self.color_identity
    }

    /// Returns true if this object is designated as a commander.
    #[must_use]
    pub const fn is_commander(self) -> bool {
        self.commander
    }

    /// Returns how many times this commander has been cast from the command zone.
    #[must_use]
    pub const fn commander_cast_count(self) -> u32 {
        self.commander_cast_count
    }

    /// Returns the generic Commander tax for this object, if it fits in `u32`.
    #[must_use]
    pub fn commander_tax(self) -> Option<ManaCost> {
        self.commander_cast_count
            .checked_mul(2)
            .map(|generic| ManaCost::new(0, 0, 0, 0, 0, generic))
    }
}

/// Arena storage for game objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectArena {
    // clone_surface: Copy-on-write object arena; GameState clones share until mutation.
    records: Arc<Vec<ObjectRecord>>,
}

impl Default for ObjectArena {
    fn default() -> Self {
        Self {
            records: Arc::new(Vec::new()),
        }
    }
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

    fn push(
        &mut self,
        card: CardId,
        owner: PlayerId,
        controller: PlayerId,
        controlled_since_turn: u32,
    ) -> ObjectId {
        let records = Arc::make_mut(&mut self.records);
        let id = ObjectId(records.len() as u32);
        records.push(ObjectRecord {
            id,
            card,
            owner,
            controller,
            tapped: false,
            base_object: BaseObjectCharacteristics::new(ObjectTypes::none(), ObjectColors::none()),
            base_creature: None,
            damage_marked: 0,
            deathtouch_damage_marked: false,
            loyalty: None,
            token: false,
            copy_source: None,
            attached_to: None,
            controlled_since_turn,
            color_identity: ObjectColors::none(),
            commander: false,
            commander_cast_count: 0,
        });
        id
    }

    fn get_mut(&mut self, id: ObjectId) -> Option<&mut ObjectRecord> {
        Arc::make_mut(&mut self.records).get_mut(id.index())
    }

    fn records_mut(&mut self) -> &mut Vec<ObjectRecord> {
        Arc::make_mut(&mut self.records)
    }
}

/// Ordered object list for a zone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Zone {
    id: ZoneId,
    // clone_surface: Copy-on-write zone membership; IDs are shared across state clones.
    objects: Arc<Vec<ObjectId>>,
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
        self.objects.as_slice()
    }

    fn objects_mut(&mut self) -> &mut Vec<ObjectId> {
        Arc::make_mut(&mut self.objects)
    }
}

/// One object slot as visible to a single observing player.
// Known object records remain inline so this public value type stays Copy.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectView {
    /// A visible object with its public object record.
    Known {
        /// Visible object record.
        object: ObjectRecord,
        /// Effective characteristics after continuous effects.
        characteristics: ObjectCharacteristics,
    },
    /// A hidden object placeholder. Count and zone position are visible, identity is not.
    Hidden,
}

impl ObjectView {
    /// Returns the object record when this view is known.
    #[must_use]
    pub const fn known(self) -> Option<ObjectRecord> {
        match self {
            Self::Known { object, .. } => Some(object),
            Self::Hidden => None,
        }
    }

    /// Returns effective characteristics when this object is known.
    #[must_use]
    pub const fn characteristics(self) -> Option<ObjectCharacteristics> {
        match self {
            Self::Known {
                characteristics, ..
            } => Some(characteristics),
            Self::Hidden => None,
        }
    }

    /// Returns true when this object slot is hidden from the observer.
    #[must_use]
    pub const fn is_hidden(self) -> bool {
        matches!(self, Self::Hidden)
    }
}

/// One zone as visible to a single observing player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZoneView {
    id: ZoneId,
    objects: Vec<ObjectView>,
}

impl ZoneView {
    /// Returns this visible zone's ID.
    #[must_use]
    pub const fn id(&self) -> ZoneId {
        self.id
    }

    /// Returns visible object slots in zone order.
    #[must_use]
    pub fn objects(&self) -> &[ObjectView] {
        &self.objects
    }
}

/// Redacted state projection for one observing player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerView {
    observer: PlayerId,
    turn_number: u32,
    outcome: GameOutcome,
    starting_player: Option<PlayerId>,
    opening_hands_drawn: bool,
    turn_order: Vec<PlayerId>,
    range_of_influence: RangeOfInfluence,
    active_player: Option<PlayerId>,
    priority_player: Option<PlayerId>,
    current_step: Option<Step>,
    players: Vec<PlayerState>,
    zones: Vec<ZoneView>,
}

/// Current canonical redacted-player-view schema version.
pub const PLAYER_VIEW_SCHEMA_VERSION: u32 = 1;

/// Deterministic hash of exactly one observer's redacted state projection.
///
/// Hidden object identities never contribute to this value. Revealed or
/// otherwise visible identities, characteristics, zone order, and public
/// scalar state do contribute.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlayerViewHash(u64);

impl PlayerViewHash {
    /// Returns the raw 64-bit FNV hash value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct KnownObject {
    observer: PlayerId,
    object: ObjectId,
}

impl PlayerView {
    /// Computes the canonical hash of this redacted projection.
    ///
    /// The hash is suitable for benchmark pairing and near-state
    /// deduplication. It is not a full-information game-state hash.
    #[must_use]
    pub fn deterministic_hash(&self) -> PlayerViewHash {
        let mut hash = Fnva64::new();
        for byte in b"forge-player-view" {
            hash.write_u8(*byte);
        }
        hash.write_u32(PLAYER_VIEW_SCHEMA_VERSION);
        hash.write_u32(self.observer.0);
        hash.write_u32(self.turn_number);
        hash.write_game_outcome(self.outcome);
        hash.write_optional_player(self.starting_player);
        hash.write_bool(self.opening_hands_drawn);
        hash.write_u32(self.turn_order.len() as u32);
        for player in &self.turn_order {
            hash.write_u32(player.0);
        }
        hash.write_u8(self.range_of_influence.canonical_code());
        hash.write_optional_player(self.active_player);
        hash.write_optional_player(self.priority_player);
        hash.write_optional_step(self.current_step);
        hash.write_u32(self.players.len() as u32);
        for player in &self.players {
            hash.write_player_state(*player);
        }
        hash.write_u32(self.zones.len() as u32);
        for zone in &self.zones {
            hash.write_zone_id(zone.id);
            hash.write_u32(zone.objects.len() as u32);
            for object in &zone.objects {
                match object {
                    ObjectView::Hidden => hash.write_u8(0),
                    ObjectView::Known {
                        object,
                        characteristics,
                    } => {
                        hash.write_u8(1);
                        hash.write_object_record(*object);
                        hash.write_object_characteristics(*characteristics);
                    }
                }
            }
        }
        PlayerViewHash(hash.finish())
    }

    /// Returns the player this projection is for.
    #[must_use]
    pub const fn observer(&self) -> PlayerId {
        self.observer
    }

    /// Returns the visible turn number.
    #[must_use]
    pub const fn turn_number(&self) -> u32 {
        self.turn_number
    }

    /// Returns the visible game outcome.
    #[must_use]
    pub const fn game_outcome(&self) -> GameOutcome {
        self.outcome
    }

    /// Returns the visible starting player chosen during setup.
    #[must_use]
    pub const fn starting_player(&self) -> Option<PlayerId> {
        self.starting_player
    }

    /// Returns whether setup has drawn visible-count opening hands.
    #[must_use]
    pub const fn opening_hands_drawn(&self) -> bool {
        self.opening_hands_drawn
    }

    /// Returns the visible explicit multiplayer turn order.
    #[must_use]
    pub fn turn_order(&self) -> &[PlayerId] {
        &self.turn_order
    }

    /// Returns the visible range-of-influence policy.
    #[must_use]
    pub const fn range_of_influence(&self) -> RangeOfInfluence {
        self.range_of_influence
    }

    /// Returns the visible active player.
    #[must_use]
    pub const fn active_player(&self) -> Option<PlayerId> {
        self.active_player
    }

    /// Returns the visible priority player.
    #[must_use]
    pub const fn priority_player(&self) -> Option<PlayerId> {
        self.priority_player
    }

    /// Returns the visible current step.
    #[must_use]
    pub const fn current_step(&self) -> Option<Step> {
        self.current_step
    }

    /// Returns visible player scalar state.
    #[must_use]
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// Returns visible zones in canonical state order.
    #[must_use]
    pub fn zones(&self) -> &[ZoneView] {
        &self.zones
    }

    /// Returns one visible zone by ID.
    #[must_use]
    pub fn zone(&self, id: ZoneId) -> Option<&ZoneView> {
        self.zones.iter().find(|zone| zone.id == id)
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
    /// AI determinization may assign only hidden hand or library slots.
    InvalidDeterminizationZone(ZoneId),
    /// An AI determinization slot lies outside the selected zone.
    DeterminizationSlotOutOfBounds {
        /// Zone containing the requested slot.
        zone: ZoneId,
        /// Zero-based requested slot.
        slot: u32,
    },
    /// An AI determinization attempted to replace an identity known to the observer.
    DeterminizationSlotKnown {
        /// Zone containing the known slot.
        zone: ZoneId,
        /// Zero-based known slot.
        slot: u32,
    },
    /// An AI determinization assigned one hidden slot more than once.
    DuplicateDeterminizationSlot {
        /// Zone containing the duplicate slot.
        zone: ZoneId,
        /// Zero-based duplicate slot.
        slot: u32,
    },
    /// An AI determinization did not cover every and only hidden slot.
    DeterminizationSlotCountMismatch {
        /// Hidden slots visible to the observer.
        expected: u32,
        /// Unique valid assignments supplied.
        actual: u32,
    },
    /// Tokens and copy objects cannot be rebound as sampled deck cards.
    InvalidDeterminizationObject(ObjectId),
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
    /// Turn order has already been decided.
    TurnOrderAlreadyDecided,
    /// Opening-hand setup requires turn order to be decided first.
    TurnOrderNotDecided,
    /// An explicit turn order did not name exactly every player.
    TurnOrderPlayerCountMismatch {
        /// Current game player count.
        expected: u32,
        /// Players named by the order request.
        actual: u32,
    },
    /// An explicit turn order named the same player more than once.
    DuplicateTurnOrderPlayer(PlayerId),
    /// An explicit turn order omitted this player.
    TurnOrderMissingPlayer(PlayerId),
    /// Turn structure tried to start with a player other than the setup starter.
    StartingPlayerMismatch {
        /// Player chosen by setup to start.
        expected: PlayerId,
        /// Player passed to start-turn.
        actual: PlayerId,
    },
    /// Opening hands have already been drawn.
    OpeningHandsAlreadyDrawn,
    /// A mulligan or keep decision was requested before opening hands were drawn.
    OpeningHandsNotDrawn,
    /// A player tried to mulligan after keeping an opening hand.
    MulliganAfterKeep(PlayerId),
    /// A player tried to keep an opening hand more than once.
    OpeningHandAlreadyKept(PlayerId),
    /// A turn was requested before a player kept their opening hand.
    OpeningHandKeepPending(PlayerId),
    /// A London mulligan count overflowed.
    MulliganCountOverflow,
    /// A London keep provided the wrong number of bottomed cards.
    InvalidOpeningHandBottomCount {
        /// Required bottomed-card count.
        expected: u32,
        /// Actual bottomed-card count.
        actual: u32,
    },
    /// A bottomed-card list named the same object more than once.
    DuplicateOpeningHandBottomCard(ObjectId),
    /// A bottomed card was not in the player's hand.
    OpeningHandBottomCardNotInHand {
        /// Player making the keep decision.
        player: PlayerId,
        /// Object that was not in that player's hand.
        object: ObjectId,
    },
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
    /// Triggered abilities must be put on the stack before priority actions.
    PendingTriggeredAbilities,
    /// The requested replacement/prevention effect ID does not exist.
    UnknownReplacementEffect(ReplacementEffectId),
    /// A replacement ordering preference named the same effect more than once.
    DuplicateReplacementEffect(ReplacementEffectId),
    /// The requested continuous effect ID does not exist.
    UnknownContinuousEffect(ContinuousEffectId),
    /// A continuous-effect definition named the same dependency more than once.
    DuplicateContinuousEffectDependency(ContinuousEffectId),
    /// The requested activated ability ID does not exist.
    UnknownActivatedAbility(ActivatedAbilityId),
    /// The requested cost modifier ID does not exist.
    UnknownCostModifier(CostModifierId),
    /// The requested targeting or combat restriction ID does not exist.
    UnknownRestriction(RestrictionId),
    /// The object cannot currently be used as that activated ability's source.
    ObjectNotActivatable(ObjectId),
    /// A closed activation condition was not satisfied.
    ActivationConditionNotMet(ActivatedAbilityId),
    /// The source object is already tapped and cannot pay a tap cost.
    SourceAlreadyTapped(ObjectId),
    /// A loyalty ability of this permanent has already been activated this turn.
    LoyaltyAbilityAlreadyActivatedThisTurn(ObjectId),
    /// The source object does not have enough loyalty for the requested cost.
    InsufficientLoyalty(ObjectId),
    /// A stack resolution was requested while the stack was empty.
    EmptyStack,
    /// The requested stack entry ID does not exist.
    UnknownStackEntry(StackEntryId),
    /// A stack entry refers to a spell object that is no longer on the stack.
    StackObjectNotOnStack(ObjectId),
    /// The requested object cannot be used as a copy source.
    ObjectNotCopyable(ObjectId),
    /// A mana arithmetic operation overflowed.
    ManaValueOverflow,
    /// A life-total arithmetic operation overflowed.
    LifeTotalOverflow,
    /// A poison-counter arithmetic operation overflowed.
    PoisonCounterOverflow,
    /// A counter arithmetic operation overflowed.
    CounterOverflow,
    /// A Commander cast-count operation overflowed.
    CommanderCastCountOverflow(ObjectId),
    /// Commander tax could not be represented as a mana cost.
    CommanderTaxOverflow(ObjectId),
    /// The object is not designated as a commander.
    ObjectNotCommander(ObjectId),
    /// A player tried to make a Commander zone choice for another player's commander.
    CommanderZoneChoiceOwnerMismatch {
        /// Player making the choice.
        player: PlayerId,
        /// Commander named by the choice.
        object: ObjectId,
    },
    /// A Commander zone choice was requested outside a graveyard or exile zone.
    InvalidCommanderZoneChoice {
        /// Commander named by the choice.
        object: ObjectId,
        /// Zone the commander currently occupies.
        from: ZoneId,
    },
    /// A player has no commander identity metadata to validate against.
    NoCommanderForPlayer(PlayerId),
    /// An object's color identity is outside its commander's allowed identity.
    CommanderColorIdentityViolation {
        /// Player whose commander identity was used.
        player: PlayerId,
        /// Object that failed validation.
        object: ObjectId,
        /// Allowed commander identity.
        allowed: ObjectColors,
        /// Actual object identity.
        actual: ObjectColors,
    },
    /// A request tried to remove more counters than are present.
    InsufficientCounters {
        /// Object with too few counters.
        object: ObjectId,
        /// Counter kind being removed.
        kind: CounterKind,
        /// Counters currently present.
        available: u32,
        /// Counters requested for removal.
        requested: u32,
    },
    /// A player does not have enough mana for a requested payment.
    InsufficientMana,
    /// A proposed explicit payment does not satisfy the cost.
    InvalidPaymentPlan,
    /// The object cannot be cast from its current zone by that player.
    ObjectNotCastable(ObjectId),
    /// The object cannot be played as a land from its current zone by that player.
    ObjectNotPlayableAsLand(ObjectId),
    /// A land play was attempted outside an active player's empty-stack main phase.
    InvalidLandPlayTiming,
    /// The player has used every currently permitted land play this turn.
    LandPlayLimitReached(PlayerId),
    /// The object is not currently a permanent on the battlefield.
    ObjectNotOnBattlefield(ObjectId),
    /// The requested spell cannot be cast at the current time.
    InvalidSpellTiming,
    /// Split second currently forbids casting spells or activating non-mana abilities.
    SplitSecondActionForbidden,
    /// Target requirements and selected targets have different lengths.
    TargetCountMismatch {
        /// Number of target slots required by the spell.
        required: u32,
        /// Number of targets selected by the player.
        selected: u32,
    },
    /// A selected target is not legal while the spell is being announced.
    IllegalTarget {
        /// Zero-based target slot.
        index: u32,
        /// Target that failed legality.
        target: TargetChoice,
    },
    /// A combat action was requested in the wrong step.
    InvalidCombatStep {
        /// Step required by that action.
        expected: Step,
        /// Actual current step.
        actual: Option<Step>,
    },
    /// A combat action was requested by the wrong player.
    InvalidCombatPlayer(PlayerId),
    /// The object is not currently a creature.
    NotACreature(ObjectId),
    /// The creature is tapped and cannot attack or block.
    CreatureTapped(ObjectId),
    /// The creature has not been controlled continuously since the turn began.
    SummoningSick(ObjectId),
    /// The same object appeared more than once in a declaration or assignment.
    DuplicateCombatObject(ObjectId),
    /// The declared attack is not legal.
    IllegalAttack(ObjectId),
    /// The declared block is not legal.
    IllegalBlock {
        /// Blocking creature.
        blocker: ObjectId,
        /// Attacking creature.
        attacker: ObjectId,
    },
    /// A damage assignment was missing for a source that must assign damage.
    MissingCombatDamageAssignment(ObjectId),
    /// A damage assignment is illegal for its source or target set.
    IllegalCombatDamageAssignment(ObjectId),
    /// Combat damage arithmetic overflowed.
    CombatDamageOverflow,
    /// State-based actions did not reach a fixpoint within the limit.
    StateBasedActionLoop,
}

/// One externally visible kernel action.
///
/// T1.R1 keeps the action surface broad enough to cover current T1 setup and
/// rules-kernel operations while preventing consumers from calling low-level
/// mutators directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Set the deterministic game seed.
    SetSeed {
        /// New deterministic seed.
        seed: u64,
    },
    /// Add one player and that player's zones.
    AddPlayer,
    /// Randomly choose the starting player from the deterministic seed stream.
    DecideTurnOrder,
    /// Set an explicit N-player turn order.
    SetTurnOrder {
        /// Player order, starting with the first player in the ring.
        order: Vec<PlayerId>,
    },
    /// Draw opening hands for all players in starting-player turn order.
    DrawOpeningHands,
    /// Take one London mulligan for a player.
    TakeMulligan {
        /// Player taking a mulligan.
        player: PlayerId,
    },
    /// Keep a London opening hand and put mulligan-count cards on the library bottom.
    KeepOpeningHand {
        /// Player keeping the hand.
        player: PlayerId,
        /// Cards to put on the bottom, in bottom-to-top order.
        bottom: Vec<ObjectId>,
    },
    /// Set a player's maximum hand size.
    SetPlayerMaxHandSize {
        /// Player to update.
        player: PlayerId,
        /// New maximum hand size.
        max_hand_size: u32,
    },
    /// Set a player's life total.
    SetPlayerLife {
        /// Player to update.
        player: PlayerId,
        /// New life total.
        life: i32,
    },
    /// Make a player lose life.
    LoseLife {
        /// Player losing life.
        player: PlayerId,
        /// Life amount to lose.
        amount: u32,
    },
    /// Make a player gain life.
    GainLife {
        /// Player gaining life.
        player: PlayerId,
        /// Life amount to gain.
        amount: u32,
    },
    /// Draw cards from a player's library into that player's hand.
    DrawCards {
        /// Player drawing cards.
        player: PlayerId,
        /// Number of cards to draw.
        count: u32,
    },
    /// Shuffle a player's library using the deterministic game RNG.
    ShuffleLibrary {
        /// Player whose library is shuffled.
        player: PlayerId,
    },
    /// Place one object already in a player's library on top of that library.
    PutObjectOnTopOfLibrary {
        /// Player whose library contains the object.
        player: PlayerId,
        /// Object to place on top.
        object: ObjectId,
    },
    /// Add poison counters to a player.
    AddPoisonCounters {
        /// Player receiving counters.
        player: PlayerId,
        /// Number of counters to add.
        amount: u32,
    },
    /// Add mana to a player's pool.
    AddManaToPool {
        /// Player receiving mana.
        player: PlayerId,
        /// Mana to add.
        mana: ManaPool,
    },
    /// Clear one player's mana pool.
    ClearManaPool {
        /// Player whose pool is cleared.
        player: PlayerId,
    },
    /// Pay one explicit mana payment plan.
    PayMana {
        /// Paying player.
        player: PlayerId,
        /// Cost being paid.
        cost: ManaCost,
        /// Payment plan to apply.
        plan: PaymentPlan,
    },
    /// Scry by moving selected top cards to the library bottom.
    Scry {
        /// Player whose library is inspected.
        player: PlayerId,
        /// Number of top library cards considered.
        count: u32,
        /// Selected cards to bottom, in bottom-to-top order.
        bottom: Vec<ObjectId>,
    },
    /// Surveil by moving selected top cards to the graveyard.
    Surveil {
        /// Player whose library is inspected.
        player: PlayerId,
        /// Number of top library cards considered.
        count: u32,
        /// Selected cards to move to that player's graveyard.
        graveyard: Vec<ObjectId>,
    },
    /// Cycle a card from hand, then draw a card.
    Cycle {
        /// Cycling player.
        player: PlayerId,
        /// Card object being cycled.
        object: ObjectId,
        /// Cycling mana cost.
        cost: ManaCost,
        /// Payment plan to apply.
        payment: PaymentPlan,
    },
    /// Attach or unattach one object.
    AttachObject {
        /// Object that becomes attached.
        attachment: ObjectId,
        /// Object to attach to, or none to unattach.
        target: Option<ObjectId>,
    },
    /// Equip an equipment object to a controlled creature.
    Equip {
        /// Activating player.
        player: PlayerId,
        /// Equipment object.
        equipment: ObjectId,
        /// Target creature controlled by the player.
        target: ObjectId,
        /// Equip mana cost.
        cost: ManaCost,
        /// Payment plan to apply.
        payment: PaymentPlan,
    },
    /// Set or clear an object's loyalty value.
    SetObjectLoyalty {
        /// Object to update.
        object: ObjectId,
        /// New loyalty value, or none to clear loyalty tracking.
        loyalty: Option<i32>,
    },
    /// Set an object's Commander color identity metadata.
    SetObjectColorIdentity {
        /// Object to update.
        object: ObjectId,
        /// Color identity metadata.
        colors: ObjectColors,
    },
    /// Designate an object as a commander with color identity metadata.
    DesignateCommander {
        /// Object to designate.
        object: ObjectId,
        /// Commander color identity.
        color_identity: ObjectColors,
    },
    /// Record one commander cast for tax tracking.
    RecordCommanderCast {
        /// Commander object.
        object: ObjectId,
    },
    /// Have a commander's owner move it from graveyard or exile to the command zone.
    ChooseCommanderZone {
        /// Player making the owner-only choice.
        player: PlayerId,
        /// Commander moving to the command zone.
        object: ObjectId,
    },
    /// Validate objects against a player's Commander color identity.
    ValidateCommanderColorIdentity {
        /// Player whose commander identity sets the allowed colors.
        player: PlayerId,
        /// Objects to validate.
        objects: Vec<ObjectId>,
    },
    /// Add counters to an object.
    AddObjectCounters {
        /// Object receiving counters.
        object: ObjectId,
        /// Counter kind to add.
        kind: CounterKind,
        /// Counter amount to add.
        amount: u32,
    },
    /// Remove counters from an object.
    RemoveObjectCounters {
        /// Object losing counters.
        object: ObjectId,
        /// Counter kind to remove.
        kind: CounterKind,
        /// Counter amount to remove.
        amount: u32,
    },
    /// Register one declarative activated ability definition.
    RegisterActivatedAbility {
        /// Data-only activated ability definition.
        definition: ActivatedAbilityDefinition,
    },
    /// Register one activated ability cost modifier.
    RegisterCostModifier {
        /// Data-only activation cost modifier.
        definition: CostModifierDefinition,
    },
    /// Register one targeting or combat restriction.
    RegisterRestriction {
        /// Data-only restriction definition.
        definition: RestrictionDefinition,
    },
    /// Activate one registered ability using an explicit payment plan.
    ActivateAbility {
        /// Activating player.
        player: PlayerId,
        /// Registered ability to activate.
        ability: ActivatedAbilityId,
        /// Mana payment selected for the effective activation cost.
        payment: PaymentPlan,
    },
    /// Set base printed card types and colors for one object.
    SetBaseObjectCharacteristics {
        /// Object to update.
        object: ObjectId,
        /// Base printed characteristics to set.
        base: BaseObjectCharacteristics,
    },
    /// Set T1 base printed creature characteristics for one object.
    SetBaseCreatureCharacteristics {
        /// Object to update.
        object: ObjectId,
        /// Base printed characteristics to set.
        base: BaseCreatureCharacteristics,
    },
    /// Clear T1 base printed creature characteristics from one object.
    ClearBaseCreatureCharacteristics {
        /// Object to update.
        object: ObjectId,
    },
    /// Set an object's tapped status.
    SetObjectTapped {
        /// Object to update.
        object: ObjectId,
        /// New tapped status.
        tapped: bool,
    },
    /// Mark damage on a creature object.
    MarkDamageOnObject {
        /// Object receiving damage.
        object: ObjectId,
        /// Damage amount to mark.
        amount: u32,
    },
    /// Deal noncombat damage through the production replacement pipeline.
    DealDamage {
        /// Damage source, when represented.
        source: Option<ObjectId>,
        /// Player or object receiving damage.
        target: CombatDamageTarget,
        /// Damage amount before replacement effects.
        amount: u32,
    },
    /// Check state-based actions to a fixpoint.
    CheckStateBasedActions,
    /// Start a turn for the chosen active player.
    StartTurn {
        /// Active player for the new turn.
        active_player: PlayerId,
    },
    /// Advance from the current step or main-phase segment.
    AdvanceStep,
    /// Pass priority for the current priority player.
    PassPriority {
        /// Player passing priority.
        player: PlayerId,
    },
    /// Cast a spell through the CR 601 pipeline.
    CastSpell {
        /// Casting player.
        player: PlayerId,
        /// Spell object.
        object: ObjectId,
        /// Cast request.
        request: CastSpellRequest,
    },
    /// Play a land as a main-phase special action without using the stack.
    PlayLand {
        /// Player taking the special action.
        player: PlayerId,
        /// Land object in that player's hand.
        object: ObjectId,
    },
    /// Counter one spell or ability currently on the stack.
    CounterStackEntry {
        /// Stack entry to counter.
        entry: StackEntryId,
    },
    /// Destroy one permanent, respecting current indestructible characteristics.
    DestroyPermanent {
        /// Permanent to destroy.
        object: ObjectId,
    },
    /// Put a spell object on the stack through the low-level T1 stack helper.
    PutSpellOnStack {
        /// Controlling player.
        player: PlayerId,
        /// Spell object.
        object: ObjectId,
        /// Stack-object kind.
        kind: StackObjectKind,
        /// Whether priority remains with that player.
        hold_priority: bool,
    },
    /// Put an ability on the stack through the low-level T1 stack helper.
    PutAbilityOnStack {
        /// Controlling player.
        player: PlayerId,
        /// Stack-object kind.
        kind: StackObjectKind,
        /// Whether priority remains with that player.
        hold_priority: bool,
    },
    /// Put simultaneous triggered abilities on the stack in APNAP order.
    PutSimultaneousAbilitiesApnap {
        /// Ability controllers in source order.
        abilities: Vec<PlayerId>,
        /// Stack-object kind.
        kind: StackObjectKind,
    },
    /// Register one declarative triggered ability definition.
    RegisterTriggeredAbility {
        /// Data-only trigger definition.
        definition: TriggerDefinition,
    },
    /// Put all currently pending triggered abilities on the stack in APNAP order.
    PutPendingTriggeredAbilitiesOnStack,
    /// Register one declarative replacement/prevention effect definition.
    RegisterReplacementEffect {
        /// Data-only replacement/prevention definition.
        definition: ReplacementDefinition,
    },
    /// Set one affected player's deterministic replacement application order.
    SetReplacementChoiceOrder {
        /// Player who makes replacement ordering choices.
        chooser: PlayerId,
        /// Effect IDs in preferred order; omitted applicable effects use ID order.
        order: Vec<ReplacementEffectId>,
    },
    /// Register one declarative continuous effect definition.
    RegisterContinuousEffect {
        /// Data-only continuous effect definition.
        definition: ContinuousEffectDefinition,
    },
    /// Record whether attackers were declared in this combat.
    SetAttackersDeclaredThisCombat {
        /// True if at least one attacker was declared.
        attackers_declared: bool,
    },
    /// Declare attackers.
    DeclareAttackers {
        /// Attacking player.
        player: PlayerId,
        /// Attack declarations.
        attacks: Vec<AttackDeclaration>,
    },
    /// Declare blockers.
    DeclareBlockers {
        /// Defending player.
        defending_player: PlayerId,
        /// Block declarations.
        blocks: Vec<BlockDeclaration>,
    },
    /// Assign and deal combat damage.
    AssignCombatDamage {
        /// Damage assignment requests.
        assignments: Vec<CombatDamageAssignmentRequest>,
    },
    /// Request the cleanup priority exception.
    RequestCleanupPriority,
    /// Add one duration marker.
    AddDurationMarker {
        /// Duration to track.
        duration: EffectDuration,
    },
    /// Create one object in a zone.
    CreateObject {
        /// Card definition ID.
        card: CardId,
        /// Object owner.
        owner: PlayerId,
        /// Object controller.
        controller: PlayerId,
        /// Destination zone.
        zone: ZoneId,
    },
    /// Move one object to another zone.
    MoveObject {
        /// Object to move.
        object: ObjectId,
        /// Destination zone.
        to: ZoneId,
    },
    /// Create one token object.
    CreateToken {
        /// Card definition ID for the token face.
        card: CardId,
        /// Token owner.
        owner: PlayerId,
        /// Token controller.
        controller: PlayerId,
        /// Base printed card types and colors.
        base_object: BaseObjectCharacteristics,
        /// Optional base creature values.
        base: Option<BaseCreatureCharacteristics>,
    },
    /// Create one permanent copy object from a source object's copiable values.
    CreatePermanentCopy {
        /// Source object being copied.
        source: ObjectId,
        /// New object's owner.
        owner: PlayerId,
        /// New object's controller.
        controller: PlayerId,
        /// Whether the copy object is also a token.
        token: bool,
    },
    /// Copy one existing stack entry.
    CopyStackEntry {
        /// Controller of the copy.
        player: PlayerId,
        /// Stack entry to copy.
        entry: StackEntryId,
    },
    /// Publicly reveal one or more existing objects without moving them.
    RevealObjects {
        /// Objects revealed in deterministic source order.
        objects: Vec<ObjectId>,
    },
    /// Destroy one permanent without allowing a regeneration shield to replace it.
    DestroyPermanentWithoutRegeneration {
        /// Permanent to destroy.
        object: ObjectId,
    },
}

/// Ordered set of currently legal actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionList {
    storage: ActionListStorage,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
// The one-action representation intentionally avoids a heap allocation.
#[allow(clippy::large_enum_variant)]
enum ActionListStorage {
    #[default]
    Empty,
    One([Action; 1]),
    Many(Vec<Action>),
}

impl ActionList {
    /// Creates an action list from canonical actions.
    #[must_use]
    pub fn new(actions: Vec<Action>) -> Self {
        let storage = match actions.len() {
            0 => ActionListStorage::Empty,
            1 => {
                let mut actions = actions;
                if let Some(action) = actions.pop() {
                    ActionListStorage::One([action])
                } else {
                    ActionListStorage::Empty
                }
            }
            _ => ActionListStorage::Many(actions),
        };
        Self { storage }
    }

    /// Creates an empty action list.
    #[must_use]
    #[inline]
    pub fn empty() -> Self {
        Self {
            storage: ActionListStorage::Empty,
        }
    }

    /// Creates an action list containing exactly one action.
    #[must_use]
    #[inline]
    pub fn single(action: Action) -> Self {
        Self {
            storage: ActionListStorage::One([action]),
        }
    }

    /// Returns the legal actions in deterministic order.
    #[must_use]
    #[inline]
    pub fn actions(&self) -> &[Action] {
        match &self.storage {
            ActionListStorage::Empty => &[],
            ActionListStorage::One(actions) => actions,
            ActionListStorage::Many(actions) => actions,
        }
    }

    /// Returns the number of actions.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.actions().len()
    }

    /// Returns true if there are no actions.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        matches!(self.storage, ActionListStorage::Empty)
    }
}

/// Result of applying one external action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The action succeeded without a more specific payload.
    Applied,
    /// A player was added.
    PlayerAdded(PlayerId),
    /// Turn order was decided and this player starts.
    TurnOrderDecided(PlayerId),
    /// An object was created.
    ObjectCreated(ObjectId),
    /// A step advanced.
    StepAdvanced(Step),
    /// Priority was passed or a stack object resolved.
    Priority(PriorityOutcome),
    /// A stack entry was created.
    StackEntryAdded(StackEntryId),
    /// Multiple stack entries were created.
    StackEntriesAdded(Vec<StackEntryId>),
    /// A triggered ability definition was registered.
    TriggerRegistered(TriggerId),
    /// A replacement/prevention effect definition was registered.
    ReplacementEffectRegistered(ReplacementEffectId),
    /// A continuous effect definition was registered.
    ContinuousEffectRegistered(ContinuousEffectId),
    /// An activated ability definition was registered.
    ActivatedAbilityRegistered(ActivatedAbilityId),
    /// An activation cost modifier was registered.
    CostModifierRegistered(CostModifierId),
    /// A targeting or combat restriction was registered.
    RestrictionRegistered(RestrictionId),
    /// Combat damage was assigned and dealt.
    CombatDamageAssigned(Vec<CombatDamageRecord>),
    /// State-based actions were checked.
    StateBasedActions(StateBasedActionReport),
    /// A duration marker was added.
    DurationMarkerAdded(DurationMarkerId),
    /// The action was rejected.
    Failed(StateError),
}

/// Returns the currently legal external actions in deterministic order.
#[must_use]
#[inline]
pub fn legal_actions(state: &GameState) -> ActionList {
    if !state.pending_triggers().is_empty() {
        return ActionList::single(Action::PutPendingTriggeredAbilitiesOnStack);
    }
    if let Some(player) = state.priority_player() {
        return ActionList::single(Action::PassPriority { player });
    }
    ActionList::empty()
}

/// Applies one external action through the kernel boundary.
#[inline]
pub fn apply(state: &mut GameState, action: Action) -> Outcome {
    if let Action::PassPriority { player } = action {
        return apply_pass_priority(state, player);
    }
    apply_fallback(state, action)
}

#[inline(always)]
fn apply_pass_priority(state: &mut GameState, player: PlayerId) -> Outcome {
    if state.pending_triggers.is_empty() && state.priority_player == Some(player) {
        let pass_count = state.priority_pass_count + 1;
        let player_count = state.players.len() as u32;
        if pass_count < player_count {
            if let Ok(next) = state.next_player_after(player) {
                state.priority_pass_count = pass_count;
                state.priority_player = Some(next);
                return Outcome::Priority(PriorityOutcome::PassedTo(next));
            }
        }
    }
    match state.pass_priority(player) {
        Ok(outcome) => Outcome::Priority(outcome),
        Err(error) => Outcome::Failed(error),
    }
}

#[inline(never)]
fn apply_fallback(state: &mut GameState, action: Action) -> Outcome {
    match action {
        Action::SetSeed { seed } => {
            state.set_seed(seed);
            Outcome::Applied
        }
        Action::AddPlayer => Outcome::PlayerAdded(state.add_player()),
        Action::DecideTurnOrder => match state.decide_turn_order() {
            Ok(player) => Outcome::TurnOrderDecided(player),
            Err(error) => Outcome::Failed(error),
        },
        Action::SetTurnOrder { order } => match state.set_turn_order(order) {
            Ok(player) => Outcome::TurnOrderDecided(player),
            Err(error) => Outcome::Failed(error),
        },
        Action::DrawOpeningHands => match state.draw_opening_hands() {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::TakeMulligan { player } => match state.take_mulligan(player) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::KeepOpeningHand { player, bottom } => {
            match state.keep_opening_hand(player, &bottom) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetPlayerMaxHandSize {
            player,
            max_hand_size,
        } => match state.set_player_max_hand_size(player, max_hand_size) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::SetPlayerLife { player, life } => match state.set_player_life(player, life) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::LoseLife { player, amount } => match state.lose_life(player, amount) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::GainLife { player, amount } => match state.gain_life(player, amount) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::DrawCards { player, count } => match state.draw_cards(player, count) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::ShuffleLibrary { player } => {
            match state.shuffle_zone(ZoneId::new(Some(player), ZoneKind::Library)) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::PutObjectOnTopOfLibrary { player, object } => {
            match state.put_object_on_top_of_library(player, object) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::AddPoisonCounters { player, amount } => {
            match state.add_poison_counters(player, amount) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::AddManaToPool { player, mana } => match state.add_mana_to_pool(player, mana) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::ClearManaPool { player } => match state.clear_mana_pool(player) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::PayMana { player, cost, plan } => match state.pay_mana(player, cost, plan) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::Scry {
            player,
            count,
            bottom,
        } => match state.scry(player, count, &bottom) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::Surveil {
            player,
            count,
            graveyard,
        } => match state.surveil(player, count, &graveyard) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::Cycle {
            player,
            object,
            cost,
            payment,
        } => match state.cycle(player, object, cost, payment) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::AttachObject { attachment, target } => {
            match state.attach_object(attachment, target) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::Equip {
            player,
            equipment,
            target,
            cost,
            payment,
        } => match state.equip(player, equipment, target, cost, payment) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::SetObjectLoyalty { object, loyalty } => {
            match state.set_object_loyalty(object, loyalty) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetObjectColorIdentity { object, colors } => {
            match state.set_object_color_identity(object, colors) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::DesignateCommander {
            object,
            color_identity,
        } => match state.designate_commander(object, color_identity) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::RecordCommanderCast { object } => match state.record_commander_cast(object) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::ChooseCommanderZone { player, object } => {
            match state.choose_commander_zone(player, object) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::ValidateCommanderColorIdentity { player, objects } => {
            match state.validate_commander_color_identity(player, &objects) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::AddObjectCounters {
            object,
            kind,
            amount,
        } => match state.add_object_counters(object, kind, amount) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::RemoveObjectCounters {
            object,
            kind,
            amount,
        } => match state.remove_object_counters(object, kind, amount) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::RegisterActivatedAbility { definition } => {
            match state.register_activated_ability(definition) {
                Ok(ability) => Outcome::ActivatedAbilityRegistered(ability),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RegisterCostModifier { definition } => {
            match state.register_cost_modifier(definition) {
                Ok(modifier) => Outcome::CostModifierRegistered(modifier),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RegisterRestriction { definition } => {
            match state.register_restriction(definition) {
                Ok(restriction) => Outcome::RestrictionRegistered(restriction),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::ActivateAbility {
            player,
            ability,
            payment,
        } => match state.activate_ability(player, ability, payment) {
            Ok(Some(entry)) => Outcome::StackEntryAdded(entry),
            Ok(None) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::SetBaseObjectCharacteristics { object, base } => {
            match state.set_base_object_characteristics(object, base) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetBaseCreatureCharacteristics { object, base } => {
            match state.set_base_creature_characteristics(object, base) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::ClearBaseCreatureCharacteristics { object } => {
            match state.clear_base_creature_characteristics(object) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetObjectTapped { object, tapped } => {
            match state.set_object_tapped(object, tapped) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::MarkDamageOnObject { object, amount } => {
            match state.mark_damage_on_object(object, amount) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::DealDamage {
            source,
            target,
            amount,
        } => match state.deal_noncombat_damage(source, target, amount) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::CheckStateBasedActions => match state.check_state_based_actions() {
            Ok(report) => Outcome::StateBasedActions(report),
            Err(error) => Outcome::Failed(error),
        },
        Action::StartTurn { active_player } => match state.start_turn(active_player) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::AdvanceStep => match state.advance_step() {
            Ok(step) => Outcome::StepAdvanced(step),
            Err(error) => Outcome::Failed(error),
        },
        Action::PassPriority { player } => apply_pass_priority(state, player),
        Action::CastSpell {
            player,
            object,
            request,
        } => match state.cast_spell(player, object, request) {
            Ok(entry) => Outcome::StackEntryAdded(entry),
            Err(error) => Outcome::Failed(error),
        },
        Action::PlayLand { player, object } => match state.play_land(player, object) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::CounterStackEntry { entry } => match state.counter_stack_entry(entry) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::DestroyPermanent { object } => match state.destroy_permanent(object, true) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::PutSpellOnStack {
            player,
            object,
            kind,
            hold_priority,
        } => match state.put_spell_on_stack(player, object, kind, hold_priority) {
            Ok(entry) => Outcome::StackEntryAdded(entry),
            Err(error) => Outcome::Failed(error),
        },
        Action::PutAbilityOnStack {
            player,
            kind,
            hold_priority,
        } => match state.put_ability_on_stack(player, kind, hold_priority) {
            Ok(entry) => Outcome::StackEntryAdded(entry),
            Err(error) => Outcome::Failed(error),
        },
        Action::PutSimultaneousAbilitiesApnap { abilities, kind } => {
            match state.put_simultaneous_abilities_apnap(&abilities, kind) {
                Ok(entries) => Outcome::StackEntriesAdded(entries),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RegisterTriggeredAbility { definition } => {
            match state.register_triggered_ability(definition) {
                Ok(trigger) => Outcome::TriggerRegistered(trigger),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::PutPendingTriggeredAbilitiesOnStack => {
            match state.put_pending_triggered_abilities_on_stack() {
                Ok(entries) => Outcome::StackEntriesAdded(entries),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RegisterReplacementEffect { definition } => {
            match state.register_replacement_effect(definition) {
                Ok(replacement) => Outcome::ReplacementEffectRegistered(replacement),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetReplacementChoiceOrder { chooser, order } => {
            match state.set_replacement_choice_order(chooser, order) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RegisterContinuousEffect { definition } => {
            match state.register_continuous_effect(definition) {
                Ok(effect) => Outcome::ContinuousEffectRegistered(effect),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::SetAttackersDeclaredThisCombat { attackers_declared } => {
            state.set_attackers_declared_this_combat(attackers_declared);
            Outcome::Applied
        }
        Action::DeclareAttackers { player, attacks } => {
            match state.declare_attackers(player, &attacks) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::DeclareBlockers {
            defending_player,
            blocks,
        } => match state.declare_blockers(defending_player, &blocks) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::AssignCombatDamage { assignments } => {
            match state.assign_combat_damage(&assignments) {
                Ok(records) => Outcome::CombatDamageAssigned(records),
                Err(error) => Outcome::Failed(error),
            }
        }
        Action::RequestCleanupPriority => {
            state.request_cleanup_priority();
            Outcome::Applied
        }
        Action::AddDurationMarker { duration } => {
            Outcome::DurationMarkerAdded(state.add_duration_marker(duration))
        }
        Action::CreateObject {
            card,
            owner,
            controller,
            zone,
        } => match state.create_object(card, owner, controller, zone) {
            Ok(object) => Outcome::ObjectCreated(object),
            Err(error) => Outcome::Failed(error),
        },
        Action::MoveObject { object, to } => match state.move_object(object, to) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::CreateToken {
            card,
            owner,
            controller,
            base_object,
            base,
        } => match state.create_token(card, owner, controller, base_object, base) {
            Ok(object) => Outcome::ObjectCreated(object),
            Err(error) => Outcome::Failed(error),
        },
        Action::CreatePermanentCopy {
            source,
            owner,
            controller,
            token,
        } => match state.create_permanent_copy(source, owner, controller, token) {
            Ok(object) => Outcome::ObjectCreated(object),
            Err(error) => Outcome::Failed(error),
        },
        Action::CopyStackEntry { player, entry } => match state.copy_stack_entry(player, entry) {
            Ok(copy) => Outcome::StackEntryAdded(copy),
            Err(error) => Outcome::Failed(error),
        },
        Action::RevealObjects { objects } => match state.reveal_objects(&objects) {
            Ok(()) => Outcome::Applied,
            Err(error) => Outcome::Failed(error),
        },
        Action::DestroyPermanentWithoutRegeneration { object } => {
            match state.destroy_permanent(object, false) {
                Ok(()) => Outcome::Applied,
                Err(error) => Outcome::Failed(error),
            }
        }
    }
}

/// Maximum number of event records retained for the current turn.
pub const EVENT_RING_CAPACITY: usize = 1024;

const EVENT_DEEP_CLONE_LIMIT: usize = 16;

/// A replay cursor into the current-turn event ring.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EventCursor {
    turn: u32,
    next_sequence: u64,
}

impl EventCursor {
    /// Returns the turn this cursor belongs to.
    #[must_use]
    pub const fn turn(self) -> u32 {
        self.turn
    }

    /// Returns the first event sequence not yet consumed by this cursor.
    #[must_use]
    pub const fn next_sequence(self) -> u64 {
        self.next_sequence
    }
}

/// Error returned when replaying from an event cursor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EventReplayError {
    /// The cursor was created for a different turn.
    CursorTurnMismatch {
        /// Cursor turn.
        cursor_turn: u32,
        /// Current game turn.
        current_turn: u32,
    },
    /// The cursor points before the oldest retained event.
    CursorTooOld {
        /// Requested sequence.
        requested: u64,
        /// Oldest retained sequence.
        oldest_retained: u64,
    },
    /// The cursor points after the next event sequence.
    CursorInFuture {
        /// Requested sequence.
        requested: u64,
        /// Next sequence that will be assigned.
        next_sequence: u64,
    },
}

/// One typed mutation event emitted by the rules kernel.
///
/// T2.1 keeps events as inert data. Later trigger and replacement systems can
/// subscribe to these variants without closures or card-specific engine code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GameEvent {
    /// The deterministic seed changed.
    SeedSet {
        /// New seed value.
        seed: u64,
    },
    /// A player and that player's zones were added.
    PlayerAdded {
        /// Added player.
        player: PlayerId,
    },
    /// The starting player was selected.
    TurnOrderDecided {
        /// Player selected to start the game.
        starting_player: PlayerId,
    },
    /// Opening hands were drawn for all players.
    OpeningHandsDrawn,
    /// A player took a London mulligan.
    MulliganTaken {
        /// Player taking the mulligan.
        player: PlayerId,
        /// Total mulligans taken by that player after this event.
        mulligans_taken: u32,
    },
    /// A player kept an opening hand.
    OpeningHandKept {
        /// Player keeping the hand.
        player: PlayerId,
    },
    /// One opening-hand card was put on the bottom of its owner's library.
    OpeningHandCardBottomed {
        /// Player bottoming the card.
        player: PlayerId,
        /// Object put on the bottom.
        object: ObjectId,
    },
    /// A player's maximum hand size changed.
    PlayerMaxHandSizeSet {
        /// Player whose hand-size limit changed.
        player: PlayerId,
        /// New maximum hand size.
        max_hand_size: u32,
    },
    /// A player's life total was set directly.
    LifeTotalSet {
        /// Player whose life changed.
        player: PlayerId,
        /// New life total.
        life: i32,
    },
    /// A player lost life.
    LifeLost {
        /// Player losing life.
        player: PlayerId,
        /// Amount of life lost.
        amount: u32,
        /// Resulting life total.
        life: i32,
    },
    /// A player gained life.
    LifeGained {
        /// Player gaining life.
        player: PlayerId,
        /// Amount of life gained.
        amount: u32,
        /// Resulting life total.
        life: i32,
    },
    /// Poison counters were added to a player.
    PoisonCountersAdded {
        /// Player receiving poison counters.
        player: PlayerId,
        /// Number of counters added.
        amount: u32,
        /// Resulting poison-counter total.
        poison: u32,
    },
    /// A player's mana pool changed to a known value.
    ManaPoolChanged {
        /// Player whose pool changed.
        player: PlayerId,
        /// New mana pool.
        mana_pool: ManaPool,
    },
    /// A player paid mana.
    ManaPaid {
        /// Player paying mana.
        player: PlayerId,
        /// Canonical payment plan consumed.
        payment: PaymentPlan,
        /// Resulting mana pool.
        mana_pool: ManaPool,
    },
    /// One object was created in a zone.
    ObjectCreated {
        /// Created object.
        object: ObjectId,
        /// Printed card definition.
        card: CardId,
        /// Object owner.
        owner: PlayerId,
        /// Object controller.
        controller: PlayerId,
        /// Zone that received the object.
        zone: ZoneId,
    },
    /// One object moved between zones.
    ObjectMoved {
        /// Moved object.
        object: ObjectId,
        /// Source zone.
        from: ZoneId,
        /// Destination zone.
        to: ZoneId,
    },
    /// A zone was shuffled.
    ZoneShuffled {
        /// Shuffled zone.
        zone: ZoneId,
    },
    /// Base creature characteristics were set.
    BaseCreatureCharacteristicsSet {
        /// Updated object.
        object: ObjectId,
        /// New base characteristics.
        base: BaseCreatureCharacteristics,
    },
    /// Base creature characteristics were cleared.
    BaseCreatureCharacteristicsCleared {
        /// Updated object.
        object: ObjectId,
    },
    /// An object's tapped status changed.
    ObjectTapped {
        /// Updated object.
        object: ObjectId,
        /// New tapped status.
        tapped: bool,
    },
    /// Damage was marked on an object.
    DamageMarked {
        /// Damaged object.
        object: ObjectId,
        /// Newly marked damage.
        amount: u32,
        /// Total marked damage after this event.
        total_damage: u32,
    },
    /// A turn began.
    TurnStarted {
        /// New turn number.
        turn: u32,
        /// Active player for the turn.
        active_player: PlayerId,
    },
    /// A step ended.
    StepEnded {
        /// Step that ended.
        step: Step,
    },
    /// A step began.
    StepBegan {
        /// Step that began.
        step: Step,
    },
    /// Priority passed from one player.
    PriorityPassed {
        /// Player who passed.
        player: PlayerId,
    },
    /// The priority holder changed.
    PriorityChanged {
        /// New priority holder, or none when no player has priority.
        player: Option<PlayerId>,
    },
    /// A stack entry was added.
    StackEntryAdded {
        /// Added stack entry.
        entry: StackEntryId,
        /// Controller of the stack entry.
        controller: PlayerId,
        /// Spell object, if this entry has one.
        object: Option<ObjectId>,
        /// Stack-entry kind.
        kind: StackObjectKind,
    },
    /// A stack entry resolved or was countered on resolution.
    StackEntryResolved {
        /// Resolved stack entry.
        entry: StackEntryId,
        /// Resolution result.
        outcome: ResolutionOutcome,
    },
    /// Attackers were declared.
    AttackersDeclared {
        /// Attacking player.
        player: PlayerId,
        /// Number of declared attackers.
        count: u32,
    },
    /// One attacker was declared.
    AttackDeclared {
        /// Attacking object.
        attacker: ObjectId,
        /// Player being attacked.
        defending_player: PlayerId,
    },
    /// Blockers were declared.
    BlockersDeclared {
        /// Defending player.
        defending_player: PlayerId,
        /// Number of declared blockers.
        count: u32,
    },
    /// One blocker was declared.
    BlockDeclared {
        /// Blocking object.
        blocker: ObjectId,
        /// Attacking object being blocked.
        attacker: ObjectId,
    },
    /// Combat damage was dealt.
    CombatDamageDealt {
        /// Combat damage record.
        record: CombatDamageRecord,
    },
    /// A player lost due to a state-based action.
    PlayerLostByStateBasedAction {
        /// Player who lost.
        player: PlayerId,
        /// State-based-action reason.
        kind: StateBasedActionKind,
    },
    /// A permanent moved to a graveyard due to a state-based action.
    PermanentMovedByStateBasedAction {
        /// Object moved.
        object: ObjectId,
        /// State-based-action reason.
        kind: StateBasedActionKind,
    },
    /// The game outcome changed.
    GameOutcomeChanged {
        /// New outcome.
        outcome: GameOutcome,
    },
    /// Cleanup priority was requested.
    CleanupPriorityRequested,
    /// A duration marker was added.
    DurationMarkerAdded {
        /// Added marker.
        marker: DurationMarkerId,
        /// Marker duration.
        duration: EffectDuration,
    },
    /// Duration markers expired.
    DurationMarkersExpired {
        /// Expired duration kind.
        duration: EffectDuration,
        /// Number of markers expired.
        count: u32,
    },
    /// Cleanup actions were performed.
    CleanupPerformed {
        /// Cleanup summary.
        report: CleanupReport,
    },
    /// All mana pools were cleared.
    ManaPoolsCleared,
    /// A player tried to draw from an empty library.
    EmptyLibraryDraw {
        /// Player who tried to draw.
        player: PlayerId,
    },
    /// A triggered ability definition was registered.
    TriggeredAbilityRegistered {
        /// Registered trigger.
        trigger: TriggerId,
        /// Trigger controller.
        controller: PlayerId,
        /// Optional trigger source object.
        source: Option<ObjectId>,
        /// Subscribed event kind.
        event_kind: GameEventKind,
        /// Whether this is a delayed-once trigger.
        duration: TriggerDuration,
    },
    /// A triggered ability was queued by a matching event.
    TriggeredAbilityQueued {
        /// Queued trigger.
        trigger: TriggerId,
        /// Trigger controller.
        controller: PlayerId,
        /// Sequence number of the event that caused the trigger.
        event_sequence: u64,
    },
    /// A queued triggered ability was put onto the stack.
    TriggeredAbilityPutOnStack {
        /// Queued trigger.
        trigger: TriggerId,
        /// New stack entry.
        entry: StackEntryId,
        /// Trigger controller.
        controller: PlayerId,
    },
    /// A replacement/prevention effect definition was registered.
    ReplacementEffectRegistered {
        /// Registered replacement/prevention effect.
        replacement: ReplacementEffectId,
        /// Effect controller.
        controller: PlayerId,
        /// Optional effect source object.
        source: Option<ObjectId>,
        /// Effect operation.
        operation: ReplacementOperation,
        /// Effect duration.
        duration: ReplacementDuration,
        /// Whether this is a self-replacement effect.
        self_replacement: bool,
    },
    /// A player's deterministic replacement ordering preference changed.
    ReplacementChoiceOrderSet {
        /// Player making replacement ordering choices.
        chooser: PlayerId,
        /// Number of ordered effect IDs stored.
        count: u32,
    },
    /// A replacement/prevention effect modified a damage event.
    ReplacementEffectApplied {
        /// Effect that applied.
        replacement: ReplacementEffectId,
        /// Player whose affected-object choice selected this effect.
        chooser: PlayerId,
        /// Damage source, if known.
        source: Option<ObjectId>,
        /// Damage target.
        target: CombatDamageTarget,
        /// Effect operation.
        operation: ReplacementOperation,
        /// Damage amount before applying this effect.
        original_amount: u32,
        /// Damage amount after applying this effect.
        resulting_amount: u32,
    },
    /// A continuous effect definition was registered.
    ContinuousEffectRegistered {
        /// Registered continuous effect.
        effect: ContinuousEffectId,
        /// Effect controller.
        controller: PlayerId,
        /// Optional effect source object.
        source: Option<ObjectId>,
        /// Target filter.
        target: ContinuousEffectTarget,
        /// Effect operation.
        operation: ContinuousEffectOperation,
        /// CR 613 layer.
        layer: ContinuousEffectLayer,
        /// Effect timestamp.
        timestamp: u64,
    },
    /// An object's loyalty value changed.
    ObjectLoyaltySet {
        /// Updated object.
        object: ObjectId,
        /// New loyalty value, or none if loyalty tracking was cleared.
        loyalty: Option<i32>,
    },
    /// An activated ability definition was registered.
    ActivatedAbilityRegistered {
        /// Registered activated ability.
        ability: ActivatedAbilityId,
        /// Registered controller fallback.
        controller: PlayerId,
        /// Optional source object.
        source: Option<ObjectId>,
        /// Whether this ability resolves without using the stack.
        mana_ability: bool,
    },
    /// An activated ability cost modifier was registered.
    CostModifierRegistered {
        /// Registered modifier.
        modifier: CostModifierId,
        /// Modifier controller.
        controller: PlayerId,
        /// Optional source object.
        source: Option<ObjectId>,
        /// Modifier operation.
        operation: CostModifierOperation,
    },
    /// An activated ability was activated.
    ActivatedAbilityActivated {
        /// Activated ability.
        ability: ActivatedAbilityId,
        /// Activating player.
        player: PlayerId,
        /// Optional source object.
        source: Option<ObjectId>,
        /// Whether this ability resolved without using the stack.
        mana_ability: bool,
    },
    /// An activated ability resolved.
    ActivatedAbilityResolved {
        /// Resolved ability.
        ability: ActivatedAbilityId,
        /// Controller at activation or resolution.
        player: PlayerId,
        /// Optional source object.
        source: Option<ObjectId>,
        /// Resolved effect.
        effect: ActivatedAbilityEffect,
    },
    /// A targeting or combat restriction definition was registered.
    RestrictionRegistered {
        /// Registered restriction.
        restriction: RestrictionId,
        /// Restriction controller.
        controller: PlayerId,
        /// Optional restriction source object.
        source: Option<ObjectId>,
        /// Restriction effect.
        effect: RestrictionEffect,
    },
    /// An object's counter total changed.
    ObjectCountersChanged {
        /// Updated object.
        object: ObjectId,
        /// Counter kind changed.
        kind: CounterKind,
        /// New total for that counter kind.
        count: u32,
    },
    /// A token object was created.
    TokenCreated {
        /// Created token object.
        object: ObjectId,
        /// Token card definition.
        card: CardId,
        /// Token owner.
        owner: PlayerId,
        /// Token controller.
        controller: PlayerId,
    },
    /// A permanent-copy object was created.
    ObjectCopyCreated {
        /// Created copy object.
        object: ObjectId,
        /// Source object copied.
        source: ObjectId,
        /// Whether the copy object is also a token.
        token: bool,
    },
    /// A stack entry was copied.
    StackEntryCopied {
        /// Source stack entry.
        source: StackEntryId,
        /// Created stack-entry copy.
        copy: StackEntryId,
        /// Controller of the copy.
        controller: PlayerId,
    },
    /// A token or copy object ceased to exist.
    ObjectCeasedToExist {
        /// Object that ceased to exist.
        object: ObjectId,
        /// State-based-action reason.
        kind: StateBasedActionKind,
    },
    /// Matching +1/+1 and -1/-1 counters were cancelled.
    CounterPairCancelled {
        /// Object whose counters were cancelled.
        object: ObjectId,
        /// Number of each counter removed.
        amount: u32,
    },
    /// An object's Commander color identity metadata changed.
    ObjectColorIdentitySet {
        /// Updated object.
        object: ObjectId,
        /// New color identity.
        colors: ObjectColors,
    },
    /// An object was designated as a commander.
    CommanderDesignated {
        /// Updated object.
        object: ObjectId,
        /// Commander owner.
        player: PlayerId,
        /// Commander color identity.
        color_identity: ObjectColors,
    },
    /// A commander cast was recorded for tax tracking.
    CommanderCastRecorded {
        /// Commander object.
        object: ObjectId,
        /// Commander owner.
        player: PlayerId,
        /// Cast count after this event.
        cast_count: u32,
        /// Current Commander tax after this event.
        tax: ManaCost,
    },
    /// Commander color identity validation completed.
    CommanderColorIdentityValidated {
        /// Player whose commander identity was used.
        player: PlayerId,
        /// Number of objects validated.
        count: u32,
        /// Allowed identity.
        allowed: ObjectColors,
    },
    /// A keyword action inspected and manipulated a library.
    LibraryManipulated {
        /// Player whose library was manipulated.
        player: PlayerId,
        /// Operation performed.
        operation: LibraryManipulation,
        /// Number of top cards inspected.
        count: u32,
        /// Number of cards moved by the operation.
        moved: u32,
    },
    /// An object's attachment pointer changed.
    ObjectAttached {
        /// Object that became attached or unattached.
        attachment: ObjectId,
        /// New attachment target, if any.
        target: Option<ObjectId>,
    },
    /// Base printed card types and colors were set.
    BaseObjectCharacteristicsSet {
        /// Updated object.
        object: ObjectId,
        /// New base characteristics.
        base: BaseObjectCharacteristics,
    },
    /// Noncombat damage was dealt after all replacement effects.
    NoncombatDamageDealt {
        /// Damage source, when the effect has a represented source object.
        source: Option<ObjectId>,
        /// Player or object that received damage.
        target: CombatDamageTarget,
        /// Final damage amount dealt.
        amount: u32,
    },
    /// One object moved from a library to its owner's hand as a card draw.
    CardDrawn {
        /// Player who drew the card.
        player: PlayerId,
        /// Object drawn.
        object: ObjectId,
    },
    /// One object's identity was publicly revealed.
    ObjectRevealed {
        /// Revealed object.
        object: ObjectId,
    },
}

impl GameEvent {
    const fn canonical_code(self) -> u8 {
        match self {
            Self::SeedSet { .. } => 0,
            Self::PlayerAdded { .. } => 1,
            Self::TurnOrderDecided { .. } => 2,
            Self::OpeningHandsDrawn => 3,
            Self::MulliganTaken { .. } => 4,
            Self::OpeningHandKept { .. } => 5,
            Self::OpeningHandCardBottomed { .. } => 6,
            Self::PlayerMaxHandSizeSet { .. } => 7,
            Self::LifeTotalSet { .. } => 8,
            Self::LifeLost { .. } => 9,
            Self::LifeGained { .. } => 10,
            Self::PoisonCountersAdded { .. } => 11,
            Self::ManaPoolChanged { .. } => 12,
            Self::ManaPaid { .. } => 13,
            Self::ObjectCreated { .. } => 14,
            Self::ObjectMoved { .. } => 15,
            Self::ZoneShuffled { .. } => 16,
            Self::BaseCreatureCharacteristicsSet { .. } => 17,
            Self::BaseCreatureCharacteristicsCleared { .. } => 18,
            Self::ObjectTapped { .. } => 19,
            Self::DamageMarked { .. } => 20,
            Self::TurnStarted { .. } => 21,
            Self::StepEnded { .. } => 22,
            Self::StepBegan { .. } => 23,
            Self::PriorityPassed { .. } => 24,
            Self::PriorityChanged { .. } => 25,
            Self::StackEntryAdded { .. } => 26,
            Self::StackEntryResolved { .. } => 27,
            Self::AttackersDeclared { .. } => 28,
            Self::AttackDeclared { .. } => 29,
            Self::BlockersDeclared { .. } => 30,
            Self::BlockDeclared { .. } => 31,
            Self::CombatDamageDealt { .. } => 32,
            Self::PlayerLostByStateBasedAction { .. } => 33,
            Self::PermanentMovedByStateBasedAction { .. } => 34,
            Self::GameOutcomeChanged { .. } => 35,
            Self::CleanupPriorityRequested => 36,
            Self::DurationMarkerAdded { .. } => 37,
            Self::DurationMarkersExpired { .. } => 38,
            Self::CleanupPerformed { .. } => 39,
            Self::ManaPoolsCleared => 40,
            Self::EmptyLibraryDraw { .. } => 41,
            Self::TriggeredAbilityRegistered { .. } => 42,
            Self::TriggeredAbilityQueued { .. } => 43,
            Self::TriggeredAbilityPutOnStack { .. } => 44,
            Self::ReplacementEffectRegistered { .. } => 45,
            Self::ReplacementChoiceOrderSet { .. } => 46,
            Self::ReplacementEffectApplied { .. } => 47,
            Self::ContinuousEffectRegistered { .. } => 48,
            Self::ObjectLoyaltySet { .. } => 49,
            Self::ActivatedAbilityRegistered { .. } => 50,
            Self::CostModifierRegistered { .. } => 51,
            Self::ActivatedAbilityActivated { .. } => 52,
            Self::ActivatedAbilityResolved { .. } => 53,
            Self::RestrictionRegistered { .. } => 54,
            Self::ObjectCountersChanged { .. } => 55,
            Self::TokenCreated { .. } => 56,
            Self::ObjectCopyCreated { .. } => 57,
            Self::StackEntryCopied { .. } => 58,
            Self::ObjectCeasedToExist { .. } => 59,
            Self::CounterPairCancelled { .. } => 60,
            Self::ObjectColorIdentitySet { .. } => 61,
            Self::CommanderDesignated { .. } => 62,
            Self::CommanderCastRecorded { .. } => 63,
            Self::CommanderColorIdentityValidated { .. } => 64,
            Self::LibraryManipulated { .. } => 65,
            Self::ObjectAttached { .. } => 66,
            Self::BaseObjectCharacteristicsSet { .. } => 67,
            Self::NoncombatDamageDealt { .. } => 68,
            Self::CardDrawn { .. } => 69,
            Self::ObjectRevealed { .. } => 70,
        }
    }

    /// Returns the coarse event kind used by trigger subscription tables.
    #[must_use]
    pub const fn kind(self) -> GameEventKind {
        match self {
            Self::SeedSet { .. } => GameEventKind::SeedSet,
            Self::PlayerAdded { .. } => GameEventKind::PlayerAdded,
            Self::TurnOrderDecided { .. } => GameEventKind::TurnOrderDecided,
            Self::OpeningHandsDrawn => GameEventKind::OpeningHandsDrawn,
            Self::MulliganTaken { .. } => GameEventKind::MulliganTaken,
            Self::OpeningHandKept { .. } => GameEventKind::OpeningHandKept,
            Self::OpeningHandCardBottomed { .. } => GameEventKind::OpeningHandCardBottomed,
            Self::PlayerMaxHandSizeSet { .. } => GameEventKind::PlayerMaxHandSizeSet,
            Self::LifeTotalSet { .. } => GameEventKind::LifeTotalSet,
            Self::LifeLost { .. } => GameEventKind::LifeLost,
            Self::LifeGained { .. } => GameEventKind::LifeGained,
            Self::PoisonCountersAdded { .. } => GameEventKind::PoisonCountersAdded,
            Self::ManaPoolChanged { .. } => GameEventKind::ManaPoolChanged,
            Self::ManaPaid { .. } => GameEventKind::ManaPaid,
            Self::ObjectCreated { .. } => GameEventKind::ObjectCreated,
            Self::ObjectMoved { .. } => GameEventKind::ObjectMoved,
            Self::ZoneShuffled { .. } => GameEventKind::ZoneShuffled,
            Self::BaseCreatureCharacteristicsSet { .. } => {
                GameEventKind::BaseCreatureCharacteristicsSet
            }
            Self::BaseCreatureCharacteristicsCleared { .. } => {
                GameEventKind::BaseCreatureCharacteristicsCleared
            }
            Self::ObjectTapped { .. } => GameEventKind::ObjectTapped,
            Self::DamageMarked { .. } => GameEventKind::DamageMarked,
            Self::TurnStarted { .. } => GameEventKind::TurnStarted,
            Self::StepEnded { .. } => GameEventKind::StepEnded,
            Self::StepBegan { .. } => GameEventKind::StepBegan,
            Self::PriorityPassed { .. } => GameEventKind::PriorityPassed,
            Self::PriorityChanged { .. } => GameEventKind::PriorityChanged,
            Self::StackEntryAdded { .. } => GameEventKind::StackEntryAdded,
            Self::StackEntryResolved { .. } => GameEventKind::StackEntryResolved,
            Self::AttackersDeclared { .. } => GameEventKind::AttackersDeclared,
            Self::AttackDeclared { .. } => GameEventKind::AttackDeclared,
            Self::BlockersDeclared { .. } => GameEventKind::BlockersDeclared,
            Self::BlockDeclared { .. } => GameEventKind::BlockDeclared,
            Self::CombatDamageDealt { .. } => GameEventKind::CombatDamageDealt,
            Self::PlayerLostByStateBasedAction { .. } => {
                GameEventKind::PlayerLostByStateBasedAction
            }
            Self::PermanentMovedByStateBasedAction { .. } => {
                GameEventKind::PermanentMovedByStateBasedAction
            }
            Self::GameOutcomeChanged { .. } => GameEventKind::GameOutcomeChanged,
            Self::CleanupPriorityRequested => GameEventKind::CleanupPriorityRequested,
            Self::DurationMarkerAdded { .. } => GameEventKind::DurationMarkerAdded,
            Self::DurationMarkersExpired { .. } => GameEventKind::DurationMarkersExpired,
            Self::CleanupPerformed { .. } => GameEventKind::CleanupPerformed,
            Self::ManaPoolsCleared => GameEventKind::ManaPoolsCleared,
            Self::EmptyLibraryDraw { .. } => GameEventKind::EmptyLibraryDraw,
            Self::TriggeredAbilityRegistered { .. } => GameEventKind::TriggeredAbilityRegistered,
            Self::TriggeredAbilityQueued { .. } => GameEventKind::TriggeredAbilityQueued,
            Self::TriggeredAbilityPutOnStack { .. } => GameEventKind::TriggeredAbilityPutOnStack,
            Self::ReplacementEffectRegistered { .. } => GameEventKind::ReplacementEffectRegistered,
            Self::ReplacementChoiceOrderSet { .. } => GameEventKind::ReplacementChoiceOrderSet,
            Self::ReplacementEffectApplied { .. } => GameEventKind::ReplacementEffectApplied,
            Self::ContinuousEffectRegistered { .. } => GameEventKind::ContinuousEffectRegistered,
            Self::ObjectLoyaltySet { .. } => GameEventKind::ObjectLoyaltySet,
            Self::ActivatedAbilityRegistered { .. } => GameEventKind::ActivatedAbilityRegistered,
            Self::CostModifierRegistered { .. } => GameEventKind::CostModifierRegistered,
            Self::ActivatedAbilityActivated { .. } => GameEventKind::ActivatedAbilityActivated,
            Self::ActivatedAbilityResolved { .. } => GameEventKind::ActivatedAbilityResolved,
            Self::RestrictionRegistered { .. } => GameEventKind::RestrictionRegistered,
            Self::ObjectCountersChanged { .. } => GameEventKind::ObjectCountersChanged,
            Self::TokenCreated { .. } => GameEventKind::TokenCreated,
            Self::ObjectCopyCreated { .. } => GameEventKind::ObjectCopyCreated,
            Self::StackEntryCopied { .. } => GameEventKind::StackEntryCopied,
            Self::ObjectCeasedToExist { .. } => GameEventKind::ObjectCeasedToExist,
            Self::CounterPairCancelled { .. } => GameEventKind::CounterPairCancelled,
            Self::ObjectColorIdentitySet { .. } => GameEventKind::ObjectColorIdentitySet,
            Self::CommanderDesignated { .. } => GameEventKind::CommanderDesignated,
            Self::CommanderCastRecorded { .. } => GameEventKind::CommanderCastRecorded,
            Self::CommanderColorIdentityValidated { .. } => {
                GameEventKind::CommanderColorIdentityValidated
            }
            Self::LibraryManipulated { .. } => GameEventKind::LibraryManipulated,
            Self::ObjectAttached { .. } => GameEventKind::ObjectAttached,
            Self::BaseObjectCharacteristicsSet { .. } => {
                GameEventKind::BaseObjectCharacteristicsSet
            }
            Self::NoncombatDamageDealt { .. } => GameEventKind::NoncombatDamageDealt,
            Self::CardDrawn { .. } => GameEventKind::CardDrawn,
            Self::ObjectRevealed { .. } => GameEventKind::ObjectRevealed,
        }
    }
}

/// One sequenced event in the current turn's event buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EventRecord {
    sequence: u64,
    turn: u32,
    event: GameEvent,
}

impl EventRecord {
    /// Returns the monotonic sequence number for this event.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns the turn number associated with this event.
    #[must_use]
    pub const fn turn(self) -> u32 {
        self.turn
    }

    /// Returns the typed event payload.
    #[must_use]
    pub const fn event(self) -> GameEvent {
        self.event
    }
}

/// One queued triggered ability waiting to be put on the stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PendingTriggeredAbility {
    trigger: TriggerId,
    controller: PlayerId,
    source: Option<ObjectId>,
    event_sequence: u64,
    event_turn: u32,
}

impl PendingTriggeredAbility {
    /// Returns the registered trigger definition.
    #[must_use]
    pub const fn trigger(self) -> TriggerId {
        self.trigger
    }

    /// Returns the ability controller.
    #[must_use]
    pub const fn controller(self) -> PlayerId {
        self.controller
    }

    /// Returns the source object, if this trigger has one.
    #[must_use]
    pub const fn source(self) -> Option<ObjectId> {
        self.source
    }

    /// Returns the sequence number of the event that queued this trigger.
    #[must_use]
    pub const fn event_sequence(self) -> u64 {
        self.event_sequence
    }

    /// Returns the turn number of the event that queued this trigger.
    #[must_use]
    pub const fn event_turn(self) -> u32 {
        self.event_turn
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TriggerSubscription {
    id: TriggerId,
    definition: TriggerDefinition,
    event_kind: GameEventKind,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ActivatedAbilitySubscription {
    id: ActivatedAbilityId,
    definition: ActivatedAbilityDefinition,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CostModifierSubscription {
    id: CostModifierId,
    definition: CostModifierDefinition,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RestrictionSubscription {
    id: RestrictionId,
    definition: RestrictionDefinition,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReplacementSubscription {
    id: ReplacementEffectId,
    definition: ReplacementDefinition,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ContinuousEffectSubscription {
    id: ContinuousEffectId,
    definition: ContinuousEffectDefinition,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DamageReplacementEvent {
    source: Option<ObjectId>,
    target: CombatDamageTarget,
    amount: u32,
    combat: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingStateBasedAction {
    PlayerLoses {
        player: PlayerId,
        kind: StateBasedActionKind,
    },
    MovePermanentToGraveyard {
        object: ObjectId,
        kind: StateBasedActionKind,
    },
    CeaseObject {
        object: ObjectId,
        kind: StateBasedActionKind,
    },
    CancelCounterPair {
        object: ObjectId,
        amount: u32,
    },
}

/// Complete T1 game state.
#[derive(Eq, PartialEq)]
pub struct GameState {
    seed: u64,
    rng_state: u64,
    turn_number: u32,
    outcome: GameOutcome,
    starting_player: Option<PlayerId>,
    opening_hands_drawn: bool,
    active_player: Option<PlayerId>,
    priority_player: Option<PlayerId>,
    priority_pass_count: u32,
    current_step: Option<Step>,
    cleanup_iteration: u32,
    cleanup_priority_requested: bool,
    cleanup_repeat_pending: bool,
    attackers_declared_this_combat: bool,
    last_cleanup_report: CleanupReport,
    // clone_surface: player scalar arena; bounded by game player count.
    players: Vec<PlayerState>,
    // clone_surface: explicit multiplayer turn ring; bounded by game player count.
    turn_order: Vec<PlayerId>,
    // clone_surface: object storage wrapper with one Copy-record arena.
    objects: ObjectArena,
    // clone_surface: fixed shared zones plus per-player zones; membership IDs live in Zone.
    zones: Arc<Vec<Zone>>,
    // clone_surface: sorted per-observer identities retained across hidden-zone moves.
    known_objects: Arc<Vec<KnownObject>>,
    next_duration_marker: u32,
    // clone_surface: duration markers are Copy records, bounded by active effects.
    duration_markers: Vec<DurationMarker>,
    next_stack_entry: u32,
    // clone_surface: stack entries are bounded by current stack depth.
    stack_entries: Vec<StackEntry>,
    // clone_surface: append-only resolution audit for deterministic replay diagnostics.
    resolution_log: Vec<ResolutionRecord>,
    next_trigger: u32,
    // clone_surface: data-only trigger definitions compiled from card IR.
    trigger_subscriptions: Vec<TriggerSubscription>,
    // clone_surface: Copy trigger instances waiting for the next priority window.
    pending_triggers: Vec<PendingTriggeredAbility>,
    next_activated_ability: u32,
    // clone_surface: data-only activated abilities compiled from card IR.
    activated_abilities: Vec<ActivatedAbilitySubscription>,
    next_cost_modifier: u32,
    // clone_surface: data-only activation cost adjustments.
    cost_modifiers: Vec<CostModifierSubscription>,
    next_restriction: u32,
    // clone_surface: data-only targeting/combat restrictions.
    restrictions: Vec<RestrictionSubscription>,
    // clone_surface: compact object counter totals keyed by object and kind.
    object_counters: Vec<ObjectCounterEntry>,
    // clone_surface: object IDs whose loyalty abilities were activated this turn.
    loyalty_activations_this_turn: Vec<ObjectId>,
    next_replacement: u32,
    // clone_surface: data-only replacement/prevention definitions compiled from card IR.
    replacement_effects: Vec<ReplacementSubscription>,
    // clone_surface: per-player replacement order preferences; bounded by active effects.
    replacement_choice_orders: Vec<ReplacementChoiceOrder>,
    next_continuous_effect: u32,
    // clone_surface: data-only continuous effects compiled from card IR.
    continuous_effects: Vec<ContinuousEffectSubscription>,
    deferred_priority_player: Option<PlayerId>,
    next_event_sequence: u64,
    // clone_surface: current-turn Copy event records for trigger/replay consumers.
    turn_events: Arc<Vec<EventRecord>>,
    // clone_surface: current combat wrapper; cleared between combats.
    combat: CombatState,
    // clone_surface: player IDs only, drained by state-based-action processing.
    empty_library_draws_since_sba: Vec<PlayerId>,
}

impl Clone for GameState {
    fn clone(&self) -> Self {
        Self {
            seed: self.seed,
            rng_state: self.rng_state,
            turn_number: self.turn_number,
            outcome: self.outcome,
            starting_player: self.starting_player,
            opening_hands_drawn: self.opening_hands_drawn,
            active_player: self.active_player,
            priority_player: self.priority_player,
            priority_pass_count: self.priority_pass_count,
            current_step: self.current_step,
            cleanup_iteration: self.cleanup_iteration,
            cleanup_priority_requested: self.cleanup_priority_requested,
            cleanup_repeat_pending: self.cleanup_repeat_pending,
            attackers_declared_this_combat: self.attackers_declared_this_combat,
            last_cleanup_report: self.last_cleanup_report,
            players: self.players.clone(),
            turn_order: self.turn_order.clone(),
            objects: self.objects.clone(),
            zones: Arc::clone(&self.zones),
            known_objects: Arc::clone(&self.known_objects),
            next_duration_marker: self.next_duration_marker,
            duration_markers: self.duration_markers.clone(),
            next_stack_entry: self.next_stack_entry,
            stack_entries: self.stack_entries.clone(),
            resolution_log: self.resolution_log.clone(),
            next_trigger: self.next_trigger,
            trigger_subscriptions: self.trigger_subscriptions.clone(),
            pending_triggers: self.pending_triggers.clone(),
            next_activated_ability: self.next_activated_ability,
            activated_abilities: self.activated_abilities.clone(),
            next_cost_modifier: self.next_cost_modifier,
            cost_modifiers: self.cost_modifiers.clone(),
            next_restriction: self.next_restriction,
            restrictions: self.restrictions.clone(),
            object_counters: self.object_counters.clone(),
            loyalty_activations_this_turn: self.loyalty_activations_this_turn.clone(),
            next_replacement: self.next_replacement,
            replacement_effects: self.replacement_effects.clone(),
            replacement_choice_orders: self.replacement_choice_orders.clone(),
            next_continuous_effect: self.next_continuous_effect,
            continuous_effects: self.continuous_effects.clone(),
            deferred_priority_player: self.deferred_priority_player,
            next_event_sequence: self.next_event_sequence,
            turn_events: Arc::clone(&self.turn_events),
            combat: self.combat.clone(),
            empty_library_draws_since_sba: self.empty_library_draws_since_sba.clone(),
        }
    }
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
            rng_state: 0,
            turn_number: 0,
            outcome: GameOutcome::InProgress,
            starting_player: None,
            opening_hands_drawn: false,
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
            turn_order: Vec::new(),
            objects: ObjectArena::default(),
            zones: Arc::new(vec![
                Zone {
                    id: ZoneId::new(None, ZoneKind::Battlefield),
                    objects: Arc::new(Vec::new()),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Exile),
                    objects: Arc::new(Vec::new()),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Stack),
                    objects: Arc::new(Vec::new()),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Command),
                    objects: Arc::new(Vec::new()),
                },
                Zone {
                    id: ZoneId::new(None, ZoneKind::Ceased),
                    objects: Arc::new(Vec::new()),
                },
            ]),
            known_objects: Arc::new(Vec::new()),
            next_duration_marker: 0,
            duration_markers: Vec::new(),
            next_stack_entry: 0,
            stack_entries: Vec::new(),
            resolution_log: Vec::new(),
            next_trigger: 0,
            trigger_subscriptions: Vec::new(),
            pending_triggers: Vec::new(),
            next_activated_ability: 0,
            activated_abilities: Vec::new(),
            next_cost_modifier: 0,
            cost_modifiers: Vec::new(),
            next_restriction: 0,
            restrictions: Vec::new(),
            object_counters: Vec::new(),
            loyalty_activations_this_turn: Vec::new(),
            next_replacement: 0,
            replacement_effects: Vec::new(),
            replacement_choice_orders: Vec::new(),
            next_continuous_effect: 0,
            continuous_effects: Vec::new(),
            deferred_priority_player: None,
            next_event_sequence: 0,
            turn_events: Arc::new(Vec::with_capacity(EVENT_DEEP_CLONE_LIMIT)),
            combat: CombatState::new(),
            empty_library_draws_since_sba: Vec::new(),
        }
    }

    /// Sets the deterministic game seed.
    fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
        self.rng_state = seed;
        self.emit_event(GameEvent::SeedSet { seed });
    }

    /// Returns the deterministic game seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the player chosen to take the first turn, if setup has decided one.
    #[must_use]
    pub const fn starting_player(&self) -> Option<PlayerId> {
        self.starting_player
    }

    /// Returns whether setup has drawn opening hands.
    #[must_use]
    pub const fn opening_hands_drawn(&self) -> bool {
        self.opening_hands_drawn
    }

    /// Returns the explicit multiplayer turn order, if setup has established one.
    #[must_use]
    pub fn turn_order(&self) -> &[PlayerId] {
        &self.turn_order
    }

    /// Returns the v1 range-of-influence policy.
    #[must_use]
    pub const fn range_of_influence(&self) -> RangeOfInfluence {
        RangeOfInfluence::Off
    }

    /// Returns the current turn number.
    #[must_use]
    pub const fn turn_number(&self) -> u32 {
        self.turn_number
    }

    /// Returns the current game outcome.
    #[must_use]
    pub const fn game_outcome(&self) -> GameOutcome {
        self.outcome
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
        self.stack_entries.last().cloned()
    }

    /// Returns resolved stack entries in resolution order.
    #[must_use]
    pub fn resolution_log(&self) -> &[ResolutionRecord] {
        &self.resolution_log
    }

    /// Returns registered trigger subscriptions in deterministic ID order.
    pub fn trigger_subscriptions(
        &self,
    ) -> impl Iterator<Item = (TriggerId, TriggerDefinition)> + '_ {
        self.trigger_subscriptions
            .iter()
            .map(|subscription| (subscription.id, subscription.definition))
    }

    /// Returns queued triggered abilities waiting to be put on the stack.
    #[must_use]
    pub fn pending_triggers(&self) -> &[PendingTriggeredAbility] {
        &self.pending_triggers
    }

    /// Returns registered activated abilities in deterministic ID order.
    pub fn activated_abilities(
        &self,
    ) -> impl Iterator<Item = (ActivatedAbilityId, ActivatedAbilityDefinition)> + '_ {
        self.activated_abilities
            .iter()
            .map(|subscription| (subscription.id, subscription.definition))
    }

    /// Returns registered activation cost modifiers in deterministic ID order.
    pub fn cost_modifiers(
        &self,
    ) -> impl Iterator<Item = (CostModifierId, CostModifierDefinition)> + '_ {
        self.cost_modifiers
            .iter()
            .map(|subscription| (subscription.id, subscription.definition))
    }

    /// Returns registered targeting/combat restrictions in deterministic ID order.
    pub fn restrictions(
        &self,
    ) -> impl Iterator<Item = (RestrictionId, RestrictionDefinition)> + '_ {
        self.restrictions
            .iter()
            .map(|subscription| (subscription.id, subscription.definition))
    }

    /// Returns compact object-counter totals in deterministic order.
    pub fn object_counters(&self) -> impl Iterator<Item = (ObjectId, CounterKind, u32)> + '_ {
        self.object_counters
            .iter()
            .map(|entry| (entry.object, entry.kind, entry.count))
    }

    /// Returns the counter count for one object and kind.
    #[must_use]
    pub fn object_counter_count(&self, object: ObjectId, kind: CounterKind) -> u32 {
        if kind == CounterKind::Loyalty {
            return self
                .objects
                .get(object)
                .and_then(ObjectRecord::loyalty)
                .map_or(0, |loyalty| u32::try_from(loyalty.max(0)).unwrap_or(0));
        }
        self.object_counters
            .iter()
            .find(|entry| entry.object == object && entry.kind == kind)
            .map_or(0, |entry| entry.count)
    }

    /// Returns source objects whose loyalty abilities were activated this turn.
    #[must_use]
    pub fn loyalty_activations_this_turn(&self) -> &[ObjectId] {
        &self.loyalty_activations_this_turn
    }

    /// Returns registered replacement/prevention effects in deterministic ID order.
    pub fn replacement_effects(
        &self,
    ) -> impl Iterator<Item = (ReplacementEffectId, ReplacementDefinition)> + '_ {
        self.replacement_effects
            .iter()
            .map(|subscription| (subscription.id, subscription.definition))
    }

    /// Returns stored replacement ordering preferences.
    #[must_use]
    pub fn replacement_choice_orders(&self) -> &[ReplacementChoiceOrder] {
        &self.replacement_choice_orders
    }

    /// Returns registered continuous effects in deterministic ID order.
    pub fn continuous_effects(
        &self,
    ) -> impl Iterator<Item = (ContinuousEffectId, &ContinuousEffectDefinition)> + '_ {
        self.continuous_effects
            .iter()
            .map(|subscription| (subscription.id, &subscription.definition))
    }

    /// Returns typed mutation events emitted during the current turn.
    #[must_use]
    pub fn events_this_turn(&self) -> &[EventRecord] {
        &self.turn_events
    }

    /// Returns a cursor positioned after the latest retained event.
    #[must_use]
    pub const fn event_cursor(&self) -> EventCursor {
        EventCursor {
            turn: self.turn_number,
            next_sequence: self.next_event_sequence,
        }
    }

    /// Returns current-turn events at or after a cursor.
    pub fn events_since(&self, cursor: EventCursor) -> Result<&[EventRecord], EventReplayError> {
        if cursor.turn != self.turn_number {
            return Err(EventReplayError::CursorTurnMismatch {
                cursor_turn: cursor.turn,
                current_turn: self.turn_number,
            });
        }
        if cursor.next_sequence > self.next_event_sequence {
            return Err(EventReplayError::CursorInFuture {
                requested: cursor.next_sequence,
                next_sequence: self.next_event_sequence,
            });
        }
        let oldest = self
            .turn_events
            .first()
            .map_or(self.next_event_sequence, |event| event.sequence());
        if cursor.next_sequence < oldest {
            return Err(EventReplayError::CursorTooOld {
                requested: cursor.next_sequence,
                oldest_retained: oldest,
            });
        }
        let offset = self
            .turn_events
            .iter()
            .position(|event| event.sequence() >= cursor.next_sequence)
            .unwrap_or(self.turn_events.len());
        Ok(&self.turn_events[offset..])
    }

    /// Returns current combat state.
    #[must_use]
    pub const fn combat_state(&self) -> &CombatState {
        &self.combat
    }

    fn emit_event(&mut self, event: GameEvent) {
        let record = self.append_event(event);
        self.queue_triggered_abilities(record);
    }

    fn emit_event_without_triggers(&mut self, event: GameEvent) {
        self.append_event(event);
    }

    fn append_event(&mut self, event: GameEvent) -> EventRecord {
        let record = EventRecord {
            sequence: self.next_event_sequence,
            turn: self.turn_number,
            event,
        };
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        let events = Arc::make_mut(&mut self.turn_events);
        events.push(record);
        if events.len() > EVENT_RING_CAPACITY {
            events.remove(0);
        }
        record
    }

    fn reset_turn_events(&mut self) {
        if Arc::strong_count(&self.turn_events) > 1 {
            self.turn_events = Arc::new(Vec::with_capacity(EVENT_DEEP_CLONE_LIMIT));
        } else {
            Arc::make_mut(&mut self.turn_events).clear();
        }
    }

    /// Adds a player and that player's owned zones.
    fn add_player(&mut self) -> PlayerId {
        let id = PlayerId(self.players.len() as u32);
        self.players.push(PlayerState::new(id));
        let zones = Arc::make_mut(&mut self.zones);
        zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Library),
            objects: Arc::new(Vec::new()),
        });
        zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Hand),
            objects: Arc::new(Vec::new()),
        });
        zones.push(Zone {
            id: ZoneId::new(Some(id), ZoneKind::Graveyard),
            objects: Arc::new(Vec::new()),
        });
        if !self.turn_order.is_empty() {
            self.turn_order.push(id);
        }
        self.emit_event(GameEvent::PlayerAdded { player: id });
        id
    }

    /// Sets a player's maximum hand size.
    fn set_player_max_hand_size(
        &mut self,
        player: PlayerId,
        max_hand_size: u32,
    ) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.max_hand_size = max_hand_size;
        self.emit_event(GameEvent::PlayerMaxHandSizeSet {
            player,
            max_hand_size,
        });
        Ok(())
    }

    /// Returns the current maximum hand size, or none when a continuous rule removes it.
    pub fn effective_max_hand_size(&self, player: PlayerId) -> Result<Option<u32>, StateError> {
        let base = self
            .players
            .get(player.index())
            .ok_or(StateError::UnknownPlayer(player))?
            .max_hand_size;
        let removed = self.restrictions.iter().any(|subscription| {
            if !self.restriction_source_is_active(subscription.definition) {
                return false;
            }
            let RestrictionEffect::PlayerRule {
                subject,
                rule: PlayerRule::NoMaximumHandSize,
            } = subscription.definition.effect()
            else {
                return false;
            };
            self.player_rule_subject_matches(subscription.definition, subject, player)
        });
        Ok((!removed).then_some(base))
    }

    /// Sets a player's life total.
    fn set_player_life(&mut self, player: PlayerId, life: i32) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.life = life;
        self.emit_event(GameEvent::LifeTotalSet { player, life });
        Ok(())
    }

    /// Makes a player lose life.
    fn lose_life(&mut self, player: PlayerId, amount: u32) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.life = player_state
            .life
            .checked_sub(i32::try_from(amount).unwrap_or(i32::MAX))
            .ok_or(StateError::LifeTotalOverflow)?;
        let life = player_state.life;
        self.emit_event(GameEvent::LifeLost {
            player,
            amount,
            life,
        });
        Ok(())
    }

    /// Makes a player gain life.
    fn gain_life(&mut self, player: PlayerId, amount: u32) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.life = player_state
            .life
            .checked_add(i32::try_from(amount).unwrap_or(i32::MAX))
            .ok_or(StateError::LifeTotalOverflow)?;
        let life = player_state.life;
        self.emit_event(GameEvent::LifeGained {
            player,
            amount,
            life,
        });
        Ok(())
    }

    /// Adds poison counters to a player.
    fn add_poison_counters(&mut self, player: PlayerId, amount: u32) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.poison = player_state
            .poison
            .checked_add(amount)
            .ok_or(StateError::PoisonCounterOverflow)?;
        let poison = player_state.poison;
        self.emit_event(GameEvent::PoisonCountersAdded {
            player,
            amount,
            poison,
        });
        Ok(())
    }

    /// Returns a player's current mana pool.
    pub fn mana_pool(&self, player: PlayerId) -> Result<ManaPool, StateError> {
        Ok(self
            .players
            .get(player.index())
            .ok_or(StateError::UnknownPlayer(player))?
            .mana_pool)
    }

    /// Adds mana to a player's pool.
    fn add_mana_to_pool(&mut self, player: PlayerId, mana: ManaPool) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.mana_pool = player_state
            .mana_pool
            .checked_add(mana)
            .ok_or(StateError::ManaValueOverflow)?;
        let mana_pool = player_state.mana_pool;
        self.emit_event(GameEvent::ManaPoolChanged { player, mana_pool });
        Ok(())
    }

    /// Clears one player's mana pool.
    fn clear_mana_pool(&mut self, player: PlayerId) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        player_state.mana_pool = ManaPool::empty();
        self.emit_event(GameEvent::ManaPoolChanged {
            player,
            mana_pool: ManaPool::empty(),
        });
        Ok(())
    }

    /// Enumerates payment plans for one player's current mana pool.
    pub fn payment_plans_for_player(
        &self,
        player: PlayerId,
        cost: ManaCost,
    ) -> Result<PaymentEnumeration, StateError> {
        enumerate_payment_plans(self.mana_pool(player)?, cost).map_err(Self::map_payment_error)
    }

    /// Returns the effective spell cost after Commander tax hooks.
    pub fn effective_spell_cost(
        &self,
        player: PlayerId,
        object: ObjectId,
        base: ManaCost,
    ) -> Result<ManaCost, StateError> {
        self.require_player(player)?;
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        let mut cost = base;
        if record.is_commander()
            && record.owner() == player
            && self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Command))
        {
            cost = Self::add_mana_costs(cost, self.commander_tax(object)?)?;
        }
        for modifier in &self.cost_modifiers {
            if self.spell_cost_modifier_applies(modifier.definition, player, object) {
                cost = Self::apply_cost_modifier(cost, modifier.definition.operation())?;
            }
        }
        Ok(cost)
    }

    /// Returns the effective spell cost for a full keyword-aware cast request.
    pub fn effective_spell_request_cost(
        &self,
        player: PlayerId,
        object: ObjectId,
        request: &CastSpellRequest,
    ) -> Result<ManaCost, StateError> {
        let mut cost = request.flashback().unwrap_or_else(|| request.cost());
        if let Some(kicker) = request.kicker() {
            cost = Self::add_mana_costs(cost, kicker)?;
        }
        self.effective_spell_cost(player, object, cost)
    }

    /// Applies one explicit payment plan to a player's mana pool.
    fn pay_mana(
        &mut self,
        player: PlayerId,
        cost: ManaCost,
        plan: PaymentPlan,
    ) -> Result<(), StateError> {
        let player_state = self
            .players
            .get_mut(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        let canonical = validate_payment_plan(player_state.mana_pool, cost, plan.paid())
            .map_err(Self::map_payment_error)?;
        if canonical != plan {
            return Err(StateError::InvalidPaymentPlan);
        }
        player_state.mana_pool = player_state
            .mana_pool
            .pay(plan)
            .map_err(Self::map_payment_error)?;
        let mana_pool = player_state.mana_pool;
        self.emit_event(GameEvent::ManaPaid {
            player,
            payment: plan,
            mana_pool,
        });
        Ok(())
    }

    fn top_library_objects(&self, library_index: usize, count: u32) -> Vec<ObjectId> {
        let objects = &self.zones[library_index].objects;
        let inspected = usize::try_from(count)
            .unwrap_or(usize::MAX)
            .min(objects.len());
        objects[objects.len() - inspected..].to_vec()
    }

    fn scry(
        &mut self,
        player: PlayerId,
        count: u32,
        bottom: &[ObjectId],
    ) -> Result<(), StateError> {
        self.require_player(player)?;
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        self.require_zone(library)?;
        let library_index = self
            .zone_index(library)
            .ok_or(StateError::UnknownZone(library))?;
        let top = self.top_library_objects(library_index, count);
        for object in bottom {
            if !top.contains(object) {
                return Err(StateError::MissingZoneMembership(*object));
            }
        }
        let zone = &mut Arc::make_mut(&mut self.zones)[library_index];
        for object in bottom {
            let position = zone
                .objects
                .iter()
                .position(|candidate| candidate == object)
                .ok_or(StateError::MissingZoneMembership(*object))?;
            zone.objects_mut().remove(position);
        }
        for (offset, object) in bottom.iter().copied().enumerate() {
            zone.objects_mut().insert(offset, object);
        }
        self.emit_event(GameEvent::LibraryManipulated {
            player,
            operation: LibraryManipulation::Scry,
            count,
            moved: bottom.len() as u32,
        });
        Ok(())
    }

    fn surveil(
        &mut self,
        player: PlayerId,
        count: u32,
        graveyard: &[ObjectId],
    ) -> Result<(), StateError> {
        self.require_player(player)?;
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        let graveyard_zone = ZoneId::new(Some(player), ZoneKind::Graveyard);
        self.require_zone(library)?;
        self.require_zone(graveyard_zone)?;
        let library_index = self
            .zone_index(library)
            .ok_or(StateError::UnknownZone(library))?;
        let top = self.top_library_objects(library_index, count);
        for object in graveyard {
            if !top.contains(object) {
                return Err(StateError::MissingZoneMembership(*object));
            }
        }
        for object in graveyard {
            self.move_object(*object, graveyard_zone)?;
        }
        self.emit_event(GameEvent::LibraryManipulated {
            player,
            operation: LibraryManipulation::Surveil,
            count,
            moved: graveyard.len() as u32,
        });
        Ok(())
    }

    fn cycle(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        cost: ManaCost,
        payment: PaymentPlan,
    ) -> Result<(), StateError> {
        self.require_priority_player(player)?;
        self.require_player(player)?;
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        if self.object_zone(object) != Some(hand) {
            return Err(StateError::ObjectNotCastable(object));
        }
        let owner = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?
            .owner();
        let canonical_payment =
            validate_payment_plan(self.mana_pool(player)?, cost, payment.paid())
                .map_err(Self::map_payment_error)?;
        if canonical_payment != payment {
            return Err(StateError::InvalidPaymentPlan);
        }
        self.pay_mana(player, cost, payment)?;
        self.move_object(object, ZoneId::new(Some(owner), ZoneKind::Graveyard))?;
        self.draw_cards(player, 1)?;
        self.after_priority_action(player, true)?;
        Ok(())
    }

    fn attach_object(
        &mut self,
        attachment: ObjectId,
        target: Option<ObjectId>,
    ) -> Result<(), StateError> {
        if self.objects.get(attachment).is_none() {
            return Err(StateError::UnknownObject(attachment));
        }
        if let Some(target) = target {
            if self.objects.get(target).is_none() {
                return Err(StateError::UnknownObject(target));
            }
        }
        let record = self
            .objects
            .get_mut(attachment)
            .ok_or(StateError::UnknownObject(attachment))?;
        record.attached_to = target;
        self.emit_event(GameEvent::ObjectAttached { attachment, target });
        Ok(())
    }

    fn equip(
        &mut self,
        player: PlayerId,
        equipment: ObjectId,
        target: ObjectId,
        cost: ManaCost,
        payment: PaymentPlan,
    ) -> Result<(), StateError> {
        self.require_priority_player(player)?;
        self.require_player(player)?;
        if !self.can_activate_with_timing(player, ActivationTiming::Sorcery) {
            return Err(StateError::InvalidSpellTiming);
        }
        if self.object_zone(equipment) != Some(ZoneId::new(None, ZoneKind::Battlefield))
            || self.object_controller(equipment)? != player
        {
            return Err(StateError::ObjectNotActivatable(equipment));
        }
        if self.object_zone(target) != Some(ZoneId::new(None, ZoneKind::Battlefield))
            || self.object_controller(target)? != player
            || self.creature_characteristics(target).is_err()
        {
            return Err(StateError::IllegalTarget {
                index: 0,
                target: TargetChoice::Object(target),
            });
        }
        let canonical_payment =
            validate_payment_plan(self.mana_pool(player)?, cost, payment.paid())
                .map_err(Self::map_payment_error)?;
        if canonical_payment != payment {
            return Err(StateError::InvalidPaymentPlan);
        }
        self.pay_mana(player, cost, payment)?;
        self.attach_object(equipment, Some(target))?;
        self.after_priority_action(player, true)?;
        Ok(())
    }

    /// Sets or clears an object's loyalty value.
    fn set_object_loyalty(
        &mut self,
        object: ObjectId,
        loyalty: Option<i32>,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.loyalty = loyalty;
        self.emit_event(GameEvent::ObjectLoyaltySet { object, loyalty });
        Ok(())
    }

    /// Sets an object's Commander color identity metadata.
    fn set_object_color_identity(
        &mut self,
        object: ObjectId,
        colors: ObjectColors,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.color_identity = colors;
        self.emit_event(GameEvent::ObjectColorIdentitySet { object, colors });
        Ok(())
    }

    /// Designates an object as a commander.
    fn designate_commander(
        &mut self,
        object: ObjectId,
        color_identity: ObjectColors,
    ) -> Result<(), StateError> {
        let player = {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            record.commander = true;
            record.color_identity = color_identity;
            record.owner
        };
        self.emit_event(GameEvent::ObjectColorIdentitySet {
            object,
            colors: color_identity,
        });
        self.emit_event(GameEvent::CommanderDesignated {
            object,
            player,
            color_identity,
        });
        Ok(())
    }

    /// Records one commander cast for tax tracking.
    fn record_commander_cast(&mut self, object: ObjectId) -> Result<(), StateError> {
        let (player, cast_count, tax) = {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            if !record.commander {
                return Err(StateError::ObjectNotCommander(object));
            }
            record.commander_cast_count = record
                .commander_cast_count
                .checked_add(1)
                .ok_or(StateError::CommanderCastCountOverflow(object))?;
            let tax = record
                .commander_tax()
                .ok_or(StateError::CommanderTaxOverflow(object))?;
            (record.owner, record.commander_cast_count, tax)
        };
        self.emit_event(GameEvent::CommanderCastRecorded {
            object,
            player,
            cast_count,
            tax,
        });
        Ok(())
    }

    /// Applies the owner's Commander zone choice after a commander reaches graveyard or exile.
    fn choose_commander_zone(
        &mut self,
        player: PlayerId,
        object: ObjectId,
    ) -> Result<(), StateError> {
        self.require_player(player)?;
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        if !record.is_commander() {
            return Err(StateError::ObjectNotCommander(object));
        }
        if record.owner() != player {
            return Err(StateError::CommanderZoneChoiceOwnerMismatch { player, object });
        }
        let from = self
            .object_zone(object)
            .ok_or(StateError::MissingZoneMembership(object))?;
        let legal_origin = from == ZoneId::new(Some(player), ZoneKind::Graveyard)
            || from == ZoneId::new(None, ZoneKind::Exile);
        if !legal_origin {
            return Err(StateError::InvalidCommanderZoneChoice { object, from });
        }
        self.move_object(object, ZoneId::new(None, ZoneKind::Command))
    }

    /// Returns the current Commander tax for one commander.
    pub fn commander_tax(&self, object: ObjectId) -> Result<ManaCost, StateError> {
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        if !record.is_commander() {
            return Err(StateError::ObjectNotCommander(object));
        }
        record
            .commander_tax()
            .ok_or(StateError::CommanderTaxOverflow(object))
    }

    /// Returns the union of a player's commander color identities.
    pub fn commander_color_identity_for_player(
        &self,
        player: PlayerId,
    ) -> Result<ObjectColors, StateError> {
        self.require_player(player)?;
        let mut found = false;
        let mut colors = ObjectColors::none();
        for record in self.objects.iter() {
            if record.owner() == player && record.is_commander() {
                found = true;
                colors = colors.union(record.color_identity());
            }
        }
        if found {
            Ok(colors)
        } else {
            Err(StateError::NoCommanderForPlayer(player))
        }
    }

    /// Returns whether one object is legal under a player's Commander identity.
    pub fn commander_color_identity_legal(
        &self,
        player: PlayerId,
        object: ObjectId,
    ) -> Result<bool, StateError> {
        let allowed = self.commander_color_identity_for_player(player)?;
        let actual = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?
            .color_identity();
        Ok(allowed.contains_all(actual))
    }

    /// Validates a set of objects under a player's Commander color identity.
    fn validate_commander_color_identity(
        &mut self,
        player: PlayerId,
        objects: &[ObjectId],
    ) -> Result<(), StateError> {
        let allowed = self.commander_color_identity_for_player(player)?;
        for object in objects {
            let actual = self
                .objects
                .get(*object)
                .ok_or(StateError::UnknownObject(*object))?
                .color_identity();
            if !allowed.contains_all(actual) {
                return Err(StateError::CommanderColorIdentityViolation {
                    player,
                    object: *object,
                    allowed,
                    actual,
                });
            }
        }
        self.emit_event(GameEvent::CommanderColorIdentityValidated {
            player,
            count: u32::try_from(objects.len()).unwrap_or(u32::MAX),
            allowed,
        });
        Ok(())
    }

    /// Adds counters to one object.
    fn add_object_counters(
        &mut self,
        object: ObjectId,
        kind: CounterKind,
        amount: u32,
    ) -> Result<(), StateError> {
        if self.objects.get(object).is_none() {
            return Err(StateError::UnknownObject(object));
        }
        if amount == 0 {
            return Ok(());
        }
        if kind == CounterKind::Loyalty {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            let current = record.loyalty.unwrap_or(0).max(0);
            let next = current
                .checked_add(i32::try_from(amount).unwrap_or(i32::MAX))
                .ok_or(StateError::CounterOverflow)?;
            record.loyalty = Some(next);
            self.emit_event(GameEvent::ObjectCountersChanged {
                object,
                kind,
                count: u32::try_from(next).unwrap_or(u32::MAX),
            });
            self.emit_event(GameEvent::ObjectLoyaltySet {
                object,
                loyalty: Some(next),
            });
            return Ok(());
        }
        let count = match self
            .object_counters
            .iter_mut()
            .find(|entry| entry.object == object && entry.kind == kind)
        {
            Some(entry) => {
                entry.count = entry
                    .count
                    .checked_add(amount)
                    .ok_or(StateError::CounterOverflow)?;
                entry.count
            }
            None => {
                self.object_counters.push(ObjectCounterEntry {
                    object,
                    kind,
                    count: amount,
                });
                self.object_counters
                    .sort_by_key(|entry| (entry.object, entry.kind));
                amount
            }
        };
        self.emit_event(GameEvent::ObjectCountersChanged {
            object,
            kind,
            count,
        });
        Ok(())
    }

    /// Removes counters from one object.
    fn remove_object_counters(
        &mut self,
        object: ObjectId,
        kind: CounterKind,
        amount: u32,
    ) -> Result<(), StateError> {
        if self.objects.get(object).is_none() {
            return Err(StateError::UnknownObject(object));
        }
        if amount == 0 {
            return Ok(());
        }
        if kind == CounterKind::Loyalty {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            let current = record.loyalty.unwrap_or(0).max(0);
            let requested = i32::try_from(amount).unwrap_or(i32::MAX);
            if current < requested {
                return Err(StateError::InsufficientCounters {
                    object,
                    kind,
                    available: u32::try_from(current).unwrap_or(0),
                    requested: amount,
                });
            }
            let next = current - requested;
            record.loyalty = Some(next);
            self.emit_event(GameEvent::ObjectCountersChanged {
                object,
                kind,
                count: u32::try_from(next).unwrap_or(0),
            });
            self.emit_event(GameEvent::ObjectLoyaltySet {
                object,
                loyalty: Some(next),
            });
            return Ok(());
        }
        let Some(index) = self
            .object_counters
            .iter()
            .position(|entry| entry.object == object && entry.kind == kind)
        else {
            return Err(StateError::InsufficientCounters {
                object,
                kind,
                available: 0,
                requested: amount,
            });
        };
        let available = self.object_counters[index].count;
        if available < amount {
            return Err(StateError::InsufficientCounters {
                object,
                kind,
                available,
                requested: amount,
            });
        }
        let count = available - amount;
        if count == 0 {
            self.object_counters.remove(index);
        } else {
            self.object_counters[index].count = count;
        }
        self.emit_event(GameEvent::ObjectCountersChanged {
            object,
            kind,
            count,
        });
        Ok(())
    }

    /// Registers one data-only activated ability definition.
    fn register_activated_ability(
        &mut self,
        definition: ActivatedAbilityDefinition,
    ) -> Result<ActivatedAbilityId, StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        let id = ActivatedAbilityId(self.next_activated_ability);
        self.next_activated_ability = self.next_activated_ability.saturating_add(1);
        self.activated_abilities
            .push(ActivatedAbilitySubscription { id, definition });
        self.emit_event(GameEvent::ActivatedAbilityRegistered {
            ability: id,
            controller: definition.controller(),
            source: definition.source(),
            mana_ability: definition.is_mana_ability(),
        });
        Ok(id)
    }

    /// Registers one data-only activation cost modifier.
    fn register_cost_modifier(
        &mut self,
        definition: CostModifierDefinition,
    ) -> Result<CostModifierId, StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        match definition.scope() {
            CostModifierScope::Ability(ability) => {
                self.activated_ability_definition(ability)?;
            }
            CostModifierScope::Source(source) => {
                if self.objects.get(source).is_none() {
                    return Err(StateError::UnknownObject(source));
                }
            }
            CostModifierScope::Controller(player) => self.require_player(player)?,
            CostModifierScope::AllActivatedAbilities | CostModifierScope::Spells(_) => {}
        }
        let id = CostModifierId(self.next_cost_modifier);
        self.next_cost_modifier = self.next_cost_modifier.saturating_add(1);
        self.cost_modifiers
            .push(CostModifierSubscription { id, definition });
        self.emit_event(GameEvent::CostModifierRegistered {
            modifier: id,
            controller: definition.controller(),
            source: definition.source(),
            operation: definition.operation(),
        });
        Ok(id)
    }

    /// Registers one data-only targeting or combat restriction.
    fn register_restriction(
        &mut self,
        definition: RestrictionDefinition,
    ) -> Result<RestrictionId, StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        match definition.effect() {
            RestrictionEffect::Targeting { subject, .. } => match subject {
                TargetRestrictionSubject::Object(object) => {
                    if self.objects.get(object).is_none() {
                        return Err(StateError::UnknownObject(object));
                    }
                }
                TargetRestrictionSubject::AllObjects => {}
                TargetRestrictionSubject::AttachedToSource => {
                    if definition.source().is_none() {
                        return Err(StateError::ObjectNotActivatable(ObjectId(0)));
                    }
                }
            },
            RestrictionEffect::Combat { subject, .. } => match subject {
                CombatRestrictionSubject::Object(object) => {
                    if self.objects.get(object).is_none() {
                        return Err(StateError::UnknownObject(object));
                    }
                }
                CombatRestrictionSubject::ControlledBy(player) => self.require_player(player)?,
                CombatRestrictionSubject::AllObjects => {}
            },
            RestrictionEffect::PlayerRule { subject, .. } => match subject {
                PlayerRuleSubject::Player(player) => self.require_player(player)?,
                PlayerRuleSubject::ControllerOfSource => {
                    if definition.source().is_none() {
                        return Err(StateError::ObjectNotActivatable(ObjectId(0)));
                    }
                }
            },
            RestrictionEffect::Indestructible { object } => {
                if self.objects.get(object).is_none() {
                    return Err(StateError::UnknownObject(object));
                }
            }
            RestrictionEffect::RegenerationShield { object } => {
                if self.objects.get(object).is_none() {
                    return Err(StateError::UnknownObject(object));
                }
                if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Battlefield)) {
                    return Err(StateError::ObjectNotOnBattlefield(object));
                }
            }
        }
        let id = RestrictionId(self.next_restriction);
        self.next_restriction = self.next_restriction.saturating_add(1);
        self.restrictions
            .push(RestrictionSubscription { id, definition });
        self.emit_event(GameEvent::RestrictionRegistered {
            restriction: id,
            controller: definition.controller(),
            source: definition.source(),
            effect: definition.effect(),
        });
        Ok(id)
    }

    /// Returns the effective activation cost after T2.5 cost modifiers.
    pub fn effective_activation_cost(
        &self,
        ability: ActivatedAbilityId,
    ) -> Result<ActivationCost, StateError> {
        let definition = self.activated_ability_definition(ability)?;
        let controller = self.activated_ability_controller(definition)?;
        let mut mana = definition.cost().mana();
        for modifier in &self.cost_modifiers {
            if self.cost_modifier_applies(modifier.definition, ability, definition, controller) {
                mana = Self::apply_cost_modifier(mana, modifier.definition.operation())?;
            }
        }
        let mut cost = ActivationCost::new(mana);
        if definition.cost().tap_source() {
            cost = cost.with_tap_source();
        }
        if definition.cost().sacrifice_source() {
            cost = cost.with_sacrifice_source();
        }
        if let Some(delta) = definition.cost().loyalty_delta() {
            cost = cost.with_loyalty_delta(delta);
        }
        Ok(cost)
    }

    fn activated_ability_definition(
        &self,
        ability: ActivatedAbilityId,
    ) -> Result<ActivatedAbilityDefinition, StateError> {
        self.activated_abilities
            .get(ability.index())
            .filter(|subscription| subscription.id == ability)
            .map(|subscription| subscription.definition)
            .ok_or(StateError::UnknownActivatedAbility(ability))
    }

    fn activated_ability_controller(
        &self,
        definition: ActivatedAbilityDefinition,
    ) -> Result<PlayerId, StateError> {
        if let Some(source) = definition.source() {
            self.object_controller(source)
        } else {
            self.require_player(definition.controller())?;
            Ok(definition.controller())
        }
    }

    fn cost_modifier_applies(
        &self,
        modifier: CostModifierDefinition,
        ability: ActivatedAbilityId,
        definition: ActivatedAbilityDefinition,
        controller: PlayerId,
    ) -> bool {
        if !self.cost_modifier_source_is_active(modifier) {
            return false;
        }
        match modifier.scope() {
            CostModifierScope::AllActivatedAbilities => true,
            CostModifierScope::Ability(expected) => expected == ability,
            CostModifierScope::Source(expected) => definition.source() == Some(expected),
            CostModifierScope::Controller(expected) => expected == controller,
            CostModifierScope::Spells(_) => false,
        }
    }

    fn spell_cost_modifier_applies(
        &self,
        modifier: CostModifierDefinition,
        player: PlayerId,
        object: ObjectId,
    ) -> bool {
        if modifier.controller() != player || !self.cost_modifier_source_is_active(modifier) {
            return false;
        }
        match modifier.scope() {
            CostModifierScope::Spells(predicate) => {
                self.object_matches_target_predicate(player, predicate, object)
            }
            CostModifierScope::AllActivatedAbilities
            | CostModifierScope::Ability(_)
            | CostModifierScope::Source(_)
            | CostModifierScope::Controller(_) => false,
        }
    }

    fn cost_modifier_source_is_active(&self, modifier: CostModifierDefinition) -> bool {
        modifier.source().map_or(true, |source| {
            self.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Battlefield))
        })
    }

    fn apply_cost_modifier(
        cost: ManaCost,
        operation: CostModifierOperation,
    ) -> Result<ManaCost, StateError> {
        match operation {
            CostModifierOperation::AddManaCost(additional) => {
                Self::add_mana_costs(cost, additional)
            }
            CostModifierOperation::AddGeneric(amount) => {
                let generic = cost
                    .generic_total()
                    .map_err(Self::map_payment_error)?
                    .checked_add(amount)
                    .ok_or(StateError::ManaValueOverflow)?;
                Ok(ManaCost::new(
                    cost.colored(ManaKind::White),
                    cost.colored(ManaKind::Blue),
                    cost.colored(ManaKind::Black),
                    cost.colored(ManaKind::Red),
                    cost.colored(ManaKind::Green),
                    generic,
                ))
            }
            CostModifierOperation::ReduceGeneric(amount) => {
                let generic = cost
                    .generic_total()
                    .map_err(Self::map_payment_error)?
                    .saturating_sub(amount);
                Ok(ManaCost::new(
                    cost.colored(ManaKind::White),
                    cost.colored(ManaKind::Blue),
                    cost.colored(ManaKind::Black),
                    cost.colored(ManaKind::Red),
                    cost.colored(ManaKind::Green),
                    generic,
                ))
            }
        }
    }

    fn activate_ability(
        &mut self,
        player: PlayerId,
        ability: ActivatedAbilityId,
        payment: PaymentPlan,
    ) -> Result<Option<StackEntryId>, StateError> {
        let definition = self.activated_ability_definition(ability)?;
        let controller = self.activated_ability_controller(definition)?;
        if controller != player {
            return Err(StateError::PriorityPlayerMismatch {
                expected: controller,
                actual: player,
            });
        }
        if !self.activation_condition_is_met(player, definition)? {
            return Err(StateError::ActivationConditionNotMet(ability));
        }
        if !definition.is_mana_ability() {
            if self.split_second_on_stack() {
                return Err(StateError::SplitSecondActionForbidden);
            }
            self.require_priority_player(player)?;
            if !self.can_activate_with_timing(player, definition.timing()) {
                return Err(StateError::InvalidSpellTiming);
            }
        }
        if definition.cost().loyalty_delta().is_some()
            && !self.can_activate_with_timing(player, ActivationTiming::Sorcery)
        {
            return Err(StateError::InvalidSpellTiming);
        }
        let effective_cost = self.effective_activation_cost(ability)?;
        let canonical_payment = validate_payment_plan(
            self.mana_pool(player)?,
            effective_cost.mana(),
            payment.paid(),
        )
        .map_err(Self::map_payment_error)?;
        if canonical_payment != payment {
            return Err(StateError::InvalidPaymentPlan);
        }
        self.validate_non_mana_activation_costs(player, definition, effective_cost)?;

        self.pay_mana(player, effective_cost.mana(), payment)?;
        self.pay_non_mana_activation_costs(definition, effective_cost)?;
        self.emit_event(GameEvent::ActivatedAbilityActivated {
            ability,
            player,
            source: definition.source(),
            mana_ability: definition.is_mana_ability(),
        });

        if definition.is_mana_ability() {
            self.resolve_activated_ability_effect(ability, player, definition)?;
            Ok(None)
        } else {
            let id = self.push_stack_entry(StackEntryRequest {
                controller: player,
                object: None,
                trigger: None,
                activated_ability: Some(ability),
                kind: StackObjectKind::ActivatedAbility,
                targets: Vec::new(),
                payment: Some(payment),
                copy_info: None,
                kicked: false,
                flashback: false,
                split_second: false,
            });
            self.after_priority_action(player, true)?;
            Ok(Some(id))
        }
    }

    fn validate_non_mana_activation_costs(
        &self,
        player: PlayerId,
        definition: ActivatedAbilityDefinition,
        cost: ActivationCost,
    ) -> Result<(), StateError> {
        if let Some(source) = definition.source() {
            if self.object_zone(source) != Some(ZoneId::new(None, ZoneKind::Battlefield)) {
                return Err(StateError::ObjectNotActivatable(source));
            }
            if self.object_controller(source)? != player {
                return Err(StateError::ObjectNotActivatable(source));
            }
            let record = self
                .objects
                .get(source)
                .ok_or(StateError::UnknownObject(source))?;
            if cost.tap_source() {
                if record.tapped() {
                    return Err(StateError::SourceAlreadyTapped(source));
                }
                if self
                    .creature_characteristics(source)
                    .is_ok_and(|characteristics| !characteristics.keywords().haste())
                    && record.controlled_since_turn() == self.turn_number
                {
                    return Err(StateError::SummoningSick(source));
                }
            }
            if let Some(delta) = cost.loyalty_delta() {
                if self.loyalty_activations_this_turn.contains(&source) {
                    return Err(StateError::LoyaltyAbilityAlreadyActivatedThisTurn(source));
                }
                let loyalty = record
                    .loyalty()
                    .ok_or(StateError::InsufficientLoyalty(source))?;
                let next = loyalty
                    .checked_add(delta)
                    .ok_or(StateError::LifeTotalOverflow)?;
                if next < 0 {
                    return Err(StateError::InsufficientLoyalty(source));
                }
            }
        } else if cost.tap_source() || cost.sacrifice_source() || cost.loyalty_delta().is_some() {
            return Err(StateError::ObjectNotActivatable(ObjectId(0)));
        }
        Ok(())
    }

    fn pay_non_mana_activation_costs(
        &mut self,
        definition: ActivatedAbilityDefinition,
        cost: ActivationCost,
    ) -> Result<(), StateError> {
        if let Some(source) = definition.source() {
            if cost.tap_source() {
                self.set_object_tapped(source, true)?;
            }
            if let Some(delta) = cost.loyalty_delta() {
                let record = self
                    .objects
                    .get_mut(source)
                    .ok_or(StateError::UnknownObject(source))?;
                let next = record
                    .loyalty()
                    .ok_or(StateError::InsufficientLoyalty(source))?
                    .checked_add(delta)
                    .ok_or(StateError::LifeTotalOverflow)?;
                record.loyalty = Some(next);
                self.loyalty_activations_this_turn.push(source);
                self.emit_event(GameEvent::ObjectLoyaltySet {
                    object: source,
                    loyalty: Some(next),
                });
            }
            if cost.sacrifice_source() {
                let owner = self
                    .objects
                    .get(source)
                    .ok_or(StateError::UnknownObject(source))?
                    .owner();
                self.move_object(source, ZoneId::new(Some(owner), ZoneKind::Graveyard))?;
            }
        }
        Ok(())
    }

    fn activation_condition_is_met(
        &self,
        controller: PlayerId,
        definition: ActivatedAbilityDefinition,
    ) -> Result<bool, StateError> {
        let Some(condition) = definition.condition() else {
            return Ok(true);
        };
        match condition {
            ActivationCondition::ControllerControlsAtLeast { predicate, count } => {
                let battlefield = self
                    .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
                    .ok_or(StateError::UnknownZone(ZoneId::new(
                        None,
                        ZoneKind::Battlefield,
                    )))?;
                let matches = battlefield
                    .iter()
                    .copied()
                    .filter(|object| {
                        self.object_matches_target_predicate(controller, predicate, *object)
                    })
                    .count();
                Ok(matches >= usize::try_from(count).unwrap_or(usize::MAX))
            }
        }
    }

    fn resolve_activated_ability_effect(
        &mut self,
        ability: ActivatedAbilityId,
        player: PlayerId,
        definition: ActivatedAbilityDefinition,
    ) -> Result<(), StateError> {
        match definition.effect() {
            ActivatedAbilityEffect::AddMana {
                player: target,
                mana,
            } => {
                let target = self.resolve_ability_player(player, target)?;
                self.add_mana_to_pool(target, mana)?;
            }
            ActivatedAbilityEffect::GainLife {
                player: target,
                amount,
            } => {
                let target = self.resolve_ability_player(player, target)?;
                self.gain_life(target, amount)?;
            }
            ActivatedAbilityEffect::LoseLife {
                player: target,
                amount,
            } => {
                let target = self.resolve_ability_player(player, target)?;
                self.lose_life(target, amount)?;
            }
            ActivatedAbilityEffect::AddManaAndDealDamage {
                player: target,
                mana,
                amount,
            } => {
                let target = self.resolve_ability_player(player, target)?;
                self.add_mana_to_pool(target, mana)?;
                self.deal_noncombat_damage(
                    definition.source(),
                    CombatDamageTarget::Player(target),
                    amount,
                )?;
            }
        }
        self.emit_event(GameEvent::ActivatedAbilityResolved {
            ability,
            player,
            source: definition.source(),
            effect: definition.effect(),
        });
        Ok(())
    }

    fn resolve_ability_player(
        &self,
        controller: PlayerId,
        player: AbilityPlayer,
    ) -> Result<PlayerId, StateError> {
        let resolved = match player {
            AbilityPlayer::Controller => controller,
            AbilityPlayer::Player(player) => player,
        };
        self.require_player(resolved)?;
        Ok(resolved)
    }

    /// Sets base printed card types and colors for one object.
    fn set_base_object_characteristics(
        &mut self,
        object: ObjectId,
        base: BaseObjectCharacteristics,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.base_object = base;
        self.emit_event(GameEvent::BaseObjectCharacteristicsSet { object, base });
        Ok(())
    }

    /// Sets or replaces base printed creature characteristics for one object.
    fn set_base_creature_characteristics(
        &mut self,
        object: ObjectId,
        base: BaseCreatureCharacteristics,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.base_creature = Some(base);
        self.emit_event(GameEvent::BaseCreatureCharacteristicsSet { object, base });
        Ok(())
    }

    /// Clears base printed creature characteristics from one object.
    fn clear_base_creature_characteristics(&mut self, object: ObjectId) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.base_creature = None;
        self.remove_object_from_combat(object);
        self.emit_event(GameEvent::BaseCreatureCharacteristicsCleared { object });
        Ok(())
    }

    /// Sets an object's tapped status.
    fn set_object_tapped(&mut self, object: ObjectId, tapped: bool) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        record.tapped = tapped;
        self.emit_event(GameEvent::ObjectTapped { object, tapped });
        Ok(())
    }

    /// Marks damage on a creature object.
    fn mark_damage_on_object(&mut self, object: ObjectId, amount: u32) -> Result<(), StateError> {
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        if record.base_creature.is_none() {
            return Err(StateError::NotACreature(object));
        }
        let replaced = self.apply_damage_replacement_effects(DamageReplacementEvent {
            source: None,
            target: CombatDamageTarget::Object(object),
            amount,
            combat: false,
        })?;
        if amount > 0 && replaced.amount == 0 {
            return Ok(());
        }
        self.mark_damage_on_object_unreplaced(object, replaced.amount)
    }

    fn deal_noncombat_damage(
        &mut self,
        source: Option<ObjectId>,
        target: CombatDamageTarget,
        amount: u32,
    ) -> Result<(), StateError> {
        if let Some(source) = source {
            self.objects
                .get(source)
                .ok_or(StateError::UnknownObject(source))?;
        }
        match target {
            CombatDamageTarget::Player(player) => self.require_player(player)?,
            CombatDamageTarget::Object(object) => {
                self.objects
                    .get(object)
                    .ok_or(StateError::UnknownObject(object))?;
            }
        }
        let replaced = self.apply_damage_replacement_effects(DamageReplacementEvent {
            source,
            target,
            amount,
            combat: false,
        })?;
        if replaced.amount == 0 {
            return Ok(());
        }

        match replaced.target {
            CombatDamageTarget::Player(player) => self.lose_life(player, replaced.amount)?,
            CombatDamageTarget::Object(object) => {
                let characteristics = self.object_characteristics(object)?;
                let mut represented = false;
                if characteristics.types().creature() {
                    self.mark_damage_on_object_unreplaced(object, replaced.amount)?;
                    represented = true;
                    if source.is_some_and(|source| {
                        self.creature_characteristics(source)
                            .is_ok_and(|source| source.keywords().deathtouch())
                    }) {
                        self.objects
                            .get_mut(object)
                            .ok_or(StateError::UnknownObject(object))?
                            .deathtouch_damage_marked = true;
                    }
                }
                if characteristics.types().planeswalker() {
                    let delta = i32::try_from(replaced.amount)
                        .map_err(|_| StateError::LifeTotalOverflow)?;
                    let record = self
                        .objects
                        .get_mut(object)
                        .ok_or(StateError::UnknownObject(object))?;
                    let loyalty = record
                        .loyalty()
                        .ok_or(StateError::InsufficientLoyalty(object))?;
                    let next = loyalty.saturating_sub(delta);
                    record.loyalty = Some(next);
                    self.emit_event(GameEvent::ObjectLoyaltySet {
                        object,
                        loyalty: Some(next),
                    });
                    represented = true;
                }
                if !represented {
                    return Err(StateError::NotACreature(object));
                }
            }
        }

        if let Some(source) = source {
            if self
                .creature_characteristics(source)
                .is_ok_and(|source| source.keywords().lifelink())
            {
                let controller = self.object_controller(source)?;
                self.gain_life(controller, replaced.amount)?;
            }
        }
        self.emit_event(GameEvent::NoncombatDamageDealt {
            source,
            target: replaced.target,
            amount: replaced.amount,
        });
        Ok(())
    }

    fn mark_damage_on_object_unreplaced(
        &mut self,
        object: ObjectId,
        amount: u32,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get_mut(object)
            .ok_or(StateError::UnknownObject(object))?;
        if record.base_creature.is_none() {
            return Err(StateError::NotACreature(object));
        }
        record.damage_marked = record
            .damage_marked
            .checked_add(amount)
            .ok_or(StateError::CombatDamageOverflow)?;
        let total_damage = record.damage_marked;
        self.emit_event(GameEvent::DamageMarked {
            object,
            amount,
            total_damage,
        });
        Ok(())
    }

    /// Checks CR 704 state-based actions until a fixpoint is reached.
    fn check_state_based_actions(&mut self) -> Result<StateBasedActionReport, StateError> {
        self.perform_state_based_actions()
    }

    fn decide_turn_order(&mut self) -> Result<PlayerId, StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        if self.starting_player.is_some() {
            return Err(StateError::TurnOrderAlreadyDecided);
        }
        let player = PlayerId(self.random_below(self.players.len()) as u32);
        self.starting_player = Some(player);
        self.turn_order = self.turn_order_from(player)?;
        self.emit_event(GameEvent::TurnOrderDecided {
            starting_player: player,
        });
        Ok(player)
    }

    fn set_turn_order(&mut self, order: Vec<PlayerId>) -> Result<PlayerId, StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        if self.starting_player.is_some() || self.current_step.is_some() || self.opening_hands_drawn
        {
            return Err(StateError::TurnOrderAlreadyDecided);
        }
        self.validate_turn_order(&order)?;
        let starting_player = order[0];
        self.starting_player = Some(starting_player);
        self.turn_order = order;
        self.emit_event(GameEvent::TurnOrderDecided { starting_player });
        Ok(starting_player)
    }

    fn draw_opening_hands(&mut self) -> Result<(), StateError> {
        if self.opening_hands_drawn {
            return Err(StateError::OpeningHandsAlreadyDrawn);
        }
        let starting = self
            .starting_player
            .ok_or(StateError::TurnOrderNotDecided)?;
        let players = self.apnap_players(starting)?;
        for player in players {
            self.draw_cards(player, OPENING_HAND_SIZE)?;
        }
        self.opening_hands_drawn = true;
        self.emit_event(GameEvent::OpeningHandsDrawn);
        Ok(())
    }

    fn take_mulligan(&mut self, player: PlayerId) -> Result<(), StateError> {
        self.require_player(player)?;
        if !self.opening_hands_drawn {
            return Err(StateError::OpeningHandsNotDrawn);
        }
        let player_state = self
            .players
            .get(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        if player_state.opening_hand_kept {
            return Err(StateError::MulliganAfterKeep(player));
        }
        let next_mulligans = player_state
            .mulligans_taken
            .checked_add(1)
            .ok_or(StateError::MulliganCountOverflow)?;
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);

        self.move_all_between_zones(hand, library)?;
        self.shuffle_zone(library)?;
        self.draw_cards(player, OPENING_HAND_SIZE)?;
        self.players[player.index()].mulligans_taken = next_mulligans;
        self.emit_event(GameEvent::MulliganTaken {
            player,
            mulligans_taken: next_mulligans,
        });
        Ok(())
    }

    fn keep_opening_hand(
        &mut self,
        player: PlayerId,
        bottom: &[ObjectId],
    ) -> Result<(), StateError> {
        self.require_player(player)?;
        if !self.opening_hands_drawn {
            return Err(StateError::OpeningHandsNotDrawn);
        }
        let player_state = self
            .players
            .get(player.index())
            .ok_or(StateError::UnknownPlayer(player))?;
        if player_state.opening_hand_kept {
            return Err(StateError::OpeningHandAlreadyKept(player));
        }
        let actual = u32::try_from(bottom.len()).unwrap_or(u32::MAX);
        if player_state.mulligans_taken != actual {
            return Err(StateError::InvalidOpeningHandBottomCount {
                expected: player_state.mulligans_taken,
                actual,
            });
        }

        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let mut seen = Vec::with_capacity(bottom.len());
        for object in bottom {
            if seen.contains(object) {
                return Err(StateError::DuplicateOpeningHandBottomCard(*object));
            }
            seen.push(*object);
            if self.object_zone(*object) != Some(hand) {
                return Err(StateError::OpeningHandBottomCardNotInHand {
                    player,
                    object: *object,
                });
            }
        }

        self.put_hand_cards_on_library_bottom(player, bottom)?;
        self.players[player.index()].opening_hand_kept = true;
        self.emit_event(GameEvent::OpeningHandKept { player });
        Ok(())
    }

    /// Returns the players in arena order.
    #[must_use]
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// Returns object arena storage.
    #[cfg(test)]
    #[must_use]
    const fn objects(&self) -> &ObjectArena {
        &self.objects
    }

    /// Returns one object record by ID.
    #[must_use]
    pub fn object(&self, object: ObjectId) -> Option<ObjectRecord> {
        self.objects.get(object)
    }

    /// Returns one zone by ID.
    #[cfg(test)]
    #[must_use]
    fn zone(&self, id: ZoneId) -> Option<&Zone> {
        let index = self.zone_index(id)?;
        self.zones.get(index)
    }

    /// Returns the redacted state projection visible to one observing player.
    pub fn player_view(&self, observer: PlayerId) -> Result<PlayerView, StateError> {
        self.require_player(observer)?;
        let mut zones = Vec::with_capacity(self.zones.len());
        for zone in self.zones.iter() {
            let hidden_from_observer = match zone.id.kind() {
                ZoneKind::Hand => zone.id.owner() != Some(observer),
                ZoneKind::Library => true,
                _ => false,
            };
            let mut objects = Vec::with_capacity(zone.objects.len());
            for object in zone.objects.iter() {
                let record = self
                    .objects
                    .get(*object)
                    .ok_or(StateError::InvalidZoneObject {
                        zone: zone.id,
                        object: *object,
                    })?;
                let remembered = self.knows_object(observer, *object);
                if hidden_from_observer && !remembered {
                    objects.push(ObjectView::Hidden);
                } else {
                    objects.push(ObjectView::Known {
                        object: record,
                        characteristics: self.object_characteristics(*object)?,
                    });
                }
            }
            zones.push(ZoneView {
                id: zone.id,
                objects,
            });
        }
        Ok(PlayerView {
            observer,
            turn_number: self.turn_number,
            outcome: self.outcome,
            starting_player: self.starting_player,
            opening_hands_drawn: self.opening_hands_drawn,
            turn_order: self.turn_order.clone(),
            range_of_influence: self.range_of_influence(),
            active_player: self.active_player,
            priority_player: self.priority_player,
            current_step: self.current_step,
            players: self.players.clone(),
            zones,
        })
    }

    /// Clones this state and populates every card identity hidden from `observer`.
    ///
    /// The live state is never mutated. Assignments must cover every redacted
    /// hand and library slot exactly once, and may not replace identities that
    /// the observer already knows. This is the only supported bridge from a
    /// hidden-information sample into a search-only [`GameState`] clone.
    pub fn determinized_clone(
        &self,
        observer: PlayerId,
        assignments: &[HiddenSlotDefinition],
    ) -> Result<Self, StateError> {
        self.require_player(observer)?;
        let expected = self
            .zones
            .iter()
            .filter(|zone| {
                zone.id.kind() == ZoneKind::Library
                    || (zone.id.kind() == ZoneKind::Hand && zone.id.owner() != Some(observer))
            })
            .flat_map(|zone| zone.objects.iter())
            .filter(|object| !self.knows_object(observer, **object))
            .count();
        let mut slots = Vec::with_capacity(assignments.len());
        for assignment in assignments {
            let zone = assignment.zone();
            if !matches!(zone.kind(), ZoneKind::Hand | ZoneKind::Library) {
                return Err(StateError::InvalidDeterminizationZone(zone));
            }
            let slot = assignment.slot();
            let slot_index = slot as usize;
            let zone_record = self
                .zones
                .iter()
                .find(|candidate| candidate.id == zone)
                .ok_or(StateError::UnknownZone(zone))?;
            let object = zone_record
                .objects
                .get(slot_index)
                .ok_or(StateError::DeterminizationSlotOutOfBounds { zone, slot })?;
            let hidden_from_observer = zone.kind() == ZoneKind::Library
                || (zone.kind() == ZoneKind::Hand && zone.owner() != Some(observer));
            if !hidden_from_observer || self.knows_object(observer, *object) {
                return Err(StateError::DeterminizationSlotKnown { zone, slot });
            }
            slots.push((zone, slot));
        }
        slots.sort_unstable();
        if let Some(duplicate) = slots.windows(2).find(|pair| pair[0] == pair[1]) {
            let (zone, slot) = duplicate[0];
            return Err(StateError::DuplicateDeterminizationSlot { zone, slot });
        }
        if expected != assignments.len() {
            return Err(StateError::DeterminizationSlotCountMismatch {
                expected: u32::try_from(expected).unwrap_or(u32::MAX),
                actual: u32::try_from(assignments.len()).unwrap_or(u32::MAX),
            });
        }

        let mut clone = self.clone();
        for assignment in assignments {
            let zone = assignment.zone();
            let object = self
                .zone_objects(zone)
                .and_then(|objects| objects.get(assignment.slot() as usize))
                .copied()
                .ok_or(StateError::DeterminizationSlotOutOfBounds {
                    zone,
                    slot: assignment.slot(),
                })?;
            let record = clone
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            if record.is_token() || record.is_copy() {
                return Err(StateError::InvalidDeterminizationObject(object));
            }
            let definition = assignment.definition();
            record.card = definition.card();
            record.base_object = definition.base_object();
            record.base_creature = definition.base_creature();
            record.color_identity = definition.color_identity();
        }
        Ok(clone)
    }

    const fn zone_hides_identity(zone: ZoneId) -> bool {
        matches!(zone.kind(), ZoneKind::Hand | ZoneKind::Library)
    }

    fn knows_object(&self, observer: PlayerId, object: ObjectId) -> bool {
        self.known_objects
            .binary_search(&KnownObject { observer, object })
            .is_ok()
    }

    fn remember_object(&mut self, observer: PlayerId, object: ObjectId) {
        let known = KnownObject { observer, object };
        let entries = Arc::make_mut(&mut self.known_objects);
        if let Err(index) = entries.binary_search(&known) {
            entries.insert(index, known);
        }
    }

    fn remember_hidden_transition(&mut self, object: ObjectId, from: ZoneId, to: ZoneId) {
        if !Self::zone_hides_identity(to) {
            return;
        }
        if !Self::zone_hides_identity(from) {
            let observers = self
                .players
                .iter()
                .map(|player| player.id())
                .collect::<Vec<_>>();
            for observer in observers {
                self.remember_object(observer, object);
            }
        } else if from.kind() == ZoneKind::Hand {
            if let Some(owner) = from.owner() {
                self.remember_object(owner, object);
            }
        }
    }

    /// Starts a turn for the chosen active player at the untap step.
    fn start_turn(&mut self, active_player: PlayerId) -> Result<(), StateError> {
        self.require_player(active_player)?;
        if let Some(starting_player) = self.starting_player {
            if active_player != starting_player {
                return Err(StateError::StartingPlayerMismatch {
                    expected: starting_player,
                    actual: active_player,
                });
            }
            if !self.opening_hands_drawn {
                return Err(StateError::OpeningHandsNotDrawn);
            }
            if let Some(player) = self
                .players
                .iter()
                .find(|player| !player.opening_hand_kept())
                .map(|player| player.id())
            {
                return Err(StateError::OpeningHandKeepPending(player));
            }
        }
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
        self.loyalty_activations_this_turn.clear();
        for player in &mut self.players {
            player.lands_played_this_turn = 0;
        }
        self.reset_turn_events();
        self.emit_event(GameEvent::TurnStarted {
            turn: self.turn_number,
            active_player,
        });
        self.begin_step(Step::Untap)
    }

    /// Advances from the current step or main-phase segment to the next one.
    ///
    /// This remains available for no-priority steps and tests. Steps with a
    /// priority window should usually end through [`Self::pass_priority`],
    /// because CR 117.4 requires all players to pass in succession.
    fn advance_step(&mut self) -> Result<Step, StateError> {
        self.advance_step_after_empty_stack()
    }

    /// Passes priority for the current priority player.
    ///
    /// If all players pass in succession, this either resolves the top stack
    /// entry or completes the current step when the stack is empty.
    #[inline(always)]
    fn pass_priority(&mut self, player: PlayerId) -> Result<PriorityOutcome, StateError> {
        if !self.pending_triggers.is_empty() {
            return Err(StateError::PendingTriggeredAbilities);
        }
        let priority_player = self.priority_player.ok_or(StateError::NoPriority)?;
        if priority_player != player {
            return Err(StateError::PriorityPlayerMismatch {
                expected: priority_player,
                actual: player,
            });
        }
        self.priority_pass_count += 1;
        let player_count = self.players.len() as u32;
        if self.priority_pass_count < player_count {
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
            self.grant_priority_after_resolution()?;
            Ok(PriorityOutcome::Resolved(resolved))
        }
    }

    /// Casts a spell through the T1.5 CR 601 pipeline.
    ///
    /// This validates priority, timing, targets, and the explicit mana payment
    /// before mutating state. On success, the spell object moves to the stack,
    /// target legality snapshots are captured, costs are paid, and priority
    /// returns to the caster.
    fn cast_spell(
        &mut self,
        player: PlayerId,
        object: ObjectId,
        request: CastSpellRequest,
    ) -> Result<StackEntryId, StateError> {
        if self.split_second_on_stack() {
            return Err(StateError::SplitSecondActionForbidden);
        }
        self.require_priority_player(player)?;
        self.require_player(player)?;
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        let zone = self.object_zone(object);
        let casting_from_hand = record.controller() == player
            && zone == Some(ZoneId::new(Some(player), ZoneKind::Hand));
        let casting_commander_from_command = record.is_commander()
            && record.owner() == player
            && zone == Some(ZoneId::new(None, ZoneKind::Command));
        let casting_from_graveyard_with_flashback = request.flashback().is_some()
            && record.owner() == player
            && zone == Some(ZoneId::new(Some(player), ZoneKind::Graveyard));
        if !casting_from_hand
            && !casting_commander_from_command
            && !casting_from_graveyard_with_flashback
        {
            return Err(StateError::ObjectNotCastable(object));
        }
        let timing = if request.flash() {
            SpellTiming::Instant
        } else {
            request.timing()
        };
        if !self.can_cast_with_timing(player, timing) {
            return Err(StateError::InvalidSpellTiming);
        }
        let target_snapshots = self.capture_target_snapshots(
            player,
            Some(object),
            request.target_requirements(),
            request.target_choices(),
        )?;
        let effective_cost = self.effective_spell_request_cost(player, object, &request)?;
        let canonical_payment = validate_payment_plan(
            self.mana_pool(player)?,
            effective_cost,
            request.payment().paid(),
        )
        .map_err(Self::map_payment_error)?;
        if canonical_payment != request.payment() {
            return Err(StateError::InvalidPaymentPlan);
        }

        self.pay_mana(player, effective_cost, request.payment())?;
        self.move_object(object, ZoneId::new(None, ZoneKind::Stack))?;
        if casting_commander_from_command {
            self.record_commander_cast(object)?;
        }
        let id = self.push_stack_entry(StackEntryRequest {
            controller: player,
            object: Some(object),
            trigger: None,
            activated_ability: None,
            kind: request.kind(),
            targets: target_snapshots,
            payment: Some(request.payment()),
            copy_info: None,
            kicked: request.kicked(),
            flashback: request.flashback().is_some(),
            split_second: request.split_second(),
        });
        self.after_priority_action(player, true)?;
        Ok(id)
    }

    fn split_second_on_stack(&self) -> bool {
        self.stack_entries.iter().any(StackEntry::split_second)
    }

    /// Plays one land from the active player's hand as a main-phase special action.
    fn play_land(&mut self, player: PlayerId, object: ObjectId) -> Result<(), StateError> {
        self.require_player(player)?;
        self.require_priority_player(player)?;
        if self.active_player != Some(player)
            || !matches!(
                self.current_step,
                Some(Step::PrecombatMain | Step::PostcombatMain)
            )
            || !self.stack_entries.is_empty()
        {
            return Err(StateError::InvalidLandPlayTiming);
        }
        if self.object_zone(object) != Some(ZoneId::new(Some(player), ZoneKind::Hand))
            || !self.object_characteristics(object)?.types().land()
        {
            return Err(StateError::ObjectNotPlayableAsLand(object));
        }
        if self.players[player.index()].lands_played_this_turn >= 1 {
            return Err(StateError::LandPlayLimitReached(player));
        }

        self.move_object(object, ZoneId::new(None, ZoneKind::Battlefield))?;
        self.players[player.index()].lands_played_this_turn = self.players[player.index()]
            .lands_played_this_turn
            .checked_add(1)
            .ok_or(StateError::TurnNumberOverflow)?;
        self.after_priority_action(player, true)
    }

    /// Puts a spell object on the stack for the current priority player.
    ///
    /// This is a low-level stack helper retained for priority and stack tests.
    /// Use [`GameState::cast_spell`] for CR 601 legality, target, and payment
    /// validation.
    ///
    /// When `hold_priority` is true, priority remains with the caster as an
    /// explicit full-control choice. T1.3 keeps the same result either way
    /// because CR 117.3c gives priority back after a spell is cast.
    fn put_spell_on_stack(
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
        let id = self.push_stack_entry(StackEntryRequest {
            controller: player,
            object: Some(object),
            trigger: None,
            activated_ability: None,
            kind,
            targets: Vec::new(),
            payment: None,
            copy_info: None,
            kicked: false,
            flashback: false,
            split_second: false,
        });
        self.after_priority_action(player, hold_priority)?;
        Ok(id)
    }

    /// Puts an ability on top of the stack for the current priority player.
    fn put_ability_on_stack(
        &mut self,
        player: PlayerId,
        kind: StackObjectKind,
        hold_priority: bool,
    ) -> Result<StackEntryId, StateError> {
        self.require_priority_player(player)?;
        self.require_player(player)?;
        let id = self.push_stack_entry(StackEntryRequest {
            controller: player,
            object: None,
            trigger: None,
            activated_ability: None,
            kind,
            targets: Vec::new(),
            payment: None,
            copy_info: None,
            kicked: false,
            flashback: false,
            split_second: false,
        });
        self.after_priority_action(player, hold_priority)?;
        Ok(id)
    }

    /// Puts simultaneous triggered abilities on the stack in APNAP order.
    ///
    /// Entries controlled by the active player are placed lowest, followed by
    /// nonactive players in turn order. Within one controller's entries, the
    /// provided order is preserved.
    fn put_simultaneous_abilities_apnap(
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
                    ids.push(self.push_stack_entry(StackEntryRequest {
                        controller: player,
                        object: None,
                        trigger: None,
                        activated_ability: None,
                        kind,
                        targets: Vec::new(),
                        payment: None,
                        copy_info: None,
                        kicked: false,
                        flashback: false,
                        split_second: false,
                    }));
                }
            }
        }
        self.priority_pass_count = 0;
        Ok(ids)
    }

    /// Registers one data-only triggered ability subscription.
    fn register_triggered_ability(
        &mut self,
        definition: TriggerDefinition,
    ) -> Result<TriggerId, StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        let id = TriggerId(self.next_trigger);
        self.next_trigger = self.next_trigger.saturating_add(1);
        let event_kind = definition.condition().subscribed_event_kind();
        self.trigger_subscriptions.push(TriggerSubscription {
            id,
            definition,
            event_kind,
        });
        self.emit_event_without_triggers(GameEvent::TriggeredAbilityRegistered {
            trigger: id,
            controller: definition.controller(),
            source: definition.source(),
            event_kind,
            duration: definition.duration(),
        });
        Ok(id)
    }

    /// Puts pending triggered abilities onto the stack using APNAP ordering.
    fn put_pending_triggered_abilities_on_stack(
        &mut self,
    ) -> Result<Vec<StackEntryId>, StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let pending = core::mem::take(&mut self.pending_triggers);
        if pending.is_empty() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::with_capacity(pending.len());
        for player in self.apnap_players(active)? {
            for trigger in &pending {
                if trigger.controller() == player {
                    let id = self.push_stack_entry(StackEntryRequest {
                        controller: player,
                        object: None,
                        trigger: Some(trigger.trigger()),
                        activated_ability: None,
                        kind: StackObjectKind::TriggeredAbility,
                        targets: Vec::new(),
                        payment: None,
                        copy_info: None,
                        kicked: false,
                        flashback: false,
                        split_second: false,
                    });
                    self.emit_event_without_triggers(GameEvent::TriggeredAbilityPutOnStack {
                        trigger: trigger.trigger(),
                        entry: id,
                        controller: player,
                    });
                    ids.push(id);
                }
            }
        }
        let priority = self.deferred_priority_player.or(self.active_player);
        self.deferred_priority_player = None;
        self.grant_priority_to(priority)?;
        Ok(ids)
    }

    fn queue_triggered_abilities(&mut self, record: EventRecord) {
        if self.trigger_subscriptions.is_empty() {
            return;
        }
        let mut queued = Vec::new();
        let mut consumed_delayed = Vec::new();
        for subscription in &self.trigger_subscriptions {
            let event_kind_matches = subscription.event_kind == record.event().kind()
                || (matches!(
                    subscription.definition.condition(),
                    TriggerCondition::StackEntryAddedOrCopied { .. }
                ) && record.event().kind() == GameEventKind::StackEntryCopied);
            if !event_kind_matches {
                continue;
            }
            if !self.trigger_condition_matches(subscription.definition, record.event()) {
                continue;
            }
            if !self.trigger_intervening_if_matches(subscription.definition) {
                continue;
            }
            queued.push(PendingTriggeredAbility {
                trigger: subscription.id,
                controller: subscription.definition.controller(),
                source: subscription.definition.source(),
                event_sequence: record.sequence(),
                event_turn: record.turn(),
            });
            if subscription.definition.duration() == TriggerDuration::DelayedOnce {
                consumed_delayed.push(subscription.id);
            }
        }
        if !consumed_delayed.is_empty() {
            self.trigger_subscriptions
                .retain(|subscription| !consumed_delayed.contains(&subscription.id));
        }
        for trigger in queued {
            self.pending_triggers.push(trigger);
            self.emit_event_without_triggers(GameEvent::TriggeredAbilityQueued {
                trigger: trigger.trigger(),
                controller: trigger.controller(),
                event_sequence: trigger.event_sequence(),
            });
        }
    }

    fn trigger_condition_matches(&self, definition: TriggerDefinition, event: GameEvent) -> bool {
        match definition.condition() {
            TriggerCondition::EventKind(kind) => event.kind() == kind,
            TriggerCondition::ObjectMoved { object, from, to } => {
                if let GameEvent::ObjectMoved {
                    object: moved,
                    from: event_from,
                    to: event_to,
                } = event
                {
                    self.trigger_object_matches(definition, object, moved)
                        && self.trigger_zone_matches(definition, from, event_from)
                        && self.trigger_zone_matches(definition, to, event_to)
                } else {
                    false
                }
            }
            TriggerCondition::StepBegan { step } => {
                matches!(event, GameEvent::StepBegan { step: event_step } if event_step == step)
            }
            TriggerCondition::LifeLost { player } => {
                matches!(event, GameEvent::LifeLost { player: event_player, .. }
                    if self.trigger_player_matches(definition, player, event_player))
            }
            TriggerCondition::LifeGained { player } => {
                matches!(event, GameEvent::LifeGained { player: event_player, .. }
                    if self.trigger_player_matches(definition, player, event_player))
            }
            TriggerCondition::DamageMarked { object } => {
                matches!(event, GameEvent::DamageMarked { object: event_object, .. }
                    if self.trigger_object_matches(definition, object, event_object))
            }
            TriggerCondition::StackEntryResolved { kind, outcome } => {
                if let GameEvent::StackEntryResolved {
                    entry,
                    outcome: event_outcome,
                } = event
                {
                    let kind_matches = kind.map_or(true, |expected| {
                        self.resolution_log
                            .iter()
                            .find(|record| record.stack_entry() == entry)
                            .is_some_and(|record| record.kind() == expected)
                    });
                    let outcome_matches =
                        outcome.map_or(true, |expected| expected == event_outcome);
                    kind_matches && outcome_matches
                } else {
                    false
                }
            }
            TriggerCondition::AttackDeclared { attacker } => {
                matches!(event, GameEvent::AttackDeclared { attacker: event_attacker, .. }
                    if self.trigger_object_matches(definition, attacker, event_attacker))
            }
            TriggerCondition::StepBeganFor { step, player } => {
                matches!(event, GameEvent::StepBegan { step: event_step } if event_step == step)
                    && self.active_player.is_some_and(|active| {
                        self.trigger_player_matches(definition, player, active)
                    })
            }
            TriggerCondition::StackEntryAdded {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            } => {
                let GameEvent::StackEntryAdded {
                    controller: event_controller,
                    object: Some(object),
                    ..
                } = event
                else {
                    return false;
                };
                let Ok(characteristics) = self.object_characteristics(object) else {
                    return false;
                };
                self.trigger_player_matches(definition, controller, event_controller)
                    && characteristics.types().contains_all(required_types)
                    && (required_any_types == ObjectTypes::none()
                        || characteristics.types().intersects(required_any_types))
                    && !characteristics.types().intersects(forbidden_types)
            }
            TriggerCondition::CombatDamageToPlayer { source } => {
                matches!(event, GameEvent::CombatDamageDealt { record }
                if matches!(record.target(), CombatDamageTarget::Player(_))
                    && record.amount() > 0
                    && self.object_matches_target_predicate(
                        definition.controller(),
                        source,
                        record.source(),
                    ))
            }
            TriggerCondition::PlayerDrewCard { player } => {
                matches!(event, GameEvent::CardDrawn { player: event_player, .. }
                    if self.trigger_player_matches(definition, player, event_player))
            }
            TriggerCondition::PermanentEnteredBattlefield {
                predicate,
                exclude_source,
            } => {
                matches!(event, GameEvent::ObjectMoved { object, to, .. }
                if to == ZoneId::new(None, ZoneKind::Battlefield)
                    && (!exclude_source || definition.source() != Some(object))
                    && self.object_matches_target_predicate(
                        definition.controller(),
                        predicate,
                        object,
                    ))
            }
            TriggerCondition::StackEntryAddedOrCopied {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            } => {
                let (event_controller, object) = match event {
                    GameEvent::StackEntryAdded {
                        controller,
                        object: Some(object),
                        ..
                    } => (controller, object),
                    GameEvent::StackEntryCopied {
                        copy, controller, ..
                    } => {
                        let Some(object) = self
                            .stack_entries
                            .iter()
                            .find(|entry| entry.id() == copy)
                            .and_then(|entry| {
                                entry.object().or_else(|| {
                                    entry.copy_info().and_then(StackCopyInfo::source_object)
                                })
                            })
                        else {
                            return false;
                        };
                        (controller, object)
                    }
                    _ => return false,
                };
                let Ok(characteristics) = self.object_characteristics(object) else {
                    return false;
                };
                self.trigger_player_matches(definition, controller, event_controller)
                    && characteristics.types().contains_all(required_types)
                    && (required_any_types == ObjectTypes::none()
                        || characteristics.types().intersects(required_any_types))
                    && !characteristics.types().intersects(forbidden_types)
            }
        }
    }

    fn trigger_intervening_if_matches(&self, definition: TriggerDefinition) -> bool {
        match definition.intervening_if() {
            TriggerInterveningIf::Always => true,
            TriggerInterveningIf::SourceInZone(zone) => definition
                .source()
                .is_some_and(|source| self.object_zone(source) == Some(zone)),
            TriggerInterveningIf::ControllerControlsSource => {
                definition.source().is_some_and(|source| {
                    matches!(self.object_controller(source), Ok(controller) if controller == definition.controller())
                })
            }
            TriggerInterveningIf::ControllerLifeAtMost(max_life) => self
                .players
                .get(definition.controller().index())
                .is_some_and(|player| player.life <= max_life),
        }
    }

    fn trigger_object_matches(
        &self,
        definition: TriggerDefinition,
        filter: TriggerObjectFilter,
        object: ObjectId,
    ) -> bool {
        match filter {
            TriggerObjectFilter::Any => true,
            TriggerObjectFilter::Source => definition.source() == Some(object),
            TriggerObjectFilter::Object(expected) => object == expected,
            TriggerObjectFilter::AttachedToSource => definition
                .source()
                .filter(|source| {
                    self.object_zone(*source) == Some(ZoneId::new(None, ZoneKind::Battlefield))
                })
                .and_then(|source| self.objects.get(source))
                .and_then(ObjectRecord::attached_to)
                .is_some_and(|attached| attached == object),
        }
    }

    fn trigger_player_matches(
        &self,
        definition: TriggerDefinition,
        filter: TriggerPlayerFilter,
        player: PlayerId,
    ) -> bool {
        match filter {
            TriggerPlayerFilter::Any => true,
            TriggerPlayerFilter::Controller => player == definition.controller(),
            TriggerPlayerFilter::OpponentOfController => player != definition.controller(),
            TriggerPlayerFilter::Player(expected) => player == expected,
        }
    }

    fn trigger_zone_matches(
        &self,
        definition: TriggerDefinition,
        filter: TriggerZoneFilter,
        zone: ZoneId,
    ) -> bool {
        match filter {
            TriggerZoneFilter::Any => true,
            TriggerZoneFilter::Exact(expected) => zone == expected,
            TriggerZoneFilter::Kind(kind) => zone.kind() == kind,
            TriggerZoneFilter::Owned { owner, kind } => {
                zone.kind() == kind
                    && zone.owner().is_some_and(|zone_owner| {
                        self.trigger_player_matches(definition, owner, zone_owner)
                    })
            }
        }
    }

    /// Registers one data-only replacement/prevention effect.
    fn register_replacement_effect(
        &mut self,
        definition: ReplacementDefinition,
    ) -> Result<ReplacementEffectId, StateError> {
        self.validate_replacement_definition(definition)?;
        let id = ReplacementEffectId(self.next_replacement);
        self.next_replacement = self.next_replacement.saturating_add(1);
        self.replacement_effects
            .push(ReplacementSubscription { id, definition });
        self.emit_event_without_triggers(GameEvent::ReplacementEffectRegistered {
            replacement: id,
            controller: definition.controller(),
            source: definition.source(),
            operation: definition.operation(),
            duration: definition.duration(),
            self_replacement: definition.self_replacement(),
        });
        Ok(id)
    }

    fn validate_replacement_definition(
        &self,
        definition: ReplacementDefinition,
    ) -> Result<(), StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        match definition.condition() {
            ReplacementCondition::DamageWouldBeDealt { source, target, .. } => {
                if let ReplacementSourceFilter::Object(object) = source {
                    if self.objects.get(object).is_none() {
                        return Err(StateError::UnknownObject(object));
                    }
                }
                match target {
                    ReplacementDamageTargetFilter::Any => {}
                    ReplacementDamageTargetFilter::Player(player) => self.require_player(player)?,
                    ReplacementDamageTargetFilter::Object(object) => {
                        if self.objects.get(object).is_none() {
                            return Err(StateError::UnknownObject(object));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn set_replacement_choice_order(
        &mut self,
        chooser: PlayerId,
        order: Vec<ReplacementEffectId>,
    ) -> Result<(), StateError> {
        self.require_player(chooser)?;
        let mut seen = Vec::with_capacity(order.len());
        for replacement in &order {
            if seen.contains(replacement) {
                return Err(StateError::DuplicateReplacementEffect(*replacement));
            }
            if !self.replacement_effect_exists(*replacement) {
                return Err(StateError::UnknownReplacementEffect(*replacement));
            }
            seen.push(*replacement);
        }
        if let Some(existing) = self
            .replacement_choice_orders
            .iter_mut()
            .find(|existing| existing.chooser == chooser)
        {
            existing.order = order;
        } else {
            self.replacement_choice_orders
                .push(ReplacementChoiceOrder::new(chooser, order));
        }
        let count = self
            .replacement_choice_orders
            .iter()
            .find(|existing| existing.chooser == chooser)
            .map_or(0, |existing| existing.order.len() as u32);
        self.emit_event_without_triggers(GameEvent::ReplacementChoiceOrderSet { chooser, count });
        Ok(())
    }

    fn replacement_effect_exists(&self, id: ReplacementEffectId) -> bool {
        self.replacement_effects
            .iter()
            .any(|subscription| subscription.id == id)
    }

    /// Registers one data-only continuous effect.
    fn register_continuous_effect(
        &mut self,
        mut definition: ContinuousEffectDefinition,
    ) -> Result<ContinuousEffectId, StateError> {
        self.validate_continuous_effect_definition(&definition)?;
        let id = ContinuousEffectId(self.next_continuous_effect);
        self.next_continuous_effect = self.next_continuous_effect.saturating_add(1);
        if definition.timestamp == 0 {
            definition.timestamp = u64::from(id.get()) + 1;
        }
        self.continuous_effects.push(ContinuousEffectSubscription {
            id,
            definition: definition.clone(),
        });
        self.emit_event_without_triggers(GameEvent::ContinuousEffectRegistered {
            effect: id,
            controller: definition.controller(),
            source: definition.source(),
            target: definition.target(),
            operation: definition.operation(),
            layer: definition.operation().layer(),
            timestamp: definition.timestamp(),
        });
        Ok(id)
    }

    fn validate_continuous_effect_definition(
        &self,
        definition: &ContinuousEffectDefinition,
    ) -> Result<(), StateError> {
        self.require_player(definition.controller())?;
        if let Some(source) = definition.source() {
            if self.objects.get(source).is_none() {
                return Err(StateError::UnknownObject(source));
            }
        }
        match definition.target() {
            ContinuousEffectTarget::Object(object) => {
                if self.objects.get(object).is_none() {
                    return Err(StateError::UnknownObject(object));
                }
            }
            ContinuousEffectTarget::AllObjects | ContinuousEffectTarget::Objects { .. } => {}
            ContinuousEffectTarget::AttachedToSource => {
                if definition.source().is_none() {
                    return Err(StateError::UnknownObject(ObjectId(0)));
                }
            }
        }
        if definition.duration() == ContinuousEffectDuration::WhileSourceOnBattlefield
            && definition.source().is_none()
        {
            return Err(StateError::UnknownObject(ObjectId(0)));
        }
        if let ContinuousEffectOperation::CopyBaseCreature { from } = definition.operation() {
            if self.objects.get(from).is_none() {
                return Err(StateError::UnknownObject(from));
            }
        }
        let mut seen = Vec::with_capacity(definition.dependencies().len());
        for dependency in definition.dependencies() {
            if seen.contains(dependency) {
                return Err(StateError::DuplicateContinuousEffectDependency(*dependency));
            }
            if !self.continuous_effect_exists(*dependency) {
                return Err(StateError::UnknownContinuousEffect(*dependency));
            }
            seen.push(*dependency);
        }
        Ok(())
    }

    fn continuous_effect_exists(&self, effect: ContinuousEffectId) -> bool {
        self.continuous_effects
            .iter()
            .any(|subscription| subscription.id == effect)
    }

    fn apply_damage_replacement_effects(
        &mut self,
        mut event: DamageReplacementEvent,
    ) -> Result<DamageReplacementEvent, StateError> {
        if event.amount == 0 || self.replacement_effects.is_empty() {
            return Ok(event);
        }
        let chooser = self.damage_replacement_chooser(event.target)?;
        let mut applied = Vec::new();
        let mut consumed_once = Vec::new();

        while event.amount > 0 {
            let Some((id, definition)) = self.next_damage_replacement(event, chooser, &applied)
            else {
                break;
            };
            let original_amount = event.amount;
            event.amount = Self::apply_replacement_operation(definition.operation(), event.amount)?;
            applied.push(id);
            if definition.duration() == ReplacementDuration::Once {
                consumed_once.push(id);
            }
            self.emit_event(GameEvent::ReplacementEffectApplied {
                replacement: id,
                chooser,
                source: event.source,
                target: event.target,
                operation: definition.operation(),
                original_amount,
                resulting_amount: event.amount,
            });
        }

        if !consumed_once.is_empty() {
            self.replacement_effects
                .retain(|subscription| !consumed_once.contains(&subscription.id));
        }
        Ok(event)
    }

    fn next_damage_replacement(
        &self,
        event: DamageReplacementEvent,
        chooser: PlayerId,
        applied: &[ReplacementEffectId],
    ) -> Option<(ReplacementEffectId, ReplacementDefinition)> {
        let mut candidates = Vec::new();
        for subscription in &self.replacement_effects {
            if applied.contains(&subscription.id) {
                continue;
            }
            if self.replacement_condition_matches(subscription.definition, event) {
                candidates.push((subscription.id, subscription.definition));
            }
        }
        candidates.sort_by_key(|(id, definition)| {
            (
                u8::from(!definition.self_replacement()),
                self.replacement_order_rank(chooser, *id),
                id.0,
            )
        });
        candidates.into_iter().next()
    }

    fn replacement_order_rank(&self, chooser: PlayerId, id: ReplacementEffectId) -> usize {
        self.replacement_choice_orders
            .iter()
            .find(|order| order.chooser == chooser)
            .and_then(|order| order.order.iter().position(|ordered| *ordered == id))
            .unwrap_or(usize::MAX)
    }

    fn replacement_condition_matches(
        &self,
        definition: ReplacementDefinition,
        event: DamageReplacementEvent,
    ) -> bool {
        match definition.condition() {
            ReplacementCondition::DamageWouldBeDealt {
                source,
                target,
                combat_only,
            } => {
                (!combat_only || event.combat)
                    && Self::replacement_source_matches(definition, source, event.source)
                    && Self::replacement_target_matches(target, event.target)
            }
        }
    }

    fn replacement_source_matches(
        definition: ReplacementDefinition,
        filter: ReplacementSourceFilter,
        source: Option<ObjectId>,
    ) -> bool {
        match filter {
            ReplacementSourceFilter::Any => true,
            ReplacementSourceFilter::Source => source == definition.source(),
            ReplacementSourceFilter::Object(expected) => source == Some(expected),
        }
    }

    fn replacement_target_matches(
        filter: ReplacementDamageTargetFilter,
        target: CombatDamageTarget,
    ) -> bool {
        match (filter, target) {
            (ReplacementDamageTargetFilter::Any, _) => true,
            (
                ReplacementDamageTargetFilter::Player(expected),
                CombatDamageTarget::Player(player),
            ) => player == expected,
            (
                ReplacementDamageTargetFilter::Object(expected),
                CombatDamageTarget::Object(object),
            ) => object == expected,
            (ReplacementDamageTargetFilter::Player(_), CombatDamageTarget::Object(_))
            | (ReplacementDamageTargetFilter::Object(_), CombatDamageTarget::Player(_)) => false,
        }
    }

    fn damage_replacement_chooser(
        &self,
        target: CombatDamageTarget,
    ) -> Result<PlayerId, StateError> {
        match target {
            CombatDamageTarget::Player(player) => {
                self.require_player(player)?;
                Ok(player)
            }
            CombatDamageTarget::Object(object) => Ok(self
                .objects
                .get(object)
                .ok_or(StateError::UnknownObject(object))?
                .controller()),
        }
    }

    fn apply_replacement_operation(
        operation: ReplacementOperation,
        amount: u32,
    ) -> Result<u32, StateError> {
        match operation {
            ReplacementOperation::PreventAllDamage => Ok(0),
            ReplacementOperation::PreventDamage(prevented) => Ok(amount.saturating_sub(prevented)),
            ReplacementOperation::AddDamage(added) => amount
                .checked_add(added)
                .ok_or(StateError::CombatDamageOverflow),
            ReplacementOperation::DoubleDamage => amount
                .checked_mul(2)
                .ok_or(StateError::CombatDamageOverflow),
            ReplacementOperation::SetDamage(replacement) => Ok(replacement),
        }
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
            Step::CombatDamage
                if self.combat.damage_step == Some(CombatDamageStepKind::FirstStrike) =>
            {
                Step::CombatDamage
            }
            step => Self::next_normal_step(step),
        };
        self.begin_step(next)?;
        Ok(next)
    }

    /// Records whether the current combat has at least one attacker.
    ///
    /// This is the T1.2 hook for CR 508.8. Full attack declaration replaces it
    /// in T1.6, but keeping the flag here makes the step machine testable now.
    fn set_attackers_declared_this_combat(&mut self, attackers_declared: bool) {
        self.attackers_declared_this_combat = attackers_declared;
    }

    /// Declares attackers for the current declare attackers step.
    #[must_use]
    pub fn can_attack(&self, player: PlayerId, attack: AttackDeclaration) -> bool {
        self.current_step == Some(Step::DeclareAttackers)
            && self.active_player == Some(player)
            && self.priority_player == Some(player)
            && self.validate_attack_declaration(player, attack).is_ok()
    }

    /// Returns whether one block declaration is legal in the current block step.
    #[must_use]
    pub fn can_block(&self, defending_player: PlayerId, block: BlockDeclaration) -> bool {
        self.current_step == Some(Step::DeclareBlockers)
            && self.active_player != Some(defending_player)
            && self
                .validate_block_declaration(defending_player, block)
                .is_ok()
    }

    /// Declares attackers for the current declare attackers step.
    fn declare_attackers(
        &mut self,
        player: PlayerId,
        attacks: &[AttackDeclaration],
    ) -> Result<(), StateError> {
        self.require_combat_step(Step::DeclareAttackers)?;
        if self.active_player != Some(player) {
            return Err(StateError::InvalidCombatPlayer(player));
        }
        self.require_priority_player(player)?;
        let mut seen = Vec::with_capacity(attacks.len());
        for attack in attacks {
            if seen.contains(&attack.attacker()) {
                return Err(StateError::DuplicateCombatObject(attack.attacker()));
            }
            seen.push(attack.attacker());
            self.validate_attack_declaration(player, *attack)?;
        }

        self.combat = CombatState::new();
        self.emit_event(GameEvent::AttackersDeclared {
            player,
            count: attacks.len() as u32,
        });
        for attack in attacks {
            let keywords = self.creature_keywords(attack.attacker())?;
            if !keywords.vigilance() {
                {
                    self.objects
                        .get_mut(attack.attacker())
                        .ok_or(StateError::UnknownObject(attack.attacker()))?
                        .tapped = true;
                }
                self.emit_event(GameEvent::ObjectTapped {
                    object: attack.attacker(),
                    tapped: true,
                });
            }
            self.combat.attackers.push(AttackingCreature {
                object: attack.attacker(),
                defending_player: attack.defending_player(),
                blocked: false,
                blockers: Vec::new(),
            });
            self.emit_event(GameEvent::AttackDeclared {
                attacker: attack.attacker(),
                defending_player: attack.defending_player(),
            });
        }
        self.attackers_declared_this_combat = !attacks.is_empty();
        self.grant_priority_to(Some(player))?;
        Ok(())
    }

    /// Declares blockers for the current declare blockers step.
    fn declare_blockers(
        &mut self,
        defending_player: PlayerId,
        blocks: &[BlockDeclaration],
    ) -> Result<(), StateError> {
        self.require_combat_step(Step::DeclareBlockers)?;
        self.require_player(defending_player)?;
        if self.active_player == Some(defending_player) {
            return Err(StateError::InvalidCombatPlayer(defending_player));
        }
        let mut seen_blockers = Vec::with_capacity(blocks.len());
        for block in blocks {
            if seen_blockers.contains(&block.blocker()) {
                return Err(StateError::DuplicateCombatObject(block.blocker()));
            }
            seen_blockers.push(block.blocker());
            self.validate_block_declaration(defending_player, *block)?;
        }
        self.validate_menace_blocks(blocks)?;

        self.combat.blockers.clear();
        for attacker in &mut self.combat.attackers {
            attacker.blockers.clear();
            attacker.blocked = false;
        }
        self.emit_event(GameEvent::BlockersDeclared {
            defending_player,
            count: blocks.len() as u32,
        });
        for block in blocks {
            self.combat.blockers.push(BlockingCreature {
                object: block.blocker(),
                attacker: block.attacker(),
            });
            if let Some(attacker) = self
                .combat
                .attackers
                .iter_mut()
                .find(|attacker| attacker.object == block.attacker())
            {
                attacker.blocked = true;
                attacker.blockers.push(block.blocker());
            }
            self.emit_event(GameEvent::BlockDeclared {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        self.grant_priority_to(self.active_player)?;
        Ok(())
    }

    /// Assigns and deals combat damage for the current combat damage step.
    fn assign_combat_damage(
        &mut self,
        assignments: &[CombatDamageAssignmentRequest],
    ) -> Result<Vec<CombatDamageRecord>, StateError> {
        self.require_combat_step(Step::CombatDamage)?;
        let step = self
            .combat
            .damage_step
            .ok_or(StateError::InvalidCombatStep {
                expected: Step::CombatDamage,
                actual: self.current_step,
            })?;
        let eligible = self.eligible_combat_damage_sources()?;
        let mut seen_sources = Vec::with_capacity(assignments.len());
        for request in assignments {
            if seen_sources.contains(&request.source()) {
                return Err(StateError::DuplicateCombatObject(request.source()));
            }
            seen_sources.push(request.source());
            if !eligible.contains(&request.source()) {
                return Err(StateError::IllegalCombatDamageAssignment(request.source()));
            }
            self.validate_combat_damage_request(request)?;
        }
        for source in &eligible {
            let profile = self.combat_damage_profile(*source)?;
            let must_assign = profile.required_total > 0;
            let supplied = assignments
                .iter()
                .any(|request| request.source() == *source);
            if must_assign && !supplied {
                return Err(StateError::MissingCombatDamageAssignment(*source));
            }
            if !must_assign && supplied {
                return Err(StateError::IllegalCombatDamageAssignment(*source));
            }
        }

        let mut records = Vec::new();
        for request in assignments {
            let keywords = self.creature_keywords(request.source())?;
            for assignment in request.assignments() {
                if assignment.amount() == 0 {
                    continue;
                }
                let record = CombatDamageRecord {
                    source: request.source(),
                    target: assignment.target(),
                    amount: assignment.amount(),
                    step,
                    source_had_deathtouch: keywords.deathtouch(),
                    source_had_lifelink: keywords.lifelink(),
                };
                if let Some(record) = self.apply_combat_damage(record)? {
                    self.emit_event(GameEvent::CombatDamageDealt { record });
                    records.push(record);
                }
            }
        }
        self.combat.damage_records.extend(records.iter().copied());
        self.perform_state_based_actions()?;
        Ok(records)
    }

    /// Requests the CR 514.3a cleanup exception after cleanup actions finish.
    fn request_cleanup_priority(&mut self) {
        self.cleanup_priority_requested = true;
        self.emit_event(GameEvent::CleanupPriorityRequested);
    }

    /// Adds a placeholder duration marker.
    fn add_duration_marker(&mut self, duration: EffectDuration) -> DurationMarkerId {
        let id = DurationMarkerId(self.next_duration_marker);
        self.next_duration_marker = self.next_duration_marker.saturating_add(1);
        self.duration_markers.push(DurationMarker { id, duration });
        self.emit_event(GameEvent::DurationMarkerAdded {
            marker: id,
            duration,
        });
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
    fn create_object(
        &mut self,
        card: CardId,
        owner: PlayerId,
        controller: PlayerId,
        zone: ZoneId,
    ) -> Result<ObjectId, StateError> {
        self.require_player(owner)?;
        self.require_player(controller)?;
        self.require_zone(zone)?;
        let object = self.objects.push(card, owner, controller, self.turn_number);
        self.zone_mut(zone)?.objects_mut().push(object);
        self.emit_event(GameEvent::ObjectCreated {
            object,
            card,
            owner,
            controller,
            zone,
        });
        Ok(object)
    }

    /// Creates one token on the battlefield.
    fn create_token(
        &mut self,
        card: CardId,
        owner: PlayerId,
        controller: PlayerId,
        base_object: BaseObjectCharacteristics,
        base: Option<BaseCreatureCharacteristics>,
    ) -> Result<ObjectId, StateError> {
        let object = self.create_object(
            card,
            owner,
            controller,
            ZoneId::new(None, ZoneKind::Battlefield),
        )?;
        {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            record.token = true;
            record.base_object = base_object;
            record.base_creature = base;
        }
        self.emit_event(GameEvent::BaseObjectCharacteristicsSet {
            object,
            base: base_object,
        });
        self.emit_event(GameEvent::TokenCreated {
            object,
            card,
            owner,
            controller,
        });
        if let Some(base) = base {
            self.emit_event(GameEvent::BaseCreatureCharacteristicsSet { object, base });
        }
        Ok(object)
    }

    /// Creates one permanent copy object on the battlefield.
    fn create_permanent_copy(
        &mut self,
        source: ObjectId,
        owner: PlayerId,
        controller: PlayerId,
        token: bool,
    ) -> Result<ObjectId, StateError> {
        self.require_player(owner)?;
        self.require_player(controller)?;
        let source_record = self
            .objects
            .get(source)
            .ok_or(StateError::UnknownObject(source))?;
        let object = self.create_object(
            source_record.card(),
            owner,
            controller,
            ZoneId::new(None, ZoneKind::Battlefield),
        )?;
        {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            record.token = token;
            record.copy_source = Some(source);
            record.base_object = source_record.base_object();
            record.base_creature = source_record.base_creature();
        }
        if token {
            self.emit_event(GameEvent::TokenCreated {
                object,
                card: source_record.card(),
                owner,
                controller,
            });
        }
        self.emit_event(GameEvent::ObjectCopyCreated {
            object,
            source,
            token,
        });
        self.emit_event(GameEvent::BaseObjectCharacteristicsSet {
            object,
            base: source_record.base_object(),
        });
        if let Some(base) = source_record.base_creature() {
            self.emit_event(GameEvent::BaseCreatureCharacteristicsSet { object, base });
        }
        Ok(object)
    }

    /// Copies one stack entry without creating a physical card object.
    fn copy_stack_entry(
        &mut self,
        player: PlayerId,
        entry: StackEntryId,
    ) -> Result<StackEntryId, StateError> {
        self.require_player(player)?;
        let source = self
            .stack_entries
            .iter()
            .find(|candidate| candidate.id() == entry)
            .cloned()
            .ok_or(StateError::UnknownStackEntry(entry))?;
        let copy = self.push_stack_entry(StackEntryRequest {
            controller: player,
            object: None,
            trigger: source.trigger(),
            activated_ability: source.activated_ability(),
            kind: source.kind(),
            targets: source.targets().to_vec(),
            payment: source.payment(),
            copy_info: Some(StackCopyInfo::new(source.id(), source.object())),
            kicked: source.kicked(),
            flashback: source.flashback(),
            split_second: source.split_second(),
        });
        self.emit_event(GameEvent::StackEntryCopied {
            source: entry,
            copy,
            controller: player,
        });
        Ok(copy)
    }

    /// Moves an object from its current zone to another zone.
    fn move_object(&mut self, object: ObjectId, to: ZoneId) -> Result<(), StateError> {
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
        self.remember_hidden_transition(object, from_zone_id, to);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let from_position = self.zones[from_index]
            .objects
            .iter()
            .position(|candidate| *candidate == object)
            .ok_or(StateError::MissingZoneMembership(object))?;
        Arc::make_mut(&mut self.zones)[from_index]
            .objects_mut()
            .remove(from_position);
        self.zone_mut(to)?.objects_mut().push(object);
        if from_zone_id == battlefield && to != battlefield {
            self.remove_object_from_combat(object);
            self.restrictions.retain(|subscription| {
                !matches!(
                    subscription.definition.effect(),
                    RestrictionEffect::RegenerationShield { object: protected }
                        if protected == object
                )
            });
            if let Some(record) = self.objects.get_mut(object) {
                record.damage_marked = 0;
                record.deathtouch_damage_marked = false;
            }
            self.clear_object_counters(object);
        }
        if from_zone_id != battlefield && to == battlefield {
            if let Some(record) = self.objects.get_mut(object) {
                record.controlled_since_turn = self.turn_number;
                record.damage_marked = 0;
                record.deathtouch_damage_marked = false;
                record.tapped = record.base_object.enters_tapped();
            }
        }
        self.emit_event(GameEvent::ObjectMoved {
            object,
            from: from_zone_id,
            to,
        });
        Ok(())
    }

    fn clear_object_counters(&mut self, object: ObjectId) {
        let cleared: Vec<ObjectCounterEntry> = self
            .object_counters
            .iter()
            .copied()
            .filter(|entry| entry.object == object && entry.count > 0)
            .collect();
        self.object_counters.retain(|entry| entry.object != object);
        for entry in cleared {
            self.emit_event(GameEvent::ObjectCountersChanged {
                object,
                kind: entry.kind,
                count: 0,
            });
        }
        let cleared_loyalty = if let Some(record) = self.objects.get_mut(object) {
            let had_loyalty = record.loyalty.is_some();
            if had_loyalty {
                record.loyalty = None;
            }
            had_loyalty
        } else {
            false
        };
        if cleared_loyalty {
            self.emit_event(GameEvent::ObjectCountersChanged {
                object,
                kind: CounterKind::Loyalty,
                count: 0,
            });
            self.emit_event(GameEvent::ObjectLoyaltySet {
                object,
                loyalty: None,
            });
        }
    }

    /// Returns the zone currently containing an object.
    #[must_use]
    pub fn object_zone(&self, object: ObjectId) -> Option<ZoneId> {
        self.zones
            .iter()
            .find(|zone| zone.objects.contains(&object))
            .map(Zone::id)
    }

    /// Returns object IDs in one zone from bottom/front to top/back order.
    #[must_use]
    pub fn zone_objects(&self, zone: ZoneId) -> Option<&[ObjectId]> {
        self.zones
            .iter()
            .find(|candidate| candidate.id == zone)
            .map(|candidate| candidate.objects.as_slice())
    }

    /// Validates that every object appears in exactly one zone.
    pub fn validate_zone_conservation(&self) -> Result<ZoneConservation, StateError> {
        let mut memberships = vec![0_u8; self.objects.len()];
        for zone in self.zones.iter() {
            self.validate_zone_id(zone.id)?;
            for object in zone.objects.iter() {
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
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = CanonicalBytes::default();
        bytes.write_u64(self.seed);
        bytes.write_u64(self.rng_state);
        bytes.write_u32(self.turn_number);
        bytes.write_game_outcome(self.outcome);
        bytes.write_optional_player(self.starting_player);
        bytes.write_bool(self.opening_hands_drawn);
        bytes.write_optional_player(self.active_player);
        bytes.write_optional_player(self.priority_player);
        bytes.write_u32(self.priority_pass_count);
        bytes.write_optional_step(self.current_step);
        bytes.write_u32(self.cleanup_iteration);
        bytes.write_bool(self.cleanup_priority_requested);
        bytes.write_bool(self.cleanup_repeat_pending);
        bytes.write_bool(self.attackers_declared_this_combat);
        bytes.write_cleanup_report(self.last_cleanup_report);
        bytes.write_u8(self.range_of_influence().canonical_code());
        bytes.write_u32(self.players.len() as u32);
        for player in &self.players {
            bytes.write_u32(player.id.0);
            bytes.write_i32(player.life);
            bytes.write_u32(player.poison);
            bytes.write_bool(player.lost);
            bytes.write_u32(player.max_hand_size);
            bytes.write_u32(player.mulligans_taken);
            bytes.write_bool(player.opening_hand_kept);
            bytes.write_mana_pool(player.mana_pool);
            bytes.write_u32(player.lands_played_this_turn);
        }
        bytes.write_u32(self.turn_order.len() as u32);
        for player in &self.turn_order {
            bytes.write_u32(player.0);
        }

        bytes.write_u32(self.objects.len() as u32);
        for object in self.objects.iter() {
            bytes.write_u32(object.id.0);
            bytes.write_u32(object.card.0);
            bytes.write_u32(object.owner.0);
            bytes.write_u32(object.controller.0);
            bytes.write_bool(object.tapped);
            bytes.write_base_object_characteristics(object.base_object());
            bytes.write_optional_base_creature_characteristics(object.base_creature());
            bytes.write_u32(object.damage_marked);
            bytes.write_bool(object.deathtouch_damage_marked);
            bytes.write_optional_i32(object.loyalty);
            bytes.write_bool(object.token);
            bytes.write_optional_object(object.copy_source);
            bytes.write_optional_object(object.attached_to);
            bytes.write_u32(object.controlled_since_turn);
            bytes.write_object_colors(object.color_identity);
            bytes.write_bool(object.commander);
            bytes.write_u32(object.commander_cast_count);
        }

        bytes.write_u32(self.zones.len() as u32);
        for zone in self.zones.iter() {
            bytes.write_zone_id(zone.id);
            bytes.write_u32(zone.objects.len() as u32);
            for object in zone.objects.iter() {
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
            bytes.write_stack_entry(entry);
        }
        bytes.write_u32(self.resolution_log.len() as u32);
        for resolution in &self.resolution_log {
            bytes.write_resolution_record(resolution);
        }
        bytes.write_u32(self.next_trigger);
        bytes.write_u32(self.trigger_subscriptions.len() as u32);
        for subscription in &self.trigger_subscriptions {
            bytes.write_trigger_subscription(*subscription);
        }
        bytes.write_u32(self.pending_triggers.len() as u32);
        for trigger in &self.pending_triggers {
            bytes.write_pending_trigger(*trigger);
        }
        bytes.write_u32(self.next_activated_ability);
        bytes.write_u32(self.activated_abilities.len() as u32);
        for ability in &self.activated_abilities {
            bytes.write_activated_ability_subscription(*ability);
        }
        bytes.write_u32(self.next_cost_modifier);
        bytes.write_u32(self.cost_modifiers.len() as u32);
        for modifier in &self.cost_modifiers {
            bytes.write_cost_modifier_subscription(*modifier);
        }
        bytes.write_u32(self.next_restriction);
        bytes.write_u32(self.restrictions.len() as u32);
        for restriction in &self.restrictions {
            bytes.write_restriction_subscription(*restriction);
        }
        bytes.write_u32(self.object_counters.len() as u32);
        for counter in &self.object_counters {
            bytes.write_object_counter_entry(*counter);
        }
        bytes.write_u32(self.loyalty_activations_this_turn.len() as u32);
        for object in &self.loyalty_activations_this_turn {
            bytes.write_u32(object.0);
        }
        bytes.write_u32(self.next_replacement);
        bytes.write_u32(self.replacement_effects.len() as u32);
        for replacement in &self.replacement_effects {
            bytes.write_replacement_subscription(*replacement);
        }
        bytes.write_u32(self.replacement_choice_orders.len() as u32);
        for order in &self.replacement_choice_orders {
            bytes.write_replacement_choice_order(order);
        }
        bytes.write_u32(self.next_continuous_effect);
        bytes.write_u32(self.continuous_effects.len() as u32);
        for effect in &self.continuous_effects {
            bytes.write_continuous_effect_subscription(effect);
        }
        bytes.write_optional_player(self.deferred_priority_player);
        bytes.write_u64(self.next_event_sequence);
        bytes.write_u32(self.turn_events.len() as u32);
        for event in self.turn_events.iter() {
            bytes.write_event_record(*event);
        }
        bytes.write_combat_state(&self.combat);
        bytes.write_u32(self.empty_library_draws_since_sba.len() as u32);
        for player in &self.empty_library_draws_since_sba {
            bytes.write_u32(player.0);
        }
        if !self.known_objects.is_empty() {
            bytes.write_u32(0x4b4e_4f57);
            bytes.write_u32(self.known_objects.len() as u32);
            for known in self.known_objects.iter() {
                bytes.write_u32(known.observer.0);
                bytes.write_u32(known.object.0);
            }
        }
        bytes.finish()
    }

    /// Computes the canonical FNV-1a state hash without allocating bytes.
    #[must_use]
    pub fn deterministic_hash_streaming(&self) -> StateHash {
        let mut hash = Fnva64::new();
        hash.write_u64(self.seed);
        hash.write_u64(self.rng_state);
        hash.write_u32(self.turn_number);
        hash.write_game_outcome(self.outcome);
        hash.write_optional_player(self.starting_player);
        hash.write_bool(self.opening_hands_drawn);
        hash.write_optional_player(self.active_player);
        hash.write_optional_player(self.priority_player);
        hash.write_u32(self.priority_pass_count);
        hash.write_optional_step(self.current_step);
        hash.write_u32(self.cleanup_iteration);
        hash.write_bool(self.cleanup_priority_requested);
        hash.write_bool(self.cleanup_repeat_pending);
        hash.write_bool(self.attackers_declared_this_combat);
        hash.write_cleanup_report(self.last_cleanup_report);
        hash.write_u8(self.range_of_influence().canonical_code());
        hash.write_u32(self.players.len() as u32);
        for player in &self.players {
            hash.write_u32(player.id.0);
            hash.write_i32(player.life);
            hash.write_u32(player.poison);
            hash.write_bool(player.lost);
            hash.write_u32(player.max_hand_size);
            hash.write_u32(player.mulligans_taken);
            hash.write_bool(player.opening_hand_kept);
            hash.write_mana_pool(player.mana_pool);
            hash.write_u32(player.lands_played_this_turn);
        }
        hash.write_u32(self.turn_order.len() as u32);
        for player in &self.turn_order {
            hash.write_u32(player.0);
        }

        hash.write_u32(self.objects.len() as u32);
        for object in self.objects.iter() {
            hash.write_u32(object.id.0);
            hash.write_u32(object.card.0);
            hash.write_u32(object.owner.0);
            hash.write_u32(object.controller.0);
            hash.write_bool(object.tapped);
            hash.write_base_object_characteristics(object.base_object());
            hash.write_optional_base_creature_characteristics(object.base_creature());
            hash.write_u32(object.damage_marked);
            hash.write_bool(object.deathtouch_damage_marked);
            hash.write_optional_i32(object.loyalty);
            hash.write_bool(object.token);
            hash.write_optional_object(object.copy_source);
            hash.write_optional_object(object.attached_to);
            hash.write_u32(object.controlled_since_turn);
            hash.write_object_colors(object.color_identity);
            hash.write_bool(object.commander);
            hash.write_u32(object.commander_cast_count);
        }

        hash.write_u32(self.zones.len() as u32);
        for zone in self.zones.iter() {
            hash.write_zone_id(zone.id);
            hash.write_u32(zone.objects.len() as u32);
            for object in zone.objects.iter() {
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
            hash.write_stack_entry(entry);
        }
        hash.write_u32(self.resolution_log.len() as u32);
        for resolution in &self.resolution_log {
            hash.write_resolution_record(resolution);
        }
        hash.write_u32(self.next_trigger);
        hash.write_u32(self.trigger_subscriptions.len() as u32);
        for subscription in &self.trigger_subscriptions {
            hash.write_trigger_subscription(*subscription);
        }
        hash.write_u32(self.pending_triggers.len() as u32);
        for trigger in &self.pending_triggers {
            hash.write_pending_trigger(*trigger);
        }
        hash.write_u32(self.next_activated_ability);
        hash.write_u32(self.activated_abilities.len() as u32);
        for ability in &self.activated_abilities {
            hash.write_activated_ability_subscription(*ability);
        }
        hash.write_u32(self.next_cost_modifier);
        hash.write_u32(self.cost_modifiers.len() as u32);
        for modifier in &self.cost_modifiers {
            hash.write_cost_modifier_subscription(*modifier);
        }
        hash.write_u32(self.next_restriction);
        hash.write_u32(self.restrictions.len() as u32);
        for restriction in &self.restrictions {
            hash.write_restriction_subscription(*restriction);
        }
        hash.write_u32(self.object_counters.len() as u32);
        for counter in &self.object_counters {
            hash.write_object_counter_entry(*counter);
        }
        hash.write_u32(self.loyalty_activations_this_turn.len() as u32);
        for object in &self.loyalty_activations_this_turn {
            hash.write_u32(object.0);
        }
        hash.write_u32(self.next_replacement);
        hash.write_u32(self.replacement_effects.len() as u32);
        for replacement in &self.replacement_effects {
            hash.write_replacement_subscription(*replacement);
        }
        hash.write_u32(self.replacement_choice_orders.len() as u32);
        for order in &self.replacement_choice_orders {
            hash.write_replacement_choice_order(order);
        }
        hash.write_u32(self.next_continuous_effect);
        hash.write_u32(self.continuous_effects.len() as u32);
        for effect in &self.continuous_effects {
            hash.write_continuous_effect_subscription(effect);
        }
        hash.write_optional_player(self.deferred_priority_player);
        hash.write_u64(self.next_event_sequence);
        hash.write_u32(self.turn_events.len() as u32);
        for event in self.turn_events.iter() {
            hash.write_event_record(*event);
        }
        hash.write_combat_state(&self.combat);
        hash.write_u32(self.empty_library_draws_since_sba.len() as u32);
        for player in &self.empty_library_draws_since_sba {
            hash.write_u32(player.0);
        }
        if !self.known_objects.is_empty() {
            hash.write_u32(0x4b4e_4f57);
            hash.write_u32(self.known_objects.len() as u32);
            for known in self.known_objects.iter() {
                hash.write_u32(known.observer.0);
                hash.write_u32(known.object.0);
            }
        }

        StateHash(hash.finish())
    }

    fn begin_step(&mut self, step: Step) -> Result<(), StateError> {
        self.current_step = Some(step);
        self.emit_event(GameEvent::StepBegan { step });
        self.expire_step_begin_markers(step);
        match step {
            Step::Untap => {
                self.untap_active_player_permanents()?;
                self.priority_player = None;
            }
            Step::Draw => {
                if !self.should_skip_first_turn_draw() {
                    self.draw_turn_card()?;
                }
                self.assign_normal_priority(step)?;
            }
            Step::BeginningOfCombat => {
                self.attackers_declared_this_combat = false;
                self.combat = CombatState::new();
                self.assign_normal_priority(step)?;
            }
            Step::CombatDamage => {
                self.begin_combat_damage_step();
                self.assign_normal_priority(step)?;
            }
            Step::Cleanup => self.begin_cleanup_step()?,
            _ => self.assign_normal_priority(step)?,
        }
        Ok(())
    }

    fn untap_active_player_permanents(&mut self) -> Result<(), StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let objects = self
            .zone_objects(battlefield)
            .ok_or(StateError::UnknownZone(battlefield))?
            .to_vec();
        for object in objects {
            let should_untap = self
                .objects
                .get(object)
                .is_some_and(|record| record.controller() == active && record.tapped());
            if !should_untap {
                continue;
            }
            self.objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?
                .tapped = false;
            self.emit_event(GameEvent::ObjectTapped {
                object,
                tapped: false,
            });
        }
        Ok(())
    }

    fn should_skip_first_turn_draw(&self) -> bool {
        self.turn_number == 1
            && self.players.len() > 1
            && self.starting_player.is_some()
            && self.active_player == self.starting_player
    }

    fn end_step(&mut self, step: Step) {
        self.emit_event(GameEvent::StepEnded { step });
        self.priority_player = None;
        self.priority_pass_count = 0;
        self.clear_all_mana_pools();
        if step == Step::EndOfCombat {
            self.expire_end_of_combat_markers();
            self.combat = CombatState::new();
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
        self.combat = CombatState::new();
        self.loyalty_activations_this_turn.clear();
        for player in &mut self.players {
            player.lands_played_this_turn = 0;
        }
        self.reset_turn_events();
        self.emit_event(GameEvent::TurnStarted {
            turn: self.turn_number,
            active_player: next_active,
        });
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

    fn assign_normal_priority(&mut self, step: Step) -> Result<(), StateError> {
        let priority_player = if step.receives_priority_normally() {
            self.active_player
        } else {
            None
        };
        self.grant_priority_to(priority_player)
    }

    fn begin_cleanup_step(&mut self) -> Result<(), StateError> {
        self.cleanup_iteration = self.cleanup_iteration.saturating_add(1);
        self.last_cleanup_report = self.perform_cleanup_actions()?;
        self.emit_event(GameEvent::CleanupPerformed {
            report: self.last_cleanup_report,
        });
        self.clear_damage_marked();
        let sba_report = self.perform_state_based_actions()?;
        let grant_priority = self.cleanup_priority_requested;
        self.cleanup_priority_requested = false;
        let needs_priority = grant_priority || sba_report.actions_performed() > 0;
        self.cleanup_repeat_pending = needs_priority && self.outcome == GameOutcome::InProgress;
        let new_priority = if needs_priority && self.outcome == GameOutcome::InProgress {
            self.active_player
        } else {
            None
        };
        self.priority_player = new_priority;
        self.priority_pass_count = 0;
        Ok(())
    }

    fn require_combat_step(&self, expected: Step) -> Result<(), StateError> {
        if self.current_step == Some(expected) {
            Ok(())
        } else {
            Err(StateError::InvalidCombatStep {
                expected,
                actual: self.current_step,
            })
        }
    }

    fn validate_attack_declaration(
        &self,
        player: PlayerId,
        attack: AttackDeclaration,
    ) -> Result<(), StateError> {
        let record = self
            .objects
            .get(attack.attacker())
            .ok_or(StateError::UnknownObject(attack.attacker()))?;
        if self.object_controller(attack.attacker())? != player
            || self.object_zone(attack.attacker()) != Some(ZoneId::new(None, ZoneKind::Battlefield))
        {
            return Err(StateError::IllegalAttack(attack.attacker()));
        }
        let creature = self.creature_characteristics(attack.attacker())?;
        if record.tapped() {
            return Err(StateError::CreatureTapped(attack.attacker()));
        }
        if record.controlled_since_turn() == self.turn_number && !creature.keywords().haste() {
            return Err(StateError::SummoningSick(attack.attacker()));
        }
        if creature.keywords().defender() {
            return Err(StateError::IllegalAttack(attack.attacker()));
        }
        self.require_player(attack.defending_player())?;
        if attack.defending_player() == player {
            return Err(StateError::IllegalAttack(attack.attacker()));
        }
        if self.combat_restriction_applies(attack.attacker(), CombatRestriction::CannotAttack) {
            return Err(StateError::IllegalAttack(attack.attacker()));
        }
        Ok(())
    }

    fn validate_block_declaration(
        &self,
        defending_player: PlayerId,
        block: BlockDeclaration,
    ) -> Result<(), StateError> {
        let blocker = self
            .objects
            .get(block.blocker())
            .ok_or(StateError::UnknownObject(block.blocker()))?;
        if self.object_controller(block.blocker())? != defending_player
            || self.object_zone(block.blocker()) != Some(ZoneId::new(None, ZoneKind::Battlefield))
        {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        self.creature_characteristics(block.blocker())?;
        if blocker.tapped() {
            return Err(StateError::CreatureTapped(block.blocker()));
        }
        if self.combat_restriction_applies(block.blocker(), CombatRestriction::CannotBlock) {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        let Some(attacker) = self
            .combat
            .attackers
            .iter()
            .find(|attacker| attacker.object == block.attacker())
        else {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        };
        if attacker.defending_player != defending_player {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        if self.combat_restriction_applies(block.attacker(), CombatRestriction::CannotBeBlocked) {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        let attacker_keywords = self.creature_keywords(block.attacker())?;
        let blocker_keywords = self.creature_keywords(block.blocker())?;
        if attacker_keywords.flying() && !(blocker_keywords.flying() || blocker_keywords.reach()) {
            return Err(StateError::IllegalBlock {
                blocker: block.blocker(),
                attacker: block.attacker(),
            });
        }
        Ok(())
    }

    fn combat_restriction_applies(&self, object: ObjectId, restriction: CombatRestriction) -> bool {
        self.restrictions.iter().any(|subscription| {
            if !self.restriction_source_is_active(subscription.definition) {
                return false;
            }
            let RestrictionEffect::Combat {
                subject,
                restriction: active,
            } = subscription.definition.effect()
            else {
                return false;
            };
            active == restriction && self.combat_restriction_subject_matches(subject, object)
        })
    }

    fn combat_restriction_subject_matches(
        &self,
        subject: CombatRestrictionSubject,
        object: ObjectId,
    ) -> bool {
        match subject {
            CombatRestrictionSubject::Object(target) => target == object,
            CombatRestrictionSubject::ControlledBy(player) => self
                .object_controller(object)
                .is_ok_and(|controller| controller == player),
            CombatRestrictionSubject::AllObjects => true,
        }
    }

    fn validate_menace_blocks(&self, blocks: &[BlockDeclaration]) -> Result<(), StateError> {
        for attacker in &self.combat.attackers {
            if self.creature_keywords(attacker.object)?.menace() {
                let block_count = blocks
                    .iter()
                    .filter(|block| block.attacker() == attacker.object)
                    .count();
                if block_count == 1 {
                    let blocker = blocks
                        .iter()
                        .find(|block| block.attacker() == attacker.object)
                        .map_or(ObjectId(u32::MAX), |block| block.blocker());
                    return Err(StateError::IllegalBlock {
                        blocker,
                        attacker: attacker.object,
                    });
                }
            }
        }
        Ok(())
    }

    fn begin_combat_damage_step(&mut self) {
        match self.combat.damage_step {
            Some(CombatDamageStepKind::FirstStrike) => {
                self.combat.damage_step = Some(CombatDamageStepKind::Regular);
            }
            _ => {
                self.combat.first_strike_participants = self
                    .active_combat_creatures()
                    .into_iter()
                    .filter(|object| {
                        self.creature_keywords(*object).is_ok_and(|keywords| {
                            keywords.first_strike() || keywords.double_strike()
                        })
                    })
                    .collect();
                self.combat.damage_step = if self.combat.first_strike_participants.is_empty() {
                    Some(CombatDamageStepKind::Normal)
                } else {
                    Some(CombatDamageStepKind::FirstStrike)
                };
            }
        }
    }

    fn eligible_combat_damage_sources(&self) -> Result<Vec<ObjectId>, StateError> {
        let step = self
            .combat
            .damage_step
            .ok_or(StateError::InvalidCombatStep {
                expected: Step::CombatDamage,
                actual: self.current_step,
            })?;
        let mut sources = Vec::new();
        for object in self.active_combat_creatures() {
            let keywords = self.creature_keywords(object)?;
            let eligible = match step {
                CombatDamageStepKind::Normal => true,
                CombatDamageStepKind::FirstStrike => {
                    keywords.first_strike() || keywords.double_strike()
                }
                CombatDamageStepKind::Regular => {
                    !self.combat.first_strike_participants.contains(&object)
                        || keywords.double_strike()
                }
            };
            if eligible {
                sources.push(object);
            }
        }
        Ok(sources)
    }

    fn validate_combat_damage_request(
        &self,
        request: &CombatDamageAssignmentRequest,
    ) -> Result<(), StateError> {
        let profile = self.combat_damage_profile(request.source())?;
        let mut total = 0_u32;
        for assignment in request.assignments() {
            if !profile.legal_targets.contains(&assignment.target()) {
                return Err(StateError::IllegalCombatDamageAssignment(request.source()));
            }
            total = total
                .checked_add(assignment.amount())
                .ok_or(StateError::CombatDamageOverflow)?;
        }
        if total != profile.required_total {
            return Err(StateError::IllegalCombatDamageAssignment(request.source()));
        }
        self.validate_blocker_assignment_order(request)?;
        if !profile.trample_blockers.is_empty() {
            self.validate_trample_assignment(request, &profile)?;
        }
        Ok(())
    }

    fn validate_trample_assignment(
        &self,
        request: &CombatDamageAssignmentRequest,
        profile: &CombatDamageProfile,
    ) -> Result<(), StateError> {
        let assigned_to_defender: u32 = request
            .assignments()
            .iter()
            .filter(|assignment| assignment.target() == profile.trample_defender)
            .map(|assignment| assignment.amount())
            .sum();
        if assigned_to_defender == 0 {
            return Ok(());
        }
        let source_keywords = self.creature_keywords(request.source())?;
        for blocker in &profile.trample_blockers {
            let assigned_to_blocker: u32 = request
                .assignments()
                .iter()
                .filter(|assignment| assignment.target() == CombatDamageTarget::Object(*blocker))
                .map(|assignment| assignment.amount())
                .sum();
            if assigned_to_blocker < self.lethal_damage_required(*blocker, source_keywords)? {
                return Err(StateError::IllegalCombatDamageAssignment(request.source()));
            }
        }
        Ok(())
    }

    fn validate_blocker_assignment_order(
        &self,
        request: &CombatDamageAssignmentRequest,
    ) -> Result<(), StateError> {
        let Some(attacker) = self
            .combat
            .attackers
            .iter()
            .find(|attacker| attacker.object == request.source())
        else {
            return Ok(());
        };
        if !attacker.blocked {
            return Ok(());
        }
        let current_blockers: Vec<ObjectId> = attacker
            .blockers
            .iter()
            .copied()
            .filter(|blocker| self.is_active_blocking_creature(*blocker))
            .collect();
        if current_blockers.len() < 2 {
            return Ok(());
        }

        let source_keywords = self.creature_keywords(request.source())?;
        let assigned_to = |target: CombatDamageTarget| -> u32 {
            request
                .assignments()
                .iter()
                .filter(|assignment| assignment.target() == target)
                .map(|assignment| assignment.amount())
                .sum()
        };
        let assigned_to_defender =
            assigned_to(CombatDamageTarget::Player(attacker.defending_player));
        for (index, blocker) in current_blockers
            .iter()
            .copied()
            .enumerate()
            .take(current_blockers.len() - 1)
        {
            let later_has_damage = assigned_to_defender > 0
                || current_blockers[index + 1..]
                    .iter()
                    .any(|later| assigned_to(CombatDamageTarget::Object(*later)) > 0);
            if later_has_damage
                && assigned_to(CombatDamageTarget::Object(blocker))
                    < self.lethal_damage_required(blocker, source_keywords)?
            {
                return Err(StateError::IllegalCombatDamageAssignment(request.source()));
            }
        }
        Ok(())
    }

    fn combat_damage_profile(&self, source: ObjectId) -> Result<CombatDamageProfile, StateError> {
        let power = self.creature_power(source)?;
        let required_total = u32::try_from(power.max(0)).unwrap_or(0);
        if let Some(attacker) = self
            .combat
            .attackers
            .iter()
            .find(|attacker| attacker.object == source)
        {
            let keywords = self.creature_keywords(source)?;
            if !attacker.blocked {
                return Ok(CombatDamageProfile {
                    legal_targets: vec![CombatDamageTarget::Player(attacker.defending_player)],
                    required_total,
                    trample_blockers: Vec::new(),
                    trample_defender: CombatDamageTarget::Player(attacker.defending_player),
                });
            }
            let current_blockers: Vec<ObjectId> = attacker
                .blockers
                .iter()
                .copied()
                .filter(|blocker| self.is_active_blocking_creature(*blocker))
                .collect();
            if current_blockers.is_empty() {
                if keywords.trample() {
                    return Ok(CombatDamageProfile {
                        legal_targets: vec![CombatDamageTarget::Player(attacker.defending_player)],
                        required_total,
                        trample_blockers: Vec::new(),
                        trample_defender: CombatDamageTarget::Player(attacker.defending_player),
                    });
                }
                return Ok(CombatDamageProfile {
                    legal_targets: Vec::new(),
                    required_total: 0,
                    trample_blockers: Vec::new(),
                    trample_defender: CombatDamageTarget::Player(attacker.defending_player),
                });
            }
            let mut legal_targets: Vec<CombatDamageTarget> = current_blockers
                .iter()
                .map(|blocker| CombatDamageTarget::Object(*blocker))
                .collect();
            let trample_defender = CombatDamageTarget::Player(attacker.defending_player);
            let trample_blockers = if keywords.trample() {
                legal_targets.push(trample_defender);
                current_blockers
            } else {
                Vec::new()
            };
            return Ok(CombatDamageProfile {
                legal_targets,
                required_total,
                trample_blockers,
                trample_defender,
            });
        }

        if let Some(blocker) = self
            .combat
            .blockers
            .iter()
            .find(|blocker| blocker.object == source)
        {
            let legal_targets = if self.is_active_attacking_creature(blocker.attacker) {
                vec![CombatDamageTarget::Object(blocker.attacker)]
            } else {
                Vec::new()
            };
            let required_total = if legal_targets.is_empty() {
                0
            } else {
                required_total
            };
            return Ok(CombatDamageProfile {
                legal_targets,
                required_total,
                trample_blockers: Vec::new(),
                trample_defender: CombatDamageTarget::Object(blocker.attacker),
            });
        }

        Err(StateError::IllegalCombatDamageAssignment(source))
    }

    fn apply_combat_damage(
        &mut self,
        mut record: CombatDamageRecord,
    ) -> Result<Option<CombatDamageRecord>, StateError> {
        let replaced = self.apply_damage_replacement_effects(DamageReplacementEvent {
            source: Some(record.source),
            target: record.target,
            amount: record.amount,
            combat: true,
        })?;
        if replaced.amount == 0 {
            return Ok(None);
        }
        record.amount = replaced.amount;
        match record.target {
            CombatDamageTarget::Player(player) => {
                self.lose_life(player, record.amount)?;
            }
            CombatDamageTarget::Object(object) => {
                self.mark_damage_on_object_unreplaced(object, record.amount)?;
                if record.source_had_deathtouch && record.amount > 0 {
                    self.objects
                        .get_mut(object)
                        .ok_or(StateError::UnknownObject(object))?
                        .deathtouch_damage_marked = true;
                }
            }
        }
        if record.source_had_lifelink {
            let controller = self
                .objects
                .get(record.source)
                .ok_or(StateError::UnknownObject(record.source))?
                .controller();
            self.gain_life(controller, record.amount)?;
        }
        Ok(Some(record))
    }

    fn active_combat_creatures(&self) -> Vec<ObjectId> {
        let mut objects = Vec::new();
        for attacker in &self.combat.attackers {
            if self.is_active_attacking_creature(attacker.object) {
                objects.push(attacker.object);
            }
        }
        for blocker in &self.combat.blockers {
            if self.is_active_blocking_creature(blocker.object) {
                objects.push(blocker.object);
            }
        }
        objects
    }

    fn is_active_attacking_creature(&self, object: ObjectId) -> bool {
        self.combat
            .attackers
            .iter()
            .any(|attacker| attacker.object == object)
            && self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
            && self.creature_characteristics(object).is_ok()
    }

    fn is_active_blocking_creature(&self, object: ObjectId) -> bool {
        self.combat
            .blockers
            .iter()
            .any(|blocker| blocker.object == object)
            && self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
            && self.creature_characteristics(object).is_ok()
    }

    /// Computes current object characteristics through the CR 613 layer system.
    pub fn object_characteristics(
        &self,
        object: ObjectId,
    ) -> Result<ObjectCharacteristics, StateError> {
        let mut characteristics = self.base_object_characteristics(object)?;

        for layer in Self::continuous_effect_layers() {
            for effect in self.ordered_continuous_effects_for_layer(object, layer) {
                self.apply_continuous_effect(object, &mut characteristics, effect)?;
            }
            if layer == ContinuousEffectLayer::PowerToughnessModify {
                self.apply_power_toughness_counters(object, &mut characteristics);
            }
        }

        Ok(characteristics)
    }

    fn base_object_characteristics(
        &self,
        object: ObjectId,
    ) -> Result<ObjectCharacteristics, StateError> {
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        let base_creature = record
            .base_creature()
            .map(BaseCreatureCharacteristics::derived);
        let base_types = if base_creature.is_some() {
            record
                .base_object()
                .types()
                .union(ObjectTypes::none().with_creature())
        } else {
            record.base_object().types()
        };
        Ok(ObjectCharacteristics::new(
            record.controller(),
            record.base_object().colors(),
            base_types,
            base_creature,
        )
        .with_type_line(
            record.base_object().supertypes(),
            record.base_object().basic_land_types(),
            record.base_object().subtypes(),
        ))
    }

    const fn continuous_effect_layers() -> [ContinuousEffectLayer; 10] {
        [
            ContinuousEffectLayer::Copy,
            ContinuousEffectLayer::Control,
            ContinuousEffectLayer::Text,
            ContinuousEffectLayer::Type,
            ContinuousEffectLayer::Color,
            ContinuousEffectLayer::Ability,
            ContinuousEffectLayer::PowerToughnessCda,
            ContinuousEffectLayer::PowerToughnessSet,
            ContinuousEffectLayer::PowerToughnessModify,
            ContinuousEffectLayer::PowerToughnessSwitch,
        ]
    }

    fn object_characteristics_before_layer(
        &self,
        object: ObjectId,
        upper_bound: ContinuousEffectLayer,
    ) -> Result<ObjectCharacteristics, StateError> {
        let mut characteristics = self.base_object_characteristics(object)?;
        for layer in Self::continuous_effect_layers() {
            if layer >= upper_bound {
                break;
            }
            for effect in self.ordered_continuous_effects_for_layer(object, layer) {
                self.apply_continuous_effect(object, &mut characteristics, effect)?;
            }
            if layer == ContinuousEffectLayer::PowerToughnessModify {
                self.apply_power_toughness_counters(object, &mut characteristics);
            }
        }

        Ok(characteristics)
    }

    /// Computes the effective controller after CR 613 layer 2.
    pub fn object_controller(&self, object: ObjectId) -> Result<PlayerId, StateError> {
        Ok(self.object_characteristics(object)?.controller())
    }

    /// Returns devotion to one colored mana kind among permanents a player controls.
    pub fn controller_devotion(
        &self,
        controller: PlayerId,
        color: ManaKind,
    ) -> Result<u32, StateError> {
        self.require_player(controller)?;
        if color == ManaKind::Colorless {
            return Ok(0);
        }
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let mut devotion = 0_u32;
        for object in self.zone_objects(battlefield).unwrap_or_default() {
            let characteristics =
                self.object_characteristics_before_layer(*object, ContinuousEffectLayer::Type)?;
            if characteristics.controller() != controller {
                continue;
            }
            let symbols = self
                .objects
                .get(*object)
                .ok_or(StateError::UnknownObject(*object))?
                .base_object()
                .printed_mana_symbols();
            devotion = devotion.saturating_add(symbols.get(color));
        }
        Ok(devotion)
    }

    /// Computes current creature characteristics through the CR 613 layer system.
    pub fn creature_characteristics(
        &self,
        object: ObjectId,
    ) -> Result<CreatureCharacteristics, StateError> {
        self.object_characteristics(object)?
            .creature()
            .ok_or(StateError::NotACreature(object))
    }

    fn ordered_continuous_effects_for_layer(
        &self,
        object: ObjectId,
        layer: ContinuousEffectLayer,
    ) -> Vec<&ContinuousEffectSubscription> {
        let mut pending = self
            .continuous_effects
            .iter()
            .filter(|subscription| {
                subscription.definition.operation().layer() == layer
                    && self.continuous_effect_applies_to(subscription, object)
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|subscription| (subscription.definition.timestamp(), subscription.id));
        let mut ordered = Vec::with_capacity(pending.len());
        while !pending.is_empty() {
            if let Some(index) = pending.iter().position(|candidate| {
                candidate
                    .definition
                    .dependencies()
                    .iter()
                    .all(|dependency| !pending.iter().any(|other| other.id == *dependency))
            }) {
                ordered.push(pending.remove(index));
            } else {
                ordered.push(pending.remove(0));
            }
        }
        ordered
    }

    fn continuous_effect_applies_to(
        &self,
        subscription: &ContinuousEffectSubscription,
        object: ObjectId,
    ) -> bool {
        let definition = &subscription.definition;
        if !self.continuous_effect_condition_matches(definition) {
            return false;
        }
        if definition.duration() == ContinuousEffectDuration::WhileSourceOnBattlefield
            && definition.source().map_or(true, |source| {
                self.object_zone(source) != Some(ZoneId::new(None, ZoneKind::Battlefield))
            })
        {
            return false;
        }
        match definition.target() {
            ContinuousEffectTarget::Object(target) => target == object,
            ContinuousEffectTarget::AllObjects => true,
            ContinuousEffectTarget::Objects {
                predicate,
                excluded,
            } => {
                if excluded == Some(object) {
                    return false;
                }
                let Ok(characteristics) = self
                    .object_characteristics_before_layer(object, definition.operation().layer())
                else {
                    return false;
                };
                self.object_target_predicate_matches_characteristics(
                    definition.controller(),
                    predicate,
                    object,
                    characteristics,
                )
            }
            ContinuousEffectTarget::AttachedToSource => definition
                .source()
                .and_then(|source| self.objects.get(source))
                .and_then(ObjectRecord::attached_to)
                .is_some_and(|attached| attached == object),
        }
    }

    fn continuous_effect_condition_matches(&self, definition: &ContinuousEffectDefinition) -> bool {
        match definition.condition() {
            ContinuousEffectCondition::Always => true,
            ContinuousEffectCondition::ControllerDevotionLessThan { color, threshold } => self
                .controller_devotion(definition.controller(), color)
                .is_ok_and(|devotion| devotion < threshold),
        }
    }

    fn apply_continuous_effect(
        &self,
        object: ObjectId,
        characteristics: &mut ObjectCharacteristics,
        effect: &ContinuousEffectSubscription,
    ) -> Result<(), StateError> {
        match effect.definition.operation() {
            ContinuousEffectOperation::CopyBaseCreature { from } => {
                let source = self
                    .objects
                    .get(from)
                    .ok_or(StateError::UnknownObject(from))?;
                if let Some(base) = source.base_creature() {
                    characteristics.types = characteristics
                        .types
                        .union(ObjectTypes::none().with_creature());
                    characteristics.creature = Some(base.derived());
                } else if object != from {
                    characteristics.types = characteristics
                        .types
                        .without(ObjectTypes::none().with_creature());
                    characteristics.sync_creature_type();
                }
            }
            ContinuousEffectOperation::ChangeController { controller } => {
                characteristics.controller = controller;
            }
            ContinuousEffectOperation::SetTextMarker { marker } => {
                characteristics.text_marker = marker;
            }
            ContinuousEffectOperation::SetTypes { types } => {
                characteristics.types = types;
                characteristics.sync_creature_type();
            }
            ContinuousEffectOperation::AddTypes { types } => {
                characteristics.types = characteristics.types.union(types);
                characteristics.sync_creature_type();
            }
            ContinuousEffectOperation::RemoveTypes { types } => {
                characteristics.types = characteristics.types.without(types);
                characteristics.sync_creature_type();
            }
            ContinuousEffectOperation::SetColors { colors } => {
                characteristics.colors = colors;
            }
            ContinuousEffectOperation::AddKeywords { keywords } => {
                if let Some(creature) = characteristics.creature {
                    characteristics.creature =
                        Some(creature.with_keywords(creature.keywords().union(keywords)));
                }
            }
            ContinuousEffectOperation::RemoveKeywords { keywords } => {
                if let Some(creature) = characteristics.creature {
                    characteristics.creature =
                        Some(creature.with_keywords(creature.keywords().without(keywords)));
                }
            }
            ContinuousEffectOperation::SetBasePowerToughness { power, toughness }
            | ContinuousEffectOperation::SetPowerToughness { power, toughness } => {
                if let Some(creature) = characteristics.creature {
                    characteristics.creature = Some(
                        CreatureCharacteristics::new(power, toughness)
                            .with_keywords(creature.keywords()),
                    );
                }
            }
            ContinuousEffectOperation::ModifyPowerToughness { power, toughness } => {
                if let Some(creature) = characteristics.creature {
                    characteristics.creature = Some(
                        CreatureCharacteristics::new(
                            creature.power().saturating_add(power),
                            creature.toughness().saturating_add(toughness),
                        )
                        .with_keywords(creature.keywords()),
                    );
                }
            }
            ContinuousEffectOperation::SwitchPowerToughness => {
                if let Some(creature) = characteristics.creature {
                    characteristics.creature = Some(
                        CreatureCharacteristics::new(creature.toughness(), creature.power())
                            .with_keywords(creature.keywords()),
                    );
                }
            }
        }
        Ok(())
    }

    fn apply_power_toughness_counters(
        &self,
        object: ObjectId,
        characteristics: &mut ObjectCharacteristics,
    ) {
        let Some(creature) = characteristics.creature() else {
            return;
        };
        let plus = i32::try_from(self.object_counter_count(object, CounterKind::PlusOnePlusOne))
            .unwrap_or(i32::MAX);
        let minus = i32::try_from(self.object_counter_count(object, CounterKind::MinusOneMinusOne))
            .unwrap_or(i32::MAX);
        let delta = plus.saturating_sub(minus);
        characteristics.creature = Some(
            CreatureCharacteristics::new(
                creature.power().saturating_add(delta),
                creature.toughness().saturating_add(delta),
            )
            .with_keywords(creature.keywords()),
        );
    }

    fn creature_keywords(&self, object: ObjectId) -> Result<CreatureKeywords, StateError> {
        Ok(self.creature_characteristics(object)?.keywords())
    }

    fn creature_power(&self, object: ObjectId) -> Result<i32, StateError> {
        Ok(self.creature_characteristics(object)?.power())
    }

    fn lethal_damage_required(
        &self,
        object: ObjectId,
        source_keywords: CreatureKeywords,
    ) -> Result<u32, StateError> {
        let record = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?;
        let creature = self.creature_characteristics(object)?;
        if source_keywords.deathtouch() {
            return Ok(1);
        }
        let remaining = creature
            .toughness()
            .saturating_sub(i32::try_from(record.damage_marked()).unwrap_or(i32::MAX));
        Ok(u32::try_from(remaining.max(0)).unwrap_or(0))
    }

    fn remove_object_from_combat(&mut self, object: ObjectId) {
        self.combat
            .attackers
            .retain(|attacker| attacker.object != object);
        self.combat
            .blockers
            .retain(|blocker| blocker.object != object);
        for attacker in &mut self.combat.attackers {
            attacker.blockers.retain(|blocker| *blocker != object);
        }
        self.combat
            .first_strike_participants
            .retain(|participant| *participant != object);
    }

    fn clear_damage_marked(&mut self) {
        if !self
            .objects
            .iter()
            .any(|record| record.damage_marked() > 0 || record.deathtouch_damage_marked())
        {
            return;
        }
        for record in self.objects.records_mut() {
            record.damage_marked = 0;
            record.deathtouch_damage_marked = false;
        }
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

    fn after_priority_action(
        &mut self,
        player: PlayerId,
        _hold_priority: bool,
    ) -> Result<(), StateError> {
        self.grant_priority_to(Some(player))
    }

    fn grant_priority_after_resolution(&mut self) -> Result<(), StateError> {
        self.grant_priority_to(self.active_player)
    }

    fn grant_priority_to(&mut self, player: Option<PlayerId>) -> Result<(), StateError> {
        self.perform_state_based_actions()?;
        if self.outcome != GameOutcome::InProgress {
            self.deferred_priority_player = None;
            self.priority_player = None;
            self.priority_pass_count = 0;
            return Ok(());
        }
        if !self.pending_triggers.is_empty() {
            self.deferred_priority_player = player;
            self.priority_player = None;
            self.priority_pass_count = 0;
            return Ok(());
        }
        let new_priority = if self.outcome == GameOutcome::InProgress {
            player
        } else {
            None
        };
        self.deferred_priority_player = None;
        self.priority_player = new_priority;
        self.priority_pass_count = 0;
        Ok(())
    }

    fn perform_state_based_actions(&mut self) -> Result<StateBasedActionReport, StateError> {
        let mut report = StateBasedActionReport::default();
        for _ in 0..SBA_FIXPOINT_LIMIT {
            let actions = self.collect_state_based_actions();
            self.clear_since_last_sba_markers();
            if actions.is_empty() {
                return Ok(report);
            }
            report.record_iteration();
            for action in actions {
                self.apply_state_based_action(action, &mut report)?;
            }
            self.refresh_game_outcome();
        }
        Err(StateError::StateBasedActionLoop)
    }

    fn collect_state_based_actions(&self) -> Vec<PendingStateBasedAction> {
        let mut actions = Vec::new();
        for kind in state_based_action_table() {
            match *kind {
                StateBasedActionKind::PlayerZeroOrLessLife => {
                    self.collect_life_total_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::PlayerDrewFromEmptyLibrary => {
                    self.collect_empty_library_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::PlayerTenOrMorePoison => {
                    self.collect_poison_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::TokenOffBattlefield => {
                    self.collect_token_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::CopyOutOfAllowedZone => {
                    self.collect_copy_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::CreatureZeroOrLessToughness
                | StateBasedActionKind::CreatureLethalDamage
                | StateBasedActionKind::CreatureDeathtouchDamage => {
                    self.collect_creature_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::PlaneswalkerZeroLoyalty => {
                    self.collect_planeswalker_sbas(*kind, &mut actions);
                }
                StateBasedActionKind::CounterPairCancellation => {
                    self.collect_counter_pair_sbas(&mut actions);
                }
                StateBasedActionKind::LegendRule
                | StateBasedActionKind::WorldRule
                | StateBasedActionKind::AuraIllegalOrUnattached
                | StateBasedActionKind::EquipmentOrFortificationIllegalAttachment
                | StateBasedActionKind::BattleCreatureOrOtherIllegalAttachment
                | StateBasedActionKind::CounterMaximum
                | StateBasedActionKind::SagaFinalChapter
                | StateBasedActionKind::DungeonCompleted
                | StateBasedActionKind::SpaceSculptorDesignation
                | StateBasedActionKind::BattleZeroDefense
                | StateBasedActionKind::BattleMissingProtector
                | StateBasedActionKind::SiegeControllerProtector
                | StateBasedActionKind::DuplicateRole
                | StateBasedActionKind::StartYourEnginesNoSpeed => {}
            }
        }
        actions
    }

    fn collect_life_total_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        for player in &self.players {
            if !player.lost && player.life <= 0 {
                Self::push_player_loss_sba(actions, player.id, kind);
            }
        }
    }

    fn collect_empty_library_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        for player in &self.empty_library_draws_since_sba {
            if self
                .players
                .get(player.index())
                .is_some_and(|state| !state.lost)
            {
                Self::push_player_loss_sba(actions, *player, kind);
            }
        }
    }

    fn collect_poison_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        for player in &self.players {
            if !player.lost && player.poison >= 10 {
                Self::push_player_loss_sba(actions, player.id, kind);
            }
        }
    }

    fn collect_token_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let ceased = ZoneId::new(None, ZoneKind::Ceased);
        for object in self.objects.iter() {
            if object.is_token()
                && self
                    .object_zone(object.id())
                    .is_some_and(|zone| zone != battlefield && zone != ceased)
            {
                Self::push_cease_sba(actions, object.id(), kind);
            }
        }
    }

    fn collect_copy_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let stack = ZoneId::new(None, ZoneKind::Stack);
        let ceased = ZoneId::new(None, ZoneKind::Ceased);
        for object in self.objects.iter() {
            if object.is_copy()
                && self
                    .object_zone(object.id())
                    .is_some_and(|zone| zone != battlefield && zone != stack && zone != ceased)
            {
                Self::push_cease_sba(actions, object.id(), kind);
            }
        }
    }

    fn collect_creature_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        let Some(battlefield_index) = self.zone_index(ZoneId::new(None, ZoneKind::Battlefield))
        else {
            return;
        };
        for object_id in self.zones[battlefield_index].objects.iter().copied() {
            let Some(object) = self.objects.get(object_id) else {
                continue;
            };
            let Ok(creature) = self.creature_characteristics(object.id()) else {
                continue;
            };
            let applies = match kind {
                StateBasedActionKind::CreatureZeroOrLessToughness => creature.toughness() <= 0,
                StateBasedActionKind::CreatureLethalDamage => {
                    creature.toughness() > 0
                        && !self.object_is_indestructible(object.id())
                        && object.damage_marked() > 0
                        && object.damage_marked()
                            >= u32::try_from(creature.toughness()).unwrap_or(u32::MAX)
                }
                StateBasedActionKind::CreatureDeathtouchDamage => {
                    creature.toughness() > 0
                        && !self.object_is_indestructible(object.id())
                        && object.deathtouch_damage_marked()
                }
                _ => false,
            };
            if applies {
                Self::push_permanent_graveyard_sba(actions, object.id(), kind);
            }
        }
    }

    fn collect_planeswalker_sbas(
        &self,
        kind: StateBasedActionKind,
        actions: &mut Vec<PendingStateBasedAction>,
    ) {
        let Some(battlefield_index) = self.zone_index(ZoneId::new(None, ZoneKind::Battlefield))
        else {
            return;
        };
        for object_id in self.zones[battlefield_index].objects.iter().copied() {
            let Some(object) = self.objects.get(object_id) else {
                continue;
            };
            if object.loyalty().is_some_and(|loyalty| loyalty <= 0)
                && self
                    .object_characteristics(object.id())
                    .is_ok_and(|characteristics| characteristics.types().planeswalker())
            {
                Self::push_permanent_graveyard_sba(actions, object.id(), kind);
            }
        }
    }

    fn collect_counter_pair_sbas(&self, actions: &mut Vec<PendingStateBasedAction>) {
        for object in self.objects.iter() {
            let plus = self.object_counter_count(object.id(), CounterKind::PlusOnePlusOne);
            let minus = self.object_counter_count(object.id(), CounterKind::MinusOneMinusOne);
            let amount = plus.min(minus);
            if amount > 0 {
                actions.push(PendingStateBasedAction::CancelCounterPair {
                    object: object.id(),
                    amount,
                });
            }
        }
    }

    fn push_player_loss_sba(
        actions: &mut Vec<PendingStateBasedAction>,
        player: PlayerId,
        kind: StateBasedActionKind,
    ) {
        if !actions.iter().any(|action| {
            matches!(
                action,
                PendingStateBasedAction::PlayerLoses {
                    player: existing,
                    ..
                } if *existing == player
            )
        }) {
            actions.push(PendingStateBasedAction::PlayerLoses { player, kind });
        }
    }

    fn push_permanent_graveyard_sba(
        actions: &mut Vec<PendingStateBasedAction>,
        object: ObjectId,
        kind: StateBasedActionKind,
    ) {
        if !actions.iter().any(|action| {
            matches!(
                action,
                PendingStateBasedAction::MovePermanentToGraveyard {
                    object: existing,
                    ..
                } if *existing == object
            )
        }) {
            actions.push(PendingStateBasedAction::MovePermanentToGraveyard { object, kind });
        }
    }

    fn push_cease_sba(
        actions: &mut Vec<PendingStateBasedAction>,
        object: ObjectId,
        kind: StateBasedActionKind,
    ) {
        if !actions.iter().any(|action| {
            matches!(
                action,
                PendingStateBasedAction::CeaseObject {
                    object: existing,
                    ..
                } if *existing == object
            )
        }) {
            actions.push(PendingStateBasedAction::CeaseObject { object, kind });
        }
    }

    fn apply_state_based_action(
        &mut self,
        action: PendingStateBasedAction,
        report: &mut StateBasedActionReport,
    ) -> Result<(), StateError> {
        match action {
            PendingStateBasedAction::PlayerLoses { player, kind } => {
                let player_state = self
                    .players
                    .get_mut(player.index())
                    .ok_or(StateError::UnknownPlayer(player))?;
                if !player_state.lost {
                    player_state.lost = true;
                    report.record_player_loss(kind);
                    self.emit_event(GameEvent::PlayerLostByStateBasedAction { player, kind });
                }
            }
            PendingStateBasedAction::MovePermanentToGraveyard { object, kind } => {
                if self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield)) {
                    let owner = self
                        .objects
                        .get(object)
                        .ok_or(StateError::UnknownObject(object))?
                        .owner();
                    self.move_object(object, ZoneId::new(Some(owner), ZoneKind::Graveyard))?;
                    report.record_permanent_move(kind);
                    self.emit_event(GameEvent::PermanentMovedByStateBasedAction { object, kind });
                }
            }
            PendingStateBasedAction::CeaseObject { object, kind } => {
                if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Ceased)) {
                    self.move_object(object, ZoneId::new(None, ZoneKind::Ceased))?;
                    report.record_ceased_object(kind);
                    self.emit_event(GameEvent::ObjectCeasedToExist { object, kind });
                }
            }
            PendingStateBasedAction::CancelCounterPair { object, amount } => {
                self.remove_object_counters(object, CounterKind::PlusOnePlusOne, amount)?;
                self.remove_object_counters(object, CounterKind::MinusOneMinusOne, amount)?;
                report.record_counter_pair_cancelled(amount);
                self.emit_event(GameEvent::CounterPairCancelled { object, amount });
            }
        }
        Ok(())
    }

    fn clear_since_last_sba_markers(&mut self) {
        self.empty_library_draws_since_sba.clear();
        if !self
            .objects
            .iter()
            .any(ObjectRecord::deathtouch_damage_marked)
        {
            return;
        }
        for object in self.objects.records_mut() {
            object.deathtouch_damage_marked = false;
        }
    }

    fn refresh_game_outcome(&mut self) {
        let old_outcome = self.outcome;
        let remaining: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|player| !player.lost)
            .map(|player| player.id)
            .collect();
        self.outcome = if self.players.is_empty()
            || remaining.len() == self.players.len()
            || remaining.len() > 1
        {
            GameOutcome::InProgress
        } else if remaining.len() == 1 {
            GameOutcome::Won(remaining[0])
        } else {
            GameOutcome::Draw
        };
        if self.outcome != old_outcome {
            self.emit_event(GameEvent::GameOutcomeChanged {
                outcome: self.outcome,
            });
        }
        if self.outcome != GameOutcome::InProgress {
            self.priority_player = None;
            self.priority_pass_count = 0;
        }
    }

    fn can_cast_with_timing(&self, player: PlayerId, timing: SpellTiming) -> bool {
        match timing {
            SpellTiming::Instant => true,
            SpellTiming::Sorcery => {
                self.active_player == Some(player)
                    && matches!(
                        self.current_step,
                        Some(Step::PrecombatMain | Step::PostcombatMain)
                    )
                    && self.stack_entries.is_empty()
            }
        }
    }

    fn can_activate_with_timing(&self, player: PlayerId, timing: ActivationTiming) -> bool {
        match timing {
            ActivationTiming::Instant => true,
            ActivationTiming::Sorcery => {
                self.active_player == Some(player)
                    && matches!(
                        self.current_step,
                        Some(Step::PrecombatMain | Step::PostcombatMain)
                    )
                    && self.stack_entries.is_empty()
            }
        }
    }

    /// Returns whether a player can legally choose one target right now.
    #[must_use]
    pub fn can_target(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        requirement: TargetRequirement,
        choice: TargetChoice,
    ) -> bool {
        self.is_target_legal_at_cast(player, source, requirement, choice)
    }

    /// Returns the ward cost that would be observed when choosing one target.
    pub fn ward_cost_for_target(&self, choice: TargetChoice) -> Result<ManaCost, StateError> {
        self.target_ward_cost(choice)
    }

    /// Validates a complete ordered target binding without mutating state.
    pub fn validate_target_choices(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        requirements: &[TargetRequirement],
        choices: &[TargetChoice],
    ) -> Result<(), StateError> {
        self.capture_target_snapshots(player, source, requirements, choices)
            .map(|_| ())
    }

    fn capture_target_snapshots(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        requirements: &[TargetRequirement],
        choices: &[TargetChoice],
    ) -> Result<Vec<TargetSnapshot>, StateError> {
        if requirements.len() != choices.len() {
            return Err(StateError::TargetCountMismatch {
                required: requirements.len() as u32,
                selected: choices.len() as u32,
            });
        }
        let mut snapshots = Vec::with_capacity(requirements.len());
        for (index, (requirement, choice)) in requirements.iter().zip(choices.iter()).enumerate() {
            if !self.is_target_legal_at_cast(player, source, *requirement, *choice) {
                return Err(StateError::IllegalTarget {
                    index: index as u32,
                    target: *choice,
                });
            }
            snapshots.push(TargetSnapshot {
                requirement: *requirement,
                choice: *choice,
                original_zone: match choice {
                    TargetChoice::Object(object) => self.object_zone(*object),
                    TargetChoice::Player(_) | TargetChoice::StackEntry(_) => None,
                },
                ward_cost: self.target_ward_cost(*choice)?,
            });
        }
        Ok(snapshots)
    }

    fn is_target_legal_at_cast(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        requirement: TargetRequirement,
        choice: TargetChoice,
    ) -> bool {
        self.target_matches_kind(requirement.kind(), choice)
            && self.target_matches_predicate(player, requirement.predicate(), choice)
            && !self.targeting_restrictions_block(player, source, choice)
    }

    fn target_matches_kind(&self, kind: TargetKind, choice: TargetChoice) -> bool {
        match (kind, choice) {
            (TargetKind::Player, TargetChoice::Player(player)) => {
                self.require_player(player).is_ok()
            }
            (TargetKind::Permanent, TargetChoice::Object(object)) => {
                self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
            }
            (TargetKind::PlayerOrPermanent, TargetChoice::Player(player)) => {
                self.require_player(player).is_ok()
            }
            (TargetKind::PlayerOrPermanent, TargetChoice::Object(object)) => {
                self.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
            }
            (TargetKind::ObjectInZone(zone), TargetChoice::Object(object)) => {
                self.object_zone(object) == Some(zone)
            }
            (TargetKind::ObjectInZoneKind(kind), TargetChoice::Object(object)) => self
                .object_zone(object)
                .is_some_and(|zone| zone.kind() == kind),
            (TargetKind::StackEntry, TargetChoice::StackEntry(entry)) => self
                .stack_entries
                .iter()
                .any(|candidate| candidate.id() == entry),
            (TargetKind::Player, TargetChoice::Object(_) | TargetChoice::StackEntry(_))
            | (
                TargetKind::Permanent
                | TargetKind::ObjectInZone(_)
                | TargetKind::ObjectInZoneKind(_),
                TargetChoice::Player(_) | TargetChoice::StackEntry(_),
            )
            | (TargetKind::PlayerOrPermanent, TargetChoice::StackEntry(_))
            | (TargetKind::StackEntry, TargetChoice::Player(_) | TargetChoice::Object(_)) => false,
        }
    }

    fn target_matches_predicate(
        &self,
        player: PlayerId,
        predicate: TargetPredicate,
        choice: TargetChoice,
    ) -> bool {
        match (predicate, choice) {
            (TargetPredicate::Any, _) => true,
            (TargetPredicate::Player(predicate), TargetChoice::Player(target)) => {
                self.player_target_predicate_matches(player, predicate, target)
            }
            (TargetPredicate::Object(predicate), TargetChoice::Object(object)) => {
                self.object_target_predicate_matches(player, predicate, object)
            }
            (TargetPredicate::Object(predicate), TargetChoice::StackEntry(entry)) => self
                .stack_entries
                .iter()
                .find(|candidate| candidate.id() == entry)
                .and_then(StackEntry::object)
                .is_some_and(|object| {
                    self.object_target_predicate_matches(player, predicate, object)
                }),
            (
                TargetPredicate::PlayerOrObject {
                    player: predicate, ..
                },
                TargetChoice::Player(target),
            ) => self.player_target_predicate_matches(player, predicate, target),
            (
                TargetPredicate::PlayerOrObject {
                    object: predicate, ..
                },
                TargetChoice::Object(object),
            ) => self.object_target_predicate_matches(player, predicate, object),
            (TargetPredicate::Player(_), TargetChoice::Object(_) | TargetChoice::StackEntry(_))
            | (TargetPredicate::Object(_), TargetChoice::Player(_))
            | (TargetPredicate::PlayerOrObject { .. }, TargetChoice::StackEntry(_)) => false,
        }
    }

    fn player_target_predicate_matches(
        &self,
        player: PlayerId,
        predicate: PlayerTargetPredicate,
        target: PlayerId,
    ) -> bool {
        match predicate {
            PlayerTargetPredicate::Any => true,
            PlayerTargetPredicate::You => target == player,
            PlayerTargetPredicate::Opponent => {
                target != player && self.require_player(target).is_ok()
            }
            PlayerTargetPredicate::Player(expected) => target == expected,
        }
    }

    /// Returns whether an object satisfies one closed target predicate for a player.
    #[must_use]
    pub fn object_matches_target_predicate(
        &self,
        player: PlayerId,
        predicate: ObjectTargetPredicate,
        object: ObjectId,
    ) -> bool {
        self.object_target_predicate_matches(player, predicate, object)
    }

    fn object_target_predicate_matches(
        &self,
        player: PlayerId,
        predicate: ObjectTargetPredicate,
        object: ObjectId,
    ) -> bool {
        let Ok(characteristics) = self.object_characteristics(object) else {
            return false;
        };
        self.object_target_predicate_matches_characteristics(
            player,
            predicate,
            object,
            characteristics,
        )
    }

    fn object_target_predicate_matches_characteristics(
        &self,
        player: PlayerId,
        predicate: ObjectTargetPredicate,
        object: ObjectId,
        characteristics: ObjectCharacteristics,
    ) -> bool {
        if !self.controller_predicate_matches(
            player,
            predicate.controller,
            characteristics.controller(),
        ) {
            return false;
        }
        let Some(record) = self.objects.get(object) else {
            return false;
        };
        if !self.controller_predicate_matches(player, predicate.owner, record.owner()) {
            return false;
        }
        let mana_value = record.base_object().mana_value();
        if predicate
            .minimum_mana_value
            .is_some_and(|minimum| mana_value < minimum)
            || predicate
                .maximum_mana_value
                .is_some_and(|maximum| mana_value > maximum)
        {
            return false;
        }
        if !characteristics
            .subtypes()
            .contains_all(predicate.required_subtypes)
        {
            return false;
        }
        if !characteristics
            .types()
            .contains_all(predicate.required_types)
            || (predicate.required_any_types != ObjectTypes::none()
                && !characteristics
                    .types()
                    .intersects(predicate.required_any_types))
            || characteristics
                .types()
                .intersects(predicate.forbidden_types)
            || !characteristics
                .colors()
                .contains_all(predicate.required_colors)
            || characteristics
                .colors()
                .intersects(predicate.forbidden_colors)
        {
            return false;
        }
        let keywords = characteristics
            .creature()
            .map(CreatureCharacteristics::keywords)
            .unwrap_or_else(CreatureKeywords::none);
        keywords.contains_all(predicate.required_keywords)
            && !keywords.intersects(predicate.forbidden_keywords)
    }

    fn controller_predicate_matches(
        &self,
        player: PlayerId,
        predicate: TargetControllerPredicate,
        controller: PlayerId,
    ) -> bool {
        match predicate {
            TargetControllerPredicate::Any => true,
            TargetControllerPredicate::You => controller == player,
            TargetControllerPredicate::Opponent => controller != player,
            TargetControllerPredicate::Player(expected) => controller == expected,
        }
    }

    fn targeting_restrictions_block(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        choice: TargetChoice,
    ) -> bool {
        let TargetChoice::Object(object) = choice else {
            return false;
        };
        self.restrictions.iter().any(|subscription| {
            if !self.restriction_source_is_active(subscription.definition) {
                return false;
            }
            let RestrictionEffect::Targeting {
                subject,
                restriction,
            } = subscription.definition.effect()
            else {
                return false;
            };
            self.target_restriction_subject_matches(subscription.definition, subject, object)
                && self.target_restriction_blocks(player, source, object, restriction)
        })
    }

    fn target_restriction_subject_matches(
        &self,
        definition: RestrictionDefinition,
        subject: TargetRestrictionSubject,
        object: ObjectId,
    ) -> bool {
        match subject {
            TargetRestrictionSubject::Object(target) => target == object,
            TargetRestrictionSubject::AllObjects => true,
            TargetRestrictionSubject::AttachedToSource => definition
                .source()
                .and_then(|source| self.objects.get(source))
                .and_then(ObjectRecord::attached_to)
                .is_some_and(|attached| attached == object),
        }
    }

    fn target_restriction_blocks(
        &self,
        player: PlayerId,
        source: Option<ObjectId>,
        object: ObjectId,
        restriction: TargetRestriction,
    ) -> bool {
        match restriction {
            TargetRestriction::Shroud => true,
            TargetRestriction::Hexproof => self
                .object_controller(object)
                .is_ok_and(|controller| controller != player),
            TargetRestriction::ProtectionFromColors { colors } => source
                .and_then(|source| self.object_characteristics(source).ok())
                .is_some_and(|source| source.colors().intersects(colors)),
            TargetRestriction::Ward { .. } => false,
        }
    }

    fn target_ward_cost(&self, choice: TargetChoice) -> Result<ManaCost, StateError> {
        let TargetChoice::Object(object) = choice else {
            return Ok(ManaCost::new(0, 0, 0, 0, 0, 0));
        };
        let mut cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        for subscription in &self.restrictions {
            if !self.restriction_source_is_active(subscription.definition) {
                continue;
            }
            let RestrictionEffect::Targeting {
                subject,
                restriction: TargetRestriction::Ward { cost: ward },
            } = subscription.definition.effect()
            else {
                continue;
            };
            if self.target_restriction_subject_matches(subscription.definition, subject, object) {
                cost = Self::add_mana_costs(cost, ward)?;
            }
        }
        Ok(cost)
    }

    fn restriction_source_is_active(&self, definition: RestrictionDefinition) -> bool {
        definition.source().map_or(true, |source| {
            self.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Battlefield))
        })
    }

    fn object_is_indestructible(&self, object: ObjectId) -> bool {
        self.object_characteristics(object)
            .ok()
            .and_then(ObjectCharacteristics::creature)
            .is_some_and(|creature| creature.keywords().indestructible())
            || self.restrictions.iter().any(|subscription| {
                self.restriction_source_is_active(subscription.definition)
                    && matches!(
                        subscription.definition.effect(),
                        RestrictionEffect::Indestructible { object: protected }
                            if protected == object
                    )
            })
    }

    fn player_rule_subject_matches(
        &self,
        definition: RestrictionDefinition,
        subject: PlayerRuleSubject,
        player: PlayerId,
    ) -> bool {
        match subject {
            PlayerRuleSubject::Player(expected) => expected == player,
            PlayerRuleSubject::ControllerOfSource => definition
                .source()
                .and_then(|source| self.object_controller(source).ok())
                .is_some_and(|controller| controller == player),
        }
    }

    fn add_mana_costs(left: ManaCost, right: ManaCost) -> Result<ManaCost, StateError> {
        left.checked_add(right).ok_or(StateError::ManaValueOverflow)
    }

    fn is_target_still_legal(
        &self,
        controller: PlayerId,
        source: Option<ObjectId>,
        snapshot: TargetSnapshot,
    ) -> bool {
        match snapshot.choice {
            TargetChoice::Player(player) => {
                snapshot.requirement.kind() == TargetKind::Player
                    && self.require_player(player).is_ok()
                    && self.is_target_legal_at_cast(
                        controller,
                        source,
                        snapshot.requirement,
                        snapshot.choice,
                    )
            }
            TargetChoice::Object(object) => {
                self.object_zone(object) == snapshot.original_zone
                    && self.is_target_legal_at_cast(
                        controller,
                        source,
                        snapshot.requirement,
                        snapshot.choice,
                    )
            }
            TargetChoice::StackEntry(entry) => {
                snapshot.requirement.kind() == TargetKind::StackEntry
                    && self
                        .stack_entries
                        .iter()
                        .any(|candidate| candidate.id() == entry)
                    && self.is_target_legal_at_cast(
                        controller,
                        source,
                        snapshot.requirement,
                        snapshot.choice,
                    )
            }
        }
    }

    fn push_stack_entry(&mut self, request: StackEntryRequest) -> StackEntryId {
        let StackEntryRequest {
            controller,
            object,
            trigger,
            activated_ability,
            kind,
            targets,
            payment,
            copy_info,
            kicked,
            flashback,
            split_second,
        } = request;
        let id = StackEntryId(self.next_stack_entry);
        self.next_stack_entry = self.next_stack_entry.saturating_add(1);
        self.stack_entries.push(StackEntry {
            id,
            controller,
            object,
            trigger,
            activated_ability,
            kind,
            targets,
            payment,
            copy_info,
            kicked,
            flashback,
            split_second,
        });
        self.emit_event(GameEvent::StackEntryAdded {
            entry: id,
            controller,
            object,
            kind,
        });
        id
    }

    fn resolve_top_stack_entry(&mut self) -> Result<StackEntryId, StateError> {
        let entry = self
            .stack_entries
            .last()
            .cloned()
            .ok_or(StateError::EmptyStack)?;
        if let Some(object) = entry.object() {
            if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Stack)) {
                return Err(StateError::StackObjectNotOnStack(object));
            }
        }
        let target_source = match entry.activated_ability() {
            Some(ability) => self.activated_ability_definition(ability)?.source(),
            None => entry.object(),
        };
        let legal_targets: Vec<bool> = entry
            .targets()
            .iter()
            .map(|target| self.is_target_still_legal(entry.controller(), target_source, *target))
            .collect();
        let outcome = if !legal_targets.is_empty() && legal_targets.iter().all(|legal| !*legal) {
            ResolutionOutcome::CounteredOnResolution
        } else {
            ResolutionOutcome::Resolved
        };
        let entry = self.stack_entries.pop().ok_or(StateError::EmptyStack)?;
        if let Some(object) = entry.object() {
            let destination = match outcome {
                ResolutionOutcome::CounteredOnResolution | ResolutionOutcome::CounteredBySpell => {
                    let owner = self
                        .objects
                        .get(object)
                        .ok_or(StateError::UnknownObject(object))?
                        .owner();
                    if entry.flashback() {
                        ZoneId::new(None, ZoneKind::Exile)
                    } else {
                        ZoneId::new(Some(owner), ZoneKind::Graveyard)
                    }
                }
                ResolutionOutcome::Resolved => match entry.kind() {
                    StackObjectKind::InstantSpell | StackObjectKind::SorcerySpell => {
                        let owner = self
                            .objects
                            .get(object)
                            .ok_or(StateError::UnknownObject(object))?
                            .owner();
                        if entry.flashback() {
                            ZoneId::new(None, ZoneKind::Exile)
                        } else {
                            ZoneId::new(Some(owner), ZoneKind::Graveyard)
                        }
                    }
                    StackObjectKind::PermanentSpell => ZoneId::new(None, ZoneKind::Battlefield),
                    StackObjectKind::ActivatedAbility | StackObjectKind::TriggeredAbility => {
                        ZoneId::new(None, ZoneKind::Stack)
                    }
                },
            };
            if destination != ZoneId::new(None, ZoneKind::Stack) {
                self.move_object(object, destination)?;
            }
        }
        if outcome == ResolutionOutcome::Resolved {
            if let Some(ability) = entry.activated_ability() {
                let definition = self.activated_ability_definition(ability)?;
                self.resolve_activated_ability_effect(ability, entry.controller(), definition)?;
            }
        }
        self.resolution_log.push(ResolutionRecord {
            stack_entry: entry.id(),
            controller: entry.controller(),
            object: entry.object(),
            trigger: entry.trigger(),
            activated_ability: entry.activated_ability(),
            kind: entry.kind(),
            targets: entry.targets().to_vec(),
            legal_targets,
            outcome,
            copy_info: entry.copy_info(),
            kicked: entry.kicked(),
            flashback: entry.flashback(),
            split_second: entry.split_second(),
        });
        self.emit_event(GameEvent::StackEntryResolved {
            entry: entry.id(),
            outcome,
        });
        Ok(entry.id())
    }

    fn counter_stack_entry(&mut self, entry_id: StackEntryId) -> Result<(), StateError> {
        let index = self
            .stack_entries
            .iter()
            .position(|entry| entry.id() == entry_id)
            .ok_or(StateError::UnknownStackEntry(entry_id))?;
        let entry = self.stack_entries[index].clone();
        if let Some(object) = entry.object() {
            if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Stack)) {
                return Err(StateError::StackObjectNotOnStack(object));
            }
        }

        let entry = self.stack_entries.remove(index);
        if let Some(object) = entry.object() {
            let owner = self
                .objects
                .get(object)
                .ok_or(StateError::UnknownObject(object))?
                .owner();
            let destination = if entry.flashback() {
                ZoneId::new(None, ZoneKind::Exile)
            } else {
                ZoneId::new(Some(owner), ZoneKind::Graveyard)
            };
            self.move_object(object, destination)?;
        }
        self.resolution_log.push(ResolutionRecord {
            stack_entry: entry.id(),
            controller: entry.controller(),
            object: entry.object(),
            trigger: entry.trigger(),
            activated_ability: entry.activated_ability(),
            kind: entry.kind(),
            targets: entry.targets().to_vec(),
            legal_targets: vec![true; entry.targets().len()],
            outcome: ResolutionOutcome::CounteredBySpell,
            copy_info: entry.copy_info(),
            kicked: entry.kicked(),
            flashback: entry.flashback(),
            split_second: entry.split_second(),
        });
        self.emit_event(GameEvent::StackEntryResolved {
            entry: entry.id(),
            outcome: ResolutionOutcome::CounteredBySpell,
        });
        Ok(())
    }

    fn destroy_permanent(
        &mut self,
        object: ObjectId,
        allow_regeneration: bool,
    ) -> Result<(), StateError> {
        if self.object_zone(object) != Some(ZoneId::new(None, ZoneKind::Battlefield)) {
            return Err(StateError::ObjectNotOnBattlefield(object));
        }
        if self.object_is_indestructible(object) {
            return Ok(());
        }
        if allow_regeneration && self.consume_regeneration_shield(object) {
            let record = self
                .objects
                .get_mut(object)
                .ok_or(StateError::UnknownObject(object))?;
            record.tapped = true;
            record.damage_marked = 0;
            record.deathtouch_damage_marked = false;
            self.remove_object_from_combat(object);
            self.emit_event(GameEvent::ObjectTapped {
                object,
                tapped: true,
            });
            return Ok(());
        }
        let owner = self
            .objects
            .get(object)
            .ok_or(StateError::UnknownObject(object))?
            .owner();
        self.move_object(object, ZoneId::new(Some(owner), ZoneKind::Graveyard))
    }

    fn consume_regeneration_shield(&mut self, object: ObjectId) -> bool {
        let Some(index) = self.restrictions.iter().position(|subscription| {
            self.restriction_source_is_active(subscription.definition)
                && matches!(
                    subscription.definition.effect(),
                    RestrictionEffect::RegenerationShield { object: protected }
                        if protected == object
                )
        }) else {
            return false;
        };
        self.restrictions.remove(index);
        true
    }

    fn map_payment_error(error: PaymentError) -> StateError {
        match error {
            PaymentError::ManaValueOverflow => StateError::ManaValueOverflow,
            PaymentError::InsufficientMana => StateError::InsufficientMana,
            PaymentError::InvalidPaymentPlan => StateError::InvalidPaymentPlan,
        }
    }

    fn clear_all_mana_pools(&mut self) {
        let mut changed = false;
        for player in &mut self.players {
            changed |= player.mana_pool != ManaPool::empty();
            player.mana_pool = ManaPool::empty();
        }
        if changed {
            self.emit_event(GameEvent::ManaPoolsCleared);
        }
    }

    fn perform_cleanup_actions(&mut self) -> Result<CleanupReport, StateError> {
        let active = self.active_player.ok_or(StateError::TurnNotStarted)?;
        let discarded = self.discard_to_max_hand_size(active)?;
        let expired_until_end_of_turn = (self
            .expire_duration_markers(EffectDuration::UntilEndOfTurn)
            .saturating_add(
                self.expire_continuous_effects(ContinuousEffectDuration::UntilEndOfTurn),
            )
            .saturating_add(self.expire_restrictions(ContinuousEffectDuration::UntilEndOfTurn)))
            as u32;
        let expired_this_turn = self.expire_duration_markers(EffectDuration::ThisTurn) as u32;
        Ok(CleanupReport {
            discarded,
            expired_until_end_of_turn,
            expired_this_turn,
        })
    }

    fn discard_to_max_hand_size(&mut self, player: PlayerId) -> Result<u32, StateError> {
        let Some(max_hand_size) = self.effective_max_hand_size(player)? else {
            return Ok(0);
        };
        let max_hand_size = max_hand_size as usize;
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
        self.draw_cards(active, 1)
    }

    fn draw_cards(&mut self, player: PlayerId, count: u32) -> Result<(), StateError> {
        self.require_player(player)?;
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        for _ in 0..count {
            if let Some(object) = self.move_last_between_zones(library, hand)? {
                self.emit_event(GameEvent::CardDrawn { player, object });
            } else {
                self.emit_event(GameEvent::EmptyLibraryDraw { player });
                if !self.empty_library_draws_since_sba.contains(&player) {
                    self.empty_library_draws_since_sba.push(player);
                }
            }
        }
        Ok(())
    }

    fn reveal_objects(&mut self, objects: &[ObjectId]) -> Result<(), StateError> {
        for object in objects {
            if self.objects.get(*object).is_none() {
                return Err(StateError::UnknownObject(*object));
            }
        }
        for object in objects {
            let observers = self
                .players
                .iter()
                .map(|player| player.id())
                .collect::<Vec<_>>();
            for observer in observers {
                self.remember_object(observer, *object);
            }
            self.emit_event(GameEvent::ObjectRevealed { object: *object });
        }
        Ok(())
    }

    fn put_hand_cards_on_library_bottom(
        &mut self,
        player: PlayerId,
        bottom: &[ObjectId],
    ) -> Result<(), StateError> {
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        self.require_zone(hand)?;
        self.require_zone(library)?;
        let hand_index = self.zone_index(hand).ok_or(StateError::UnknownZone(hand))?;
        let library_index = self
            .zone_index(library)
            .ok_or(StateError::UnknownZone(library))?;
        let mut moved = Vec::with_capacity(bottom.len());
        for object in bottom {
            let position = self.zones[hand_index]
                .objects
                .iter()
                .position(|candidate| candidate == object)
                .ok_or(StateError::OpeningHandBottomCardNotInHand {
                    player,
                    object: *object,
                })?;
            self.remember_hidden_transition(*object, hand, library);
            moved.push(
                Arc::make_mut(&mut self.zones)[hand_index]
                    .objects_mut()
                    .remove(position),
            );
        }
        for (offset, object) in moved.into_iter().enumerate() {
            Arc::make_mut(&mut self.zones)[library_index]
                .objects_mut()
                .insert(offset, object);
            self.emit_event(GameEvent::ObjectMoved {
                object,
                from: hand,
                to: library,
            });
            self.emit_event(GameEvent::OpeningHandCardBottomed { player, object });
        }
        Ok(())
    }

    fn move_all_between_zones(&mut self, from: ZoneId, to: ZoneId) -> Result<(), StateError> {
        while self.move_last_between_zones(from, to)?.is_some() {}
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
        let Some(object) = Arc::make_mut(&mut self.zones)[from_index]
            .objects_mut()
            .pop()
        else {
            return Ok(None);
        };
        self.remember_hidden_transition(object, from, to);
        let to_index = self.zone_index(to).ok_or(StateError::UnknownZone(to))?;
        Arc::make_mut(&mut self.zones)[to_index]
            .objects_mut()
            .push(object);
        self.emit_event(GameEvent::ObjectMoved { object, from, to });
        Ok(Some(object))
    }

    fn shuffle_zone(&mut self, zone: ZoneId) -> Result<(), StateError> {
        self.require_zone(zone)?;
        let zone_index = self.zone_index(zone).ok_or(StateError::UnknownZone(zone))?;
        let shuffled_objects = self.zones[zone_index].objects.as_ref().clone();
        let len = self.zones[zone_index].objects.len();
        for index in (1..len).rev() {
            let swap_with = self.random_below(index + 1);
            Arc::make_mut(&mut self.zones)[zone_index]
                .objects_mut()
                .swap(index, swap_with);
        }
        if Self::zone_hides_identity(zone) {
            Arc::make_mut(&mut self.known_objects)
                .retain(|known| !shuffled_objects.contains(&known.object));
        }
        self.emit_event(GameEvent::ZoneShuffled { zone });
        Ok(())
    }

    fn put_object_on_top_of_library(
        &mut self,
        player: PlayerId,
        object: ObjectId,
    ) -> Result<(), StateError> {
        self.require_player(player)?;
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        self.require_zone(library)?;
        if self.object_zone(object) != Some(library) {
            return Err(StateError::MissingZoneMembership(object));
        }
        let library_index = self
            .zone_index(library)
            .ok_or(StateError::UnknownZone(library))?;
        let zone = &mut Arc::make_mut(&mut self.zones)[library_index];
        let position = zone
            .objects
            .iter()
            .position(|candidate| *candidate == object)
            .ok_or(StateError::MissingZoneMembership(object))?;
        zone.objects_mut().remove(position);
        zone.objects_mut().push(object);
        self.emit_event(GameEvent::LibraryManipulated {
            player,
            operation: LibraryManipulation::TutorToTop,
            count: 1,
            moved: 1,
        });
        Ok(())
    }

    fn random_below(&mut self, upper: usize) -> usize {
        debug_assert!(upper > 0);
        let random = self.next_random_u64();
        ((u128::from(random) * (upper as u128)) >> 64) as usize
    }

    fn next_random_u64(&mut self) -> u64 {
        self.rng_state = self.rng_state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.rng_state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn zone_len(&self, id: ZoneId) -> Result<usize, StateError> {
        self.require_zone(id)?;
        let index = self.zone_index(id).ok_or(StateError::UnknownZone(id))?;
        Ok(self.zones[index].objects.len())
    }

    fn validate_turn_order(&self, order: &[PlayerId]) -> Result<(), StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        if order.len() != self.players.len() {
            return Err(StateError::TurnOrderPlayerCountMismatch {
                expected: self.players.len() as u32,
                actual: u32::try_from(order.len()).unwrap_or(u32::MAX),
            });
        }
        let mut seen = vec![false; self.players.len()];
        for player in order {
            self.require_player(*player)?;
            if seen[player.index()] {
                return Err(StateError::DuplicateTurnOrderPlayer(*player));
            }
            seen[player.index()] = true;
        }
        for player in &self.players {
            if !seen[player.id().index()] {
                return Err(StateError::TurnOrderMissingPlayer(player.id()));
            }
        }
        Ok(())
    }

    fn turn_order_from(&self, starting_player: PlayerId) -> Result<Vec<PlayerId>, StateError> {
        self.require_player(starting_player)?;
        let mut order = Vec::with_capacity(self.players.len());
        for offset in 0..self.players.len() {
            let index = (starting_player.index() + offset) % self.players.len();
            order.push(PlayerId(index as u32));
        }
        Ok(order)
    }

    fn next_player_after(&self, player: PlayerId) -> Result<PlayerId, StateError> {
        if self.players.is_empty() {
            return Err(StateError::NoPlayers);
        }
        self.require_player(player)?;
        if !self.turn_order.is_empty() {
            let position = self
                .turn_order
                .iter()
                .position(|candidate| *candidate == player)
                .ok_or(StateError::TurnOrderMissingPlayer(player))?;
            let next = (position + 1) % self.turn_order.len();
            return Ok(self.turn_order[next]);
        }
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
        let before = self.duration_markers.len();
        self.duration_markers.retain(|marker| {
            !matches!(
                marker.duration,
                EffectDuration::UntilStepBegins(marker_step) if marker_step == step
            )
        });
        let count = before - self.duration_markers.len();
        if count > 0 {
            self.emit_event(GameEvent::DurationMarkersExpired {
                duration: EffectDuration::UntilStepBegins(step),
                count: count as u32,
            });
        }
    }

    fn expire_phase_end_markers(&mut self, phase: Phase) {
        let before = self.duration_markers.len();
        self.duration_markers.retain(|marker| {
            !matches!(
                marker.duration,
                EffectDuration::UntilPhaseEnds(marker_phase) if marker_phase == phase
            )
        });
        let count = before - self.duration_markers.len();
        if count > 0 {
            self.emit_event(GameEvent::DurationMarkersExpired {
                duration: EffectDuration::UntilPhaseEnds(phase),
                count: count as u32,
            });
        }
    }

    fn expire_end_of_combat_markers(&mut self) {
        let before = self.duration_markers.len();
        self.duration_markers
            .retain(|marker| marker.duration != EffectDuration::UntilEndOfCombat);
        let count = before - self.duration_markers.len();
        if count > 0 {
            self.emit_event(GameEvent::DurationMarkersExpired {
                duration: EffectDuration::UntilEndOfCombat,
                count: count as u32,
            });
        }
    }

    fn expire_duration_markers(&mut self, duration: EffectDuration) -> usize {
        let before = self.duration_markers.len();
        self.duration_markers
            .retain(|marker| marker.duration != duration);
        let count = before - self.duration_markers.len();
        if count > 0 {
            self.emit_event(GameEvent::DurationMarkersExpired {
                duration,
                count: count as u32,
            });
        }
        count
    }

    fn expire_continuous_effects(&mut self, duration: ContinuousEffectDuration) -> usize {
        let before = self.continuous_effects.len();
        self.continuous_effects
            .retain(|effect| effect.definition.duration() != duration);
        before - self.continuous_effects.len()
    }

    fn expire_restrictions(&mut self, duration: ContinuousEffectDuration) -> usize {
        let before = self.restrictions.len();
        self.restrictions
            .retain(|restriction| restriction.definition.duration() != duration);
        before - self.restrictions.len()
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
        Ok(&mut Arc::make_mut(&mut self.zones)[index])
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

    fn write_optional_i32(&mut self, value: Option<i32>) {
        match value {
            Some(value) => {
                self.write_u8(1);
                self.write_i32(value);
            }
            None => self.write_u8(0),
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

    fn write_game_outcome(&mut self, outcome: GameOutcome) {
        self.write_u8(outcome.canonical_code());
        if let GameOutcome::Won(player) = outcome {
            self.write_u32(player.0);
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

    fn write_mana_pool(&mut self, pool: ManaPool) {
        for amount in pool.amounts {
            self.write_u32(amount);
        }
    }

    fn write_player_state(&mut self, player: PlayerState) {
        self.write_u32(player.id.0);
        self.write_i32(player.life);
        self.write_u32(player.poison);
        self.write_bool(player.lost);
        self.write_u32(player.max_hand_size);
        self.write_u32(player.mulligans_taken);
        self.write_bool(player.opening_hand_kept);
        self.write_mana_pool(player.mana_pool);
        self.write_u32(player.lands_played_this_turn);
    }

    fn write_object_record(&mut self, object: ObjectRecord) {
        self.write_u32(object.id.0);
        self.write_u32(object.card.0);
        self.write_u32(object.owner.0);
        self.write_u32(object.controller.0);
        self.write_bool(object.tapped);
        self.write_base_object_characteristics(object.base_object);
        self.write_optional_base_creature_characteristics(object.base_creature);
        self.write_u32(object.damage_marked);
        self.write_bool(object.deathtouch_damage_marked);
        self.write_optional_i32(object.loyalty);
        self.write_bool(object.token);
        self.write_optional_object(object.copy_source);
        self.write_optional_object(object.attached_to);
        self.write_u32(object.controlled_since_turn);
        self.write_object_colors(object.color_identity);
        self.write_bool(object.commander);
        self.write_u32(object.commander_cast_count);
    }

    fn write_object_characteristics(&mut self, characteristics: ObjectCharacteristics) {
        self.write_u32(characteristics.controller.0);
        self.write_object_colors(characteristics.colors);
        self.write_object_types(characteristics.types);
        self.write_object_supertypes(characteristics.supertypes);
        self.write_basic_land_types(characteristics.basic_land_types);
        self.write_object_subtypes(characteristics.subtypes);
        match characteristics.creature {
            Some(creature) => {
                self.write_u8(1);
                self.write_i32(creature.power);
                self.write_i32(creature.toughness);
                self.write_creature_keywords(creature.keywords);
            }
            None => self.write_u8(0),
        }
        self.write_u32(characteristics.text_marker);
    }

    fn write_counter_kind(&mut self, kind: CounterKind) {
        self.write_u8(kind.canonical_code());
        if let CounterKind::Named(id) = kind {
            self.write_u32(id);
        }
    }

    fn write_object_counter_entry(&mut self, entry: ObjectCounterEntry) {
        self.write_u32(entry.object.0);
        self.write_counter_kind(entry.kind);
        self.write_u32(entry.count);
    }

    fn write_creature_keywords(&mut self, keywords: CreatureKeywords) {
        self.write_u32(u32::from(keywords.canonical_bits()));
    }

    fn write_object_colors(&mut self, colors: ObjectColors) {
        self.write_u8(colors.canonical_bits());
    }

    fn write_object_types(&mut self, types: ObjectTypes) {
        self.write_u8(types.canonical_bits());
    }

    fn write_object_supertypes(&mut self, supertypes: ObjectSupertypes) {
        self.write_u8(supertypes.canonical_bits());
    }

    fn write_basic_land_types(&mut self, land_types: BasicLandTypes) {
        self.write_u8(land_types.canonical_bits());
    }

    fn write_object_subtypes(&mut self, subtypes: ObjectSubtypes) {
        self.write_u8(subtypes.len);
        for subtype in subtypes.as_slice() {
            self.write_u8(subtype.len);
            for byte in subtype.as_bytes() {
                self.write_u8(*byte);
            }
        }
    }

    fn write_base_object_characteristics(&mut self, base: BaseObjectCharacteristics) {
        self.write_object_types(base.types());
        self.write_object_colors(base.colors());
        self.write_object_supertypes(base.supertypes());
        self.write_basic_land_types(base.basic_land_types());
        self.write_object_subtypes(base.subtypes());
        self.write_u32(base.mana_value());
        self.write_mana_pool(base.printed_mana_symbols());
        self.write_bool(base.enters_tapped());
    }

    fn write_base_creature_characteristics(&mut self, base: BaseCreatureCharacteristics) {
        self.write_i32(base.power);
        self.write_i32(base.toughness);
        self.write_creature_keywords(base.keywords);
    }

    fn write_optional_base_creature_characteristics(
        &mut self,
        base: Option<BaseCreatureCharacteristics>,
    ) {
        match base {
            Some(base) => {
                self.write_u8(1);
                self.write_base_creature_characteristics(base);
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

    fn write_optional_u32(&mut self, value: Option<u32>) {
        match value {
            Some(value) => {
                self.write_u8(1);
                self.write_u32(value);
            }
            None => self.write_u8(0),
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

    fn write_optional_trigger(&mut self, trigger: Option<TriggerId>) {
        match trigger {
            Some(trigger) => {
                self.write_u8(1);
                self.write_u32(trigger.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_optional_activated_ability(&mut self, ability: Option<ActivatedAbilityId>) {
        match ability {
            Some(ability) => {
                self.write_u8(1);
                self.write_u32(ability.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_optional_stack_copy_info(&mut self, copy_info: Option<StackCopyInfo>) {
        match copy_info {
            Some(copy_info) => {
                self.write_u8(1);
                self.write_u32(copy_info.source_entry.0);
                self.write_optional_object(copy_info.source_object);
            }
            None => self.write_u8(0),
        }
    }

    fn write_target_kind(&mut self, kind: TargetKind) {
        self.write_u8(kind.canonical_code());
        match kind {
            TargetKind::ObjectInZone(zone) => self.write_zone_id(zone),
            TargetKind::ObjectInZoneKind(kind) => self.write_u8(kind.canonical_code()),
            TargetKind::Player
            | TargetKind::Permanent
            | TargetKind::StackEntry
            | TargetKind::PlayerOrPermanent => {}
        }
    }

    fn write_target_controller_predicate(&mut self, predicate: TargetControllerPredicate) {
        self.write_u8(predicate.canonical_code());
        if let TargetControllerPredicate::Player(player) = predicate {
            self.write_u32(player.0);
        }
    }

    fn write_object_target_predicate(&mut self, predicate: ObjectTargetPredicate) {
        self.write_target_controller_predicate(predicate.controller);
        self.write_target_controller_predicate(predicate.owner);
        self.write_object_types(predicate.required_types);
        self.write_object_types(predicate.required_any_types);
        self.write_object_types(predicate.forbidden_types);
        self.write_object_colors(predicate.required_colors);
        self.write_object_colors(predicate.forbidden_colors);
        self.write_creature_keywords(predicate.required_keywords);
        self.write_creature_keywords(predicate.forbidden_keywords);
        self.write_object_subtypes(predicate.required_subtypes);
        self.write_optional_u32(predicate.minimum_mana_value);
        self.write_optional_u32(predicate.maximum_mana_value);
    }

    fn write_player_target_predicate(&mut self, predicate: PlayerTargetPredicate) {
        self.write_u8(predicate.canonical_code());
        if let PlayerTargetPredicate::Player(player) = predicate {
            self.write_u32(player.0);
        }
    }

    fn write_target_predicate(&mut self, predicate: TargetPredicate) {
        self.write_u8(predicate.canonical_code());
        match predicate {
            TargetPredicate::Any => {}
            TargetPredicate::Object(predicate) => self.write_object_target_predicate(predicate),
            TargetPredicate::Player(predicate) => self.write_player_target_predicate(predicate),
            TargetPredicate::PlayerOrObject { player, object } => {
                self.write_player_target_predicate(player);
                self.write_object_target_predicate(object);
            }
        }
    }

    fn write_target_requirement(&mut self, requirement: TargetRequirement) {
        self.write_target_kind(requirement.kind);
        self.write_target_predicate(requirement.predicate);
    }

    fn write_target_choice(&mut self, choice: TargetChoice) {
        self.write_u8(choice.canonical_code());
        match choice {
            TargetChoice::Player(player) => self.write_u32(player.0),
            TargetChoice::Object(object) => self.write_u32(object.0),
            TargetChoice::StackEntry(entry) => self.write_u32(entry.0),
        }
    }

    fn write_optional_zone(&mut self, zone: Option<ZoneId>) {
        match zone {
            Some(zone) => {
                self.write_u8(1);
                self.write_zone_id(zone);
            }
            None => self.write_u8(0),
        }
    }

    fn write_target_snapshot(&mut self, target: TargetSnapshot) {
        self.write_target_requirement(target.requirement);
        self.write_target_choice(target.choice);
        self.write_optional_zone(target.original_zone);
        self.write_mana_cost(target.ward_cost);
    }

    fn write_payment_plan(&mut self, payment: PaymentPlan) {
        self.write_mana_pool(payment.paid);
        self.write_mana_pool(payment.generic_paid);
        self.write_u32(payment.generic_required);
        self.write_u32(payment.x_value);
        self.write_u32(payment.waste_score);
    }

    fn write_optional_payment_plan(&mut self, payment: Option<PaymentPlan>) {
        match payment {
            Some(payment) => {
                self.write_u8(1);
                self.write_payment_plan(payment);
            }
            None => self.write_u8(0),
        }
    }

    fn write_game_event_kind(&mut self, kind: GameEventKind) {
        self.write_u8(kind.canonical_code());
    }

    fn write_trigger_object_filter(&mut self, filter: TriggerObjectFilter) {
        self.write_u8(filter.canonical_code());
        if let TriggerObjectFilter::Object(object) = filter {
            self.write_u32(object.0);
        }
    }

    fn write_trigger_player_filter(&mut self, filter: TriggerPlayerFilter) {
        self.write_u8(filter.canonical_code());
        if let TriggerPlayerFilter::Player(player) = filter {
            self.write_u32(player.0);
        }
    }

    fn write_trigger_zone_filter(&mut self, filter: TriggerZoneFilter) {
        self.write_u8(filter.canonical_code());
        match filter {
            TriggerZoneFilter::Any => {}
            TriggerZoneFilter::Exact(zone) => self.write_zone_id(zone),
            TriggerZoneFilter::Kind(kind) => self.write_u8(kind.canonical_code()),
            TriggerZoneFilter::Owned { owner, kind } => {
                self.write_trigger_player_filter(owner);
                self.write_u8(kind.canonical_code());
            }
        }
    }

    fn write_trigger_condition(&mut self, condition: TriggerCondition) {
        self.write_u8(condition.canonical_code());
        match condition {
            TriggerCondition::EventKind(kind) => self.write_game_event_kind(kind),
            TriggerCondition::ObjectMoved { object, from, to } => {
                self.write_trigger_object_filter(object);
                self.write_trigger_zone_filter(from);
                self.write_trigger_zone_filter(to);
            }
            TriggerCondition::StepBegan { step } => self.write_u8(step.canonical_code()),
            TriggerCondition::LifeLost { player } | TriggerCondition::LifeGained { player } => {
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::DamageMarked { object } => {
                self.write_trigger_object_filter(object);
            }
            TriggerCondition::StackEntryResolved { kind, outcome } => {
                match kind {
                    Some(kind) => {
                        self.write_u8(1);
                        self.write_u8(kind.canonical_code());
                    }
                    None => self.write_u8(0),
                }
                match outcome {
                    Some(outcome) => {
                        self.write_u8(1);
                        self.write_u8(outcome.canonical_code());
                    }
                    None => self.write_u8(0),
                }
            }
            TriggerCondition::AttackDeclared { attacker } => {
                self.write_trigger_object_filter(attacker);
            }
            TriggerCondition::StepBeganFor { step, player } => {
                self.write_u8(step.canonical_code());
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::StackEntryAdded {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            }
            | TriggerCondition::StackEntryAddedOrCopied {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            } => {
                self.write_trigger_player_filter(controller);
                self.write_object_types(required_types);
                self.write_object_types(required_any_types);
                self.write_object_types(forbidden_types);
            }
            TriggerCondition::CombatDamageToPlayer { source } => {
                self.write_object_target_predicate(source);
            }
            TriggerCondition::PlayerDrewCard { player } => {
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::PermanentEnteredBattlefield {
                predicate,
                exclude_source,
            } => {
                self.write_object_target_predicate(predicate);
                self.write_bool(exclude_source);
            }
        }
    }

    fn write_trigger_intervening_if(&mut self, intervening_if: TriggerInterveningIf) {
        self.write_u8(intervening_if.canonical_code());
        match intervening_if {
            TriggerInterveningIf::Always | TriggerInterveningIf::ControllerControlsSource => {}
            TriggerInterveningIf::SourceInZone(zone) => self.write_zone_id(zone),
            TriggerInterveningIf::ControllerLifeAtMost(life) => self.write_i32(life),
        }
    }

    fn write_trigger_duration(&mut self, duration: TriggerDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_trigger_definition(&mut self, definition: TriggerDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_trigger_condition(definition.condition);
        self.write_trigger_intervening_if(definition.intervening_if);
        self.write_trigger_duration(definition.duration);
    }

    fn write_trigger_subscription(&mut self, subscription: TriggerSubscription) {
        self.write_u32(subscription.id.0);
        self.write_trigger_definition(subscription.definition);
        self.write_game_event_kind(subscription.event_kind);
    }

    fn write_pending_trigger(&mut self, trigger: PendingTriggeredAbility) {
        self.write_u32(trigger.trigger.0);
        self.write_u32(trigger.controller.0);
        self.write_optional_object(trigger.source);
        self.write_u64(trigger.event_sequence);
        self.write_u32(trigger.event_turn);
    }

    fn write_activation_timing(&mut self, timing: ActivationTiming) {
        self.write_u8(timing.canonical_code());
    }

    fn write_ability_player(&mut self, player: AbilityPlayer) {
        self.write_u8(player.canonical_code());
        if let AbilityPlayer::Player(player) = player {
            self.write_u32(player.0);
        }
    }

    fn write_activated_ability_effect(&mut self, effect: ActivatedAbilityEffect) {
        self.write_u8(effect.canonical_code());
        match effect {
            ActivatedAbilityEffect::AddMana { player, mana } => {
                self.write_ability_player(player);
                self.write_mana_pool(mana);
            }
            ActivatedAbilityEffect::GainLife { player, amount }
            | ActivatedAbilityEffect::LoseLife { player, amount } => {
                self.write_ability_player(player);
                self.write_u32(amount);
            }
            ActivatedAbilityEffect::AddManaAndDealDamage {
                player,
                mana,
                amount,
            } => {
                self.write_ability_player(player);
                self.write_mana_pool(mana);
                self.write_u32(amount);
            }
        }
    }

    fn write_activation_cost(&mut self, cost: ActivationCost) {
        self.write_mana_cost(cost.mana);
        self.write_bool(cost.tap_source);
        self.write_bool(cost.sacrifice_source);
        self.write_optional_i32(cost.loyalty_delta);
    }

    fn write_activation_condition(&mut self, condition: Option<ActivationCondition>) {
        match condition {
            None => self.write_u8(0),
            Some(
                condition @ ActivationCondition::ControllerControlsAtLeast { predicate, count },
            ) => {
                self.write_u8(1);
                self.write_u8(condition.canonical_code());
                self.write_object_target_predicate(predicate);
                self.write_u32(count);
            }
        }
    }

    fn write_mana_cost(&mut self, cost: ManaCost) {
        for kind in COLORED_MANA_KINDS {
            self.write_u32(cost.colored(kind));
        }
        self.write_u32(cost.generic);
        self.write_u32(cost.x_count);
        self.write_u32(cost.x_value);
    }

    fn write_activated_ability_definition(&mut self, definition: ActivatedAbilityDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_activation_timing(definition.timing);
        self.write_activation_cost(definition.cost);
        self.write_activated_ability_effect(definition.effect);
        self.write_bool(definition.mana_ability);
        self.write_activation_condition(definition.condition);
    }

    fn write_activated_ability_subscription(&mut self, subscription: ActivatedAbilitySubscription) {
        self.write_u32(subscription.id.0);
        self.write_activated_ability_definition(subscription.definition);
    }

    fn write_cost_modifier_scope(&mut self, scope: CostModifierScope) {
        self.write_u8(scope.canonical_code());
        match scope {
            CostModifierScope::AllActivatedAbilities => {}
            CostModifierScope::Ability(ability) => self.write_u32(ability.0),
            CostModifierScope::Source(object) => self.write_u32(object.0),
            CostModifierScope::Controller(player) => self.write_u32(player.0),
            CostModifierScope::Spells(predicate) => {
                self.write_object_target_predicate(predicate);
            }
        }
    }

    fn write_cost_modifier_operation(&mut self, operation: CostModifierOperation) {
        self.write_u8(operation.canonical_code());
        match operation {
            CostModifierOperation::AddManaCost(cost) => self.write_mana_cost(cost),
            CostModifierOperation::AddGeneric(amount)
            | CostModifierOperation::ReduceGeneric(amount) => self.write_u32(amount),
        }
    }

    fn write_cost_modifier_definition(&mut self, definition: CostModifierDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_cost_modifier_scope(definition.scope);
        self.write_cost_modifier_operation(definition.operation);
    }

    fn write_cost_modifier_subscription(&mut self, subscription: CostModifierSubscription) {
        self.write_u32(subscription.id.0);
        self.write_cost_modifier_definition(subscription.definition);
    }

    fn write_target_restriction_subject(&mut self, subject: TargetRestrictionSubject) {
        self.write_u8(subject.canonical_code());
        if let TargetRestrictionSubject::Object(object) = subject {
            self.write_u32(object.0);
        }
    }

    fn write_target_restriction(&mut self, restriction: TargetRestriction) {
        self.write_u8(restriction.canonical_code());
        match restriction {
            TargetRestriction::Shroud | TargetRestriction::Hexproof => {}
            TargetRestriction::ProtectionFromColors { colors } => self.write_object_colors(colors),
            TargetRestriction::Ward { cost } => self.write_mana_cost(cost),
        }
    }

    fn write_combat_restriction_subject(&mut self, subject: CombatRestrictionSubject) {
        self.write_u8(subject.canonical_code());
        match subject {
            CombatRestrictionSubject::Object(object) => self.write_u32(object.0),
            CombatRestrictionSubject::ControlledBy(player) => self.write_u32(player.0),
            CombatRestrictionSubject::AllObjects => {}
        }
    }

    fn write_combat_restriction(&mut self, restriction: CombatRestriction) {
        self.write_u8(restriction.canonical_code());
    }

    fn write_player_rule_subject(&mut self, subject: PlayerRuleSubject) {
        self.write_u8(subject.canonical_code());
        if let PlayerRuleSubject::Player(player) = subject {
            self.write_u32(player.0);
        }
    }

    fn write_player_rule(&mut self, rule: PlayerRule) {
        self.write_u8(rule.canonical_code());
    }

    fn write_restriction_effect(&mut self, effect: RestrictionEffect) {
        self.write_u8(effect.canonical_code());
        match effect {
            RestrictionEffect::Targeting {
                subject,
                restriction,
            } => {
                self.write_target_restriction_subject(subject);
                self.write_target_restriction(restriction);
            }
            RestrictionEffect::Combat {
                subject,
                restriction,
            } => {
                self.write_combat_restriction_subject(subject);
                self.write_combat_restriction(restriction);
            }
            RestrictionEffect::PlayerRule { subject, rule } => {
                self.write_player_rule_subject(subject);
                self.write_player_rule(rule);
            }
            RestrictionEffect::Indestructible { object }
            | RestrictionEffect::RegenerationShield { object } => self.write_u32(object.0),
        }
    }

    fn write_restriction_definition(&mut self, definition: RestrictionDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_restriction_effect(definition.effect);
        self.write_u8(definition.duration.canonical_code());
    }

    fn write_restriction_subscription(&mut self, subscription: RestrictionSubscription) {
        self.write_u32(subscription.id.0);
        self.write_restriction_definition(subscription.definition);
    }

    fn write_replacement_source_filter(&mut self, filter: ReplacementSourceFilter) {
        self.write_u8(filter.canonical_code());
        if let ReplacementSourceFilter::Object(object) = filter {
            self.write_u32(object.0);
        }
    }

    fn write_replacement_damage_target_filter(&mut self, filter: ReplacementDamageTargetFilter) {
        self.write_u8(filter.canonical_code());
        match filter {
            ReplacementDamageTargetFilter::Any => {}
            ReplacementDamageTargetFilter::Player(player) => self.write_u32(player.0),
            ReplacementDamageTargetFilter::Object(object) => self.write_u32(object.0),
        }
    }

    fn write_replacement_condition(&mut self, condition: ReplacementCondition) {
        self.write_u8(condition.canonical_code());
        match condition {
            ReplacementCondition::DamageWouldBeDealt {
                source,
                target,
                combat_only,
            } => {
                self.write_replacement_source_filter(source);
                self.write_replacement_damage_target_filter(target);
                self.write_bool(combat_only);
            }
        }
    }

    fn write_replacement_operation(&mut self, operation: ReplacementOperation) {
        self.write_u8(operation.canonical_code());
        match operation {
            ReplacementOperation::PreventAllDamage | ReplacementOperation::DoubleDamage => {}
            ReplacementOperation::PreventDamage(amount)
            | ReplacementOperation::AddDamage(amount)
            | ReplacementOperation::SetDamage(amount) => self.write_u32(amount),
        }
    }

    fn write_replacement_duration(&mut self, duration: ReplacementDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_replacement_definition(&mut self, definition: ReplacementDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_replacement_condition(definition.condition);
        self.write_replacement_operation(definition.operation);
        self.write_replacement_duration(definition.duration);
        self.write_bool(definition.self_replacement);
    }

    fn write_replacement_subscription(&mut self, subscription: ReplacementSubscription) {
        self.write_u32(subscription.id.0);
        self.write_replacement_definition(subscription.definition);
    }

    fn write_replacement_choice_order(&mut self, order: &ReplacementChoiceOrder) {
        self.write_u32(order.chooser.0);
        self.write_u32(order.order.len() as u32);
        for replacement in &order.order {
            self.write_u32(replacement.0);
        }
    }

    fn write_continuous_effect_target(&mut self, target: ContinuousEffectTarget) {
        self.write_u8(target.canonical_code());
        match target {
            ContinuousEffectTarget::Object(object) => self.write_u32(object.0),
            ContinuousEffectTarget::AllObjects | ContinuousEffectTarget::AttachedToSource => {}
            ContinuousEffectTarget::Objects {
                predicate,
                excluded,
            } => {
                self.write_object_target_predicate(predicate);
                self.write_optional_object(excluded);
            }
        }
    }

    fn write_continuous_effect_layer(&mut self, layer: ContinuousEffectLayer) {
        self.write_u8(layer.canonical_code());
    }

    fn write_continuous_effect_duration(&mut self, duration: ContinuousEffectDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_continuous_effect_operation(&mut self, operation: ContinuousEffectOperation) {
        self.write_u8(operation.canonical_code());
        self.write_continuous_effect_layer(operation.layer());
        match operation {
            ContinuousEffectOperation::CopyBaseCreature { from } => self.write_u32(from.0),
            ContinuousEffectOperation::ChangeController { controller } => {
                self.write_u32(controller.0);
            }
            ContinuousEffectOperation::SetTextMarker { marker } => self.write_u32(marker),
            ContinuousEffectOperation::SetTypes { types }
            | ContinuousEffectOperation::AddTypes { types }
            | ContinuousEffectOperation::RemoveTypes { types } => self.write_object_types(types),
            ContinuousEffectOperation::SetColors { colors } => self.write_object_colors(colors),
            ContinuousEffectOperation::AddKeywords { keywords }
            | ContinuousEffectOperation::RemoveKeywords { keywords } => {
                self.write_creature_keywords(keywords);
            }
            ContinuousEffectOperation::SetBasePowerToughness { power, toughness }
            | ContinuousEffectOperation::SetPowerToughness { power, toughness }
            | ContinuousEffectOperation::ModifyPowerToughness { power, toughness } => {
                self.write_i32(power);
                self.write_i32(toughness);
            }
            ContinuousEffectOperation::SwitchPowerToughness => {}
        }
    }

    fn write_continuous_effect_definition(&mut self, definition: &ContinuousEffectDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_continuous_effect_target(definition.target);
        self.write_continuous_effect_operation(definition.operation);
        self.write_continuous_effect_duration(definition.duration);
        self.write_u64(definition.timestamp);
        self.write_u32(definition.dependencies.len() as u32);
        for dependency in &definition.dependencies {
            self.write_u32(dependency.0);
        }
        self.write_continuous_effect_condition(definition.condition());
    }

    fn write_continuous_effect_condition(&mut self, condition: ContinuousEffectCondition) {
        self.write_u8(condition.canonical_code());
        if let ContinuousEffectCondition::ControllerDevotionLessThan { color, threshold } =
            condition
        {
            self.write_u8(color.index() as u8);
            self.write_u32(threshold);
        }
    }

    fn write_continuous_effect_subscription(
        &mut self,
        subscription: &ContinuousEffectSubscription,
    ) {
        self.write_u32(subscription.id.0);
        self.write_continuous_effect_definition(&subscription.definition);
    }

    fn write_stack_entry(&mut self, entry: &StackEntry) {
        self.write_u32(entry.id.0);
        self.write_u32(entry.controller.0);
        self.write_optional_object(entry.object);
        self.write_optional_trigger(entry.trigger);
        self.write_optional_activated_ability(entry.activated_ability);
        self.write_u8(entry.kind.canonical_code());
        self.write_u32(entry.targets.len() as u32);
        for target in &entry.targets {
            self.write_target_snapshot(*target);
        }
        self.write_optional_payment_plan(entry.payment);
        self.write_optional_stack_copy_info(entry.copy_info);
        self.write_bool(entry.kicked);
        self.write_bool(entry.flashback);
        self.write_bool(entry.split_second);
    }

    fn write_resolution_record(&mut self, record: &ResolutionRecord) {
        self.write_u32(record.stack_entry.0);
        self.write_u32(record.controller.0);
        self.write_optional_object(record.object);
        self.write_optional_trigger(record.trigger);
        self.write_optional_activated_ability(record.activated_ability);
        self.write_u8(record.kind.canonical_code());
        self.write_u32(record.targets.len() as u32);
        for target in &record.targets {
            self.write_target_snapshot(*target);
        }
        self.write_u32(record.legal_targets.len() as u32);
        for legal in &record.legal_targets {
            self.write_bool(*legal);
        }
        self.write_u8(record.outcome.canonical_code());
        self.write_optional_stack_copy_info(record.copy_info);
        self.write_bool(record.kicked);
        self.write_bool(record.flashback);
        self.write_bool(record.split_second);
    }

    fn write_event_record(&mut self, record: EventRecord) {
        self.write_u64(record.sequence);
        self.write_u32(record.turn);
        self.write_game_event(record.event);
    }

    fn write_game_event(&mut self, event: GameEvent) {
        self.write_u8(event.canonical_code());
        match event {
            GameEvent::SeedSet { seed } => self.write_u64(seed),
            GameEvent::PlayerAdded { player }
            | GameEvent::OpeningHandKept { player }
            | GameEvent::PriorityPassed { player }
            | GameEvent::EmptyLibraryDraw { player } => self.write_u32(player.0),
            GameEvent::TurnOrderDecided { starting_player } => self.write_u32(starting_player.0),
            GameEvent::OpeningHandsDrawn
            | GameEvent::CleanupPriorityRequested
            | GameEvent::ManaPoolsCleared => {}
            GameEvent::MulliganTaken {
                player,
                mulligans_taken,
            } => {
                self.write_u32(player.0);
                self.write_u32(mulligans_taken);
            }
            GameEvent::OpeningHandCardBottomed { player, object } => {
                self.write_u32(player.0);
                self.write_u32(object.0);
            }
            GameEvent::PlayerMaxHandSizeSet {
                player,
                max_hand_size,
            } => {
                self.write_u32(player.0);
                self.write_u32(max_hand_size);
            }
            GameEvent::LifeTotalSet { player, life } => {
                self.write_u32(player.0);
                self.write_i32(life);
            }
            GameEvent::LifeLost {
                player,
                amount,
                life,
            }
            | GameEvent::LifeGained {
                player,
                amount,
                life,
            } => {
                self.write_u32(player.0);
                self.write_u32(amount);
                self.write_i32(life);
            }
            GameEvent::PoisonCountersAdded {
                player,
                amount,
                poison,
            } => {
                self.write_u32(player.0);
                self.write_u32(amount);
                self.write_u32(poison);
            }
            GameEvent::ManaPoolChanged { player, mana_pool } => {
                self.write_u32(player.0);
                self.write_mana_pool(mana_pool);
            }
            GameEvent::ManaPaid {
                player,
                payment,
                mana_pool,
            } => {
                self.write_u32(player.0);
                self.write_payment_plan(payment);
                self.write_mana_pool(mana_pool);
            }
            GameEvent::ObjectCreated {
                object,
                card,
                owner,
                controller,
                zone,
            } => {
                self.write_u32(object.0);
                self.write_u32(card.0);
                self.write_u32(owner.0);
                self.write_u32(controller.0);
                self.write_zone_id(zone);
            }
            GameEvent::ObjectMoved { object, from, to } => {
                self.write_u32(object.0);
                self.write_zone_id(from);
                self.write_zone_id(to);
            }
            GameEvent::ZoneShuffled { zone } => self.write_zone_id(zone),
            GameEvent::BaseCreatureCharacteristicsSet { object, base } => {
                self.write_u32(object.0);
                self.write_base_creature_characteristics(base);
            }
            GameEvent::BaseCreatureCharacteristicsCleared { object } => {
                self.write_u32(object.0);
            }
            GameEvent::ObjectTapped { object, tapped } => {
                self.write_u32(object.0);
                self.write_bool(tapped);
            }
            GameEvent::DamageMarked {
                object,
                amount,
                total_damage,
            } => {
                self.write_u32(object.0);
                self.write_u32(amount);
                self.write_u32(total_damage);
            }
            GameEvent::TurnStarted {
                turn,
                active_player,
            } => {
                self.write_u32(turn);
                self.write_u32(active_player.0);
            }
            GameEvent::StepEnded { step } | GameEvent::StepBegan { step } => {
                self.write_u8(step.canonical_code());
            }
            GameEvent::PriorityChanged { player } => self.write_optional_player(player),
            GameEvent::StackEntryAdded {
                entry,
                controller,
                object,
                kind,
            } => {
                self.write_u32(entry.0);
                self.write_u32(controller.0);
                self.write_optional_object(object);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::StackEntryResolved { entry, outcome } => {
                self.write_u32(entry.0);
                self.write_u8(outcome.canonical_code());
            }
            GameEvent::AttackersDeclared { player, count } => {
                self.write_u32(player.0);
                self.write_u32(count);
            }
            GameEvent::AttackDeclared {
                attacker,
                defending_player,
            } => {
                self.write_u32(attacker.0);
                self.write_u32(defending_player.0);
            }
            GameEvent::BlockersDeclared {
                defending_player,
                count,
            } => {
                self.write_u32(defending_player.0);
                self.write_u32(count);
            }
            GameEvent::BlockDeclared { blocker, attacker } => {
                self.write_u32(blocker.0);
                self.write_u32(attacker.0);
            }
            GameEvent::CombatDamageDealt { record } => {
                self.write_combat_damage_record(record);
            }
            GameEvent::PlayerLostByStateBasedAction { player, kind } => {
                self.write_u32(player.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::PermanentMovedByStateBasedAction { object, kind } => {
                self.write_u32(object.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::GameOutcomeChanged { outcome } => self.write_game_outcome(outcome),
            GameEvent::DurationMarkerAdded { marker, duration } => {
                self.write_u32(marker.0);
                self.write_effect_duration(duration);
            }
            GameEvent::DurationMarkersExpired { duration, count } => {
                self.write_effect_duration(duration);
                self.write_u32(count);
            }
            GameEvent::CleanupPerformed { report } => self.write_cleanup_report(report),
            GameEvent::TriggeredAbilityRegistered {
                trigger,
                controller,
                source,
                event_kind,
                duration,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_game_event_kind(event_kind);
                self.write_trigger_duration(duration);
            }
            GameEvent::TriggeredAbilityQueued {
                trigger,
                controller,
                event_sequence,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(controller.0);
                self.write_u64(event_sequence);
            }
            GameEvent::TriggeredAbilityPutOnStack {
                trigger,
                entry,
                controller,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(entry.0);
                self.write_u32(controller.0);
            }
            GameEvent::ReplacementEffectRegistered {
                replacement,
                controller,
                source,
                operation,
                duration,
                self_replacement,
            } => {
                self.write_u32(replacement.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_replacement_operation(operation);
                self.write_replacement_duration(duration);
                self.write_bool(self_replacement);
            }
            GameEvent::ReplacementChoiceOrderSet { chooser, count } => {
                self.write_u32(chooser.0);
                self.write_u32(count);
            }
            GameEvent::ReplacementEffectApplied {
                replacement,
                chooser,
                source,
                target,
                operation,
                original_amount,
                resulting_amount,
            } => {
                self.write_u32(replacement.0);
                self.write_u32(chooser.0);
                self.write_optional_object(source);
                self.write_combat_damage_target(target);
                self.write_replacement_operation(operation);
                self.write_u32(original_amount);
                self.write_u32(resulting_amount);
            }
            GameEvent::ContinuousEffectRegistered {
                effect,
                controller,
                source,
                target,
                operation,
                layer,
                timestamp,
            } => {
                self.write_u32(effect.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_continuous_effect_target(target);
                self.write_continuous_effect_operation(operation);
                self.write_continuous_effect_layer(layer);
                self.write_u64(timestamp);
            }
            GameEvent::ObjectLoyaltySet { object, loyalty } => {
                self.write_u32(object.0);
                self.write_optional_i32(loyalty);
            }
            GameEvent::ActivatedAbilityRegistered {
                ability,
                controller,
                source,
                mana_ability,
            } => {
                self.write_u32(ability.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_bool(mana_ability);
            }
            GameEvent::CostModifierRegistered {
                modifier,
                controller,
                source,
                operation,
            } => {
                self.write_u32(modifier.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_cost_modifier_operation(operation);
            }
            GameEvent::ActivatedAbilityActivated {
                ability,
                player,
                source,
                mana_ability,
            } => {
                self.write_u32(ability.0);
                self.write_u32(player.0);
                self.write_optional_object(source);
                self.write_bool(mana_ability);
            }
            GameEvent::ActivatedAbilityResolved {
                ability,
                player,
                source,
                effect,
            } => {
                self.write_u32(ability.0);
                self.write_u32(player.0);
                self.write_optional_object(source);
                self.write_activated_ability_effect(effect);
            }
            GameEvent::RestrictionRegistered {
                restriction,
                controller,
                source,
                effect,
            } => {
                self.write_u32(restriction.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_restriction_effect(effect);
            }
            GameEvent::ObjectCountersChanged {
                object,
                kind,
                count,
            } => {
                self.write_u32(object.0);
                self.write_counter_kind(kind);
                self.write_u32(count);
            }
            GameEvent::TokenCreated {
                object,
                card,
                owner,
                controller,
            } => {
                self.write_u32(object.0);
                self.write_u32(card.0);
                self.write_u32(owner.0);
                self.write_u32(controller.0);
            }
            GameEvent::ObjectCopyCreated {
                object,
                source,
                token,
            } => {
                self.write_u32(object.0);
                self.write_u32(source.0);
                self.write_bool(token);
            }
            GameEvent::StackEntryCopied {
                source,
                copy,
                controller,
            } => {
                self.write_u32(source.0);
                self.write_u32(copy.0);
                self.write_u32(controller.0);
            }
            GameEvent::ObjectCeasedToExist { object, kind } => {
                self.write_u32(object.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::CounterPairCancelled { object, amount } => {
                self.write_u32(object.0);
                self.write_u32(amount);
            }
            GameEvent::ObjectColorIdentitySet { object, colors } => {
                self.write_u32(object.0);
                self.write_object_colors(colors);
            }
            GameEvent::CommanderDesignated {
                object,
                player,
                color_identity,
            } => {
                self.write_u32(object.0);
                self.write_u32(player.0);
                self.write_object_colors(color_identity);
            }
            GameEvent::CommanderCastRecorded {
                object,
                player,
                cast_count,
                tax,
            } => {
                self.write_u32(object.0);
                self.write_u32(player.0);
                self.write_u32(cast_count);
                self.write_mana_cost(tax);
            }
            GameEvent::CommanderColorIdentityValidated {
                player,
                count,
                allowed,
            } => {
                self.write_u32(player.0);
                self.write_u32(count);
                self.write_object_colors(allowed);
            }
            GameEvent::LibraryManipulated {
                player,
                operation,
                count,
                moved,
            } => {
                self.write_u32(player.0);
                self.write_u8(operation.canonical_code());
                self.write_u32(count);
                self.write_u32(moved);
            }
            GameEvent::ObjectAttached { attachment, target } => {
                self.write_u32(attachment.0);
                self.write_optional_object(target);
            }
            GameEvent::BaseObjectCharacteristicsSet { object, base } => {
                self.write_u32(object.0);
                self.write_base_object_characteristics(base);
            }
            GameEvent::NoncombatDamageDealt {
                source,
                target,
                amount,
            } => {
                self.write_optional_object(source);
                self.write_combat_damage_target(target);
                self.write_u32(amount);
            }
            GameEvent::CardDrawn { player, object } => {
                self.write_u32(player.0);
                self.write_u32(object.0);
            }
            GameEvent::ObjectRevealed { object } => self.write_u32(object.0),
        }
    }

    fn write_combat_damage_step(&mut self, step: Option<CombatDamageStepKind>) {
        match step {
            Some(step) => {
                self.write_u8(1);
                self.write_u8(step.canonical_code());
            }
            None => self.write_u8(0),
        }
    }

    fn write_combat_damage_target(&mut self, target: CombatDamageTarget) {
        self.write_u8(target.canonical_code());
        match target {
            CombatDamageTarget::Player(player) => self.write_u32(player.0),
            CombatDamageTarget::Object(object) => self.write_u32(object.0),
        }
    }

    fn write_attacking_creature(&mut self, attacker: &AttackingCreature) {
        self.write_u32(attacker.object.0);
        self.write_u32(attacker.defending_player.0);
        self.write_bool(attacker.blocked);
        self.write_u32(attacker.blockers.len() as u32);
        for blocker in &attacker.blockers {
            self.write_u32(blocker.0);
        }
    }

    fn write_blocking_creature(&mut self, blocker: BlockingCreature) {
        self.write_u32(blocker.object.0);
        self.write_u32(blocker.attacker.0);
    }

    fn write_combat_damage_record(&mut self, record: CombatDamageRecord) {
        self.write_u32(record.source.0);
        self.write_combat_damage_target(record.target);
        self.write_u32(record.amount);
        self.write_u8(record.step.canonical_code());
        self.write_bool(record.source_had_deathtouch);
        self.write_bool(record.source_had_lifelink);
    }

    fn write_combat_state(&mut self, combat: &CombatState) {
        self.write_u32(combat.attackers.len() as u32);
        for attacker in &combat.attackers {
            self.write_attacking_creature(attacker);
        }
        self.write_u32(combat.blockers.len() as u32);
        for blocker in &combat.blockers {
            self.write_blocking_creature(*blocker);
        }
        self.write_u32(combat.damage_records.len() as u32);
        for record in &combat.damage_records {
            self.write_combat_damage_record(*record);
        }
        self.write_combat_damage_step(combat.damage_step);
        self.write_u32(combat.first_strike_participants.len() as u32);
        for participant in &combat.first_strike_participants {
            self.write_u32(participant.0);
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

    fn write_optional_i32(&mut self, value: Option<i32>) {
        match value {
            Some(value) => {
                self.write_u8(1);
                self.write_i32(value);
            }
            None => self.write_u8(0),
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

    fn write_optional_trigger(&mut self, trigger: Option<TriggerId>) {
        match trigger {
            Some(trigger) => {
                self.write_u8(1);
                self.write_u32(trigger.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_optional_activated_ability(&mut self, ability: Option<ActivatedAbilityId>) {
        match ability {
            Some(ability) => {
                self.write_u8(1);
                self.write_u32(ability.0);
            }
            None => self.write_u8(0),
        }
    }

    fn write_optional_stack_copy_info(&mut self, copy_info: Option<StackCopyInfo>) {
        match copy_info {
            Some(copy_info) => {
                self.write_u8(1);
                self.write_u32(copy_info.source_entry.0);
                self.write_optional_object(copy_info.source_object);
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

    fn write_game_outcome(&mut self, outcome: GameOutcome) {
        self.write_u8(outcome.canonical_code());
        if let GameOutcome::Won(player) = outcome {
            self.write_u32(player.0);
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

    fn write_mana_pool(&mut self, pool: ManaPool) {
        for amount in pool.amounts {
            self.write_u32(amount);
        }
    }

    fn write_counter_kind(&mut self, kind: CounterKind) {
        self.write_u8(kind.canonical_code());
        if let CounterKind::Named(id) = kind {
            self.write_u32(id);
        }
    }

    fn write_object_counter_entry(&mut self, entry: ObjectCounterEntry) {
        self.write_u32(entry.object.0);
        self.write_counter_kind(entry.kind);
        self.write_u32(entry.count);
    }

    fn write_creature_keywords(&mut self, keywords: CreatureKeywords) {
        self.write_u32(u32::from(keywords.canonical_bits()));
    }

    fn write_object_colors(&mut self, colors: ObjectColors) {
        self.write_u8(colors.canonical_bits());
    }

    fn write_object_types(&mut self, types: ObjectTypes) {
        self.write_u8(types.canonical_bits());
    }

    fn write_object_supertypes(&mut self, supertypes: ObjectSupertypes) {
        self.write_u8(supertypes.canonical_bits());
    }

    fn write_basic_land_types(&mut self, land_types: BasicLandTypes) {
        self.write_u8(land_types.canonical_bits());
    }

    fn write_object_subtypes(&mut self, subtypes: ObjectSubtypes) {
        self.write_u8(subtypes.len);
        for subtype in subtypes.as_slice() {
            self.write_u8(subtype.len);
            for byte in subtype.as_bytes() {
                self.write_u8(*byte);
            }
        }
    }

    fn write_base_object_characteristics(&mut self, base: BaseObjectCharacteristics) {
        self.write_object_types(base.types());
        self.write_object_colors(base.colors());
        self.write_object_supertypes(base.supertypes());
        self.write_basic_land_types(base.basic_land_types());
        self.write_object_subtypes(base.subtypes());
        self.write_u32(base.mana_value());
        self.write_mana_pool(base.printed_mana_symbols());
        self.write_bool(base.enters_tapped());
    }

    fn write_base_creature_characteristics(&mut self, base: BaseCreatureCharacteristics) {
        self.write_i32(base.power);
        self.write_i32(base.toughness);
        self.write_creature_keywords(base.keywords);
    }

    fn write_optional_base_creature_characteristics(
        &mut self,
        base: Option<BaseCreatureCharacteristics>,
    ) {
        match base {
            Some(base) => {
                self.write_u8(1);
                self.write_base_creature_characteristics(base);
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

    fn write_optional_u32(&mut self, value: Option<u32>) {
        match value {
            Some(value) => {
                self.write_u8(1);
                self.write_u32(value);
            }
            None => self.write_u8(0),
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

    fn write_target_kind(&mut self, kind: TargetKind) {
        self.write_u8(kind.canonical_code());
        match kind {
            TargetKind::ObjectInZone(zone) => self.write_zone_id(zone),
            TargetKind::ObjectInZoneKind(kind) => self.write_u8(kind.canonical_code()),
            TargetKind::Player
            | TargetKind::Permanent
            | TargetKind::StackEntry
            | TargetKind::PlayerOrPermanent => {}
        }
    }

    fn write_target_controller_predicate(&mut self, predicate: TargetControllerPredicate) {
        self.write_u8(predicate.canonical_code());
        if let TargetControllerPredicate::Player(player) = predicate {
            self.write_u32(player.0);
        }
    }

    fn write_object_target_predicate(&mut self, predicate: ObjectTargetPredicate) {
        self.write_target_controller_predicate(predicate.controller);
        self.write_target_controller_predicate(predicate.owner);
        self.write_object_types(predicate.required_types);
        self.write_object_types(predicate.required_any_types);
        self.write_object_types(predicate.forbidden_types);
        self.write_object_colors(predicate.required_colors);
        self.write_object_colors(predicate.forbidden_colors);
        self.write_creature_keywords(predicate.required_keywords);
        self.write_creature_keywords(predicate.forbidden_keywords);
        self.write_object_subtypes(predicate.required_subtypes);
        self.write_optional_u32(predicate.minimum_mana_value);
        self.write_optional_u32(predicate.maximum_mana_value);
    }

    fn write_player_target_predicate(&mut self, predicate: PlayerTargetPredicate) {
        self.write_u8(predicate.canonical_code());
        if let PlayerTargetPredicate::Player(player) = predicate {
            self.write_u32(player.0);
        }
    }

    fn write_target_predicate(&mut self, predicate: TargetPredicate) {
        self.write_u8(predicate.canonical_code());
        match predicate {
            TargetPredicate::Any => {}
            TargetPredicate::Object(predicate) => self.write_object_target_predicate(predicate),
            TargetPredicate::Player(predicate) => self.write_player_target_predicate(predicate),
            TargetPredicate::PlayerOrObject { player, object } => {
                self.write_player_target_predicate(player);
                self.write_object_target_predicate(object);
            }
        }
    }

    fn write_target_requirement(&mut self, requirement: TargetRequirement) {
        self.write_target_kind(requirement.kind);
        self.write_target_predicate(requirement.predicate);
    }

    fn write_target_choice(&mut self, choice: TargetChoice) {
        self.write_u8(choice.canonical_code());
        match choice {
            TargetChoice::Player(player) => self.write_u32(player.0),
            TargetChoice::Object(object) => self.write_u32(object.0),
            TargetChoice::StackEntry(entry) => self.write_u32(entry.0),
        }
    }

    fn write_optional_zone(&mut self, zone: Option<ZoneId>) {
        match zone {
            Some(zone) => {
                self.write_u8(1);
                self.write_zone_id(zone);
            }
            None => self.write_u8(0),
        }
    }

    fn write_target_snapshot(&mut self, target: TargetSnapshot) {
        self.write_target_requirement(target.requirement);
        self.write_target_choice(target.choice);
        self.write_optional_zone(target.original_zone);
        self.write_mana_cost(target.ward_cost);
    }

    fn write_payment_plan(&mut self, payment: PaymentPlan) {
        self.write_mana_pool(payment.paid);
        self.write_mana_pool(payment.generic_paid);
        self.write_u32(payment.generic_required);
        self.write_u32(payment.x_value);
        self.write_u32(payment.waste_score);
    }

    fn write_optional_payment_plan(&mut self, payment: Option<PaymentPlan>) {
        match payment {
            Some(payment) => {
                self.write_u8(1);
                self.write_payment_plan(payment);
            }
            None => self.write_u8(0),
        }
    }

    fn write_game_event_kind(&mut self, kind: GameEventKind) {
        self.write_u8(kind.canonical_code());
    }

    fn write_trigger_object_filter(&mut self, filter: TriggerObjectFilter) {
        self.write_u8(filter.canonical_code());
        if let TriggerObjectFilter::Object(object) = filter {
            self.write_u32(object.0);
        }
    }

    fn write_trigger_player_filter(&mut self, filter: TriggerPlayerFilter) {
        self.write_u8(filter.canonical_code());
        if let TriggerPlayerFilter::Player(player) = filter {
            self.write_u32(player.0);
        }
    }

    fn write_trigger_zone_filter(&mut self, filter: TriggerZoneFilter) {
        self.write_u8(filter.canonical_code());
        match filter {
            TriggerZoneFilter::Any => {}
            TriggerZoneFilter::Exact(zone) => self.write_zone_id(zone),
            TriggerZoneFilter::Kind(kind) => self.write_u8(kind.canonical_code()),
            TriggerZoneFilter::Owned { owner, kind } => {
                self.write_trigger_player_filter(owner);
                self.write_u8(kind.canonical_code());
            }
        }
    }

    fn write_trigger_condition(&mut self, condition: TriggerCondition) {
        self.write_u8(condition.canonical_code());
        match condition {
            TriggerCondition::EventKind(kind) => self.write_game_event_kind(kind),
            TriggerCondition::ObjectMoved { object, from, to } => {
                self.write_trigger_object_filter(object);
                self.write_trigger_zone_filter(from);
                self.write_trigger_zone_filter(to);
            }
            TriggerCondition::StepBegan { step } => self.write_u8(step.canonical_code()),
            TriggerCondition::LifeLost { player } | TriggerCondition::LifeGained { player } => {
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::DamageMarked { object } => {
                self.write_trigger_object_filter(object);
            }
            TriggerCondition::StackEntryResolved { kind, outcome } => {
                match kind {
                    Some(kind) => {
                        self.write_u8(1);
                        self.write_u8(kind.canonical_code());
                    }
                    None => self.write_u8(0),
                }
                match outcome {
                    Some(outcome) => {
                        self.write_u8(1);
                        self.write_u8(outcome.canonical_code());
                    }
                    None => self.write_u8(0),
                }
            }
            TriggerCondition::AttackDeclared { attacker } => {
                self.write_trigger_object_filter(attacker);
            }
            TriggerCondition::StepBeganFor { step, player } => {
                self.write_u8(step.canonical_code());
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::StackEntryAdded {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            }
            | TriggerCondition::StackEntryAddedOrCopied {
                controller,
                required_types,
                required_any_types,
                forbidden_types,
            } => {
                self.write_trigger_player_filter(controller);
                self.write_object_types(required_types);
                self.write_object_types(required_any_types);
                self.write_object_types(forbidden_types);
            }
            TriggerCondition::CombatDamageToPlayer { source } => {
                self.write_object_target_predicate(source);
            }
            TriggerCondition::PlayerDrewCard { player } => {
                self.write_trigger_player_filter(player);
            }
            TriggerCondition::PermanentEnteredBattlefield {
                predicate,
                exclude_source,
            } => {
                self.write_object_target_predicate(predicate);
                self.write_bool(exclude_source);
            }
        }
    }

    fn write_trigger_intervening_if(&mut self, intervening_if: TriggerInterveningIf) {
        self.write_u8(intervening_if.canonical_code());
        match intervening_if {
            TriggerInterveningIf::Always | TriggerInterveningIf::ControllerControlsSource => {}
            TriggerInterveningIf::SourceInZone(zone) => self.write_zone_id(zone),
            TriggerInterveningIf::ControllerLifeAtMost(life) => self.write_i32(life),
        }
    }

    fn write_trigger_duration(&mut self, duration: TriggerDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_trigger_definition(&mut self, definition: TriggerDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_trigger_condition(definition.condition);
        self.write_trigger_intervening_if(definition.intervening_if);
        self.write_trigger_duration(definition.duration);
    }

    fn write_trigger_subscription(&mut self, subscription: TriggerSubscription) {
        self.write_u32(subscription.id.0);
        self.write_trigger_definition(subscription.definition);
        self.write_game_event_kind(subscription.event_kind);
    }

    fn write_pending_trigger(&mut self, trigger: PendingTriggeredAbility) {
        self.write_u32(trigger.trigger.0);
        self.write_u32(trigger.controller.0);
        self.write_optional_object(trigger.source);
        self.write_u64(trigger.event_sequence);
        self.write_u32(trigger.event_turn);
    }

    fn write_activation_timing(&mut self, timing: ActivationTiming) {
        self.write_u8(timing.canonical_code());
    }

    fn write_ability_player(&mut self, player: AbilityPlayer) {
        self.write_u8(player.canonical_code());
        if let AbilityPlayer::Player(player) = player {
            self.write_u32(player.0);
        }
    }

    fn write_activated_ability_effect(&mut self, effect: ActivatedAbilityEffect) {
        self.write_u8(effect.canonical_code());
        match effect {
            ActivatedAbilityEffect::AddMana { player, mana } => {
                self.write_ability_player(player);
                self.write_mana_pool(mana);
            }
            ActivatedAbilityEffect::GainLife { player, amount }
            | ActivatedAbilityEffect::LoseLife { player, amount } => {
                self.write_ability_player(player);
                self.write_u32(amount);
            }
            ActivatedAbilityEffect::AddManaAndDealDamage {
                player,
                mana,
                amount,
            } => {
                self.write_ability_player(player);
                self.write_mana_pool(mana);
                self.write_u32(amount);
            }
        }
    }

    fn write_mana_cost(&mut self, cost: ManaCost) {
        for kind in COLORED_MANA_KINDS {
            self.write_u32(cost.colored(kind));
        }
        self.write_u32(cost.generic);
        self.write_u32(cost.x_count);
        self.write_u32(cost.x_value);
    }

    fn write_activation_cost(&mut self, cost: ActivationCost) {
        self.write_mana_cost(cost.mana);
        self.write_bool(cost.tap_source);
        self.write_bool(cost.sacrifice_source);
        self.write_optional_i32(cost.loyalty_delta);
    }

    fn write_activation_condition(&mut self, condition: Option<ActivationCondition>) {
        match condition {
            None => self.write_u8(0),
            Some(
                condition @ ActivationCondition::ControllerControlsAtLeast { predicate, count },
            ) => {
                self.write_u8(1);
                self.write_u8(condition.canonical_code());
                self.write_object_target_predicate(predicate);
                self.write_u32(count);
            }
        }
    }

    fn write_activated_ability_definition(&mut self, definition: ActivatedAbilityDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_activation_timing(definition.timing);
        self.write_activation_cost(definition.cost);
        self.write_activated_ability_effect(definition.effect);
        self.write_bool(definition.mana_ability);
        self.write_activation_condition(definition.condition);
    }

    fn write_activated_ability_subscription(&mut self, subscription: ActivatedAbilitySubscription) {
        self.write_u32(subscription.id.0);
        self.write_activated_ability_definition(subscription.definition);
    }

    fn write_cost_modifier_scope(&mut self, scope: CostModifierScope) {
        self.write_u8(scope.canonical_code());
        match scope {
            CostModifierScope::AllActivatedAbilities => {}
            CostModifierScope::Ability(ability) => self.write_u32(ability.0),
            CostModifierScope::Source(object) => self.write_u32(object.0),
            CostModifierScope::Controller(player) => self.write_u32(player.0),
            CostModifierScope::Spells(predicate) => {
                self.write_object_target_predicate(predicate);
            }
        }
    }

    fn write_cost_modifier_operation(&mut self, operation: CostModifierOperation) {
        self.write_u8(operation.canonical_code());
        match operation {
            CostModifierOperation::AddManaCost(cost) => self.write_mana_cost(cost),
            CostModifierOperation::AddGeneric(amount)
            | CostModifierOperation::ReduceGeneric(amount) => self.write_u32(amount),
        }
    }

    fn write_cost_modifier_definition(&mut self, definition: CostModifierDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_cost_modifier_scope(definition.scope);
        self.write_cost_modifier_operation(definition.operation);
    }

    fn write_cost_modifier_subscription(&mut self, subscription: CostModifierSubscription) {
        self.write_u32(subscription.id.0);
        self.write_cost_modifier_definition(subscription.definition);
    }

    fn write_target_restriction_subject(&mut self, subject: TargetRestrictionSubject) {
        self.write_u8(subject.canonical_code());
        if let TargetRestrictionSubject::Object(object) = subject {
            self.write_u32(object.0);
        }
    }

    fn write_target_restriction(&mut self, restriction: TargetRestriction) {
        self.write_u8(restriction.canonical_code());
        match restriction {
            TargetRestriction::Shroud | TargetRestriction::Hexproof => {}
            TargetRestriction::ProtectionFromColors { colors } => self.write_object_colors(colors),
            TargetRestriction::Ward { cost } => self.write_mana_cost(cost),
        }
    }

    fn write_combat_restriction_subject(&mut self, subject: CombatRestrictionSubject) {
        self.write_u8(subject.canonical_code());
        match subject {
            CombatRestrictionSubject::Object(object) => self.write_u32(object.0),
            CombatRestrictionSubject::ControlledBy(player) => self.write_u32(player.0),
            CombatRestrictionSubject::AllObjects => {}
        }
    }

    fn write_combat_restriction(&mut self, restriction: CombatRestriction) {
        self.write_u8(restriction.canonical_code());
    }

    fn write_player_rule_subject(&mut self, subject: PlayerRuleSubject) {
        self.write_u8(subject.canonical_code());
        if let PlayerRuleSubject::Player(player) = subject {
            self.write_u32(player.0);
        }
    }

    fn write_player_rule(&mut self, rule: PlayerRule) {
        self.write_u8(rule.canonical_code());
    }

    fn write_restriction_effect(&mut self, effect: RestrictionEffect) {
        self.write_u8(effect.canonical_code());
        match effect {
            RestrictionEffect::Targeting {
                subject,
                restriction,
            } => {
                self.write_target_restriction_subject(subject);
                self.write_target_restriction(restriction);
            }
            RestrictionEffect::Combat {
                subject,
                restriction,
            } => {
                self.write_combat_restriction_subject(subject);
                self.write_combat_restriction(restriction);
            }
            RestrictionEffect::PlayerRule { subject, rule } => {
                self.write_player_rule_subject(subject);
                self.write_player_rule(rule);
            }
            RestrictionEffect::Indestructible { object }
            | RestrictionEffect::RegenerationShield { object } => self.write_u32(object.0),
        }
    }

    fn write_restriction_definition(&mut self, definition: RestrictionDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_restriction_effect(definition.effect);
        self.write_u8(definition.duration.canonical_code());
    }

    fn write_restriction_subscription(&mut self, subscription: RestrictionSubscription) {
        self.write_u32(subscription.id.0);
        self.write_restriction_definition(subscription.definition);
    }

    fn write_replacement_source_filter(&mut self, filter: ReplacementSourceFilter) {
        self.write_u8(filter.canonical_code());
        if let ReplacementSourceFilter::Object(object) = filter {
            self.write_u32(object.0);
        }
    }

    fn write_replacement_damage_target_filter(&mut self, filter: ReplacementDamageTargetFilter) {
        self.write_u8(filter.canonical_code());
        match filter {
            ReplacementDamageTargetFilter::Any => {}
            ReplacementDamageTargetFilter::Player(player) => self.write_u32(player.0),
            ReplacementDamageTargetFilter::Object(object) => self.write_u32(object.0),
        }
    }

    fn write_replacement_condition(&mut self, condition: ReplacementCondition) {
        self.write_u8(condition.canonical_code());
        match condition {
            ReplacementCondition::DamageWouldBeDealt {
                source,
                target,
                combat_only,
            } => {
                self.write_replacement_source_filter(source);
                self.write_replacement_damage_target_filter(target);
                self.write_bool(combat_only);
            }
        }
    }

    fn write_replacement_operation(&mut self, operation: ReplacementOperation) {
        self.write_u8(operation.canonical_code());
        match operation {
            ReplacementOperation::PreventAllDamage | ReplacementOperation::DoubleDamage => {}
            ReplacementOperation::PreventDamage(amount)
            | ReplacementOperation::AddDamage(amount)
            | ReplacementOperation::SetDamage(amount) => self.write_u32(amount),
        }
    }

    fn write_replacement_duration(&mut self, duration: ReplacementDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_replacement_definition(&mut self, definition: ReplacementDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_replacement_condition(definition.condition);
        self.write_replacement_operation(definition.operation);
        self.write_replacement_duration(definition.duration);
        self.write_bool(definition.self_replacement);
    }

    fn write_replacement_subscription(&mut self, subscription: ReplacementSubscription) {
        self.write_u32(subscription.id.0);
        self.write_replacement_definition(subscription.definition);
    }

    fn write_replacement_choice_order(&mut self, order: &ReplacementChoiceOrder) {
        self.write_u32(order.chooser.0);
        self.write_u32(order.order.len() as u32);
        for replacement in &order.order {
            self.write_u32(replacement.0);
        }
    }

    fn write_continuous_effect_target(&mut self, target: ContinuousEffectTarget) {
        self.write_u8(target.canonical_code());
        match target {
            ContinuousEffectTarget::Object(object) => self.write_u32(object.0),
            ContinuousEffectTarget::AllObjects | ContinuousEffectTarget::AttachedToSource => {}
            ContinuousEffectTarget::Objects {
                predicate,
                excluded,
            } => {
                self.write_object_target_predicate(predicate);
                self.write_optional_object(excluded);
            }
        }
    }

    fn write_continuous_effect_layer(&mut self, layer: ContinuousEffectLayer) {
        self.write_u8(layer.canonical_code());
    }

    fn write_continuous_effect_duration(&mut self, duration: ContinuousEffectDuration) {
        self.write_u8(duration.canonical_code());
    }

    fn write_continuous_effect_operation(&mut self, operation: ContinuousEffectOperation) {
        self.write_u8(operation.canonical_code());
        self.write_continuous_effect_layer(operation.layer());
        match operation {
            ContinuousEffectOperation::CopyBaseCreature { from } => self.write_u32(from.0),
            ContinuousEffectOperation::ChangeController { controller } => {
                self.write_u32(controller.0);
            }
            ContinuousEffectOperation::SetTextMarker { marker } => self.write_u32(marker),
            ContinuousEffectOperation::SetTypes { types }
            | ContinuousEffectOperation::AddTypes { types }
            | ContinuousEffectOperation::RemoveTypes { types } => self.write_object_types(types),
            ContinuousEffectOperation::SetColors { colors } => self.write_object_colors(colors),
            ContinuousEffectOperation::AddKeywords { keywords }
            | ContinuousEffectOperation::RemoveKeywords { keywords } => {
                self.write_creature_keywords(keywords);
            }
            ContinuousEffectOperation::SetBasePowerToughness { power, toughness }
            | ContinuousEffectOperation::SetPowerToughness { power, toughness }
            | ContinuousEffectOperation::ModifyPowerToughness { power, toughness } => {
                self.write_i32(power);
                self.write_i32(toughness);
            }
            ContinuousEffectOperation::SwitchPowerToughness => {}
        }
    }

    fn write_continuous_effect_definition(&mut self, definition: &ContinuousEffectDefinition) {
        self.write_u32(definition.controller.0);
        self.write_optional_object(definition.source);
        self.write_continuous_effect_target(definition.target);
        self.write_continuous_effect_operation(definition.operation);
        self.write_continuous_effect_duration(definition.duration);
        self.write_u64(definition.timestamp);
        self.write_u32(definition.dependencies.len() as u32);
        for dependency in &definition.dependencies {
            self.write_u32(dependency.0);
        }
        self.write_continuous_effect_condition(definition.condition());
    }

    fn write_continuous_effect_condition(&mut self, condition: ContinuousEffectCondition) {
        self.write_u8(condition.canonical_code());
        if let ContinuousEffectCondition::ControllerDevotionLessThan { color, threshold } =
            condition
        {
            self.write_u8(color.index() as u8);
            self.write_u32(threshold);
        }
    }

    fn write_continuous_effect_subscription(
        &mut self,
        subscription: &ContinuousEffectSubscription,
    ) {
        self.write_u32(subscription.id.0);
        self.write_continuous_effect_definition(&subscription.definition);
    }

    fn write_stack_entry(&mut self, entry: &StackEntry) {
        self.write_u32(entry.id.0);
        self.write_u32(entry.controller.0);
        self.write_optional_object(entry.object);
        self.write_optional_trigger(entry.trigger);
        self.write_optional_activated_ability(entry.activated_ability);
        self.write_u8(entry.kind.canonical_code());
        self.write_u32(entry.targets.len() as u32);
        for target in &entry.targets {
            self.write_target_snapshot(*target);
        }
        self.write_optional_payment_plan(entry.payment);
        self.write_optional_stack_copy_info(entry.copy_info);
        self.write_bool(entry.kicked);
        self.write_bool(entry.flashback);
        self.write_bool(entry.split_second);
    }

    fn write_resolution_record(&mut self, record: &ResolutionRecord) {
        self.write_u32(record.stack_entry.0);
        self.write_u32(record.controller.0);
        self.write_optional_object(record.object);
        self.write_optional_trigger(record.trigger);
        self.write_optional_activated_ability(record.activated_ability);
        self.write_u8(record.kind.canonical_code());
        self.write_u32(record.targets.len() as u32);
        for target in &record.targets {
            self.write_target_snapshot(*target);
        }
        self.write_u32(record.legal_targets.len() as u32);
        for legal in &record.legal_targets {
            self.write_bool(*legal);
        }
        self.write_u8(record.outcome.canonical_code());
        self.write_optional_stack_copy_info(record.copy_info);
        self.write_bool(record.kicked);
        self.write_bool(record.flashback);
        self.write_bool(record.split_second);
    }

    fn write_event_record(&mut self, record: EventRecord) {
        self.write_u64(record.sequence);
        self.write_u32(record.turn);
        self.write_game_event(record.event);
    }

    fn write_game_event(&mut self, event: GameEvent) {
        self.write_u8(event.canonical_code());
        match event {
            GameEvent::SeedSet { seed } => self.write_u64(seed),
            GameEvent::PlayerAdded { player }
            | GameEvent::OpeningHandKept { player }
            | GameEvent::PriorityPassed { player }
            | GameEvent::EmptyLibraryDraw { player } => self.write_u32(player.0),
            GameEvent::TurnOrderDecided { starting_player } => self.write_u32(starting_player.0),
            GameEvent::OpeningHandsDrawn
            | GameEvent::CleanupPriorityRequested
            | GameEvent::ManaPoolsCleared => {}
            GameEvent::MulliganTaken {
                player,
                mulligans_taken,
            } => {
                self.write_u32(player.0);
                self.write_u32(mulligans_taken);
            }
            GameEvent::OpeningHandCardBottomed { player, object } => {
                self.write_u32(player.0);
                self.write_u32(object.0);
            }
            GameEvent::PlayerMaxHandSizeSet {
                player,
                max_hand_size,
            } => {
                self.write_u32(player.0);
                self.write_u32(max_hand_size);
            }
            GameEvent::LifeTotalSet { player, life } => {
                self.write_u32(player.0);
                self.write_i32(life);
            }
            GameEvent::LifeLost {
                player,
                amount,
                life,
            }
            | GameEvent::LifeGained {
                player,
                amount,
                life,
            } => {
                self.write_u32(player.0);
                self.write_u32(amount);
                self.write_i32(life);
            }
            GameEvent::PoisonCountersAdded {
                player,
                amount,
                poison,
            } => {
                self.write_u32(player.0);
                self.write_u32(amount);
                self.write_u32(poison);
            }
            GameEvent::ManaPoolChanged { player, mana_pool } => {
                self.write_u32(player.0);
                self.write_mana_pool(mana_pool);
            }
            GameEvent::ManaPaid {
                player,
                payment,
                mana_pool,
            } => {
                self.write_u32(player.0);
                self.write_payment_plan(payment);
                self.write_mana_pool(mana_pool);
            }
            GameEvent::ObjectCreated {
                object,
                card,
                owner,
                controller,
                zone,
            } => {
                self.write_u32(object.0);
                self.write_u32(card.0);
                self.write_u32(owner.0);
                self.write_u32(controller.0);
                self.write_zone_id(zone);
            }
            GameEvent::ObjectMoved { object, from, to } => {
                self.write_u32(object.0);
                self.write_zone_id(from);
                self.write_zone_id(to);
            }
            GameEvent::ZoneShuffled { zone } => self.write_zone_id(zone),
            GameEvent::BaseCreatureCharacteristicsSet { object, base } => {
                self.write_u32(object.0);
                self.write_base_creature_characteristics(base);
            }
            GameEvent::BaseCreatureCharacteristicsCleared { object } => {
                self.write_u32(object.0);
            }
            GameEvent::ObjectTapped { object, tapped } => {
                self.write_u32(object.0);
                self.write_bool(tapped);
            }
            GameEvent::DamageMarked {
                object,
                amount,
                total_damage,
            } => {
                self.write_u32(object.0);
                self.write_u32(amount);
                self.write_u32(total_damage);
            }
            GameEvent::TurnStarted {
                turn,
                active_player,
            } => {
                self.write_u32(turn);
                self.write_u32(active_player.0);
            }
            GameEvent::StepEnded { step } | GameEvent::StepBegan { step } => {
                self.write_u8(step.canonical_code());
            }
            GameEvent::PriorityChanged { player } => self.write_optional_player(player),
            GameEvent::StackEntryAdded {
                entry,
                controller,
                object,
                kind,
            } => {
                self.write_u32(entry.0);
                self.write_u32(controller.0);
                self.write_optional_object(object);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::StackEntryResolved { entry, outcome } => {
                self.write_u32(entry.0);
                self.write_u8(outcome.canonical_code());
            }
            GameEvent::AttackersDeclared { player, count } => {
                self.write_u32(player.0);
                self.write_u32(count);
            }
            GameEvent::AttackDeclared {
                attacker,
                defending_player,
            } => {
                self.write_u32(attacker.0);
                self.write_u32(defending_player.0);
            }
            GameEvent::BlockersDeclared {
                defending_player,
                count,
            } => {
                self.write_u32(defending_player.0);
                self.write_u32(count);
            }
            GameEvent::BlockDeclared { blocker, attacker } => {
                self.write_u32(blocker.0);
                self.write_u32(attacker.0);
            }
            GameEvent::CombatDamageDealt { record } => {
                self.write_combat_damage_record(record);
            }
            GameEvent::PlayerLostByStateBasedAction { player, kind } => {
                self.write_u32(player.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::PermanentMovedByStateBasedAction { object, kind } => {
                self.write_u32(object.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::GameOutcomeChanged { outcome } => self.write_game_outcome(outcome),
            GameEvent::DurationMarkerAdded { marker, duration } => {
                self.write_u32(marker.0);
                self.write_effect_duration(duration);
            }
            GameEvent::DurationMarkersExpired { duration, count } => {
                self.write_effect_duration(duration);
                self.write_u32(count);
            }
            GameEvent::CleanupPerformed { report } => self.write_cleanup_report(report),
            GameEvent::TriggeredAbilityRegistered {
                trigger,
                controller,
                source,
                event_kind,
                duration,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_game_event_kind(event_kind);
                self.write_trigger_duration(duration);
            }
            GameEvent::TriggeredAbilityQueued {
                trigger,
                controller,
                event_sequence,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(controller.0);
                self.write_u64(event_sequence);
            }
            GameEvent::TriggeredAbilityPutOnStack {
                trigger,
                entry,
                controller,
            } => {
                self.write_u32(trigger.0);
                self.write_u32(entry.0);
                self.write_u32(controller.0);
            }
            GameEvent::ReplacementEffectRegistered {
                replacement,
                controller,
                source,
                operation,
                duration,
                self_replacement,
            } => {
                self.write_u32(replacement.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_replacement_operation(operation);
                self.write_replacement_duration(duration);
                self.write_bool(self_replacement);
            }
            GameEvent::ReplacementChoiceOrderSet { chooser, count } => {
                self.write_u32(chooser.0);
                self.write_u32(count);
            }
            GameEvent::ReplacementEffectApplied {
                replacement,
                chooser,
                source,
                target,
                operation,
                original_amount,
                resulting_amount,
            } => {
                self.write_u32(replacement.0);
                self.write_u32(chooser.0);
                self.write_optional_object(source);
                self.write_combat_damage_target(target);
                self.write_replacement_operation(operation);
                self.write_u32(original_amount);
                self.write_u32(resulting_amount);
            }
            GameEvent::ContinuousEffectRegistered {
                effect,
                controller,
                source,
                target,
                operation,
                layer,
                timestamp,
            } => {
                self.write_u32(effect.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_continuous_effect_target(target);
                self.write_continuous_effect_operation(operation);
                self.write_continuous_effect_layer(layer);
                self.write_u64(timestamp);
            }
            GameEvent::ObjectLoyaltySet { object, loyalty } => {
                self.write_u32(object.0);
                self.write_optional_i32(loyalty);
            }
            GameEvent::ActivatedAbilityRegistered {
                ability,
                controller,
                source,
                mana_ability,
            } => {
                self.write_u32(ability.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_bool(mana_ability);
            }
            GameEvent::CostModifierRegistered {
                modifier,
                controller,
                source,
                operation,
            } => {
                self.write_u32(modifier.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_cost_modifier_operation(operation);
            }
            GameEvent::ActivatedAbilityActivated {
                ability,
                player,
                source,
                mana_ability,
            } => {
                self.write_u32(ability.0);
                self.write_u32(player.0);
                self.write_optional_object(source);
                self.write_bool(mana_ability);
            }
            GameEvent::ActivatedAbilityResolved {
                ability,
                player,
                source,
                effect,
            } => {
                self.write_u32(ability.0);
                self.write_u32(player.0);
                self.write_optional_object(source);
                self.write_activated_ability_effect(effect);
            }
            GameEvent::RestrictionRegistered {
                restriction,
                controller,
                source,
                effect,
            } => {
                self.write_u32(restriction.0);
                self.write_u32(controller.0);
                self.write_optional_object(source);
                self.write_restriction_effect(effect);
            }
            GameEvent::ObjectCountersChanged {
                object,
                kind,
                count,
            } => {
                self.write_u32(object.0);
                self.write_counter_kind(kind);
                self.write_u32(count);
            }
            GameEvent::TokenCreated {
                object,
                card,
                owner,
                controller,
            } => {
                self.write_u32(object.0);
                self.write_u32(card.0);
                self.write_u32(owner.0);
                self.write_u32(controller.0);
            }
            GameEvent::ObjectCopyCreated {
                object,
                source,
                token,
            } => {
                self.write_u32(object.0);
                self.write_u32(source.0);
                self.write_bool(token);
            }
            GameEvent::StackEntryCopied {
                source,
                copy,
                controller,
            } => {
                self.write_u32(source.0);
                self.write_u32(copy.0);
                self.write_u32(controller.0);
            }
            GameEvent::ObjectCeasedToExist { object, kind } => {
                self.write_u32(object.0);
                self.write_u8(kind.canonical_code());
            }
            GameEvent::CounterPairCancelled { object, amount } => {
                self.write_u32(object.0);
                self.write_u32(amount);
            }
            GameEvent::ObjectColorIdentitySet { object, colors } => {
                self.write_u32(object.0);
                self.write_object_colors(colors);
            }
            GameEvent::CommanderDesignated {
                object,
                player,
                color_identity,
            } => {
                self.write_u32(object.0);
                self.write_u32(player.0);
                self.write_object_colors(color_identity);
            }
            GameEvent::CommanderCastRecorded {
                object,
                player,
                cast_count,
                tax,
            } => {
                self.write_u32(object.0);
                self.write_u32(player.0);
                self.write_u32(cast_count);
                self.write_mana_cost(tax);
            }
            GameEvent::CommanderColorIdentityValidated {
                player,
                count,
                allowed,
            } => {
                self.write_u32(player.0);
                self.write_u32(count);
                self.write_object_colors(allowed);
            }
            GameEvent::LibraryManipulated {
                player,
                operation,
                count,
                moved,
            } => {
                self.write_u32(player.0);
                self.write_u8(operation.canonical_code());
                self.write_u32(count);
                self.write_u32(moved);
            }
            GameEvent::ObjectAttached { attachment, target } => {
                self.write_u32(attachment.0);
                self.write_optional_object(target);
            }
            GameEvent::BaseObjectCharacteristicsSet { object, base } => {
                self.write_u32(object.0);
                self.write_base_object_characteristics(base);
            }
            GameEvent::NoncombatDamageDealt {
                source,
                target,
                amount,
            } => {
                self.write_optional_object(source);
                self.write_combat_damage_target(target);
                self.write_u32(amount);
            }
            GameEvent::CardDrawn { player, object } => {
                self.write_u32(player.0);
                self.write_u32(object.0);
            }
            GameEvent::ObjectRevealed { object } => self.write_u32(object.0),
        }
    }

    fn write_combat_damage_step(&mut self, step: Option<CombatDamageStepKind>) {
        match step {
            Some(step) => {
                self.write_u8(1);
                self.write_u8(step.canonical_code());
            }
            None => self.write_u8(0),
        }
    }

    fn write_combat_damage_target(&mut self, target: CombatDamageTarget) {
        self.write_u8(target.canonical_code());
        match target {
            CombatDamageTarget::Player(player) => self.write_u32(player.0),
            CombatDamageTarget::Object(object) => self.write_u32(object.0),
        }
    }

    fn write_attacking_creature(&mut self, attacker: &AttackingCreature) {
        self.write_u32(attacker.object.0);
        self.write_u32(attacker.defending_player.0);
        self.write_bool(attacker.blocked);
        self.write_u32(attacker.blockers.len() as u32);
        for blocker in &attacker.blockers {
            self.write_u32(blocker.0);
        }
    }

    fn write_blocking_creature(&mut self, blocker: BlockingCreature) {
        self.write_u32(blocker.object.0);
        self.write_u32(blocker.attacker.0);
    }

    fn write_combat_damage_record(&mut self, record: CombatDamageRecord) {
        self.write_u32(record.source.0);
        self.write_combat_damage_target(record.target);
        self.write_u32(record.amount);
        self.write_u8(record.step.canonical_code());
        self.write_bool(record.source_had_deathtouch);
        self.write_bool(record.source_had_lifelink);
    }

    fn write_combat_state(&mut self, combat: &CombatState) {
        self.write_u32(combat.attackers.len() as u32);
        for attacker in &combat.attackers {
            self.write_attacking_creature(attacker);
        }
        self.write_u32(combat.blockers.len() as u32);
        for blocker in &combat.blockers {
            self.write_blocking_creature(*blocker);
        }
        self.write_u32(combat.damage_records.len() as u32);
        for record in &combat.damage_records {
            self.write_combat_damage_record(*record);
        }
        self.write_combat_damage_step(combat.damage_step);
        self.write_u32(combat.first_strike_participants.len() as u32);
        for participant in &combat.first_strike_participants {
            self.write_u32(participant.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply, auto_payment_plan, crate_ready, enumerate_auto_tap_payment_plans,
        enumerate_payment_plans, legal_actions, state_based_action_table, validate_payment_plan,
        AbilityPlayer, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect,
        ActivationCondition, ActivationCost, ActivationTiming, AttackDeclaration,
        BaseCreatureCharacteristics, BaseObjectCharacteristics, BasicLandTypes, BlockDeclaration,
        CardId, CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest,
        CombatDamageStepKind, CombatDamageTarget, ContinuousEffectCondition,
        ContinuousEffectDefinition, ContinuousEffectDuration, ContinuousEffectId,
        ContinuousEffectOperation, ContinuousEffectTarget, CostModifierDefinition,
        CostModifierOperation, CostModifierScope, CreatureCharacteristics, CreatureKeywords,
        EffectDuration, EventReplayError, GameEvent, GameOutcome, GameState, HiddenCardDefinition,
        HiddenSlotDefinition, ManaCost, ManaKind, ManaPool, ManaSource, ObjectColors,
        ObjectSubtype, ObjectSubtypes, ObjectSupertypes, ObjectTargetPredicate, ObjectTypes,
        ObjectView, Outcome, PaymentError, Phase, PlayerId, PlayerRule, PlayerRuleSubject,
        PriorityOutcome, ReplacementCondition, ReplacementDamageTargetFilter,
        ReplacementDefinition, ReplacementDuration, ReplacementEffectId, ReplacementOperation,
        ReplacementSourceFilter, ResolutionOutcome, RestrictionDefinition, RestrictionEffect,
        SpellTiming, StackEntryId, StackEntryRequest, StackObjectKind, StateBasedActionKind,
        StateBasedActionReport, StateError, Step, TargetChoice, TargetControllerPredicate,
        TargetKind, TargetRequirement, TargetRestriction, TargetRestrictionSubject,
        TriggerCondition, TriggerDefinition, TriggerInterveningIf, TriggerObjectFilter,
        TriggerPlayerFilter, TriggerZoneFilter, ZoneConservation, ZoneId, ZoneKind,
        EVENT_RING_CAPACITY, NORMAL_TURN_STEPS, OPENING_HAND_SIZE, PAYMENT_PLAN_LIMIT,
    };

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn action_surface_creates_setup_state_without_public_mutators() {
        let mut state = GameState::new();

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 17 }),
            Outcome::Applied
        );
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected outcome: {other:?}"),
        };
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(700),
                owner: player,
                controller: player,
                zone: hand,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected outcome: {other:?}"),
        };

        assert_eq!(state.seed(), 17);
        assert_eq!(state.object_zone(object), Some(hand));
    }

    #[test]
    fn event_log_records_setup_and_zone_mutations() {
        let mut state = GameState::new();
        let cursor = state.event_cursor();

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 22 }),
            Outcome::Applied
        );
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player outcome: {other:?}"),
        };
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(2_101),
                owner: player,
                controller: player,
                zone: hand,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected object outcome: {other:?}"),
        };
        assert_eq!(
            apply(
                &mut state,
                Action::MoveObject {
                    object,
                    to: battlefield,
                },
            ),
            Outcome::Applied
        );

        let events: Vec<GameEvent> = state
            .events_since(cursor)
            .unwrap_or_else(|error| panic!("unexpected cursor error: {error:?}"))
            .iter()
            .map(|record| record.event())
            .collect();
        assert_eq!(
            events,
            vec![
                GameEvent::SeedSet { seed: 22 },
                GameEvent::PlayerAdded { player },
                GameEvent::ObjectCreated {
                    object,
                    card: CardId::new(2_101),
                    owner: player,
                    controller: player,
                    zone: hand,
                },
                GameEvent::ObjectMoved {
                    object,
                    from: hand,
                    to: battlefield,
                },
            ]
        );
        for (expected, record) in state.events_this_turn().iter().enumerate() {
            assert_eq!(record.sequence(), expected as u64);
            assert_eq!(record.turn(), 0);
        }
    }

    #[test]
    fn failed_actions_do_not_emit_events() {
        let mut state = GameState::new();
        let cursor = state.event_cursor();

        assert_eq!(
            apply(
                &mut state,
                Action::MoveObject {
                    object: super::ObjectId(99),
                    to: ZoneId::new(None, ZoneKind::Battlefield),
                },
            ),
            Outcome::Failed(StateError::UnknownObject(super::ObjectId(99)))
        );
        assert!(state
            .events_since(cursor)
            .unwrap_or_else(|error| panic!("unexpected cursor error: {error:?}"))
            .is_empty());
        assert_eq!(state.event_cursor(), cursor);
    }

    #[test]
    fn event_ring_is_bounded_and_reports_stale_cursors() {
        let mut state = GameState::new();
        let stale = state.event_cursor();
        for seed in 0..(EVENT_RING_CAPACITY as u64 + 3) {
            assert_eq!(
                apply(&mut state, Action::SetSeed { seed }),
                Outcome::Applied
            );
        }

        assert_eq!(state.events_this_turn().len(), EVENT_RING_CAPACITY);
        assert_eq!(state.events_this_turn()[0].sequence(), 3);
        assert_eq!(
            state.events_since(stale),
            Err(EventReplayError::CursorTooOld {
                requested: 0,
                oldest_retained: 3,
            })
        );
    }

    #[test]
    fn new_turn_resets_event_ring_and_invalidates_old_turn_cursor() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player outcome: {other:?}"),
        };
        let old_turn = state.event_cursor();

        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: player,
                },
            ),
            Outcome::Applied
        );

        assert_eq!(
            state.events_since(old_turn),
            Err(EventReplayError::CursorTurnMismatch {
                cursor_turn: 0,
                current_turn: 1,
            })
        );
        let events: Vec<GameEvent> = state
            .events_this_turn()
            .iter()
            .map(|record| record.event())
            .collect();
        assert_eq!(
            events,
            vec![
                GameEvent::TurnStarted {
                    turn: 1,
                    active_player: player,
                },
                GameEvent::StepBegan { step: Step::Untap },
            ]
        );
    }

    #[test]
    fn triggered_object_move_queues_and_stacks_before_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let controller = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let object = state
            .create_object(CardId::new(2_201), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected upkeep advance error: {error:?}"));

        let trigger = match apply(
            &mut state,
            Action::RegisterTriggeredAbility {
                definition: TriggerDefinition::new(
                    controller,
                    TriggerCondition::ObjectMoved {
                        object: TriggerObjectFilter::Object(object),
                        from: TriggerZoneFilter::Exact(hand),
                        to: TriggerZoneFilter::Exact(battlefield),
                    },
                ),
            },
        ) {
            Outcome::TriggerRegistered(trigger) => trigger,
            other => panic!("unexpected trigger registration outcome: {other:?}"),
        };

        assert_eq!(
            apply(
                &mut state,
                Action::MoveObject {
                    object,
                    to: battlefield,
                },
            ),
            Outcome::Applied
        );

        assert_eq!(state.pending_triggers().len(), 1);
        assert_eq!(state.pending_triggers()[0].trigger(), trigger);
        assert_eq!(state.pending_triggers()[0].controller(), controller);
        assert_eq!(
            legal_actions(&state).actions(),
            &[Action::PutPendingTriggeredAbilitiesOnStack]
        );
        assert_eq!(
            apply(&mut state, Action::PassPriority { player: active }),
            Outcome::Failed(StateError::PendingTriggeredAbilities)
        );

        let entries = match apply(&mut state, Action::PutPendingTriggeredAbilitiesOnStack) {
            Outcome::StackEntriesAdded(entries) => entries,
            other => panic!("unexpected pending trigger stack outcome: {other:?}"),
        };
        assert_eq!(entries.len(), 1);
        let top = state
            .stack_top()
            .unwrap_or_else(|| panic!("missing trigger stack entry"));
        assert_eq!(top.id(), entries[0]);
        assert_eq!(top.controller(), controller);
        assert_eq!(top.trigger(), Some(trigger));
        assert_eq!(top.kind(), StackObjectKind::TriggeredAbility);
        assert_eq!(state.priority_player(), Some(active));
        assert_eq!(
            state.events_this_turn().last().map(|record| record.event()),
            Some(GameEvent::TriggeredAbilityPutOnStack {
                trigger,
                entry: entries[0],
                controller,
            })
        );
    }

    #[test]
    fn opponent_card_draw_trigger_queues_once_per_successful_draw() {
        let mut state = GameState::new();
        let controller = add_player_action(&mut state);
        let opponent = add_player_action(&mut state);
        seed_library_cards(&mut state, opponent, 2_250, 2);
        let trigger = match apply(
            &mut state,
            Action::RegisterTriggeredAbility {
                definition: TriggerDefinition::new(
                    controller,
                    TriggerCondition::PlayerDrewCard {
                        player: TriggerPlayerFilter::OpponentOfController,
                    },
                ),
            },
        ) {
            Outcome::TriggerRegistered(trigger) => trigger,
            other => panic!("unexpected trigger registration outcome: {other:?}"),
        };

        assert_eq!(
            apply(
                &mut state,
                Action::DrawCards {
                    player: opponent,
                    count: 2,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.pending_triggers().len(), 2);
        assert!(state
            .pending_triggers()
            .iter()
            .all(|pending| pending.trigger() == trigger && pending.controller() == controller));
    }

    #[test]
    fn triggered_abilities_use_apnap_stack_order() {
        let mut state = GameState::new();
        let active = state.add_player();
        let second = state.add_player();
        let third = state.add_player();
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));

        for controller in [third, active, second] {
            assert!(matches!(
                apply(
                    &mut state,
                    Action::RegisterTriggeredAbility {
                        definition: TriggerDefinition::new(
                            controller,
                            TriggerCondition::LifeLost {
                                player: TriggerPlayerFilter::Any,
                            },
                        ),
                    },
                ),
                Outcome::TriggerRegistered(_)
            ));
        }

        assert_eq!(
            apply(
                &mut state,
                Action::LoseLife {
                    player: second,
                    amount: 1,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.pending_triggers().len(), 3);

        let entries = match apply(&mut state, Action::PutPendingTriggeredAbilitiesOnStack) {
            Outcome::StackEntriesAdded(entries) => entries,
            other => panic!("unexpected APNAP stack outcome: {other:?}"),
        };
        assert_eq!(entries.len(), 3);
        let controllers: Vec<PlayerId> = state
            .stack_entries()
            .iter()
            .map(|entry| entry.controller())
            .collect();
        assert_eq!(controllers, vec![active, second, third]);
        assert_eq!(
            state.stack_top().map(|entry| entry.controller()),
            Some(third)
        );
    }

    #[test]
    fn intervening_if_blocks_and_delayed_trigger_retires() {
        let mut blocked = GameState::new();
        let active = blocked.add_player();
        let source = blocked
            .create_object(
                CardId::new(2_202),
                active,
                active,
                ZoneId::new(Some(active), ZoneKind::Hand),
            )
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        assert!(matches!(
            apply(
                &mut blocked,
                Action::RegisterTriggeredAbility {
                    definition: TriggerDefinition::new(
                        active,
                        TriggerCondition::StepBegan { step: Step::Upkeep },
                    )
                    .with_source(source)
                    .with_intervening_if(TriggerInterveningIf::SourceInZone(ZoneId::new(
                        None,
                        ZoneKind::Battlefield,
                    ))),
                },
            ),
            Outcome::TriggerRegistered(_)
        ));
        blocked
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected blocked start error: {error:?}"));
        blocked
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blocked upkeep error: {error:?}"));
        assert!(blocked.pending_triggers().is_empty());
        assert_eq!(blocked.priority_player(), Some(active));

        let mut delayed = GameState::new();
        let active = delayed.add_player();
        let source = delayed
            .create_object(
                CardId::new(2_203),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected delayed source create error: {error:?}"));
        let trigger = match apply(
            &mut delayed,
            Action::RegisterTriggeredAbility {
                definition: TriggerDefinition::new(
                    active,
                    TriggerCondition::StepBegan { step: Step::Upkeep },
                )
                .with_source(source)
                .with_intervening_if(TriggerInterveningIf::ControllerControlsSource)
                .delayed_once(),
            },
        ) {
            Outcome::TriggerRegistered(trigger) => trigger,
            other => panic!("unexpected delayed registration outcome: {other:?}"),
        };
        delayed
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected delayed start error: {error:?}"));
        delayed
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected delayed upkeep error: {error:?}"));

        assert_eq!(delayed.pending_triggers().len(), 1);
        assert_eq!(delayed.pending_triggers()[0].trigger(), trigger);
        assert_eq!(delayed.trigger_subscriptions().count(), 0);
        assert_eq!(delayed.priority_player(), None);
    }

    #[test]
    fn trigger_state_participates_in_canonical_hashes() {
        let mut state = GameState::new();
        let active = state.add_player();
        assert!(matches!(
            apply(
                &mut state,
                Action::RegisterTriggeredAbility {
                    definition: TriggerDefinition::new(
                        active,
                        TriggerCondition::LifeGained {
                            player: TriggerPlayerFilter::Controller,
                        },
                    )
                    .with_intervening_if(TriggerInterveningIf::ControllerLifeAtMost(25)),
                },
            ),
            Outcome::TriggerRegistered(_)
        ));
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );

        assert_eq!(
            apply(
                &mut state,
                Action::GainLife {
                    player: active,
                    amount: 1,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.pending_triggers().len(), 1);
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn affected_player_orders_damage_replacements() {
        let mut default_order = GameState::new();
        let source_controller = default_order.add_player();
        let affected = default_order.add_player();
        let source = battlefield_creature(
            &mut default_order,
            source_controller,
            2_301,
            3,
            3,
            CreatureKeywords::default(),
        );
        let condition = ReplacementCondition::DamageWouldBeDealt {
            source: ReplacementSourceFilter::Any,
            target: ReplacementDamageTargetFilter::Player(affected),
            combat_only: true,
        };
        let double = register_replacement(
            &mut default_order,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::DoubleDamage),
        );
        let prevent = register_replacement(
            &mut default_order,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::PreventDamage(2)),
        );
        let record = combat_damage_record(source, CombatDamageTarget::Player(affected), 3);
        let final_record = default_order
            .apply_combat_damage(record)
            .unwrap_or_else(|error| panic!("unexpected combat damage error: {error:?}"))
            .unwrap_or_else(|| panic!("damage should remain after default replacements"));
        assert_eq!(final_record.amount(), 4);
        assert_eq!(default_order.players()[affected.index()].life(), 16);
        assert_eq!(
            replacement_applications(&default_order),
            vec![double, prevent]
        );

        let mut chosen_order = GameState::new();
        let source_controller = chosen_order.add_player();
        let affected = chosen_order.add_player();
        let source = battlefield_creature(
            &mut chosen_order,
            source_controller,
            2_302,
            3,
            3,
            CreatureKeywords::default(),
        );
        let double = register_replacement(
            &mut chosen_order,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::DoubleDamage),
        );
        let prevent = register_replacement(
            &mut chosen_order,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::PreventDamage(2)),
        );
        assert_eq!(
            apply(
                &mut chosen_order,
                Action::SetReplacementChoiceOrder {
                    chooser: affected,
                    order: vec![prevent, double],
                },
            ),
            Outcome::Applied
        );
        let final_record = chosen_order
            .apply_combat_damage(combat_damage_record(
                source,
                CombatDamageTarget::Player(affected),
                3,
            ))
            .unwrap_or_else(|error| panic!("unexpected chosen combat damage error: {error:?}"))
            .unwrap_or_else(|| panic!("damage should remain after chosen replacements"));
        assert_eq!(final_record.amount(), 2);
        assert_eq!(chosen_order.players()[affected.index()].life(), 18);
        assert_eq!(
            replacement_applications(&chosen_order),
            vec![prevent, double]
        );
    }

    #[test]
    fn self_replacement_applies_before_affected_choice_order() {
        let mut state = GameState::new();
        let source_controller = state.add_player();
        let affected = state.add_player();
        let source = battlefield_creature(
            &mut state,
            source_controller,
            2_303,
            3,
            3,
            CreatureKeywords::default(),
        );
        let condition = ReplacementCondition::DamageWouldBeDealt {
            source: ReplacementSourceFilter::Any,
            target: ReplacementDamageTargetFilter::Player(affected),
            combat_only: true,
        };
        let prevent = register_replacement(
            &mut state,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::PreventDamage(2)),
        );
        let self_double = register_replacement(
            &mut state,
            ReplacementDefinition::new(affected, condition, ReplacementOperation::DoubleDamage)
                .with_self_replacement(),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetReplacementChoiceOrder {
                    chooser: affected,
                    order: vec![prevent, self_double],
                },
            ),
            Outcome::Applied
        );

        let final_record = state
            .apply_combat_damage(combat_damage_record(
                source,
                CombatDamageTarget::Player(affected),
                3,
            ))
            .unwrap_or_else(|error| panic!("unexpected self replacement error: {error:?}"))
            .unwrap_or_else(|| panic!("damage should remain after self replacement"));
        assert_eq!(final_record.amount(), 4);
        assert_eq!(state.players()[affected.index()].life(), 16);
        assert_eq!(replacement_applications(&state), vec![self_double, prevent]);
    }

    #[test]
    fn prevent_all_combat_damage_blocks_lifelink_deathtouch_and_damage_marking() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let source =
            battlefield_creature(&mut state, active, 2_304, 3, 3, CreatureKeywords::default());
        let target = battlefield_creature(
            &mut state,
            defender,
            2_305,
            3,
            3,
            CreatureKeywords::default(),
        );
        let prevention = register_replacement(
            &mut state,
            ReplacementDefinition::new(
                defender,
                ReplacementCondition::DamageWouldBeDealt {
                    source: ReplacementSourceFilter::Any,
                    target: ReplacementDamageTargetFilter::Object(target),
                    combat_only: true,
                },
                ReplacementOperation::PreventAllDamage,
            ),
        );

        let final_record = state
            .apply_combat_damage(super::CombatDamageRecord {
                source,
                target: CombatDamageTarget::Object(target),
                amount: 3,
                step: CombatDamageStepKind::Regular,
                source_had_deathtouch: true,
                source_had_lifelink: true,
            })
            .unwrap_or_else(|error| panic!("unexpected prevention error: {error:?}"));
        assert_eq!(final_record, None);
        let target_record = state
            .objects()
            .get(target)
            .unwrap_or_else(|| panic!("missing target creature"));
        assert_eq!(target_record.damage_marked(), 0);
        assert!(!target_record.deathtouch_damage_marked());
        assert_eq!(state.players()[active.index()].life(), 20);
        assert_eq!(replacement_applications(&state), vec![prevention]);
        assert!(!state
            .events_this_turn()
            .iter()
            .any(|record| matches!(record.event(), GameEvent::DamageMarked { .. })));
    }

    #[test]
    fn replacement_state_participates_in_canonical_hashes() {
        let mut state = GameState::new();
        let active = state.add_player();
        let affected = state.add_player();
        let source =
            battlefield_creature(&mut state, active, 2_306, 2, 2, CreatureKeywords::default());
        let replacement = register_replacement(
            &mut state,
            ReplacementDefinition::new(
                affected,
                ReplacementCondition::DamageWouldBeDealt {
                    source: ReplacementSourceFilter::Any,
                    target: ReplacementDamageTargetFilter::Player(affected),
                    combat_only: true,
                },
                ReplacementOperation::PreventDamage(1),
            )
            .with_duration(ReplacementDuration::Once),
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetReplacementChoiceOrder {
                    chooser: affected,
                    order: vec![replacement],
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );

        let final_record = state
            .apply_combat_damage(combat_damage_record(
                source,
                CombatDamageTarget::Player(affected),
                2,
            ))
            .unwrap_or_else(|error| panic!("unexpected once replacement error: {error:?}"))
            .unwrap_or_else(|| panic!("one damage should remain"));
        assert_eq!(final_record.amount(), 1);
        assert_eq!(state.replacement_effects().count(), 0);
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    mod layers {
        use super::*;

        #[test]
        fn same_layer_dependency_overrides_timestamp_order() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let object = battlefield_creature(
                &mut state,
                controller,
                2_401,
                2,
                2,
                CreatureKeywords::none(),
            );
            let black = register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::SetColors {
                        colors: ObjectColors::none().with_black(),
                    },
                )
                .with_timestamp(10),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::SetColors {
                        colors: ObjectColors::none().with_green(),
                    },
                )
                .with_timestamp(1)
                .with_dependencies(vec![black]),
            );

            let characteristics = state
                .object_characteristics(object)
                .unwrap_or_else(|error| panic!("unexpected characteristics error: {error:?}"));
            assert_eq!(characteristics.colors(), ObjectColors::none().with_green());
            assert_eq!(
                state.deterministic_hash(),
                state.deterministic_hash_streaming()
            );
        }

        #[test]
        fn power_toughness_sublayers_apply_in_cr_order() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let object = battlefield_creature(
                &mut state,
                controller,
                2_402,
                1,
                4,
                CreatureKeywords::none(),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::SwitchPowerToughness,
                )
                .with_timestamp(2),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::ModifyPowerToughness {
                        power: 2,
                        toughness: -1,
                    },
                )
                .with_timestamp(1),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::SetPowerToughness {
                        power: 3,
                        toughness: 5,
                    },
                )
                .with_timestamp(5),
            );

            let creature = state
                .creature_characteristics(object)
                .unwrap_or_else(|error| panic!("unexpected creature error: {error:?}"));
            assert_eq!(creature.power(), 4);
            assert_eq!(creature.toughness(), 5);
        }

        #[test]
        fn copy_layer_uses_base_copiable_values() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let source = battlefield_creature(
                &mut state,
                controller,
                2_403,
                2,
                3,
                CreatureKeywords::none(),
            );
            let target = battlefield_creature(
                &mut state,
                controller,
                2_404,
                1,
                1,
                CreatureKeywords::none(),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(source),
                    ContinuousEffectOperation::ModifyPowerToughness {
                        power: 7,
                        toughness: 7,
                    },
                ),
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(target),
                    ContinuousEffectOperation::CopyBaseCreature { from: source },
                ),
            );

            let source_creature = state
                .creature_characteristics(source)
                .unwrap_or_else(|error| panic!("unexpected source creature error: {error:?}"));
            let target_creature = state
                .creature_characteristics(target)
                .unwrap_or_else(|error| panic!("unexpected target creature error: {error:?}"));
            assert_eq!(source_creature.power(), 9);
            assert_eq!(source_creature.toughness(), 10);
            assert_eq!(target_creature.power(), 2);
            assert_eq!(target_creature.toughness(), 3);
        }

        #[test]
        fn control_layer_updates_effective_controller() {
            let mut state = GameState::new();
            let original = state.add_player();
            let new_controller = state.add_player();
            let object =
                battlefield_creature(&mut state, original, 2_405, 2, 2, CreatureKeywords::none());
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    new_controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::ChangeController {
                        controller: new_controller,
                    },
                ),
            );

            assert_eq!(state.object_controller(object), Ok(new_controller));
            assert_ne!(
                state
                    .objects()
                    .get(object)
                    .map(|record| record.controller()),
                Some(new_controller)
            );
        }

        #[test]
        fn source_bound_predicate_effect_tracks_entrants_and_source_zone() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let opponent = state.add_player();
            let source = battlefield_creature(
                &mut state,
                controller,
                2_407,
                2,
                2,
                CreatureKeywords::none(),
            );
            let first = battlefield_creature(
                &mut state,
                controller,
                2_408,
                2,
                2,
                CreatureKeywords::none(),
            );
            let other =
                battlefield_creature(&mut state, opponent, 2_409, 2, 2, CreatureKeywords::none());
            let predicate = ObjectTargetPredicate::any()
                .with_controller(TargetControllerPredicate::You)
                .with_required_types(ObjectTypes::none().with_creature());
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Objects {
                        predicate,
                        excluded: Some(source),
                    },
                    ContinuousEffectOperation::ModifyPowerToughness {
                        power: 1,
                        toughness: 1,
                    },
                )
                .with_source(source)
                .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield),
            );

            assert_eq!(
                state.creature_characteristics(source).map(|c| c.power()),
                Ok(2)
            );
            assert_eq!(
                state.creature_characteristics(first).map(|c| c.power()),
                Ok(3)
            );
            assert_eq!(
                state.creature_characteristics(other).map(|c| c.power()),
                Ok(2)
            );

            let entrant = battlefield_creature(
                &mut state,
                controller,
                2_410,
                2,
                2,
                CreatureKeywords::none(),
            );
            assert_eq!(
                state.creature_characteristics(entrant).map(|c| c.power()),
                Ok(3)
            );

            state
                .move_object(source, ZoneId::new(Some(controller), ZoneKind::Graveyard))
                .unwrap_or_else(|error| panic!("unexpected source move error: {error:?}"));
            assert_eq!(
                state.creature_characteristics(first).map(|c| c.power()),
                Ok(2)
            );
        }

        #[test]
        fn source_bound_attachment_effects_follow_reattachment_and_expire() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let opponent = state.add_player();
            let source = state
                .create_object(
                    CardId::new(2_411),
                    controller,
                    controller,
                    ZoneId::new(None, ZoneKind::Battlefield),
                )
                .unwrap_or_else(|error| panic!("unexpected equipment create error: {error:?}"));
            state
                .set_base_object_characteristics(
                    source,
                    BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_artifact(),
                        ObjectColors::none(),
                    ),
                )
                .unwrap_or_else(|error| {
                    panic!("unexpected equipment characteristics error: {error:?}")
                });
            let first = battlefield_creature(
                &mut state,
                controller,
                2_412,
                2,
                2,
                CreatureKeywords::none(),
            );
            let second = battlefield_creature(
                &mut state,
                controller,
                2_413,
                2,
                2,
                CreatureKeywords::none(),
            );
            assert_eq!(
                apply(
                    &mut state,
                    Action::AttachObject {
                        attachment: source,
                        target: Some(first),
                    },
                ),
                Outcome::Applied
            );
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::AttachedToSource,
                    ContinuousEffectOperation::ModifyPowerToughness {
                        power: 1,
                        toughness: 1,
                    },
                )
                .with_source(source)
                .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield),
            );
            assert!(matches!(
                apply(
                    &mut state,
                    Action::RegisterRestriction {
                        definition: RestrictionDefinition::new(
                            controller,
                            RestrictionEffect::Targeting {
                                subject: TargetRestrictionSubject::AttachedToSource,
                                restriction: TargetRestriction::Shroud,
                            },
                        )
                        .with_source(source),
                    },
                ),
                Outcome::RestrictionRegistered(_)
            ));
            let trigger = TriggerDefinition::new(
                controller,
                TriggerCondition::AttackDeclared {
                    attacker: TriggerObjectFilter::AttachedToSource,
                },
            )
            .with_source(source);
            let requirement = [TargetRequirement::new(TargetKind::Permanent)];

            assert_eq!(
                state.creature_characteristics(first).map(|c| c.power()),
                Ok(3)
            );
            assert_eq!(
                state.creature_characteristics(second).map(|c| c.power()),
                Ok(2)
            );
            assert!(state
                .validate_target_choices(
                    opponent,
                    None,
                    &requirement,
                    &[TargetChoice::Object(first)],
                )
                .is_err());
            assert!(state.trigger_condition_matches(
                trigger,
                GameEvent::AttackDeclared {
                    attacker: first,
                    defending_player: opponent,
                },
            ));

            assert_eq!(
                apply(
                    &mut state,
                    Action::AttachObject {
                        attachment: source,
                        target: Some(second),
                    },
                ),
                Outcome::Applied
            );
            assert_eq!(
                state.creature_characteristics(first).map(|c| c.power()),
                Ok(2)
            );
            assert_eq!(
                state.creature_characteristics(second).map(|c| c.power()),
                Ok(3)
            );
            assert!(state
                .validate_target_choices(
                    opponent,
                    None,
                    &requirement,
                    &[TargetChoice::Object(first)],
                )
                .is_ok());
            assert!(state
                .validate_target_choices(
                    opponent,
                    None,
                    &requirement,
                    &[TargetChoice::Object(second)],
                )
                .is_err());
            assert!(!state.trigger_condition_matches(
                trigger,
                GameEvent::AttackDeclared {
                    attacker: first,
                    defending_player: opponent,
                },
            ));
            assert!(state.trigger_condition_matches(
                trigger,
                GameEvent::AttackDeclared {
                    attacker: second,
                    defending_player: opponent,
                },
            ));

            state
                .move_object(source, ZoneId::new(Some(controller), ZoneKind::Graveyard))
                .unwrap_or_else(|error| panic!("unexpected equipment move error: {error:?}"));
            assert_eq!(
                state.creature_characteristics(second).map(|c| c.power()),
                Ok(2)
            );
            assert!(state
                .validate_target_choices(
                    opponent,
                    None,
                    &requirement,
                    &[TargetChoice::Object(second)],
                )
                .is_ok());
            assert_eq!(
                state.deterministic_hash(),
                state.deterministic_hash_streaming()
            );
        }

        #[test]
        fn continuous_effect_state_participates_in_hashes() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let object = battlefield_creature(
                &mut state,
                controller,
                2_406,
                2,
                2,
                CreatureKeywords::none(),
            );
            let before = state.deterministic_hash();

            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(object),
                    ContinuousEffectOperation::SetTypes {
                        types: ObjectTypes::none().with_artifact().with_creature(),
                    },
                ),
            );

            assert_ne!(state.deterministic_hash(), before);
            assert_eq!(
                state.deterministic_hash(),
                state.deterministic_hash_streaming()
            );
        }

        #[test]
        fn devotion_condition_tracks_printed_symbols_and_toggles_type_removal() {
            let mut state = GameState::new();
            let controller = state.add_player();
            let source = battlefield_creature(
                &mut state,
                controller,
                2_420,
                6,
                5,
                CreatureKeywords::none(),
            );
            state
                .set_base_object_characteristics(
                    source,
                    BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_creature().with_enchantment(),
                        ObjectColors::none().with_red(),
                    )
                    .with_printed_mana_symbols(ManaPool::of(ManaKind::Red, 1)),
                )
                .unwrap_or_else(|error| panic!("unexpected source setup error: {error:?}"));
            register_continuous(
                &mut state,
                ContinuousEffectDefinition::new(
                    controller,
                    ContinuousEffectTarget::Object(source),
                    ContinuousEffectOperation::RemoveTypes {
                        types: ObjectTypes::none().with_creature(),
                    },
                )
                .with_source(source)
                .with_duration(ContinuousEffectDuration::WhileSourceOnBattlefield)
                .with_condition(
                    ContinuousEffectCondition::ControllerDevotionLessThan {
                        color: ManaKind::Red,
                        threshold: 5,
                    },
                ),
            );

            assert_eq!(state.controller_devotion(controller, ManaKind::Red), Ok(1));
            assert_eq!(
                state.creature_characteristics(source),
                Err(StateError::NotACreature(source))
            );

            let anchor = state
                .create_object(
                    CardId::new(2_421),
                    controller,
                    controller,
                    ZoneId::new(None, ZoneKind::Battlefield),
                )
                .unwrap_or_else(|error| panic!("unexpected anchor create error: {error:?}"));
            state
                .set_base_object_characteristics(
                    anchor,
                    BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_enchantment(),
                        ObjectColors::none().with_red(),
                    )
                    .with_printed_mana_symbols(ManaPool::of(ManaKind::Red, 4)),
                )
                .unwrap_or_else(|error| panic!("unexpected anchor setup error: {error:?}"));
            assert_eq!(state.controller_devotion(controller, ManaKind::Red), Ok(5));
            assert!(state.creature_characteristics(source).is_ok());

            state
                .move_object(anchor, ZoneId::new(Some(controller), ZoneKind::Graveyard))
                .unwrap_or_else(|error| panic!("unexpected anchor move error: {error:?}"));
            assert_eq!(state.controller_devotion(controller, ManaKind::Red), Ok(1));
            assert_eq!(
                state.creature_characteristics(source),
                Err(StateError::NotACreature(source))
            );
        }
    }

    #[test]
    fn legal_actions_return_actions_that_apply_accepts() {
        let mut state = GameState::new();
        let active = state.add_player();
        let next = state.add_player();

        assert_eq!(state.start_turn(active), Ok(()));
        assert_eq!(state.advance_step(), Ok(Step::Upkeep));

        let actions = legal_actions(&state);

        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions.actions(),
            &[Action::PassPriority { player: active }]
        );
        assert_eq!(
            apply(&mut state, actions.actions()[0].clone()),
            Outcome::Priority(PriorityOutcome::PassedTo(next))
        );
    }

    #[test]
    fn base_characteristics_are_derived_before_rules_use() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player outcome: {other:?}"),
        };
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(701),
                owner: player,
                controller: player,
                zone: battlefield,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected object outcome: {other:?}"),
        };
        let base = BaseCreatureCharacteristics::new(2, 0)
            .with_keywords(CreatureKeywords::none().with_vigilance());
        let base_object = BaseObjectCharacteristics::new(
            ObjectTypes::none().with_artifact().with_creature(),
            ObjectColors::none().with_blue(),
        );

        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object,
                    base: base_object,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseCreatureCharacteristics { object, base }
            ),
            Outcome::Applied
        );
        assert_eq!(
            state
                .objects()
                .get(object)
                .and_then(|record| record.base_creature()),
            Some(base)
        );
        assert_eq!(
            state
                .objects()
                .get(object)
                .map(|record| record.base_object()),
            Some(base_object)
        );
        let characteristics = state
            .object_characteristics(object)
            .unwrap_or_else(|error| panic!("unexpected object characteristics: {error:?}"));
        assert!(characteristics.types().artifact());
        assert!(characteristics.types().creature());
        assert!(characteristics.colors().blue());
        assert_eq!(
            state.creature_characteristics(object),
            Ok(CreatureCharacteristics::new(2, 0)
                .with_keywords(CreatureKeywords::none().with_vigilance()))
        );
        assert_eq!(
            apply(&mut state, Action::CheckStateBasedActions),
            Outcome::StateBasedActions(StateBasedActionReport {
                iterations: 1,
                actions_performed: 1,
                players_lost: 0,
                permanents_moved_to_graveyard: 1,
                empty_library_draw_losses: 0,
                zero_toughness_creatures: 1,
                lethal_damage_creatures: 0,
                deathtouch_damage_creatures: 0,
                tokens_ceased: 0,
                copies_ceased: 0,
                counter_pairs_cancelled: 0,
                counters_removed: 0,
            })
        );
        assert_eq!(
            state.object_zone(object),
            Some(ZoneId::new(Some(player), ZoneKind::Graveyard))
        );
    }

    #[test]
    fn basic_land_type_line_is_derived_and_participates_in_state_hashes() {
        let mut state = GameState::new();
        let player = add_player_action(&mut state);
        let forest = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(702),
                owner: player,
                controller: player,
                zone: ZoneId::new(Some(player), ZoneKind::Hand),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected forest outcome: {other:?}"),
        };
        let before = state.deterministic_hash();
        let base =
            BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), ObjectColors::none())
                .with_supertypes(ObjectSupertypes::none().with_basic())
                .with_basic_land_types(BasicLandTypes::none().with_forest());

        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: forest,
                    base,
                },
            ),
            Outcome::Applied
        );
        let characteristics = state
            .object_characteristics(forest)
            .unwrap_or_else(|error| panic!("unexpected forest characteristics: {error:?}"));
        assert!(characteristics.types().land());
        assert!(characteristics.supertypes().basic());
        assert!(characteristics.basic_land_types().forest());
        assert_ne!(state.deterministic_hash(), before);
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn player_view_hides_opponent_hand_and_library_objects() {
        let mut state = GameState::new();
        let alice = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected alice outcome: {other:?}"),
        };
        let bob = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected bob outcome: {other:?}"),
        };
        let alice_hand = ZoneId::new(Some(alice), ZoneKind::Hand);
        let alice_library = ZoneId::new(Some(alice), ZoneKind::Library);
        let bob_hand = ZoneId::new(Some(bob), ZoneKind::Hand);
        let bob_library = ZoneId::new(Some(bob), ZoneKind::Library);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);

        let alice_hand_object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(710),
                owner: alice,
                controller: alice,
                zone: alice_hand,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected alice hand object outcome: {other:?}"),
        };
        match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(714),
                owner: alice,
                controller: alice,
                zone: alice_library,
            },
        ) {
            Outcome::ObjectCreated(_) => {}
            other => panic!("unexpected alice library object outcome: {other:?}"),
        }
        match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(711),
                owner: bob,
                controller: bob,
                zone: bob_hand,
            },
        ) {
            Outcome::ObjectCreated(_) => {}
            other => panic!("unexpected bob hand object outcome: {other:?}"),
        }
        match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(712),
                owner: bob,
                controller: bob,
                zone: bob_library,
            },
        ) {
            Outcome::ObjectCreated(_) => {}
            other => panic!("unexpected bob library object outcome: {other:?}"),
        }
        let battlefield_object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(713),
                owner: bob,
                controller: bob,
                zone: battlefield,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected battlefield object outcome: {other:?}"),
        };

        let view = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("unexpected player view error: {error:?}"));
        let alice_hand_view = view
            .zone(alice_hand)
            .unwrap_or_else(|| panic!("missing alice hand view"));
        let alice_library_view = view
            .zone(alice_library)
            .unwrap_or_else(|| panic!("missing alice library view"));
        let bob_hand_view = view
            .zone(bob_hand)
            .unwrap_or_else(|| panic!("missing bob hand view"));
        let bob_library_view = view
            .zone(bob_library)
            .unwrap_or_else(|| panic!("missing bob library view"));
        let battlefield_view = view
            .zone(battlefield)
            .unwrap_or_else(|| panic!("missing battlefield view"));

        assert_eq!(alice_hand_view.objects().len(), 1);
        assert_eq!(
            alice_hand_view.objects()[0].known(),
            state.objects().get(alice_hand_object)
        );
        assert_eq!(
            alice_hand_view.objects()[0].characteristics(),
            Some(
                state
                    .object_characteristics(alice_hand_object)
                    .unwrap_or_else(|error| panic!(
                        "missing alice hand characteristics: {error:?}"
                    ))
            )
        );
        assert_eq!(alice_library_view.objects(), &[ObjectView::Hidden]);
        assert_eq!(bob_hand_view.objects(), &[ObjectView::Hidden]);
        assert_eq!(bob_library_view.objects(), &[ObjectView::Hidden]);
        assert_eq!(battlefield_view.objects().len(), 1);
        assert_eq!(
            battlefield_view.objects()[0].known(),
            state.objects().get(battlefield_object)
        );
        assert_eq!(
            battlefield_view.objects()[0].characteristics(),
            Some(
                state
                    .object_characteristics(battlefield_object)
                    .unwrap_or_else(|error| {
                        panic!("missing battlefield characteristics: {error:?}")
                    })
            )
        );
        assert!(bob_hand_view.objects()[0].is_hidden());
        assert_eq!(bob_hand_view.objects()[0].known(), None);
    }

    #[test]
    fn player_view_hash_ignores_hidden_identity_and_tracks_visible_identity() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        let hidden = create_object_action(
            &mut state,
            CardId::new(720),
            bob,
            ZoneId::new(Some(bob), ZoneKind::Hand),
        );
        let visible = create_object_action(
            &mut state,
            CardId::new(721),
            bob,
            ZoneId::new(None, ZoneKind::Battlefield),
        );
        let baseline = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("baseline view failed: {error:?}"))
            .deterministic_hash();

        let mut hidden_poisoned = state.clone();
        hidden_poisoned
            .objects
            .get_mut(hidden)
            .unwrap_or_else(|| panic!("missing hidden object"))
            .card = CardId::new(999_720);
        let hidden_poisoned_hash = hidden_poisoned
            .player_view(alice)
            .unwrap_or_else(|error| panic!("hidden-poison view failed: {error:?}"))
            .deterministic_hash();
        assert_eq!(baseline, hidden_poisoned_hash);

        let mut visible_changed = state.clone();
        visible_changed
            .objects
            .get_mut(visible)
            .unwrap_or_else(|| panic!("missing visible object"))
            .card = CardId::new(999_721);
        let visible_changed_hash = visible_changed
            .player_view(alice)
            .unwrap_or_else(|error| panic!("visible-change view failed: {error:?}"))
            .deterministic_hash();
        assert_ne!(baseline, visible_changed_hash);
    }

    #[test]
    fn player_view_rejects_unknown_observer() {
        let state = GameState::new();

        assert_eq!(
            state.player_view(PlayerId(99)),
            Err(StateError::UnknownPlayer(PlayerId(99)))
        );
    }

    #[test]
    fn player_view_retains_known_hidden_identity_until_library_shuffle() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        let bob_library = ZoneId::new(Some(bob), ZoneKind::Library);
        let object = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(714),
                owner: bob,
                controller: bob,
                zone: bob_library,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected hidden object outcome: {other:?}"),
        };

        let hidden = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("unexpected hidden view error: {error:?}"));
        assert!(hidden
            .zone(bob_library)
            .unwrap_or_else(|| panic!("missing library"))
            .objects()[0]
            .is_hidden());

        assert_eq!(
            apply(
                &mut state,
                Action::RevealObjects {
                    objects: vec![object],
                }
            ),
            Outcome::Applied
        );
        let revealed = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("unexpected revealed view error: {error:?}"));
        assert_eq!(
            revealed
                .zone(bob_library)
                .unwrap_or_else(|| panic!("missing revealed library"))
                .objects()[0]
                .known()
                .map(|record| record.card()),
            Some(CardId::new(714))
        );

        assert_eq!(
            apply(&mut state, Action::ShuffleLibrary { player: bob }),
            Outcome::Applied
        );
        let shuffled = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("unexpected shuffled view error: {error:?}"));
        assert!(shuffled
            .zone(bob_library)
            .unwrap_or_else(|| panic!("missing shuffled library"))
            .objects()[0]
            .is_hidden());
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn determinized_clone_rebinds_only_complete_hidden_slots() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        let bob_library = ZoneId::new(Some(bob), ZoneKind::Library);
        let first = create_object_action(&mut state, CardId::new(801), bob, bob_library);
        let second = create_object_action(&mut state, CardId::new(802), bob, bob_library);
        let creature = BaseCreatureCharacteristics::new(3, 4);
        let red = ObjectColors::none().with_red();
        let green = ObjectColors::none().with_green();
        let first_definition = HiddenCardDefinition::new(
            CardId::new(901),
            BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), red),
            Some(creature),
            red,
        );
        let second_definition = HiddenCardDefinition::new(
            CardId::new(902),
            BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), green),
            None,
            green,
        );

        let sampled = state
            .determinized_clone(
                alice,
                &[
                    HiddenSlotDefinition::new(bob_library, 0, first_definition),
                    HiddenSlotDefinition::new(bob_library, 1, second_definition),
                ],
            )
            .unwrap_or_else(|error| panic!("unexpected determinization error: {error:?}"));

        assert_eq!(
            state.object(first).map(|object| object.card()),
            Some(CardId::new(801))
        );
        assert_eq!(
            state.object(second).map(|object| object.card()),
            Some(CardId::new(802))
        );
        let sampled_first = sampled
            .object(first)
            .unwrap_or_else(|| panic!("missing first sampled object"));
        let sampled_second = sampled
            .object(second)
            .unwrap_or_else(|| panic!("missing second sampled object"));
        assert_eq!(sampled_first.card(), CardId::new(901));
        assert_eq!(sampled_first.base_creature(), Some(creature));
        assert_eq!(sampled_second.card(), CardId::new(902));
        assert_eq!(sampled_second.base_creature(), None);
        assert_ne!(state.deterministic_hash(), sampled.deterministic_hash());
        assert_eq!(
            sampled.deterministic_hash(),
            sampled.deterministic_hash_streaming()
        );
    }

    #[test]
    fn determinized_clone_rejects_partial_duplicate_and_known_assignments() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        let bob_library = ZoneId::new(Some(bob), ZoneKind::Library);
        let object = create_object_action(&mut state, CardId::new(811), bob, bob_library);
        let _other = create_object_action(&mut state, CardId::new(812), bob, bob_library);
        let definition = HiddenCardDefinition::new(
            CardId::new(911),
            BaseObjectCharacteristics::new(
                ObjectTypes::none().with_artifact(),
                ObjectColors::none(),
            ),
            None,
            ObjectColors::none(),
        );

        assert_eq!(
            state
                .determinized_clone(
                    alice,
                    &[HiddenSlotDefinition::new(bob_library, 0, definition)]
                )
                .err(),
            Some(StateError::DeterminizationSlotCountMismatch {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            state
                .determinized_clone(
                    alice,
                    &[
                        HiddenSlotDefinition::new(bob_library, 0, definition),
                        HiddenSlotDefinition::new(bob_library, 0, definition),
                    ],
                )
                .err(),
            Some(StateError::DuplicateDeterminizationSlot {
                zone: bob_library,
                slot: 0,
            })
        );

        assert_eq!(
            apply(
                &mut state,
                Action::RevealObjects {
                    objects: vec![object],
                }
            ),
            Outcome::Applied
        );
        assert_eq!(
            state
                .determinized_clone(
                    alice,
                    &[
                        HiddenSlotDefinition::new(bob_library, 0, definition),
                        HiddenSlotDefinition::new(bob_library, 1, definition),
                    ],
                )
                .err(),
            Some(StateError::DeterminizationSlotKnown {
                zone: bob_library,
                slot: 0,
            })
        );
    }

    #[test]
    fn turn_order_decision_is_deterministic_from_seed() {
        let mut left = GameState::new();
        let mut right = GameState::new();
        for state in [&mut left, &mut right] {
            assert_eq!(
                apply(state, Action::SetSeed { seed: 103 }),
                Outcome::Applied
            );
            add_player_action(state);
            add_player_action(state);
        }

        let left_start = apply(&mut left, Action::DecideTurnOrder);
        let right_start = apply(&mut right, Action::DecideTurnOrder);

        assert_eq!(left_start, right_start);
        assert_eq!(left.starting_player(), right.starting_player());
        assert_eq!(left.deterministic_hash(), right.deterministic_hash());
        assert_eq!(
            left.deterministic_hash(),
            left.deterministic_hash_streaming()
        );
        assert_eq!(
            apply(&mut left, Action::DecideTurnOrder),
            Outcome::Failed(StateError::TurnOrderAlreadyDecided)
        );
    }

    #[test]
    fn opening_hands_draw_in_seeded_turn_order() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        seed_library_cards(&mut state, alice, 1_000, OPENING_HAND_SIZE);
        seed_library_cards(&mut state, bob, 2_000, OPENING_HAND_SIZE);

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 777 }),
            Outcome::Applied
        );
        let starting = match apply(&mut state, Action::DecideTurnOrder) {
            Outcome::TurnOrderDecided(player) => player,
            other => panic!("unexpected turn-order outcome: {other:?}"),
        };
        assert_eq!(state.starting_player(), Some(starting));
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Applied
        );

        assert!(state.opening_hands_drawn());
        for player in [alice, bob] {
            assert_eq!(
                state
                    .zone(ZoneId::new(Some(player), ZoneKind::Hand))
                    .unwrap_or_else(|| panic!("hand zone missing"))
                    .objects()
                    .len(),
                OPENING_HAND_SIZE as usize
            );
            assert_eq!(
                state
                    .zone(ZoneId::new(Some(player), ZoneKind::Library))
                    .unwrap_or_else(|| panic!("library zone missing"))
                    .objects()
                    .len(),
                0
            );
            assert_eq!(state.players()[player.index()].mulligans_taken(), 0);
            assert!(!state.players()[player.index()].opening_hand_kept());
        }
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Failed(StateError::OpeningHandsAlreadyDrawn)
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn london_mulligan_redraws_and_bottoms_mulligan_count() {
        let mut state = GameState::new();
        let player = add_player_action(&mut state);
        seed_library_cards(&mut state, player, 3_000, OPENING_HAND_SIZE + 1);

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 91 }),
            Outcome::Applied
        );
        assert!(matches!(
            apply(&mut state, Action::DecideTurnOrder),
            Outcome::TurnOrderDecided(_)
        ));
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Applied
        );
        assert_eq!(
            apply(&mut state, Action::TakeMulligan { player }),
            Outcome::Applied
        );

        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        assert_eq!(state.players()[player.index()].mulligans_taken(), 1);
        assert_eq!(
            state
                .zone(hand)
                .unwrap_or_else(|| panic!("hand zone missing"))
                .objects()
                .len(),
            OPENING_HAND_SIZE as usize
        );
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects()
                .len(),
            1
        );

        let bottom = state
            .zone(hand)
            .unwrap_or_else(|| panic!("hand zone missing"))
            .objects()[0];
        assert_eq!(
            apply(
                &mut state,
                Action::KeepOpeningHand {
                    player,
                    bottom: vec![bottom],
                },
            ),
            Outcome::Applied
        );

        assert_eq!(state.players()[player.index()].mulligans_taken(), 1);
        assert!(state.players()[player.index()].opening_hand_kept());
        assert_eq!(
            state
                .zone(hand)
                .unwrap_or_else(|| panic!("hand zone missing"))
                .objects()
                .len(),
            (OPENING_HAND_SIZE - 1) as usize
        );
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects()[0],
            bottom
        );
        assert_eq!(
            apply(&mut state, Action::TakeMulligan { player }),
            Outcome::Failed(StateError::MulliganAfterKeep(player))
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn setup_start_turn_respects_keeps_and_chosen_starter() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        seed_library_cards(&mut state, alice, 3_500, OPENING_HAND_SIZE);
        seed_library_cards(&mut state, bob, 3_600, OPENING_HAND_SIZE);

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 144 }),
            Outcome::Applied
        );
        let starting = match apply(&mut state, Action::DecideTurnOrder) {
            Outcome::TurnOrderDecided(player) => player,
            other => panic!("unexpected turn-order outcome: {other:?}"),
        };
        let other = if starting == alice { bob } else { alice };
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Applied
        );

        assert!(matches!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: starting
                },
            ),
            Outcome::Failed(StateError::OpeningHandKeepPending(_))
        ));
        for player in [alice, bob] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::KeepOpeningHand {
                        player,
                        bottom: Vec::new(),
                    },
                ),
                Outcome::Applied
            );
        }
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: other
                },
            ),
            Outcome::Failed(StateError::StartingPlayerMismatch {
                expected: starting,
                actual: other,
            })
        );
        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: starting
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.active_player(), Some(starting));
    }

    #[test]
    fn starting_player_skips_first_turn_draw_step_after_turn_order_setup() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        let alice_library = ZoneId::new(Some(alice), ZoneKind::Library);
        let alice_hand = ZoneId::new(Some(alice), ZoneKind::Hand);
        seed_library_cards(&mut state, alice, 3_700, OPENING_HAND_SIZE + 1);
        seed_library_cards(&mut state, bob, 3_800, OPENING_HAND_SIZE);

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 3 }),
            Outcome::Applied
        );
        let starting = match apply(&mut state, Action::DecideTurnOrder) {
            Outcome::TurnOrderDecided(player) => player,
            other => panic!("unexpected turn-order outcome: {other:?}"),
        };
        assert_eq!(starting, alice);
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Applied
        );
        for player in [alice, bob] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::KeepOpeningHand {
                        player,
                        bottom: Vec::new(),
                    },
                ),
                Outcome::Applied
            );
        }

        assert_eq!(
            apply(
                &mut state,
                Action::StartTurn {
                    active_player: alice,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            apply(&mut state, Action::AdvanceStep),
            Outcome::StepAdvanced(Step::Upkeep)
        );
        assert_eq!(
            apply(&mut state, Action::AdvanceStep),
            Outcome::StepAdvanced(Step::Draw)
        );

        assert_eq!(state.current_step(), Some(Step::Draw));
        assert_eq!(state.priority_player(), Some(alice));
        assert_eq!(
            state
                .zone(alice_hand)
                .unwrap_or_else(|| panic!("alice hand missing"))
                .objects()
                .len(),
            OPENING_HAND_SIZE as usize
        );
        assert_eq!(
            state
                .zone(alice_library)
                .unwrap_or_else(|| panic!("alice library missing"))
                .objects()
                .len(),
            1
        );
    }

    #[test]
    fn opening_hand_player_view_hides_opponent_hand_and_libraries() {
        let mut state = GameState::new();
        let alice = add_player_action(&mut state);
        let bob = add_player_action(&mut state);
        seed_library_cards(&mut state, alice, 4_000, OPENING_HAND_SIZE + 1);
        seed_library_cards(&mut state, bob, 5_000, OPENING_HAND_SIZE + 1);

        assert_eq!(
            apply(&mut state, Action::SetSeed { seed: 43 }),
            Outcome::Applied
        );
        assert!(matches!(
            apply(&mut state, Action::DecideTurnOrder),
            Outcome::TurnOrderDecided(_)
        ));
        assert_eq!(
            apply(&mut state, Action::DrawOpeningHands),
            Outcome::Applied
        );

        let view = state
            .player_view(alice)
            .unwrap_or_else(|error| panic!("unexpected player view error: {error:?}"));
        let alice_hand = view
            .zone(ZoneId::new(Some(alice), ZoneKind::Hand))
            .unwrap_or_else(|| panic!("missing alice hand view"));
        let alice_library = view
            .zone(ZoneId::new(Some(alice), ZoneKind::Library))
            .unwrap_or_else(|| panic!("missing alice library view"));
        let bob_hand = view
            .zone(ZoneId::new(Some(bob), ZoneKind::Hand))
            .unwrap_or_else(|| panic!("missing bob hand view"));
        let bob_library = view
            .zone(ZoneId::new(Some(bob), ZoneKind::Library))
            .unwrap_or_else(|| panic!("missing bob library view"));

        assert_eq!(alice_hand.objects().len(), OPENING_HAND_SIZE as usize);
        assert!(alice_hand
            .objects()
            .iter()
            .all(|object| object.known().is_some()));
        assert_eq!(alice_library.objects().len(), 1);
        assert!(alice_library
            .objects()
            .iter()
            .all(|object| object.is_hidden()));
        assert_eq!(bob_hand.objects().len(), OPENING_HAND_SIZE as usize);
        assert!(bob_hand.objects().iter().all(|object| object.is_hidden()));
        assert_eq!(bob_library.objects().len(), 1);
        assert!(bob_library
            .objects()
            .iter()
            .all(|object| object.is_hidden()));
    }

    #[test]
    fn state_based_action_table_tracks_cr_704_rows() {
        let table = state_based_action_table();

        assert_eq!(table.len(), 24);
        assert_eq!(table[0], StateBasedActionKind::PlayerZeroOrLessLife);
        assert_eq!(table[23], StateBasedActionKind::StartYourEnginesNoSpeed);
        assert!(table.contains(&StateBasedActionKind::CreatureLethalDamage));
        assert!(table.contains(&StateBasedActionKind::CreatureDeathtouchDamage));
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
    fn draw_cards_action_moves_library_cards_to_hand() {
        let mut state = GameState::new();
        let player = add_player_action(&mut state);
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        let hand = ZoneId::new(Some(player), ZoneKind::Hand);
        seed_library_cards(&mut state, player, 8_100, 4);
        let expected = state
            .zone(library)
            .unwrap_or_else(|| panic!("library zone missing"))
            .objects()[2..]
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        let cursor = state.event_cursor();

        assert_eq!(
            apply(&mut state, Action::DrawCards { player, count: 2 }),
            Outcome::Applied
        );
        let draw_events = state
            .events_since(cursor)
            .unwrap_or_else(|error| panic!("draw event replay failed: {error:?}"))
            .iter()
            .filter_map(|record| match record.event() {
                GameEvent::CardDrawn {
                    player: event_player,
                    object,
                } => Some((event_player, object)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            draw_events,
            vec![(player, expected[0]), (player, expected[1])]
        );
        assert_eq!(
            state
                .zone(hand)
                .unwrap_or_else(|| panic!("hand zone missing"))
                .objects(),
            expected.as_slice()
        );
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects()
                .len(),
            2
        );
        assert_eq!(
            state.validate_zone_conservation(),
            Ok(ZoneConservation { object_count: 4 })
        );
    }

    #[test]
    fn reveal_objects_emits_public_events_without_moving_cards() {
        let mut state = GameState::new();
        let player = add_player_action(&mut state);
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        seed_library_cards(&mut state, player, 8_125, 2);
        let objects = state
            .zone(library)
            .unwrap_or_else(|| panic!("library zone missing"))
            .objects()
            .to_vec();
        let cursor = state.event_cursor();

        assert_eq!(
            apply(
                &mut state,
                Action::RevealObjects {
                    objects: objects.clone(),
                },
            ),
            Outcome::Applied
        );
        let revealed = state
            .events_since(cursor)
            .unwrap_or_else(|error| panic!("reveal event replay failed: {error:?}"))
            .iter()
            .filter_map(|record| match record.event() {
                GameEvent::ObjectRevealed { object } => Some(object),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(revealed, objects);
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects(),
            revealed.as_slice()
        );
    }

    #[test]
    fn cast_or_copy_trigger_observes_stack_copies_without_open_matching() {
        let mut state = GameState::new();
        let controller = add_player_action(&mut state);
        let spell = state
            .create_object(
                CardId::new(8_140),
                controller,
                controller,
                ZoneId::new(Some(controller), ZoneKind::Hand),
            )
            .unwrap_or_else(|error| panic!("spell create failed: {error:?}"));
        state
            .set_base_object_characteristics(
                spell,
                BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_instant(),
                    ObjectColors::none(),
                ),
            )
            .unwrap_or_else(|error| panic!("spell characteristics failed: {error:?}"));
        let original = state.push_stack_entry(StackEntryRequest {
            controller,
            object: Some(spell),
            trigger: None,
            activated_ability: None,
            kind: StackObjectKind::InstantSpell,
            targets: Vec::new(),
            payment: None,
            copy_info: None,
            kicked: false,
            flashback: false,
            split_second: false,
        });
        let matching_types = ObjectTypes::none()
            .with_instant()
            .union(ObjectTypes::none().with_sorcery());
        let copy_trigger = state
            .register_triggered_ability(TriggerDefinition::new(
                controller,
                TriggerCondition::StackEntryAddedOrCopied {
                    controller: TriggerPlayerFilter::Controller,
                    required_types: ObjectTypes::none(),
                    required_any_types: matching_types,
                    forbidden_types: ObjectTypes::none(),
                },
            ))
            .unwrap_or_else(|error| panic!("copy trigger registration failed: {error:?}"));
        state
            .register_triggered_ability(TriggerDefinition::new(
                controller,
                TriggerCondition::StackEntryAdded {
                    controller: TriggerPlayerFilter::Controller,
                    required_types: ObjectTypes::none(),
                    required_any_types: matching_types,
                    forbidden_types: ObjectTypes::none(),
                },
            ))
            .unwrap_or_else(|error| panic!("cast trigger registration failed: {error:?}"));

        state
            .copy_stack_entry(controller, original)
            .unwrap_or_else(|error| panic!("stack copy failed: {error:?}"));

        assert_eq!(state.pending_triggers().len(), 1);
        assert_eq!(state.pending_triggers()[0].trigger(), copy_trigger);
    }

    #[test]
    fn regeneration_shield_replaces_normal_but_not_prohibited_destruction() {
        let mut state = GameState::new();
        let controller = add_player_action(&mut state);
        let creature = battlefield_creature(
            &mut state,
            controller,
            8_145,
            3,
            3,
            CreatureKeywords::none(),
        );
        assert_eq!(
            apply(
                &mut state,
                Action::MarkDamageOnObject {
                    object: creature,
                    amount: 2,
                },
            ),
            Outcome::Applied
        );
        let shield = || {
            RestrictionDefinition::new(
                controller,
                RestrictionEffect::RegenerationShield { object: creature },
            )
            .with_duration(ContinuousEffectDuration::UntilEndOfTurn)
        };
        assert!(matches!(
            apply(
                &mut state,
                Action::RegisterRestriction {
                    definition: shield(),
                },
            ),
            Outcome::RestrictionRegistered(_)
        ));

        assert_eq!(
            apply(&mut state, Action::DestroyPermanent { object: creature }),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(creature),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        let record = state
            .object(creature)
            .unwrap_or_else(|| panic!("regenerated creature missing"));
        assert!(record.tapped());
        assert_eq!(record.damage_marked(), 0);
        assert_eq!(state.restrictions().count(), 0);

        assert!(matches!(
            apply(
                &mut state,
                Action::RegisterRestriction {
                    definition: shield(),
                },
            ),
            Outcome::RestrictionRegistered(_)
        ));
        assert_eq!(
            apply(
                &mut state,
                Action::DestroyPermanentWithoutRegeneration { object: creature },
            ),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(creature),
            Some(ZoneId::new(Some(controller), ZoneKind::Graveyard))
        );
        assert_eq!(state.restrictions().count(), 0);
    }

    #[test]
    fn put_object_on_top_of_library_reorders_without_changing_membership() {
        let mut state = GameState::new();
        let player = add_player_action(&mut state);
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        seed_library_cards(&mut state, player, 8_150, 4);
        let selected = state
            .zone(library)
            .unwrap_or_else(|| panic!("library zone missing"))
            .objects()[1];

        assert_eq!(
            apply(
                &mut state,
                Action::PutObjectOnTopOfLibrary {
                    player,
                    object: selected,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            state
                .zone(library)
                .unwrap_or_else(|| panic!("library zone missing"))
                .objects()
                .last(),
            Some(&selected)
        );
        assert_eq!(
            state.validate_zone_conservation(),
            Ok(ZoneConservation { object_count: 4 })
        );
    }

    #[test]
    fn shuffle_library_action_is_seeded_and_deterministic() {
        let mut left = GameState::new();
        let player = add_player_action(&mut left);
        seed_library_cards(&mut left, player, 8_200, 12);
        assert_eq!(
            apply(&mut left, Action::SetSeed { seed: 0x5eed }),
            Outcome::Applied
        );
        let mut right = left.clone();

        assert_eq!(
            apply(&mut left, Action::ShuffleLibrary { player }),
            Outcome::Applied
        );
        assert_eq!(
            apply(&mut right, Action::ShuffleLibrary { player }),
            Outcome::Applied
        );
        assert_eq!(left.canonical_bytes(), right.canonical_bytes());
        assert_eq!(left.deterministic_hash(), right.deterministic_hash());
        assert_eq!(
            left.validate_zone_conservation(),
            Ok(ZoneConservation { object_count: 12 })
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
        left.move_object(left_object, left_battlefield)
            .unwrap_or_else(|error| panic!("unexpected move error: {error:?}"));

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
    fn insufficient_colored_mana_has_no_payment_plans() {
        let available = ManaPool::new(0, 0, 0, 0, 0, 5);
        let cost = ManaCost::new(0, 0, 0, 1, 0, 1);

        let plans = enumerate_payment_plans(available, cost)
            .unwrap_or_else(|error| panic!("unexpected payment error: {error:?}"));

        assert!(plans.plans().is_empty());
        assert!(!plans.truncated());
    }

    #[test]
    fn generic_cost_uses_colorless_before_colored_mana() {
        let available = ManaPool::new(1, 1, 0, 1, 0, 2);
        let cost = ManaCost::new(0, 0, 0, 1, 0, 2);

        let plans = enumerate_payment_plans(available, cost)
            .unwrap_or_else(|error| panic!("unexpected payment error: {error:?}"));
        let best = plans
            .best()
            .unwrap_or_else(|| panic!("missing best payment plan"));

        assert_eq!(best.paid(), ManaPool::new(0, 0, 0, 1, 0, 2));
        assert_eq!(best.generic_paid(), ManaPool::new(0, 0, 0, 0, 0, 2));
        assert_eq!(best.waste_score(), 0);
        assert!(plans
            .plans()
            .windows(2)
            .all(|window| window[0].waste_score() <= window[1].waste_score()));
    }

    #[test]
    fn x_cost_is_added_to_generic_requirement() {
        let available = ManaPool::new(0, 0, 0, 1, 0, 4);
        let cost = ManaCost::new(0, 0, 0, 1, 0, 1).with_x(1, 3);

        let best = auto_payment_plan(available, cost)
            .unwrap_or_else(|error| panic!("unexpected payment error: {error:?}"))
            .unwrap_or_else(|| panic!("missing best payment plan"));

        assert_eq!(best.x_value(), 3);
        assert_eq!(best.generic_required(), 4);
        assert_eq!(best.paid(), available);
    }

    #[test]
    fn x_cost_overflow_is_reported() {
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0).with_x(u32::MAX, 2);

        assert_eq!(
            enumerate_payment_plans(ManaPool::empty(), cost),
            Err(PaymentError::ManaValueOverflow)
        );
    }

    #[test]
    fn explicit_payment_plan_is_validated_and_applied() {
        let mut state = GameState::new();
        let player = state.add_player();
        let available = ManaPool::new(0, 1, 0, 1, 0, 2);
        let cost = ManaCost::new(0, 1, 0, 0, 0, 2);
        state
            .add_mana_to_pool(player, available)
            .unwrap_or_else(|error| panic!("unexpected add mana error: {error:?}"));
        let plan = validate_payment_plan(available, cost, ManaPool::new(0, 1, 0, 1, 0, 1))
            .unwrap_or_else(|error| panic!("unexpected plan validation error: {error:?}"));

        state
            .pay_mana(player, cost, plan)
            .unwrap_or_else(|error| panic!("unexpected payment error: {error:?}"));

        assert_eq!(
            state
                .mana_pool(player)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaPool::new(0, 0, 0, 0, 0, 1)
        );
    }

    #[test]
    fn invalid_explicit_payment_is_rejected() {
        let available = ManaPool::new(0, 0, 0, 1, 0, 2);
        let cost = ManaCost::new(0, 0, 0, 1, 0, 2);

        assert_eq!(
            validate_payment_plan(available, cost, ManaPool::new(0, 0, 0, 1, 0, 1)),
            Err(PaymentError::InvalidPaymentPlan)
        );
    }

    #[test]
    fn payment_enumeration_caps_at_sixty_four_distinct_plans() {
        let available = ManaPool::new(20, 20, 20, 20, 20, 0);
        let cost = ManaCost::new(0, 0, 0, 0, 0, 10);

        let plans = enumerate_payment_plans(available, cost)
            .unwrap_or_else(|error| panic!("unexpected payment error: {error:?}"));

        assert_eq!(plans.plans().len(), PAYMENT_PLAN_LIMIT);
        assert!(plans.truncated());
        assert!(plans
            .plans()
            .windows(2)
            .all(|window| window[0].waste_score() <= window[1].waste_score()));
    }

    #[test]
    fn auto_tap_prefers_exact_sources_with_minimal_waste() {
        let mut state = GameState::new();
        let player = state.add_player();
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let red_source = state
            .create_object(CardId::new(70), player, player, battlefield)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let colorless_source = state
            .create_object(CardId::new(71), player, player, battlefield)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let green_source = state
            .create_object(CardId::new(72), player, player, battlefield)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let sources = [
            ManaSource::new(green_source, ManaPool::of(ManaKind::Green, 1)),
            ManaSource::new(colorless_source, ManaPool::of(ManaKind::Colorless, 1)),
            ManaSource::new(red_source, ManaPool::of(ManaKind::Red, 1)),
        ];

        let plans = enumerate_auto_tap_payment_plans(&sources, ManaCost::new(0, 0, 0, 1, 0, 1))
            .unwrap_or_else(|error| panic!("unexpected auto-tap error: {error:?}"));
        let best = plans
            .best()
            .unwrap_or_else(|| panic!("missing best auto-tap plan"));
        let tapped: Vec<_> = best.taps().iter().map(|tap| tap.source()).collect();

        assert_eq!(tapped, vec![red_source, colorless_source]);
        assert_eq!(best.total_waste_score(), 0);
        assert_eq!(best.unspent(), ManaPool::empty());
    }

    #[test]
    fn auto_tap_keeps_equivalent_sources_distinct() {
        let mut state = GameState::new();
        let player = state.add_player();
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let first = state
            .create_object(CardId::new(80), player, player, battlefield)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let second = state
            .create_object(CardId::new(81), player, player, battlefield)
            .unwrap_or_else(|error| panic!("unexpected create error: {error:?}"));
        let sources = [
            ManaSource::new(first, ManaPool::of(ManaKind::Red, 1)),
            ManaSource::new(second, ManaPool::of(ManaKind::Red, 1)),
        ];

        let plans = enumerate_auto_tap_payment_plans(&sources, ManaCost::new(0, 0, 0, 0, 0, 1))
            .unwrap_or_else(|error| panic!("unexpected auto-tap error: {error:?}"));
        let one_tap_sources: Vec<_> = plans
            .plans()
            .iter()
            .filter(|plan| plan.taps().len() == 1)
            .map(|plan| plan.taps()[0].source())
            .collect();

        assert_eq!(one_tap_sources, vec![first, second]);
    }

    #[test]
    fn mana_pool_changes_state_hash_and_matches_streaming_hash() {
        let mut state = GameState::new();
        let player = state.add_player();
        let before = state.deterministic_hash();

        state
            .add_mana_to_pool(player, ManaPool::of(ManaKind::Green, 1))
            .unwrap_or_else(|error| panic!("unexpected add mana error: {error:?}"));

        assert_ne!(state.deterministic_hash(), before);
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn mana_pool_empties_when_step_ends() {
        let mut state = GameState::new();
        let active = state.add_player();
        ensure_library_card(&mut state, active);
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected upkeep advance error: {error:?}"));
        state
            .add_mana_to_pool(active, ManaPool::of(ManaKind::Red, 1))
            .unwrap_or_else(|error| panic!("unexpected add mana error: {error:?}"));

        assert_eq!(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaPool::of(ManaKind::Red, 1)
        );

        state
            .pass_priority(active)
            .unwrap_or_else(|error| panic!("unexpected pass error: {error:?}"));

        assert_eq!(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaPool::empty()
        );
    }

    #[test]
    fn successful_instant_cast_pays_cost_and_records_target_snapshot() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let spell = state
            .create_object(CardId::new(90), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let target = state
            .create_object(CardId::new(91), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected target create error: {error:?}"));
        start_upkeep(&mut state, active);
        let cost = ManaCost::new(0, 0, 0, 1, 0, 0);
        state
            .add_mana_to_pool(active, ManaPool::of(ManaKind::Red, 1))
            .unwrap_or_else(|error| panic!("unexpected add mana error: {error:?}"));
        let payment = validate_payment_plan(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            cost,
            ManaPool::of(ManaKind::Red, 1),
        )
        .unwrap_or_else(|error| panic!("unexpected payment validation error: {error:?}"));
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            payment,
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::Permanent)],
            vec![TargetChoice::Object(target)],
        );

        let entry = state
            .cast_spell(active, spell, request)
            .unwrap_or_else(|error| panic!("unexpected cast error: {error:?}"));

        assert_eq!(
            state.object_zone(spell),
            Some(ZoneId::new(None, ZoneKind::Stack))
        );
        assert_eq!(state.priority_player(), Some(active));
        assert_eq!(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaPool::empty()
        );
        let stack_entry = state
            .stack_top()
            .unwrap_or_else(|| panic!("missing stack entry"));
        assert_eq!(stack_entry.id(), entry);
        assert_eq!(stack_entry.targets().len(), 1);
        assert_eq!(
            stack_entry.targets()[0].choice(),
            TargetChoice::Object(target)
        );
        assert_eq!(stack_entry.targets()[0].original_zone(), Some(battlefield));
        assert_eq!(stack_entry.payment(), Some(payment));
    }

    #[test]
    fn split_second_blocks_spells_and_non_mana_abilities_but_allows_mana_abilities() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let active_hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let responder_hand = ZoneId::new(Some(responder), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let split_spell = state
            .create_object(CardId::new(9_210), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected split spell create error: {error:?}"));
        let follow_up_spell = state
            .create_object(CardId::new(9_211), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected follow-up create error: {error:?}"));
        let blocked_spell = state
            .create_object(CardId::new(9_212), responder, responder, responder_hand)
            .unwrap_or_else(|error| panic!("unexpected blocked spell create error: {error:?}"));
        let non_mana_source = state
            .create_object(CardId::new(9_213), responder, responder, battlefield)
            .unwrap_or_else(|error| panic!("unexpected non-mana source error: {error:?}"));
        let mana_source = state
            .create_object(CardId::new(9_214), responder, responder, battlefield)
            .unwrap_or_else(|error| panic!("unexpected mana source error: {error:?}"));
        let zero = ManaCost::new(0, 0, 0, 0, 0, 0);
        let non_mana_ability = state
            .register_activated_ability(ActivatedAbilityDefinition::new(
                responder,
                Some(non_mana_source),
                ActivationTiming::Instant,
                ActivationCost::new(zero),
                ActivatedAbilityEffect::GainLife {
                    player: AbilityPlayer::Controller,
                    amount: 1,
                },
            ))
            .unwrap_or_else(|error| panic!("unexpected non-mana ability error: {error:?}"));
        let mana_ability = state
            .register_activated_ability(
                ActivatedAbilityDefinition::new(
                    responder,
                    Some(mana_source),
                    ActivationTiming::Instant,
                    ActivationCost::new(zero).with_tap_source(),
                    ActivatedAbilityEffect::AddMana {
                        player: AbilityPlayer::Controller,
                        mana: ManaPool::of(ManaKind::Green, 1),
                    },
                )
                .as_mana_ability(),
            )
            .unwrap_or_else(|error| panic!("unexpected mana ability error: {error:?}"));
        start_upkeep(&mut state, active);

        let split_entry = state
            .cast_spell(
                active,
                split_spell,
                CastSpellRequest::new(
                    StackObjectKind::InstantSpell,
                    SpellTiming::Instant,
                    zero,
                    zero_payment(zero),
                )
                .with_split_second(),
            )
            .unwrap_or_else(|error| panic!("unexpected split-second cast error: {error:?}"));
        assert!(state.stack_top().is_some_and(|entry| entry.split_second()));
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::PassedTo(responder))
        );
        let ordinary_request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            zero,
            zero_payment(zero),
        );
        assert_eq!(
            state.cast_spell(responder, blocked_spell, ordinary_request.clone()),
            Err(StateError::SplitSecondActionForbidden)
        );
        assert_eq!(
            state.activate_ability(responder, non_mana_ability, zero_payment(zero)),
            Err(StateError::SplitSecondActionForbidden)
        );
        assert_eq!(
            state.activate_ability(responder, mana_ability, zero_payment(zero)),
            Ok(None)
        );
        assert_eq!(
            state.mana_pool(responder),
            Ok(ManaPool::of(ManaKind::Green, 1))
        );
        assert_eq!(
            state.pass_priority(responder),
            Ok(PriorityOutcome::Resolved(split_entry))
        );
        assert!(state.resolution_log()[0].split_second());
        state
            .cast_spell(active, follow_up_spell, ordinary_request)
            .unwrap_or_else(|error| panic!("unexpected post-resolution cast error: {error:?}"));
    }

    #[test]
    fn stack_entry_target_is_captured_and_revalidated() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let active_hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let responder_hand = ZoneId::new(Some(responder), ZoneKind::Hand);
        let target_spell = state
            .create_object(CardId::new(9_200), active, active, active_hand)
            .unwrap_or_else(|error| panic!("unexpected target spell create error: {error:?}"));
        let counter_spell = state
            .create_object(CardId::new(9_201), responder, responder, responder_hand)
            .unwrap_or_else(|error| panic!("unexpected counter spell create error: {error:?}"));
        start_upkeep(&mut state, active);
        let target_entry = state
            .put_spell_on_stack(active, target_spell, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected target stack error: {error:?}"));
        state
            .pass_priority(active)
            .unwrap_or_else(|error| panic!("unexpected priority pass error: {error:?}"));
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            zero_payment(cost),
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::StackEntry)],
            vec![TargetChoice::StackEntry(target_entry)],
        );

        let counter_entry = state
            .cast_spell(responder, counter_spell, request)
            .unwrap_or_else(|error| panic!("unexpected counter cast error: {error:?}"));
        let stack_entry = state
            .stack_top()
            .unwrap_or_else(|| panic!("missing counter stack entry"));
        assert_eq!(stack_entry.id(), counter_entry);
        assert_eq!(
            stack_entry.targets()[0].choice(),
            TargetChoice::StackEntry(target_entry)
        );
        assert_eq!(stack_entry.targets()[0].original_zone(), None);

        state
            .counter_stack_entry(target_entry)
            .unwrap_or_else(|error| panic!("unexpected target counter error: {error:?}"));
        assert!(!state.is_target_still_legal(
            responder,
            Some(counter_spell),
            stack_entry.targets()[0],
        ));
    }

    #[test]
    fn illegal_target_during_announcement_leaves_state_unchanged() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        let spell = state
            .create_object(CardId::new(92), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let target = state
            .create_object(CardId::new(93), active, active, graveyard)
            .unwrap_or_else(|error| panic!("unexpected target create error: {error:?}"));
        start_upkeep(&mut state, active);
        let before = state.canonical_bytes();
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            zero_payment(cost),
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::Permanent)],
            vec![TargetChoice::Object(target)],
        );

        assert_eq!(
            state.cast_spell(active, spell, request),
            Err(StateError::IllegalTarget {
                index: 0,
                target: TargetChoice::Object(target)
            })
        );
        assert_eq!(state.canonical_bytes(), before);
    }

    #[test]
    fn invalid_payment_during_cast_leaves_state_unchanged() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let spell = state
            .create_object(CardId::new(94), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let target = state
            .create_object(CardId::new(95), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected target create error: {error:?}"));
        start_upkeep(&mut state, active);
        state
            .add_mana_to_pool(active, ManaPool::of(ManaKind::Red, 1))
            .unwrap_or_else(|error| panic!("unexpected add mana error: {error:?}"));
        let before = state.canonical_bytes();
        let cost = ManaCost::new(0, 0, 0, 1, 0, 0);
        let bad_payment = zero_payment(ManaCost::new(0, 0, 0, 0, 0, 0));
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            bad_payment,
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::Permanent)],
            vec![TargetChoice::Object(target)],
        );

        assert_eq!(
            state.cast_spell(active, spell, request),
            Err(StateError::InvalidPaymentPlan)
        );
        assert_eq!(state.canonical_bytes(), before);
    }

    #[test]
    fn play_land_enforces_timing_type_and_once_per_turn_limit() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let first = state
            .create_object(CardId::new(9_401), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected first land create error: {error:?}"));
        let second = state
            .create_object(CardId::new(9_402), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected second land create error: {error:?}"));
        let spell = state
            .create_object(CardId::new(9_403), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        for land in [first, second] {
            assert_eq!(
                apply(
                    &mut state,
                    Action::SetBaseObjectCharacteristics {
                        object: land,
                        base: BaseObjectCharacteristics::new(
                            ObjectTypes::none().with_land(),
                            ObjectColors::none(),
                        ),
                    },
                ),
                Outcome::Applied
            );
        }
        start_upkeep(&mut state, active);
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: first,
                },
            ),
            Outcome::Failed(StateError::InvalidLandPlayTiming)
        );
        while state.current_step() != Some(Step::PrecombatMain) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: spell,
                },
            ),
            Outcome::Failed(StateError::ObjectNotPlayableAsLand(spell))
        );
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: first,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.object_zone(first), Some(battlefield));
        assert_eq!(state.players()[active.index()].lands_played_this_turn(), 1);
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: second,
                },
            ),
            Outcome::Failed(StateError::LandPlayLimitReached(active))
        );
        assert_eq!(state.object_zone(second), Some(hand));
        assert_eq!(
            state
                .validate_zone_conservation()
                .map(|value| value.object_count()),
            Ok(4)
        );
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );

        assert_eq!(
            apply(
                &mut state,
                Action::SetObjectTapped {
                    object: first,
                    tapped: true,
                },
            ),
            Outcome::Applied
        );
        ensure_library_card(&mut state, active);
        while state.turn_number() < 2 || state.current_step() != Some(Step::PrecombatMain) {
            if let Some(player) = state.priority_player() {
                assert!(matches!(
                    apply(&mut state, Action::PassPriority { player }),
                    Outcome::Priority(_)
                ));
            } else {
                assert!(matches!(
                    apply(&mut state, Action::AdvanceStep),
                    Outcome::StepAdvanced(_)
                ));
            }
        }
        assert!(state.object(first).is_some_and(|record| !record.tapped()));
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: second,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.object_zone(second), Some(battlefield));
        assert_eq!(state.players()[active.index()].lands_played_this_turn(), 1);
    }

    #[test]
    fn land_with_enters_tapped_characteristic_enters_tapped_atomically() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let land = state
            .create_object(CardId::new(9_404), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected land create error: {error:?}"));
        assert_eq!(
            apply(
                &mut state,
                Action::SetBaseObjectCharacteristics {
                    object: land,
                    base: BaseObjectCharacteristics::new(
                        ObjectTypes::none().with_land(),
                        ObjectColors::none(),
                    )
                    .with_enters_tapped(),
                },
            ),
            Outcome::Applied
        );
        start_upkeep(&mut state, active);
        while state.current_step() != Some(Step::PrecombatMain) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }
        assert_eq!(
            apply(
                &mut state,
                Action::PlayLand {
                    player: active,
                    object: land,
                },
            ),
            Outcome::Applied
        );
        assert!(state.object(land).is_some_and(|record| record.tapped()));
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn sorcery_timing_rejects_non_main_phase_and_nonempty_stack() {
        let mut state = GameState::new();
        let active = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let first = state
            .create_object(CardId::new(96), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let second = state
            .create_object(CardId::new(97), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        start_upkeep(&mut state, active);
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let sorcery_request = CastSpellRequest::new(
            StackObjectKind::SorcerySpell,
            SpellTiming::Sorcery,
            cost,
            zero_payment(cost),
        );

        assert_eq!(
            state.cast_spell(active, first, sorcery_request.clone()),
            Err(StateError::InvalidSpellTiming)
        );

        while state.current_step() != Some(Step::PrecombatMain) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected advance error: {error:?}"));
        }
        state
            .cast_spell(active, first, sorcery_request)
            .unwrap_or_else(|error| panic!("unexpected main phase cast error: {error:?}"));
        let second_request = CastSpellRequest::new(
            StackObjectKind::SorcerySpell,
            SpellTiming::Sorcery,
            cost,
            zero_payment(cost),
        );
        assert_eq!(
            state.cast_spell(active, second, second_request),
            Err(StateError::InvalidSpellTiming)
        );
    }

    #[test]
    fn all_targets_illegal_before_resolution_counter_spell_by_rules() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        let spell = state
            .create_object(CardId::new(98), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let target = state
            .create_object(CardId::new(99), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected target create error: {error:?}"));
        start_upkeep(&mut state, active);
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            zero_payment(cost),
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::Permanent)],
            vec![TargetChoice::Object(target)],
        );
        let entry = state
            .cast_spell(active, spell, request)
            .unwrap_or_else(|error| panic!("unexpected cast error: {error:?}"));

        state
            .move_object(target, graveyard)
            .unwrap_or_else(|error| panic!("unexpected target move error: {error:?}"));
        pass_round(&mut state, active, responder, entry);

        assert_eq!(state.object_zone(spell), Some(graveyard));
        assert_eq!(
            state.resolution_log()[0].outcome(),
            ResolutionOutcome::CounteredOnResolution
        );
        assert_eq!(state.resolution_log()[0].legal_targets(), &[false]);
        assert_eq!(state.priority_player(), Some(active));
    }

    #[test]
    fn one_legal_target_allows_spell_to_resolve() {
        let mut state = GameState::new();
        let active = state.add_player();
        let responder = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        let spell = state
            .create_object(CardId::new(100), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected spell create error: {error:?}"));
        let first_target = state
            .create_object(CardId::new(101), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected first target create error: {error:?}"));
        let second_target = state
            .create_object(CardId::new(102), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected second target create error: {error:?}"));
        start_upkeep(&mut state, active);
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            zero_payment(cost),
        )
        .with_targets(
            vec![
                TargetRequirement::new(TargetKind::Permanent),
                TargetRequirement::new(TargetKind::Permanent),
            ],
            vec![
                TargetChoice::Object(first_target),
                TargetChoice::Object(second_target),
            ],
        );
        let entry = state
            .cast_spell(active, spell, request)
            .unwrap_or_else(|error| panic!("unexpected cast error: {error:?}"));

        state
            .move_object(first_target, graveyard)
            .unwrap_or_else(|error| panic!("unexpected target move error: {error:?}"));
        pass_round(&mut state, active, responder, entry);

        assert_eq!(state.object_zone(spell), Some(graveyard));
        assert_eq!(
            state.resolution_log()[0].outcome(),
            ResolutionOutcome::Resolved
        );
        assert_eq!(state.resolution_log()[0].legal_targets(), &[false, true]);
    }

    #[test]
    fn target_choices_affect_canonical_hash() {
        let mut left = GameState::new();
        let left_active = left.add_player();
        let left_hand = ZoneId::new(Some(left_active), ZoneKind::Hand);
        let left_battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let left_spell = left
            .create_object(CardId::new(103), left_active, left_active, left_hand)
            .unwrap_or_else(|error| panic!("unexpected left spell create error: {error:?}"));
        let left_first = left
            .create_object(CardId::new(104), left_active, left_active, left_battlefield)
            .unwrap_or_else(|error| panic!("unexpected left target create error: {error:?}"));
        left.create_object(CardId::new(105), left_active, left_active, left_battlefield)
            .unwrap_or_else(|error| {
                panic!("unexpected left second target create error: {error:?}")
            });
        start_upkeep(&mut left, left_active);

        let mut right = GameState::new();
        let right_active = right.add_player();
        let right_hand = ZoneId::new(Some(right_active), ZoneKind::Hand);
        let right_battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let right_spell = right
            .create_object(CardId::new(103), right_active, right_active, right_hand)
            .unwrap_or_else(|error| panic!("unexpected right spell create error: {error:?}"));
        right
            .create_object(
                CardId::new(104),
                right_active,
                right_active,
                right_battlefield,
            )
            .unwrap_or_else(|error| panic!("unexpected right target create error: {error:?}"));
        let right_second = right
            .create_object(
                CardId::new(105),
                right_active,
                right_active,
                right_battlefield,
            )
            .unwrap_or_else(|error| {
                panic!("unexpected right second target create error: {error:?}")
            });
        start_upkeep(&mut right, right_active);

        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        cast_zero_cost_target_spell(&mut left, left_active, left_spell, left_first);
        cast_zero_cost_target_spell(&mut right, right_active, right_spell, right_second);

        assert_ne!(left.deterministic_hash(), right.deterministic_hash());
        assert_eq!(
            left.deterministic_hash(),
            left.deterministic_hash_streaming()
        );
        assert_eq!(
            right.deterministic_hash(),
            right.deterministic_hash_streaming()
        );
        assert_eq!(cost.generic_total(), Ok(0));
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
        ensure_library_card(&mut state, active);

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
        ensure_library_card(&mut state, active);

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
    fn declare_attackers_taps_nonvigilance_and_preserves_vigilance() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let first = battlefield_creature(&mut state, active, 200, 2, 2, CreatureKeywords::none());
        let vigilant = battlefield_creature(
            &mut state,
            active,
            201,
            3,
            3,
            CreatureKeywords::none().with_vigilance(),
        );
        start_declare_attackers(&mut state, active);
        let before = state.deterministic_hash();

        state
            .declare_attackers(
                active,
                &[
                    AttackDeclaration::new(first, defender),
                    AttackDeclaration::new(vigilant, defender),
                ],
            )
            .unwrap_or_else(|error| panic!("unexpected attack error: {error:?}"));

        assert!(state
            .objects()
            .get(first)
            .unwrap_or_else(|| panic!("missing first attacker"))
            .tapped());
        assert!(!state
            .objects()
            .get(vigilant)
            .unwrap_or_else(|| panic!("missing vigilant attacker"))
            .tapped());
        assert_eq!(state.combat_state().attackers().len(), 2);
        assert_ne!(state.deterministic_hash(), before);
        assert_eq!(
            state.deterministic_hash(),
            state.deterministic_hash_streaming()
        );
    }

    #[test]
    fn illegal_attack_declarations_leave_state_unchanged() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let tapped = battlefield_creature(&mut state, active, 202, 2, 2, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .set_object_tapped(tapped, true)
            .unwrap_or_else(|error| panic!("unexpected tap error: {error:?}"));
        let before = state.canonical_bytes();

        assert_eq!(
            state.declare_attackers(active, &[AttackDeclaration::new(tapped, defender)]),
            Err(StateError::CreatureTapped(tapped))
        );
        assert_eq!(state.canonical_bytes(), before);

        let fresh = state
            .create_object(
                CardId::new(203),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected fresh create error: {error:?}"));
        state
            .set_base_creature_characteristics(fresh, BaseCreatureCharacteristics::new(2, 2))
            .unwrap_or_else(|error| panic!("unexpected fresh creature error: {error:?}"));
        let before = state.canonical_bytes();
        assert_eq!(
            state.declare_attackers(active, &[AttackDeclaration::new(fresh, defender)]),
            Err(StateError::SummoningSick(fresh))
        );
        assert_eq!(state.canonical_bytes(), before);
    }

    #[test]
    fn declare_no_attackers_skips_blockers_and_damage() {
        let mut state = GameState::new();
        let active = state.add_player();
        start_declare_attackers(&mut state, active);

        state
            .declare_attackers(active, &[])
            .unwrap_or_else(|error| panic!("unexpected empty attack error: {error:?}"));

        assert_eq!(
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected no-attack advance error: {error:?}")),
            Step::EndOfCombat
        );
    }

    #[test]
    fn flying_reach_and_menace_block_legality_is_enforced() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let flyer = battlefield_creature(
            &mut state,
            active,
            204,
            2,
            2,
            CreatureKeywords::none().with_flying(),
        );
        let ground =
            battlefield_creature(&mut state, defender, 205, 2, 2, CreatureKeywords::none());
        let reach = battlefield_creature(
            &mut state,
            defender,
            206,
            2,
            2,
            CreatureKeywords::none().with_reach(),
        );
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(flyer, defender)])
            .unwrap_or_else(|error| panic!("unexpected flying attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers step error: {error:?}"));
        let before = state.canonical_bytes();

        assert_eq!(
            state.declare_blockers(defender, &[BlockDeclaration::new(ground, flyer)]),
            Err(StateError::IllegalBlock {
                blocker: ground,
                attacker: flyer
            })
        );
        assert_eq!(state.canonical_bytes(), before);
        state
            .declare_blockers(defender, &[BlockDeclaration::new(reach, flyer)])
            .unwrap_or_else(|error| panic!("unexpected reach block error: {error:?}"));

        let mut menace_state = GameState::new();
        let menace_active = menace_state.add_player();
        let menace_defender = menace_state.add_player();
        let menace = battlefield_creature(
            &mut menace_state,
            menace_active,
            207,
            3,
            3,
            CreatureKeywords::none().with_menace(),
        );
        let first_blocker = battlefield_creature(
            &mut menace_state,
            menace_defender,
            208,
            1,
            1,
            CreatureKeywords::none(),
        );
        let second_blocker = battlefield_creature(
            &mut menace_state,
            menace_defender,
            209,
            1,
            1,
            CreatureKeywords::none(),
        );
        start_declare_attackers(&mut menace_state, menace_active);
        menace_state
            .declare_attackers(
                menace_active,
                &[AttackDeclaration::new(menace, menace_defender)],
            )
            .unwrap_or_else(|error| panic!("unexpected menace attack error: {error:?}"));
        menace_state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected menace blockers step error: {error:?}"));
        assert_eq!(
            menace_state.declare_blockers(
                menace_defender,
                &[BlockDeclaration::new(first_blocker, menace)]
            ),
            Err(StateError::IllegalBlock {
                blocker: first_blocker,
                attacker: menace
            })
        );
        menace_state
            .declare_blockers(
                menace_defender,
                &[
                    BlockDeclaration::new(first_blocker, menace),
                    BlockDeclaration::new(second_blocker, menace),
                ],
            )
            .unwrap_or_else(|error| panic!("unexpected two-blocker menace error: {error:?}"));
    }

    #[test]
    fn blocked_attacker_remains_blocked_after_blocker_leaves() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let attacker =
            battlefield_creature(&mut state, active, 210, 4, 4, CreatureKeywords::none());
        let blocker =
            battlefield_creature(&mut state, defender, 211, 1, 1, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(attacker, defender)])
            .unwrap_or_else(|error| panic!("unexpected attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, attacker)])
            .unwrap_or_else(|error| panic!("unexpected block error: {error:?}"));
        state
            .move_object(blocker, ZoneId::new(Some(defender), ZoneKind::Graveyard))
            .unwrap_or_else(|error| panic!("unexpected blocker move error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));

        assert!(state.combat_state().attackers()[0].blocked());
        assert!(state.assign_combat_damage(&[]).is_ok());
        assert_eq!(state.players()[defender.index()].life(), 20);
    }

    #[test]
    fn double_strike_creates_two_damage_steps() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let striker = battlefield_creature(
            &mut state,
            active,
            212,
            2,
            2,
            CreatureKeywords::none().with_double_strike(),
        );
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(striker, defender)])
            .unwrap_or_else(|error| panic!("unexpected double-strike attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[])
            .unwrap_or_else(|error| panic!("unexpected empty block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected first damage advance error: {error:?}"));
        assert_eq!(
            state.combat_state().damage_step(),
            Some(CombatDamageStepKind::FirstStrike)
        );
        state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                striker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Player(defender),
                    2,
                )],
            )])
            .unwrap_or_else(|error| panic!("unexpected first damage error: {error:?}"));
        assert_eq!(
            state.advance_step().unwrap_or_else(|error| panic!(
                "unexpected regular damage advance error: {error:?}"
            )),
            Step::CombatDamage
        );
        assert_eq!(
            state.combat_state().damage_step(),
            Some(CombatDamageStepKind::Regular)
        );
        state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                striker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Player(defender),
                    2,
                )],
            )])
            .unwrap_or_else(|error| panic!("unexpected regular damage error: {error:?}"));

        assert_eq!(state.players()[defender.index()].life(), 16);
    }

    #[test]
    fn trample_with_deathtouch_allows_one_damage_to_each_blocker() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let trampler = battlefield_creature(
            &mut state,
            active,
            213,
            5,
            5,
            CreatureKeywords::none().with_trample().with_deathtouch(),
        );
        let blocker =
            battlefield_creature(&mut state, defender, 214, 3, 3, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(trampler, defender)])
            .unwrap_or_else(|error| panic!("unexpected trample attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, trampler)])
            .unwrap_or_else(|error| panic!("unexpected trample block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));

        state
            .assign_combat_damage(&[
                CombatDamageAssignmentRequest::new(
                    trampler,
                    vec![
                        CombatDamageAssignment::new(CombatDamageTarget::Object(blocker), 1),
                        CombatDamageAssignment::new(CombatDamageTarget::Player(defender), 4),
                    ],
                ),
                CombatDamageAssignmentRequest::new(
                    blocker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(trampler),
                        3,
                    )],
                ),
            ])
            .unwrap_or_else(|error| panic!("unexpected trample damage error: {error:?}"));

        assert_eq!(
            state.object_zone(blocker),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
        assert_eq!(
            state
                .objects()
                .get(blocker)
                .unwrap_or_else(|| panic!("missing blocker"))
                .damage_marked(),
            0
        );
        assert_eq!(state.players()[defender.index()].life(), 16);
    }

    #[test]
    fn trample_without_lethal_blocker_damage_is_rejected() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let trampler = battlefield_creature(
            &mut state,
            active,
            215,
            5,
            5,
            CreatureKeywords::none().with_trample(),
        );
        let blocker =
            battlefield_creature(&mut state, defender, 216, 3, 3, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(trampler, defender)])
            .unwrap_or_else(|error| panic!("unexpected trample attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, trampler)])
            .unwrap_or_else(|error| panic!("unexpected trample block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));
        let before = state.canonical_bytes();

        assert_eq!(
            state.assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                trampler,
                vec![
                    CombatDamageAssignment::new(CombatDamageTarget::Object(blocker), 1),
                    CombatDamageAssignment::new(CombatDamageTarget::Player(defender), 4),
                ],
            )]),
            Err(StateError::IllegalCombatDamageAssignment(trampler))
        );
        assert_eq!(state.canonical_bytes(), before);
    }

    #[test]
    fn double_block_damage_must_follow_blocker_order() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let attacker =
            battlefield_creature(&mut state, active, 217, 4, 4, CreatureKeywords::none());
        let first_blocker =
            battlefield_creature(&mut state, defender, 218, 0, 2, CreatureKeywords::none());
        let second_blocker =
            battlefield_creature(&mut state, defender, 219, 0, 2, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(attacker, defender)])
            .unwrap_or_else(|error| panic!("unexpected attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(
                defender,
                &[
                    BlockDeclaration::new(first_blocker, attacker),
                    BlockDeclaration::new(second_blocker, attacker),
                ],
            )
            .unwrap_or_else(|error| panic!("unexpected block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));
        let before = state.canonical_bytes();

        assert_eq!(
            state.assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                attacker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Object(second_blocker),
                    4,
                )],
            )]),
            Err(StateError::IllegalCombatDamageAssignment(attacker))
        );
        assert_eq!(state.canonical_bytes(), before);

        state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                attacker,
                vec![
                    CombatDamageAssignment::new(CombatDamageTarget::Object(first_blocker), 2),
                    CombatDamageAssignment::new(CombatDamageTarget::Object(second_blocker), 2),
                ],
            )])
            .unwrap_or_else(|error| panic!("unexpected ordered damage error: {error:?}"));

        assert_eq!(
            state.object_zone(first_blocker),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
        assert_eq!(
            state.object_zone(second_blocker),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
    }

    #[test]
    fn lifelink_combat_damage_gains_life_as_damage_is_dealt() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let lifelinker = battlefield_creature(
            &mut state,
            active,
            217,
            3,
            3,
            CreatureKeywords::none().with_lifelink(),
        );
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(lifelinker, defender)])
            .unwrap_or_else(|error| panic!("unexpected lifelink attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[])
            .unwrap_or_else(|error| panic!("unexpected empty block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));

        let records = state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                lifelinker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Player(defender),
                    3,
                )],
            )])
            .unwrap_or_else(|error| panic!("unexpected lifelink damage error: {error:?}"));

        assert_eq!(records.len(), 1);
        assert!(records[0].source_had_lifelink());
        assert_eq!(state.players()[active.index()].life(), 23);
        assert_eq!(state.players()[defender.index()].life(), 17);
    }

    #[test]
    fn life_total_sba_causes_loss_and_win_before_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let opponent = state.add_player();
        state
            .set_player_life(active, 0)
            .unwrap_or_else(|error| panic!("unexpected life set error: {error:?}"));

        start_upkeep(&mut state, active);

        assert!(state.players()[active.index()].lost());
        assert!(!state.players()[opponent.index()].lost());
        assert_eq!(state.game_outcome(), GameOutcome::Won(opponent));
        assert_eq!(state.priority_player(), None);
    }

    #[test]
    fn poison_sba_causes_loss() {
        let mut state = GameState::new();
        let active = state.add_player();
        let opponent = state.add_player();
        state
            .add_poison_counters(active, 10)
            .unwrap_or_else(|error| panic!("unexpected poison error: {error:?}"));

        let report = state
            .check_state_based_actions()
            .unwrap_or_else(|error| panic!("unexpected SBA error: {error:?}"));

        assert_eq!(report.players_lost(), 1);
        assert!(state.players()[active.index()].lost());
        assert_eq!(state.game_outcome(), GameOutcome::Won(opponent));
    }

    #[test]
    fn simultaneous_lethal_life_loss_is_a_draw() {
        let mut state = GameState::new();
        let active = state.add_player();
        let opponent = state.add_player();
        state
            .lose_life(active, 20)
            .unwrap_or_else(|error| panic!("unexpected active life loss error: {error:?}"));
        state
            .lose_life(opponent, 20)
            .unwrap_or_else(|error| panic!("unexpected opponent life loss error: {error:?}"));

        let report = state
            .check_state_based_actions()
            .unwrap_or_else(|error| panic!("unexpected SBA error: {error:?}"));

        assert_eq!(report.iterations(), 1);
        assert_eq!(report.players_lost(), 2);
        assert_eq!(state.game_outcome(), GameOutcome::Draw);
        assert_eq!(state.priority_player(), None);
    }

    #[test]
    fn empty_library_draw_step_loses_before_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let opponent = state.add_player();
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));

        while state.current_step() != Some(Step::Draw) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected draw walk error: {error:?}"));
        }

        assert_eq!(state.current_step(), Some(Step::Draw));
        assert!(state.players()[active.index()].lost());
        assert_eq!(state.game_outcome(), GameOutcome::Won(opponent));
        assert_eq!(state.priority_player(), None);
    }

    #[test]
    fn lifelink_is_applied_before_loss_state_based_actions() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let lifelinker = battlefield_creature(
            &mut state,
            active,
            220,
            3,
            3,
            CreatureKeywords::none().with_lifelink(),
        );
        state
            .set_player_life(active, 3)
            .unwrap_or_else(|error| panic!("unexpected life set error: {error:?}"));
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(lifelinker, defender)])
            .unwrap_or_else(|error| panic!("unexpected lifelink attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[])
            .unwrap_or_else(|error| panic!("unexpected empty block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));
        state
            .lose_life(active, 3)
            .unwrap_or_else(|error| panic!("unexpected pre-damage life loss error: {error:?}"));

        state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                lifelinker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Player(defender),
                    3,
                )],
            )])
            .unwrap_or_else(|error| panic!("unexpected lifelink damage error: {error:?}"));

        assert_eq!(state.players()[active.index()].life(), 3);
        assert!(!state.players()[active.index()].lost());
        assert_eq!(state.game_outcome(), GameOutcome::InProgress);
    }

    #[test]
    fn lethal_combat_damage_moves_creatures_to_owner_graveyards_before_priority() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let attacker =
            battlefield_creature(&mut state, active, 221, 2, 2, CreatureKeywords::none());
        let blocker =
            battlefield_creature(&mut state, defender, 222, 2, 2, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(attacker, defender)])
            .unwrap_or_else(|error| panic!("unexpected attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, attacker)])
            .unwrap_or_else(|error| panic!("unexpected block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));

        state
            .assign_combat_damage(&[
                CombatDamageAssignmentRequest::new(
                    attacker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(blocker),
                        2,
                    )],
                ),
                CombatDamageAssignmentRequest::new(
                    blocker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(attacker),
                        2,
                    )],
                ),
            ])
            .unwrap_or_else(|error| panic!("unexpected lethal combat damage error: {error:?}"));

        assert_eq!(
            state.object_zone(attacker),
            Some(ZoneId::new(Some(active), ZoneKind::Graveyard))
        );
        assert_eq!(
            state.object_zone(blocker),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
        assert_eq!(state.priority_player(), Some(active));
    }

    #[test]
    fn first_strike_lethal_damage_removes_blocker_before_regular_damage() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let striker = battlefield_creature(
            &mut state,
            active,
            223,
            2,
            2,
            CreatureKeywords::none().with_first_strike(),
        );
        let blocker =
            battlefield_creature(&mut state, defender, 224, 2, 2, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(striker, defender)])
            .unwrap_or_else(|error| panic!("unexpected first strike attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, striker)])
            .unwrap_or_else(|error| panic!("unexpected first strike block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected first damage advance error: {error:?}"));
        state
            .assign_combat_damage(&[CombatDamageAssignmentRequest::new(
                striker,
                vec![CombatDamageAssignment::new(
                    CombatDamageTarget::Object(blocker),
                    2,
                )],
            )])
            .unwrap_or_else(|error| panic!("unexpected first strike damage error: {error:?}"));

        assert_eq!(
            state.object_zone(blocker),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
        assert!(state.combat_state().attackers()[0].blocked());
        assert_eq!(
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected regular damage advance: {error:?}")),
            Step::CombatDamage
        );
        assert_eq!(
            state.combat_state().damage_step(),
            Some(CombatDamageStepKind::Regular)
        );
        state
            .assign_combat_damage(&[])
            .unwrap_or_else(|error| panic!("unexpected empty regular damage error: {error:?}"));
        assert_eq!(state.players()[defender.index()].life(), 20);
    }

    #[test]
    fn deathtouch_damage_does_not_persist_past_one_sba_check() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let deathtoucher = battlefield_creature(
            &mut state,
            active,
            225,
            1,
            1,
            CreatureKeywords::none().with_deathtouch(),
        );
        let large = battlefield_creature(&mut state, defender, 226, 5, 5, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(deathtoucher, defender)])
            .unwrap_or_else(|error| panic!("unexpected deathtouch attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(large, deathtoucher)])
            .unwrap_or_else(|error| panic!("unexpected deathtouch block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));
        state
            .assign_combat_damage(&[
                CombatDamageAssignmentRequest::new(
                    deathtoucher,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(large),
                        1,
                    )],
                ),
                CombatDamageAssignmentRequest::new(
                    large,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(deathtoucher),
                        5,
                    )],
                ),
            ])
            .unwrap_or_else(|error| panic!("unexpected deathtouch damage error: {error:?}"));

        assert_eq!(
            state.object_zone(large),
            Some(ZoneId::new(Some(defender), ZoneKind::Graveyard))
        );
        state
            .move_object(large, ZoneId::new(None, ZoneKind::Battlefield))
            .unwrap_or_else(|error| panic!("unexpected return to battlefield error: {error:?}"));
        let report = state
            .check_state_based_actions()
            .unwrap_or_else(|error| panic!("unexpected stale SBA error: {error:?}"));

        assert_eq!(report.actions_performed(), 0);
        assert_eq!(
            state.object_zone(large),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
    }

    #[test]
    fn zero_toughness_creature_goes_to_owner_graveyard() {
        let mut state = GameState::new();
        let player = state.add_player();
        let doomed = battlefield_creature(&mut state, player, 227, 0, 0, CreatureKeywords::none());

        let report = state
            .check_state_based_actions()
            .unwrap_or_else(|error| panic!("unexpected zero toughness SBA error: {error:?}"));

        assert_eq!(report.zero_toughness_creatures(), 1);
        assert_eq!(
            state.object_zone(doomed),
            Some(ZoneId::new(Some(player), ZoneKind::Graveyard))
        );
    }

    #[test]
    fn combat_damage_marks_persist_until_cleanup() {
        let mut state = GameState::new();
        let active = state.add_player();
        let defender = state.add_player();
        let attacker =
            battlefield_creature(&mut state, active, 218, 1, 3, CreatureKeywords::none());
        let blocker =
            battlefield_creature(&mut state, defender, 219, 1, 3, CreatureKeywords::none());
        start_declare_attackers(&mut state, active);
        state
            .declare_attackers(active, &[AttackDeclaration::new(attacker, defender)])
            .unwrap_or_else(|error| panic!("unexpected attack error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected blockers advance error: {error:?}"));
        state
            .declare_blockers(defender, &[BlockDeclaration::new(blocker, attacker)])
            .unwrap_or_else(|error| panic!("unexpected block error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected damage advance error: {error:?}"));
        state
            .assign_combat_damage(&[
                CombatDamageAssignmentRequest::new(
                    attacker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(blocker),
                        1,
                    )],
                ),
                CombatDamageAssignmentRequest::new(
                    blocker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Object(attacker),
                        1,
                    )],
                ),
            ])
            .unwrap_or_else(|error| panic!("unexpected creature damage error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected end combat advance error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected postcombat advance error: {error:?}"));
        assert_eq!(
            state
                .objects()
                .get(attacker)
                .unwrap_or_else(|| panic!("missing attacker"))
                .damage_marked(),
            1
        );
        assert_eq!(
            state
                .objects()
                .get(blocker)
                .unwrap_or_else(|| panic!("missing blocker"))
                .damage_marked(),
            1
        );

        while state.current_step() != Some(Step::Cleanup) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected cleanup walk error: {error:?}"));
        }
        assert_eq!(
            state
                .objects()
                .get(attacker)
                .unwrap_or_else(|| panic!("missing attacker"))
                .damage_marked(),
            0
        );
        assert_eq!(
            state
                .objects()
                .get(blocker)
                .unwrap_or_else(|| panic!("missing blocker"))
                .damage_marked(),
            0
        );
    }

    #[test]
    fn end_of_turn_durations_survive_end_step_and_expire_during_cleanup() {
        let mut state = GameState::new();
        let active = state.add_player();
        ensure_library_card(&mut state, active);
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
    fn temporary_hexproof_and_indestructible_apply_to_objects_and_expire() {
        let mut state = GameState::new();
        let active = state.add_player();
        let opponent = state.add_player();
        ensure_library_card(&mut state, active);
        let artifact = state
            .create_object(
                CardId::new(26_100),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected artifact create error: {error:?}"));
        state
            .set_base_object_characteristics(
                artifact,
                BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_artifact(),
                    ObjectColors::none(),
                ),
            )
            .unwrap_or_else(|error| panic!("unexpected artifact characteristics error: {error:?}"));
        let creature =
            battlefield_creature(&mut state, active, 26_101, 2, 2, CreatureKeywords::none());
        for definition in [
            RestrictionDefinition::new(
                active,
                RestrictionEffect::Targeting {
                    subject: TargetRestrictionSubject::Object(creature),
                    restriction: TargetRestriction::Hexproof,
                },
            ),
            RestrictionDefinition::new(
                active,
                RestrictionEffect::Indestructible { object: artifact },
            ),
            RestrictionDefinition::new(
                active,
                RestrictionEffect::Indestructible { object: creature },
            ),
        ] {
            state
                .register_restriction(
                    definition.with_duration(ContinuousEffectDuration::UntilEndOfTurn),
                )
                .unwrap_or_else(|error| {
                    panic!("unexpected temporary restriction error: {error:?}")
                });
        }

        let requirements = [TargetRequirement::new(TargetKind::Permanent)];
        assert!(state
            .validate_target_choices(
                opponent,
                None,
                &requirements,
                &[TargetChoice::Object(creature)],
            )
            .is_err());
        assert!(state
            .validate_target_choices(
                active,
                None,
                &requirements,
                &[TargetChoice::Object(creature)],
            )
            .is_ok());
        assert_eq!(
            apply(&mut state, Action::DestroyPermanent { object: artifact }),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(artifact),
            Some(ZoneId::new(None, ZoneKind::Battlefield))
        );
        assert_eq!(
            apply(
                &mut state,
                Action::MarkDamageOnObject {
                    object: creature,
                    amount: 2,
                },
            ),
            Outcome::Applied
        );
        assert!(matches!(
            apply(&mut state, Action::CheckStateBasedActions),
            Outcome::StateBasedActions(report) if report.actions_performed() == 0
        ));

        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        while state.current_step() != Some(Step::Cleanup) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected cleanup walk error: {error:?}"));
        }
        assert_eq!(state.last_cleanup_report().expired_until_end_of_turn(), 3);
        assert_eq!(state.restrictions().count(), 0);
        assert!(state
            .validate_target_choices(
                opponent,
                None,
                &requirements,
                &[TargetChoice::Object(creature)],
            )
            .is_ok());
        assert_eq!(
            apply(&mut state, Action::DestroyPermanent { object: artifact }),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(artifact),
            Some(ZoneId::new(Some(active), ZoneKind::Graveyard))
        );
        assert_eq!(
            apply(
                &mut state,
                Action::MarkDamageOnObject {
                    object: creature,
                    amount: 2,
                },
            ),
            Outcome::Applied
        );
        assert!(matches!(
            apply(&mut state, Action::CheckStateBasedActions),
            Outcome::StateBasedActions(report) if report.actions_performed() == 1
        ));
        assert_eq!(
            state.object_zone(creature),
            Some(ZoneId::new(Some(active), ZoneKind::Graveyard))
        );
    }

    #[test]
    fn cleanup_discards_to_max_hand_size_before_next_turn() {
        let mut state = GameState::new();
        let active = state.add_player();
        let next = state.add_player();
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let graveyard = ZoneId::new(Some(active), ZoneKind::Graveyard);
        ensure_library_card(&mut state, active);
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

        assert_eq!(state.last_cleanup_report().discarded(), 4);
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
            4
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
        ensure_library_card(&mut state, active);
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
    fn cleanup_state_based_action_grants_priority_and_repeats_cleanup() {
        let mut state = GameState::new();
        let active = state.add_player();
        state.add_player();
        ensure_library_card(&mut state, active);
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        while state.current_step() != Some(Step::End) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected end-step walk error: {error:?}"));
        }
        let doomed = battlefield_creature(&mut state, active, 228, 0, 0, CreatureKeywords::none());

        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected cleanup advance error: {error:?}"));

        assert_eq!(state.current_step(), Some(Step::Cleanup));
        assert_eq!(state.cleanup_iteration(), 1);
        assert_eq!(state.priority_player(), Some(active));
        assert_eq!(
            state.object_zone(doomed),
            Some(ZoneId::new(Some(active), ZoneKind::Graveyard))
        );

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
    fn mana_activated_ability_resolves_without_priority_or_stack() {
        let mut state = GameState::new();
        let active = state.add_player();
        let source = state
            .create_object(
                CardId::new(25_001),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        let ability = state
            .register_activated_ability(
                ActivatedAbilityDefinition::new(
                    active,
                    Some(source),
                    ActivationTiming::Instant,
                    ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_tap_source(),
                    ActivatedAbilityEffect::AddMana {
                        player: AbilityPlayer::Controller,
                        mana: ManaPool::new(0, 0, 0, 0, 1, 0),
                    },
                )
                .as_mana_ability(),
            )
            .unwrap_or_else(|error| panic!("unexpected ability registration error: {error:?}"));

        assert_eq!(
            state.activate_ability(
                active,
                ability,
                zero_payment(ManaCost::new(0, 0, 0, 0, 0, 0))
            ),
            Ok(None)
        );

        assert!(state
            .object(source)
            .unwrap_or_else(|| panic!("source missing"))
            .tapped());
        assert!(state.stack_entries().is_empty());
        assert_eq!(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaPool::new(0, 0, 0, 0, 1, 0)
        );
        assert_eq!(state.priority_player(), None);
    }

    #[test]
    fn mana_activation_condition_fails_closed_until_permanent_count_is_met() {
        let mut state = GameState::new();
        let active = state.add_player();
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let land_base =
            BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), ObjectColors::none());
        let mut lands = Vec::new();
        for offset in 0..4 {
            let land = state
                .create_object(CardId::new(25_100 + offset), active, active, battlefield)
                .unwrap_or_else(|error| panic!("unexpected land create error: {error:?}"));
            state
                .set_base_object_characteristics(land, land_base)
                .unwrap_or_else(|error| panic!("unexpected land type error: {error:?}"));
            lands.push(land);
        }
        let source = lands[0];
        let condition = ActivationCondition::ControllerControlsAtLeast {
            predicate: ObjectTargetPredicate::any()
                .with_controller(TargetControllerPredicate::You)
                .with_required_types(ObjectTypes::none().with_land()),
            count: 5,
        };
        let ability = state
            .register_activated_ability(
                ActivatedAbilityDefinition::new(
                    active,
                    Some(source),
                    ActivationTiming::Instant,
                    ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_tap_source(),
                    ActivatedAbilityEffect::AddMana {
                        player: AbilityPlayer::Controller,
                        mana: ManaPool::of(ManaKind::Colorless, 2),
                    },
                )
                .with_condition(condition)
                .as_mana_ability(),
            )
            .unwrap_or_else(|error| panic!("unexpected ability registration error: {error:?}"));
        let payment = zero_payment(ManaCost::new(0, 0, 0, 0, 0, 0));
        let before = state.canonical_bytes();

        assert_eq!(
            state.activate_ability(active, ability, payment),
            Err(StateError::ActivationConditionNotMet(ability))
        );
        assert_eq!(state.canonical_bytes(), before);

        let fifth = state
            .create_object(CardId::new(25_104), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected fifth land create error: {error:?}"));
        state
            .set_base_object_characteristics(fifth, land_base)
            .unwrap_or_else(|error| panic!("unexpected fifth land type error: {error:?}"));
        assert_eq!(state.activate_ability(active, ability, payment), Ok(None));
        assert!(state.object(source).is_some_and(|record| record.tapped()));
        assert_eq!(
            state.mana_pool(active),
            Ok(ManaPool::of(ManaKind::Colorless, 2))
        );
    }

    #[test]
    fn loyalty_activated_ability_is_once_per_turn() {
        let mut state = GameState::new();
        let active = state.add_player();
        let source = state
            .create_object(
                CardId::new(25_002),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        state
            .set_object_loyalty(source, Some(3))
            .unwrap_or_else(|error| panic!("unexpected loyalty setup error: {error:?}"));
        let ability = state
            .register_activated_ability(ActivatedAbilityDefinition::new(
                active,
                Some(source),
                ActivationTiming::Sorcery,
                ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)).with_loyalty_delta(1),
                ActivatedAbilityEffect::GainLife {
                    player: AbilityPlayer::Controller,
                    amount: 1,
                },
            ))
            .unwrap_or_else(|error| panic!("unexpected ability registration error: {error:?}"));
        start_upkeep(&mut state, active);
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected draw advance error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected main advance error: {error:?}"));

        let payment = zero_payment(ManaCost::new(0, 0, 0, 0, 0, 0));
        let entry = state
            .activate_ability(active, ability, payment)
            .unwrap_or_else(|error| panic!("unexpected loyalty activation error: {error:?}"))
            .unwrap_or_else(|| panic!("loyalty ability should use the stack"));
        assert_eq!(
            state
                .object(source)
                .unwrap_or_else(|| panic!("source missing"))
                .loyalty(),
            Some(4)
        );
        assert_eq!(
            state.pass_priority(active),
            Ok(PriorityOutcome::Resolved(entry))
        );
        assert_eq!(
            state.activate_ability(active, ability, payment),
            Err(StateError::LoyaltyAbilityAlreadyActivatedThisTurn(source))
        );
    }

    #[test]
    fn activated_cost_modifier_changes_required_payment() {
        let mut state = GameState::new();
        let active = state.add_player();
        let source = state
            .create_object(
                CardId::new(25_003),
                active,
                active,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        let ability = state
            .register_activated_ability(ActivatedAbilityDefinition::new(
                active,
                Some(source),
                ActivationTiming::Instant,
                ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 1)),
                ActivatedAbilityEffect::GainLife {
                    player: AbilityPlayer::Controller,
                    amount: 1,
                },
            ))
            .unwrap_or_else(|error| panic!("unexpected ability registration error: {error:?}"));
        state
            .register_cost_modifier(CostModifierDefinition::new(
                active,
                None,
                CostModifierScope::Ability(ability),
                CostModifierOperation::AddGeneric(1),
            ))
            .unwrap_or_else(|error| panic!("unexpected modifier registration error: {error:?}"));
        start_upkeep(&mut state, active);
        state
            .add_mana_to_pool(active, ManaPool::new(0, 0, 0, 0, 0, 1))
            .unwrap_or_else(|error| panic!("unexpected mana add error: {error:?}"));
        let base_payment = validate_payment_plan(
            state
                .mana_pool(active)
                .unwrap_or_else(|error| panic!("unexpected mana pool error: {error:?}")),
            ManaCost::new(0, 0, 0, 0, 0, 1),
            ManaPool::new(0, 0, 0, 0, 0, 1),
        )
        .unwrap_or_else(|error| panic!("unexpected base payment error: {error:?}"));

        assert_eq!(
            state.activate_ability(active, ability, base_payment),
            Err(StateError::InvalidPaymentPlan)
        );
        assert_eq!(state.players()[active.index()].life(), 20);
        assert!(state.stack_entries().is_empty());
    }

    #[test]
    fn source_bound_spell_cost_modifier_matches_subtype_and_expires_off_battlefield() {
        let mut state = GameState::new();
        let active = state.add_player();
        let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
        let hand = ZoneId::new(Some(active), ZoneKind::Hand);
        let source = state
            .create_object(CardId::new(25_200), active, active, battlefield)
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        let goblin = state
            .create_object(CardId::new(25_201), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected Goblin create error: {error:?}"));
        let other = state
            .create_object(CardId::new(25_202), active, active, hand)
            .unwrap_or_else(|error| panic!("unexpected other create error: {error:?}"));
        let goblin_subtype =
            ObjectSubtype::parse("Goblin").unwrap_or_else(|| panic!("Goblin subtype must parse"));
        let goblin_subtypes = ObjectSubtypes::none()
            .try_with(goblin_subtype)
            .unwrap_or_else(|| panic!("Goblin subtype set must fit"));
        state
            .set_base_object_characteristics(
                goblin,
                BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_creature(),
                    ObjectColors::none().with_red(),
                )
                .with_subtypes(goblin_subtypes),
            )
            .unwrap_or_else(|error| panic!("unexpected Goblin characteristics error: {error:?}"));
        state
            .set_base_object_characteristics(
                other,
                BaseObjectCharacteristics::new(
                    ObjectTypes::none().with_creature(),
                    ObjectColors::none().with_red(),
                ),
            )
            .unwrap_or_else(|error| panic!("unexpected other characteristics error: {error:?}"));
        let predicate = ObjectTargetPredicate::any()
            .with_required_types(ObjectTypes::none().with_creature())
            .with_required_subtypes(goblin_subtypes);
        state
            .register_cost_modifier(CostModifierDefinition::new(
                active,
                Some(source),
                CostModifierScope::Spells(predicate),
                CostModifierOperation::ReduceGeneric(1),
            ))
            .unwrap_or_else(|error| panic!("unexpected modifier registration error: {error:?}"));
        let base = ManaCost::new(0, 0, 0, 1, 0, 2);

        assert_eq!(
            state.effective_spell_cost(active, goblin, base),
            Ok(ManaCost::new(0, 0, 0, 1, 0, 1))
        );
        assert_eq!(state.effective_spell_cost(active, other, base), Ok(base));

        state
            .move_object(source, ZoneId::new(Some(active), ZoneKind::Graveyard))
            .unwrap_or_else(|error| panic!("unexpected source move error: {error:?}"));
        assert_eq!(state.effective_spell_cost(active, goblin, base), Ok(base));
    }

    #[test]
    fn source_bound_player_rule_tracks_controller_and_expires_off_battlefield() {
        let mut state = GameState::new();
        let first = state.add_player();
        let second = state.add_player();
        let source = state
            .create_object(
                CardId::new(25_300),
                first,
                first,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected source create error: {error:?}"));
        state
            .register_restriction(
                RestrictionDefinition::new(
                    first,
                    RestrictionEffect::PlayerRule {
                        subject: PlayerRuleSubject::ControllerOfSource,
                        rule: PlayerRule::NoMaximumHandSize,
                    },
                )
                .with_source(source),
            )
            .unwrap_or_else(|error| panic!("unexpected player-rule registration error: {error:?}"));

        assert_eq!(state.effective_max_hand_size(first), Ok(None));
        assert_eq!(state.effective_max_hand_size(second), Ok(Some(7)));

        register_continuous(
            &mut state,
            ContinuousEffectDefinition::new(
                second,
                ContinuousEffectTarget::Object(source),
                ContinuousEffectOperation::ChangeController { controller: second },
            ),
        );
        assert_eq!(state.effective_max_hand_size(first), Ok(Some(7)));
        assert_eq!(state.effective_max_hand_size(second), Ok(None));

        state
            .move_object(source, ZoneId::new(Some(first), ZoneKind::Graveyard))
            .unwrap_or_else(|error| panic!("unexpected source move error: {error:?}"));
        assert_eq!(state.effective_max_hand_size(first), Ok(Some(7)));
        assert_eq!(state.effective_max_hand_size(second), Ok(Some(7)));
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
    fn counter_stack_entry_moves_spell_and_records_distinct_outcome() {
        let mut state = GameState::new();
        let caster = state.add_player();
        let _responder = state.add_player();
        let caster_hand = ZoneId::new(Some(caster), ZoneKind::Hand);
        let target = state
            .create_object(CardId::new(9_100), caster, caster, caster_hand)
            .unwrap_or_else(|error| panic!("unexpected target create error: {error:?}"));
        start_upkeep(&mut state, caster);
        let target_entry = state
            .put_spell_on_stack(caster, target, StackObjectKind::InstantSpell, true)
            .unwrap_or_else(|error| panic!("unexpected target stack error: {error:?}"));

        assert_eq!(
            apply(
                &mut state,
                Action::CounterStackEntry {
                    entry: target_entry,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(
            state.object_zone(target),
            Some(ZoneId::new(Some(caster), ZoneKind::Graveyard))
        );
        assert!(state.stack_entries().is_empty());
        assert_eq!(state.resolution_log().len(), 1);
        assert_eq!(state.resolution_log()[0].stack_entry(), target_entry);
        assert_eq!(
            state.resolution_log()[0].outcome(),
            ResolutionOutcome::CounteredBySpell
        );
        assert_eq!(
            apply(
                &mut state,
                Action::CounterStackEntry {
                    entry: target_entry,
                },
            ),
            Outcome::Failed(StateError::UnknownStackEntry(target_entry))
        );
        assert_eq!(
            state.validate_zone_conservation(),
            Ok(ZoneConservation { object_count: 2 })
        );
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

    fn add_player_action(state: &mut GameState) -> PlayerId {
        match apply(state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected add-player outcome: {other:?}"),
        }
    }

    fn create_object_action(
        state: &mut GameState,
        card: CardId,
        owner: PlayerId,
        zone: ZoneId,
    ) -> super::ObjectId {
        match apply(
            state,
            Action::CreateObject {
                card,
                owner,
                controller: owner,
                zone,
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            other => panic!("unexpected create-object outcome: {other:?}"),
        }
    }

    fn seed_library_cards(state: &mut GameState, player: PlayerId, first_card: u32, count: u32) {
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        for offset in 0..count {
            match apply(
                state,
                Action::CreateObject {
                    card: CardId::new(first_card + offset),
                    owner: player,
                    controller: player,
                    zone: library,
                },
            ) {
                Outcome::ObjectCreated(_) => {}
                other => panic!("unexpected library seed outcome: {other:?}"),
            }
        }
    }

    fn start_upkeep(state: &mut GameState, active: PlayerId) {
        ensure_library_card(state, active);
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        state
            .advance_step()
            .unwrap_or_else(|error| panic!("unexpected upkeep advance error: {error:?}"));
    }

    fn start_declare_attackers(state: &mut GameState, active: PlayerId) {
        ensure_library_card(state, active);
        state
            .start_turn(active)
            .unwrap_or_else(|error| panic!("unexpected start error: {error:?}"));
        while state.current_step() != Some(Step::DeclareAttackers) {
            state
                .advance_step()
                .unwrap_or_else(|error| panic!("unexpected combat walk error: {error:?}"));
        }
    }

    fn ensure_library_card(state: &mut GameState, player: PlayerId) {
        let library = ZoneId::new(Some(player), ZoneKind::Library);
        if state
            .zone(library)
            .unwrap_or_else(|| panic!("library zone missing"))
            .objects()
            .is_empty()
        {
            state
                .create_object(CardId::new(9_999), player, player, library)
                .unwrap_or_else(|error| panic!("unexpected library seed error: {error:?}"));
        }
    }

    fn battlefield_creature(
        state: &mut GameState,
        controller: PlayerId,
        card: u32,
        power: i32,
        toughness: i32,
        keywords: CreatureKeywords,
    ) -> super::ObjectId {
        let object = state
            .create_object(
                CardId::new(card),
                controller,
                controller,
                ZoneId::new(None, ZoneKind::Battlefield),
            )
            .unwrap_or_else(|error| panic!("unexpected battlefield create error: {error:?}"));
        state
            .set_base_creature_characteristics(
                object,
                BaseCreatureCharacteristics::new(power, toughness).with_keywords(keywords),
            )
            .unwrap_or_else(|error| panic!("unexpected creature characteristics error: {error:?}"));
        object
    }

    fn register_replacement(
        state: &mut GameState,
        definition: ReplacementDefinition,
    ) -> ReplacementEffectId {
        match apply(state, Action::RegisterReplacementEffect { definition }) {
            Outcome::ReplacementEffectRegistered(replacement) => replacement,
            other => panic!("unexpected replacement registration outcome: {other:?}"),
        }
    }

    fn register_continuous(
        state: &mut GameState,
        definition: ContinuousEffectDefinition,
    ) -> ContinuousEffectId {
        match apply(state, Action::RegisterContinuousEffect { definition }) {
            Outcome::ContinuousEffectRegistered(effect) => effect,
            other => panic!("unexpected continuous effect registration outcome: {other:?}"),
        }
    }

    fn combat_damage_record(
        source: super::ObjectId,
        target: CombatDamageTarget,
        amount: u32,
    ) -> super::CombatDamageRecord {
        super::CombatDamageRecord {
            source,
            target,
            amount,
            step: CombatDamageStepKind::Regular,
            source_had_deathtouch: false,
            source_had_lifelink: false,
        }
    }

    fn replacement_applications(state: &GameState) -> Vec<ReplacementEffectId> {
        state
            .events_this_turn()
            .iter()
            .filter_map(|record| match record.event() {
                GameEvent::ReplacementEffectApplied { replacement, .. } => Some(replacement),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn commander_zone_choice_is_owner_only_and_fail_closed() {
        let mut state = GameState::new();
        let owner = state.add_player();
        let opponent = state.add_player();
        let graveyard = ZoneId::new(Some(owner), ZoneKind::Graveyard);
        let command = ZoneId::new(None, ZoneKind::Command);
        let commander = state
            .create_object(CardId::new(9_901), owner, owner, graveyard)
            .unwrap_or_else(|error| panic!("unexpected commander create error: {error:?}"));
        state
            .designate_commander(commander, ObjectColors::none())
            .unwrap_or_else(|error| panic!("unexpected designation error: {error:?}"));

        assert_eq!(
            apply(
                &mut state,
                Action::ChooseCommanderZone {
                    player: opponent,
                    object: commander,
                },
            ),
            Outcome::Failed(StateError::CommanderZoneChoiceOwnerMismatch {
                player: opponent,
                object: commander,
            })
        );
        assert_eq!(state.object_zone(commander), Some(graveyard));
        assert_eq!(
            apply(
                &mut state,
                Action::ChooseCommanderZone {
                    player: owner,
                    object: commander,
                },
            ),
            Outcome::Applied
        );
        assert_eq!(state.object_zone(commander), Some(command));
        assert_eq!(
            apply(
                &mut state,
                Action::ChooseCommanderZone {
                    player: owner,
                    object: commander,
                },
            ),
            Outcome::Failed(StateError::InvalidCommanderZoneChoice {
                object: commander,
                from: command,
            })
        );

        let ordinary = state
            .create_object(CardId::new(9_902), owner, owner, graveyard)
            .unwrap_or_else(|error| panic!("unexpected ordinary create error: {error:?}"));
        assert_eq!(
            apply(
                &mut state,
                Action::ChooseCommanderZone {
                    player: owner,
                    object: ordinary,
                },
            ),
            Outcome::Failed(StateError::ObjectNotCommander(ordinary))
        );
    }

    fn zero_payment(cost: ManaCost) -> super::PaymentPlan {
        validate_payment_plan(ManaPool::empty(), cost, ManaPool::empty())
            .unwrap_or_else(|error| panic!("unexpected zero payment error: {error:?}"))
    }

    fn cast_zero_cost_target_spell(
        state: &mut GameState,
        active: PlayerId,
        spell: super::ObjectId,
        target: super::ObjectId,
    ) {
        let cost = ManaCost::new(0, 0, 0, 0, 0, 0);
        let request = CastSpellRequest::new(
            StackObjectKind::InstantSpell,
            SpellTiming::Instant,
            cost,
            zero_payment(cost),
        )
        .with_targets(
            vec![TargetRequirement::new(TargetKind::Permanent)],
            vec![TargetChoice::Object(target)],
        );
        state
            .cast_spell(active, spell, request)
            .unwrap_or_else(|error| panic!("unexpected target cast error: {error:?}"));
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
