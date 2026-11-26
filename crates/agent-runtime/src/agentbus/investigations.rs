//! Investigation data types and transitions on the bus.

use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::ids::{AgentName, BranchName, TaskId};
use serde::{Deserialize, Deserializer};

/// The kind of an investigation request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InvestigationKind {
    /// A question from an agent (`""`).
    #[default]
    Question,
    /// A PR review (`"review"`).
    Review,
    /// Any other kind that an earlier or later bus version wrote.
    Unknown(String),
}

impl<'de> Deserialize<'de> for InvestigationKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "" => Self::Question,
            "review" => Self::Review,
            _ => Self::Unknown(s),
        })
    }
}

/// The lifecycle status returned by the bus, retaining future values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvestigationStatus {
    /// Waiting for a free investigator.
    Queued,
    /// Waiting outside the bounded queue.
    Waitroom,
    /// Claimed but not yet assigned a task.
    Starting,
    /// Assigned to an investigator task.
    Running,
    /// Waiting for a human decision.
    WaitingHuman,
    /// Answered successfully.
    Done,
    /// Ended without an answer.
    Failed,
    /// Refused by the queue.
    Rejected,
    /// An absent status or one introduced by another bus version.
    Unknown(String),
}

impl<'de> Deserialize<'de> for InvestigationStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::Unknown(raw))
    }
}

/// An investigation request from the bus.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Investigation {
    /// The sequential number of the request on the bus.
    pub id: NonZeroU32,
    /// The agent that asked for help.
    pub from_agent: AgentName,
    /// The task that asked for help.
    pub from_task: TaskId,
    /// The branch of the requesting task.
    pub branch: BranchName,
    /// The specific question or problem to investigate.
    pub question: String,
    /// Context the requester already knows.
    #[serde(default)]
    pub context: String,
    /// The kind of investigation.
    #[serde(default)]
    pub kind: InvestigationKind,
    /// Path to the markdown record file.
    pub file: PathBuf,
}

/// A transition of an investigation request's state on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvTransition {
    /// The investigation started running under `task`.
    Running {
        /// The task ID of the investigator run.
        task: TaskId,
    },
    /// The investigation finished with a summary.
    Done {
        /// Short result summary.
        summary: String,
    },
    /// The investigation failed without a usable result.
    Failed {
        /// Reason for failure.
        reason: String,
    },
    /// The request was put back into the queue.
    Requeued,
}
