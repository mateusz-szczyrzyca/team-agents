//! Investigation data types and transitions on the bus.

use std::num::NonZeroU32;
use std::path::PathBuf;

use agent_core::ids::{AgentName, BranchName, ItemId, TaskId};
use serde::{Deserialize, Deserializer};

/// The kind of an investigation request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InvestigationKind {
    /// A question from an agent (`""`).
    #[default]
    Question,
    /// A coordinator asks for a split of a validated task-list item.
    Split {
        /// The parent item to divide.
        item: ItemId,
    },
    /// A coordinator asks for help resolving an item pull request conflict.
    Conflict {
        /// The item whose pull request must be investigated.
        item: ItemId,
    },
    /// A PR review (`"review"`).
    Review,
    /// Any other kind that an earlier or later bus version wrote.
    Unknown(String),
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
        Ok(match raw.as_str() {
            "queued" => Self::Queued,
            "waitroom" => Self::Waitroom,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "waiting_human" => Self::WaitingHuman,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "rejected" => Self::Rejected,
            _ => Self::Unknown(raw),
        })
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
    #[serde(flatten, deserialize_with = "deserialize_request_kind")]
    pub kind: InvestigationKind,
    /// Bus lifecycle status, including queued and running requests returned by `inv_open`.
    #[serde(default)]
    pub status: Option<InvestigationStatus>,
    /// Investigator task assigned by the daemon, absent before assignment.
    #[serde(default)]
    pub inv_task: Option<TaskId>,
    /// Path to the markdown record file.
    pub file: PathBuf,
}

/// Reads the kind and its item together so split requests carry a validated item.
fn deserialize_request_kind<'de, D>(deserializer: D) -> Result<InvestigationKind, D::Error>
where
    D: Deserializer<'de>,
{
    /// Wire fields whose meaning depends on the request kind.
    #[derive(Deserialize)]
    struct RequestKind {
        /// Legacy requests omit their question kind.
        #[serde(default)]
        kind: String,
        /// Split requests must name a valid task-list item.
        item: Option<String>,
    }
    let raw = RequestKind::deserialize(deserializer)?;
    Ok(match raw.kind.as_str() {
        "split" => InvestigationKind::Split {
            item: raw
                .item
                .ok_or_else(|| serde::de::Error::missing_field("item"))?
                .parse()
                .map_err(serde::de::Error::custom)?,
        },
        "" | "question" => InvestigationKind::Question,
        "review" => InvestigationKind::Review,
        _ => InvestigationKind::Unknown(raw.kind),
    })
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
