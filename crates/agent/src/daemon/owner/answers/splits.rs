//! Atomic interpretation of a coordinator's approval of a split plan.

use agent_core::kinds::Role;
use agent_core::questions::QuestionsFile;
use agent_core::state::{Team, split::SplitPlan};

/// A validated plan and the acknowledgement sent to the human; neither for an ordinary answer.
#[derive(Debug)]
pub(super) struct Decision {
    /// The whole plan ready to activate, absent when refused.
    pub(super) plan: Option<SplitPlan>,
    /// What was recorded, or why nothing was recorded.
    pub(super) acknowledgement: Option<String>,
}

/// Checks a whole questions file without changing its team.
pub(super) fn prepare(_team: &Team, _role: Role, _file: &QuestionsFile) -> Decision {
    Decision {
        plan: None,
        acknowledgement: None,
    }
}

#[cfg(test)]
mod tests;
