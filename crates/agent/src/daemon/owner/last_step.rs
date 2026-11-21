//! After the last step of the ladder: a task handed over whose new holder went bad too, or a lead given a member's work
//! that went bad too, is not failed at once — the human is asked once, in a questions file of the harness's own text,
//! whether to start it once more in a new session of its agent or to stop it, and the answer does what it says.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::decide::ladder::Trouble;
use agent_core::decide::replacement::ReplacementMode;
use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::questions::{Entry, QuestionsFile, Verdict};
use agent_core::state::{ItemPhase, Team, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::harness_prompts::RenderTextError;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use agent_runtime::text_file::{ReplaceError, replace_text_file};

use super::briefing::reason_word;
use super::start::next_number;
use super::{Asked, Located, Notice, Owner, OwnerError};
use agent_core::protocol::EventKind;

/// The permission bits of a questions file the harness writes: the human's to read and edit.
const QUESTIONS_MODE: u32 = 0o600;

/// What the text of the question says of a task that no agent held before the one that holds it.
const NOBODY_BEFORE: &str = "none";

/// What the human's answer to the question after the last step says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastStepAnswer {
    /// `a`: the task starts once more, in the new session its question began.
    OnceMore,
    /// `b`, or the recommendation accepted: the task stops.
    Stop,
}

