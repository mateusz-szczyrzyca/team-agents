//! Pull requests: which items are published through which branch, and what is written on the pull request.
//!
//! A pull request is made of the branch of the lead of the team: the coordinator integrates the members' branches into
//! its own and gets the gate green on it, and a solo agent's pull request is made of the branch of its item. The
//! daemon publishes that branch when the lead's last gate result passes on a clean tree at its tip; what is decided
//! here is which items go in which pull request, and the texts. Pushing and talking to GitHub is the owner's
//! (`owner/rounds.rs`).

use agent_core::config::RepoName;
use agent_core::ids::{BranchName, InvalidBranchName, InvalidName, ItemId};
use agent_core::kinds::Role;
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::state::{GateRecord, ItemPhase, Member, PullRequest, RunEnd, RunState, Team};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_core::task_list::Item;
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};

use super::team::{agent_name, task_branch};

/// The longest title of a pull request the harness writes, in characters: GitHub's limit is 256.
pub(super) const MAX_TITLE_CHARS: usize = 200;

/// An item that joins a pull request: finished by its member, to be checked against the branch of the pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Joining {
    /// The item.
    pub(super) item: ItemId,
    /// The branch its member worked on it on; it is part of the pull request when the head contains it.
    pub(super) branch: BranchName,
}

/// What is published of a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Publish {
    /// A pull request is opened for these items, none of which has one.
    Open {
        /// The items that go in it.
        items: Vec<Joining>,
    },
    /// The pull request is updated: its items were fixed, and these finished items join it.
    Update {
        /// The pull request.
        pr: PullRequest,
        /// The items of the pull request that are being fixed.
        fixing: Vec<ItemId>,
        /// The finished items that join it.
        joining: Vec<Joining>,
    },
}

/// A branch that is to be published, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Publication {
    /// The branch the pull request is made of.
    pub(super) head_branch: BranchName,
    /// What is done to the pull request.
    pub(super) publish: Publish,
}

