use crate::{SearchReport, SearchStopReason};
use forge_core::CanonicalActionId;

/// One concrete root alternative exposed for hints and postgame review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsideredAction {
    action: CanonicalActionId,
    visits: u64,
    mean_value: i64,
    value_delta_from_selected: i64,
}

impl ConsideredAction {
    /// Returns the canonical concrete action ID.
    #[must_use]
    pub const fn action(self) -> CanonicalActionId {
        self.action
    }

    /// Returns aggregate visits across determinizations.
    #[must_use]
    pub const fn visits(self) -> u64 {
        self.visits
    }

    /// Returns mean root value.
    #[must_use]
    pub const fn mean_value(self) -> i64 {
        self.mean_value
    }

    /// Returns this action's mean value minus the selected action's value.
    #[must_use]
    pub const fn value_delta_from_selected(self) -> i64 {
        self.value_delta_from_selected
    }
}

/// Inspectable summary of the latest searched decision.
///
/// This hook contains canonical IDs and numeric search evidence only. UI text
/// remains presentation data and is never an authority for replay or learning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastDecisionReport {
    selected_action: CanonicalActionId,
    considered: Vec<ConsideredAction>,
    simulations: u64,
    nodes: u64,
    determinizations: u32,
    wall_time_us: u64,
    stop_reason: SearchStopReason,
}

impl LastDecisionReport {
    /// Builds a top-three concrete-action report from a complete search report.
    #[must_use]
    pub fn from_search(report: &SearchReport) -> Self {
        let selected_value = report
            .actions()
            .iter()
            .find(|action| action.action() == report.selected_action())
            .map_or(0, |action| action.mean_value());
        let mut ranked = report.actions().to_vec();
        ranked.sort_by(|left, right| {
            right
                .visits()
                .cmp(&left.visits())
                .then_with(|| right.mean_value().cmp(&left.mean_value()))
                .then_with(|| left.action().cmp(&right.action()))
        });
        let considered = ranked
            .into_iter()
            .take(3)
            .map(|action| ConsideredAction {
                action: action.action(),
                visits: action.visits(),
                mean_value: action.mean_value(),
                value_delta_from_selected: action.mean_value().saturating_sub(selected_value),
            })
            .collect();
        Self {
            selected_action: report.selected_action(),
            considered,
            simulations: report.simulations(),
            nodes: report.nodes(),
            determinizations: report.determinizations(),
            wall_time_us: report.actual_wall_time_us(),
            stop_reason: report.stop_reason(),
        }
    }

    /// Returns the selected canonical action.
    #[must_use]
    pub const fn selected_action(&self) -> CanonicalActionId {
        self.selected_action
    }

    /// Returns up to three root alternatives in search rank order.
    #[must_use]
    pub fn considered(&self) -> &[ConsideredAction] {
        &self.considered
    }

    /// Returns total simulations.
    #[must_use]
    pub const fn simulations(&self) -> u64 {
        self.simulations
    }

    /// Returns total allocated search nodes.
    #[must_use]
    pub const fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Returns completed hidden-information samples.
    #[must_use]
    pub const fn determinizations(&self) -> u32 {
        self.determinizations
    }

    /// Returns measured search wall time in microseconds.
    #[must_use]
    pub const fn wall_time_us(&self) -> u64 {
        self.wall_time_us
    }

    /// Returns the search stop reason.
    #[must_use]
    pub const fn stop_reason(&self) -> SearchStopReason {
        self.stop_reason
    }
}
