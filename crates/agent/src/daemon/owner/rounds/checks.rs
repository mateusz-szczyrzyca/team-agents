//! The checks before a review, carried out: reading what the round did from git, asking the round machine what it comes
//! to, and what follows — the team is sent back with the reasons, the human is asked, or the review goes on.
//!
//! The round of a publication is checked when its branch is ready — the lead waits and its gate passed at the tip — and
//! before the pull request goes to the reviewers: for a first round after the draft is opened and before it is marked
//! ready, for a fix pass before the branch is pushed. What the checks hold for each item — passes, what the human allowed,
//! the question they wait on, what the lead is still to be told — is the record in the lead's task directory
//! (`checks.json`), so that a daemon that restarts goes on. Without a `checks` table for the repository nothing is checked.

use std::path::PathBuf;

use std::collections::BTreeMap;

use agent_core::decide::checks::commits::{self, CommitFinding, Said, Settled};
use agent_core::decide::checks::{Issue, Judge, RoundFacts, Subject, UnchangedLine, assess_near, markers};
use agent_core::decide::disclosure::{test_fix_section, with_section};
use agent_core::decide::round::{AskReason, Checking, ChecksPolicy, ChecksStep, CommitsLeft};
use agent_core::history::{CheckOutcome, HistoryEvent};
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::prompt::Variables;
use agent_core::repo_path::RepoPath;
use agent_core::state::{Asked, Basis, ChecksRecord, Covers, PrNumber, Sent, Timestamp};
use agent_runtime::git::Revision;
use agent_runtime::history::log_decision;
use agent_runtime::notify::Notification;
use agent_runtime::text_file::replace_text_file;

