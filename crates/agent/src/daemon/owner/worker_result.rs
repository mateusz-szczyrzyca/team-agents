//! What becomes of the result of a worker: its check, the bringing over of its commit and the answer to the requester.
//!
//! When a worker's run ends `STATUS: DONE`, [`Owner::keep_worker_result`] writes the request to a file of the worker's
//! task directory and the next look at the team settles it, stage by stage, writing the file again after each stage:
//! the harness runs the request's check in the worker's worktree, [`decide`] says what the result is worth, a result that
//! is good is brought over to the requester's branch, and the requester is told. A daemon that stops between two stages
//! goes on from the stage the file names, so that the check runs once and a commit is brought over once.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::decide::worker_result::{CheckOutcome, Elsewhere, Facts, Scope, Verdict, WorkerEnding, Wrong, decide};
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::Check;
use agent_core::plan::judge::JudgementState;
use agent_core::state::{CommitId, Timestamp};
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus, DelegationUpdate, FailureClass, RequestId};
use agent_runtime::git::{BringOver, CommitIdentity, Git, Revision};
use agent_runtime::plan::check::{CheckContext, execute_request_check, read_check_tail, request_check_outcome};
use agent_runtime::process::error_chain;
use agent_runtime::team_prompts::TeamPrompts;
use agent_runtime::worker_request::{
    Checked, Found, Pending, PendingError, Settlement, Stage, pending_path, read_pending_each, request_of_file,
    set_aside, write_pending,
};

use super::teams::{Outbound, Served, is_transient};
use super::{Owner, OwnerError};
use crate::daemon::team::{BusJob, BusNotice, NoticeRenderError};

/// What the requester reads of a check's output when the check printed nothing.
const NO_OUTPUT: &str = "(the check printed nothing)";

/// A request whose check runs in the background: what is kept of it until the check ends.
#[derive(Debug)]
pub(super) struct Checking {
    /// The task directory of the worker, where the file of the request is.
    task_dir: PathBuf,
    /// The request as its file has it: the stage `Ended`.
    pending: Pending,
    /// What is known of the result before the check.
    checked: Checked,
}

/// What reading a request and the worker's worktree came to.
enum Gathered {
    /// The bus cannot say now: the request is tried again at the next look.
    Later,
    /// The bus has no such request: its file is dropped.
    Gone,
    /// The result is wrong before any check: the reasons.
    Wrong(Checked, Vec<String>),
    /// The result is to be checked with `text`, for the step `step`, in the background.
    Check(Checked, ItemId, String),
    /// There is no check to run.
    Checked(Checked),
}

/// What the files of a request, as the bus has them, say it asks for.
fn scope_of(files: &[String]) -> Scope {
    if files.is_empty() { Scope::Facts } else { Scope::Files }
}

/// The facts of a result for [`decide`]: no other provider is looked for in this version, so a wrong result is returned
/// to the requester at once.
fn facts_of(checked: &Checked, run_seconds: u64, limit_seconds: u64) -> Facts {
    Facts {
        ending: WorkerEnding::Done,
        check: checked
            .found
            .as_ref()
            .map_or(CheckOutcome::Absent, |found| found.outcome),
        changed_outside_files: !checked.outside.is_empty(),
        past_seconds: limit_seconds > 0 && run_seconds > limit_seconds,
        scope: scope_of(&checked.files),
        try_number: 1,
        tries: 1,
        elsewhere: Elsewhere::None,
    }
}

/// The reasons a result is wrong, in the words the requester reads.
fn reasons_of(checked: &Checked, wrong: &[Wrong], limit_seconds: u64) -> Vec<String> {
    wrong
        .iter()
        .map(|wrong| match wrong {
            Wrong::EndedOtherwise(_) => "the worker did not end its run with STATUS: DONE".to_owned(),
            Wrong::CheckFailed => match &checked.found {
                Some(found) => format!(
                    "the check `{}` failed{}",
                    found.line,
                    found
                        .reason
                        .as_ref()
                        .map_or_else(String::new, |reason| format!(" ({reason})"))
                ),
                None => "the check failed".to_owned(),
            },
            Wrong::CheckTimedOut => "the check ran past its time limit".to_owned(),
            Wrong::OutsideFiles => format!(
                "the worker changed files the request did not name: {}",
                checked.outside.join(", ")
            ),
            Wrong::NoCheck => "the request names files but no check, so nothing shows the commit right".to_owned(),
            Wrong::PastSeconds => format!("the worker's run lasted past {limit_seconds} seconds"),
        })
        .collect()
}

