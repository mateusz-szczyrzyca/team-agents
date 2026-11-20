//! The owner of the state: the one task that reads and changes it.
//!
//! Everything that wants the state asks the owner through a channel (CON-3): the connections of the socket for their
//! requests, the engine tasks through their results. The owner answers requests, starts a run for every queued task,
//! applies the end of each run to its task, wakes paused tasks when their time comes and publishes what happens as
//! events. A state it cannot save ends the daemon: models must not go on working with a state that is not kept.

mod answers;
mod controls;
mod disk;
#[cfg(test)]
mod fixture;
mod gate;
mod history;
mod plan;
mod round_state;
mod rounds;
mod start;
mod teams;
mod wakes;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::cli::SessionId as CliSessionId;
use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::decide::outcome::{
    AfterHang, AfterNoStatus, AfterTurnLimit, after_hang, after_no_status, after_turn_limit, consecutive_no_status,
    consecutive_turn_limits,
};
use agent_core::history::{HistoryEvent, RunWhy};
use agent_core::ids::{GenerateIdError, InvalidName, RoundId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::protocol::{
    Client, Command, Event, EventKind, Failure, FailureKind, Reply, RoundDetail, RoundTask, Target, TeamSummary,
};
use agent_core::state::{
    BeginRunError, CloseRoundError, InvalidTimestamp, NewRun, Question, QuestionError, RunEnd, RunError, SessionId,
    State, StateOpenRoundError, Team, Timestamp,
};
use agent_core::status::{InvalidTransition, RunOutcome, TaskStatus};
use agent_runtime::agentbus::AgentBus;
use agent_runtime::clock::SharedClock;
use agent_runtime::disk::GIBIBYTE;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{History, Scoped, log_decision};
use agent_runtime::notify::Notification;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::questions_prompts::QuestionsPrompts;
use agent_runtime::run::{FinishedRun, Progress, RunEngine, RunError as EngineError, RunVerdict, WatchdogHold};
use agent_runtime::settings::Settings;
use agent_runtime::store::{StateStore, StoreError};
use agent_runtime::team_prompts::TeamPrompts;
use rand::rngs::StdRng;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{Id as JoinId, JoinError, JoinSet};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

pub(crate) use self::rounds::{Pulls, PullsError};
use self::start::{Context, Plan, StartProblem, next_number, plan, run_why};
use self::teams::{Outbound, Served};
use super::control::{Halt, Steer, after_halt, halted_status};
use super::gate::GateQueue;
use super::history::health_to_tell;
use super::notice::{Notice, NoticeContext};
use super::notifier::Delivery;
use super::questions;
use super::questions::watch::{Report, Saves, Watcher};
use super::recovery::{RecoveryError, interrupted_status, recover};
use super::settle::{Settled, settle};
use super::team::{AssignmentError, DoneNoticeError, NoticeRenderError, Why};
use agent_runtime::process::error_chain;
use time::UtcOffset;

/// How many runs may go at once.
pub(super) const MAX_RUNS: NonZeroUsize = NonZeroUsize::new(16).unwrap();

/// How many messages may wait for the owner.
pub(super) const INBOX: usize = 64;

/// How many events a subscriber may fall behind before it is let go.
pub(super) const EVENT_BACKLOG: usize = 1024;

/// Seconds in a minute, for the notice of a hang.
const SECS_PER_MINUTE: u64 = 60;

/// What a connection asks of the owner.
#[derive(Debug)]
pub(super) enum Message {
    /// A request, to answer.
    Request {
        /// Who asks.
        client: Client,
        /// What it asks.
        command: Command,
        /// Where the answer goes.
        reply: oneshot::Sender<Result<Reply, Failure>>,
    },
    /// An answer request whose split interpretation accompanies the ordinary reply.
    Answer {
        /// Who supplies the answers.
        client: Client,
        /// The answers to record.
        request: agent_core::protocol::AnswerRequest,
        /// Where the ordinary reply and optional split interpretation go.
        reply: oneshot::Sender<Result<(Reply, Option<String>), Failure>>,
    },
    /// A subscription: the teams as they are, and the events from then on.
    Subscribe {
        /// The team to follow; every team when `None`.
        team: Option<TeamId>,
        /// Where the teams and the events go.
        reply: oneshot::Sender<(Vec<TeamSummary>, broadcast::Receiver<Event>)>,
    },
}

/// The task that carried out a run panicked or was cancelled: the run's end is not known.
#[derive(Debug, thiserror::Error)]
#[error("the task of the run failed")]
struct RunTaskFailed(#[source] JoinError);

/// Why the owner stopped with an error: something it cannot go on without.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OwnerError {
    /// A second opinion cannot be scheduled or retained.
    #[error(transparent)]
    Opinion(#[from] rounds::OpinionDaemonError),
    /// A fix response correction cannot be rendered.
    #[error("cannot render fix response correction")]
    FixResponse(#[from] rounds::ResponseTextError),
    /// The permanent finding numbers have been used up.
    #[error("cannot number review findings")]
    FindingNumbers(#[from] agent_core::state::FindingNumbersExhausted),
    /// The state cannot be saved.
    #[error("the state cannot be saved")]
    Store(#[from] StoreError),
    /// The clock gives a time that cannot be recorded.
    #[error("the clock gives a time that cannot be recorded")]
    Clock(#[from] InvalidTimestamp),
    /// Recovery on start failed.
    #[error("cannot recover the state")]
    Recovery(#[from] RecoveryError),
    /// A task that must be in the state is not.
    #[error("task {task} is not in the state")]
    LostTask {
        /// The task.
        task: TaskId,
    },
    /// A change of the status of a task is not allowed.
    #[error("the status of task {task} cannot be changed")]
    Status {
        /// The task.
        task: TaskId,
        /// What the state said.
        source: InvalidTransition<TaskStatus>,
    },
    /// A run of a task cannot be recorded.
    #[error("the run of task {task} cannot be recorded")]
    Run {
        /// The task.
        task: TaskId,
        /// What the state said.
        source: RunError,
    },
    /// An approved split cannot be activated.
    #[error("cannot activate the approved split")]
    Split(#[from] agent_core::state::ActivateSplitError),
    /// Questions cannot be recorded.
    #[error("the questions of task {task} cannot be recorded")]
    Question {
        /// The task.
        task: TaskId,
        /// What the state said.
        source: QuestionError,
    },
    /// A run cannot be recorded in the state with its rounds.
    #[error("the run of task {task} cannot be recorded with its rounds")]
    BeginRun {
        /// The task.
        task: TaskId,
        /// What the state said.
        source: BeginRunError,
    },
    /// A round cannot be opened.
    #[error("a round of team {team} cannot be opened")]
    OpenRound {
        /// The team.
        team: TeamId,
        /// What the state said.
        source: StateOpenRoundError,
    },
    /// A round cannot be closed.
    #[error("the round {round} cannot be closed")]
    CloseRound {
        /// The round.
        round: RoundId,
        /// What the state said.
        source: CloseRoundError,
    },
    /// An item that has started has no round.
    #[error("item {item} of team {team} has no round")]
    LostRound {
        /// The team.
        team: TeamId,
        /// The item.
        item: agent_core::ids::ItemId,
    },
    /// The ID of a round cannot be made.
    #[error("the ID of a round cannot be made")]
    RoundId(#[from] GenerateIdError),
    /// Every ID drawn for a round was used already.
    #[error("no round ID could be drawn that the state does not have")]
    NoFreeRoundId,
    /// A session ID is not valid.
    #[error("a session ID is not valid")]
    Session(#[from] InvalidValue),
    /// The question rounds of a task are used up.
    #[error("task {task} has no number of questions left")]
    QuestionNumbers {
        /// The task.
        task: TaskId,
    },
    /// A run ended that the owner did not start.
    #[error("the run task {id} is not one the owner started")]
    UnknownRun {
        /// The task's ID in the runtime.
        id: JoinId,
    },
    /// A name in the state is not an agent name.
    #[error("a name in the state is not an agent name")]
    Name(#[from] InvalidName),
    /// The assignments of a team cannot be decided or recorded.
    #[error("the assignments of team {team} cannot be decided")]
    Assign {
        /// The team.
        team: TeamId,
        /// What went wrong.
        source: AssignmentError,
    },
    /// The outcome of a review cannot be recorded.
    #[error("the outcome of a review of item {item} of team {team} cannot be recorded")]
    Review {
        /// The team.
        team: TeamId,
        /// The item.
        item: agent_core::ids::ItemId,
        /// What the state said.
        source: agent_core::state::SubmitReviewError,
    },
    /// The pull requests of a team cannot be decided.
    #[error("the pull requests of team {team} cannot be decided")]
    Publications {
        /// The team.
        team: TeamId,
        /// What went wrong.
        source: crate::daemon::pr::PlanError,
    },
    /// A team that must be in the state is not.
    #[error("team {team} is not in the state")]
    LostTeam {
        /// The team.
        team: TeamId,
    },
    /// An item that must be in a team is not.
    #[error("item {item} is not in team {team}")]
    LostItem {
        /// The team.
        team: TeamId,
        /// The item.
        item: agent_core::ids::ItemId,
    },
    /// An item cannot be given to a member.
    #[error("item {item} of team {team} cannot be given to a member")]
    Item {
        /// The team.
        team: TeamId,
        /// The item.
        item: agent_core::ids::ItemId,
        /// What the state said.
        source: agent_core::state::AdvanceError,
    },
    /// What a member's run ending tells the others cannot be made.
    #[error("what a run's end tells the team cannot be made")]
    Told(#[from] super::team::NoticeError),
    /// A message for the bus cannot be made from its text.
    #[error("a message for the bus cannot be made")]
    Notice(#[from] NoticeRenderError),
    /// The notification that a team is finished cannot be made.
    #[error("the notification that a team is finished cannot be made")]
    TeamDone(#[from] DoneNoticeError),
}

/// What the owner is made of.
pub(super) struct OwnerParts {
    /// The store the state is saved in.
    pub(super) store: Arc<StateStore>,
    /// The state, as recovered.
    pub(super) state: State,
    /// The configuration and the secrets.
    pub(super) settings: Arc<Settings>,
    /// The run prompts.
    pub(super) texts: Arc<HarnessPrompts>,
    /// The texts of the teams.
    pub(super) team_texts: Arc<TeamPrompts>,
    /// The texts of the questions and their answers.
    pub(super) question_texts: Arc<QuestionsPrompts>,
    /// The rules.
    pub(super) rules: Arc<PromptLoader>,
    /// The bus of the home: the teams' members, their messages and their requests for workers.
    pub(super) bus: AgentBus,
    /// Where the questions files to open in VS Code go; `None` when the configuration does not ask for it.
    pub(super) opens: Option<mpsc::Sender<std::path::PathBuf>>,
    /// The engine that carries out runs.
    pub(super) engine: RunEngine,
    /// The clock.
    pub(super) clock: SharedClock,
    /// The source of the IDs.
    pub(super) rng: StdRng,
    /// Where the notifications go.
    pub(super) notices: mpsc::Sender<Delivery>,
    /// Where the events go.
    pub(super) events: broadcast::Sender<Event>,
    /// The machine's offset from UTC.
    pub(super) local_offset: UtcOffset,
    /// How often paused tasks are looked at.
    pub(super) tick: Duration,
    /// The notifier that delivers notifications.
    pub(super) notifier: agent_runtime::notify::Notifier,
    /// Measurer of free disk space.
    pub(super) free_space: agent_runtime::disk::SharedFreeSpace,
}

/// What became of a run, for the owner to apply.
enum Outcome {
    /// The engine returned.
    Verdict(Box<RunVerdict>),
    /// The run could not be carried out, or its task failed; why, as text for the human.
    Failed(String),
}

/// The facts about a task that its team gives and that do not change while the owner works on it.
struct Located {
    /// The task's team.
    team: TeamId,
    /// The task's role.
    role: Role,
    /// The agent's name.
    agent: String,
}

/// A run that went to its end, as its task is told of it.
struct Ended<'a> {
    /// The task.
    task: &'a TaskId,
    /// The facts about the task.
    located: &'a Located,
    /// When the run ended.
    now: Timestamp,
    /// What the end does to the task.
    settled: Settled,
    /// Whether the run did anything before it ended.
    progress: Progress,
}

/// How a questions file went.
enum Asked {
    /// The questions are recorded and the human is told.
    Recorded(Notice),
    /// The file cannot be used: the task fails.
    Unusable(Notice),
}

/// The owner.
pub(super) struct Owner {
    /// What it is made of.
    parts: OwnerParts,
    /// The runs that are going.
    runs: JoinSet<Result<RunVerdict, EngineError>>,
    /// The task of each run that is going, by the run's task in the runtime.
    running: HashMap<JoinId, TaskId>,
    /// The tasks that have a run going.
    in_flight: HashSet<TaskId>,
    /// What stops the run of each task that has one going, by task.
    cancels: HashMap<TaskId, CancellationToken>,
    /// The tasks whose run the human has stopped, and how: the end of the run decides their status.
    halts: HashMap<TaskId, Halt>,
    /// When each paused task is queued again.
    wakes: HashMap<TaskId, Timestamp>,
    /// How many hangs in a row each task has had without progress.
    hangs: HashMap<TaskId, u32>,
    /// Why a queued task of a team starts now; read when its run is made ready.
    reasons: HashMap<TaskId, Why>,
    /// How often each task was woken for the newest message that waits for it.
    wake_counts: HashMap<TaskId, wakes::WakeCount>,
    /// What the log line of the run of a task that was woken says of the wake-up; read when the run starts.
    wake_notes: HashMap<TaskId, wakes::WakeNote>,
    /// What is still to be done on the bus, oldest first: it is tried again at every look until it is done.
    outbox: VecDeque<Outbound>,
    /// The requests for workers that are being served, by the task of the worker.
    serving: HashMap<TaskId, Served>,
    /// The teams that are finished and closed: the human was told.
    closed: HashSet<TeamId>,
    /// The teams to look at at the next turn of the loop, in the order of their IDs.
    looks: BTreeSet<TeamId>,
    /// The watch on the questions files; `None` when the system would not make one: the answers then arrive only
    /// through `agent answer` and the dashboard.
    watcher: Option<Watcher>,
    /// The questions files the human saved, as the watch reports them.
    saved: Option<Saves>,
    /// The tasks whose directory is watched.
    watched: HashSet<TaskId>,
    /// For each task that waits for answers, the problems of the last save the human was told about, so that the same
    /// save is not told twice.
    incomplete: HashMap<TaskId, String>,
    /// The holds on the watchdog of each task that has a run going: a gate that waits for its turn takes one.
    holds: HashMap<TaskId, WatchdogHold>,
    /// What stops the gates of each task that asked for one, whether its run is going or not; the human's stop or pause
    /// of the task cancels and removes it.
    gate_stops: HashMap<TaskId, CancellationToken>,
    /// The queue of the gate service; `None` for an owner that runs without one, which refuses every gate.
    gate: Option<GateQueue>,
    /// The autopilot; `None` for an owner that runs without one: no pull request is opened.
    pulls: Option<Pulls>,
    /// The clients of GitHub, by repository.
    github: HashMap<agent_core::config::RepoName, agent_runtime::github::GitHub>,
    /// What the autopilot remembers between two looks.
    watch: rounds::PullWatch,
    /// Where what happens is recorded for good; off for an owner that runs without one.
    history: History,
    /// Cancelled when the daemon is to stop: the runs it starts stop with it.
    shutdown: CancellationToken,
    /// Whether free space was below low threshold on last check.
    was_low_space: bool,
    /// Disk work that runs off the loop: build directory removals and the low-space listing. Each outcome is handled
    /// by the loop when it is ready.
    disk_jobs: JoinSet<disk::DiskJob>,
    /// Build directories of tasks whose run was going when their round closed; removed when the run ends.
    deferred_targets: BTreeMap<TaskId, PathBuf>,
    /// Build directory removals not yet ended; a task with one is not started.
    pending_removals: disk::PendingRemovals,
    /// Whether free space was below min threshold on last check.
    was_min_space: bool,
}

/// The next report of the watch on the questions files; never, when the files are not watched.
async fn next_report(saves: &mut Option<Saves>) -> Option<Report> {
    match saves {
        Some(saves) => saves.recv().await,
        None => std::future::pending().await,
    }
}

impl Owner {
    /// An owner made of `parts`.
    pub(super) fn new(parts: OwnerParts, shutdown: CancellationToken) -> Self {
        let (watcher, saved) = match Watcher::new() {
            Ok((watcher, saves)) => (Some(watcher), Some(saves)),
            Err(error) => {
                // The daemon is useful without it: the answers still come through `agent answer` and the dashboard.
                tracing::warn!(
                    event = LogEvent::QuestionsFileInvalid.as_str(),
                    error = %error_chain(&error),
                    "the questions files are not watched: a saved answer is not noticed"
                );
                (None, None)
            }
        };
        Self {
            parts,
            runs: JoinSet::new(),
            running: HashMap::new(),
            in_flight: HashSet::new(),
            cancels: HashMap::new(),
            halts: HashMap::new(),
            wakes: HashMap::new(),
            hangs: HashMap::new(),
            reasons: HashMap::new(),
            wake_counts: HashMap::new(),
            wake_notes: HashMap::new(),
            outbox: VecDeque::new(),
            serving: HashMap::new(),
            closed: HashSet::new(),
            looks: BTreeSet::new(),
            watcher,
            saved,
            watched: HashSet::new(),
            incomplete: HashMap::new(),
            holds: HashMap::new(),
            gate_stops: HashMap::new(),
            gate: None,
            pulls: None,
            github: HashMap::new(),
            watch: rounds::PullWatch::default(),
            history: History::disabled(),
            shutdown,
            was_low_space: false,
            disk_jobs: JoinSet::new(),
            deferred_targets: BTreeMap::new(),
            pending_removals: disk::PendingRemovals::default(),
            was_min_space: false,
        }
    }

    /// The same, recording what happens in `history`.
    pub(super) fn with_history(mut self, history: History) -> Self {
        self.history = history;
        self
    }

    /// The same, with the gates it is asked for going to `queue`.
    pub(super) fn with_gate(mut self, queue: GateQueue) -> Self {
        self.gate = Some(queue);
        self
    }

    /// The same, with the pull requests it is asked for opened and reviewed as `pulls` says.
    pub(super) fn with_pulls(mut self, pulls: Pulls) -> Self {
        self.pulls = Some(pulls);
        self
    }

    /// Serves `inbox` and runs the tasks until the shutdown is cancelled; then stops the runs and records them.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn run(mut self, mut inbox: mpsc::Receiver<Message>) -> Result<(), OwnerError> {
        self.recover().await?;
        self.adopt_open_questions().await?;
        let mut retained_at = None;
        let mut tick = tokio::time::interval(self.parts.tick.max(Duration::from_millis(1)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if let Some(request) = super::history::retention_due(
                &self.parts.settings.config,
                self.parts.store.home(),
                Timestamp::now(&self.parts.clock)?,
                &mut retained_at,
            ) {
                self.history.retain(request);
            }
            self.team_life().await?;
            self.start_queued().await?;
            tokio::select! {
                () = self.shutdown.cancelled() => break,
                message = inbox.recv() => match message {
                    Some(message) => self.handle(message).await?,
                    None => break,
                },
                Some(joined) = self.runs.join_next_with_id() => self.settle_joined(joined).await?,
                Some(report) = next_report(&mut self.saved) => self.reported(report).await?,
                Some(joined) = self.disk_jobs.join_next_with_id() => self.disk_job_done(joined).await,
                _ = tick.tick() => {
                    self.wake_due().await?;
                    self.look_at_every_team();
                }
            }
        }
        let stopped = self.stop_runs().await;
        // The removals and listings already started finish before the owner ends: a stopped daemon leaves no build
        // directory half removed.
        while let Some(joined) = self.disk_jobs.join_next_with_id().await {
            self.disk_job_done(joined).await;
        }
        stopped
    }

    // ---- start and recovery ---------------------------------------------------------------------------------

    /// Ends the runs a restart left going and finds the pauses to wake.
    async fn recover(&mut self) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let recovery = recover(&mut self.parts.state, now, self.parts.settings.config.limits())?;
        for (at, task) in recovery.wakes {
            self.wakes.insert(task, at);
        }
        self.hangs.extend(recovery.hangs);
        self.record_recovered_ends(&recovery.interrupted);
        for task in &recovery.exhausted {
            let located = self.locate(task)?;
            self.commit(task, &located, now, Vec::new(), Some(Notice::NoStatus { exit: None }))
                .await?;
            self.after_run(task, &located, &RunEnd::NoStatus { exit_code: None }, None)?;
        }
        let repaired = super::recovery::reconcile_findings(&mut self.parts.state);
        for repair in &repaired {
            tracing::warn!(
                event = LogEvent::RestartCorrected.as_str(),
                team = %repair.team,
                pr = %repair.pr,
                items = ?repair.items,
                finding = %repair.finding.number,
                explanation = ?repair.finding.explanation,
                status = ?repair.finding.status,
                "the items of a pull request disagreed on a finding: the most advanced record now holds for all"
            );
        }
        if !repaired.is_empty() {
            self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
            self.save().await?;
        }
        self.ask_unanswered_reviews().await?;
        self.hold_restarted_opinions().await?;
        self.remember_closed_teams();
        self.look_at_every_team();
        if !recovery.interrupted.is_empty() {
            tracing::warn!(
                event = LogEvent::RestartCorrected.as_str(),
                tasks = recovery.interrupted.len(),
                "runs that a restart interrupted have been reconciled"
            );
            self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
            self.save().await?;
        }
        Ok(())
    }

    /// Starts a run for every queued task that has none, as far as the limit allows.
    async fn start_queued(&mut self) -> Result<(), OwnerError> {
        let queued: Vec<TaskId> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let crew = team.crew().roster().into_iter().map(|(_, member)| member);
                crew.chain(team.reviewers().iter())
                    .filter(|member| member.status() == TaskStatus::Queued)
                    .map(|member| member.task().clone())
                    .collect::<Vec<_>>()
            })
            .filter(|task| !self.in_flight.contains(task) && !self.pending_removals.contains(task))
            .collect();
        if queued.is_empty() {
            return Ok(());
        }
        if !self.space_allows_run().await {
            return Ok(());
        }
        for task in queued {
            if self.runs.len() >= MAX_RUNS.get() {
                break;
            }
            // Each launch is measured on its own: the space may have fallen since the last one started.
            if !self.space_allows_run().await {
                break;
            }
            self.start(task).await?;
        }
        Ok(())
    }

    /// Whether a new run may start now: the free space is measured and is at least `disk.min_free_gb`. A refusal
    /// notifies the human once while the space stays below the minimum.
    async fn space_allows_run(&mut self) -> bool {
        let Some(free_bytes) = self.check_free_space() else {
            return false;
        };
        let min_limit_gb = self.parts.settings.config.disk().min_free_gb;
        if free_bytes >= min_limit_gb.saturating_mul(GIBIBYTE) {
            return true;
        }
        if !self.was_min_space {
            self.was_min_space = true;
            self.notify_rounds_waiting(free_bytes, min_limit_gb).await;
        }
        false
    }

    /// Starts the run of `task`, or fails the task when it cannot be made ready.
    async fn start(&mut self, task: TaskId) -> Result<(), OwnerError> {
        let why = self.reasons.remove(&task);
        // What a wake-up noted is for the run it starts, and for no other.
        let woke = self
            .wake_notes
            .remove(&task)
            .filter(|_| matches!(why, Some(Why::Woken)));
        let planned = {
            let context = Context {
                state: &self.parts.state,
                settings: &self.parts.settings,
                texts: &self.parts.texts,
                team_texts: &self.parts.team_texts,
                question_texts: &self.parts.question_texts,
                rules: &self.parts.rules,
                home: self.parts.store.home(),
                clock: &self.parts.clock,
                why: why.as_ref(),
                autopilot: self.pulls.as_ref().map(Pulls::texts),
                checks: self.pulls.as_ref().map(Pulls::checks),
            };
            plan(&context, &mut self.parts.rng, &task).await
        };
        let plan = match planned {
            Ok(plan) => plan,
            Err(problem) => {
                if let Ok(located) = self.locate(&task) {
                    let span = tracing::error_span!("task", team = %located.team, task = %task);
                    let _entered = span.enter();
                    match &problem {
                        StartProblem::AgentGone { name } => {
                            tracing::error!(
                                event = LogEvent::ConfigurationNameMissing.as_str(),
                                kind = "agent",
                                name = %name,
                                "an agent in the state is gone from the configuration"
                            );
                        }
                        StartProblem::RepoGone { repo } => {
                            tracing::error!(
                                event = LogEvent::ConfigurationNameMissing.as_str(),
                                kind = "repo",
                                name = %repo,
                                "a repository in the state is gone from the configuration"
                            );
                        }
                        StartProblem::NoSuchTask { .. }
                        | StartProblem::NoWorktree { .. }
                        | StartProblem::WorktreeNotDirectory { .. }
                        | StartProblem::Path { .. }
                        | StartProblem::NumbersExhausted
                        | StartProblem::Prompt(_)
                        | StartProblem::Id(_)
                        | StartProblem::Answers(_)
                        | StartProblem::AnswersText(_)
                        | StartProblem::Team(_)
                        | StartProblem::Name(_)
                        | StartProblem::Branch(_)
                        | StartProblem::Session(_)
                        | StartProblem::Autopilot(_) => {}
                    }
                }
                return self.fail_to_start(&task, error_chain(&problem)).await;
            }
        };
        let rounds = round_state::rounds_for(&self.parts.state, &task, why.as_ref())?;
        let reason = run_why(&self.parts.state, &task, why.as_ref());
        self.begin(&task, plan, rounds, (reason, woke)).await
    }

    /// Records the start of the run of `plan`, which works for `rounds`, and hands it to the engine.
    async fn begin(
        &mut self,
        task: &TaskId,
        plan: Plan,
        rounds: Vec<RoundId>,
        (reason, woke): (RunWhy, Option<wakes::WakeNote>),
    ) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.parts
            .state
            .begin_run(
                task,
                NewRun {
                    id: plan.run.clone(),
                    at: now,
                    session: plan.recorded_session.clone(),
                    rounds: rounds.clone(),
                },
            )
            .map_err(|source| OwnerError::BeginRun {
                task: task.clone(),
                source,
            })?;
        // The run's prompt has the reason it was sent back for its plan, if any: no later run is told it again.
        // So are the answers about its plan that no run asked for.
        if let Some((_, mut member)) = self.parts.state.find_task_mut(task) {
            let reason = member.take_plan_reason();
            let answers = member.take_unread_plan_answers();
            if reason.is_some() || answers.is_some() {
                tracing::debug!(task = %task, ?reason, ?answers, "the run is told what waited for it of its plan");
            }
        }
        events.push(EventKind::RunStarted {
            team: located.team.clone(),
            task: task.clone(),
            run: plan.run.clone(),
            rounds: rounds.clone(),
        });
        self.move_to(task, &located.team, TaskStatus::Running, &mut events)?;
        self.commit(task, &located, now, events, None).await?;
        // The run is in the state: the history is told before the engine can say anything about it.
        self.record_run_start(task, &plan, &rounds, reason, now);
        let rounds_field = rounds.iter().map(RoundId::as_str).collect::<Vec<_>>().join(",");
        let run_span = tracing::error_span!(
            "run",
            team = %located.team,
            rounds = %rounds_field,
            task = %task,
            run = %plan.run
        );
        {
            let _entered = run_span.enter();
            if let Some(note) = &woke {
                tracing::info!(
                    event = LogEvent::RunStarted.as_str(),
                    task = %task,
                    run = %plan.run,
                    agent = %located.agent,
                    why = "wake_up",
                    message_id = ?note.message.as_ref().map(|message| message.id),
                    from = note.message.as_ref().map_or("", |message| message.from.as_str()),
                    wake_ups = note.wake_ups,
                    "run started: the bus woke the agent for a message"
                );
            } else {
                tracing::info!(
                    event = LogEvent::RunStarted.as_str(),
                    task = %task,
                    run = %plan.run,
                    agent = %located.agent,
                    "run started"
                );
            }
        }
        let engine = self.parts.engine.clone();
        let cancel = self.shutdown.child_token();
        self.cancels.insert(task.clone(), cancel.clone());
        let hold = WatchdogHold::new();
        self.holds.insert(task.clone(), hold.clone());
        let job = plan.job;
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        // Scoped, not only polled with the subscriber: a run that is aborted is dropped by the runtime, and its spans must
        // close in the registry that made them.
        let handle = self.runs.spawn(Scoped::new(
            async move { engine.run_held(job, &cancel, &hold).await }.instrument(run_span),
            dispatch,
        ));
        self.running.insert(handle.id(), task.clone());
        self.in_flight.insert(task.clone());
        Ok(())
    }

    /// Fails `task`, which could not be started, and tells the human why; its team learns it as of a run that ended
    /// without a status: the coordinator about a member, the requester about the request of a worker.
    async fn fail_to_start(&mut self, task: &TaskId, reason: String) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        {
            let span = tracing::error_span!("task", team = %located.team, task = %task);
            let _entered = span.enter();
            tracing::warn!(
                event = LogEvent::RunEndedWithoutStatus.as_str(),
                task = %task,
                %reason,
                "a run cannot be started"
            );
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.move_to(task, &located.team, TaskStatus::Running, &mut events)?;
        self.move_to(task, &located.team, TaskStatus::Failed, &mut events)?;
        // No run exists for this failure: the record is told, because it cannot tell it from an earlier run's end.
        if let Some(mut team) = self.parts.state.team_mut(&located.team) {
            team.fail_activities_before_a_run(task, now);
        }
        self.commit(task, &located, now, events, Some(Notice::StartFailed { reason }))
            .await?;
        if let Some(pulls) = self.pulls.clone() {
            self.retry_opinion(&pulls, &located.team, task, agent_core::state::OpinionFailure::Failed)
                .await?;
        }
        self.after_run(task, &located, &RunEnd::NoStatus { exit_code: None }, None)
    }

    // ---- the end of a run -----------------------------------------------------------------------------------

    /// Applies the end of a run to its task.
    async fn settle_joined(
        &mut self,
        joined: Result<(JoinId, Result<RunVerdict, EngineError>), JoinError>,
    ) -> Result<(), OwnerError> {
        let (id, outcome) = match joined {
            Ok((id, Ok(verdict))) => (id, Outcome::Verdict(Box::new(verdict))),
            Ok((id, Err(error))) => (id, Outcome::Failed(error_chain(&error))),
            Err(error) => {
                let id = error.id();
                (id, Outcome::Failed(error_chain(&RunTaskFailed(error))))
            }
        };
        let task = self.running.remove(&id).ok_or(OwnerError::UnknownRun { id })?;
        self.in_flight.remove(&task);
        self.release_deferred_target(&task);
        self.cancels.remove(&task);
        self.holds.remove(&task);
        let result = match outcome {
            Outcome::Verdict(verdict) => match *verdict {
                RunVerdict::Finished(run) => self.finished(&task, *run).await,
                RunVerdict::Interrupted { session } => self.interrupted(&task, session).await,
            },
            Outcome::Failed(reason) => self.failed(&task, reason).await,
        };
        let free_bytes = self.check_free_space();
        match self.locate(&task) {
            Ok(located) if located.role == Role::Reviewer => {
                let low_limit_bytes = self.parts.settings.config.disk().low_free_gb.saturating_mul(GIBIBYTE);
                if let Some(free) = free_bytes
                    && free < low_limit_bytes
                    && let Some(team) = self.parts.state.team(&located.team)
                {
                    let worktree_dir = self.parts.store.home().worktree_dir(team.repo(), &task);
                    self.remove_target(&task, &worktree_dir);
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %task,
                    error = %error_chain(&error),
                    "cannot tell whether the finished run's task reviews: its build directory is not released"
                );
            }
        }
        result
    }

    /// A run that could not be carried out: its task fails.
    async fn failed(&mut self, task: &TaskId, reason: String) -> Result<(), OwnerError> {
        // Ordinary tasks remain failed; an unanswered opinion retains its bounded replacement request.
        self.halts.remove(task);
        let located = self.locate(task)?;
        {
            let span = tracing::error_span!("task", team = %located.team, task = %task);
            let _entered = span.enter();
            tracing::warn!(
                event = LogEvent::RunEndedWithoutStatus.as_str(),
                task = %task,
                %reason,
                "a run failed"
            );
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.end_run(
            task,
            &located.team,
            now,
            RunEnd::NoStatus { exit_code: None },
            None,
            &mut events,
        )?;
        self.move_to(task, &located.team, TaskStatus::Failed, &mut events)?;
        self.hangs.remove(task);
        self.commit(task, &located, now, events, Some(Notice::RunFailed { reason }))
            .await?;
        if let Some(pulls) = self.pulls.clone() {
            self.retry_opinion(&pulls, &located.team, task, agent_core::state::OpinionFailure::Failed)
                .await?;
        }
        self.after_run(task, &located, &RunEnd::NoStatus { exit_code: None }, None)
    }

    /// A run that was stopped: by the human, whose word sets the status of the task, or because the daemon shuts down, when
    /// the run is recorded as interrupted and queued, unless its no-status retry budget is exhausted.
    async fn interrupted(&mut self, task: &TaskId, session: Option<CliSessionId>) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.attach_session(task, session)?;
        let Some(halt) = self.halts.remove(task) else {
            self.end_run(
                task,
                &located.team,
                now,
                RunEnd::NoStatus { exit_code: None },
                None,
                &mut events,
            )?;
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            let status = interrupted_status(located.role, found.member);
            self.move_to(task, &located.team, status, &mut events)?;
            let notice = (status == TaskStatus::Failed).then_some(Notice::NoStatus { exit: None });
            self.commit(task, &located, now, events, notice).await?;
            if status == TaskStatus::Failed {
                self.after_run(task, &located, &RunEnd::NoStatus { exit_code: None }, None)?;
            }
            return Ok(());
        };
        self.end_run(task, &located.team, now, RunEnd::Stopped, None, &mut events)?;
        self.move_to(task, &located.team, halted_status(halt), &mut events)?;
        self.hangs.remove(task);
        self.commit(task, &located, now, events, None).await?;
        self.after_run(task, &located, &RunEnd::Stopped, None)
    }

    /// Retains the lead's bounded fix response alongside its verification context.
    fn remember_fix_message(&mut self, located: &Located, message: Option<&str>) -> Result<(), OwnerError> {
        if matches!(located.role, Role::Solo | Role::Coordinator)
            && let Some(message) = message
            && let Some(mut team) = self.parts.state.team_mut(&located.team)
        {
            let ids: Vec<_> = team.items().iter().map(agent_core::state::ItemProgress::id).collect();
            for id in ids {
                let mut item = team.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                    team: located.team.clone(),
                    item: id,
                })?;
                if matches!(item.phase(), agent_core::state::ItemPhase::Fixing { .. })
                    && let Some(context) = item.findings_mut().and_then(|ledger| ledger.verification_mut())
                {
                    context.lead_message =
                        Some(message.chars().take(crate::daemon::review::MAX_REPORT_CHARS).collect());
                }
            }
        }
        Ok(())
    }

    /// A run that went to its end: its task goes where the end says.
    async fn finished(&mut self, task: &TaskId, run: FinishedRun) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        self.keep_plan_out_of_delivery(task, &located).await;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        let settle_span = self.settle_span_of(task, &located.team);
        self.attach_session(task, run.session.clone())?;
        self.remember_fix_message(&located, run.final_message.as_deref())?;
        let settled = settle(
            located.role,
            &run.classified.end,
            run.classified.pause.as_ref(),
            run.final_message.as_deref(),
        );
        let settled = self.solo_waits_for_its_items(located.role, &located.team, settled);
        let settled = self.reviewer_waits(located.role, &located.team, settled);
        let (settled, held) = self.plan_at_end(task, &located, &run.classified.end, settled).await?;
        {
            let _entered = settle_span.enter();
            log_decision(
                "settle",
                &format!("role={:?}, end={:?}", located.role, run.classified.end),
                &format!("{settled:?}"),
            );
        }
        // The human asked for a stop or a pause while the run was ending by itself: their word wins where it can. Over a
        // limit's pause or a hang it wins entirely: the run ended by the human's word, with no wake-up, no strike and no
        // notice of a pause that nobody waits for.
        let halt = self.halts.remove(task);
        let overruled = halt.filter(|_| {
            matches!(
                settled,
                Settled::Paused(_) | Settled::Hung | Settled::TurnLimit | Settled::NoStatus { .. }
            )
        });
        let end = if overruled.is_some() {
            RunEnd::Stopped
        } else {
            run.classified.end.clone()
        };
        if overruled.is_none() {
            let _entered = settle_span.enter();
            log_unusual_end(&end);
        }
        self.end_run(
            task,
            &located.team,
            now,
            end.clone(),
            run.final_message.as_deref(),
            &mut events,
        )?;
        let (status, notice) = if let Some(halt) = overruled {
            self.hangs.remove(task);
            self.wakes.remove(task);
            (halted_status(halt), None)
        } else {
            let ended = Ended {
                task,
                located: &located,
                now,
                settled,
                progress: run.progress,
            };
            self.settled_status(ended, &mut events)
                .instrument(settle_span.clone())
                .await?
        };
        let status = halt.map_or(status, |halt| after_halt(halt, status));
        self.move_to(task, &located.team, status, &mut events)?;
        if matches!(located.role, Role::Solo | Role::Coordinator)
            && let Some(pulls) = self.pulls.clone()
        {
            let mut look = teams::Look::default();
            self.read_fix_responses(&pulls, &located.team, &mut look).await?;
            events.extend(look.events);
        }
        self.commit(task, &located, now, events, notice).await?;
        if located.role == Role::Reviewer
            && held == plan::PlanHeld::No
            && matches!(
                end,
                RunEnd::Reported {
                    outcome: RunOutcome::Done
                }
            )
        {
            self.review_ended(task, &located, run.final_message.as_deref()).await?;
        }
        self.retry_unanswered_opinion(task, &located, &end).await?;
        if held == plan::PlanHeld::Yes {
            // The run said DONE but goes back for its plan, or waits for the human: nobody is told it is done.
            self.look_at(&located.team);
            return Ok(());
        }
        self.after_run(task, &located, &end, run.final_message.as_deref())
    }

    /// Replaces a failed opinion while preserving ordinary task settlement.
    async fn retry_unanswered_opinion(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
    ) -> Result<(), OwnerError> {
        if !matches!(
            end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        ) && let Some(pulls) = self.pulls.clone()
        {
            let failure = match end {
                RunEnd::NoStatus { .. } => agent_core::state::OpinionFailure::NoStatus,
                RunEnd::Hung => agent_core::state::OpinionFailure::Silent,
                RunEnd::Reported { .. }
                | RunEnd::QuotaExhausted { .. }
                | RunEnd::RateLimited
                | RunEnd::TurnLimit
                | RunEnd::Stopped => agent_core::state::OpinionFailure::Failed,
            };
            self.retry_opinion(&pulls, &located.team, task, failure).await?;
        }
        Ok(())
    }

    /// Builds the `tracing::Span` for settling the current run of `task` in `team`.
    fn settle_span_of(&self, task: &TaskId, team: &TeamId) -> tracing::Span {
        let (run_id, run_rounds) = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.current_run())
            .map_or_else(
                || (String::new(), String::new()),
                |current| {
                    let rounds_str = current
                        .rounds()
                        .iter()
                        .map(RoundId::as_str)
                        .collect::<Vec<_>>()
                        .join(",");
                    (current.id().as_str().to_owned(), rounds_str)
                },
            );
        tracing::error_span!(
            "run_settle",
            team = %team,
            rounds = %run_rounds,
            task = %task,
            run = %run_id
        )
    }

    /// The status a run's end sends its task to, and what the human is told of it.
    async fn settled_status(
        &mut self,
        ended: Ended<'_>,
        events: &mut Vec<EventKind>,
    ) -> Result<(TaskStatus, Option<Notice>), OwnerError> {
        let Ended {
            task,
            located,
            now,
            settled,
            progress,
        } = ended;
        let next = match settled {
            Settled::Status(status) => {
                self.hangs.remove(task);
                (status, None)
            }
            Settled::NoStatus { exit_code } => {
                self.hangs.remove(task);
                self.no_status(task, located.role, exit_code)?
            }
            Settled::Failed(notice) => {
                self.hangs.remove(task);
                (TaskStatus::Failed, Some(notice))
            }
            Settled::Questions => {
                self.hangs.remove(task);
                match self.ask(task, located, now, events).await? {
                    Asked::Recorded(notice) => (TaskStatus::Questions, Some(notice)),
                    Asked::Unusable(notice) => (TaskStatus::Failed, Some(notice)),
                }
            }
            Settled::Paused(pause) => {
                self.hangs.remove(task);
                self.wakes.insert(task.clone(), pause.until);
                let notice = Notice::Paused {
                    until: pause.until,
                    reason: pause.reason,
                    evidence: pause.evidence,
                };
                (TaskStatus::Paused, Some(notice))
            }
            Settled::Hung => self.hung(task, progress)?,
            Settled::TurnLimit => {
                self.hangs.remove(task);
                self.turn_limit(task)?
            }
        };
        Ok(next)
    }

    /// Decide continuation from the persisted suffix, so a restart never resets its budget.
    fn turn_limit(&self, task: &TaskId) -> Result<(TaskStatus, Option<Notice>), OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let count = consecutive_turn_limits(found.member.runs());
        let session = found.member.runs().last().and_then(|run| run.session());
        let limits = self.parts.settings.config.limits();
        let next = match after_turn_limit(count, session, limits) {
            AfterTurnLimit::Resume => (TaskStatus::Queued, None),
            AfterTurnLimit::Exhausted => (
                TaskStatus::Failed,
                Some(Notice::TurnLimitExhausted {
                    count,
                    limit: limits.turn_limit_resumes,
                }),
            ),
            AfterTurnLimit::NoSession => (TaskStatus::Failed, Some(Notice::TurnLimitNoSession)),
        };
        Ok(next)
    }

    /// What follows a run that ended without a STATUS: line: it is resumed up to twice in a row,
    /// or fails after retries are exhausted.
    fn no_status(
        &mut self,
        task: &TaskId,
        role: Role,
        exit: Option<i32>,
    ) -> Result<(TaskStatus, Option<Notice>), OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let count = consecutive_no_status(found.member.runs());
        let session = found.member.runs().last().and_then(|run| run.session());
        let next = match after_no_status(role, count, session) {
            AfterNoStatus::Resume => {
                self.reasons.insert(task.clone(), Why::NoStatus);
                (TaskStatus::Queued, None)
            }
            AfterNoStatus::Fail => (TaskStatus::Failed, Some(Notice::NoStatus { exit })),
        };
        Ok(next)
    }

    /// What follows a hung run: it is resumed, or after the last strike the task fails and the human is told.
    fn hung(&mut self, task: &TaskId, progress: Progress) -> Result<(TaskStatus, Option<Notice>), OwnerError> {
        let watchdog = self.parts.settings.config.watchdog();
        // A run that did something before it went silent starts the count again.
        let count = match progress {
            Progress::Made => 1,
            Progress::Nothing => self.hangs.get(task).copied().unwrap_or(0).saturating_add(1),
        };
        self.hangs.insert(task.clone(), count);
        let session = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.last_session())
            .map(|session| CliSessionId::from_str(session.as_str()))
            .transpose()?;
        let decision = after_hang(count, session.as_ref(), watchdog);
        log_decision(
            "after_hang",
            &format!("count={count}, has_session={}", session.is_some()),
            &format!("{decision:?}"),
        );
        let next = match decision {
            AfterHang::Resume => (TaskStatus::Queued, None),
            AfterHang::MarkHung => {
                self.hangs.remove(task);
                let notice = Notice::Hung {
                    minutes: watchdog.hang_minutes.as_duration().as_secs() / SECS_PER_MINUTE,
                    strikes: watchdog.hang_strikes.get(),
                };
                (TaskStatus::Failed, Some(notice))
            }
        };
        Ok(next)
    }

    /// Records the questions the agent asked, when their file is usable.
    async fn ask(
        &mut self,
        task: &TaskId,
        located: &Located,
        now: Timestamp,
        events: &mut Vec<EventKind>,
    ) -> Result<Asked, OwnerError> {
        let number =
            next_number(&self.member_mut(task)?.1).ok_or_else(|| OwnerError::QuestionNumbers { task: task.clone() })?;
        let path = self.parts.store.home().questions_file(task, number);
        let unusable = |reason: String| {
            Asked::Unusable(Notice::QuestionsMissing {
                file: path.clone(),
                reason,
            })
        };
        let loaded = match questions::read(&path).await {
            Ok(loaded) => loaded,
            Err(problem) => {
                let reason = error_chain(&problem);
                tracing::warn!(
                    event = LogEvent::QuestionsFileInvalid.as_str(),
                    path = %path.display(),
                    %reason,
                    "a questions file cannot be read or is invalid"
                );
                return Ok(unusable(reason));
            }
        };
        let file = match path.to_str().map(AbsolutePath::from_str) {
            Some(Ok(file)) => file,
            Some(Err(problem)) => return Ok(unusable(error_chain(&problem))),
            None => return Ok(unusable("the path is not UTF-8 text".to_owned())),
        };
        let (_, mut member) = self.member_mut(task)?;
        member
            .ask_question(Question::new(number, file, now))
            .map_err(|source| OwnerError::Question {
                task: task.clone(),
                source,
            })?;
        events.push(EventKind::QuestionAsked {
            team: located.team.clone(),
            task: task.clone(),
            number,
        });
        self.prepare_questions(task, &path, &loaded).await;
        // What the human sees is the file with the daemon's instructions in it.
        if let Some(text) = self
            .questions_text(
                now,
                &located.team,
                task,
                &path,
                agent_core::history::EventKind::QuestionAsked,
            )
            .await
        {
            self.record_event_at(
                now,
                &located.team,
                self.rounds_of_tasks(std::slice::from_ref(task)),
                HistoryEvent::QuestionAsked {
                    task: task.clone(),
                    number,
                    text,
                },
            );
        }
        Ok(Asked::Recorded(Notice::Questions { number, file: path }))
    }

    // ---- paused tasks and shutdown --------------------------------------------------------------------------

    /// Queues the paused tasks whose time has come.
    async fn wake_due(&mut self) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let due: Vec<TaskId> = self
            .wakes
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(task, _)| task.clone())
            .collect();
        for task in due {
            self.wakes.remove(&task);
            let still_paused = self
                .parts
                .state
                .find_task(&task)
                .is_some_and(|found| found.member.status() == TaskStatus::Paused);
            if !still_paused {
                continue;
            }
            let located = self.locate(&task)?;
            let mut events = Vec::new();
            self.move_to(&task, &located.team, TaskStatus::Queued, &mut events)?;
            self.commit(&task, &located, now, events, None).await?;
        }
        Ok(())
    }

    /// Waits for the runs, which are stopping because the shutdown is cancelled, and records how each ended.
    async fn stop_runs(&mut self) -> Result<(), OwnerError> {
        self.shutdown.cancel();
        while let Some(joined) = self.runs.join_next_with_id().await {
            self.settle_joined(joined).await?;
        }
        Ok(())
    }

    // ---- requests -------------------------------------------------------------------------------------------

    /// Answers one message of a connection.
    ///
    /// # Errors
    /// [`OwnerError`] when a request changes the state and it cannot be saved.
    async fn handle(&mut self, message: Message) -> Result<(), OwnerError> {
        let span = owner_request_span();
        async move {
            match message {
                Message::Request { client, command, reply } => {
                    // A gate is answered when it has run, by the service: the reply goes with the job.
                    let command = match command {
                        Command::Gate(request) => {
                            self.gate_request(request, reply);
                            return Ok(());
                        }
                        other => other,
                    };
                    let answer = match command {
                        Command::Run(request) => self.run_request(request).await?,
                        Command::Answer(request) => self
                            .answer_request(&client, request)
                            .await?
                            .map(|(reply, _split)| reply),
                        Command::Stop(request) => self.control_request(&client, Steer::Stop, &request.target).await?,
                        Command::Pause(request) => self.control_request(&client, Steer::Pause, &request.target).await?,
                        Command::Unpause(request) => {
                            self.control_request(&client, Steer::Unpause, &request.target).await?
                        }
                        Command::Resume(request) => {
                            if self.resume_waits(&request.target) > 0 {
                                // The resume is for the steps that waited: they are tried at the next look, and no
                                // task of the team is started again for it.
                                Ok(Reply::Applied { tasks: Vec::new() })
                            } else {
                                self.control_request(&client, Steer::Resume, &request.target).await?
                            }
                        }
                        Command::Say(request) => self.say_request(&client, request).await?,
                        other => self.answer(&other),
                    };
                    if let Err(ref failure) = answer {
                        tracing::warn!(
                            event = LogEvent::RequestRefused.as_str(),
                            kind = ?failure.kind,
                            reason = %failure.message,
                            "a client request was refused"
                        );
                    }
                    if reply.send(answer).is_err() {
                        // The connection closed while its request waited: nobody is left to answer.
                        tracing::debug!(
                            event = LogEvent::PeerGone.as_str(),
                            "the connection of a request closed before the answer"
                        );
                    }
                }
                Message::Answer { client, request, reply } => {
                    let answer = self.answer_request(&client, request).await?;
                    if let Err(ref failure) = answer {
                        tracing::warn!(event = LogEvent::RequestRefused.as_str(),
                            kind = ?failure.kind, reason = %failure.message, "an answer request was refused");
                    }
                    if reply.send(answer).is_err() {
                        // The connection closed while the answer was being recorded: there is nobody left to notify.
                        tracing::debug!(event = LogEvent::PeerGone.as_str(), "the answer connection closed");
                    }
                }
                Message::Subscribe { team, reply } => {
                    // The receiver is made before the teams are read, and nothing changes between: no event is missed.
                    let events = self.parts.events.subscribe();
                    let teams = self.summaries(team.as_ref());
                    if reply.send((teams, events)).is_err() {
                        tracing::debug!(
                            event = LogEvent::PeerGone.as_str(),
                            "the connection of a subscription closed before the answer"
                        );
                    }
                }
            }
            Ok(())
        }
        .instrument(span)
        .await
    }

    /// The summaries of the teams, or of the one named.
    fn summaries(&self, team: Option<&TeamId>) -> Vec<TeamSummary> {
        self.parts
            .state
            .teams()
            .iter()
            .filter(|known| team.is_none_or(|wanted| known.id() == wanted))
            .map(TeamSummary::from)
            .collect()
    }

    /// The answer to a request.
    fn answer(&self, command: &Command) -> Result<Reply, Failure> {
        match command {
            Command::Status(request) => self.status(request.target.as_ref()),
            Command::Show(request) => self.show(&request.target),
            Command::Run(_) => Err(Failure {
                kind: FailureKind::Internal,
                message: "a run is started by the owner, not answered from the state".to_owned(),
            }),
            Command::Answer(_) => Err(Failure {
                kind: FailureKind::Internal,
                message: "answers are taken by the owner, not answered from the state".to_owned(),
            }),
            Command::Stop(_) | Command::Pause(_) | Command::Unpause(_) | Command::Resume(_) | Command::Say(_) => {
                Err(Failure {
                    kind: FailureKind::Internal,
                    message: "the controls are carried out by the owner, not answered from the state".to_owned(),
                })
            }
            Command::Gate(_) => Err(Failure {
                kind: FailureKind::Internal,
                message: "a gate is queued by the owner, not answered from the state".to_owned(),
            }),
            Command::Subscribe(_) => Err(Failure {
                kind: FailureKind::Internal,
                message: "a subscription is made by the connection, not asked of the owner".to_owned(),
            }),
        }
    }

    /// The teams, all or the one a target belongs to.
    fn status(&self, target: Option<&Target>) -> Result<Reply, Failure> {
        let teams = match target {
            None => self.summaries(None),
            Some(Target::Team(team)) => {
                let found = self.summaries(Some(team));
                if found.is_empty() {
                    return Err(not_found(format!("there is no team {team}")));
                }
                found
            }
            Some(Target::Task(task)) => {
                let found = self
                    .parts
                    .state
                    .find_task(task)
                    .map(|found| TeamSummary::from(found.team))
                    .ok_or_else(|| not_found(format!("there is no task {task}")))?;
                vec![found]
            }
            Some(Target::Round(round)) => {
                let found = self
                    .parts
                    .state
                    .find_round(round)
                    .map(|(team, _)| TeamSummary::from(team))
                    .ok_or_else(|| not_found(format!("there is no round {round}")))?;
                vec![found]
            }
        };
        Ok(match health_to_tell(&self.history) {
            Some(history) => Reply::Overview { teams, history },
            None => Reply::Teams { teams },
        })
    }

    /// A whole team, or one task with its role, or one round with the tasks that worked for it.
    fn show(&self, target: &Target) -> Result<Reply, Failure> {
        match target {
            Target::Team(team) => self
                .parts
                .state
                .team(team)
                .map(|found| Reply::Team {
                    team: Box::new(found.clone()),
                })
                .ok_or_else(|| not_found(format!("there is no team {team}"))),
            Target::Task(task) => self
                .parts
                .state
                .find_task(task)
                .map(|found| Reply::Task {
                    team: found.team.id().clone(),
                    role: found.role,
                    member: Box::new(found.member.clone()),
                })
                .ok_or_else(|| not_found(format!("there is no task {task}"))),
            Target::Round(round) => {
                let (team, found) = self
                    .parts
                    .state
                    .find_round(round)
                    .ok_or_else(|| not_found(format!("there is no round {round}")))?;
                let tasks = self.round_tasks(team, round)?;
                let mut detail = RoundDetail::new(team, found, tasks);
                detail.waiting = self
                    .watch
                    .github_waits
                    .line_of(team.id(), round, self.parts.local_offset)
                    .map(|line| format!("GitHub: {line}"))
                    .or_else(|| self.waiting_for(team, found).map(|wait| wait.text().to_owned()));
                Ok(Reply::Round {
                    round: Box::new(detail),
                })
            }
        }
    }

    /// The tasks of `team` that worked for `round`, each with its worktree, its branch and the runs that named the round.
    fn round_tasks(&self, team: &Team, round: &RoundId) -> Result<Vec<RoundTask>, Failure> {
        let internal = |message: String| Failure {
            kind: FailureKind::Internal,
            message,
        };
        team.members_in(round)
            .into_iter()
            .map(|(role, member)| {
                let path = self.parts.store.home().worktree_dir(team.repo(), member.task());
                let worktree = path
                    .to_str()
                    .ok_or_else(|| internal(format!("the worktree {} is not UTF-8 text", path.display())))
                    .and_then(|text| {
                        AbsolutePath::from_str(text).map_err(|error| {
                            internal(format!("the worktree {} cannot be used: {error}", path.display()))
                        })
                    })?;
                let branch = start::work_branch(team, member).map_err(|error| {
                    internal(format!("the branch of task {}: {}", member.task(), error_chain(&error)))
                })?;
                Ok(RoundTask {
                    task: member.task().clone(),
                    role,
                    agent: member.agent().clone(),
                    worktree,
                    branch,
                    runs: member
                        .runs()
                        .iter()
                        .filter(|run| run.rounds().contains(round))
                        .cloned()
                        .collect(),
                })
            })
            .collect()
    }

    // ---- the state ------------------------------------------------------------------------------------------

    /// Where `task` is: its team, its role and its agent.
    fn locate(&self, task: &TaskId) -> Result<Located, OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        Ok(Located {
            team: found.team.id().clone(),
            role: found.role,
            agent: found.member.agent().name().to_owned(),
        })
    }

    /// The context needed to lay out a notification about `task`.
    pub(super) fn notice_context<'a>(
        &'a self,
        task: &'a TaskId,
        time: Timestamp,
        pr: Option<&'a str>,
    ) -> Result<NoticeContext<'a>, OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let (cli, model) = match found.member.agent() {
            agent_core::state::Participant::Normal(name) | agent_core::state::Participant::Worker(name) => {
                self.parts.settings.config.agent(name).map_or((None, None), |a| {
                    (Some(a.cli), a.model.as_ref().map(agent_core::config::ModelName::as_str))
                })
            }
            agent_core::state::Participant::Investigator(name) => {
                self.parts.settings.config.investigator(name).map_or((None, None), |a| {
                    (Some(a.cli), a.model.as_ref().map(agent_core::config::ModelName::as_str))
                })
            }
        };
        let team = found.team.crew().coordinator().is_some().then_some(found.team.id());
        Ok(NoticeContext {
            repo: found.team.repo().as_str(),
            agent: found.member.agent().name(),
            cli,
            model,
            time,
            offset: self.parts.local_offset,
            task,
            team,
            pr,
        })
    }

    /// The member of `task`, to change.
    fn member_mut(&mut self, task: &TaskId) -> Result<(Role, agent_core::state::MemberMut<'_>), OwnerError> {
        self.parts
            .state
            .find_task_mut(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })
    }

    /// Changes the status of `task`, and says so in `events`.
    fn move_to(
        &mut self,
        task: &TaskId,
        team: &TeamId,
        status: TaskStatus,
        events: &mut Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        let rounds = self
            .rounds_of_tasks(std::slice::from_ref(task))
            .iter()
            .map(RoundId::as_str)
            .collect::<Vec<_>>()
            .join(",");
        let (_, mut member) = self.member_mut(task)?;
        let from = member.status();
        member.transition(status).map_err(|source| OwnerError::Status {
            task: task.clone(),
            source,
        })?;
        let span = tracing::error_span!("task", team = %team, rounds = %rounds, task = %task);
        {
            let _entered = span.enter();
            tracing::info!(
                event = LogEvent::TaskStatusChanged.as_str(),
                from = %from.as_str(),
                to = %status.as_str(),
                "task status changed"
            );
        }
        events.push(EventKind::TaskStatus {
            team: team.clone(),
            task: task.clone(),
            status,
        });
        Ok(())
    }

    /// Records how the run that is going ended, and says so in `events`.
    fn end_run(
        &mut self,
        task: &TaskId,
        team: &TeamId,
        at: Timestamp,
        end: RunEnd,
        final_message: Option<&str>,
        events: &mut Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        let (_, mut member) = self.member_mut(task)?;
        let (run, started_at, rounds) = member
            .current_run()
            .map(|run| (run.id().clone(), run.started_at(), run.rounds().to_vec()))
            .ok_or_else(|| OwnerError::Run {
                task: task.clone(),
                source: RunError::NotRunning,
            })?;
        // A clock that was set back during the run must not end it before it started.
        member
            .end_run(at.max(started_at), end.clone())
            .map_err(|source| OwnerError::Run {
                task: task.clone(),
                source,
            })?;
        let rounds_field = rounds.iter().map(RoundId::as_str).collect::<Vec<_>>().join(",");
        let span = tracing::error_span!(
            "run",
            team = %team,
            rounds = %rounds_field,
            task = %task,
            run = %run
        );
        {
            let _entered = span.enter();
            tracing::info!(
                event = LogEvent::RunEnded.as_str(),
                task = %task,
                run = %run,
                end = ?end,
                "run ended"
            );
        }
        // A clock that was set back must not end the run before it started in the history either.
        self.record_run_end(&run, at.max(started_at), &end, final_message);
        events.push(EventKind::RunEnded {
            team: team.clone(),
            task: task.clone(),
            run,
            rounds,
            end,
        });
        Ok(())
    }

    /// Records the session the CLI named for the run that is going, when it is not the one the run began with.
    fn attach_session(&mut self, task: &TaskId, session: Option<CliSessionId>) -> Result<(), OwnerError> {
        let Some(session) = session else {
            return Ok(());
        };
        let session = SessionId::from_str(session.as_str())?;
        let (_, mut member) = self.member_mut(task)?;
        if member.current_run().and_then(|run| run.session()) == Some(&session) {
            return Ok(());
        }
        member.attach_session(session).map_err(|source| OwnerError::Run {
            task: task.clone(),
            source,
        })
    }

    /// Saves the state, then tells the subscribers and the human what changed.
    async fn commit(
        &mut self,
        task: &TaskId,
        located: &Located,
        at: Timestamp,
        events: Vec<EventKind>,
        notice: Option<Notice>,
    ) -> Result<(), OwnerError> {
        self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
        self.save().await?;
        for kind in events {
            self.publish(at, kind);
        }
        if let Some(team) = self.parts.state.team(&located.team) {
            self.publish(
                at,
                EventKind::TeamUpdated {
                    summary: Box::new(TeamSummary::from(team)),
                },
            );
        }
        if let Some(notice) = notice {
            match self.notice_context(task, at, None) {
                Ok(context) => match notice.render(&self.parts.texts, &context) {
                    Ok(notification) => self.notify(at, located, task, notification),
                    Err(error) => tracing::error!(
                        event = LogEvent::NotificationFailed.as_str(),
                        task = %task,
                        error = %error_chain(&error),
                        "the notification of a task cannot be made"
                    ),
                },
                Err(error) => tracing::error!(
                    event = LogEvent::NotificationFailed.as_str(),
                    task = %task,
                    error = %error_chain(&error),
                    "a notification has no task context"
                ),
            }
        }
        Ok(())
    }

    /// Saves the state.
    async fn save(&self) -> Result<(), OwnerError> {
        if let Err(error) = self.parts.store.save(&self.parts.state).await {
            let chain = error_chain(&error);
            tracing::error!(
                event = LogEvent::StateSaveFailed.as_str(),
                error = %chain,
                "state.json cannot be saved"
            );
            if let Err(notify_err) = self.parts.notifier.notify_emergency(&chain).await {
                tracing::warn!(
                    event = LogEvent::NotificationFailed.as_str(),
                    error = %error_chain(&notify_err),
                    "emergency notification failed"
                );
            }
            return Err(OwnerError::Store(error));
        }
        Ok(())
    }

    /// Sends an event to the subscribers.
    fn publish(&self, at: Timestamp, kind: EventKind) {
        if self.parts.events.send(Event::new(at, kind)).is_err() {
            // `send` fails when nobody is subscribed: the event is for nobody.
            tracing::trace!("an event had no subscriber");
        }
    }

    /// Queues a notification for the human.
    fn notify(&self, at: Timestamp, located: &Located, task: &TaskId, notification: Notification) {
        let delivery = Delivery {
            at,
            team: located.team.clone(),
            rounds: self.rounds_of_tasks(std::slice::from_ref(task)),
            task: task.clone(),
            notification,
        };
        if let Err(error) = self.parts.notices.try_send(delivery) {
            // The human is told through files and a command, not through the state: a notification that cannot be
            // queued is logged, and the task goes on.
            tracing::error!(
                event = LogEvent::NotificationFailed.as_str(),
                task = %task,
                %error,
                "a notification could not be queued"
            );
        }
    }

    /// Notifies the human that open rounds wait for disk space.
    pub(super) async fn notify_rounds_waiting(&mut self, free_bytes: u64, min_gb: u64) {
        let free_gb = free_bytes / GIBIBYTE;
        let values = match agent_core::prompt::Variables::new()
            .with("free", free_gb.to_string())
            .and_then(|v| v.with("min", min_gb.to_string()))
        {
            Ok(values) => values,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "cannot build the variables of the disk wait notice");
                return;
            }
        };
        let rendered = match self
            .parts
            .texts
            .render(agent_core::prompt::HarnessPrompt::NoticeDiskWait, &values)
        {
            Ok(rendered) => rendered,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "cannot render the disk wait notice");
                return;
            }
        };
        let mut split = rendered.splitn(2, '\n');
        let title = split.next().unwrap_or("Rounds waiting for free disk space").trim();
        let body = split.next().unwrap_or_default().trim();
        self.deliver(title, body, "disk wait notice").await;
    }
}

