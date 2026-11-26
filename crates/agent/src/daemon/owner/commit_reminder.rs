//! The reminder to commit: a run that holds many uncommitted lines and has made no commit for a while is told once, on the
//! bus, to commit what is finished. Both bounds of `[commit_reminder]` must be reached, and a commit re-arms the reminder.
//!
//! The daemon looks at the worktree of every task with a run going, at most once a minute. What it keeps between two looks
//! is in memory only: a restart of the daemon re-arms the reminder.

use std::path::PathBuf;
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::config::RepoName;
use agent_core::decide::checks::{FileChange, Lines};
use agent_core::decide::commit_reminder::{CommitLook, CutDue, ReminderDue, cut_due, is_reminded, reminder_due};
use agent_core::ids::{AgentName, InvalidName, RunId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_core::protocol::EventKind;
use agent_core::state::{CommitId, InvalidTimestamp, Round, RunEnd, State, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::Outgoing;
use agent_runtime::git::{CommitIdentity, CommitTimeError, Git, GitError, Revision};
use agent_runtime::process::error_chain;
use agent_runtime::team_prompts::RenderTeamTextError;

use super::test_lock::{self, LockHolder, TestLockError};
use super::{Located, Owner, OwnerError};
use crate::daemon::team::Why;

/// The seconds between two looks at the worktree of one task.
const LOOK_SECONDS: i64 = 60;

/// What a run that ignored the reminder to commit is told when its session is resumed after the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CommitOrder {
    /// The changed lines that stayed uncommitted.
    pub(super) lines: u64,
    /// The whole minutes they stayed so after the reminder.
    pub(super) minutes: u64,
}

/// A task with a run going that is reminded when it is due.
pub(super) struct Watched {
    /// The repository of the task's team.
    repo: RepoName,
    /// The task.
    pub(super) task: TaskId,
    /// The run that is going.
    pub(super) run: RunId,
    /// When the run's CLI was started.
    run_started: Timestamp,
    /// The agent that holds the task.
    pub(super) agent: AgentName,
}

/// Why a look at a worktree for the reminder to commit came to nothing.
#[derive(Debug, thiserror::Error)]
pub(super) enum LookError {
    /// The clock gives a time no timestamp can hold.
    #[error("the clock gives no time the reminder to commit can use")]
    Clock {
        /// What refused the time.
        source: InvalidTimestamp,
    },
    /// The agent of the task is no agent name.
    #[error("the agent of the task has no name the bus knows")]
    Agent {
        /// What refused the name.
        source: InvalidName,
    },
    /// The worktree of the task is no absolute path of text.
    #[error("the worktree {path:?} of the task cannot be used for git")]
    Worktree {
        /// The worktree.
        path: PathBuf,
        /// What refused it, when it is a path of text.
        source: Option<agent_core::config::InvalidValue>,
    },
    /// Git cannot say what the worktree holds.
    #[error("git cannot say what the worktree holds")]
    Git {
        /// What git refused.
        source: GitError,
    },
    /// Git cannot say when the head was committed.
    #[error("git cannot say when the head was committed")]
    CommitTime {
        /// What git refused.
        source: CommitTimeError,
    },
    /// The machine's test lock cannot be read.
    #[error("the test lock cannot be read")]
    TestLock {
        /// What refused it.
        source: TestLockError,
    },
    /// The values of the text of the reminder are refused.
    #[error("the values of the reminder to commit are refused")]
    Values {
        /// What refused them.
        source: VariablesError,
    },
    /// The loader of the texts cannot render the reminder.
    #[error("the reminder to commit cannot be rendered")]
    Render {
        /// What the loader refused.
        source: RenderTeamTextError,
    },
    /// The task has no recipient on the bus.
    #[error("the reminder to commit has no recipient")]
    Recipient {
        /// Why the task has none.
        source: OwnerError,
    },
    /// The bus refused the reminder.
    #[error("the reminder to commit cannot be put on the bus")]
    Bus {
        /// What the bus said.
        source: OwnerError,
    },
}

impl LookError {
    /// The log event the failure is logged under: a bus that is down, a git call that failed, or a task that failed at its
    /// reminder.
    const fn event(&self) -> LogEvent {
        match self {
            Self::Bus { .. } => LogEvent::BusDown,
            Self::Git { .. } | Self::CommitTime { .. } => LogEvent::GitCallFailed,
            Self::TestLock { .. }
            | Self::Clock { .. }
            | Self::Agent { .. }
            | Self::Worktree { .. }
            | Self::Values { .. }
            | Self::Render { .. }
            | Self::Recipient { .. } => LogEvent::DaemonTaskFailed,
        }
    }
}

/// What the look saw of a reminder that may be ignored.
struct Ignored {
    /// The head the run was reminded at and the time of the reminder, if it was reminded.
    reminder: Option<(CommitId, Timestamp)>,
    /// Whether the head moved since the reminder.
    head_moved: bool,
    /// The lines that are uncommitted now.
    lines: u64,
}

/// The lines the changed files differ by: added plus removed, a binary file counting none.
fn changed_lines(changes: &[FileChange]) -> u64 {
    changes
        .iter()
        .map(|change| match change.lines {
            Lines::Text { added, removed } => u64::from(added) + u64::from(removed),
            Lines::Binary => 0,
        })
        .sum()
}

/// The whole minutes from `earlier` to `later`; none when `later` is not after it.
fn minutes_between(earlier: Timestamp, later: Timestamp) -> u64 {
    let minutes = (later.as_utc() - earlier.as_utc()).whole_minutes();
    if minutes <= 0 { 0 } else { minutes.unsigned_abs() }
}

/// The run of `task` that is to be looked at for the reminder: the run going of a solo agent, a coordinator or a member
/// that works for a round that is open. None for a worker, a reviewer, a task with no run going, and a run that names no
/// round or only rounds that are closed.
///
/// # Errors
/// [`LookError::Agent`] when the agent of the task is no agent name the bus knows.
pub(super) fn watched_run(state: &State, task: &TaskId) -> Result<Option<Watched>, LookError> {
    let Some(found) = state.find_task(task) else {
        return Ok(None);
    };
    if !is_reminded(found.role) {
        return Ok(None);
    }
    let Some(run) = found.member.current_run() else {
        return Ok(None);
    };
    let in_open_round = run
        .rounds()
        .iter()
        .any(|id| found.team.round(id).is_some_and(Round::is_open));
    if !in_open_round {
        return Ok(None);
    }
    let agent = found
        .member
        .agent()
        .name()
        .parse()
        .map_err(|source| LookError::Agent { source })?;
    Ok(Some(Watched {
        repo: found.team.repo().clone(),
        task: task.clone(),
        run: run.id().clone(),
        run_started: run.started_at(),
        agent,
    }))
}

impl Owner {
    /// The tasks of `team` that have a run going: the ones [`Owner::remind_to_commit`] may have to look at.
    pub(super) fn tasks_with_a_run(&self, team: &TeamId) -> Vec<TaskId> {
        self.parts.state.team(team).map_or_else(Vec::new, |found| {
            found
                .crew()
                .roster()
                .into_iter()
                .filter(|(_, member)| member.current_run().is_some())
                .map(|(_, member)| member.task().clone())
                .collect()
        })
    }

    /// Looks at the worktree of `task` when it has a run going for an open round and reminds the run to commit when it is
    /// due. A look that fails is logged and the run goes on.
    ///
    /// What is kept for the task — the head that was reminded, and the time of the last look — is kept until a look sees
    /// another head: the end of a run does not re-arm the reminder, a commit does.
    ///
    /// The clock is read once per call, that is once per look at a task, and only after the task is found to be watched.
    pub(super) async fn remind_to_commit(&mut self, task: &TaskId) {
        let section = *self.parts.settings.config.commit_reminder();
        if section.lines == 0 || section.minutes == 0 {
            return;
        }
        let target = match watched_run(&self.parts.state, task) {
            Ok(Some(target)) => target,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    task = %task,
                    operation = "commit_reminder",
                    error = %error_chain(&error),
                    "the agent of a task has no name the bus knows"
                );
                return;
            }
        };
        let now = match Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(source) => {
                tracing::warn!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    task = %task,
                    operation = "commit_reminder",
                    error = %error_chain(&LookError::Clock { source }),
                    "the worktree was not looked at for the reminder to commit"
                );
                return;
            }
        };
        if let Err(error) = self
            .remind_one(&target, now, section.lines, section.minutes, section.cut_after)
            .await
        {
            tracing::warn!(
                event = error.event().as_str(),
                task = %target.task,
                run = %target.run,
                operation = "commit_reminder",
                error = %error_chain(&error),
                "the worktree was not looked at for the reminder to commit"
            );
        }
    }

    /// Looks at the worktree of one task and reminds its run when the bounds are reached.
    ///
    /// # Errors
    /// [`LookError`] when the worktree cannot be read, or the reminder cannot be made or put on the bus.
    async fn remind_one(
        &mut self,
        target: &Watched,
        now: Timestamp,
        lines: u32,
        minutes: u32,
        cut_after: u32,
    ) -> Result<(), LookError> {
        // The idle time is counted from the later of the last commit and the start of the run, so it cannot reach `minutes`
        // before the run is that old: a young run needs no look at its worktree.
        if minutes_between(target.run_started, now) < u64::from(minutes) {
            return Ok(());
        }
        if self
            .commit_looked_at
            .get(&target.task)
            .is_some_and(|last| (now.as_utc() - last.as_utc()).whole_seconds() < LOOK_SECONDS)
        {
            return Ok(());
        }
        self.commit_looked_at.insert(target.task.clone(), now);
        let git = self.commit_git(target)?;
        let head = git
            .rev_parse(&Revision::Head)
            .await
            .map_err(|source| LookError::Git { source })?;
        let reminder = self.commit_reminded.get(&target.task).cloned();
        let head_moved = reminder
            .as_ref()
            .is_some_and(|(reminded_head, _)| *reminded_head != head);
        if let Some((_, reminded)) = &reminder
            && head_moved
        {
            tracing::info!(
                event = LogEvent::CommitFollowed.as_str(),
                task = %target.task,
                run = %target.run,
                agent = %target.agent,
                minutes = minutes_between(*reminded, now),
                "a run reminded to commit has committed"
            );
            self.commit_reminded.remove(&target.task);
            self.commit_cut.remove(&target.task);
        }
        let last_commit = git
            .commit_time(&Revision::Head)
            .await
            .map_err(|source| LookError::CommitTime { source })?;
        let changes = git
            .uncommitted_changes()
            .await
            .map_err(|source| LookError::Git { source })?;
        let look = CommitLook {
            uncommitted: changed_lines(&changes),
            last_commit,
            run_started: target.run_started,
            now,
            head: head.clone(),
            reminded_at: self
                .commit_reminded
                .get(&target.task)
                .map(|(reminded, _)| reminded.clone()),
        };
        let ReminderDue::Due {
            lines: due_lines,
            minutes: due_minutes,
        } = reminder_due(&look, lines, minutes)
        else {
            let ignored = Ignored {
                reminder,
                head_moved,
                lines: look.uncommitted,
            };
            return self.cut_if_ignored(target, now, cut_after, ignored);
        };
        self.put_commit_reminder(target, due_lines, due_minutes).await?;
        self.commit_reminded.insert(target.task.clone(), (head, now));
        tracing::info!(
            event = LogEvent::CommitReminded.as_str(),
            task = %target.task,
            run = %target.run,
            agent = %target.agent,
            lines = due_lines,
            minutes = due_minutes,
            "a run with uncommitted work was reminded to commit"
        );
        Ok(())
    }

    /// Cuts the run of `target` when it was reminded, ignored the reminder for `cut_after` minutes and no
    /// gate or test run of its worktree is going.
    ///
    /// # Errors
    /// [`LookError::TestLock`] when the test lock cannot be read.
    fn cut_if_ignored(
        &mut self,
        target: &Watched,
        now: Timestamp,
        cut_after: u32,
        ignored: Ignored,
    ) -> Result<(), LookError> {
        let Some(reminded) = ignored.reminder.map(|(_, at)| at) else {
            return Ok(());
        };
        let busy = cut_after != 0 && self.worktree_busy(target)?;
        let cut_already = self.commit_cut.contains(&target.task);
        let CutDue::Cut { minutes } = cut_due(reminded, now, cut_after, ignored.head_moved, busy, cut_already) else {
            return Ok(());
        };
        self.commit_cut.insert(target.task.clone());
        self.cut_for_commit(target, ignored.lines, minutes);
        Ok(())
    }

    /// Whether a gate of `agent gate` runs for the task, or a test run started in its worktree holds the machine's test
    /// lock: the run is waiting for them, not ignoring the reminder.
    ///
    /// # Errors
    /// [`LookError::TestLock`] when the test lock cannot be read.
    fn worktree_busy(&self, target: &Watched) -> Result<bool, LookError> {
        if self.gate_stops.contains_key(&target.task) {
            return Ok(true);
        }
        let worktree = self.parts.store.home().worktree_dir(&target.repo, &target.task);
        let holder =
            test_lock::holder_of(&self.test_lock_file, &worktree).map_err(|source| LookError::TestLock { source })?;
        Ok(match holder {
            LockHolder::Worktree => true,
            LockHolder::Nobody | LockHolder::Elsewhere => false,
        })
    }

    /// Stops the run of `target`, as a stop of the human does but without a halt: the end of the run resumes its session
    /// with the order to commit (see [`Owner::resume_after_cut`]).
    fn cut_for_commit(&mut self, target: &Watched, lines: u64, minutes: u64) {
        self.commit_cuts
            .insert(target.task.clone(), CommitOrder { lines, minutes });
        if let Some(cancel) = self.cancels.get(&target.task) {
            cancel.cancel();
        }
        tracing::info!(
            event = LogEvent::CommitCut.as_str(),
            task = %target.task,
            run = %target.run,
            agent = %target.agent,
            lines,
            minutes,
            "a run that ignored the reminder to commit was cut"
        );
    }

    /// Ends the run `order` was given for and queues the task to resume in its session with the order: the run ended as a
    /// stop, which no retry, turn count, rejection or replacement counts; what the cut left in the worktree is
    /// snapshotted first.
    ///
    /// # Errors
    /// [`OwnerError`] when the run cannot be ended, the task moved or the state saved.
    pub(super) async fn resume_after_cut(
        &mut self,
        task: &TaskId,
        located: &Located,
        now: Timestamp,
        order: CommitOrder,
        mut events: Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        let cut = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.current_run())
            .map(|run| run.id().clone())
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        self.end_run(task, &located.team, now, RunEnd::Stopped, false, None, &mut events)?;
        // The clock of the round goes on through this cut (`[limits] round_minutes`): only a stop of the human starts it
        // again.
        self.parts.state.record_commit_cut(task.clone(), cut);
        self.reasons.insert(
            task.clone(),
            Why::CommitOrder {
                lines: order.lines,
                minutes: order.minutes,
            },
        );
        self.move_to(task, &located.team, TaskStatus::Queued, &mut events)?;
        self.commit(task, located, now, events, None).await?;
        self.snapshot_interrupted(std::slice::from_ref(task), now).await;
        // The session goes on: nobody is briefed from the snapshot, and a later new session takes its own.
        self.cut_snapshots.remove(task);
        Ok(())
    }

    /// Drops the order to commit of `task`, cut or not yet resumed: its task was stopped or paused.
    pub(super) fn drop_commit_order(&mut self, task: &TaskId) {
        self.commit_cuts.remove(task);
        if matches!(self.reasons.get(task), Some(Why::CommitOrder { .. })) {
            self.reasons.remove(task);
        }
    }

    /// Git in the worktree of the task, committing as its agent.
    fn commit_git(&self, target: &Watched) -> Result<Git, LookError> {
        let path = self.parts.store.home().worktree_dir(&target.repo, &target.task);
        let text = path.to_str().ok_or_else(|| LookError::Worktree {
            path: path.clone(),
            source: None,
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| LookError::Worktree {
            path: path.clone(),
            source: Some(source),
        })?;
        Ok(Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), &target.agent),
        ))
    }

    /// Puts the reminder for `lines` uncommitted lines and `minutes` minutes without a commit on the bus for the run.
    async fn put_commit_reminder(&self, target: &Watched, lines: u64, minutes: u64) -> Result<(), LookError> {
        let values = Variables::new()
            .with("lines", lines.to_string())
            .and_then(|values| values.with("minutes", minutes.to_string()))
            .map_err(|source| LookError::Values { source })?;
        let text = self
            .parts
            .team_texts
            .render(TeamPrompt::CommitRemind, &values)
            .map_err(|source| LookError::Render { source })?;
        let recipient = self
            .recipient_of(&target.task)
            .map_err(|source| LookError::Recipient { source })?;
        self.send_from_harness(&recipient, &Outgoing::Chat { text })
            .await
            .map_err(|source| LookError::Bus { source })
    }
}

#[cfg(test)]
pub(super) mod tests;
