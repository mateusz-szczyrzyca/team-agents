//! The owner of the state: the one task that reads and changes it.
//!
//! Everything that wants the state asks the owner through a channel (CON-3): the connections of the socket for their
//! requests, the engine tasks through their results. The owner answers requests, starts a run for every queued task,
//! applies the end of each run to its task, wakes paused tasks when their time comes and publishes what happens as
//! events. A state it cannot save ends the daemon: models must not go on working with a state that is not kept.

mod questions;
mod start;
mod teams;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::cli::SessionId as CliSessionId;
use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::decide::outcome::{AfterHang, after_hang};
use agent_core::ids::{InvalidName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::protocol::{Command, Event, EventKind, Failure, FailureKind, Reply, Target, TeamSummary};
use agent_core::state::InvalidState;
use agent_core::state::{InvalidTimestamp, Question, QuestionError, RunEnd, RunError, SessionId, State, Timestamp};
use agent_core::status::{InvalidTransition, TaskStatus};
use agent_runtime::agentbus::AgentBus;
use agent_runtime::clock::SharedClock;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::run::{FinishedRun, Progress, RunEngine, RunError as EngineError, RunVerdict};
use agent_runtime::settings::Settings;
use agent_runtime::store::{StateStore, StoreError};
use agent_runtime::team_prompts::TeamPrompts;
use rand::rngs::StdRng;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::{Id as JoinId, JoinError, JoinSet};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use self::start::{Context, Plan, next_round, plan};
use self::teams::{Look, Outbound, Served};
use super::notice::Notice;
use super::notifier::Delivery;
use super::recovery::{RecoveryError, recover};
use super::settle::{Settled, settle};
use super::team::{AssignmentError, DoneNoticeError, NoticeRenderError, Why};
use agent_runtime::process::error_chain;

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
        /// What it asks.
        command: Command,
        /// Where the answer goes.
        reply: oneshot::Sender<Result<Reply, Failure>>,
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
    /// A round of questions cannot be recorded.
    #[error("the questions of task {task} cannot be recorded")]
    Question {
        /// The task.
        task: TaskId,
        /// What the state said.
        source: QuestionError,
    },
    /// A session ID is not valid.
    #[error("a session ID is not valid")]
    Session(#[from] InvalidValue),
    /// The question rounds of a task are used up.
    #[error("task {task} has no round of questions left")]
    Rounds {
        /// The task.
        task: TaskId,
    },
    /// A run ended that the owner did not start.
    #[error("the run task {id} is not one the owner started")]
    UnknownRun {
        /// The task's ID in the runtime.
        id: JoinId,
    },
    /// A new team cannot be put in the state: a mistake of the daemon.
    #[error("the team cannot be put in the state")]
    InsertTeam(#[source] InvalidState),
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
    /// The rules.
    pub(super) rules: Arc<PromptLoader>,
    /// The bus of the home: the teams' members, their messages and their requests for workers.
    pub(super) bus: AgentBus,
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
    /// How often paused tasks are looked at.
    pub(super) tick: Duration,
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

/// How a round of questions went.
enum Asked {
    /// The round is recorded and the human is told.
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
    /// When each paused task is queued again.
    wakes: HashMap<TaskId, Timestamp>,
    /// How many hangs in a row each task has had without progress.
    hangs: HashMap<TaskId, u32>,
    /// Why a queued task of a team starts now; read when its run is made ready.
    reasons: HashMap<TaskId, Why>,
    /// What is still to be done on the bus, oldest first: it is tried again at every look until it is done.
    outbox: VecDeque<Outbound>,
    /// The requests for workers that are being served, by the task of the worker.
    serving: HashMap<TaskId, Served>,
    /// The teams that are finished and closed: the human was told.
    closed: HashSet<TeamId>,
    /// The teams to look at next, and why.
    looks: BTreeMap<TeamId, Look>,
    /// Cancelled when the daemon is to stop: the runs it starts stop with it.
    shutdown: CancellationToken,
}

impl Owner {
    /// An owner made of `parts`.
    pub(super) fn new(parts: OwnerParts, shutdown: CancellationToken) -> Self {
        Self {
            parts,
            runs: JoinSet::new(),
            running: HashMap::new(),
            in_flight: HashSet::new(),
            wakes: HashMap::new(),
            hangs: HashMap::new(),
            reasons: HashMap::new(),
            outbox: VecDeque::new(),
            serving: HashMap::new(),
            closed: HashSet::new(),
            looks: BTreeMap::new(),
            shutdown,
        }
    }

    /// Serves `inbox` and runs the tasks until the shutdown is cancelled; then stops the runs and records them.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn run(mut self, mut inbox: mpsc::Receiver<Message>) -> Result<(), OwnerError> {
        self.recover().await?;
        let mut tick = tokio::time::interval(self.parts.tick.max(Duration::from_millis(1)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            self.team_life().await?;
            self.start_queued().await?;
            tokio::select! {
                () = self.shutdown.cancelled() => break,
                message = inbox.recv() => match message {
                    Some(message) => self.handle(message).await?,
                    None => break,
                },
                Some(joined) = self.runs.join_next_with_id() => self.settle_joined(joined).await?,
                _ = tick.tick() => {
                    self.wake_due().await?;
                    self.look_at_every_team(Look::Poll);
                }
            }
        }
        self.stop_runs().await
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
        self.remember_closed_teams();
        self.look_at_every_team(Look::Event);
        if !recovery.interrupted.is_empty() {
            tracing::info!(
                tasks = recovery.interrupted.len(),
                "runs that a restart interrupted are queued again"
            );
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
            .filter(|task| !self.in_flight.contains(task))
            .collect();
        for task in queued {
            if self.runs.len() >= MAX_RUNS.get() {
                break;
            }
            self.start(task).await?;
        }
        Ok(())
    }

    /// Starts the run of `task`, or fails the task when it cannot be made ready.
    async fn start(&mut self, task: TaskId) -> Result<(), OwnerError> {
        let why = self.reasons.remove(&task);
        let planned = {
            let context = Context {
                state: &self.parts.state,
                settings: &self.parts.settings,
                texts: &self.parts.texts,
                team_texts: &self.parts.team_texts,
                rules: &self.parts.rules,
                home: self.parts.store.home(),
                clock: &self.parts.clock,
                why: why.as_ref(),
            };
            plan(&context, &mut self.parts.rng, &task).await
        };
        let plan = match planned {
            Ok(plan) => plan,
            Err(problem) => return self.fail_to_start(&task, error_chain(&problem)).await,
        };
        self.begin(&task, plan).await
    }

    /// Records the start of the run of `plan` and hands it to the engine.
    async fn begin(&mut self, task: &TaskId, plan: Plan) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        {
            let (_, mut member) = self.member_mut(task)?;
            member
                .begin_run(plan.run.clone(), now, plan.recorded_session)
                .map_err(|source| OwnerError::Run {
                    task: task.clone(),
                    source,
                })?;
        }
        events.push(EventKind::RunStarted {
            team: located.team.clone(),
            task: task.clone(),
            run: plan.run.clone(),
        });
        self.move_to(task, &located.team, TaskStatus::Running, &mut events)?;
        self.commit(task, &located, now, events, None).await?;
        tracing::info!(task = %task, run = %plan.run, agent = %located.agent, "run started");
        let engine = self.parts.engine.clone();
        let cancel = self.shutdown.child_token();
        let job = plan.job;
        let handle = self.runs.spawn(async move { engine.run(job, &cancel).await });
        self.running.insert(handle.id(), task.clone());
        self.in_flight.insert(task.clone());
        Ok(())
    }

    /// Fails `task`, which could not be started, and tells the human why; its team learns it as of a run that ended
    /// without a status: the coordinator about a member, the requester about the request of a worker.
    async fn fail_to_start(&mut self, task: &TaskId, reason: String) -> Result<(), OwnerError> {
        tracing::error!(task = %task, %reason, "a run cannot be started");
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.move_to(task, &located.team, TaskStatus::Running, &mut events)?;
        self.move_to(task, &located.team, TaskStatus::Failed, &mut events)?;
        self.commit(task, &located, now, events, Some(Notice::StartFailed { reason }))
            .await?;
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
        match outcome {
            Outcome::Verdict(verdict) => match *verdict {
                RunVerdict::Finished(run) => self.finished(&task, *run).await,
                RunVerdict::Interrupted { session } => self.interrupted(&task, session).await,
            },
            Outcome::Failed(reason) => self.failed(&task, reason).await,
        }
    }

    /// A run that could not be carried out: its task fails.
    async fn failed(&mut self, task: &TaskId, reason: String) -> Result<(), OwnerError> {
        tracing::error!(task = %task, %reason, "a run failed");
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.end_run(
            task,
            &located.team,
            now,
            RunEnd::NoStatus { exit_code: None },
            &mut events,
        )?;
        self.move_to(task, &located.team, TaskStatus::Failed, &mut events)?;
        self.hangs.remove(task);
        self.commit(task, &located, now, events, Some(Notice::RunFailed { reason }))
            .await?;
        self.after_run(task, &located, &RunEnd::NoStatus { exit_code: None }, None)
    }

    /// A run the daemon stopped because it shuts down: recorded as interrupted, and its task queued for the next start.
    async fn interrupted(&mut self, task: &TaskId, session: Option<CliSessionId>) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.attach_session(task, session)?;
        self.end_run(
            task,
            &located.team,
            now,
            RunEnd::NoStatus { exit_code: None },
            &mut events,
        )?;
        self.move_to(task, &located.team, TaskStatus::Queued, &mut events)?;
        self.commit(task, &located, now, events, None).await
    }

    /// A run that went to its end: its task goes where the end says.
    async fn finished(&mut self, task: &TaskId, run: FinishedRun) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        tracing::info!(task = %task, end = ?run.classified.end, "run ended");
        self.attach_session(task, run.session.clone())?;
        let settled = settle(
            located.role,
            &run.classified.end,
            run.classified.pause.as_ref(),
            run.final_message.as_deref(),
        );
        let settled = self.solo_waits_for_its_items(located.role, &located.team, settled);
        self.end_run(task, &located.team, now, run.classified.end.clone(), &mut events)?;
        let (status, notice) = match settled {
            Settled::Status(status) => {
                self.hangs.remove(task);
                (status, None)
            }
            Settled::Failed(notice) => {
                self.hangs.remove(task);
                (TaskStatus::Failed, Some(notice))
            }
            Settled::Questions => {
                self.hangs.remove(task);
                match self.ask(task, &located, now, &mut events).await? {
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
            Settled::Hung => self.hung(task, run.progress)?,
        };
        self.move_to(task, &located.team, status, &mut events)?;
        self.commit(task, &located, now, events, notice).await?;
        self.after_run(task, &located, &run.classified.end, run.final_message.as_deref())
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
        let next = match after_hang(count, session.as_ref(), watchdog) {
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

    /// Records the round of questions the agent asked, when its file is usable.
    async fn ask(
        &mut self,
        task: &TaskId,
        located: &Located,
        now: Timestamp,
        events: &mut Vec<EventKind>,
    ) -> Result<Asked, OwnerError> {
        let round = next_round(&self.member_mut(task)?.1).ok_or_else(|| OwnerError::Rounds { task: task.clone() })?;
        let path = self.parts.store.home().questions_file(task, round);
        let unusable = |reason: String| {
            Asked::Unusable(Notice::QuestionsMissing {
                file: path.clone(),
                reason,
            })
        };
        if let Err(problem) = questions::read(&path).await {
            return Ok(unusable(error_chain(&problem)));
        }
        let file = match path.to_str().map(AbsolutePath::from_str) {
            Some(Ok(file)) => file,
            Some(Err(problem)) => return Ok(unusable(error_chain(&problem))),
            None => return Ok(unusable("the path is not UTF-8 text".to_owned())),
        };
        let (_, mut member) = self.member_mut(task)?;
        member
            .ask_question(Question::new(round, file, now))
            .map_err(|source| OwnerError::Question {
                task: task.clone(),
                source,
            })?;
        events.push(EventKind::QuestionAsked {
            team: located.team.clone(),
            task: task.clone(),
            round,
        });
        Ok(Asked::Recorded(Notice::Questions { round, file: path }))
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
        match message {
            Message::Request { command, reply, .. } => {
                let answer = match command {
                    Command::Run(request) => self.run_request(request).await?,
                    other => self.answer(&other),
                };
                if reply.send(answer).is_err() {
                    // The connection closed while its request waited: nobody is left to answer.
                    tracing::debug!("the connection of a request closed before the answer");
                }
            }
            Message::Subscribe { team, reply } => {
                // The receiver is made before the teams are read, and nothing changes between: no event is missed.
                let events = self.parts.events.subscribe();
                let teams = self.summaries(team.as_ref());
                if reply.send((teams, events)).is_err() {
                    tracing::debug!("the connection of a subscription closed before the answer");
                }
            }
        }
        Ok(())
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
            Command::Answer(_) => Err(not_served("answer")),
            Command::Stop(_) => Err(not_served("stop")),
            Command::Pause(_) => Err(not_served("pause")),
            Command::Unpause(_) => Err(not_served("unpause")),
            Command::Resume(_) => Err(not_served("resume")),
            Command::Say(_) => Err(not_served("say")),
            Command::Gate(_) => Err(not_served("gate")),
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
        };
        Ok(Reply::Teams { teams })
    }

    /// A whole team, or one task with its role.
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
        }
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
        let (_, mut member) = self.member_mut(task)?;
        member.transition(status).map_err(|source| OwnerError::Status {
            task: task.clone(),
            source,
        })?;
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
        events: &mut Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        let (_, mut member) = self.member_mut(task)?;
        let (run, started_at) = member
            .current_run()
            .map(|run| (run.id().clone(), run.started_at()))
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
        events.push(EventKind::RunEnded {
            team: team.clone(),
            task: task.clone(),
            run,
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
            self.notify(at, located, task, notice);
        }
        Ok(())
    }

    /// Saves the state.
    async fn save(&self) -> Result<(), OwnerError> {
        self.parts
            .store
            .save(&self.parts.state)
            .await
            .map_err(OwnerError::Store)
    }

    /// Sends an event to the subscribers.
    fn publish(&self, at: Timestamp, kind: EventKind) {
        if self.parts.events.send(Event::new(at, kind)).is_err() {
            // `send` fails when nobody is subscribed: the event is for nobody.
            tracing::trace!("an event had no subscriber");
        }
    }

    /// Queues a notification for the human.
    fn notify(&self, at: Timestamp, located: &Located, task: &TaskId, notice: Notice) {
        let delivery = Delivery {
            at,
            agent: located.agent.clone(),
            task: task.clone(),
            notice,
        };
        if let Err(error) = self.parts.notices.try_send(delivery) {
            // The human is told through files and a command, not through the state: a notification that cannot be
            // queued is logged, and the task goes on.
            tracing::error!(task = %task, %error, "a notification could not be queued");
        }
    }
}

/// The refusal of a command the daemon does not serve yet.
fn not_served(name: &str) -> Failure {
    Failure {
        kind: FailureKind::Refused,
        message: format!("this daemon does not serve `{name}` yet"),
    }
}

/// The answer for something that is not there.
fn not_found(message: String) -> Failure {
    Failure {
        kind: FailureKind::NotFound,
        message,
    }
}
