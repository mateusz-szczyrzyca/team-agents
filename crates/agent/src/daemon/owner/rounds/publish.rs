//! Publishing: pushing the branch of the lead, opening or updating its pull request, and putting the items in review.

use agent_core::config::RepoName;
use agent_core::decide::fix::{Answers, Budget, Progress, Step, fix_step};
use agent_core::decide::publish::{GateHold, GateStep, HarnessGate, Waiting, gate_step};
use agent_core::decide::round::{RoundEvent, RoundPolicy, RoundState, Stage};
use agent_core::decide::wait::Obligation;
use agent_core::history::HistoryEvent;
use std::num::NonZeroU32;

use agent_core::ids::{BranchName, ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::{EventKind, Failure, Reply};
use agent_core::state::{CommitId, GateRecord, ItemPhase, PullRequest, Timestamp};
use agent_runtime::git::{Git, Revision};
use agent_runtime::github::{NewPullRequest, PullRequestUpdate, Readiness, RepoSlug};
use agent_runtime::process::error_chain;
use tokio::sync::oneshot;

use crate::daemon::team::Why;

use super::super::teams::Look;
use crate::daemon::pr::{Joining, Publication, Publish, plan_publications, pull_text, returned_delivery};

use super::super::{Owner, OwnerError};
use crate::daemon::merge::count;

use super::checks::{Held, Reviewing};
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};

/// Refreshes only links attributable to the origin used by this publication.
async fn refresh_harness_files(git: &Git, description: &str, harness: &str) -> String {
    let repository = async {
        let origin = git.origin_url().await?;
        Ok::<_, PublishError>(RepoSlug::from_remote_url(&origin)?)
    }
    .await;
    match repository {
        Ok(slug) => {
            let url = format!("https://github.com/{}/{}", slug.owner(), slug.name());
            crate::daemon::checks::unlinked_harness_files_in_repository(description, harness, &url)
        }
        Err(error) => {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                error = %error_chain(&error),
                "cannot determine origin for refreshing file links: text is sent unchanged"
            );
            description.to_owned()
        }
    }
}

/// How many times in a row a lead is sent back because the gate has not passed at the tip of its branch; after that the
/// human is told.
pub(super) const GATE_SEND_BACKS: u32 = 3;

/// A gate the harness runs by itself, for a lead that was done without one at the tip of its branch.
#[derive(Debug)]
pub(super) enum OwnGate {
    /// It waits or runs.
    Running {
        /// The tip it was started for.
        tip: CommitId,
        /// Whether it is run once more after a gate had not passed at that tip.
        retry: bool,
        /// Where the gate service answers when it is over.
        answer: oneshot::Receiver<Result<Reply, Failure>>,
    },
    /// It ran: the record says what it found.
    Ran {
        /// The tip it was started for.
        tip: CommitId,
        /// Whether it was run once more after a gate had not passed at that tip.
        retry: bool,
    },
}

/// Where the harness's own gate for the tip of a lead's branch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Own {
    /// None ran for this tip, or the one that was started could not run.
    NotRun,
    /// One waits or runs; it may be for an earlier tip.
    Running,
    /// One ran for this tip.
    Ran {
        /// Whether it was run once more after a gate had not passed at the tip.
        retry: bool,
    },
}

/// How long a branch that could not be published is left alone before the next attempt.
pub(super) const PUBLISH_BACKOFF: time::Duration = time::Duration::seconds(60);

/// Where a publication is made: the autopilot, the team and its lead.
#[derive(Clone, Copy)]
pub(super) struct Site<'a> {
    /// The autopilot.
    pub(super) pulls: &'a Pulls,
    /// The team.
    pub(super) team: &'a TeamId,
    /// The lead of the team.
    pub(super) lead: &'a Lead,
}

/// A branch whose gate passed at its tip.
pub(super) struct Ready {
    /// Git in the lead's worktree.
    pub(super) git: Git,
    /// The tip of the branch.
    pub(super) tip: CommitId,
    /// What the gate recorded.
    pub(super) record: GateRecord,
}

/// What an attempt to publish a branch came to.
pub(super) enum Attempt {
    /// There is nothing to publish yet.
    Nothing,
    /// The pull request was opened or updated.
    Published(Published),
    /// The checks before the review hold the round back; the owner does what they ask.
    Held(Held),
    /// The lead is done and the gate has not passed at `tip`: it is sent back.
    SentBack {
        /// The tip of its branch.
        tip: CommitId,
        /// Why the gate does not count.
        hold: GateHold,
    },
    /// A fix pass claimed a fix but handed back the reviewed commit.
    Uncommitted {
        /// The unchanged reviewed commit.
        tip: CommitId,
        /// Catalog-rendered reason for the human notice.
        reason: String,
    },
    /// The lead is done and the gate passed, but no finished branch was joined.
    NotJoined {
        /// The tip of its branch.
        tip: CommitId,
        /// The finished items, none of whose branches the tip contains.
        missing: Vec<Joining>,
    },
}

