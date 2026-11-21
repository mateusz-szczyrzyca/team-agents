//! The choice of who takes a task over when its own agent cannot go on: another configured agent of the same or a
//! higher place, the team's lead, or the human.
//!
//! [`choose`] is the whole of the decision: a table over the place of the task ([`Role`]), the place of the agent that
//! held it, the configured agents in order with their places and whether each is in a run, and `[replacement] mode`.
//! It touches nothing; the daemon asks who holds it and records the answer.

use crate::ids::AgentName;
use crate::kinds::{Role, Tier};

/// Whether an agent is working in a run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// It is not working: it may take the task.
    Idle,
    /// A run of it is going: it is not taken from that run, and it is not asked to take another.
    InRun,
}

/// An agent the configuration names in `[replacement] agents`, in the order it names them.
///
/// The agent that holds the task now is left out of `candidates`: the choice does not know its name, and an agent is
/// never asked to take over its own task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// The agent.
    pub agent: &'a AgentName,
    /// Its place in the configuration.
    pub tier: Tier,
    /// Whether it works in a run now.
    pub activity: Activity,
}

/// `[replacement] mode`: how far the harness goes to keep a task going without the human.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementMode {
    /// Another suitable agent, then the team's lead, then the human.
    #[default]
    Auto,
    /// The team's lead; the human when the task has no lead.
    Lead,
    /// The human at once: the behaviour of the versions before `[replacement]`.
    Off,
}

/// Who takes the task over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handover<'a> {
    /// This configured agent, of the same or a higher place than the one it replaces.
    Agent(&'a AgentName),
    /// The team's lead, whose own task is given the work.
    Lead,
    /// The human, who is asked in the task's questions file.
    Human,
}

/// The place of a tier as a number: a worker below a normal agent, which is below an investigator.
#[must_use]
pub const fn rank(tier: Tier) -> u8 {
    match tier {
        Tier::Worker => 0,
        Tier::Normal => 1,
        Tier::Investigator => 2,
    }
}

/// Who takes over the task of `role`, whose agent had the tier `replaced`, under `mode`: `candidates` are the agents of
/// `[replacement] agents`, in the order the configuration names them, without the agent that holds the task now.
///
/// The answer is the first candidate that is idle and of the same or a higher place than `replaced`; when there is none,
/// a member's work goes to its team's lead, and everything else to the human. `[replacement] mode` narrows that: `lead`
/// always answers the team's lead, and `off` the human. A worker and a reviewer are outside this choice: the human
/// decides.
#[must_use]
pub fn choose<'a>(role: Role, replaced: Tier, candidates: &[Candidate<'a>], mode: ReplacementMode) -> Handover<'a> {
    // A worker's and a reviewer's task is outside this choice, whatever the configuration names.
    match role {
        Role::Worker | Role::Reviewer => return Handover::Human,
        Role::Solo | Role::Coordinator | Role::Member => {}
    }
    match mode {
        ReplacementMode::Off => Handover::Human,
        ReplacementMode::Lead => lead_of(role),
        ReplacementMode::Auto => {
            if let Some(candidate) = first_fit(replaced, candidates) {
                return Handover::Agent(candidate.agent);
            }
            lead_of(role)
        }
    }
}

/// The first candidate that is idle and of the same or a higher place than `replaced`.
fn first_fit<'a, 'b>(replaced: Tier, candidates: &'b [Candidate<'a>]) -> Option<&'b Candidate<'a>> {
    candidates
        .iter()
        .find(|candidate| candidate.activity == Activity::Idle && rank(candidate.tier) >= rank(replaced))
}

/// Who takes the work when no configured agent does: the lead of a member, and the human for a task that has no other
/// lead — a solo agent's, and a coordinator's, which is its own team's lead.
const fn lead_of(role: Role) -> Handover<'static> {
    match role {
        Role::Member => Handover::Lead,
        Role::Solo | Role::Coordinator | Role::Worker | Role::Reviewer => Handover::Human,
    }
}

#[cfg(test)]
mod tests;
