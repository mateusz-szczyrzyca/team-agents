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
use agent_core::decide::commit_reminder::{CommitLook, ReminderDue, is_reminded, reminder_due};
use agent_core::ids::{AgentName, InvalidName, RunId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_core::state::{InvalidTimestamp, Round, State, Timestamp};
use agent_runtime::agentbus::Outgoing;
use agent_runtime::git::{CommitIdentity, CommitTimeError, Git, GitError, Revision};
use agent_runtime::process::error_chain;
use agent_runtime::team_prompts::RenderTeamTextError;

use super::{Owner, OwnerError};

/// The seconds between two looks at the worktree of one task.
const LOOK_SECONDS: i64 = 60;

/// A task with a run going that is reminded when it is due.
struct Watched {
    /// The repository of the task's team.
    repo: RepoName,
    /// The task.
    task: TaskId,
    /// The run that is going.
    run: RunId,
    /// When the run's CLI was started.
    run_started: Timestamp,
    /// The agent that holds the task.
    agent: AgentName,
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
            Self::Clock { .. }
            | Self::Agent { .. }
            | Self::Worktree { .. }
            | Self::Values { .. }
            | Self::Render { .. }
            | Self::Recipient { .. } => LogEvent::DaemonTaskFailed,
        }
    }
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
fn watched_run(state: &State, task: &TaskId) -> Result<Option<Watched>, LookError> {
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
        if let Err(error) = self.remind_one(&target, now, section.lines, section.minutes).await {
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
        if let Some((reminded_head, reminded)) = self.commit_reminded.get(&target.task)
            && *reminded_head != head
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
            return Ok(());
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
