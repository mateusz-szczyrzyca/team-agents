//! The owner's side of the questions: the file a run's questions leave, the watch on it, and the answers of the human —
//! from the protocol (`agent answer`, the dashboard) and from the file saved with the marker — each of which queues the
//! task again with them.
//!
//! The questions and the answers are in the file; the state keeps that a round was asked and answered, when, and
//! through which channel. What the next run is told is made from the file when that run is made ready (`start.rs`), so
//! it survives a restart of the daemon.

use std::path::Path;

use agent_core::cli::ActivityEvent;
use agent_core::ids::TaskId;
use agent_core::protocol::{AnswerRequest, Client, EventKind, Failure, FailureKind, Reply};
use agent_core::state::{AnsweredVia, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::process::error_chain;
use agent_runtime::text_file::replace_text_file;

use super::{Located, Notice, Owner, OwnerError};
use crate::daemon::questions::watch::{Report, Saved};
use crate::daemon::questions::{self, Judgement, Loaded};

/// The round of questions a task waits for answers to.
struct OpenRound {
    /// The task's team, role and agent.
    located: Located,
    /// The round.
    round: std::num::NonZeroU32,
    /// The questions file of the round.
    path: std::path::PathBuf,
}

impl Owner {
    // ---- the file of a new round ----------------------------------------------------------------------------

    /// Gives the file of the round that was just asked its instructions and starts watching for the human's save.
    ///
    /// What cannot be done is told in the log and does not stop the round: the questions are recorded and the human is
    /// notified, and can answer with `agent answer` whatever the file looks like.
    pub(super) async fn prepare_questions(&mut self, task: &TaskId, path: &Path, loaded: &Loaded) {
        match questions::with_instructions(&loaded.file, &self.parts.question_texts, task) {
            Ok(prepared) => {
                if let Err(error) = replace_text_file(path, &prepared.render(), loaded.mode).await {
                    tracing::warn!(task = %task, error = %error_chain(&error), "the instructions cannot be put in the questions file");
                }
            }
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "the instructions of the questions file cannot be made");
            }
        }
        self.watch_task(task);
        self.open_in_editor(task, path);
    }

    /// Asks for the questions file to be opened in the editor of the human, when the configuration asks for it.
    fn open_in_editor(&self, task: &TaskId, path: &Path) {
        let Some(opens) = &self.parts.opens else {
            return;
        };
        if let Err(error) = opens.try_send(path.to_path_buf()) {
            // The human is told by the notification: a file that is not opened is where it was.
            tracing::warn!(task = %task, %error, "the questions file cannot be queued to be opened in the editor");
        }
    }

    /// Starts watching the directory of `task` for a save of its questions file.
    fn watch_task(&mut self, task: &TaskId) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        if self.watched.contains(task) {
            return;
        }
        match watcher.watch(&self.parts.store.home().task_dir(task)) {
            Ok(()) => {
                self.watched.insert(task.clone());
            }
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "a save of the questions file is not noticed: answer with `agent answer`");
            }
        }
    }

    /// Stops watching the directory of `task`, whose questions are answered.
    fn unwatch_task(&mut self, task: &TaskId) {
        self.incomplete.remove(task);
        if !self.watched.remove(task) {
            return;
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        if let Err(error) = watcher.unwatch(&self.parts.store.home().task_dir(task)) {
            // The directory may be gone with the task: a watch that cannot be let go costs nothing.
            tracing::warn!(task = %task, error = %error_chain(&error), "the watch on the questions file cannot be let go");
        }
    }

    /// Watches the tasks that wait for answers, and looks at their files: after a start, the human may have saved the
    /// answers while the daemon was down; after a rescan, saves may have been missed.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn adopt_open_questions(&mut self) -> Result<(), OwnerError> {
        let open: Vec<Saved> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let crew = team.crew().roster().into_iter().map(|(_, member)| member);
                crew.chain(team.reviewers().iter())
                    .filter(|member| member.status() == TaskStatus::Questions)
                    .filter_map(|member| {
                        member.open_question().map(|question| Saved {
                            task: member.task().clone(),
                            round: question.round(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        for saved in open {
            self.watch_task(&saved.task);
            self.file_saved(saved).await?;
        }
        Ok(())
    }

    // ---- the answers ----------------------------------------------------------------------------------------

    /// Takes the answers of the request: they are put in the file, and the task goes on with them. The answer is the
    /// refusal when they cannot be taken.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn answer_request(
        &mut self,
        client: &Client,
        request: AnswerRequest,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        let AnswerRequest { task, round, answers } = request;
        let open = match self.open_round(&task) {
            Ok(open) => open,
            Err(failure) => return Ok(Err(failure)),
        };
        if open.round != round {
            return Ok(Err(Failure {
                kind: FailureKind::Conflict,
                message: format!(
                    "round {round} of task {task} is not the one that waits for answers: it is round {}",
                    open.round
                ),
            }));
        }
        let loaded = match questions::read(&open.path).await {
            Ok(loaded) => loaded,
            Err(problem) => {
                return Ok(Err(Failure {
                    kind: FailureKind::Internal,
                    message: format!("cannot read the questions: {}", error_chain(&problem)),
                }));
            }
        };
        let answered = match questions::apply(&loaded.file, &answers) {
            Ok(answered) => answered,
            Err(refusal) => return Ok(Err(refusal.failure(&loaded.file))),
        };
        // The file keeps the answers that were given, as the one place they are.
        if let Err(error) = replace_text_file(&open.path, &answered.render(), loaded.mode).await {
            return Ok(Err(Failure {
                kind: FailureKind::Internal,
                message: format!(
                    "cannot write the answers into the questions file: {}",
                    error_chain(&error)
                ),
            }));
        }
        let via = match client {
            Client::Cli => AnsweredVia::Command,
            Client::Dashboard { .. } => AnsweredVia::Dashboard,
        };
        self.record_answer(&task, &open, via).await?;
        Ok(Ok(Reply::Applied { tasks: vec![task] }))
    }

    /// The round `task` waits for answers to, or why there is none.
    fn open_round(&self, task: &TaskId) -> Result<OpenRound, Failure> {
        let found = self.parts.state.find_task(task).ok_or_else(|| Failure {
            kind: FailureKind::NotFound,
            message: format!("there is no task {task}"),
        })?;
        let waiting = found.member.status() == TaskStatus::Questions;
        let Some(question) = found.member.open_question().filter(|_| waiting) else {
            return Err(Failure {
                kind: FailureKind::Conflict,
                message: format!("task {task} is {}, not waiting for answers", found.member.status()),
            });
        };
        Ok(OpenRound {
            located: Located {
                team: found.team.id().clone(),
                role: found.role,
                agent: found.member.agent().name().to_owned(),
            },
            round: question.round(),
            path: question.file().as_path().to_path_buf(),
        })
    }

    /// Acts on what the watch on the questions files reported: a save is read, and a rescan reads every open file.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn reported(&mut self, report: Report) -> Result<(), OwnerError> {
        match report {
            Report::Saved(saved) => self.file_saved(saved).await,
            Report::Rescan => self.adopt_open_questions().await,
        }
    }

    /// The human saved the questions file `saved`: when it is marked answered and complete, the task goes on with its
    /// answers; when it is marked and not complete, the human is told what is missing.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn file_saved(&mut self, saved: Saved) -> Result<(), OwnerError> {
        let Ok(open) = self.open_round(&saved.task) else {
            // A task that does not wait for answers: the save is the file's own change or a late one.
            return Ok(());
        };
        if open.round != saved.round {
            return Ok(());
        }
        let loaded = match questions::read(&open.path).await {
            Ok(loaded) => loaded,
            Err(problem) => {
                // Most likely a save in the middle of a write: the next event of the save comes with the whole file.
                tracing::warn!(task = %saved.task, error = %error_chain(&problem), "the saved questions file cannot be read");
                return Ok(());
            }
        };
        match questions::judge(&loaded.file) {
            Judgement::NotYet => Ok(()),
            Judgement::Answered => self.record_answer(&saved.task, &open, AnsweredVia::SavedFile).await,
            Judgement::Incomplete { problems } => {
                let text = questions::describe(&loaded.file, &problems);
                self.tell_incomplete(&saved.task, &open, text);
                Ok(())
            }
        }
    }

    /// Tells the human that the answers saved for `task` are not complete, unless the last save said the same.
    fn tell_incomplete(&mut self, task: &TaskId, open: &OpenRound, problems: String) {
        if self.incomplete.get(task) == Some(&problems) {
            return;
        }
        let made = questions::incomplete_notification(
            &self.parts.question_texts,
            &open.located.agent,
            task,
            &open.path,
            &problems,
        );
        self.incomplete.insert(task.clone(), problems);
        match made {
            Ok(notification) => match Timestamp::now(&self.parts.clock) {
                Ok(now) => self.notify(now, &open.located, task, Notice::Prepared(notification)),
                Err(error) => tracing::error!(task = %task, %error, "the clock gives no time for a notification"),
            },
            Err(error) => {
                tracing::error!(task = %task, error = %error_chain(&error), "the notification of incomplete answers cannot be made");
            }
        }
    }

    /// Records that the human answered `open` through `via` and queues the task, which goes on with the answers.
    async fn record_answer(&mut self, task: &TaskId, open: &OpenRound, via: AnsweredVia) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        {
            let (_, mut member) = self.member_mut(task)?;
            member
                .answer_question(open.round, now, via)
                .map_err(|source| OwnerError::Question {
                    task: task.clone(),
                    source,
                })?;
        }
        events.push(EventKind::QuestionAnswered {
            team: open.located.team.clone(),
            task: task.clone(),
            round: open.round,
            via,
        });
        self.move_to(task, &open.located.team, TaskStatus::Queued, &mut events)?;
        self.unwatch_task(task);
        self.commit(task, &open.located, now, events, None).await?;
        let channel = match via {
            AnsweredVia::Command => "agent answer",
            AnsweredVia::SavedFile => "a saved file",
            AnsweredVia::Dashboard => "the dashboard",
        };
        self.log_activity(
            task,
            now,
            ActivityEvent::Info(format!(
                "the questions of round {} were answered through {channel}",
                open.round
            )),
        )
        .await;
        tracing::info!(task = %task, round = %open.round, channel, "the questions are answered");
        self.look_at(&open.located.team);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
