//! The checkpoints of a round: the state of a holder's work after each step, with a snapshot of its worktree.
//!
//! A step is queued where it happens ([`Owner::queue_checkpoint`] knows the facts of the state at that moment) and taken at
//! the top of the next turn of the owner's loop ([`Owner::flush_checkpoints`]): the snapshot of the worktree is written,
//! then the record, then the history is flushed, before the owner acts on anything else. A checkpoint that cannot be taken
//! is logged as `checkpoint_failed` and never stops the round.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use agent_core::history::{CheckpointCause, CheckpointJob, EventRecord, HistoryEvent, Record};
use agent_core::ids::{ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::{Role, Tier};
use agent_core::log::LogEvent;
use agent_core::state::{CommitId, Completion, ItemPhase, PhaseKind, PrNumber, Round, RoundEnd, Timestamp};
use agent_runtime::git::Revision;
use agent_runtime::history::{Flush, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::snapshot::{prune_snapshots, take_checkpoint_snapshot};

use super::Owner;
use super::commit_reminder::watched_run;
use super::snapshots::validate_worktree_path;

/// The seconds between two looks at the worktree of one task for a new commit.
const LOOK_SECONDS: i64 = 60;

/// How long the owner waits for the writer of the history to have a checkpoint's record, while the writer keeps up: a
/// few milliseconds, as the writer is a thread that is already running.
const FLUSH_WAIT: Duration = Duration::from_secs(2);

/// How long the owner waits once the writer has not answered in [`FLUSH_WAIT`]: only long enough to see that it is back.
const STALLED_FLUSH_WAIT: Duration = Duration::from_millis(20);

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

/// What the owner knows of the pull request of a round from its checkpoints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PullState {
    /// The number of the pull request, once it is known.
    number: Option<PrNumber>,
    /// Whether the pull request is a draft: opened and not yet marked ready.
    draft: bool,
}

/// What the owner keeps of checkpoints between two turns of its loop.
#[derive(Debug, Default)]
pub(super) struct Checkpoints {
    /// The steps waiting for their snapshot, oldest first. Guarded because steps are queued from `&self` methods.
    queued: Mutex<Vec<Queued>>,
    /// The pull request of each round that has one, and whether it is still a draft; read back from the history when the
    /// daemon starts. Guarded because steps are queued from `&self` methods.
    pulls: Mutex<HashMap<RoundId, PullState>>,
    /// What the last checkpoint of each task in each round kept; the snapshot of the next one is compared with it. A
    /// reference of one round is never the base of the snapshot of another.
    last: HashMap<(RoundId, TaskId), LastCheckpoint>,
    /// When each task was last looked at for a new commit.
    looked_at: HashMap<TaskId, Timestamp>,
    /// The rounds that have a checkpoint of their opening, queued or in the history.
    opened: HashSet<RoundId>,
    /// Whether the last wait for the history's writer ended without its answer: the writer is stuck or far behind, and the
    /// owner does not wait for it longer than [`STALLED_FLUSH_WAIT`] until it answers again.
    stalled: bool,
}

/// Locks `mutex`; a lock left poisoned by a panic still guards a consistent list, as every change of it is one call.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pull request an event is about, for the events that carry its number.
fn event_pull_request(event: &HistoryEvent) -> Option<PrNumber> {
    if let HistoryEvent::PullOpened { number, .. } = event {
        return Some(*number);
    }
    if let HistoryEvent::PullPushed { pr, .. }
    | HistoryEvent::PullReady { pr, .. }
    | HistoryEvent::PullMerged { pr, .. }
    | HistoryEvent::PullHandedOver { pr, .. }
    | HistoryEvent::PullClosed { pr, .. } = event
    {
        return Some(*pr);
    }
    None
}

/// Whether the snapshot references of the round an event is about are removed after it: its pull request was merged or
/// closed, or its round ended with no pull request left for the human (an approval handed to the human keeps them until
/// the human merges or closes it).
fn event_prunes(event: &HistoryEvent) -> bool {
    if let HistoryEvent::PullMerged { .. } | HistoryEvent::PullClosed { .. } = event {
        return true;
    }
    if let HistoryEvent::RoundClosed { end, .. } = event {
        return *end != RoundEnd::Approved;
    }
    false
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
    /// Whether the snapshot references of the round's checkpoints are removed after the record: the pull request was
    /// merged or closed, or the round ended with no pull request left for the human.
    prunes: bool,
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
            prunes: event_prunes(event),
        }
    }
}

