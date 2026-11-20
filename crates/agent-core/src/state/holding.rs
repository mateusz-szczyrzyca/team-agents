//! Who held a task before the agent that holds it now: the records of the hand-overs the ladder made, oldest first.

use serde::{Deserialize, Serialize};

use super::Participant;
use super::values::Timestamp;

/// A hand-over of one task from one agent to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holding {
    /// The agent the task was taken from: it held the task until [`Holding::until`].
    pub agent: Participant,
    /// When the task was given to the agent that holds it now.
    pub until: Timestamp,
}