/// What a lead that is done is sent back to its branch with.
struct Back {
    /// The text it is given.
    prompt: AutopilotPrompt,
    /// The variable of that text that says what is wrong, beside the branch and the count: its name and its value.
    variable: (&'static str, String),
    /// The outcome the log names.
    outcome: &'static str,
    /// What is wrong, as the human is told it when the lead was sent back in vain: a sentence.
    told: String,
    /// What the lead ended without, each time, for the human: the end of "ended again without".
    undone: &'static str,
}

/// A pull request that was opened or updated.
pub(super) struct Published {
    /// The pull request.
    pub(super) pr: PullRequest,
    /// The commit the gate passed on, which was pushed.
    pub(super) gated: CommitId,
    /// The commit GitHub says the pull request carries.
    pub(super) carried: CommitId,
    /// The items that go in it, in the order of the list.
    pub(super) items: Vec<ItemId>,
}

/// Makes `origin` carry `ready.tip` on `branch`: pushes it, unless `origin` has it there already (`carried`, what
/// GitHub says the branch is at), which a push over a rewritten history would be refused for.
async fn carry(ready: &Ready, branch: &BranchName, carried: Option<&CommitId>) -> Result<(), PublishError> {
    if carried == Some(&ready.tip) {
        ready.git.adopt_pushed(branch, &ready.tip).await?;
    } else {
        ready.git.push(branch).await?;
    }
    Ok(())
}

/// The numbers of the items `publication` is for.
fn items_of(publication: &Publication) -> Vec<String> {
    match &publication.publish {
        Publish::Open { items } => items.iter().map(|joining| joining.item.to_string()).collect(),
        Publish::Update { fixing, joining, .. } => fixing
            .iter()
            .copied()
            .chain(joining.iter().map(|joining| joining.item))
            .map(|id| id.to_string())
            .collect(),
    }
}

/// The items of `items` whose branch is part of the history of `tip`.
pub(super) async fn contained(git: &Git, items: &[Joining], tip: &CommitId) -> Result<Vec<ItemId>, PublishError> {
    let mut found = Vec::new();
    for joining in items {
        let part = git
            .is_ancestor(
                &Revision::Branch(joining.branch.clone()),
                &Revision::Commit(tip.clone()),
            )
            .await?;
        if part {
            found.push(joining.item);
        }
    }
    Ok(found)
}

impl Owner {
    /// Publishes the branches of `team_id` that are ready: its pull requests are opened or updated, and its items go in
    /// review.
    pub(super) async fn publish_branches(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let plan = plan_publications(team).map_err(|source| OwnerError::Publications {
            team: team_id.clone(),
            source,
        })?;
        if plan.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for publication in plan {
            if let Publish::Update { fixing, .. } = &publication.publish
                && self.held_for_response(team_id, fixing)
            {
                continue;
            }

            let now = Timestamp::now(&self.parts.clock)?;
            let key = (team_id.clone(), publication.head_branch.clone());
            if self.watch.retry_at.get(&key).is_some_and(|retry| *retry > now) {
                continue;
            }
            let site = Site {
                pulls,
                team: team_id,
                lead: &lead,
            };
            let items = items_of(&publication);
            let rounds = self.publication_rounds(team_id, &items);
            for round in &rounds {
                self.watch.waits.remove(round);
            }
            let attempt = self.try_publish(site, &publication, now).await;
            self.remember_publication_wait(&rounds, &attempt);
            match attempt {
                Ok(Attempt::Nothing) => {}
                Ok(Attempt::Published(published)) => {
                    self.watch.retry_at.remove(&key);
                    self.watch.gate_sent_back.remove(&lead.task);
                    self.watch.own_gates.remove(&lead.task);
                    if self.record_publication(team_id, &publication, &published, look)? {
                        self.note_reviewed(&lead, &published.pr.branch, &published.gated).await;
                    }
                }
                Ok(Attempt::Held(held)) => {
                    self.watch.retry_at.remove(&key);
                    self.hold_round(pulls, team_id, &lead, held, look)?;
                }
                Ok(Attempt::SentBack { tip, hold }) => {
                    self.send_back_for_gate(site, &publication, (&tip, &hold), now, look)?;
                }
                Ok(Attempt::Uncommitted { tip, reason }) => {
                    let back = Back {
                        prompt: AutopilotPrompt::ReviewFixNoCommit,
                        variable: ("tip", tip.to_string()),
                        outcome: "fix_without_commit",
                        told: reason,
                        undone: include_str!("../../../../../../prompts/harness/review/fix-no-commit-undone.md"),
                    };
                    self.send_lead_back(site, &publication, &tip, &back, now, look)?;
                }
                Ok(Attempt::NotJoined { tip, missing }) => {
                    self.send_back_to_join(site, &publication, (&tip, &missing), now, look)?;
                }
                Err(error) => self.publish_failed(pulls, team_id, &lead, &publication, &error, now)?,
            }
        }
        Ok(())
    }

    /// Finds the rounds whose items this publication carries.
    fn publication_rounds(&self, team: &TeamId, items: &[String]) -> Vec<agent_core::ids::RoundId> {
        self.parts
            .state
            .team(team)
            .into_iter()
            .flat_map(agent_core::state::Team::rounds)
            .filter(|round| round.item().is_some_and(|id| items.contains(&id.to_string())))
            .map(|round| round.id().clone())
            .collect()
    }

