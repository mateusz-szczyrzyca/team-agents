//! The checkpoints of a round: the state of a holder's work after each step, with a snapshot of its worktree.
//!
//! A step is queued where it happens ([`Owner::queue_checkpoint`] knows the facts of the state at that moment) and taken at
//! the top of the next turn of the owner's loop ([`Owner::flush_checkpoints`]): the snapshot of the worktree is written,
//! then the record, then the history is flushed, before the owner acts on anything else. A checkpoint that cannot be taken
//! is logged as `checkpoint_failed` and never stops the round.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use agent_core::history::{CheckpointCause, CheckpointJob, EventRecord, HistoryEvent, Record};
use agent_core::ids::{ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::{Role, Tier};
use agent_core::log::LogEvent;
use agent_core::state::{CommitId, ItemPhase, PhaseKind, PrNumber, Round, Timestamp};
use agent_runtime::git::Revision;
use agent_runtime::history::Reader;
use agent_runtime::process::error_chain;
use agent_runtime::snapshot::{prune_snapshots, take_checkpoint_snapshot};

use super::Owner;
use super::commit_reminder::watched_run;
use super::snapshots::validate_worktree_path;

/// The seconds between two looks at the worktree of one task for a new commit.
const LOOK_SECONDS: i64 = 60;

/// What the state said of a round when a step was queued.
#[derive(Debug, Clone)]
pub(super) struct Facts {
    /// The item of the round, when it has one.
    item: Option<ItemId>,
    /// The holder whose worktree the checkpoint keeps.
    task: TaskId,
    /// The tier of the holder.
    tier: Tier,
    /// The pull request of the round, when there is one.
    pull_request: Option<PrNumber>,
    /// Whether the pull request is a draft.
    draft: bool,
    /// The phase of the item.
    phase: PhaseKind,
    /// The review pass; `None` before the first review was asked.
    review_pass: Option<u32>,
    /// The fix pass, when the team is fixing.
    fix_pass: Option<u32>,
    /// The worktree of the holder.
    worktree: PathBuf,
}

/// A step waiting for its snapshot.
#[derive(Debug, Clone)]
pub(super) enum Queued {
    /// A checkpoint to take.
    Take {
        /// The team of the round.
        team: TeamId,
        /// The round.
        round: RoundId,
        /// The step it follows.
        cause: CheckpointCause,
        /// When the step happened.
        at: Timestamp,
        /// What the state said then.
        facts: Box<Facts>,
    },
    /// The snapshot references of a round that ended are to be removed.
    Prune {
        /// The team of the round.
        team: TeamId,
        /// The round.
        round: RoundId,
        /// The worktrees of the team that may still exist: any of them reaches the repository.
        worktrees: Vec<PathBuf>,
    },
}

/// What the last checkpoint of a task kept.
#[derive(Debug, Clone, Default)]
pub(super) struct LastCheckpoint {
    /// The tip of the worktree's branch then.
    head: Option<CommitId>,
    /// The reference of the snapshot, when one was taken.
    snapshot: Option<String>,
}

/// What the owner keeps of checkpoints between two turns of its loop.
#[derive(Debug, Default)]
pub(super) struct Checkpoints {
    /// The steps waiting for their snapshot, oldest first. Guarded because steps are queued from `&self` methods.
    queued: Mutex<Vec<Queued>>,
    /// The rounds whose pull request is a draft: opened and not yet marked ready.
    drafts: Mutex<HashSet<RoundId>>,
    /// What the last checkpoint of each task kept; the snapshot of the next one is compared with it.
    last: HashMap<TaskId, LastCheckpoint>,
    /// When each task was last looked at for a new commit.
    looked_at: HashMap<TaskId, Timestamp>,
    /// The rounds that have a checkpoint of their opening, queued or in the history.
    opened: HashSet<RoundId>,
}

/// Locks `mutex`; a lock left poisoned by a panic still guards a consistent list, as every change of it is one call.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pull request an event is about, for the events that open or move one.
fn event_pull_request(event: &HistoryEvent) -> Option<PrNumber> {
    if let HistoryEvent::PullOpened { number, .. } = event {
        return Some(*number);
    }
    if let HistoryEvent::PullPushed { pr, .. } | HistoryEvent::PullReady { pr, .. } = event {
        return Some(*pr);
    }
    None
}

/// What a record of the history of a round means for the checkpoints.
#[derive(Debug, Clone)]
pub(super) struct Step {
    /// The checkpoint that follows the record, when one does.
    cause: Option<CheckpointCause>,
    /// The pull request the record is about, when it says which.
    pull_request: Option<PrNumber>,
    /// The round the record closes, when it closes one.
    closed: Option<RoundId>,
}

impl Step {
    /// What `event` means for the checkpoints.
    pub(super) fn of(event: &HistoryEvent) -> Self {
        Self {
            cause: event.checkpoint_cause(),
            pull_request: event_pull_request(event),
            closed: if let HistoryEvent::RoundClosed { round, .. } = event {
                Some(round.clone())
            } else {
                None
            },
        }
    }
}

impl Owner {
    /// Queues the checkpoints that follow the record of the event `step` was made of, and the removal of the snapshots of
    /// a round that the event closed. Called by the one function that writes the history of a round.
    pub(super) fn checkpoints_after(&self, at: Timestamp, team: &TeamId, rounds: &[RoundId], step: &Step) {
        if let Some(round) = &step.closed {
            self.queue_prune(team, round);
        }
        let Some(cause) = step.cause else {
            return;
        };
        for round in rounds {
            self.queue_checkpoint(at, team, round, cause, None, step.pull_request);
        }
    }

    /// Queues the checkpoint of `cause` for `round` of `team`: of `holder` when the step is one of its own, else of the
    /// holder of the round's item. A round the state does not know, or one with no holder, has none.
    pub(super) fn queue_checkpoint(
        &self,
        at: Timestamp,
        team: &TeamId,
        round: &RoundId,
        cause: CheckpointCause,
        holder: Option<&TaskId>,
        pull_request: Option<PrNumber>,
    ) {
        if self.history.is_disabled() {
            return;
        }
        let draft = {
            let mut drafts = locked(&self.checkpoints.drafts);
            match cause {
                CheckpointCause::PullOpened => {
                    drafts.insert(round.clone());
                }
                CheckpointCause::PullReady | CheckpointCause::PullMerged | CheckpointCause::PullClosed => {
                    drafts.remove(round);
                }
                CheckpointCause::RoundOpened
                | CheckpointCause::Commit
                | CheckpointCause::PullPushed
                | CheckpointCause::Checked
                | CheckpointCause::SplitDecided
                | CheckpointCause::ReviewSubmitted
                | CheckpointCause::FixPassStarted
                | CheckpointCause::FindingAnswered
                | CheckpointCause::FindingJudged
                | CheckpointCause::WaitsForHuman
                | CheckpointCause::QuestionAsked
                | CheckpointCause::QuestionAnswered
                | CheckpointCause::PullApproved
                | CheckpointCause::MergeRefused
                | CheckpointCause::PullHandedOver
                | CheckpointCause::RunEnded
                | CheckpointCause::RoundOverdue
                | CheckpointCause::HolderChanged
                | CheckpointCause::DaemonStarted => {}
            }
            drafts.contains(round)
        };
        let Some(facts) = self.checkpoint_facts(team, round, holder, pull_request, draft) else {
            return;
        };
        locked(&self.checkpoints.queued).push(Queued::Take {
            team: team.clone(),
            round: round.clone(),
            cause,
            at,
            facts: Box::new(facts),
        });
    }

    /// What the state says of `round` of `team` for a checkpoint of `holder`, or of the holder of the round's item.
    fn checkpoint_facts(
        &self,
        team_id: &TeamId,
        round_id: &RoundId,
        holder: Option<&TaskId>,
        pull_request: Option<PrNumber>,
        draft: bool,
    ) -> Option<Facts> {
        let team = self.parts.state.team(team_id)?;
        let round = team.round(round_id)?;
        let item = round.item();
        let progress = item.and_then(|id| team.item(id));
        let phase = progress.map(agent_core::state::ItemProgress::phase);
        let assignee = phase.and_then(|phase| match phase {
            ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => Some(assignee.as_str()),
            ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                None
            }
        });
        let roster = team.crew().roster();
        let (_, member) = holder
            .and_then(|task| roster.iter().find(|(_, member)| member.task() == task))
            .or_else(|| {
                roster
                    .iter()
                    .find(|(_, member)| Some(member.agent().name()) == assignee)
            })
            .or_else(|| {
                roster
                    .iter()
                    .find(|(role, _)| matches!(role, Role::Solo | Role::Coordinator))
            })
            .or_else(|| roster.first())?;
        let in_pull_request = phase.and_then(|phase| match phase {
            ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. } => {
                Some(pr.number)
            }
            ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => None,
        });
        let kind = phase.map_or(PhaseKind::InProgress, ItemPhase::kind);
        let reviewed = matches!(kind, PhaseKind::InReview | PhaseKind::Fixing | PhaseKind::Approved);
        Some(Facts {
            item,
            task: member.task().clone(),
            tier: member.agent().tier(),
            pull_request: pull_request.or(in_pull_request),
            draft,
            phase: kind,
            review_pass: progress
                .filter(|_| reviewed)
                .and_then(|progress| u32::try_from(progress.completed_review_passes()).ok()),
            fix_pass: progress
                .filter(|_| kind == PhaseKind::Fixing)
                .and_then(|progress| u32::try_from(progress.fix_passes().len()).ok()),
            worktree: self.parts.store.home().worktree_dir(team.repo(), member.task()),
        })
    }

    /// Queues the removal of the snapshot references of `round`, which has ended.
    fn queue_prune(&self, team_id: &TeamId, round: &RoundId) {
        let Some(team) = self.parts.state.team(team_id) else {
            return;
        };
        let home = self.parts.store.home();
        let worktrees = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| home.worktree_dir(team.repo(), member.task()))
            .collect();
        locked(&self.checkpoints.queued).push(Queued::Prune {
            team: team_id.clone(),
            round: round.clone(),
            worktrees,
        });
    }

    /// Takes the queued checkpoints and removes the snapshots of the rounds that ended, in the order of the steps, and
    /// waits until the history has them.
    pub(super) async fn flush_checkpoints(&mut self) {
        let queued = std::mem::take(&mut *locked(&self.checkpoints.queued));
        if queued.is_empty() {
            return;
        }
        for step in queued {
            match step {
                Queued::Take {
                    team,
                    round,
                    cause,
                    at,
                    facts,
                } => self.take_checkpoint(&team, &round, cause, at, *facts).await,
                Queued::Prune { team, round, worktrees } => {
                    // The records of the round are read from the history: everything before is written first.
                    self.history.flush();
                    self.prune_round(&team, &round, &worktrees).await;
                }
            }
        }
        // The next step of a round is acted on only when this one is in `history.db`; the wait is a few milliseconds of a
        // writer thread that is already running.
        self.history.flush();
    }

    /// Takes the snapshot of the holder's worktree and records the checkpoint; a snapshot that cannot be taken leaves a
    /// checkpoint without one.
    async fn take_checkpoint(
        &mut self,
        team: &TeamId,
        round: &RoundId,
        cause: CheckpointCause,
        at: Timestamp,
        facts: Facts,
    ) {
        let previous = self
            .checkpoints
            .last
            .get(&facts.task)
            .and_then(|last| last.snapshot.clone());
        let taken = match validate_worktree_path(&facts.worktree) {
            Ok(worktree) => take_checkpoint_snapshot(&worktree, round, &facts.task, previous.as_deref())
                .await
                .map_err(|error| error_chain(&error)),
            Err(cause) => Err(cause),
        };
        if cause == CheckpointCause::DaemonStarted
            && let Ok(taken) = &taken
            && previous.as_deref() == Some(taken.reference.as_str())
        {
            // The worktree is as the last checkpoint kept it: a start of the daemon changed nothing to keep.
            return;
        }
        let (head, snapshot) = match taken {
            Ok(taken) => {
                self.checkpoints.last.insert(
                    facts.task.clone(),
                    LastCheckpoint {
                        head: Some(taken.snapshot.head.clone()),
                        snapshot: Some(taken.reference.clone()),
                    },
                );
                (Some(taken.snapshot.head), Some(taken.reference))
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::CheckpointFailed.as_str(),
                    team = %team,
                    round = %round,
                    task = %facts.task,
                    cause = %cause,
                    error = %error,
                    "a checkpoint has no snapshot of the worktree: the round goes on"
                );
                (None, None)
            }
        };
        let event = HistoryEvent::Checkpoint {
            round: round.clone(),
            item: facts.item,
            task: facts.task,
            job: CheckpointJob::Item,
            tier: facts.tier,
            cause,
            pull_request: facts.pull_request,
            head,
            draft: facts.draft,
            phase: facts.phase,
            review_pass: facts.review_pass,
            fix_pass: facts.fix_pass,
            snapshot,
            content: None,
        };
        self.history.record(Record::Event(Box::new(EventRecord {
            at,
            team: team.clone(),
            rounds: vec![round.clone()],
            event,
        })));
    }

    /// Removes every snapshot reference the checkpoints of `round` kept, and forgets them.
    async fn prune_round(&mut self, team: &TeamId, round: &RoundId, worktrees: &[PathBuf]) {
        let history_file = self.parts.store.home().history_file();
        let kept = match Reader::open(&history_file).and_then(|reader| reader.checkpoints_of_round(round)) {
            Ok(kept) => kept,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::CheckpointFailed.as_str(),
                    team = %team,
                    round = %round,
                    error = %error_chain(&error),
                    "the snapshots of a round that ended cannot be listed: they stay"
                );
                return;
            }
        };
        let mut references: Vec<String> = kept.into_iter().filter_map(|checkpoint| checkpoint.snapshot).collect();
        references.sort();
        references.dedup();
        if references.is_empty() {
            return;
        }
        self.checkpoints
            .last
            .retain(|_, last| last.snapshot.as_ref().is_none_or(|kept| !references.contains(kept)));
        let Some(worktree) = worktrees.iter().find(|path| path.exists()) else {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                team = %team,
                round = %round,
                "no worktree of the team is left to reach the repository: the snapshots of a round that ended stay"
            );
            return;
        };
        let pruned = match validate_worktree_path(worktree) {
            Ok(worktree) => prune_snapshots(&worktree, round, &references)
                .await
                .map_err(|error| error_chain(&error)),
            Err(cause) => Err(cause),
        };
        if let Err(error) = pruned {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                team = %team,
                round = %round,
                error = %error,
                "the snapshots of a round that ended cannot be removed: they stay"
            );
        }
    }

    /// The team and the open round `task` works for: the newest open round of its current run, else of its last run, else
    /// the only open round of its team. `None` when the task is in no open round.
    pub(super) fn checkpoint_round_of(&self, task: &TaskId) -> Option<(TeamId, RoundId)> {
        let found = self.parts.state.find_task(task)?;
        let open = |id: &&RoundId| found.team.round(id).is_some_and(Round::is_open);
        let run = found.member.current_run().or_else(|| found.member.runs().last());
        let round = run
            .and_then(|run| run.rounds().iter().rev().find(open))
            .cloned()
            .or_else(|| {
                let mut opens = found.team.open_rounds();
                let only = opens.next()?;
                opens.next().is_none().then(|| only.id().clone())
            })?;
        Some((found.team.id().clone(), round))
    }

    /// Queues the checkpoint of `cause` for `task`, in the open round it works for; a task in none has no checkpoint.
    pub(super) fn queue_for_task(&self, task: &TaskId, cause: CheckpointCause) {
        let Some((team, round)) = self.checkpoint_round_of(task) else {
            return;
        };
        let Ok(at) = Timestamp::now(&self.parts.clock) else {
            return;
        };
        self.queue_checkpoint(at, &team, &round, cause, Some(task), None);
    }

    /// Takes the checkpoint of `cause` for `task` now, with the ones queued before it: for a step the owner is about to
    /// act on at once, where the next turn of its loop is too late.
    pub(super) async fn checkpoint_now(&mut self, task: &TaskId, cause: CheckpointCause) {
        self.queue_for_task(task, cause);
        self.flush_checkpoints().await;
    }

    /// Takes the checkpoint of the opening of the round `task` is about to work in, once per round, before its run starts.
    pub(super) async fn checkpoint_round_opened(&mut self, task: &TaskId) {
        let Some((_, round)) = self.checkpoint_round_of(task) else {
            return;
        };
        if self.checkpoints.opened.insert(round) {
            self.checkpoint_now(task, CheckpointCause::RoundOpened).await;
        }
    }

    /// Reads the checkpoints the history already has for the open rounds, so that the first look and the first snapshot
    /// after a start of the daemon compare with them, then takes a `daemon_started` checkpoint of each task of an open
    /// round whose worktree is not as its last checkpoint kept it.
    pub(super) async fn checkpoint_daemon_started(&mut self) {
        let history_file = self.parts.store.home().history_file();
        let mut tasks = Vec::new();
        for team in self.parts.state.teams() {
            for round in team.open_rounds() {
                let kept = match Reader::open(&history_file).and_then(|reader| reader.checkpoints_of_round(round.id()))
                {
                    Ok(kept) => kept,
                    Err(error) => {
                        tracing::warn!(
                            event = LogEvent::CheckpointFailed.as_str(),
                            team = %team.id(),
                            round = %round.id(),
                            error = %error_chain(&error),
                            "the checkpoints of an open round cannot be read at the start of the daemon"
                        );
                        continue;
                    }
                };
                if !kept.is_empty() {
                    self.checkpoints.opened.insert(round.id().clone());
                }
                for checkpoint in kept {
                    self.checkpoints.last.insert(
                        checkpoint.task,
                        LastCheckpoint {
                            head: checkpoint.head,
                            snapshot: checkpoint.snapshot,
                        },
                    );
                }
            }
            tasks.extend(
                team.crew()
                    .roster()
                    .into_iter()
                    .map(|(_, member)| member.task().clone()),
            );
        }
        let home = self.parts.store.home().clone();
        for task in tasks {
            let Some(repo) = self.parts.state.find_task(&task).map(|found| found.team.repo().clone()) else {
                continue;
            };
            if home.worktree_dir(&repo, &task).exists() {
                self.queue_for_task(&task, CheckpointCause::DaemonStarted);
            }
        }
        self.flush_checkpoints().await;
    }

    /// Looks at the worktree of `task`, when it has a run going for an open round, at most once a minute, and queues a
    /// `commit` checkpoint when its head is not the head of the task's last checkpoint. Several commits in one minute are
    /// one checkpoint; a task with no checkpoint yet is only noted.
    pub(super) async fn look_for_commits(&mut self, task: &TaskId) {
        let Ok(Some(target)) = watched_run(&self.parts.state, task) else {
            return;
        };
        let Some((team, round)) = self.checkpoint_round_of(task) else {
            return;
        };
        let Ok(now) = Timestamp::now(&self.parts.clock) else {
            return;
        };
        if self
            .checkpoints
            .looked_at
            .get(task)
            .is_some_and(|last| (now.as_utc() - last.as_utc()).whole_seconds() < LOOK_SECONDS)
        {
            return;
        }
        self.checkpoints.looked_at.insert(task.clone(), now);
        let head = match self.commit_git(&target) {
            Ok(git) => git
                .rev_parse(&Revision::Head)
                .await
                .map_err(|error| error_chain(&error)),
            Err(error) => Err(error_chain(&error)),
        };
        let head = match head {
            Ok(head) => head,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %task,
                    operation = "checkpoint_look",
                    error = %error,
                    "the worktree was not looked at for a new commit"
                );
                return;
            }
        };
        match self.checkpoints.last.get(task).map(|last| last.head.as_ref()) {
            Some(Some(last)) if *last != head => {
                self.queue_checkpoint(now, &team, &round, CheckpointCause::Commit, Some(task), None);
            }
            Some(Some(_)) => {}
            Some(None) | None => {
                self.checkpoints.last.entry(task.clone()).or_default().head = Some(head);
            }
        }
    }
}

#[cfg(test)]
mod tests;
