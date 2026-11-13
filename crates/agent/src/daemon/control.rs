//! What a control of the human does to a task: the decision for each status, as a table.
//!
//! `stop`, `pause`, `unpause` and `resume` reach every task of a target; each task is looked at by itself, so a team
//! with one task that cannot be controlled is still controlled, and the answer names what was reached. Nothing here
//! touches the state: the owner carries out what [`decide`] says (`owner/controls.rs`).

use agent_core::protocol::ControlAction;
use agent_core::status::TaskStatus;

/// A control that changes the course of a task: `say` is not one, it only talks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Steer {
    /// Ends the task's work until it is resumed.
    Stop,
    /// Holds the task, with its run stopped, until it is unpaused.
    Pause,
    /// Lets a paused task go on.
    Unpause,
    /// Starts a stopped, failed or waiting task again.
    Resume,
}

impl From<Steer> for ControlAction {
    fn from(steer: Steer) -> Self {
        match steer {
            Steer::Stop => Self::Stop,
            Steer::Pause => Self::Pause,
            Steer::Unpause => Self::Unpause,
            Steer::Resume => Self::Resume,
        }
    }
}

/// How a run that is going is ended on the human's word; its end then sets the status of the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Halt {
    /// The task is stopped.
    Stop,
    /// The task is paused.
    Pause,
}

/// What is done to one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// The task has no run going: it goes to this status at once.
    Move(TaskStatus),
    /// The task has a run going: it is stopped, and the end of the run sets the status.
    Halt(Halt),
    /// The control does nothing to the task.
    Skip(Skipped),
}

/// Why a control does nothing to a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Skipped {
    /// The task is done: nothing is left to do for it.
    Over,
    /// The task is in the state the control asks for.
    Already,
    /// The task waits for the human's answers, which `agent answer` gives.
    Questions,
    /// The task is not paused, so there is nothing to unpause.
    NotPaused,
    /// The task is paused: `unpause` lets it go on.
    Paused,
    /// The task is going or about to start.
    Going,
    /// The task is stopped or failed: nothing is going that could be paused.
    Inactive,
}

impl Skipped {
    /// Why `steer` does nothing to the task, in words for the human.
    pub(super) const fn because(self, steer: Steer) -> &'static str {
        match (self, steer) {
            (Self::Over, _) => "it is done: nothing is left to do for it",
            (Self::Already, Steer::Stop) => "it is already stopped",
            (Self::Already, _) => "it is already paused",
            (Self::Questions, _) => "it waits for your answers and runs nothing: answer them with `agent answer`",
            (Self::NotPaused, _) => "it is not paused: there is nothing to unpause",
            (Self::Paused, _) => "it is paused: `agent unpause` lets it go on",
            (Self::Going, _) => "it is already going",
            (Self::Inactive, _) => "it is stopped or failed: nothing is going that could be paused",
        }
    }
}

/// What `steer` does to a task whose status is `status`.
pub(super) const fn decide(steer: Steer, status: TaskStatus) -> Step {
    use TaskStatus::{Done, Failed, Paused, Questions, Queued, Running, Stopped, Waiting};
    match (steer, status) {
        (Steer::Stop | Steer::Pause | Steer::Unpause | Steer::Resume, Done) => Step::Skip(Skipped::Over),
        (Steer::Stop, Queued | Waiting | Paused | Failed) => Step::Move(Stopped),
        (Steer::Stop, Running) => Step::Halt(Halt::Stop),
        (Steer::Stop, Stopped) | (Steer::Pause, Paused) => Step::Skip(Skipped::Already),
        (Steer::Pause, Queued | Waiting) => Step::Move(Paused),
        (Steer::Pause, Running) => Step::Halt(Halt::Pause),
        (Steer::Pause, Stopped | Failed) => Step::Skip(Skipped::Inactive),
        (Steer::Unpause, Queued | Running | Waiting | Questions | Stopped | Failed) => Step::Skip(Skipped::NotPaused),
        (Steer::Unpause, Paused) | (Steer::Resume, Waiting | Stopped | Failed) => Step::Move(Queued),
        (Steer::Resume, Queued | Running) => Step::Skip(Skipped::Going),
        (Steer::Resume, Paused) => Step::Skip(Skipped::Paused),
        // A task that asks runs nothing, and its open round must be answered before it asks another: a stop would leave
        // the round open under a task that could run and ask again.
        (Steer::Stop | Steer::Pause | Steer::Resume, Questions) => Step::Skip(Skipped::Questions),
    }
}

/// The status of a task whose run ended by itself while `halt` was asked of it, and which `settled` would send to:
/// the human's word wins where it can.
pub(super) const fn after_halt(halt: Halt, settled: TaskStatus) -> TaskStatus {
    match (halt, settled) {
        (Halt::Stop, TaskStatus::Queued | TaskStatus::Waiting | TaskStatus::Paused) => TaskStatus::Stopped,
        (Halt::Pause, TaskStatus::Queued | TaskStatus::Waiting) => TaskStatus::Paused,
        // A task that asks the human, is over or failed keeps what its run made of it: the human still has to answer,
        // and a finished task is not undone.
        (
            Halt::Stop | Halt::Pause,
            TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Stopped
            | TaskStatus::Failed
            | TaskStatus::Done,
        ) => settled,
    }
}

#[cfg(test)]
mod tests;