impl Owner {
    /// Keeps the request `served` of `team`, whose worker's run ended `STATUS: DONE` with the report `summary`, for the
    /// next look at the team to settle.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock gives a time that cannot be recorded; [`PendingError`] when the file cannot be written.
    pub(super) fn keep_worker_result(
        &mut self,
        team: &TeamId,
        served: &Served,
        worker_task: &TaskId,
        summary: String,
    ) -> Result<(), PendingError> {
        let now = Timestamp::now(&self.parts.clock).map_err(PendingError::Clock)?;
        let elapsed = now.as_utc() - served.started.as_utc();
        let pending = Pending {
            request: served.request,
            team: team.clone(),
            requester: served.requester.clone(),
            requester_task: served.requester_task.clone(),
            worker: served.worker.clone(),
            worker_task: worker_task.clone(),
            summary,
            run_seconds: u64::try_from(elapsed.whole_seconds()).unwrap_or(0),
            base: served.base.clone(),
            stage: Stage::Ended,
        };
        let task_dir = self.parts.store.home().task_dir(worker_task);
        write_pending(&task_dir, &pending)
    }

    /// The git handle of the worktree of `task`, a task of `team`, for `agent`; `None` when the team or the path is not
    /// there.
    fn git_of(&self, team: &TeamId, task: &TaskId, agent: &AgentName) -> Option<Git> {
        let path = self.worker_worktree(team, task)?;
        let worktree = AbsolutePath::from_str(path.to_str()?).ok()?;
        Some(Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), agent),
        ))
    }

    /// The worktree of `task`, a task of `team`.
    fn worker_worktree(&self, team: &TeamId, task: &TaskId) -> Option<PathBuf> {
        let repo = self.parts.state.team(team)?.repo().clone();
        Some(self.parts.store.home().worktree_dir(&repo, task))
    }

    /// The commit the worktree of the worker task `task` is at, as text.
    pub(super) async fn worker_head(&self, team: &TeamId, task: &TaskId, agent: &AgentName) -> Option<String> {
        let git = self.git_of(team, task, agent)?;
        git.rev_parse(&Revision::Head).await.ok().map(|head| head.to_string())
    }

    /// The round the requester's last run worked for, which a snapshot of its worktree is kept under.
    fn requester_round(&self, team: &TeamId, task: &TaskId) -> Option<agent_core::ids::RoundId> {
        let (_, member) = self.parts.state.team(team)?.find(task)?;
        member.runs().last()?.rounds().first().cloned()
    }

    /// Settles the requests of the workers of `team` whose runs ended: one stage after another, each written down before
    /// the next, and the answer at the end. A stage the bus cannot take now is tried again at the next look; a file that
    /// cannot be read is told once and left alone.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock or the state fails.
    pub(super) async fn settle_worker_results(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        let Some(found) = self.parts.state.team(team) else {
            return Ok(());
        };
        let workers: Vec<(TaskId, AgentName)> = found
            .crew()
            .roster()
            .into_iter()
            .filter(|(role, _)| *role == Role::Worker)
            .map(|(_, member)| (member.task().clone(), member.agent().agent_name()))
            .collect();
        for (task, worker) in workers {
            let task_dir = self.parts.store.home().task_dir(&task);
            let read = match read_pending_each(&task_dir).await {
                Ok(read) => read,
                Err(error) => {
                    self.tell_unreadable_once(&error);
                    continue;
                }
            };
            for item in read {
                match item {
                    Ok(pending) => {
                        if self
                            .request_checking
                            .values()
                            .any(|checking| checking.pending.request == pending.request)
                        {
                            continue;
                        }
                        self.settle_one(&task_dir, pending).await?;
                    }
                    Err(error) => self.settle_damaged(team, &task, &worker, error).await?,
                }
            }
        }
        Ok(())
    }

    /// Settles a request whose file cannot be used (cut short, unknown field, cannot be read): it is the fault of that one
    /// request. The file is set aside, kept, the fault is logged with the file's name, and the requester is told that the
    /// result of the worker cannot be judged — a wrong result — through a settled file of its own, so that the answer
    /// goes out as every other one does, once, also across a restart. The other requests and the owner go on.
    async fn settle_damaged(
        &mut self,
        team: &TeamId,
        worker_task: &TaskId,
        worker: &AgentName,
        error: PendingError,
    ) -> Result<(), OwnerError> {
        let Some(path) = error.path().map(Path::to_owned) else {
            self.tell_unreadable_once(&error);
            return Ok(());
        };
        let request = request_of_file(&path);
        // What the bus knows of the request: who is to be told. A bus that cannot say now is asked again at the next look,
        // before anything is moved.
        let known = match request {
            Some(request) => match self.parts.bus.helper_get(request).await {
                Ok(known) => Some((request, known)),
                Err(failure) if failure.class() == Some(FailureClass::NotFound) => None,
                Err(failure) => {
                    self.tell_unreadable_once(&error);
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        file = ?path,
                        error = %error_chain(&failure),
                        "who asked for a request whose file is damaged cannot be read now: it is tried again at the next look"
                    );
                    return Ok(());
                }
            },
            None => None,
        };
        let aside = match set_aside(&path).await {
            Ok(aside) => aside,
            Err(failure) => {
                self.tell_unreadable_once(&error);
                tracing::warn!(
                    event = LogEvent::RestartCorrected.as_str(),
                    file = ?path,
                    error = %error_chain(&failure),
                    "the damaged file of a worker's request cannot be set aside: it is left as it is"
                );
                return Ok(());
            }
        };
        tracing::warn!(
            event = LogEvent::RestartCorrected.as_str(),
            team = %team,
            task = %worker_task,
            file = ?path,
            kept_as = ?aside,
            request = ?request.map(|request| request.to_string()),
            error = %error_chain(&error),
            "the file of a worker's request cannot be used: it is set aside and the request is settled as a wrong result"
        );
        let Some((request, known)) = known else {
            return Ok(());
        };
        // A request the bus has answered already is not answered again: the file is only set aside.
        if !is_running_for(&known, worker_task) {
            return Ok(());
        }
        let step = match self.parts.bus.helper_work(request).await {
            Ok(work) => work.step,
            Err(failure) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    request = %request,
                    error = %error_chain(&failure),
                    "the step of a request whose file is damaged cannot be read: the requester is told without it"
                );
                None
            }
        };
        let pending = returned_for_damage(
            (team, worker_task, worker),
            (request, &known),
            step,
            damage_reason(&path, &aside, &error),
        );
        let task_dir = self.parts.store.home().task_dir(worker_task);
        // Kept first, so that an answer the bus cannot take now is tried again at the next look.
        match write_pending(&task_dir, &pending) {
            Ok(()) => self.settle_one(&task_dir, pending).await,
            Err(failure) => self.settle_unkept(&task_dir, pending, &failure).await,
        }
    }

    /// Says once, in the log, that a request of a worker cannot be read: the daemon goes on without it.
    fn tell_unreadable_once(&mut self, error: &PendingError) {
        if self.unreadable_requests.insert(error.to_string()) {
            tracing::warn!(
                event = LogEvent::RestartCorrected.as_str(),
                error = %error_chain(error),
                "the file of a worker's request cannot be read: it is left as it is and the daemon goes on without it"
            );
        }
    }

    /// Takes `pending` through the stages that remain.
    async fn settle_one(&mut self, task_dir: &Path, mut pending: Pending) -> Result<(), OwnerError> {
        // A request that was answered is answered once: what is left of it is only a file that could not be removed.
        if self.answered_requests.contains(&pending.request) {
            self.drop_pending_once(task_dir, pending.request);
            return Ok(());
        }
        loop {
            let next = match &pending.stage {
                Stage::Ended | Stage::Checked(_) if !self.still_the_workers(&pending).await => {
                    self.drop_pending_once(task_dir, pending.request);
                    return Ok(());
                }
                Stage::Ended => match self.gather_request(&pending).await {
                    Gathered::Later => return Ok(()),
                    Gathered::Gone => {
                        tracing::warn!(
                            request = %pending.request,
                            "the bus has no such request any more: the result of the worker is dropped"
                        );
                        Self::drop_pending(task_dir, pending.request);
                        return Ok(());
                    }
                    Gathered::Wrong(checked, reasons) => Stage::Settled {
                        checked: Box::new(checked),
                        settlement: Settlement::Returned { reasons },
                    },
                    Gathered::Checked(checked) => Stage::Checked(Box::new(checked)),
                    Gathered::Check(checked, step, text) => {
                        self.start_request_check(task_dir, pending, checked, step, text);
                        return Ok(());
                    }
                },
                Stage::Checked(checked) => {
                    let settlement = self.settle_result(&pending, checked).await;
                    Stage::Settled {
                        checked: checked.clone(),
                        settlement,
                    }
                }
                Stage::Settled { checked, settlement } => {
                    if self.answer_requester(&pending, checked, settlement).await? {
                        self.answered_requests.insert(pending.request);
                        self.drop_pending_once(task_dir, pending.request);
                    }
                    return Ok(());
                }
            };
            pending.stage = next;
            if let Err(error) = write_pending(task_dir, &pending) {
                return self.settle_unkept(task_dir, pending, &error).await;
            }
        }
    }

    /// A stage of `pending` cannot be written: that is the fault of this request only. Its file, if there is one, is set aside
    /// (kept), the fault is logged with the file's name, and the requester is told that the result cannot be judged, from
    /// memory, without another file. The other requests and the owner go on.
    async fn settle_unkept(
        &mut self,
        task_dir: &Path,
        mut pending: Pending,
        error: &PendingError,
    ) -> Result<(), OwnerError> {
        let path = pending_path(task_dir, pending.request);
        let kept_as = match set_aside(&path).await {
            Ok(aside) => Some(aside),
            Err(PendingError::Write { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(failure) => {
                tracing::warn!(
                    event = LogEvent::RestartCorrected.as_str(),
                    file = ?path,
                    error = %error_chain(&failure),
                    "the file of a worker's request cannot be set aside"
                );
                None
            }
        };
        tracing::warn!(
            event = LogEvent::RestartCorrected.as_str(),
            team = %pending.team,
            task = %pending.worker_task,
            file = ?path,
            kept_as = ?kept_as,
            request = %pending.request,
            error = %error_chain(error),
            "a stage of a worker's request cannot be kept: the request is settled as a wrong result"
        );
        let step = match &pending.stage {
            Stage::Ended => None,
            Stage::Checked(checked) | Stage::Settled { checked, .. } => checked.step,
        };
        let checked = Checked {
            step,
            files: Vec::new(),
            head: None,
            committed: false,
            outside: Vec::new(),
            found: None,
        };
        let settlement = Settlement::Returned {
            reasons: vec![format!(
                "the harness cannot keep its record of the worker's result in {} ({}), so what the worker did cannot be judged",
                path.file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                error_chain(error)
            )],
        };
        pending.stage = Stage::Settled {
            checked: Box::new(checked.clone()),
            settlement: settlement.clone(),
        };
        if self.answer_requester(&pending, &checked, &settlement).await? {
            self.answered_requests.insert(pending.request);
        }
        Ok(())
    }

    /// Whether the bus still has the request of `pending` running for its worker's task. A request that was answered, or is
    /// another worker's, is not settled from its file; one the bus cannot be asked about now is taken as running, and the
    /// bus is asked again at the next stage.
    async fn still_the_workers(&self, pending: &Pending) -> bool {
        match self.parts.bus.helper_get(pending.request).await {
            Ok(known) if is_running_for(&known, &pending.worker_task) => true,
            Ok(known) => {
                tracing::warn!(
                    event = LogEvent::RestartCorrected.as_str(),
                    request = %pending.request,
                    status = ?known.status,
                    "the bus does not have a request running for the worker whose file holds its result: the file is dropped"
                );
                false
            }
            Err(error) if error.class() == Some(FailureClass::NotFound) => true, // The stage that reads the request drops its file.
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    request = %pending.request,
                    error = %error_chain(&error),
                    "the state of a request cannot be read now: its result is settled as if it still ran"
                );
                true
            }
        }
    }

    /// Removes the file of `request` from `task_dir`, and says once, if it cannot, so that a look does not repeat it.
    fn drop_pending_once(&mut self, task_dir: &Path, request: RequestId) {
        let path = pending_path(task_dir, request);
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
            && self.unreadable_requests.insert(format!("remove {}", path.display()))
        {
            tracing::warn!(
                event = LogEvent::RestartCorrected.as_str(),
                path = ?path,
                error = %error,
                "the file of a settled request cannot be removed"
            );
        }
    }

    /// Removes the file of `request` from `task_dir`; a file that is not there is the end the request wants.
    fn drop_pending(task_dir: &Path, request: RequestId) {
        let path = pending_path(task_dir, request);
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                event = LogEvent::RestartCorrected.as_str(),
                path = ?path,
                error = %error,
                "the file of a settled request cannot be removed"
            );
        }
    }

    /// Reads what the request asks and what the worker left, before the check: a result that is wrong already is told so.
    async fn gather_request(&self, pending: &Pending) -> Gathered {
        let work = match self.parts.bus.helper_work(pending.request).await {
            Ok(work) => work,
            Err(error) if error.class() == Some(FailureClass::NotFound) => return Gathered::Gone,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    request = %pending.request,
                    error = %error_chain(&error),
                    "what a request asks cannot be read now: it is tried again at the next look"
                );
                return Gathered::Later;
            }
        };
        let files: Vec<String> = work.files.iter().map(|path| path.as_str().to_owned()).collect();
        let mut checked = Checked {
            step: work.step,
            files,
            head: None,
            committed: false,
            outside: Vec::new(),
            found: None,
        };
        // A request that writes files is judged by what the worker committed: whatever cannot be established is a reason to
        // return the step, never a reason to bring something over.
        if scope_of(&checked.files) == Scope::Files && checked.step.is_some() {
            match self.what_the_worker_left(pending, &work.files).await {
                Ok((head, committed, outside)) => {
                    checked.head = Some(head);
                    checked.committed = committed;
                    checked.outside = outside;
                }
                Err(reasons) => return Gathered::Wrong(checked, reasons),
            }
        }
        match (work.check, work.step) {
            (Some(text), Some(step)) => Gathered::Check(checked, step, text),
            _ => Gathered::Checked(checked),
        }
    }

    /// What the worker's worktree holds: its commit, whether it committed since the request was given to it, the files its
    /// commits changed outside `allowed`; or why that cannot be established, or the worker left changes uncommitted.
    async fn what_the_worker_left(
        &self,
        pending: &Pending,
        allowed: &[agent_core::repo_path::RepoPath],
    ) -> Result<(String, bool, Vec<String>), Vec<String>> {
        let fail = |reason: String| vec![reason];
        let Some(git) = self.git_of(&pending.team, &pending.worker_task, &pending.worker) else {
            return Err(fail("the worker's worktree cannot be reached".to_owned()));
        };
        let head = git
            .rev_parse(&Revision::Head)
            .await
            .map_err(|error| fail(format!("the worker's commit cannot be read: {}", error_chain(&error))))?;
        let base = pending
            .base
            .as_deref()
            .and_then(|base| CommitId::from_str(base).ok())
            .ok_or_else(|| {
                fail("the commit the worker started from is not known, so what it changed cannot be told".to_owned())
            })?;
        let committed = head != base;
        let mut outside = Vec::new();
        if committed {
            let changed = git.changed_between(&base, &head).await.map_err(|error| {
                fail(format!(
                    "the files the worker changed cannot be listed: {}",
                    error_chain(&error)
                ))
            })?;
            outside = changed
                .into_iter()
                .filter(|path| !allowed.contains(path))
                .map(|path| path.as_str().to_owned())
                .collect();
        }
        let uncommitted = git.uncommitted_files().await.map_err(|error| {
            fail(format!(
                "the files the worker left cannot be listed: {}",
                error_chain(&error)
            ))
        })?;
        // Only what the request names matters: a file the request did not name is never brought over, so leaving it is
        // harmless, and the files the harness itself keeps in a worktree are not the worker's work.
        let left: Vec<&agent_core::repo_path::RepoPath> = uncommitted
            .iter()
            .filter(|path| {
                allowed.iter().any(|named| {
                    path.as_str() == named.as_str()
                        || path
                            .as_str()
                            .strip_prefix(named.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                })
            })
            .collect();
        if !left.is_empty() {
            let names: Vec<&str> = left.iter().map(|path| path.as_str()).collect();
            return Err(fail(format!(
                "the worker left files uncommitted that a commit does not hold: {}",
                names.join(", ")
            )));
        }
        Ok((head.to_string(), committed, outside))
    }

    /// Starts the check `text` of the request in the background, so that the owner goes on while it runs.
    fn start_request_check(&mut self, task_dir: &Path, pending: Pending, checked: Checked, step: ItemId, text: String) {
        let config = &self.parts.settings.config;
        let job = RequestCheck {
            worktree: self.worker_worktree(&pending.team, &pending.worker_task),
            task_dir: task_dir.to_owned(),
            step,
            text,
            seconds: config.plan().check_seconds(),
            refuse: config.plan().check_refuse().to_vec(),
        };
        let abort = self.request_checks.spawn(run_request_check(job));
        self.request_checking.insert(
            abort.id(),
            Checking {
                task_dir: task_dir.to_owned(),
                pending,
                checked,
            },
        );
    }

    /// Takes the end of a check that ran in the background: the request goes on from the stage `Checked`.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock or the state fails.
    pub(super) async fn request_check_done(
        &mut self,
        joined: Result<(tokio::task::Id, Found), tokio::task::JoinError>,
    ) -> Result<(), OwnerError> {
        let (id, found) = match joined {
            Ok(done) => done,
            Err(error) => (
                error.id(),
                Found {
                    outcome: CheckOutcome::Failed,
                    line: String::new(),
                    tail: String::new(),
                    reason: Some(format!("the check could not run: {error}")),
                    duration_ms: 0,
                    output: None,
                },
            ),
        };
        let Some(checking) = self.request_checking.remove(&id) else {
            return Ok(());
        };
        let Checking {
            task_dir,
            mut pending,
            mut checked,
        } = checking;
        checked.found = Some(found);
        let team = pending.team.clone();
        pending.stage = Stage::Checked(Box::new(checked));
        if let Err(error) = write_pending(&task_dir, &pending) {
            return self.settle_unkept(&task_dir, pending, &error).await;
        }
        self.look_at(&team);
        Ok(())
    }

    /// Decides what the checked result is worth and, when it is good, brings the worker's commit over.
    async fn settle_result(&self, pending: &Pending, checked: &Checked) -> Settlement {
        if checked.step.is_none() || (checked.files.is_empty() && checked.found.is_none()) {
            return Settlement::Report;
        }
        let limit = self.parts.settings.config.workers().seconds();
        let facts = facts_of(checked, pending.run_seconds, limit);
        match decide(&facts) {
            Verdict::Accept if facts.scope == Scope::Facts => Settlement::Report,
            Verdict::Accept if !checked.committed => Settlement::Returned {
                reasons: vec!["the worker committed nothing, so there is nothing to bring over".to_owned()],
            },
            Verdict::Accept => self.bring_over(pending, checked).await,
            Verdict::GiveToAnother { wrong } | Verdict::Return { wrong } => Settlement::Returned {
                reasons: reasons_of(checked, &wrong, limit),
            },
        }
    }

    /// Brings what the worker committed since the request was given to it to the requester's branch, once: a commit that is
    /// there already is not brought again.
    async fn bring_over(&self, pending: &Pending, checked: &Checked) -> Settlement {
        let returned = |reason: String| Settlement::Returned { reasons: vec![reason] };
        let Some(head) = checked.head.as_deref().and_then(|head| CommitId::from_str(head).ok()) else {
            return returned("the worker's commit cannot be read".to_owned());
        };
        let Some(base) = pending.base.as_deref().and_then(|base| CommitId::from_str(base).ok()) else {
            return returned("the commit the worker started from is not known".to_owned());
        };
        let Some(git) = self.git_of(&pending.team, &pending.requester_task, &pending.requester) else {
            return returned("the requester's worktree cannot be reached".to_owned());
        };
        match git.brought_over(&head).await {
            Ok(Some(commit)) => {
                return Settlement::Brought {
                    commit: commit.to_string(),
                };
            }
            Ok(None) => {}
            Err(error) => {
                return returned(format!(
                    "the requester's branch cannot be read: {}",
                    error_chain(&error)
                ));
            }
        }
        let Some(round) = self.requester_round(&pending.team, &pending.requester_task) else {
            return returned("the requester has no round to keep a snapshot of its worktree under".to_owned());
        };
        match git
            .bring_over_since(&base, &head, &round, &pending.requester_task)
            .await
        {
            Ok(BringOver::Applied { commit, .. }) => Settlement::Brought {
                commit: commit.to_string(),
            },
            Ok(BringOver::Conflicted { files, .. } | BringOver::InTheWay { files, .. }) => Settlement::Conflict {
                files: files.iter().map(|path| path.as_str().to_owned()).collect(),
            },
            Err(error) => returned(format!("the commit cannot be brought over: {}", error_chain(&error))),
        }
    }

    /// Answers the bus and tells the requester for a settled request; `true` when both are done, `false` when the bus
    /// cannot take them now and they are tried again at the next look.
    async fn answer_requester(
        &mut self,
        pending: &Pending,
        checked: &Checked,
        settlement: &Settlement,
    ) -> Result<bool, OwnerError> {
        let (status, summary, notice) = match self.words_of(pending, checked, settlement) {
            Ok(words) => words,
            Err(error) => {
                // The catalog's texts are checked by a test: this is a bug, and the requester is told what it can be told.
                tracing::warn!(
                    event = LogEvent::RestartCorrected.as_str(),
                    error = %error_chain(&error),
                    "the answer to a request cannot be worded"
                );
                let text = format!(
                    "Your request H{} to the workers is answered, but the harness could not word the answer. The worker's report:\n\n{}",
                    pending.request.number(),
                    pending.summary
                );
                (
                    DelegationStatus::Failed,
                    text.clone(),
                    BusNotice::HelperFailed {
                        request: pending.request,
                        worker: pending.worker.clone(),
                        reason: text,
                    },
                )
            }
        };
        tracing::info!(
            request = %pending.request,
            worker = %pending.worker,
            requester = %pending.requester,
            step = ?checked.step.map(|step| step.to_string()),
            outcome = settlement.outcome(),
            run_seconds = pending.run_seconds,
            "the result of a worker is settled"
        );
        let jobs = [
            BusJob::Answer {
                request: pending.request,
                update: DelegationUpdate {
                    status: Some(status),
                    task: None,
                    summary: Some(summary),
                },
            },
            BusJob::Tell {
                to: pending.requester.clone(),
                notice,
            },
        ];
        for job in jobs {
            let outbound = Outbound {
                team: pending.team.clone(),
                job,
            };
            match self.carry_out(&outbound).await? {
                Ok(()) => {}
                // What the bus refuses for good would be asked for ever: it is told and let go.
                Err(error) if !is_transient(&error) => {
                    tracing::warn!(
                        request = %pending.request,
                        error = %error_chain(&error),
                        "the bus refuses the answer to a request for good: it is dropped"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        request = %pending.request,
                        error = %error_chain(&error),
                        "the bus does not answer: the answer to a request is tried again at the next look"
                    );
                    return Ok(false);
                }
            }
        }
        if let (Settlement::Brought { commit }, Some(step)) = (settlement, checked.step) {
            self.record_confirmed(pending, checked, step, commit).await?;
        }
        Ok(true)
    }

    /// The status, the summary and the notice of the answer.
    fn words_of(
        &self,
        pending: &Pending,
        checked: &Checked,
        settlement: &Settlement,
    ) -> Result<(DelegationStatus, String, BusNotice), NoticeRenderError> {
        let texts: &TeamPrompts = &self.parts.team_texts;
        let step = checked.step.map_or_else(String::new, |step| step.to_string());
        let tail = checked
            .found
            .as_ref()
            .map(|found| found.tail.trim().to_owned())
            .filter(|tail| !tail.is_empty())
            .unwrap_or_else(|| NO_OUTPUT.to_owned());
        let files = checked.files.join(", ");
        let (status, text) = match settlement {
            Settlement::Report => (DelegationStatus::Done, pending.summary.clone()),
            Settlement::Brought { commit } => (
                DelegationStatus::Done,
                crate::daemon::team::worker_result_brought(texts, &pending.worker, &step, commit, &files, &tail)?,
            ),
            Settlement::Conflict { files } => (
                DelegationStatus::Failed,
                crate::daemon::team::worker_result_conflict(
                    texts,
                    &pending.worker,
                    &step,
                    checked.head.as_deref().unwrap_or_default(),
                    &files.join(", "),
                )?,
            ),
            Settlement::Returned { reasons } => (
                DelegationStatus::Failed,
                crate::daemon::team::worker_step_returned(texts, pending.request, &step, reasons)?,
            ),
        };
        let notice = match status {
            DelegationStatus::Done => BusNotice::HelperResult {
                request: pending.request,
                worker: pending.worker.clone(),
                summary: text.clone(),
            },
            _ => BusNotice::HelperFailed {
                request: pending.request,
                worker: pending.worker.clone(),
                reason: text.clone(),
            },
        };
        Ok((status, text, notice))
    }

    /// Records in the history that the check of step `step` of the requester's plan passed on the worker's commit.
    async fn record_confirmed(
        &mut self,
        pending: &Pending,
        checked: &Checked,
        step: ItemId,
        commit: &str,
    ) -> Result<(), OwnerError> {
        let at = Timestamp::now(&self.parts.clock)?;
        let record = agent_runtime::history::JudgementRecord {
            task: pending.requester_task.clone(),
            team: pending.team.clone(),
            run: None,
            at,
            step,
            state: JudgementState::Confirmed,
            check: checked.found.as_ref().map(|found| found.line.clone()),
            reason: Some(format!(
                "done by the worker {} (H{}), commit {commit}",
                pending.worker,
                pending.request.number()
            )),
            duration_ms: checked.found.as_ref().map_or(0, |found| found.duration_ms),
            output_path: checked.found.as_ref().and_then(|found| found.output.clone()),
        };
        self.record_judgements(&pending.requester_task, vec![record])
            .await
            .map(drop)
    }
}

