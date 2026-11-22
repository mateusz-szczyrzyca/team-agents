//! Whether the result of a worker serves the request it was given.
//!
//! A worker's run ends, and the harness checks what it left: its status line, the files it changed, the request's
//! check and the time it took. [`decide`] reads those facts and answers what the harness does with the result: take it
//! and bring the worker's commit to the requester's branch ([`Verdict::Accept`]), give the request once more to a free
//! worker of another provider ([`Verdict::GiveToAnother`]), or return the step to the requester with every reason
//! ([`Verdict::Return`]). A request that names no path to write is looked up: its result goes to the requester as it
//! always did, and a check that fails still makes it a wrong result.
//!
//! The decision is a pure function of what [`Facts`] says; [`worker_for_second_try`] is the second one, of who may
//! serve the retry.

use crate::ids::AgentName;
use crate::kinds::CliKind;

/// How the worker's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerEnding {
    /// The run ended with `STATUS: DONE`.
    Done,
    /// The run ended with `STATUS: FAILED`.
    Failed,
    /// The run ended without a `STATUS:` line.
    NoStatus,
    /// The run said it could not do the work without an answer from the requester.
    Cannot,
}

/// What the check of the request found in the worker's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    /// The check command exited `0`, or `exists PATH` found the path.
    Passed,
    /// The check command failed, `exists PATH` found nothing, or the path escapes the worktree.
    Failed,
    /// The check ran past its time limit and was stopped.
    TimedOut,
    /// The request has no check; nothing shows the result right or wrong.
    Absent,
}

/// A free worker of another provider than the one whose result is wrong: the retry [`Verdict::GiveToAnother`] takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Elsewhere {
    /// A free worker of another provider waits.
    Free,
    /// Every free worker is of the provider that just failed, or no worker is free at all: the same provider's
    /// worker is the retry, if `[workers] tries` allows one.
    None,
}

/// What the request asks for: files to write, or facts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The request names paths the worker may write: its commit is what the step is.
    Files,
    /// The request names no path: its result is a look-up, given to the requester as it always was.
    Facts,
}

/// What the check and the worker's report show of the request's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// How the worker's run ended.
    pub ending: WorkerEnding,
    /// What the request's check found.
    pub check: CheckOutcome,
    /// Whether the worker changed a file the request's `files` do not name.
    pub changed_outside_files: bool,
    /// Whether the worker's run lasted past `[workers] seconds`; `seconds` is `0` when the run has no limit.
    pub past_seconds: bool,
    /// Whether the request names paths to write, or only asks for facts.
    pub scope: Scope,
    /// How many workers the request has been given to, its first try included.
    pub try_number: u32,
    /// How many tries `[workers] tries` allows; `1` never gives the request to a second worker.
    pub tries: u32,
    /// Whether a free worker of another provider than the one that just failed remains.
    pub elsewhere: Elsewhere,
}

/// Why the result of a worker is wrong: the facts that make it so, in the order the requester reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrong {
    /// The worker's run ended otherwise than `STATUS: DONE`: failed, without a status, or asking for an answer.
    EndedOtherwise(WorkerEnding),
    /// The request's check failed in the worker's worktree.
    CheckFailed,
    /// The request's check ran past its time limit.
    CheckTimedOut,
    /// The worker changed a file outside the request's `files`.
    OutsideFiles,
    /// The worker's run lasted past `[workers] seconds`.
    PastSeconds,
}

/// What the harness does with the result of a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The result serves the request: the worker's commit is brought to the requester's branch — for a request for
    /// facts only there is no commit, and the result is the worker's.
    Accept,
    /// Give the request once more to a free worker of another provider; `wrong` is why this result did not serve.
    GiveToAnother {
        /// Why the result did not serve the request.
        wrong: Vec<Wrong>,
    },
    /// The step goes back to the requester, who does it itself; `wrong` is every reason it did not serve.
    Return {
        /// Why the results did not serve the request.
        wrong: Vec<Wrong>,
    },
}

/// What the harness does with a worker's result.
#[must_use]
pub fn decide(facts: &Facts) -> Verdict {
    let _ = facts;
    Verdict::Accept
}

/// The free worker that serves the second try of a request: one whose `cli` differs from the one that failed, in the
/// configured order, or, when none of another provider is free, a free worker of the same provider.
#[must_use]
pub fn worker_for_second_try<'a>(
    workers: &'a [(&'a AgentName, CliKind)],
    failed: CliKind,
    busy: &[AgentName],
) -> Option<&'a AgentName> {
    let _ = (workers, failed, busy);
    None
}

#[cfg(test)]
mod tests;
