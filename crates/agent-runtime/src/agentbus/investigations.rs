//! Investigation data types and transitions on the bus.

use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::ids::{AgentName, BranchName, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The kind of an investigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvestigationKind {
    /// A question from an agent (`""`).
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
        let _unused = String::deserialize(deserializer)?;
        // STUB: return Question for everything
        Ok(Self::Question)
    }
}

impl Serialize for InvestigationKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Question => serializer.serialize_str(""),
            Self::Review => serializer.serialize_str("review"),
            Self::Unknown(s) => serializer.serialize_str(s),
        }
    }
}

impl Default for InvestigationKind {
    fn default() -> Self {
        Self::Question
    }
}

/// An investigation request from the bus.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
