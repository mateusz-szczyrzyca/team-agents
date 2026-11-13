//! The checks before a review, carried out: reading what the round did from git, asking the round machine what it comes
//! to, and what follows — the team is sent back with the reasons, the human is asked, or the review goes on.
//!
//! The round of a publication is checked when its branch is ready — the lead waits and its gate passed at the tip — and
//! before the pull request goes to the reviewers: for a first round after the draft is opened and before it is marked
//! ready, for a fix pass before the branch is pushed. What the checks hold for each item — passes, what the human allowed,
//! the question they wait on, what the lead is still to be told — is the record in the lead's task directory
//! (`checks.json`), so that a daemon that restarts goes on. Without a `checks` table for the repository nothing is checked.

use std::path::PathBuf;

use agent_core::decide::checks::{Subject, assess_near, markers};
use agent_core::decide::round::{Checking, ChecksPolicy, ChecksStep};
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::prompt::Variables;
use agent_core::repo_path::RepoPath;
use agent_core::state::{Asked, ChecksRecord, PrNumber, Sent, Timestamp};
use agent_runtime::git::Revision;
use agent_runtime::notify::Notification;
use agent_runtime::text_file::replace_text_file;

use super::super::start::next_number;
use super::super::{Notice, Owner, OwnerError};
use super::publish::{Ready, Site};
use super::{Announcement, Lead, PublishError, Pulls};
use crate::daemon::checks::{MAX_RELAYS, authored, decisions, questions, team_message};
use crate::daemon::questions;
use crate::daemon::team::Why;

use super::super::teams::Look;

/// The permission bits of a questions file the harness writes: the owner writes it, everybody may read it.
const QUESTIONS_FILE_MODE: u32 = 0o644;

/// A round to check: the items that share the pull request, the branch tip the gate passed on and the pull request with
/// its description as it is now.
pub(super) struct Reviewing<'a> {
    /// The items of the pull request, in the order of the list.
    pub(super) items: &'a [ItemId],
    /// The branch that is ready, in the lead's worktree.
    pub(super) ready: &'a Ready,
    /// The pull request the round goes to.
    pub(super) pr: PrNumber,
    /// The description of the pull request.
    pub(super) description: &'a str,
}

/// What the checks ask the owner to do about a round they hold back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    /// The round waits on a question that is open, and nothing is to be done.
    Waiting,
    /// The lead is woken: to be sent back with the reasons, or to put the question to the human.
    WakeLead,
    /// The lead was woken to put the question and did not: the human is told so directly.
    Notify,
}

/// Where the question the checks wait on is.
enum Asking {
    /// The human has not answered.
    Open,
    /// The human answered; the questions file is here.
    Answered(PathBuf),
    /// The lead has not put the question yet; `ran` is whether it ran since the question was written.
    NotYet {
        /// Whether the lead started a run since.
        ran: bool,
    },
}

impl Owner {
    /// Checks the round of `reviewing`; `None` when the review may start, and what to do when it may not.
    ///
    /// # Errors
    /// [`PublishError`] when git, the record, the questions or a text of the checks fail: the publication is tried again.
    pub(super) async fn check_before_review(
        &mut self,
        site: Site<'_>,
        reviewing: &Reviewing<'_>,
        now: Timestamp,
    ) -> Result<Option<Held>, PublishError> {
        let Site { pulls, team, lead } = site;
        let Some(rules) = self
            .parts
            .settings
            .config
            .repo(&lead.repo)
            .and_then(|repo| repo.checks.clone())
        else {
            return Ok(None);
        };
        // A lead the checks have not seen has no record: the empty one is what it holds.
        let mut record = self
            .parts
            .store
            .load_checks_record(&lead.task)
            .await?
            .unwrap_or_default();
        let mut refused: Vec<Subject> = Vec::new();
        if let Some(asked) = record.asked.clone() {
            match self.where_is(&lead.task, &asked)? {
                Asking::Open => return Ok(Some(Held::Waiting)),
                Asking::NotYet { ran: false } => return Ok(Some(Held::WakeLead)),
                Asking::NotYet { ran: true } if asked.relays >= MAX_RELAYS => return Ok(Some(Held::Notify)),
                Asking::NotYet { ran: true } => {
                    record.asked = Some(Asked {
                        relays: asked.relays.saturating_add(1),
                        at: now,
                        ..asked
                    });
                    self.parts.store.save_checks_record(&lead.task, &record).await?;
                    return Ok(Some(Held::WakeLead));
                }
                Asking::Answered(file) => {
                    let loaded = questions::read(&file).await?;
                    let decided = decisions(&asked.subjects, &loaded.file);
                    record.allow(&asked.items, &decided.allowed);
                    record.restart_passes(&asked.items);
                    record.asked = None;
                    self.parts.store.save_checks_record(&lead.task, &record).await?;
                    tracing::info!(team = %team, allowed = decided.allowed.len(), refused = decided.refused.len(), "the human answered the questions of the checks");
                    refused = decided.refused;
                }
            }
        }
        // The description the harness would write for these items names files in passing: what counts is what somebody added.
        let harness = self.pull_text_of(pulls, team, &lead.repo, reviewing.items, &reviewing.ready.record)?;
        let written = authored(reviewing.description, &harness.body);
        let issues = self.round_issues(lead, reviewing, &written, &rules, &record).await?;
        let policy = ChecksPolicy::new(self.parts.settings.config.review());
        let checking = Checking::after(record.passes_of(reviewing.items));
        match checking.conclude(&issues, &refused, policy) {
            ChecksStep::StartReviews => Ok(None),
            ChecksStep::BackToTeam { pass } => {
                let text = team_message(
                    pulls.checks(),
                    reviewing.pr,
                    pass,
                    policy.check_passes,
                    &issues,
                    &refused,
                )?;
                record.set_passes(reviewing.items, pass.get());
                record.asked = None;
                record.sent = Some(Sent {
                    at: now,
                    pass: pass.get(),
                    of: policy.check_passes,
                    text,
                });
                self.parts.store.save_checks_record(&lead.task, &record).await?;
                tracing::info!(team = %team, pr = %reviewing.pr, pass = pass.get(), issues = issues.len(), "the checks send the round back to the team");
                Ok(Some(Held::WakeLead))
            }
            ChecksStep::AskHuman { reason } => {
                let number = self
                    .parts
                    .state
                    .find_task(&lead.task)
                    .and_then(|found| next_number(found.member))
                    .ok_or(PublishError::QuestionNumbers)?;
                let asked = questions(pulls.checks(), team, &lead.task, &issues, reason, checking.passes())?;
                let file = self.parts.store.home().questions_file(&lead.task, number);
                replace_text_file(&file, &asked.file.render(), QUESTIONS_FILE_MODE).await?;
                record.sent = None;
                record.asked = Some(Asked {
                    number,
                    at: now,
                    items: reviewing.items.to_vec(),
                    subjects: asked.subjects,
                    relays: 0,
                });
                self.parts.store.save_checks_record(&lead.task, &record).await?;
                tracing::info!(team = %team, pr = %reviewing.pr, ?reason, issues = issues.len(), "the checks ask the human about the round");
                Ok(Some(Held::WakeLead))
            }
        }
    }