/// The reason a request whose file cannot be used is returned with: the file, why, and where it is kept.
fn damage_reason(path: &Path, aside: &Path, error: &PendingError) -> String {
    let name = |path: &Path| {
        path.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    };
    format!(
        "the harness's record of the worker's result, {}, cannot be used ({}); it is kept as {}, and what the worker did cannot \
         be judged",
        name(path),
        error_chain(error),
        name(aside)
    )
}

/// The file of a request that is settled as returned, with `reason`, for the request `known` to the bus, given to the worker
/// of `who` (team, worker task, worker).
fn returned_for_damage(
    who: (&TeamId, &TaskId, &AgentName),
    (request, known): (RequestId, &DelegationRequest),
    step: Option<ItemId>,
    reason: String,
) -> Pending {
    Pending {
        request,
        team: who.0.clone(),
        requester: known.from.clone(),
        requester_task: known.from_task.clone(),
        worker: who.2.clone(),
        worker_task: who.1.clone(),
        summary: known.summary.clone(),
        run_seconds: 0,
        base: None,
        stage: Stage::Settled {
            checked: Box::new(Checked {
                step,
                files: Vec::new(),
                head: None,
                committed: false,
                outside: Vec::new(),
                found: None,
            }),
            settlement: Settlement::Returned { reasons: vec![reason] },
        },
    }
}