use super::super::start::next_number;
use super::super::{Owner, OwnerError};
use super::publish::{Ready, Site};
use super::{Announcement, Lead, PublishError, Pulls};
use crate::daemon::checks::{
    CommitQuestion, MAX_RELAYS, authored, grouped_decisions, named, questions_with_commits, reason, team_message,
};
use crate::daemon::notice::{NoticeContext, NoticeKind, layout_notification};
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
    /// Keeps authored remote disclosures and the current lead contribution visible to the checks.
    async fn written_description(&self, site: Site<'_>, reviewing: &Reviewing<'_>) -> Result<String, PublishError> {
        let Site { pulls, team, lead } = site;
        let header = self.harness_text_of(pulls, team, &lead.repo, reviewing.items, &reviewing.ready.record)?;
        let fresh = self
            .pull_text_of(pulls, team, &lead.repo, reviewing.items, &reviewing.ready.record)
            .await?;
        Ok([
            crate::daemon::pr::description::checks_description(&authored(reviewing.description, &header.body)),
            crate::daemon::pr::description::checks_description(&authored(&fresh.body, &header.body)),
        ]
        .join("\n\n"))
    }

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
        // What the human refused in the answer that is taken now: the team is told once.
        let mut refused_now: Vec<Subject> = Vec::new();
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
                    refused_now = self.take_answer(site, reviewing, &mut record, &asked, &file).await?;
                }
            }
        }
        let written = self.written_description(site, reviewing).await?;
        let gathered = self.gather(lead, reviewing, &written).await?;
        let team_allowances = self
            .parts
            .state
            .find_task(&lead.task)
            .ok_or(PublishError::Lost)?
            .team
            .allowances()
            .to_vec();
        // A file the human allowed before the findings about commits had a subject of their own covers the commits that
        // were in the round then: they are kept as they are before the file's allowance may be taken back.
        let inherited = judge(&gathered, &rules, &record, reviewing.items, &team_allowances)
            .settled
            .inherited;
        if !inherited.is_empty() {
            record.allow(reviewing.items, &inherited);
            self.parts.store.save_checks_record(&lead.task, &record).await?;
        }
        self.revoke_changed_allowances(site, reviewing, &mut record).await?;
        let Assessed { issues, settled } = judge(&gathered, &rules, &record, reviewing.items, &team_allowances);
        // What the human refused about a file keeps the round back until the team undoes it; what they refused about a
        // commit is told to the team once, when they refuse it: the team cannot undo a commit.
        let refused: Vec<Subject> = record
            .refused_of(reviewing.items)
            .into_iter()
            .filter(|subject| !matches!(subject, Subject::Commit { .. }))
            .collect();
        let refused_commits = refused_now
            .iter()
            .filter(|subject| matches!(subject, Subject::Commit { .. }))
            .count();
        let policy = ChecksPolicy::new(self.parts.settings.config.review());
        let checking = Checking::after(record.passes_of(reviewing.items));
        let step = checking.conclude_with_commits(
            &issues,
            &refused,
            CommitsLeft {
                to_ask: settled.ask.len(),
                refused_now: refused_commits,
            },
            policy,
        );
        log_decision(
            "conclude_checks",
            &format!(
                "pr={}, issues={}, refused={}",
                reviewing.pr,
                issues.len(),
                refused.len()
            ),
            &format!("{step:?}"),
        );
        match step {
            ChecksStep::StartReviews => Ok(None),
            ChecksStep::BackToTeam { pass } => {
                // The team is told what the human refused just now, and what they refused before about a file that
                // the round still changes.
                let told: Vec<Subject> = refused_now
                    .iter()
                    .chain(
                        refused
                            .iter()
                            .filter(|subject| issues.iter().any(|issue| issue.subject() == **subject)),
                    )
                    .cloned()
                    .collect::<std::collections::BTreeSet<Subject>>()
                    .into_iter()
                    .collect();
                let text = team_message(pulls.checks(), reviewing.pr, pass, policy.check_passes, &issues, &told)?;
                record.send_back(reviewing.items, pass.get());
                record.sent = Some(Sent {
                    at: now,
                    pass: pass.get(),
                    of: policy.check_passes,
                    text,
                });
                self.parts.store.save_checks_record(&lead.task, &record).await?;
                let lines = issue_lines(pulls, &issues)?;
                self.record_checked(
                    team,
                    reviewing,
                    lines,
                    CheckOutcome::SentBack {
                        pass,
                        of: policy.check_passes,
                    },
                );
                tracing::info!(
                    event = agent_core::log::LogEvent::ChecksDecided.as_str(),
                    outcome = "sent_back",
                    team = %team,
                    pr = %reviewing.pr,
                    pass = pass.get(),
                    issues = issues.len(),
                    "the checks send the round back to the team"
                );
                Ok(Some(Held::WakeLead))
            }
            ChecksStep::AskHuman { reason } => {
                let asking = Putting {
                    issues,
                    findings: settled.ask,
                    facts: &gathered.facts,
                };
                self.put_question(site, reviewing, &mut record, asking, (reason, checking.passes()), now)
                    .await
            }
        }
    }

    /// Takes the human's answer to the questions of the checks: what they allowed and what they refused is kept for the
    /// items. Returns the refused subjects.
    async fn take_answer(
        &mut self,
        site: Site<'_>,
        reviewing: &Reviewing<'_>,
        record: &mut ChecksRecord,
        asked: &Asked,
        file: &std::path::Path,
    ) -> Result<Vec<Subject>, PublishError> {
        let Site { pulls, team, lead } = site;
        let loaded = questions::read(file).await?;
        let decided = grouped_decisions(&asked.subjects, &loaded.file)?;
        record.answered(&asked.items, &decided.allowed, &decided.refused);
        self.parts.store.save_checks_record(&lead.task, record).await?;
        tracing::info!(
            event = agent_core::log::LogEvent::ChecksDecided.as_str(),
            outcome = "human_answered",
            team = %team,
            allowed = decided.allowed.len(),
            refused = decided.refused.len(),
            "the human answered the questions of the checks"
        );
        if !decided.allowed.is_empty() {
            let allowed = decided
                .allowed
                .iter()
                .map(|subject| named(pulls.checks(), subject))
                .collect::<Result<Vec<String>, _>>()?;
            self.record_checked(team, reviewing, allowed, CheckOutcome::Allowed);
        }
        Ok(decided.refused)
    }

    /// Writes the questions that name the `issues` for the lead to put to the human, and keeps what the checks wait on.
    async fn put_question(
        &mut self,
        site: Site<'_>,
        reviewing: &Reviewing<'_>,
        record: &mut ChecksRecord,
        asking: Putting<'_>,
        (reason, passes): (AskReason, u32),
        now: Timestamp,
    ) -> Result<Option<Held>, PublishError> {
        let Site { pulls, team, lead } = site;
        let Putting {
            mut issues,
            findings,
            facts,
        } = asking;
        let number = self
            .parts
            .state
            .find_task(&lead.task)
            .and_then(|found| next_number(found.member))
            .ok_or(PublishError::QuestionNumbers)?;
        // A question about the commits alone spends and restarts no pass of the team: they were never the team's to settle.
        let covers = if reason == AskReason::PassesUsed
            || (!issues.is_empty() && issues.iter().all(|issue| issue.judge() == Judge::Human))
        {
            Covers::Everything
        } else {
            Covers::HumanPart
        };
        issues.retain(|issue| covers == Covers::Everything || issue.judge() == Judge::Human);
        let commits = self.commit_question(reviewing, record, &findings, facts).await?;
        let asked = questions_with_commits(pulls.checks(), team, &lead.task, &issues, &commits, reason, passes)?;
        let file = self.parts.store.home().questions_file(&lead.task, number);
        replace_text_file(&file, &asked.file.render(), QUESTIONS_FILE_MODE).await?;
        record.sent = None;
        record.basis = Some(Basis {
            head: reviewing.ready.tip.clone(),
            covers,
        });
        record.asked = Some(Asked {
            number,
            at: now,
            items: reviewing.items.to_vec(),
            subjects: asked.subjects,
            relays: 0,
        });
        self.parts.store.save_checks_record(&lead.task, record).await?;
        let mut lines = issue_lines(pulls, &issues)?;
        for note in &commits.notes {
            lines.push(named(pulls.checks(), &note.finding.subject())?);
        }
        self.record_checked(
            team,
            reviewing,
            lines,
            CheckOutcome::AskedHuman {
                reason,
                question: number,
            },
        );
        tracing::info!(
            event = agent_core::log::LogEvent::ChecksDecided.as_str(),
            outcome = "asked_human",
            team = %team,
            pr = %reviewing.pr,
            ?reason,
            issues = issues.len(),
            commits = commits.notes.len(),
            "the checks ask the human about the round"
        );
        Ok(Some(Held::WakeLead))
    }

    /// What the human is shown about the `findings` about commits, and what was settled for the items before.
    async fn commit_question(
        &self,
        reviewing: &Reviewing<'_>,
        record: &ChecksRecord,
        findings: &[CommitFinding],
        facts: &RoundFacts,
    ) -> Result<CommitQuestion, PublishError> {
        if findings.is_empty() {
            return Ok(CommitQuestion::default());
        }
        let ids: Vec<_> = findings.iter().map(|finding| finding.commit.clone()).collect();
        let authors = reviewing.ready.git.commit_authors(&ids).await?;
        let notes = findings
            .iter()
            .map(|finding| commits::describe(finding, facts, &authors))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CommitQuestion {
            notes,
            before: record.allowed_of(reviewing.items),
        })
    }

    /// Hands the history the issues the checks found at the head under review, and what followed.
    fn record_checked(&self, team: &TeamId, reviewing: &Reviewing<'_>, issues: Vec<String>, outcome: CheckOutcome) {
        self.record_event(
            team,
            self.rounds_of_items(team, reviewing.items),
            HistoryEvent::Checked {
                pr: Some(reviewing.pr),
                head: reviewing.ready.tip.clone(),
                issues,
                outcome,
            },
        );
    }

    /// `description`, the description of the pull request of the branch `ready` of `lead`, with the section that lists
    /// the `TEST-FIX:` commits of the round as it is now: added, replaced, or taken out when the round has none.
    pub(super) async fn disclosed(
        &self,
        lead: &Lead,
        ready: &Ready,
        description: &str,
    ) -> Result<String, PublishError> {
        let base = self.base_branch(&lead.repo)?;
        let merge_base = ready
            .git
            .merge_base(&Revision::RemoteBranch(base), &Revision::Commit(ready.tip.clone()))
            .await?;
        let facts = ready.git.round_facts(&merge_base, &ready.tip, description).await?;
        Ok(with_section(description, test_fix_section(&facts.commits).as_deref()))
    }

    /// What git says about the round of `reviewing`, whose pull request somebody described as `description`.
    async fn gather(
        &self,
        lead: &Lead,
        reviewing: &Reviewing<'_>,
        description: &str,
    ) -> Result<Gathered, PublishError> {
        let git = &reviewing.ready.git;
        let tip = &reviewing.ready.tip;
        let base = self.base_branch(&lead.repo)?;
        git.fetch(&base).await?;
        let merge_base = git
            .merge_base(&Revision::RemoteBranch(base), &Revision::Commit(tip.clone()))
            .await?;
        let mut facts = git.round_facts(&merge_base, tip, description).await?;
        // The harness lists the `TEST-FIX:` commits in the description itself (see `disclosed`): the round is judged with
        // the description it is about to have, not the one an agent could not edit.
        facts.description = with_section(&facts.description, test_fix_section(&facts.commits).as_deref());
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
        Ok(Gathered { facts, unchanged })
    }

    /// Takes back the allowances for files that the round changed after the human allowed them: the checks ask about them
    /// again. An allowance with no head is one an earlier daemon recorded, and holds.
    async fn revoke_changed_allowances(
        &self,
        site: Site<'_>,
        reviewing: &Reviewing<'_>,
        record: &mut ChecksRecord,
    ) -> Result<(), PublishError> {
        let at_heads = record.allowed_at_of(reviewing.items);
        let tip = &reviewing.ready.tip;
        let mut stale: Vec<Subject> = Vec::new();
        let mut heads: Vec<&agent_core::state::CommitId> = Vec::new();
        for head in at_heads.values() {
            if head != tip && !heads.contains(&head) {
                heads.push(head);
            }
        }
        for head in heads {
            let changed = match reviewing.ready.git.range_changes(head, tip).await {
                Ok(files) => Some(files),
                Err(error) => {
                    // What changed since cannot be told (the head may be gone after a rebase): the allowances of that head
                    // are taken back and asked about again, which is the safe side.
                    tracing::warn!(
                        event = agent_core::log::LogEvent::ChecksDecided.as_str(),
                        outcome = "allowance_head_unreadable",
                        %head,
                        error = %agent_runtime::process::error_chain(&error),
                        "the checks cannot tell what changed since the human allowed"
                    );
                    None
                }
            };
            for (path, granted) in &at_heads {
                if granted != head {
                    continue;
                }
                let touched = changed.as_ref().is_none_or(|files| {
                    files.iter().any(|file| {
                        file.path == *path
                            || matches!(&file.change, agent_core::decide::checks::Change::Renamed { from } if from == path)
                    })
                });
                if touched {
                    stale.push(Subject::Path(path.clone()));
                }
            }
        }
        if stale.is_empty() {
            return Ok(());
        }
        record.revoke(reviewing.items, &stale);
        self.parts.store.save_checks_record(&site.lead.task, record).await?;
        Ok(())
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
                match self.notice_context_for(&lead.task, now, None, None) {
                    Ok(context) => match not_asked(pulls, team_id, &lead.task, &context) {
                        Ok(notification) => match self.locate(&lead.task) {
                            Ok(located) => self.notify(now, &located, &lead.task, notification),
                            Err(error) => {
                                tracing::error!(
                                    event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                                    task = %lead.task,
                                    %error,
                                    "a notification of the checks has no task to be about"
                                );
                            }
                        },
                        Err(error) => {
                            tracing::error!(
                                event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                                task = %lead.task,
                                error = %agent_runtime::process::error_chain(&error),
                                "a notification of the checks cannot be made"
                            );
                        }
                    },
                    Err(error) => {
                        tracing::error!(
                            event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                            task = %lead.task,
                            %error,
                            "a notification of the checks has no task context"
                        );
                    }
                }
                Ok(())
            }
        }
    }
}

