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
pub(super) fn prepare(team: &Team, role: Role, file: &QuestionsFile) -> Decision {
    let mut plans = Vec::new();
    let mut mentions_split = false;
    for verdict in file.entries().iter().filter_map(agent_core::questions::Entry::verdict) {
        if let agent_core::questions::Verdict::Own(text) = verdict {
            mentions_split |= text.as_str().to_lowercase().contains("split");
        }
        match agent_core::questions::split::read_split(verdict) {
            Ok(Some(plan)) => plans.push(plan),
            Ok(None) => {}
            Err(error) => return refused(&agent_runtime::process::error_chain(&error)),
        }
    }
    if plans.is_empty() {
        return if mentions_split {
            refused("use the explicit split verdict form; free words do not approve parts")
        } else {
            Decision {
                plan: None,
                acknowledgement: None,
            }
        };
    }
    if role != Role::Coordinator {
        return refused("only the coordinator can request a split");
    }
    if plans.len() != 1 {
        return refused("approve exactly one split in one questions file");
    }
    let Some(plan) = plans.pop() else {
        return refused("approve exactly one split in one questions file");
    };
    let mut checked = team.clone();
    if let Err(error) = checked.activate_split(plan.clone()) {
        return refused(&format!(
            "cannot activate: {}",
            agent_runtime::process::error_chain(&error)
        ));
    }
    let parts = plan
        .entries()
        .map(|(id, part)| format!("{id} — {}", part.title))
        .collect::<Vec<_>>()
        .join("; ");
    Decision {
        plan: Some(plan),
        acknowledgement: Some(format!("Recorded split parts: {parts}.")),
    }
}

/// A split-related answer whose refusal must be visible while the ordinary answer still resumes its task.
fn refused(reason: &str) -> Decision {
    Decision {
        plan: None,
        acknowledgement: Some(format!("No split recorded: {reason}.")),
    }
}

#[cfg(test)]
mod tests;