/// Why the question after the last step cannot be written.
#[derive(Debug, thiserror::Error)]
enum LastStepQuestionError {
    /// A variable of the question is not valid.
    #[error("a variable of the question is not valid")]
    Variable(#[from] VariablesError),
    /// The question cannot be rendered.
    #[error("the question cannot be rendered")]
    Render(#[from] RenderTextError),
    /// The directory of the task's files cannot be made.
    #[error("cannot make the directory {path:?}")]
    Directory {
        /// The directory.
        path: std::path::PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The file cannot be written.
    #[error("the questions file cannot be written")]
    Write(#[from] ReplaceError),
}

impl Owner {
    /// The human is asked, in the task's next questions file, what becomes of `task`, whose runs went bad with `trouble`
    /// after the last step of the ladder; `None` when the task is not to be asked — mode `off`, asked once already, or
    /// neither taken over nor a lead given a member's work — or the question cannot be written (logged): the task then
    /// fails as before.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone or the state refuses the question.
    pub(super) async fn ask_after_last_step(
        &mut self,
        task: &TaskId,
        located: &Located,
        trouble: Trouble,
        now: Timestamp,
        events: &mut Vec<EventKind>,
    ) -> Result<Option<(TaskStatus, Option<Notice>)>, OwnerError> {
        if !self.asks_after_last_step(task, located.role)? {
            return Ok(None);
        }
        let (number, values) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            let number = next_number(found.member).ok_or_else(|| OwnerError::QuestionNumbers { task: task.clone() })?;
            let held: Vec<&str> = found
                .member
                .past_holders()
                .iter()
                .map(|held| held.agent.name())
                .collect();
            let held = if held.is_empty() {
                NOBODY_BEFORE.to_owned()
            } else {
                held.join(", ")
            };
            (number, (found.member.agent().name().to_owned(), held))
        };
        let path = self.parts.store.home().questions_file(task, number);
        if let Err(error) = self.put_last_step_question(task, &values, trouble, &path).await {
            // The human is told the task failed, as before the question existed: a task asked nothing is not held.
            tracing::warn!(
                event = LogEvent::QuestionsFileInvalid.as_str(),
                task = %task,
                path = %path.display(),
                error = %error_chain(&error),
                "the question after the last step cannot be written: the task fails"
            );
            return Ok(None);
        }
        let at = self.after_last_run(task)?;
        self.parts.state.ask_after_last_step(task, trouble, number, at)?;
        log_decision("last_step", &format!("task={task}, trouble={trouble:?}"), "ask_human");
        let next = match self.ask(task, located, now, events).await? {
            Asked::Recorded(notice) => (TaskStatus::Questions, Some(notice)),
            Asked::Unusable(notice) => (TaskStatus::Failed, Some(notice)),
        };
        Ok(Some(next))
    }

    /// Whether `task` of `role` is asked after its last step: not in mode `off`, not asked once already, and either
    /// handed over before or a coordinator holding the item of a member whose task stopped.
    fn asks_after_last_step(&self, task: &TaskId, role: Role) -> Result<bool, OwnerError> {
        if self.parts.settings.config.replacement().mode() == ReplacementMode::Off {
            return Ok(false);
        }
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        if found.member.asked_after_last_step().is_some() {
            return Ok(false);
        }
        let asked = match role {
            Role::Solo | Role::Member => !found.member.past_holders().is_empty(),
            Role::Coordinator => !found.member.past_holders().is_empty() || holds_work_of_a_stopped_member(found.team),
            Role::Worker | Role::Reviewer => false,
        };
        Ok(asked)
    }

    /// Renders the question after the last step of `task` — the agent that holds it and those before it in `values` —
    /// and writes it to `path`.
    async fn put_last_step_question(
        &self,
        task: &TaskId,
        values: &(String, String),
        trouble: Trouble,
        path: &Path,
    ) -> Result<(), LastStepQuestionError> {
        let (agent, held) = values;
        let values = Variables::new()
            .with("task", task.as_str())?
            .with("agent", agent.as_str())?
            .with("held", held.as_str())?
            .with("reason", reason_word(trouble))?;
        let text = self.parts.texts.render(HarnessPrompt::NoticeLastStep, &values)?;
        if let Some(directory) = path.parent() {
            tokio::fs::create_dir_all(directory)
                .await
                .map_err(|source| LastStepQuestionError::Directory {
                    path: directory.to_path_buf(),
                    source,
                })?;
        }
        replace_text_file(path, &text, QUESTIONS_MODE).await?;
        Ok(())
    }

    /// The status `task` goes to after the human answered its questions `number` in `file`: when those are its question
    /// after the last step, stopped for `b` or the recommendation — and a member's work is given to its lead — or queued
    /// for `a`; queued for any other questions.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone.
    pub(super) fn after_answer(
        &mut self,
        task: &TaskId,
        located: &Located,
        number: NonZeroU32,
        file: &QuestionsFile,
    ) -> Result<TaskStatus, OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        if found.member.asked_after_last_step() != Some(number) {
            return Ok(TaskStatus::Queued);
        }
        let reason = found.member.renewals().map(agent_core::state::Renewals::reason);
        let answer = last_step_answer(file);
        log_decision("last_step_answer", &task.as_str(), &format!("{answer:?}"));
        match answer {
            LastStepAnswer::OnceMore => Ok(TaskStatus::Queued),
            LastStepAnswer::Stop => {
                if located.role == Role::Member
                    && let Some(trouble) = reason
                {
                    self.lead_briefings.push((task.clone(), trouble));
                }
                Ok(TaskStatus::Stopped)
            }
        }
    }
}

/// Whether a member of `team` whose task stopped holds an item that is still worked on: its lead has its work.
fn holds_work_of_a_stopped_member(team: &Team) -> bool {
    team.crew()
        .roster()
        .into_iter()
        .filter(|(role, member)| *role == Role::Member && member.status() == TaskStatus::Stopped)
        .any(|(_, member)| {
            team.items().iter().any(|progress| match progress.phase() {
                ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                    assignee.as_str() == member.agent().name()
                }
                ItemPhase::Pending
                | ItemPhase::InReview { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => false,
            })
        })
}

/// What the answer in `file` says: `a` starts the task once more; `b`, the recommendation accepted, or any other answer
/// stops it, so that nothing more runs on a task nobody asked to go on.
fn last_step_answer(file: &QuestionsFile) -> LastStepAnswer {
    match file.entries().first().and_then(Entry::verdict) {
        Some(Verdict::Own(text)) if super::plan::starts_as_option_a(text.as_str()) => LastStepAnswer::OnceMore,
        Some(Verdict::Accept | Verdict::Own(_)) | None => LastStepAnswer::Stop,
    }
}