    /// Retains the pending operation when publication leaves an idle round open.
    fn remember_publication_wait(
        &mut self,
        rounds: &[agent_core::ids::RoundId],
        attempt: &Result<Attempt, PublishError>,
    ) {
        let obligation = match attempt {
            Ok(Attempt::Nothing | Attempt::Published(_)) => None,
            Ok(Attempt::Held(super::checks::Held::Waiting | super::checks::Held::Notify)) => Some(Obligation::Human),
            Ok(
                Attempt::Held(super::checks::Held::WakeLead)
                | Attempt::SentBack { .. }
                | Attempt::Uncommitted { .. }
                | Attempt::NotJoined { .. },
            ) => Some(Obligation::LeadCorrection),
            Err(_) => Some(Obligation::Review),
        };
        if let Some(obligation) = obligation {
            for round in rounds {
                self.watch.waits.insert(round.clone(), obligation);
            }
        }
    }

    /// Pushes the branch of `publication` and opens or updates its pull request when the lead's gate passed at its tip and
    /// the checks before the review let the round go to it.
    pub(super) async fn try_publish(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let lead = site.lead;
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let independent = items_of(publication).iter().any(|id| {
            self.parts.state.team(site.team).is_some_and(|team| {
                team.items()
                    .iter()
                    .find(|item| item.id().to_string() == *id)
                    .is_some_and(|item| {
                        returned_delivery(team, item.id())
                            || team.round_of(item.id()).is_some_and(|round| round.part().is_some())
                    })
            })
        });
        let source = if independent {
            crate::daemon::team::task_branch(&lead.agent, &lead.task)?
        } else {
            publication.head_branch.clone()
        };
        let Some(tip) = git.branch_tip(&source).await? else {
            return Ok(Attempt::Nothing);
        };
        // A gate of the harness that still runs decides nothing yet; the record is read after it, so that it is not older.
        let own = self.own_gate_state(&lead.task, &tip);
        if own == Own::Running {
            return Ok(Attempt::Nothing);
        }
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        let base = Revision::Branch(self.base_branch(&lead.repo)?);
        let waiting = Waiting {
            tip: &tip,
            lead_done: lead.done,
            // A tip the base branch has is the base branch: the lead committed nothing.
            own_work: !git.is_ancestor(&Revision::Commit(tip.clone()), &base).await?,
            record: record.as_ref(),
            harness_gate: match own {
                Own::NotRun | Own::Running => HarnessGate::NotRun,
                Own::Ran { retry: false } => HarnessGate::Ran,
                Own::Ran { retry: true } => HarnessGate::Retried,
            },
        };
        let record = match (gate_step(&waiting), record) {
            (GateStep::Publish, Some(record)) => record,
            (GateStep::Publish | GateStep::Wait, _) => return Ok(Attempt::Nothing),
            (GateStep::Run, _) => {
                self.start_own_gate(&lead.task, &tip, false);
                return Ok(Attempt::Nothing);
            }
            (GateStep::Retry, _) => {
                self.start_own_gate(&lead.task, &tip, true);
                return Ok(Attempt::Nothing);
            }
            (GateStep::SendBack(hold), _) => return Ok(Attempt::SentBack { tip, hold }),
        };
        let ready = Ready { git, tip, record };
        match &publication.publish {
            Publish::Open { items } => {
                self.open_pull_request(site, &publication.head_branch, items, &ready, now)
                    .await
            }
            Publish::Update { pr, fixing, joining } => {
                self.update_pull_request(site, (pr, fixing), joining, &ready, now).await
            }
        }
    }

    /// Opens the pull request of the finished `items` whose branches `ready.tip` contains: pushes the branch, finds the
    /// pull request an earlier attempt opened or opens a draft, and marks it ready.
    async fn open_pull_request(
        &mut self,
        site: Site<'_>,
        head_branch: &BranchName,
        items: &[Joining],
        ready: &Ready,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let Site { pulls, team, lead } = site;
        let ids = contained(&ready.git, items, &ready.tip).await?;
        if ids.is_empty() {
            // A lead that is done handed its branch over: with none of the finished items in it nothing would ever be
            // published, and nobody would be told. A lead that is not done is still at it.
            return Ok(if lead.done && !items.is_empty() {
                Attempt::NotJoined {
                    tip: ready.tip.clone(),
                    missing: items.to_vec(),
                }
            } else {
                Attempt::Nothing
            });
        }
        let published_branch = self.publication_branch(team, &ids, head_branch, ready).await?;
        let head_branch = published_branch.as_ref();
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        let existing = github.find_open_pull_request(head_branch).await?;
        carry(ready, head_branch, existing.as_ref().map(|found| &found.head)).await?;
        let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &ready.record).await?;
        let base = self.base_branch(&lead.repo)?;
        let remote = if let Some(mut found) = existing {
            if found.head != ready.tip {
                found.head = ready.tip.clone();
                self.record_event(
                    team,
                    self.rounds_of_items(team, &ids),
                    HistoryEvent::PullPushed {
                        pr: found.number,
                        head: ready.tip.clone(),
                    },
                );
            }
            found
        } else {
            let body = self.link_commits(&ready.git, &ready.tip, &text.body).await;
            let created = github
                .create_pull_request(&NewPullRequest {
                    title: text.title,
                    body,
                    head: head_branch.clone(),
                    base,
                    readiness: Readiness::Draft,
                })
                .await?;
            // The opening is what GitHub made of it: its number, its head, its title and its body whole.
            self.record_event(
                team,
                self.rounds_of_items(team, &ids),
                HistoryEvent::PullOpened {
                    number: created.number,
                    branch: head_branch.clone(),
                    head: created.head.clone(),
                    title: created.title.clone(),
                    body: created.body.clone(),
                },
            );
            created
        };
        // The round goes to the reviewers only when the checks let it: until then the pull request stays a draft.
        let reviewing = Reviewing {
            items: &ids,
            ready,
            pr: remote.number,
            description: &remote.body,
        };
        if let Some(held) = self.check_before_review(site, &reviewing, now).await? {
            return Ok(Attempt::Held(held));
        }
        // The reviewers find the `TEST-FIX:` commits of the round listed in the description: the harness writes that.
        let body = self.disclosed(lead, ready, &text.body).await?;
        let body = if body == remote.body {
            body
        } else {
            refresh_harness_files(&ready.git, &body, &text.body).await
        };
        let body = self.link_commits(&ready.git, &ready.tip, &body).await;
        if body != remote.body {
            let update = PullRequestUpdate {
                body: Some(body),
                ..PullRequestUpdate::default()
            };
            github.update_pull_request(remote.number, &update).await?;
        }
        github.mark_ready(remote.number).await?;
        self.record_event(
            team,
            self.rounds_of_items(team, &ids),
            HistoryEvent::PullReady { pr: remote.number },
        );
        Ok(Attempt::Published(Published {
            pr: PullRequest {
                number: remote.number,
                branch: head_branch.clone(),
                opened_at: now,
            },
            gated: ready.tip.clone(),
            carried: remote.head,
            items: ids,
        }))
    }