/// Emits the corresponding `warn` event when a run ended without a status line, hit a limit, or hung.
fn log_unusual_end(end: &RunEnd) {
    match end {
        RunEnd::NoStatus { exit_code } => {
            tracing::warn!(
                event = LogEvent::RunEndedWithoutStatus.as_str(),
                exit_code = ?exit_code,
                "a run ended without a STATUS line"
            );
        }
        RunEnd::QuotaExhausted { resets_at } => {
            tracing::warn!(
                event = LogEvent::RunQuotaExhausted.as_str(),
                resets_at = ?resets_at,
                "a run hit a quota limit and paused its task"
            );
        }
        RunEnd::RateLimited => {
            tracing::warn!(
                event = LogEvent::RunRateLimited.as_str(),
                "a run hit a rate limit and paused its task"
            );
        }
        RunEnd::Hung => {
            tracing::warn!(
                event = LogEvent::RunHung.as_str(),
                "a run hung and was stopped by the watchdog"
            );
        }
        RunEnd::Reported { .. } | RunEnd::Stopped | RunEnd::TurnLimit => {}
    }
}

/// The answer for something that is not there.
fn not_found(message: String) -> Failure {
    Failure {
        kind: FailureKind::NotFound,
        message,
    }
}

/// The callsite of an owner request span.
static OWNER_REQUEST_CALLSITE: tracing::callsite::DefaultCallsite =
    tracing::callsite::DefaultCallsite::new(&OWNER_REQUEST_METADATA);

/// The metadata of an owner request span.
static OWNER_REQUEST_METADATA: tracing::Metadata<'static> = tracing::Metadata::new(
    "request",
    module_path!(),
    tracing::Level::DEBUG,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    tracing::field::FieldSet::new(&[], tracing::callsite::Identifier(&OWNER_REQUEST_CALLSITE)),
    tracing::metadata::Kind::SPAN,
);

/// Builds the span around one owner request through [`tracing::Span::new`] so a subscriber installed with
/// [`tracing::subscriber::with_default`] sees it even when another thread without a subscriber ran first.
fn owner_request_span() -> tracing::Span {
    agent_runtime::history::ensure_scoped_callsites();
    let fields = OWNER_REQUEST_METADATA.fields();
    let pairs: [(&tracing::field::Field, Option<&dyn tracing::field::Value>); 0] = [];
    tracing::Span::new(&OWNER_REQUEST_METADATA, &fields.value_set(&pairs))
}
