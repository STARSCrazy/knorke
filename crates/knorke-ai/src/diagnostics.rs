//! Errors as [`Diagnostic`]s with stable codes `knorke.ai.*` (CLAUDE.md, ADR 0012).
//!
//! Every public error of the crate has a `to_diagnostic`. The codes are part of the public
//! contract: renaming one is a breaking change.

use knorke_types::{Diagnostic, DiagnosticCode};

use crate::DecideError;
use crate::group::{CrewError, PlanError};
use crate::memory::{StepError, ZeroCapacity};
use crate::mission::MissionError;
use crate::state::UnknownState;

fn error(code: &'static str, e: &impl core::fmt::Display) -> Diagnostic {
    Diagnostic::error(DiagnosticCode::new(code), e.to_string())
}

impl DecideError {
    /// As a diagnostic, code `knorke.ai.decide.*`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error(
            match self {
                DecideError::UnknownOption { .. } => "knorke.ai.decide.unknown_option",
                DecideError::InputOutOfRange { .. } => "knorke.ai.decide.input_out_of_range",
                DecideError::InputsTooShort => "knorke.ai.decide.inputs_too_short",
                DecideError::RowLengths => "knorke.ai.decide.row_lengths",
            },
            self,
        )
    }
}

impl UnknownState {
    /// As a diagnostic, code `knorke.ai.state.unknown`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error("knorke.ai.state.unknown", self)
    }
}

impl ZeroCapacity {
    /// As a diagnostic, code `knorke.ai.memory.zero_capacity`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error("knorke.ai.memory.zero_capacity", self)
    }
}

impl StepError {
    /// As a diagnostic, code `knorke.ai.memory.goal_guess.*`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error(
            match self {
                StepError::Shape => "knorke.ai.memory.goal_guess.shape",
                StepError::Jump => "knorke.ai.memory.goal_guess.jump",
            },
            self,
        )
    }
}

impl CrewError {
    /// As a diagnostic, code `knorke.ai.group.crew.*`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error(
            match self {
                CrewError::ZeroCapacity => "knorke.ai.group.crew.zero_capacity",
                CrewError::HexNeedsHexMetric => "knorke.ai.group.crew.hex_needs_hex_metric",
            },
            self,
        )
    }
}

impl PlanError {
    /// As a diagnostic, code `knorke.ai.group.squad.*`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error(
            match self {
                PlanError::InputOutOfRange { .. } => "knorke.ai.group.squad.input_out_of_range",
                PlanError::TooManySlots { .. } => "knorke.ai.group.squad.too_many_slots",
                PlanError::OptionsChanged => "knorke.ai.group.squad.options_changed",
                PlanError::NeedsRules => "knorke.ai.group.squad.needs_rules",
            },
            self,
        )
    }
}

impl MissionError {
    /// As a diagnostic, code `knorke.ai.mission.*`.
    pub fn to_diagnostic(&self) -> Diagnostic {
        error(
            match self {
                MissionError::ZeroCapacity => "knorke.ai.mission.zero_capacity",
                MissionError::EmptyPlan => "knorke.ai.mission.empty_plan",
                MissionError::TooManySteps => "knorke.ai.mission.too_many_steps",
                MissionError::PlaybookFull => "knorke.ai.mission.playbook_full",
                MissionError::TooManyPlans => "knorke.ai.mission.too_many_plans",
                MissionError::HexNeedsHexMetric => "knorke.ai.mission.hex_needs_hex_metric",
            },
            self,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let d = DecideError::RowLengths.to_diagnostic();
        assert_eq!(d.code.as_str(), "knorke.ai.decide.row_lengths");
        assert_eq!(
            UnknownState(4).to_diagnostic().code.as_str(),
            "knorke.ai.state.unknown"
        );
        assert_eq!(
            MissionError::EmptyPlan.to_diagnostic().code.as_str(),
            "knorke.ai.mission.empty_plan"
        );
        assert_eq!(
            PlanError::NeedsRules.to_diagnostic().code.as_str(),
            "knorke.ai.group.squad.needs_rules"
        );
        assert!(!ZeroCapacity.to_diagnostic().message.is_empty());
    }
}
