//! Who held a task before the agent that holds it now: the records of the hand-overs the ladder made, oldest first.

use serde::{Deserialize, Serialize};

use super::Participant;
use super::values::Timestamp;
use crate::ids::TaskId;
use crate::kinds::Role;

/// A hand-over of one task from one agent to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holding {
    /// The agent the task was taken from: it held the task until [`Holding::until`].
    pub agent: Participant,
    /// When the task was given to the agent that holds it now.
    pub until: Timestamp,
}

/// Why [`State::hand_over_task`](super::State::hand_over_task) refused to give a task to another agent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandOverError {
    /// The task is not in the state.
    #[error("task {task} is not in the state")]
    NoTask {
        /// The task.
        task: TaskId,
    },
    /// The task's role is never handed over: a worker serves one request, a reviewer one review.
    #[error("task {task} is a {role} and is never handed over")]
    Role {
        /// The task.
        task: TaskId,
        /// Its role.
        role: Role,
    },
    /// The agent named is the one that holds the task.
    #[error("task {task} is already held by that agent")]
    SameHolder {
        /// The task.
        task: TaskId,
    },
    /// The agent named is already in the task's team: one agent has one place in a team.
    #[error("{agent} is already in the team of task {task}")]
    InTeam {
        /// The task.
        task: TaskId,
        /// The agent's name.
        agent: String,
    },
    /// A run of the task is going: its agent still works on it.
    #[error("a run of task {task} is going")]
    InRun {
        /// The task.
        task: TaskId,
    },
}

#[cfg(test)]
mod tests;
