//! Recovery on start: what a restart left half done in the state.

use agent_core::config::{Limits, RestartSession};
use agent_core::decide::ladder::Trouble;
use agent_core::decide::outcome::{
    AfterNoStatus, ClassifyError, NO_STATUS_RETRIES, after_no_status, consecutive_no_status,
};
use agent_core::ids::{RoundId, TaskId};
use agent_core::kinds::Role;
use agent_core::state::{Member, RenewTaskError, Round, Run, RunEnd, RunError, RunState, State, Timestamp};
use agent_core::status::{InvalidTransition, TaskStatus};

use super::settle::wake_time;

mod findings;
pub(super) use findings::reconcile_findings;

/// What recovery found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Recovery {
    /// Tasks whose runs a restart ended: queued again, or failed when no-status retries are exhausted.
    pub(super) interrupted: Vec<TaskId>,
    /// The interrupted tasks whose persisted no-status retries are exhausted.
    pub(super) exhausted: Vec<TaskId>,
    /// The paused tasks and when each goes on.
    pub(super) wakes: Vec<(Timestamp, TaskId)>,
    /// The tasks whose cut run began a new session of their agent under `[restart] session = "new"`.
    pub(super) renewed: Vec<TaskId>,
    /// The queued tasks whose last runs hung, and how many hangs in a row: the watchdog's count, which a restart must
    /// not reset.
    pub(super) hangs: Vec<(TaskId, u32)>,
}

/// Why the state cannot be recovered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecoveryError {
    /// A task listed in the state cannot be found in it.
    #[error("task {task} is in the state but cannot be found in it")]
    Missing {
        /// The task.
        task: TaskId,
    },
    /// The run of a task cannot be ended.
    #[error("the run of task {task} cannot be ended")]
    Run {
        /// The task.
        task: TaskId,
        /// What the state refused.
        source: RunError,
    },
    /// The status of a task cannot be changed.
    #[error("the status of task {task} cannot be changed")]
    Status {
        /// The task.
        task: TaskId,
        /// What the state refused.
        source: InvalidTransition<TaskStatus>,
    },
    /// The new session of a task whose run was cut cannot be recorded.
    #[error("the new session of task {task} cannot be recorded")]
    Renew {
        /// The task.
        task: TaskId,
        /// What the state refused.
        source: RenewTaskError,
    },
    /// A pause cannot be timed.
    #[error("the pause of task {task} cannot be timed")]
    Pause {
        /// The task.
        task: TaskId,
        /// What the classifier refused.
        source: ClassifyError,
    },
}

/// Ends interrupted runs, queues or fails their tasks within the retry budget, and finds the pauses to wake.
///
/// # Errors
/// [`RecoveryError`] when the state refuses a change or a pause cannot be timed.
#[cfg(test)]
pub(super) fn recover(state: &mut State, now: Timestamp, limits: &Limits) -> Result<Recovery, RecoveryError> {
    recover_with(state, now, limits, RestartSession::Same)
}

/// Ends interrupted runs as `recover` of the tests does, and a run that was going when the daemon went down begins a new session of its task when `restart`
/// and [`after_cut`] say so, before the retries of the session are counted: a cut the daemon caused is no retry. Only a
/// run that recovery itself ends is a cut by the daemon: an earlier end with no exit code may be a signal's.
///
/// # Errors
/// [`RecoveryError`] when the state refuses a change or a pause cannot be timed.
pub(super) fn recover_with(
    state: &mut State,
    now: Timestamp,
    limits: &Limits,
    restart: RestartSession,
) -> Result<Recovery, RecoveryError> {
    let waiting = state.queued_order();
    let mut recovery = Recovery::default();
    for task in all_tasks(state) {
        let role = state
            .find_task(&task)
            .ok_or_else(|| RecoveryError::Missing { task: task.clone() })?
            .role;
        let open_round = open_round_of(state, &task);
        let (_, mut member) = state
            .find_task_mut(&task)
            .ok_or_else(|| RecoveryError::Missing { task: task.clone() })?;
        if member.plan_running_check().is_some() && member.current_run().is_some() {
            // The CLI run already finished and is waiting for its plan command checks or settlement: the owner
            // resumes that settlement on startup instead of marking the CLI run interrupted, while preserving any
            // prior consecutive hang strikes of the session.
            let hangs = trailing_hangs(member.session_runs());
            if hangs > 0 {
                recovery.hangs.push((task.clone(), hangs));
            }
            continue;
        }
        let was_going = member.current_run().is_some();
        let was_running = member.status() == TaskStatus::Running;
        let mut cut_at = None;
        if let Some(started_at) = member.current_run().map(Run::started_at) {
            // No code: the restart ended the process by a signal, or its end was never seen. A clock that was set back
            // since the run started would end it before its start: it ends when it started, at the earliest.
            let ended_at = now.max(started_at);
            member
                .end_run(ended_at, RunEnd::NoStatus { exit_code: None })
                .map_err(|source| RecoveryError::Run {
                    task: task.clone(),
                    source,
                })?;
            cut_at = Some(ended_at);
        }
        if let Some(at) = cut_at
            && cuts_begin_a_session(state, &task, role, restart, open_round.as_ref())
        {
            state
                .begin_holder_session(&task, Trouble::DaemonDown, at)
                .map_err(|source| RecoveryError::Renew {
                    task: task.clone(),
                    source,
                })?;
            recovery.renewed.push(task.clone());
        }
        let (_, mut member) = state
            .find_task_mut(&task)
            .ok_or_else(|| RecoveryError::Missing { task: task.clone() })?;
        if was_running {
            let status = interrupted_status(role, &member);
            if status == TaskStatus::Failed {
                recovery.exhausted.push(task.clone());
            }
            member.transition(status).map_err(|source| RecoveryError::Status {
                task: task.clone(),
                source,
            })?;
        }
        if was_going || was_running {
            recovery.interrupted.push(task.clone());
        }
        let hangs = trailing_hangs(member.session_runs());
        if member.status() == TaskStatus::Queued && hangs > 0 {
            recovery.hangs.push((task.clone(), hangs));
        }
        if member.status() == TaskStatus::Paused
            && let Some(wake) = paused_until(member.runs().last(), limits).map_err(|source| RecoveryError::Pause {
                task: task.clone(),
                source,
            })?
        {
            recovery.wakes.push((wake, task));
        }
    }
    if !recovery.interrupted.is_empty() {
        state.place_waiting_after_interrupted(&waiting);
    }
    Ok(recovery)
}