/// Whether the bus has the request `known` still in the hands of the worker whose task is `worker_task`: only such a request
/// is settled from a file; one that is answered, queued or another worker's is not the file's to settle.
fn is_running_for(known: &DelegationRequest, worker_task: &TaskId) -> bool {
    matches!(known.status, DelegationStatus::Running | DelegationStatus::Starting)
        && known.helper_task.as_ref() == Some(worker_task)
}

/// What a check in the background needs: it runs without the owner.
#[derive(Debug)]
struct RequestCheck {
    /// The worker's worktree; `None` when it is not there.
    worktree: Option<PathBuf>,
    /// The worker's task directory, where the output is kept.
    task_dir: PathBuf,
    /// The step of the requester's plan the request is for.
    step: ItemId,
    /// The check, as the request has it.
    text: String,
    /// `[plan] check_seconds`.
    seconds: u64,
    /// `[plan] check_refuse`.
    refuse: Vec<String>,
}

/// Runs the check of a request in the worker's worktree.
async fn run_request_check(job: RequestCheck) -> Found {
    let failed = |reason: String| Found {
        outcome: CheckOutcome::Failed,
        line: job.text.clone(),
        tail: String::new(),
        reason: Some(reason),
        duration_ms: 0,
        output: None,
    };
    let Ok(check) = Check::from_str(&job.text) else {
        return failed("the check is not one the harness can run".to_owned());
    };
    let Some(worktree) = &job.worktree else {
        return failed("the worker's worktree cannot be reached".to_owned());
    };
    let context = CheckContext {
        worktree,
        task_dir: &job.task_dir,
        run: None,
        check_seconds: job.seconds,
        check_refuse: &job.refuse,
        envs: &[],
    };
    match execute_request_check(&context, &job.step, &check).await {
        Ok(result) => {
            let tail = match &result.output_path {
                Some(path) => match read_check_tail(&job.task_dir.join(path)).await {
                    Ok(tail) => tail,
                    Err(error) => {
                        tracing::warn!(
                            event = LogEvent::RestartCorrected.as_str(),
                            path = ?path,
                            error = %error_chain(&error),
                            "the output of a request's check cannot be read: the requester is told without its end"
                        );
                        String::new()
                    }
                },
                None => result.tail.clone(),
            };
            Found {
                outcome: request_check_outcome(&result),
                line: result.check_line.clone().unwrap_or_else(|| job.text.clone()),
                tail,
                reason: result.reason.clone(),
                duration_ms: result.duration_ms,
                output: result.output_path.clone(),
            }
        }
        Err(error) => failed(error_chain(&error)),
    }
}

#[cfg(test)]
mod tests;