impl Owner {
    /// Queues the checkpoints that follow the record of the event `step` was made of, and the removal of the snapshots of
    /// a round that the event closed. Called by the one function that writes the history of a round.
    pub(super) fn checkpoints_after(&self, at: Timestamp, team: &TeamId, rounds: &[RoundId], step: &Step) {
        if let Some(cause) = step.cause {
            for round in rounds {
                self.queue_checkpoint(at, team, round, cause, None, step.pull_request);
            }
        }
        if step.prunes {
            // The snapshots go after the checkpoint of the merge or the close: it is one of them.
            match &step.closed {
                Some(round) => self.queue_prune(team, round),
                None => rounds.iter().for_each(|round| self.queue_prune(team, round)),
            }
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
        let pull = {
            let mut pulls = locked(&self.checkpoints.pulls);
            let state = pulls.entry(round.clone()).or_default();
            if pull_request.is_some() {
                state.number = pull_request;
            }
            match cause {
                CheckpointCause::PullOpened => state.draft = true,
                CheckpointCause::PullReady | CheckpointCause::PullMerged | CheckpointCause::PullClosed => {
                    state.draft = false;
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
            *state
        };
        let Some(facts) = self.checkpoint_facts(team, round, holder, pull) else {
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
        pull: PullState,
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
        // Only the lead and the members of the round hold an item: a reviewer or a worker leaves no checkpoint of its own.
        let roster = team.crew().roster();
        let holders: Vec<_> = roster
            .iter()
            .filter(|(role, _)| matches!(role, Role::Solo | Role::Coordinator | Role::Member))
            .collect();
        let (_, member) = if let Some(task) = holder {
            *holders.iter().find(|(_, member)| member.task() == task)?
        } else {
            *holders
                .iter()
                .find(|(_, member)| Some(member.agent().name()) == assignee)
                .or_else(|| {
                    holders
                        .iter()
                        .find(|(role, _)| matches!(role, Role::Solo | Role::Coordinator))
                })?
        };
        let in_pull_request = phase.and_then(|phase| match phase {
            ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. } => {
                Some(pr.number)
            }
            ItemPhase::Done {
                completion: Completion::Merged { pr, .. },
                ..
            } => Some(pr.number),
            ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::Done {
                completion: Completion::Listed,
                ..
            } => None,
        });
        let kind = phase.map_or(PhaseKind::InProgress, ItemPhase::kind);
        let reviewed = matches!(kind, PhaseKind::InReview | PhaseKind::Fixing | PhaseKind::Approved);
        Some(Facts {
            item,
            task: member.task().clone(),
            tier: member.agent().tier(),
            pull_request: pull
                .number
                .or(in_pull_request)
                .or_else(|| round.part_pull_request().map(|pr| pr.number)),
            draft: pull.draft,
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
        let mut last_round = None;
        for step in queued {
            match step {
                Queued::Take {
                    team,
                    round,
                    cause,
                    at,
                    facts,
                } => {
                    self.take_checkpoint(&team, &round, cause, at, *facts).await;
                    last_round = Some(round);
                }
                Queued::Prune { team, round, worktrees } => {
                    // The records of the round are read from the history: everything before is written first.
                    self.wait_for_history(&round).await;
                    self.prune_round(&team, &round, &worktrees).await;
                    last_round = Some(round);
                }
            }
        }
        // The next step of a round is acted on only when this one is in `history.db`, as long as the writer answers: a
        // writer that is stuck (a database another process holds) never stops a round.
        if let Some(round) = last_round {
            self.wait_for_history(&round).await;
        }
    }

    /// Waits until the history's writer has what was handed in before, for a limited time, and logs the first time that
    /// the writer does not answer in it.
    async fn wait_for_history(&mut self, round: &RoundId) {
        let limit = if self.checkpoints.stalled {
            STALLED_FLUSH_WAIT
        } else {
            FLUSH_WAIT
        };
        match self.history.flush_within_async(limit).await {
            Flush::Done => self.checkpoints.stalled = false,
            Flush::TimedOut => {
                if !self.checkpoints.stalled {
                    tracing::warn!(
                        event = LogEvent::CheckpointFailed.as_str(),
                        round = %round,
                        cause = "history_stalled",
                        error = "the history's writer did not answer in time",
                        "the record of a checkpoint may not be written yet: the round goes on"
                    );
                }
                self.checkpoints.stalled = true;
            }
        }
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
            .get(&(round.clone(), facts.task.clone()))
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
                    (round.clone(), facts.task.clone()),
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
                    cause = "prune_list",
                    error = %error_chain(&error),
                    "the snapshots of a round that ended cannot be listed: they stay"
                );
                return;
            }
        };
        // The round has ended: what the checkpoints compared with and the pull request they knew are of no use any more.
        self.checkpoints.last.retain(|(of, _), _| of != round);
        locked(&self.checkpoints.pulls).remove(round);
        let mut references: Vec<String> = kept.into_iter().filter_map(|checkpoint| checkpoint.snapshot).collect();
        references.sort();
        references.dedup();
        if references.is_empty() {
            return;
        }

        let Some(worktree) = worktrees.iter().find(|path| path.exists()) else {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                team = %team,
                round = %round,
                cause = "prune_no_worktree",
                error = "no worktree of the team exists",
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
                cause = "prune_remove",
                error = %error,
                "the snapshots of a round that ended cannot be removed: they stay"
            );
        }
    }

    /// The team and the open round `task` works for: the newest open round of its current run, else the open round of the
    /// item it is assigned, else the newest of its last run, else the only open round of its team. `None` when the task is
    /// in no open round.
    pub(super) fn checkpoint_round_of(&self, task: &TaskId) -> Option<(TeamId, RoundId)> {
        let found = self.parts.state.find_task(task)?;
        let open = |id: &&RoundId| found.team.round(id).is_some_and(Round::is_open);
        let of_run = |run: &agent_core::state::Run| run.rounds().iter().rev().find(open).cloned();
        let assigned = || {
            found.team.items().iter().find_map(|progress| match progress.phase() {
                ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. }
                    if assignee.as_str() == found.member.agent().name() =>
                {
                    found
                        .team
                        .open_rounds()
                        .find(|round| round.item() == Some(progress.id()))
                        .map(|round| round.id().clone())
                }
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => None,
            })
        };
        let round = found
            .member
            .current_run()
            .and_then(of_run)
            // A task whose run has not started is in the round of the item it is assigned.
            .or_else(assigned)
            .or_else(|| found.member.runs().last().and_then(of_run))
            .or_else(|| {
                let mut opens = found.team.open_rounds();
                let only = opens.next()?;
                opens.next().is_none().then(|| only.id().clone())
            })?;
        Some((found.team.id().clone(), round))
    }

