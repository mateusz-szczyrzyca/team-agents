//! The briefing of a task the ladder started anew: what its runs left, gathered from facts just before the run of its
//! new session begins — the snapshot of the worktree first, then the commits of its branch, the files left
//! uncommitted, its plan of steps and the messages of its team that no run of it read.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::decide::ladder::Trouble;
use agent_core::history::{CommitRecord, Record};
use agent_core::ids::{AgentName, BranchName, InvalidName, ItemId, RoundId, RunId, TaskId};
use agent_core::log::LogEvent;
use agent_core::plan::judge::JudgementState;
use agent_core::plan::{PLAN_FILE, Plan, StepState, parse_plan};
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::questions::MAX_QUESTIONS_BYTES;
use agent_core::questions::{QuestionsFile, Verdict};
use agent_core::state::{InvalidTimestamp, RunEnd, Timestamp};
use agent_core::task_list::MAX_TASK_LIST_BYTES;
use agent_runtime::agentbus::{Completeness, UnreadMessage, UnreadMessagesError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{JudgementRecord, ReadError, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{
    Briefing, BriefingAnswer, BriefingCommit, BriefingMessage, BriefingNotes, BriefingRun, BriefingStep, ComposeError,
    PlanHeadings, briefing_variables_with,
};
use agent_runtime::snapshot::{SnapshotError, take_snapshot};
use agent_runtime::text_file::read_text_file;

use super::round_state;
use super::start::{StartProblem, work_branch};
use super::{Owner, OwnerError};
use crate::daemon::team::{BusJob, lead_takes_work};

/// The variable of the briefing that holds the task's own prompt, which a lead's text leaves out.
const PROMPT_VARIABLE: &str = "prompt";

/// One megabyte, in bytes: the unit of `[snapshot] warn_mb`.
const MEGABYTE: u64 = 1024 * 1024;

/// How many of the newest runs of the task the briefing lists; the older ones are left out and noted.
const RUNS_SHOWN: usize = 10;

/// How many of the newest answers of the human the briefing lists; the older ones are left out and noted.
const ANSWERS_SHOWN: usize = 10;

/// What the briefing of a task says, gathered before the run of its new session begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BriefingFacts {
    /// Why the task was started anew.
    reason: Trouble,
    /// The branch the task's branch was forked from.
    base: BranchName,
    /// The task's branch.
    branch: BranchName,
    /// The commits of the branch since its base, oldest first.
    commits: Vec<CommitRecord>,
    /// The files of the worktree that differ from its head.
    uncommitted: Vec<String>,
    /// The steps of the task's plan as id, title and state; `None` when the worktree has no plan that reads.
    plan: Option<Vec<(String, String, &'static str)>>,
    /// The messages of its team that no run of the task read.
    messages: Vec<UnreadMessage>,
    /// Whether [`BriefingFacts::messages`] are all of them or only the newest.
    completeness: Completeness,
    /// The reference of the snapshot taken of the worktree.
    snapshot: String,
}

/// Why the briefing of a task cannot be gathered.
#[derive(Debug, thiserror::Error)]
pub(super) enum BriefingProblem {
    /// The task is not in a team of the state.
    #[error("task {task} is not in the state")]
    NoTask {
        /// The task.
        task: TaskId,
    },
    /// The worktree's path is not an absolute path.
    #[error("the worktree of task {task} has no usable path")]
    Path {
        /// The task.
        task: TaskId,
        /// What is wrong with the path.
        source: InvalidValue,
    },
    /// The worktree's path is not text.
    #[error("the worktree of task {task} has a path that is not text")]
    PathText {
        /// The task.
        task: TaskId,
    },
    /// What the state names cannot be used.
    #[error("the branch or the agent of the task cannot be named")]
    Start(#[from] StartProblem),
    /// The agent's name is not an agent name.
    #[error("the agent of the task cannot be named")]
    Name(#[from] InvalidName),
    /// The snapshot of the worktree cannot be taken.
    #[error("cannot take the snapshot of the worktree")]
    Snapshot(#[from] SnapshotError),
    /// Git cannot tell the commits or the files of the worktree.
    #[error("cannot read the branch or the worktree of the task")]
    Git(#[from] GitError),
    /// The unread messages cannot be read.
    #[error("cannot read the unread messages of the task")]
    Messages(#[from] UnreadMessagesError),
    /// The clock gives no valid time for the snapshot.
    #[error("cannot read the time of the snapshot")]
    Clock(#[from] InvalidTimestamp),
    /// The rounds the next run of the task works for cannot be told.
    #[error("cannot tell the rounds the next run of the task works for")]
    Rounds(#[source] OwnerError),
    /// The runs of the task cannot be read from the history.
    #[error("cannot read the runs of the task from the history")]
    Runs(#[source] Box<ReadError>),
    /// The reader of the history's runs cannot be joined.
    #[error("cannot join the reader of the runs of the task")]
    RunsJoin(#[source] tokio::task::JoinError),
    /// The judgements of the steps cannot be read from the history.
    #[error("cannot read the step judgements of the task")]
    Judgements(#[source] OwnerError),
    /// The folder of the task cannot be read for its question files.
    #[error("cannot read the question files of the task")]
    AnswersDir(#[source] std::io::Error),
    /// A question file of the task cannot be read.
    #[error("cannot read a question file of the task")]
    Answers(#[source] agent_runtime::text_file::TextFileError),
}

impl Owner {
    /// The facts of the briefing of `task` when the run it starts now is the first of a new session the ladder started
    /// it in, with the snapshot of its worktree taken and recorded first; `None` for any other run.
    ///
    /// # Errors
    /// [`BriefingProblem`] when a fact cannot be read.
    pub(super) async fn briefing_facts(&mut self, task: &TaskId) -> Result<Option<BriefingDraft>, BriefingProblem> {
        let no_task = || BriefingProblem::NoTask { task: task.clone() };
        let found = self.parts.state.find_task(task).ok_or_else(no_task)?;
        let Some(renewals) = found.member.renewals().filter(|_| found.member.awaits_new_session()) else {
            return Ok(None);
        };
        let reason = renewals.reason();
        self.gather_facts(task, reason).await
    }

    /// The facts of what the runs of `task` left, which went bad with `reason`, with the snapshot of its worktree taken
    /// and recorded first; `None` for a task that works for no open round.
    ///
    /// # Errors
    /// [`BriefingProblem`] when a fact cannot be read.
    async fn gather_facts(&mut self, task: &TaskId, reason: Trouble) -> Result<Option<BriefingDraft>, BriefingProblem> {
        let no_task = || BriefingProblem::NoTask { task: task.clone() };
        let found = self.parts.state.find_task(task).ok_or_else(no_task)?;
        let (team, member) = (found.team, found.member);
        let config = &self.parts.settings.config;
        let base = config
            .repo(team.repo())
            .map(|repo| repo.base_branch.clone())
            .ok_or_else(|| StartProblem::RepoGone {
                repo: team.repo().to_string(),
            })?;
        let branch = work_branch(team, member)?;
        let round = member
            .runs()
            .iter()
            .rev()
            .flat_map(|run| run.rounds().iter().rev())
            .find(|round| team.round(round).is_some_and(agent_core::state::Round::is_open))
            .cloned();
        // A task that never ran works for the rounds its first run is given: a busy agent's hand-over briefs it so.
        let round = match round {
            Some(round) => Some(round),
            None => round_state::rounds_for(&self.parts.state, task, None)
                .map_err(BriefingProblem::Rounds)?
                .into_iter()
                .find(|round| team.round(round).is_some_and(agent_core::state::Round::is_open)),
        };
        let Some(round) = round else {
            // A snapshot is kept under a round: a task that works for none (a state of an older version) starts its new
            // session with its whole prompt, unbriefed, rather than not at all.
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %task,
                "a task started anew works for no open round: its new session is not briefed"
            );
            return Ok(None);
        };
        let agent = AgentName::from_str(member.agent().name())?;
        let path = self.parts.store.home().worktree_dir(team.repo(), task);
        let text = path
            .to_str()
            .ok_or_else(|| BriefingProblem::PathText { task: task.clone() })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| BriefingProblem::Path {
            task: task.clone(),
            source,
        })?;
        // The messages are those to the name the task has on its team's bus, whoever holds it now.
        let reader = (AgentName::from_str(member.bus_agent().name())?, team.id().clone());
        let git = Git::new(worktree.clone(), CommitIdentity::for_agent(config.git(), &agent));
        // The snapshot a restart took of the cut run's worktree is the one the new session is briefed from; nothing has
        // touched the worktree since.
        let snapshot = match self.cut_snapshots.remove(task) {
            Some(reference) if reason == Trouble::DaemonDown => reference,
            Some(_) | None => self.snapshot(&worktree, &round, task).await?,
        };
        let head = git.rev_parse(&Revision::Head).await?;
        let forked = git.merge_base(&Revision::Head, &Revision::Branch(base.clone())).await?;
        let commits = git.range_log(&forked, &head).await?;
        let uncommitted = git
            .uncommitted_changes()
            .await?
            .into_iter()
            .map(|change| change.path.to_string())
            .collect();
        let read = read_plan(&worktree, task).await;
        let plan = read.as_ref().map(steps_of);
        let unread = self
            .parts
            .bus
            .unread_messages(agent_runtime::agentbus::Reader {
                agent: &reader.0,
                team: &reader.1,
                task,
            })
            .await?;
        let history = self.gather_history(task, read.as_ref()).await?;
        Ok(Some(BriefingDraft {
            facts: BriefingFacts {
                reason,
                base,
                branch,
                commits,
                uncommitted,
                plan,
                messages: unread.shown,
                completeness: unread.completeness,
                snapshot,
            },
            history,
        }))
    }

    /// What the history and the question files of `task` say for its briefing: the newest runs, the judgements of its
    /// steps and the newest answers of the human.
    ///
    /// # Errors
    /// [`BriefingProblem`] when the history or a question file cannot be read.
    async fn gather_history(&self, task: &TaskId, plan: Option<&Plan>) -> Result<BriefingHistory, BriefingProblem> {
        let (runs, runs_left_out) = newest_shown(self.read_runs(task).await?, RUNS_SHOWN);
        let judged = self
            .latest_judgements(task)
            .await
            .map_err(BriefingProblem::Judgements)?;
        let judgements = match plan {
            Some(plan) => {
                let (_, plans_by_run) = self.earlier_plans(task, None).await;
                standing_judgements(plan, &judged, &plans_by_run)
            }
            None => Vec::new(),
        };
        let folder = self.parts.store.home().task_dir(task);
        let (answers, answers_left_out) = newest_shown(read_answers(&folder, task).await?, ANSWERS_SHOWN);
        Ok(BriefingHistory {
            runs,
            runs_left_out,
            answers,
            answers_left_out,
            judgements,
        })
    }

    /// The runs of `task` in the history, oldest first, as the briefing shows them; none when no daemon has written the
    /// history yet.
    ///
    /// # Errors
    /// [`BriefingProblem::Runs`] or [`BriefingProblem::RunsJoin`] when the history cannot be read.
    async fn read_runs(&self, task: &TaskId) -> Result<Vec<RunLine>, BriefingProblem> {
        let path = self.parts.store.home().history_file();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let task_id = task.clone();
        let read =
            tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.runs_of_task(&task_id)))
                .await
                .map_err(BriefingProblem::RunsJoin)?;
        let rows = match read {
            Ok(rows) => rows,
            Err(ReadError::Empty { .. }) => return Ok(Vec::new()),
            Err(source) => return Err(BriefingProblem::Runs(Box::new(source))),
        };
        Ok(rows
            .iter()
            .map(|row| RunLine {
                started: self.parts.zone.stamp_minute(row.start.value.at),
                agent: row.start.value.agent.name.clone(),
                end: run_end_word(row.end.as_ref().map(|end| &end.value.end)),
            })
            .collect())
    }

    /// Tells the leads of the member tasks whose work the state owes them, and which are not being told already, that the
    /// work is their own, each with the briefing of what the member's runs left. A briefing that cannot be gathered is
    /// left out and logged, and the message goes all the same: the lead still has the branch and the worktree of the
    /// work. The state stops owing the work when the bus took the message ([`Owner::flush_outbox`]).
    pub(super) async fn brief_leads(&mut self) {
        let owed: Vec<(TaskId, Trouble)> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| team.crew().roster().into_iter().map(|(_, member)| member))
            .filter(|member| member.owes_lead() && !self.leads_being_told.contains(member.task()))
            .filter_map(|member| {
                member
                    .renewals()
                    .map(|renewals| (member.task().clone(), renewals.reason()))
            })
            .collect();
        for (task, trouble) in owed {
            self.leads_being_told.insert(task.clone());
            let briefing = match self.gather_facts(&task, trouble).await {
                Ok(Some(draft)) => match draft.render_for_lead(&self.parts.texts, &task) {
                    Ok(text) => Some(text),
                    Err(error) => {
                        tracing::warn!(event = LogEvent::HistoryRecordIncomplete.as_str(), task = %task,
                            error = %error_chain(&error), "the briefing of a member's work cannot be written for its lead");
                        None
                    }
                },
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(event = LogEvent::HistoryRecordIncomplete.as_str(), task = %task,
                        error = %error_chain(&error), "the briefing of a member's work cannot be gathered for its lead");
                    None
                }
            };
            let Some(found) = self.parts.state.find_task(&task) else {
                tracing::warn!(event = LogEvent::BusDown.as_str(), task = %task,
                    "the lead of a task that left the state is not told of its work");
                continue;
            };
            let team = found.team.id().clone();
            let worktree = self.parts.store.home().worktree_dir(found.team.repo(), &task);
            match lead_takes_work(found.team, found.member, &worktree, briefing) {
                Ok(Some(told)) => self.tell_bus(
                    &team,
                    BusJob::Tell {
                        to: told.to,
                        notice: told.notice,
                    },
                ),
                // A team without a lead has nobody to give the work to; the ladder gives it to none.
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(event = LogEvent::BusDown.as_str(), task = %task, error = %error_chain(&error),
                        "the lead of a member's task cannot be told of its work");
                }
            }
        }
    }

    /// Takes the snapshot of `worktree` for `task` of `round`, records it in the history, and returns its reference.
    async fn snapshot(
        &mut self,
        worktree: &AbsolutePath,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<String, BriefingProblem> {
        let now = Timestamp::now(&self.parts.clock)?;
        let warn_bytes = self.parts.settings.config.snapshot().warn_mb.saturating_mul(MEGABYTE);
        let record = take_snapshot(worktree, round, task, now, warn_bytes).await?;
        let reference = format!("refs/agents/snapshots/{round}/{task}/{}", record.number);
        self.history.record(Record::Snapshot(Box::new(record)));
        Ok(reference)
    }
}

/// The facts of a briefing: what the task's own record gives (its facts) and what its history and question files give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BriefingDraft {
    /// The facts of the work the runs left.
    facts: BriefingFacts,
    /// The runs, the judgements and the answers of the task.
    history: BriefingHistory,
}

/// What the history and the question files of a task say for its briefing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BriefingHistory {
    /// The newest runs of the task, oldest first.
    runs: Vec<RunLine>,
    /// Whether older runs of the task are left out of [`BriefingHistory::runs`].
    runs_left_out: bool,
    /// The newest answers of the human to the questions of the task, oldest first.
    answers: Vec<AnswerLine>,
    /// Whether older answers are left out of [`BriefingHistory::answers`].
    answers_left_out: bool,
    /// The judgement the harness gave each step that is marked done, by step ID, sorted by ID.
    judgements: Vec<(String, JudgementState)>,
}

/// One run of the task as the briefing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunLine {
    /// When the run started, as the zone of the host writes it.
    started: String,
    /// The agent that ran it.
    agent: String,
    /// How it ended, as [`run_end_word`] names it.
    end: &'static str,
}

/// One answer of the human as the briefing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AnswerLine {
    /// The number of the question in its file.
    number: String,
    /// The rest of the question's heading line.
    title: String,
    /// The verdict, as [`verdict_word`] names it.
    verdict: String,
}

impl BriefingDraft {
    /// The briefing of `task`, whose own prompt is `prompt`, rendered through `texts`.
    ///
    /// # Errors
    /// [`ComposeError`] when the text of the briefing cannot be filled.
    pub(super) fn render(&self, texts: &HarnessPrompts, task: &TaskId, prompt: &str) -> Result<String, ComposeError> {
        self.facts.render_with(texts, task, prompt, &self.history)
    }

    /// What a lead given the work of `task` is told the runs of the task left, rendered through `texts`.
    ///
    /// # Errors
    /// [`ComposeError`] when the text cannot be filled.
    pub(super) fn render_for_lead(&self, texts: &HarnessPrompts, task: &TaskId) -> Result<String, ComposeError> {
        self.facts.render_for_lead(texts, task, &self.history)
    }
}

impl BriefingFacts {
    /// The briefing of `task` with no history, rendered through `texts`: the form the facts alone give.
    ///
    /// # Errors
    /// [`ComposeError`] when the text of the briefing cannot be filled.
    #[cfg(test)]
    pub(super) fn render(&self, texts: &HarnessPrompts, task: &TaskId, prompt: &str) -> Result<String, ComposeError> {
        self.render_with(texts, task, prompt, &BriefingHistory::default())
    }

    /// The briefing of `task`, whose own prompt is `prompt`, with `history`, rendered through `texts`.
    ///
    /// # Errors
    /// [`ComposeError`] when the text of the briefing cannot be filled.
    fn render_with(
        &self,
        texts: &HarnessPrompts,
        task: &TaskId,
        prompt: &str,
        history: &BriefingHistory,
    ) -> Result<String, ComposeError> {
        let values = self.variables(texts, task, prompt, history)?;
        Ok(texts.render(HarnessPrompt::Briefing, &values)?)
    }

    /// What a lead given the work of `task` is told the runs of the task left, rendered through `texts`: the facts of the
    /// briefing, without the task's prompt, which the lead's own list holds.
    ///
    /// # Errors
    /// [`ComposeError`] when the text cannot be filled.
    fn render_for_lead(
        &self,
        texts: &HarnessPrompts,
        task: &TaskId,
        history: &BriefingHistory,
    ) -> Result<String, ComposeError> {
        let all = self.variables(texts, task, "", history)?;
        let mut values = Variables::new();
        for name in all.names().filter(|name| name.as_str() != PROMPT_VARIABLE) {
            if let Some(value) = all.get(name) {
                values = values.with(name.as_str(), value)?;
            }
        }
        Ok(texts.render(HarnessPrompt::LeadBriefing, &values)?)
    }

    /// The values of the briefing of `task`, whose own prompt is `prompt`, with `history`.
    fn variables(
        &self,
        texts: &HarnessPrompts,
        task: &TaskId,
        prompt: &str,
        history: &BriefingHistory,
    ) -> Result<Variables, ComposeError> {
        let commits: Vec<String> = self.commits.iter().map(|commit| commit.id.to_string()).collect();
        let listed: Vec<BriefingCommit<'_>> = self
            .commits
            .iter()
            .zip(&commits)
            .map(|(commit, id)| BriefingCommit {
                id: id.get(..8).unwrap_or(id),
                subject: &commit.subject,
            })
            .collect();
        let uncommitted: Vec<&str> = self.uncommitted.iter().map(String::as_str).collect();
        let steps: Option<Vec<BriefingStep<'_>>> = self.plan.as_ref().map(|steps| {
            steps
                .iter()
                .map(|(id, title, state)| BriefingStep { id, title, state })
                .collect()
        });
        let ids: Vec<String> = self.messages.iter().map(|message| message.id.to_string()).collect();
        let messages: Vec<BriefingMessage<'_>> = self
            .messages
            .iter()
            .zip(&ids)
            .map(|(message, id)| BriefingMessage {
                id,
                from: &message.from,
                text: &message.text,
            })
            .collect();
        let left_out = match self.completeness {
            Completeness::All => None,
            Completeness::Newest => Some(texts.render(HarnessPrompt::BriefingMessagesLeftOut, &Variables::new())?),
        };
        let runs: Vec<BriefingRun<'_>> = history
            .runs
            .iter()
            .map(|run| BriefingRun {
                started: &run.started,
                agent: &run.agent,
                end: run.end,
            })
            .collect();
        let answers: Vec<BriefingAnswer<'_>> = history
            .answers
            .iter()
            .map(|answer| BriefingAnswer {
                number: &answer.number,
                title: &answer.title,
                verdict: &answer.verdict,
            })
            .collect();
        let judgements: Vec<(&str, JudgementState)> = history
            .judgements
            .iter()
            .map(|(step, state)| (step.as_str(), *state))
            .collect();
        let confirmed = texts.render(HarnessPrompt::BriefingPlanConfirmed, &Variables::new())?;
        let declared = texts.render(HarnessPrompt::BriefingPlanDeclared, &Variables::new())?;
        let mismatch = texts.render(HarnessPrompt::BriefingPlanMismatch, &Variables::new())?;
        let not_done = texts.render(HarnessPrompt::BriefingPlanNotDone, &Variables::new())?;
        let runs_left_out = left_out_note(texts, history.runs_left_out, HarnessPrompt::BriefingRunsLeftOut)?;
        let answers_left_out = left_out_note(texts, history.answers_left_out, HarnessPrompt::BriefingAnswersLeftOut)?;
        let facts = Briefing {
            reason: reason_word(self.reason),
            task: task.as_str(),
            prompt,
            base: self.base.as_str(),
            branch: self.branch.as_str(),
            commits: &listed,
            uncommitted: &uncommitted,
            plan: steps.as_deref(),
            messages: &messages,
            left_out: left_out.as_deref(),
            snapshot: &self.snapshot,
        };
        let notes = BriefingNotes {
            runs: &runs,
            runs_left_out: runs_left_out.as_deref(),
            answers: &answers,
            answers_left_out: answers_left_out.as_deref(),
            judgements: &judgements,
            headings: PlanHeadings {
                confirmed: &confirmed,
                declared: &declared,
                mismatch: &mismatch,
                not_done: &not_done,
            },
        };
        Ok(briefing_variables_with(&facts, &notes)?)
    }
}

/// The note that older items are left out of a list, rendered through `texts`, when `left_out` says some are.
///
/// # Errors
/// [`ComposeError`] when the note cannot be rendered.
fn left_out_note(texts: &HarnessPrompts, left_out: bool, note: HarnessPrompt) -> Result<Option<String>, ComposeError> {
    if left_out {
        Ok(Some(texts.render(note, &Variables::new())?))
    } else {
        Ok(None)
    }
}

/// The newest `shown` of `items`, oldest first, and whether older ones were left out.
pub(super) fn newest_shown<T>(items: Vec<T>, shown: usize) -> (Vec<T>, bool) {
    let left_out = items.len() > shown;
    let skip = items.len().saturating_sub(shown);
    (items.into_iter().skip(skip).collect(), left_out)
}

/// The word the briefing gives how a run ended; a run with no recorded end was cut before it.
pub(super) fn run_end_word(end: Option<&RunEnd>) -> &'static str {
    match end {
        None => "cut before its end was recorded",
        Some(RunEnd::Reported { outcome }) => (*outcome).into(),
        Some(RunEnd::NoStatus { .. }) => "no status",
        Some(RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited) => "failed",
        Some(RunEnd::Hung) => "hang",
        Some(RunEnd::TurnLimit) => "turn limit",
        Some(RunEnd::Stopped) => "stopped",
    }
}

/// The word the briefing gives a verdict: `ok` for the recommendation, otherwise the human's own answer.
pub(super) fn verdict_word(verdict: &Verdict) -> &str {
    match verdict {
        Verdict::Accept => "ok",
        Verdict::Own(answer) => answer.as_str(),
    }
}

/// The answers the human gave in the question files of `task` in `folder`, oldest question first. A file that does not
/// read is logged and left out: the new session reads the files itself.
///
/// # Errors
/// [`BriefingProblem::AnswersDir`] or [`BriefingProblem::Answers`] when the folder or a file cannot be read.
async fn read_answers(folder: &Path, task: &TaskId) -> Result<Vec<AnswerLine>, BriefingProblem> {
    let mut entries = match tokio::fs::read_dir(folder).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(BriefingProblem::AnswersDir(source)),
    };
    let mut numbers: Vec<NonZeroU32> = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(BriefingProblem::AnswersDir)? {
        // A name that is not `questions-<number>.md` is no question file: it is not an error.
        if let Some(number) = entry.file_name().to_str().and_then(question_file_number) {
            numbers.push(number);
        }
    }
    numbers.sort();
    let mut answers = Vec::new();
    for number in numbers {
        let path = folder.join(format!("questions-{number}.md"));
        let Some(file) = read_text_file(&path, MAX_QUESTIONS_BYTES)
            .await
            .map_err(BriefingProblem::Answers)?
        else {
            continue;
        };
        match QuestionsFile::parse(&file.text) {
            Ok(questions) => answers.extend(questions.entries().iter().filter_map(|entry| {
                entry.verdict().map(|verdict| AnswerLine {
                    number: entry.number().to_string(),
                    title: entry.title().to_owned(),
                    verdict: verdict_word(verdict).to_owned(),
                })
            })),
            Err(error) => tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %task,
                error = %error_chain(&error),
                "a question file of a task started anew does not read: its briefing leaves its answers out"
            ),
        }
    }
    Ok(answers)
}