    /// Keeps split parts and returned deliveries on independent branches at the integrated, gated tip.
    async fn publication_branch<'a>(
        &self,
        team: &TeamId,
        ids: &[ItemId],
        head: &'a BranchName,
        ready: &Ready,
    ) -> Result<std::borrow::Cow<'a, BranchName>, PublishError> {
        let independent = ids.iter().any(|id| {
            self.parts.state.team(team).is_some_and(|team| {
                returned_delivery(team, *id) || team.round_of(*id).is_some_and(|round| round.part().is_some())
            })
        });
        if !independent {
            return Ok(std::borrow::Cow::Borrowed(head));
        }
        ready.git.branch_point(head, &ready.tip).await?;
        Ok(std::borrow::Cow::Borrowed(head))
    }

    /// Whether the update of a pull request whose items `fixing` are fixed waits: the lead's fix response to the pull
    /// request's fix pass is not taken yet, or a second opinion of that pass is still out or confirmed a blocker.
    fn held_for_response(&self, team_id: &TeamId, fixing: &[ItemId]) -> bool {
        let Some(team) = self.parts.state.team(team_id) else {
            return false;
        };
        fixing.iter().filter_map(|id| team.item(*id)).any(|item| {
            let passes = crate::daemon::merge::pull_passes(team, item);
            item.findings().is_some_and(|ledger| {
                ledger.disputes().is_some_and(|d| d.answered_pass != passes)
                    || std::num::NonZeroU32::new(passes).is_some_and(|pass| {
                        matches!(
                            agent_core::decide::opinion::after_opinions(
                                ledger,
                                pass,
                                &ledger.fixed_answers(pass),
                                agent_core::decide::opinion::Tip::Changed
                            ),
                            agent_core::decide::opinion::Next::AwaitOpinions
                                | agent_core::decide::opinion::Next::FixConfirmed(_)
                        )
                    })
            })
        })
    }

    /// Checks whether every blocking finding was answered without claiming a committed fix.
    fn unchanged_answers(&self, team: &TeamId, fixing: &[ItemId]) -> Answers {
        let accounted = fixing.iter().all(|id| {
            self.parts
                .state
                .team(team)
                .and_then(|t| Some((t, t.item(*id)?)))
                .is_some_and(|(team, item)| {
                    item.findings().is_some_and(|ledger| {
                        NonZeroU32::new(crate::daemon::merge::pull_passes(team, item))
                            .is_none_or(|pass| ledger.fixed_answers(pass).is_empty())
                            && ledger
                                .entries()
                                .iter()
                                .filter(|entry| {
                                    entry.status == agent_core::state::FindingStatus::Open
                                        && entry.finding.severity.blocks()
                                })
                                .all(|entry| {
                                    ledger.disputes().is_some_and(|disputes| {
                                        disputes.explanations.get(&entry.number).is_some_and(|explanation| {
                                            explanation.status == agent_core::state::ExplanationStatus::Pending
                                        })
                                    })
                                })
                    })
                })
        });
        if accounted {
            Answers::Accounted
        } else {
            Answers::Missing
        }
    }

    /// Updates the pull request `pr` whose items `fixing` were fixed: pushes the branch, tells the reviewers in a comment,
    /// and takes the finished `joining` items whose branches `ready.tip` contains in with it.
    async fn update_pull_request(
        &mut self,
        site: Site<'_>,
        (pr, fixing): (&PullRequest, &[ItemId]),
        joining: &[Joining],
        ready: &Ready,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let Site { pulls, team, lead } = site;
        let joined = contained(&ready.git, joining, &ready.tip).await?;
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        let before = github.get_pull_request(pr.number).await?;
        // What the reviewers saw is what the harness put in review, not what GitHub carries: a fix may be on `origin`
        // before the harness looks (the agent pushed it, or GitHub showed the harness's own push late). A branch that
        // went in review before the harness kept this is judged by what GitHub carries.
        let reviewed = ready
            .git
            .reviewed_tip(&pr.branch)
            .await?
            .unwrap_or_else(|| before.head.clone());
        if reviewed == ready.tip {
            if !lead.done {
                return Ok(Attempt::Nothing);
            }
            let answers = self.unchanged_answers(team, fixing);
            let budget = if self.watch.gate_sent_back.get(&lead.task).copied().unwrap_or(0) < GATE_SEND_BACKS {
                Budget::Available
            } else {
                Budget::Exhausted
            };
            match fix_step(Progress::Unchanged(answers), budget) {
                Step::Correct(_) | Step::AskHuman(_) => {
                    return Ok(Attempt::Uncommitted {
                        tip: ready.tip.clone(),
                        reason: pulls.texts.render(
                            AutopilotPrompt::ReviewFixNoCommitReason,
                            &Variables::new().with("tip", ready.tip.as_str())?,
                        )?,
                    });
                }
                Step::Verify => {}
                Step::WaitGate => return Ok(Attempt::Nothing),
            }
        }
        let mut ids = fixing.to_vec();
        ids.extend(joined.iter().copied());
        let ids = self.in_list_order(team, &ids);
        // A head the checks hold back is not pushed: the reviewers keep the head they reviewed.
        let reviewing = Reviewing {
            items: &ids,
            ready,
            pr: pr.number,
            description: &before.body,
        };
        if let Some(held) = self.check_before_review(site, &reviewing, now).await? {
            return Ok(Attempt::Held(held));
        }
        self.publication_branch(team, &ids, &pr.branch, ready).await?;
        carry(ready, &pr.branch, Some(&before.head)).await?;
        self.record_event(
            team,
            self.rounds_of_items(team, &ids),
            HistoryEvent::PullPushed {
                pr: pr.number,
                head: ready.tip.clone(),
            },
        );
        // The description follows the round: the items that joined, and the `TEST-FIX:` commits the round has now.
        let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &ready.record).await?;
        let mut update = PullRequestUpdate::default();
        if text.title != before.title {
            update.title = Some(text.title);
        }
        let body = self.disclosed(lead, ready, &text.body).await?;
        let body = refresh_harness_files(&ready.git, &body, &text.body).await;
        let body = self.link_commits(&ready.git, &ready.tip, &body).await;
        if body != before.body {
            update.body = Some(body);
        }
        if update != PullRequestUpdate::default() {
            github.update_pull_request(pr.number, &update).await?;
        }
        let comment = pulls.texts.render(
            AutopilotPrompt::PrUpdate,
            &Variables::new()
                .with("head", ready.tip.as_str())?
                .with("pass", self.fix_passes_of(team, fixing).to_string())?
                .with("gate", ready.record.summary())?,
        )?;
        let comment = self.link_commits(&ready.git, &ready.tip, &comment).await;
        github.post_comment(pr.number, comment.trim_end()).await?;
        let after = github.get_pull_request(pr.number).await?;
        Ok(Attempt::Published(Published {
            pr: pr.clone(),
            gated: ready.tip.clone(),
            carried: after.head,
            items: ids,
        }))
    }

    /// Refreshes descriptions independently of publication, including file-only changes during review.
    pub(super) async fn refresh_descriptions(&mut self, pulls: &Pulls, team: &TeamId) -> Result<(), PublishError> {
        let found = self.parts.state.team(team).ok_or(PublishError::Lost)?;
        let mut open = std::collections::BTreeMap::new();
        for item in found.items() {
            let pr = match item.phase() {
                ItemPhase::InReview { pr, .. } | ItemPhase::Approved { pr, .. } | ItemPhase::Fixing { pr, .. } => pr,
                ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => continue,
            };
            open.entry(pr.number).or_insert_with(Vec::new).push(item.id());
        }
        if open.is_empty() {
            return Ok(());
        }
        let (_, member) = found.crew().roster().into_iter().next().ok_or(PublishError::Lost)?;
        let lead = Lead {
            repo: found.repo().clone(),
            task: member.task().clone(),
            agent: member.agent().name().parse()?,
            done: false,
        };
        let Some(record) = self.parts.store.load_gate_record(&lead.task).await? else {
            return Ok(());
        };
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        for (number, ids) in open {
            let remote = github.get_pull_request(number).await?;
            if remote.state != agent_runtime::github::PullState::Open || remote.head != record.result.head {
                continue;
            }
            let ready = Ready {
                git: git.clone(),
                tip: remote.head.clone(),
                record: record.clone(),
            };
            let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &record).await?;
            let body = self.disclosed(&lead, &ready, &text.body).await?;
            let body = refresh_harness_files(&git, &body, &text.body).await;
            let body = self.link_commits(&git, &remote.head, &body).await;
            if body != remote.body {
                github
                    .update_pull_request(
                        number,
                        &PullRequestUpdate {
                            body: Some(body),
                            ..PullRequestUpdate::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Renders only the harness header, so authored file disclosures remain distinguishable.
    pub(super) fn harness_text_of(
        &self,
        pulls: &Pulls,
        team_id: &TeamId,
        repo: &RepoName,
        ids: &[ItemId],
        record: &GateRecord,
    ) -> Result<crate::daemon::pr::PullText, PublishError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })?;
        Ok(pull_text(&pulls.texts, team, repo, ids, record)?)
    }

    /// The title and the description of the pull request of `ids`.
    pub(super) async fn pull_text_of(
        &self,
        pulls: &Pulls,
        team_id: &TeamId,
        repo: &RepoName,
        ids: &[ItemId],
        record: &GateRecord,
    ) -> Result<crate::daemon::pr::PullText, PublishError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })?;
        let (_, lead) = team.crew().roster().into_iter().next().ok_or(PublishError::Lost)?;
        let git = self.worktree_git(repo, lead.task(), &lead.agent().name().parse()?)?;
        let path = self
            .parts
            .store
            .home()
            .task_dir(lead.task())
            .join(crate::daemon::pr::description::LEAD_DESCRIPTION_FILE);
        let contribution =
            agent_runtime::text_file::read_text_file(&path, crate::daemon::pr::description::MAX_DESCRIPTION_BYTES)
                .await
                .map_err(|source| PublishError::Description { path, source })?;
        let mut contribution =
            crate::daemon::pr::description::lead_sections(contribution.as_ref().map_or("", |file| file.text.as_str()));
        if contribution.is_empty() {
            contribution = pulls.texts.render(AutopilotPrompt::PrNoLead, &Variables::new())?;
        }
        let base = Revision::RemoteBranch(self.base_branch(repo)?);
        let size = git.pull_request_size(&base, &record.result.head).await?;
        let previous = self.parts.store.previous_gate_record(lead.task()).await?;
        let measured =
            crate::daemon::pr::description::measured_sections(&pulls.texts, size, record, previous.as_ref())?;
        let mut text = self.harness_text_of(pulls, team_id, repo, ids, record)?;
        let allowed = if team.allowances().is_empty() {
            None
        } else {
            let merge_base = git
                .merge_base(&base, &Revision::Commit(record.result.head.clone()))
                .await?;
            let files = git.range_changes(&merge_base, &record.result.head).await?;
            crate::daemon::pr::description::allowed_section(&pulls.texts, &files, team.allowances())?
        };
        text.body = [text.body.trim_end(), contribution.trim(), &measured].join("\n\n");
        if let Some(section) = allowed {
            text.body.push_str("\n\n");
            text.body.push_str(section.trim_end());
        }
        Ok(text)
    }

    /// The branch the pull requests of `repo` are made against.
    pub(super) fn base_branch(&self, repo: &RepoName) -> Result<BranchName, PublishError> {
        self.parts
            .settings
            .config
            .repo(repo)
            .map(|configured| configured.base_branch.clone())
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })
    }

    /// `ids` in the order of the task list of `team`.
    pub(super) fn in_list_order(&self, team: &TeamId, ids: &[ItemId]) -> Vec<ItemId> {
        self.parts.state.team(team).map_or_else(Vec::new, |found| {
            found
                .items()
                .iter()
                .map(agent_core::state::ItemProgress::id)
                .filter(|id| ids.contains(id))
                .collect()
        })
    }

    /// How many fix passes the first of `items` has had.
    pub(super) fn fix_passes_of(&self, team: &TeamId, items: &[ItemId]) -> usize {
        items
            .first()
            .and_then(|first| self.parts.state.team(team).and_then(|found| found.item(*first)))
            .map_or(0, |progress| progress.fix_passes().len())
    }

    /// The items of `published` for which the round machine agrees that the pull request carries the commit the gate passed
    /// on; the others are looked at again.
    fn confirmed(&self, team_id: &TeamId, publication: &Publication, published: &Published) -> Vec<ItemId> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Vec::new();
        };
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        let mut confirmed = Vec::new();
        for id in &published.items {
            let stage = match (&publication.publish, team.item(*id)) {
                (Publish::Update { fixing, .. }, Some(progress)) if fixing.contains(id) => Stage::Fixing {
                    pr: published.pr.number,
                    pass: NonZeroU32::MIN.saturating_add(count(progress.fix_passes().len()).saturating_sub(1)),
                },
                _ => Stage::First,
            };
            let publishing = RoundState::Publishing {
                stage,
                head: published.gated.clone(),
            };
            let event = RoundEvent::PullRequestPublished {
                pr: published.pr.number,
                head: published.carried.clone(),
            };
            match publishing.transition(&event, policy) {
                Ok(_) => confirmed.push(*id),
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::GithubCallFailed.as_str(),
                        team = %team_id,
                        item = %id,
                        error = %error_chain(&error),
                        "a pull request does not carry what the gate passed: it is looked at again"
                    );
                }
            }
        }
        confirmed
    }

    /// Records that the items of `published` are in review, after the round machine checked that the pull request carries
    /// the commit the gate passed on. Returns whether any item went in review.
    pub(super) fn record_publication(
        &mut self,
        team_id: &TeamId,
        publication: &Publication,
        published: &Published,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        let confirmed = self.confirmed(team_id, publication, published);
        // A joining item shares the PR ledger, including the legacy protocol and existing comment ID.
        let findings = match &publication.publish {
            Publish::Open { .. } => Some(agent_core::state::Findings::verifying(
                self.parts.settings.config.review().min_first_full_reviews(),
            )),
            Publish::Update { fixing, .. } => fixing
                .first()
                .and_then(|id| self.parts.state.team(team_id)?.item(*id)?.findings())
                .cloned(),
        };
        let in_review = !confirmed.is_empty();
        for id in confirmed {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            item.advance(ItemPhase::InReview {
                pr: published.pr.clone(),
                reviews: Vec::new(),
            })
            .map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: id,
                source,
            })?;
            // An item that was being fixed has its pull request already; the others have it opened for them now.
            let opened = !matches!(&publication.publish, Publish::Update { fixing, .. } if fixing.contains(&id));
            if opened && let Some(findings) = &findings {
                item.start_findings();
                if let Some(ledger) = item.findings_mut() {
                    ledger.clone_from(findings);
                }
            }
            let round = self.round_of_item(team_id, id)?;
            if opened {
                look.events.push(EventKind::PullRequestOpened {
                    team: team_id.clone(),
                    item: id,
                    round: round.clone(),
                    pr: published.pr.number,
                });
            }
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                round: round.clone(),
                status: agent_core::status::ItemStatus::InReview,
            });
            look.changed = true;
            let span = tracing::error_span!("round", team = %team_id, rounds = %round);
            let _entered = span.enter();
            tracing::info!(
                event = LogEvent::ItemPhaseChanged.as_str(),
                team = %team_id,
                item = %id,
                phase = "in_review",
                pr = %published.pr.number,
                "an item is in review"
            );
        }
        Ok(in_review)
    }

    /// Keeps `tip` as what the reviewers of `branch` were asked to look at. A tip that cannot be kept costs at most a fix
    /// that waits for the next commit of the lead, so the round goes on; git said why in the log.
    async fn note_reviewed(&self, lead: &Lead, branch: &BranchName, tip: &CommitId) {
        let kept = match self.worktree_git(&lead.repo, &lead.task, &lead.agent) {
            Ok(git) => git.record_reviewed(branch, tip).await.map_err(PublishError::from),
            Err(error) => Err(error),
        };
        if let Err(error) = kept {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                branch = %branch,
                reason = %error_chain(&error),
                "what the reviewers were asked to look at cannot be kept"
            );
        }
    }

    /// Where the gate the harness started by itself for `task` is, for the branch tip `tip`. A gate that is over is taken
    /// off the service: one that ran is remembered with its tip, one that could not run is forgotten and started again.
    pub(super) fn own_gate_state(&mut self, task: &TaskId, tip: &CommitId) -> Own {
        let over = match self.watch.own_gates.get_mut(task) {
            Some(OwnGate::Running {
                tip: started,
                retry,
                answer,
            }) => match answer.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => return Own::Running,
                Ok(Ok(_)) => Some(Ok((started.clone(), *retry))),
                Ok(Err(failure)) => Some(Err(failure.message)),
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(Err("the gate service ended before it answered".to_owned()))
                }
            },
            Some(OwnGate::Ran { .. }) | None => None,
        };
        match over {
            Some(Ok((started, retry))) => {
                self.watch
                    .own_gates
                    .insert(task.clone(), OwnGate::Ran { tip: started, retry });
            }
            Some(Err(reason)) => {
                tracing::warn!(
                    event = LogEvent::GateCannotStart.as_str(),
                    task = %task,
                    %reason,
                    "the gate the harness runs for a lead that is done did not run: it is started again"
                );
                self.watch.own_gates.remove(task);
            }
            None => {}
        }
        match self.watch.own_gates.get(task) {
            Some(OwnGate::Ran { tip: ran, retry }) if ran == tip => Own::Ran { retry: *retry },
            Some(OwnGate::Ran { .. } | OwnGate::Running { .. }) | None => Own::NotRun,
        }
    }

    /// Starts the gate of `task` for the branch tip `tip`: nobody ran one, and the lead says it is done; or, with `retry`,
    /// one did not pass there and is run once more.
    pub(super) fn start_own_gate(&mut self, task: &TaskId, tip: &CommitId, retry: bool) {
        match self.own_gate(task) {
            Ok(answer) => {
                tracing::info!(
                    event = LogEvent::ChecksDecided.as_str(),
                    outcome = if retry { "gate_retried" } else { "gate_started" },
                    task = %task,
                    tip = %tip,
                    "the lead is done and no gate passed at the tip of its branch: the harness runs the gate"
                );
                self.watch.own_gates.insert(
                    task.clone(),
                    OwnGate::Running {
                        tip: tip.clone(),
                        retry,
                        answer,
                    },
                );
            }
            Err(failure) => {
                tracing::warn!(
                    event = LogEvent::GateCannotStart.as_str(),
                    task = %task,
                    reason = %failure.message,
                    "the gate the harness runs for a lead that is done cannot be started: it is tried again"
                );
            }
        }
    }

    /// The lead is done and the gate has not passed at the tip of its branch: it is sent back with the reason, and after
    /// [`GATE_SEND_BACKS`] times the human is told.
    fn send_back_for_gate(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        (tip, hold): (&CommitId, &GateHold),
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let back = Back {
            prompt: AutopilotPrompt::GateNotPassed,
            variable: ("reason", hold.to_string()),
            outcome: "gate_sent_back",
            told: format!(
                "the gate has not passed at the tip of the branch {}: {hold}",
                publication.head_branch
            ),
            undone: "a gate that passes",
        };
        self.send_lead_back(site, publication, tip, &back, now, look)
    }

    /// The lead is done, the gate passed, and its branch contains the branch of none of the finished items in `missing`:
    /// it is sent back to merge them, and after [`GATE_SEND_BACKS`] times the human is told.
    fn send_back_to_join(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        (tip, missing): (&CommitId, &[Joining]),
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lines: Vec<String> = missing
            .iter()
            .map(|joining| format!("- item {}: `{}`", joining.item, joining.branch))
            .collect();
        let named: Vec<String> = missing
            .iter()
            .map(|joining| format!("item {}: {}", joining.item, joining.branch))
            .collect();
        let back = Back {
            prompt: AutopilotPrompt::BranchNotJoined,
            variable: ("missing", lines.join("\n")),
            outcome: "not_joined_sent_back",
            told: format!(
                "the gate passed, but the branch {} contains the branch of none of the finished items ({}).",
                publication.head_branch,
                named.join("; ")
            ),
            undone: "their branches merged",
        };
        self.send_lead_back(site, publication, tip, &back, now, look)
    }

    /// Sends the lead, which is done, back to its branch with the text of `back`; after [`GATE_SEND_BACKS`] times in a row
    /// the human is told once instead, and the lead waits for them.
    fn send_lead_back(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        tip: &CommitId,
        back: &Back,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Site { pulls, team, lead } = site;
        let sent = self.watch.gate_sent_back.get(&lead.task).copied().unwrap_or(0);
        if sent >= GATE_SEND_BACKS {
            let announcement = Announcement::GateNotPassed {
                team: team.clone(),
                branch: publication.head_branch.clone(),
                tip: tip.clone(),
            };
            if self.watch.announced.insert(announcement) {
                let reason = format!(
                    "{} The lead was sent back {sent} times and ended again without {}; it waits for you.",
                    back.told, back.undone
                );
                let values = Variables::new()
                    .with("team", team.as_str())
                    .and_then(|values| values.with("items", items_of(publication).join(", ")))
                    .and_then(|values| values.with("reason", &reason));
                let items = items_of(publication);
                let rounds = self
                    .parts
                    .state
                    .team(team)
                    .into_iter()
                    .flat_map(agent_core::state::Team::rounds)
                    .filter(|round| round.item().is_some_and(|item| items.contains(&item.to_string())))
                    .map(|round| round.id().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let values = values
                    .and_then(|v| v.with("rounds", rounds))
                    .and_then(|v| v.with("task", lead.task.as_str()));
                self.tell_human(pulls, AutopilotNotice::HandoverHeld, values, lead, now);
            }
            return Ok(());
        }
        let sent = sent.saturating_add(1);
        let text = Variables::new()
            .with("branch", publication.head_branch.as_str())
            .and_then(|values| values.with(back.variable.0, back.variable.1.as_str()))
            .and_then(|values| values.with("sent", sent.to_string()))
            .and_then(|values| values.with("limit", GATE_SEND_BACKS.to_string()))
            .map_err(PublishError::from)
            .and_then(|values| Ok(pulls.texts.render(back.prompt, &values)?));
        let text = match text {
            Ok(text) => text,
            Err(error) => return self.publish_failed(pulls, team, lead, publication, &error, now),
        };
        tracing::info!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = back.outcome,
            team = %team,
            task = %lead.task,
            tip = %tip,
            sent,
            reason = %back.told,
            "the lead is done and its branch cannot be published as it is: it is sent back"
        );
        self.watch.gate_sent_back.insert(lead.task.clone(), sent);
        self.release(&lead.task, Why::Gate(text.trim_end().to_owned()), look)
    }

    /// What follows a branch that could not be published: it is logged, tried again after a while, and the human is told
    /// once.
    pub(super) fn publish_failed(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        publication: &Publication,
        error: &PublishError,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        if let PublishError::RepoGone { repo } = error {
            tracing::error!(
                event = LogEvent::ConfigurationNameMissing.as_str(),
                team = %team_id,
                kind = "repo",
                name = %repo,
                "a repository in the state is gone from the configuration"
            );
        }
        let reason = error_chain(error);
        tracing::warn!(
            event = LogEvent::GithubCallFailed.as_str(),
            team = %team_id,
            branch = %publication.head_branch,
            %reason,
            "a branch cannot be published: it is tried again in a minute"
        );
        let retry = Timestamp::from_utc(now.as_utc().saturating_add(PUBLISH_BACKOFF))?;
        self.watch
            .retry_at
            .insert((team_id.clone(), publication.head_branch.clone()), retry);
        let announcement = Announcement::PublishFailed {
            team: team_id.clone(),
            branch: publication.head_branch.clone(),
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("items", items_of(publication).join(", ")))
            .and_then(|values| values.with("reason", &reason));
        self.tell_human(pulls, AutopilotNotice::PublishFailed, values, lead, now);
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/joined_pass_readers.rs"]
mod joined_pass_readers_tests;