    /// Logs that the clock gave no time for the checkpoint of `cause`, which is therefore not taken.
    fn clock_failed(team: &TeamId, round: &RoundId, cause: CheckpointCause, error: &str) {
        tracing::warn!(
            event = LogEvent::CheckpointFailed.as_str(),
            team = %team,
            round = %round,
            cause = %cause,
            error = %error,
            "the clock gives no time for a checkpoint: it is not taken, the round goes on"
        );
    }

    /// Queues the checkpoint of `cause` for `task`, in the open round it works for; a task in none has no checkpoint.
    pub(super) fn queue_for_task(&self, task: &TaskId, cause: CheckpointCause) {
        let Some((team, round)) = self.checkpoint_round_of(task) else {
            return;
        };
        let at = match Timestamp::now(&self.parts.clock) {
            Ok(at) => at,
            Err(error) => {
                Self::clock_failed(&team, &round, cause, &error_chain(&error));
                return;
            }
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
                            cause = %CheckpointCause::DaemonStarted,
                            error = %error_chain(&error),
                            "the checkpoints of an open round cannot be read at the start of the daemon"
                        );
                        continue;
                    }
                };
                if !kept.is_empty() {
                    self.checkpoints.opened.insert(round.id().clone());
                }
                if let Some(latest) = kept.last() {
                    // A restart between the opening of a pull request and its marking ready keeps both.
                    locked(&self.checkpoints.pulls).insert(
                        round.id().clone(),
                        PullState {
                            number: latest.pull_request,
                            draft: latest.draft,
                        },
                    );
                }
                for checkpoint in kept {
                    self.checkpoints.last.insert(
                        (round.id().clone(), checkpoint.task),
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
        // A worktree that is gone is no reason to leave the start unrecorded: the checkpoint is then taken with no snapshot
        // and the failure is logged.
        for task in tasks {
            self.queue_for_task(&task, CheckpointCause::DaemonStarted);
        }
        self.flush_checkpoints().await;
    }

    /// Looks at the worktree of `task`, when it has a run going for an open round, once a minute, and queues a
    /// `commit` checkpoint when its head is not the head of the task's last checkpoint. Several commits in one minute are
    /// one checkpoint; a task with no checkpoint yet is only noted.
    pub(super) async fn look_for_commits(&mut self, task: &TaskId) {
        let Ok(Some(target)) = watched_run(&self.parts.state, task) else {
            return;
        };
        let Some((team, round)) = self.checkpoint_round_of(task) else {
            return;
        };
        let now = match Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(error) => {
                Self::clock_failed(&team, &round, CheckpointCause::Commit, &error_chain(&error));
                return;
            }
        };
        match self.checkpoints.looked_at.get(task) {
            // The first sighting of a task only starts its minute: the checkpoint of its round's opening, or of the start of
            // the daemon, was taken just before it.
            None => {
                self.checkpoints.looked_at.insert(task.clone(), now);
                return;
            }
            Some(last) if (now.as_utc() - last.as_utc()).whole_seconds() < LOOK_SECONDS => return,
            Some(_) => {}
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
        match self
            .checkpoints
            .last
            .get(&(round.clone(), task.clone()))
            .map(|last| last.head.as_ref())
        {
            Some(Some(last)) if *last != head => {
                self.queue_checkpoint(now, &team, &round, CheckpointCause::Commit, Some(task), None);
            }
            Some(Some(_)) => {}
            Some(None) | None => {
                self.checkpoints
                    .last
                    .entry((round.clone(), task.clone()))
                    .or_default()
                    .head = Some(head);
            }
        }
    }
}

#[cfg(test)]
mod tests;