/// Why the publications of a team cannot be decided.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlanError {
    /// The name of the lead is not an agent name.
    #[error("the name of the lead of the team is not an agent name")]
    Name(#[from] InvalidName),
    /// The branch of the lead is not a branch name.
    #[error("the branch of the lead of the team is not a branch name")]
    Branch(#[from] InvalidBranchName),
}

/// The branches of `team` that are ready to be published: the lead has finished (its task waits) and has something to
/// publish.
///
/// - A finished item (its member waits, or its member is stopped or gone and the lead ended with `STATUS: DONE`) with
///   no pull request opens one. A branch has one pull request at a time: while
///   an item of the team is in review or approved on it, the finished items wait, except that a team's items that the
///   head of an update contains join it.
/// - An item that is being fixed, when its lead waits, updates its pull request.
///
/// # Errors
/// [`PlanError`] when a name of the team is not valid.
pub(super) fn plan_publications(team: &Team) -> Result<Vec<Publication>, PlanError> {
    let roster = team.crew().roster();
    let Some((role, lead)) = roster.first() else {
        return Ok(Vec::new());
    };
    // The lead that works may still change its branch: what it has done is published when it is done.
    if lead.status() != TaskStatus::Waiting {
        return Ok(Vec::new());
    }
    let team_branch = match role {
        Role::Solo => None,
        Role::Coordinator | Role::Member | Role::Worker | Role::Reviewer => {
            Some(task_branch(&agent_name(lead.agent())?, lead.task())?)
        }
    };
    let mut groups: Vec<Group> = Vec::new();
    for progress in team.items() {
        // An accepted delivery waits for the merge of another PR, also when a stop of the team abandoned its round.
        if team.delivery_waits().any(|round| round.item() == Some(progress.id()))
            || team
                .round_of(progress.id())
                .is_some_and(|round| round.delivery_closed_at().is_some())
        {
            continue;
        }
        match progress.phase() {
            ItemPhase::InProgress { assignee, branch } if joins(&roster, assignee.as_str(), lead) => {
                let head = match (team_branch.as_ref(), team.round_of(progress.id())) {
                    (Some(integration), Some(round)) if returned_delivery(team, progress.id()) => {
                        format!("{integration}-{}-{}", progress.id(), round.id()).parse()?
                    }
                    (Some(integration), round) => match round.and_then(agent_core::state::Round::part) {
                        Some(key) => format!("{integration}-{}{key}", progress.id()).parse()?,
                        None => integration.clone(),
                    },
                    (None, Some(_) | None) => branch.clone(),
                };
                with_group(&mut groups, &head, |group| {
                    group.joining.push(Joining {
                        item: progress.id(),
                        branch: branch.clone(),
                    });
                });
            }
            ItemPhase::Fixing { pr, .. } => with_group(&mut groups, &pr.branch, |group| {
                group.fixing.push(progress.id());
                group.pr.get_or_insert_with(|| pr.clone());
            }),
            ItemPhase::InReview { pr, .. } | ItemPhase::Approved { pr, .. } => {
                with_group(&mut groups, &pr.branch, |group| group.occupied = true);
            }
            ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => {}
        }
    }
    Ok(groups.into_iter().filter_map(Group::publication).collect())
}

/// Whether an observed closed claim requires independent publication of this item's replacement round.
pub(super) fn returned_delivery(team: &Team, item: ItemId) -> bool {
    team.rounds()
        .iter()
        .any(|round| round.item() == Some(item) && round.delivery_closed_at().is_some())
}

/// What is known of one branch while the items are looked at.
struct Group {
    /// The branch.
    head: BranchName,
    /// Whether an item of it is in review or approved: its pull request is open and not being fixed.
    occupied: bool,
    /// The items of its pull request that are being fixed.
    fixing: Vec<ItemId>,
    /// The pull request of the items being fixed.
    pr: Option<PullRequest>,
    /// The items that are finished and have no pull request.
    joining: Vec<Joining>,
}

impl Group {
    /// What is to be done to the branch, if anything.
    fn publication(self) -> Option<Publication> {
        let publish = match self.pr {
            Some(pr) => Publish::Update {
                pr,
                fixing: self.fixing,
                joining: self.joining,
            },
            None if !self.occupied && !self.joining.is_empty() => Publish::Open { items: self.joining },
            None => return None,
        };
        Some(Publication {
            head_branch: self.head,
            publish,
        })
    }
}

/// Changes the group of `head` in `groups` with `change`; the group is made first when there is none.
fn with_group(groups: &mut Vec<Group>, head: &BranchName, change: impl FnOnce(&mut Group)) {
    if let Some(group) = groups.iter_mut().find(|group| group.head == *head) {
        change(group);
        return;
    }
    let mut group = Group {
        head: head.clone(),
        occupied: false,
        fixing: Vec::new(),
        pr: None,
        joining: Vec::new(),
    };
    change(&mut group);
    groups.push(group);
}

/// Whether the item in progress with the member of the crew called `name` joins a publication of the branch of `lead`:
/// its member has finished (its task waits), or the member's task is stopped or no longer in the crew and the lead's
/// last run ended with `STATUS: DONE`. The item follows its task, not the agent that started it: a lead that delivered
/// the work of a member that is gone publishes it.
fn joins(roster: &[(Role, &Member)], name: &str, lead: &Member) -> bool {
    let holder = roster.iter().find(|(_, member)| member.agent().name() == name);
    match holder.map(|(_, member)| member.status()) {
        Some(TaskStatus::Waiting) => true,
        Some(TaskStatus::Stopped) | None => reported_done(lead),
        Some(
            TaskStatus::Queued
            | TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Done
            | TaskStatus::Failed,
        ) => false,
    }
}

/// Whether the last run of `member` ended with `STATUS: DONE`.
fn reported_done(member: &Member) -> bool {
    member.runs().last().is_some_and(|run| {
        matches!(
            run.state(),
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                ..
            }
        )
    })
}