/// What git says about a round.
struct Gathered {
    /// The files, the commits and the added lines of the round.
    facts: RoundFacts,
    /// The lines near the added ones that the round left as they were.
    unchanged: Vec<UnchangedLine>,
}

/// What the checks find in a round, told apart by whom they wait for.
struct Assessed {
    /// The issues the team and the human are still to settle, without what the human allowed.
    issues: Vec<Issue>,
    /// The findings about commits: what the human is still to be asked, and what an earlier allowance covers.
    settled: Settled,
}

/// What a question is put about.
struct Putting<'a> {
    /// The issues the question names.
    issues: Vec<Issue>,
    /// The findings about commits the question names.
    findings: Vec<CommitFinding>,
    /// The facts of the round, which describe the commits.
    facts: &'a RoundFacts,
}

/// What the checks find in the round `gathered`, without what the human allowed for `items` in `record` and what the task
/// list allows beforehand in `allowances`.
fn judge(
    gathered: &Gathered,
    rules: &agent_core::config::Checks,
    record: &ChecksRecord,
    items: &[ItemId],
    allowances: &[agent_core::task_allowance::TaskAllowance],
) -> Assessed {
    let found = commits::split(assess_near(rules, &gathered.facts, &gathered.unchanged, &[]));
    let human_only = record.human_only_of(items);
    let allowed_for_all: Vec<Subject> = record
        .allowed_of(items)
        .into_iter()
        .filter(|subject| !human_only.contains(subject))
        .collect();
    let issues = found
        .rest
        .into_iter()
        .filter(|issue| !allowed_for_all.contains(&issue.subject()))
        // An allowance given to a question about the human's part covers the issues the human alone may allow.
        .filter(|issue| !(issue.judge() == Judge::Human && human_only.contains(&issue.subject())))
        .filter(|issue| issue.allowed_beforehand(allowances).is_none())
        .collect();
    let allowed = record.allowed_of(items);
    let refused = record.refused_of(items);
    let heads = record.allowed_at_of(items);
    let paths: BTreeMap<RepoPath, Option<agent_core::state::CommitId>> = allowed_for_all
        .iter()
        .filter_map(|subject| match subject {
            Subject::Path(path) => Some((path.clone(), heads.get(path).cloned())),
            Subject::NoTests | Subject::Commit { .. } => None,
        })
        .collect();
    let settled = commits::settle(
        found.commits,
        &gathered.facts,
        &Said {
            allowed: &allowed,
            refused: &refused,
            paths: &paths,
            task_list: allowances,
        },
    );
    Assessed { issues, settled }
}

/// The `issues` in words, a line each, as the history keeps them.
fn issue_lines(pulls: &Pulls, issues: &[Issue]) -> Result<Vec<String>, PublishError> {
    Ok(issues
        .iter()
        .map(|issue| reason(pulls.checks(), issue))
        .collect::<Result<Vec<String>, _>>()?)
}

/// The notification that the lead `task` of `team` did not put the question of the checks to the human.
fn not_asked(
    pulls: &Pulls,
    team: &TeamId,
    task: &TaskId,
    context: &NoticeContext<'_>,
) -> Result<Notification, PublishError> {
    let values = Variables::new()
        .with("team", team.as_str())?
        .with("task", task.as_str())?;
    let rendered = pulls
        .checks()
        .render(agent_core::prompt::ChecksPrompt::NoticeNotAsked, &values)
        .map_err(crate::daemon::checks::ChecksTextError::from)?;
    Ok(layout_notification(NoticeKind::NotAsked, context, &rendered)?)
}