/// The number of a question file named `questions-<number>.md`; `None` for any other name.
fn question_file_number(name: &str) -> Option<NonZeroU32> {
    name.strip_prefix("questions-")?.strip_suffix(".md")?.parse().ok()
}

/// Why the session before went bad, as the state records it; the briefing's text says what each word means.
pub(super) const fn reason_word(reason: Trouble) -> &'static str {
    match reason {
        Trouble::NoStatus => "no_status",
        Trouble::Hang => "hang",
        Trouble::TurnLimit => "turn_limit",
        Trouble::Busy => "busy",
        Trouble::DaemonDown => "daemon_down",
    }
}

/// The plan in the worktree; `None` when there is none or it does not read. A plan that does not read is logged and left
/// out: the new session reads the file itself.
async fn read_plan(worktree: &AbsolutePath, task: &TaskId) -> Option<Plan> {
    let read = read_text_file(&worktree.as_path().join(PLAN_FILE), MAX_TASK_LIST_BYTES).await;
    let parsed = match read {
        Ok(None) => return None,
        Ok(Some(file)) => parse_plan(&file.text).map_err(|error| error_chain(&error)),
        Err(error) => Err(error_chain(&error)),
    };
    match parsed {
        Ok(plan) => Some(plan),
        Err(error) => {
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %task,
                error = %error,
                "the plan of a task started anew does not read: its briefing has no steps"
            );
            None
        }
    }
}