/// The title and the description of a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PullText {
    /// The title, on one line.
    pub(super) title: String,
    /// The description.
    pub(super) body: String,
}

/// Why the text of a pull request cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum PullTextError {
    /// There is no item to write about.
    #[error("a pull request is written for at least one item")]
    NoItems,
    /// An item is not in the task list: a mistake of the daemon.
    #[error("item {item} is not in the task list")]
    NoSuchItem {
        /// The item.
        item: ItemId,
    },
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A text does not take the values it is given.
    #[error("cannot render a text of the pull request")]
    Render(#[from] RenderAutopilotTextError),
}

/// The title and the description of the pull request of `items` of `team`, after the gate said `gate`.
///
/// # Errors
/// [`PullTextError`].
pub(super) fn pull_text(
    texts: &AutopilotPrompts,
    team: &Team,
    repo: &RepoName,
    items: &[ItemId],
    gate: &GateRecord,
) -> Result<PullText, PullTextError> {
    if items.is_empty() {
        return Err(PullTextError::NoItems);
    }
    let list = team.task_list().list();
    let listed = items
        .iter()
        .map(|id| list.item(*id).ok_or(PullTextError::NoSuchItem { item: *id }))
        .collect::<Result<Vec<&Item>, _>>()?;
    let ids: Vec<String> = listed.iter().map(|item| item.id.to_string()).collect();
    let titles: Vec<&str> = listed.iter().map(|item| item.title.as_str()).collect();
    let title = texts.render(
        AutopilotPrompt::PrTitle,
        &Variables::new()
            .with("item_ids", ids.join(", "))?
            .with("item_titles", titles.join("; "))?,
    )?;
    let sections = listed
        .iter()
        .map(|item| item_section(texts, item))
        .collect::<Result<Vec<String>, _>>()?;
    let mut body = texts.render(
        AutopilotPrompt::PrBody,
        &Variables::new()
            .with("team", team.id().as_str())?
            .with("repo", repo.as_str())?
            .with("gate", gate.summary())?
            .with("items", sections.join(SECTION_SEPARATOR))?,
    )?;
    if let Some(base) = team.unpushed_base() {
        body.push_str(SECTION_SEPARATOR);
        body.push_str(
            &texts.render(
                AutopilotPrompt::PrUnpushedBase,
                &Variables::new()
                    .with("base", base.branch.as_str())?
                    .with("count", base.count.to_string())?
                    .with("first", format!("{:.7}", base.first.as_str()))?
                    .with("subject", &base.subject)?,
            )?,
        );
    }
    Ok(PullText {
        title: title.trim().chars().take(MAX_TITLE_CHARS).collect(),
        body: body.trim_end().to_owned(),
    })
}

/// Between two sections of a description: a blank line.
const SECTION_SEPARATOR: &str = "\n\n";

/// One item in a description: the item, then its expected result and its checks, those it has.
fn item_section(texts: &AutopilotPrompts, item: &Item) -> Result<String, PullTextError> {
    let values = Variables::new()
        .with("item_id", item.id.to_string())?
        .with("item_title", &item.title)?
        .with("item_description", &item.description)?;
    let mut sections = vec![texts.render(AutopilotPrompt::PrBodyItem, &values)?];
    if let Some(expected) = &item.expected_result {
        let values = Variables::new().with("expected", expected)?;
        sections.push(texts.render(AutopilotPrompt::PrBodyExpected, &values)?);
    }
    if let Some(checks) = &item.checks {
        let values = Variables::new().with("checks", checks)?;
        sections.push(texts.render(AutopilotPrompt::PrBodyChecks, &values)?);
    }
    Ok(sections
        .iter()
        .map(|section| section.trim_end())
        .collect::<Vec<_>>()
        .join(SECTION_SEPARATOR))
}

#[cfg(test)]
mod tests;

pub(super) mod description;