    /// What the checks find in the round of `reviewing`, whose pull request somebody described as `description`, without what
    /// the human allowed for its items.
    async fn round_issues(
        &self,
        lead: &Lead,
        reviewing: &Reviewing<'_>,
        description: &str,
        rules: &agent_core::config::Checks,
        record: &ChecksRecord,
    ) -> Result<Vec<agent_core::decide::checks::Issue>, PublishError> {
        let git = &reviewing.ready.git;
        let tip = &reviewing.ready.tip;
        let base = self.base_branch(&lead.repo)?;
        git.fetch(&base).await?;
        let merge_base = git
            .merge_base(&Revision::RemoteBranch(base), &Revision::Commit(tip.clone()))
            .await?;
        let facts = git.round_facts(&merge_base, tip, description).await?;
        // The files with added lines, once each, whose attributes wrap over lines: their unchanged lines complete an
        // attribute that the round edits.
        let mut edited: Vec<RepoPath> = facts
            .added
            .iter()
            .filter(|line| line.path.extension().is_some_and(markers::wraps_attributes))
            .map(|line| line.path.clone())
            .collect();
        edited.dedup();
        let unchanged = git.range_unchanged_lines(&merge_base, tip, &edited).await?;
        Ok(assess_near(
            rules,
            &facts,
            &unchanged,
            &record.allowed_of(reviewing.items),
        ))
    }

    /// Where the question `asked` of the lead `task` is.
    fn where_is(&self, task: &TaskId, asked: &Asked) -> Result<Asking, PublishError> {
        let found = self.parts.state.find_task(task).ok_or(PublishError::Lost)?;
        Ok(
            match found
                .member
                .questions()
                .iter()
                .find(|question| question.number() == asked.number)
            {
                Some(question) if question.is_open() => Asking::Open,
                Some(question) => Asking::Answered(question.file().as_path().to_path_buf()),
                None => Asking::NotYet {
                    ran: found
                        .member
                        .runs()
                        .last()
                        .is_some_and(|run| run.started_at() >= asked.at),
                },
            },
        )
    }

    /// Does what the checks ask about the round of the team `team_id`: wakes its lead, or tells the human.
    pub(super) fn hold_round(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        held: Held,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        match held {
            Held::Waiting => Ok(()),
            Held::WakeLead => self.release(&lead.task, Why::Checks, look),
            Held::Notify => {
                let announcement = Announcement::NotAsked {
                    team: team_id.clone(),
                    task: lead.task.clone(),
                };
                if !self.watch.announced.insert(announcement) {
                    return Ok(());
                }
                let now = Timestamp::now(&self.parts.clock)?;
                match not_asked(pulls, team_id, &lead.task) {
                    Ok(notification) => match self.locate(&lead.task) {
                        Ok(located) => self.notify(now, &located, &lead.task, Notice::Prepared(notification)),
                        Err(error) => {
                            tracing::error!(task = %lead.task, %error, "a notification of the checks has no task to be about");
                        }
                    },
                    Err(error) => {
                        tracing::error!(task = %lead.task, error = %agent_runtime::process::error_chain(&error), "a notification of the checks cannot be made");
                    }
                }
                Ok(())
            }
        }
    }
}

/// The notification that the lead `task` of `team` did not put the question of the checks to the human.
fn not_asked(pulls: &Pulls, team: &TeamId, task: &TaskId) -> Result<Notification, PublishError> {
    let values = Variables::new()
        .with("team", team.as_str())?
        .with("task", task.as_str())?;
    let rendered = pulls
        .checks()
        .render(agent_core::prompt::ChecksPrompt::NoticeNotAsked, &values)
        .map_err(crate::daemon::checks::ChecksTextError::from)?;
    let (title, body) = rendered.split_once('\n').unwrap_or((rendered.as_str(), ""));
    Ok(Notification::new(title.trim_end(), body.trim())?)
}
