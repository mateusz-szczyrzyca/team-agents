//! Whether the result of a worker serves the request it was given.
//!
//! A worker's run ends, and the harness checks what it left: its status line, the files it changed, the request's
//! check and the time it took. [`decide`] reads those facts and answers what the harness does with the result: take it
//! and bring the worker's commit to the requester's branch ([`Verdict::Accept`]), give the request once more to a free
//! worker of another provider, or of the same one when no other is free ([`Verdict::GiveToAnother`]), or return the
//! step to the requester with every reason ([`Verdict::Return`]). A request that names no path to write is looked up: its result goes to the requester as it
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
    /// The request has no check. Only a request for facts can lack one; for a request that names files nothing then
    /// shows the commit right, and the result is wrong ([`Wrong::NoCheck`]).
    Absent,
}

/// Which free worker, if any, the retry [`Verdict::GiveToAnother`] can go to; [`worker_for_second_try`] finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elsewhere {
    /// A free worker of another provider than the one whose result is wrong waits.
    Free,
    /// Every free worker is of the provider that just failed: the same provider serves the retry, as there is no other.
    SameProvider,
    /// No worker is free at all.
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
    /// Which free worker the retry may go to.
    pub elsewhere: Elsewhere,
}

/// Why the result of a worker is wrong: the facts that make it so, in the order the requester reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrong {
    /// No commit was left for a request that writes files.
    NoCommit,
    /// The worker's run ended otherwise than `STATUS: DONE`: failed, without a status, or asking for an answer.
    EndedOtherwise(WorkerEnding),
    /// The request's check failed in the worker's worktree.
    CheckFailed,
    /// The request's check ran past its time limit.
    CheckTimedOut,
    /// The worker changed a file outside the request's `files`.
    OutsideFiles,
    /// The request names files to write but no check: nothing shows the commit right.
    NoCheck,
    /// The worker's run lasted past `[workers] seconds`.
    PastSeconds,
}

/// What the harness does with the result of a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The result serves the request: the worker's commit is brought to the requester's branch — for a request for
    /// facts only there is no commit, and the result is the worker's.
    Accept,
    /// Give the request once more to a free worker, of another provider when one is free; `wrong` is why this result did not serve.
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
    let mut wrong = Vec::new();
    if facts.ending != WorkerEnding::Done {
        wrong.push(Wrong::EndedOtherwise(facts.ending));
    }
    if facts.changed_outside_files {
        wrong.push(Wrong::OutsideFiles);
    }
    if facts.past_seconds {
        wrong.push(Wrong::PastSeconds);
    }
    // A check may be absent only for a request for facts: a commit that no check shows right is never accepted.
    match (facts.check, facts.scope) {
        (CheckOutcome::Passed, _) | (CheckOutcome::Absent, Scope::Facts) => {}
        (CheckOutcome::Absent, Scope::Files) => wrong.push(Wrong::NoCheck),
        (CheckOutcome::Failed, _) => wrong.push(Wrong::CheckFailed),
        (CheckOutcome::TimedOut, _) => wrong.push(Wrong::CheckTimedOut),
    }
    if wrong.is_empty() {
        return Verdict::Accept;
    }
    let first_try = facts.try_number <= 1;
    let somebody_free = matches!(facts.elsewhere, Elsewhere::Free | Elsewhere::SameProvider);
    if first_try && facts.tries > 1 && somebody_free {
        Verdict::GiveToAnother { wrong }
    } else {
        Verdict::Return { wrong }
    }
}

/// The free worker that serves the second try of a request: one whose `cli` differs from the one that failed, in the
/// configured order, or, when none of another provider is free, a free worker of the same provider.
#[must_use]
pub fn worker_for_second_try<'a>(
    workers: &'a [(&'a AgentName, CliKind)],
    failed: CliKind,
    busy: &[AgentName],
) -> Option<&'a AgentName> {
    let is_free = |name: &AgentName| !busy.contains(name);
    workers
        .iter()
        .find(|(name, cli)| is_free(name) && *cli != failed)
        .or_else(|| workers.iter().find(|(name, cli)| is_free(name) && *cli == failed))
        .map(|(name, _)| *name)
}

#[cfg(test)]
mod tests;