/// What a restart of the daemon does with the session of a run it cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AfterCut {
    /// The task goes on in a new session of its agent, whose first prompt is the briefing of what the cut run left.
    NewSession,
    /// The task goes on in the session the run was cut in, as the harness did before briefings.
    SameSession,
}

/// What a restart of the daemon does with the session of the last run of `member`, a task of `role`, when `round` is the
/// open round that run worked for (`None` when it worked for none) and `setting` is `[restart] session`. The caller knows
/// that the daemon cut the run: recovery that ended it, or the stop that did. A run that ended with no exit code on its own
/// is no proof, a signal may have killed it.
pub(super) fn after_cut(setting: RestartSession, role: Role, member: &Member, round: Option<&RoundId>) -> AfterCut {
    let cut = matches!(
        member.runs().last().map(Run::state),
        Some(RunState::Ended {
            end: RunEnd::NoStatus { exit_code: None },
            ..
        })
    );
    // A worker is a request of a task and a reviewer keeps its findings file only: neither has a session worth a briefing.
    let works_on_its_own = match role {
        Role::Solo | Role::Coordinator | Role::Member => true,
        Role::Worker | Role::Reviewer => false,
    };
    // A task with no open round has no snapshot to brief from, and one that already awaits a new session is briefed.
    let briefable = round.is_some()
        && matches!(member.status(), TaskStatus::Queued | TaskStatus::Running)
        && !member.awaits_new_session();
    match setting {
        RestartSession::New if cut && works_on_its_own && briefable => AfterCut::NewSession,
        RestartSession::New | RestartSession::Same => AfterCut::SameSession,
    }
}

/// The open round the last run of `task` worked for, if there is one.
pub(super) fn open_round_of(state: &State, task: &TaskId) -> Option<RoundId> {
    let found = state.find_task(task)?;
    found
        .member
        .runs()
        .last()?
        .rounds()
        .iter()
        .rev()
        .find(|round| found.team.round(round).is_some_and(Round::is_open))
        .cloned()
}

/// Whether the run of `task` that recovery just ended begins a new session of the task.
fn cuts_begin_a_session(
    state: &State,
    task: &TaskId,
    role: Role,
    restart: RestartSession,
    open_round: Option<&RoundId>,
) -> bool {
    state
        .find_task(task)
        .is_some_and(|found| after_cut(restart, role, found.member, open_round) == AfterCut::NewSession)
}

/// Keeps interruption recovery queued except when an eligible session has used its entire no-status budget.
pub(super) fn interrupted_status(role: Role, member: &Member) -> TaskStatus {
    // Ask the core decision at the last permitted retry to establish eligibility without changing the shutdown
    // policy of workers, reviewers or tasks without a resumable session.
    let eligible = after_no_status(role, NO_STATUS_RETRIES, member.last_session()) == AfterNoStatus::Resume;
    // The retries are of the session the task is in: those before a renewal were the old session's.
    if eligible && consecutive_no_status(member.session_runs()) > NO_STATUS_RETRIES {
        TaskStatus::Failed
    } else {
        TaskStatus::Queued
    }
}

/// How many of the last `runs` hung in a row. A run a restart interrupted is no strike and does not end the row.
/// Whether a hung run had made progress is not saved: after a restart, every hang of the row counts.
fn trailing_hangs(runs: &[Run]) -> u32 {
    let ends = runs.iter().rev().filter_map(|run| match run.state() {
        RunState::Ended { end, .. } => Some(end),
        RunState::Running => None,
    });
    let hangs = ends
        .filter(|end| !matches!(end, RunEnd::NoStatus { exit_code: None }))
        .take_while(|end| matches!(end, RunEnd::Hung))
        .count();
    u32::try_from(hangs).unwrap_or(u32::MAX)
}

/// When a task whose last run is `last` goes on, if its pause is a provider's limit.
fn paused_until(last: Option<&Run>, limits: &Limits) -> Result<Option<Timestamp>, ClassifyError> {
    match last.map(Run::state) {
        Some(RunState::Ended { at, end }) => wake_time(end, *at, limits),
        Some(RunState::Running) | None => Ok(None),
    }
}

/// Every task of the state: the crew of each team and its reviewers, team by team.
fn all_tasks(state: &State) -> Vec<TaskId> {
    state
        .teams()
        .iter()
        .flat_map(|team| {
            let crew = team.crew().roster().into_iter().map(|(_, member)| member);
            crew.chain(team.reviewers().iter()).map(|member| member.task().clone())
        })
        .collect()
}

#[cfg(test)]
mod tests;