/// The steps of `plan` as id, title and state.
fn steps_of(plan: &Plan) -> Vec<(String, String, &'static str)> {
    plan.steps()
        .map(|step| (step.id().to_string(), step.title().to_owned(), state_text(step.state())))
        .collect()
}

/// The judgements that still hold for the steps of `plan`: the harness's latest judgement of a step stands only while the
/// step is as it was in the plan read at the end of the run that was judged. A step edited since, one whose judged plan
/// was not kept, and a judgement made outside a run are left out, so that the step is shown as declared. Sorted by step.
fn standing_judgements(
    plan: &Plan,
    judged: &HashMap<ItemId, JudgementRecord>,
    plans_by_run: &HashMap<RunId, Plan>,
) -> Vec<(String, JudgementState)> {
    let mut standing: Vec<(String, JudgementState)> = judged
        .iter()
        .map(|(step, record)| (step.to_string(), record.state))
        .collect();
    standing.sort_by(|a, b| a.0.cmp(&b.0));
    let _ = (plan, plans_by_run);
    standing
}

/// The mark of a step in the plan's own syntax; the briefing's text says what each mark means.
const fn state_text(state: StepState) -> &'static str {
    match state {
        StepState::Todo => "[ ]",
        StepState::InHand => "[>]",
        StepState::Done => "[x]",
        StepState::GivenUp => "[-]",
    }
}

#[cfg(test)]
mod tests;
