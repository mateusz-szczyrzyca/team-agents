//! Teams: who is in them and in which role, the task list they work through and the progress of each item.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::checked::CheckedMut;
use super::item::{Completion, ItemMut, ItemPhase, ItemProgress};
use super::member::{Member, MemberMut, Participant};
use super::round::{Round, RoundEnd};
use super::strict::checked;
use super::values::Timestamp;
use crate::config::RepoName;
use crate::ids::{ItemId, RoundId, TaskId, TeamId};
use crate::kinds::{Keyword, Role, Tier, UnknownKeyword};
use crate::task_list::{ItemMark, TaskList, TaskListError};

mod legacy;
mod rounds;

pub(super) use self::legacy::{LegacyError, TeamV1};
pub use self::rounds::{AdvanceItemError, BeginRunError, CloseRoundError, NewRun, OpenRoundError, RoundsError};

/// What happens to a pull request that both reviewers approve.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<MergeMode>)]
pub enum MergeMode {
    /// The harness merges it and starts the next item (`agent run`'s default).
    Autopilot,
    /// The harness stops before the merge and hands the pull request to the human (`--no-merge`).
    NoMerge,
}

impl Keyword for MergeMode {
    const WHAT: &'static str = "merge mode";
}

impl TryFrom<String> for MergeMode {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl MergeMode {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The variant's text: what `Display` prints and `FromStr` and serde accept.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// A task list as it was given: its text, which the state stores, and what the text parses into.
///
/// Made only by [`TaskListText::new`], so the parsed list always is the parse of the stored text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskListText {
    /// The text, as given.
    text: String,
    /// What the text parses into.
    list: TaskList,
}

impl TaskListText {
    /// Parses `text` as a task list and keeps both.
    ///
    /// # Errors
    /// The [`TaskListError`] of the parse.
    pub fn new(text: String) -> Result<Self, TaskListError> {
        let list = TaskList::parse(&text)?;
        Ok(Self { text, list })
    }

    getter!("The text, as given.", pub fn text(&self) -> &str = &self.text);

    getter!("The parsed list.", pub const fn list(&self) -> &TaskList = &self.list);
}

impl TryFrom<String> for TaskListText {
    type Error = TaskListError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text)
    }
}

impl From<TaskListText> for String {
    fn from(value: TaskListText) -> Self {
        value.text
    }
}

/// The people of a team and their roles: one agent alone, or a coordinator with members and workers.
///
/// Made by [`Crew::solo`] and [`Crew::team`], which check what the roles need; reading a state checks the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Shape", into = "Shape")]
pub struct Crew {
    /// How the crew is made up.
    shape: Shape,
}

/// How a crew is made up, before its checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Shape {
    /// One agent alone.
    Solo {
        /// The agent.
        member: Member,
    },
    /// A coordinator, the members who take items, and the workers who serve `delegate` requests.
    Team {
        /// The coordinator.
        coordinator: Member,
        /// The members, in the order they were assigned items.
        members: Vec<Member>,
        /// The workers, in the configured order, which is the order of delegation.
        workers: Vec<Member>,
    },
}

/// The people of a team, by role, for [`Crew::team`]: named, so that members and workers cannot be swapped by
/// accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMembers {
    /// The coordinator: a normal agent or an investigator.
    pub coordinator: Member,
    /// The members who take items: normal agents or investigators, at least one.
    pub members: Vec<Member>,
    /// The workers who serve `delegate` requests, in the configured order.
    pub workers: Vec<Member>,
}

/// Why a crew cannot be made from its members.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CrewError {
    /// The participant's tier does not fit the role.
    #[error("task {task}: a {tier} cannot be {role}")]
    WrongTier {
        /// The member's task.
        task: TaskId,
        /// The role it was given.
        role: Role,
        /// The participant's tier.
        tier: Tier,
    },
    /// A team has no member who takes items.
    #[error("a team needs at least one member besides its coordinator")]
    NoMembers,
    /// A name appears twice.
    #[error("{agent} is in the crew twice")]
    DuplicateAgent {
        /// The second participant with the name.
        agent: Participant,
    },
    /// A task ID appears twice.
    #[error("task {task} is in the crew twice")]
    DuplicateTask {
        /// The repeated ID.
        task: TaskId,
    },
}

impl Crew {
    /// One agent alone: a normal agent or an investigator.
    ///
    /// # Errors
    /// [`CrewError::WrongTier`] for a worker.
    pub fn solo(member: Member) -> Result<Self, CrewError> {
        Self::try_from(Shape::Solo { member })
    }

    /// A coordinator (a normal agent or an investigator), members who take items (normal agents or investigators) and
    /// workers.
    ///
    /// # Errors
    /// [`CrewError`]: a participant of the wrong tier for its role, no member, a name or a task ID twice.
    pub fn team(people: TeamMembers) -> Result<Self, CrewError> {
        let TeamMembers {
            coordinator,
            members,
            workers,
        } = people;
        Self::try_from(Shape::Team {
            coordinator,
            members,
            workers,
        })
    }

    /// The coordinator; `None` for one agent alone.
    #[must_use]
    pub const fn coordinator(&self) -> Option<&Member> {
        match &self.shape {
            Shape::Solo { .. } => None,
            Shape::Team { coordinator, .. } => Some(coordinator),
        }
    }

    getter!(
        "Every member with its role: the solo agent, or the coordinator, the members and the workers in that order.",
        pub fn roster(&self) -> Vec<(Role, &Member)> = self.shape.roster()
    );

    /// The member of task `task` with its role.
    #[must_use]
    pub fn find(&self, task: &TaskId) -> Option<(Role, &Member)> {
        self.roster().into_iter().find(|(_, member)| member.task() == task)
    }

    /// The member of task `task`, to change its task through the operations of [`MemberMut`].
    pub fn find_mut(&mut self, task: &TaskId) -> Option<(Role, MemberMut<'_>)> {
        let (role, member) = match &mut self.shape {
            Shape::Solo { member } => (member.task() == task).then_some((Role::Solo, member)),
            Shape::Team {
                coordinator,
                members,
                workers,
            } => std::iter::once((Role::Coordinator, coordinator))
                .chain(members.iter_mut().map(|member| (Role::Member, member)))
                .chain(workers.iter_mut().map(|worker| (Role::Worker, worker)))
                .find(|(_, member)| member.task() == task),
        }?;
        Some((role, MemberMut::new(member)))
    }
}

/// The people of a team inside a state, to change through their members' operations only.
///
/// It reads like a [`Crew`] (it dereferences to one) but is not a `&mut Crew`: the crew cannot be overwritten with
/// another, so the roles, names and task IDs the crew checked stay as they were.
pub type CrewMut<'a> = CheckedMut<'a, Crew>;

impl<'a> CheckedMut<'a, Crew> {
    /// The member of task `task` with its role, to change its task. Takes the handle, so that the member handle
    /// lives as long as the borrow of the team, not as long as this temporary.
    #[must_use]
    pub fn find_mut(self, task: &TaskId) -> Option<(Role, MemberMut<'a>)> {
        self.value.find_mut(task)
    }
}

impl Shape {
    /// Every member with its role: the solo agent, or the coordinator, the members and the workers in that order.
    fn roster(&self) -> Vec<(Role, &Member)> {
        match self {
            Self::Solo { member } => vec![(Role::Solo, member)],
            Self::Team {
                coordinator,
                members,
                workers,
            } => std::iter::once((Role::Coordinator, coordinator))
                .chain(members.iter().map(|member| (Role::Member, member)))
                .chain(workers.iter().map(|worker| (Role::Worker, worker)))
                .collect(),
        }
    }

    /// Checks the rules of a crew, in this order: the tiers, a member besides the coordinator, the names, the tasks.
    fn check(&self) -> Result<(), CrewError> {
        let roster = self.roster();
        if let Some((role, member)) = roster.iter().find(|(role, member)| !tier_fits(*role, member.agent())) {
            return Err(CrewError::WrongTier {
                task: member.task().clone(),
                role: *role,
                tier: member.agent().tier(),
            });
        }
        if matches!(self, Self::Team { members, .. } if members.is_empty()) {
            return Err(CrewError::NoMembers);
        }
        let mut names = HashSet::new();
        if let Some((_, member)) = roster.iter().find(|(_, member)| !names.insert(member.agent().name())) {
            return Err(CrewError::DuplicateAgent {
                agent: member.agent().clone(),
            });
        }
        let mut tasks = HashSet::new();
        if let Some((_, member)) = roster.iter().find(|(_, member)| !tasks.insert(member.task())) {
            return Err(CrewError::DuplicateTask {
                task: member.task().clone(),
            });
        }
        Ok(())
    }
}

impl TryFrom<Shape> for Crew {
    type Error = CrewError;

    fn try_from(shape: Shape) -> Result<Self, Self::Error> {
        shape.check()?;
        Ok(Self { shape })
    }
}

impl From<Crew> for Shape {
    fn from(crew: Crew) -> Self {
        crew.shape
    }
}

/// A team: its repository, its task list, the progress of each item, its crew and the investigators that review it.
///
/// The progress has one entry for each item of the task list, in its order, and an item the list marks `[x]` is done
/// from the start; [`Team::new`] makes it so and reading a state checks it. Reviewers are investigators outside the
/// crew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Team {
    /// The team's ID.
    id: TeamId,
    /// The repository the team works on, as `[repos.NAME]` of the configuration.
    repo: RepoName,
    /// When the team was started.
    created_at: Timestamp,
    /// What happens to an approved pull request.
    merge: MergeMode,
    /// The task list the team works through.
    task_list: TaskListText,
    /// The progress of each item, in the order of the task list.
    items: Vec<ItemProgress>,
    /// The rounds of the team, oldest first: the work of the team on each item that has started, or on the whole list
    /// when it has no items.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rounds: Vec<Round>,
    /// The people of the team.
    crew: Crew,
    /// The investigators that review the team's pull requests, with their tasks.
    reviewers: Vec<Member>,
}

/// How a [`Team`] is read; see [`checked!`].
#[derive(Deserialize)]
#[serde(remote = "Team", deny_unknown_fields)]
struct TeamWire {
    /// See [`Team`].
    id: TeamId,
    /// See [`Team`].
    repo: RepoName,
    /// See [`Team`].
    created_at: Timestamp,
    /// See [`Team`].
    merge: MergeMode,
    /// See [`Team`].
    task_list: TaskListText,
    /// See [`Team`].
    items: Vec<ItemProgress>,
    /// See [`Team`]; a team written without rounds has none.
    #[serde(default)]
    rounds: Vec<Round>,
    /// See [`Team`].
    crew: Crew,
    /// See [`Team`]; a team written without reviewers has none.
    #[serde(default)]
    reviewers: Vec<Member>,
}

checked!(Team, TeamWire, check_team);

/// Why a team's parts do not fit together.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeamError {
    /// The progress is not one entry for each item of the task list, in order.
    #[error("team {team}: the progress at position {position} is for {found:?}, the task list has {expected:?} there")]
    ItemsMismatch {
        /// The team.
        team: TeamId,
        /// The first position (from 0) where they differ.
        position: usize,
        /// The item the task list has there; `None` when the progress is longer.
        expected: Option<ItemId>,
        /// The item the progress has there; `None` when it is shorter.
        found: Option<ItemId>,
    },
    /// An item the task list marks `[x]` is not done, or the other way round.
    #[error("team {team}: item {item} is listed {listed:?} but its progress says otherwise")]
    ListedMismatch {
        /// The team.
        team: TeamId,
        /// The item.
        item: ItemId,
        /// What the list says.
        listed: ItemMark,
    },
    /// The rounds do not fit the items and the runs.
    #[error("team {team}: {source}")]
    Rounds {
        /// The team.
        team: TeamId,
        /// What is wrong.
        source: RoundsError,
    },
    /// A reviewer is not an investigator, or is in the crew, or two reviewers share a name or a task.
    #[error("team {team}: {reason}")]
    Reviewer {
        /// The team.
        team: TeamId,
        /// What is wrong.
        reason: ReviewerProblem,
    },
}

/// What is wrong with a reviewer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewerProblem {
    /// The reviewer is not an investigator.
    #[error("{agent} reviews but is not an investigator")]
    NotInvestigator {
        /// The reviewer.
        agent: Participant,
    },
    /// The reviewer is in the team's crew: reviewers are outside it.
    #[error("{agent} reviews its own team")]
    InCrew {
        /// The reviewer.
        agent: Participant,
    },
    /// The reviewer's name appears twice among the reviewers.
    #[error("{agent} is a reviewer twice")]
    DuplicateName {
        /// The second reviewer with the name.
        agent: Participant,
    },
    /// A task ID appears twice among the reviewers or is the crew's.
    #[error("task {task} is used twice")]
    DuplicateTask {
        /// The repeated ID.
        task: TaskId,
    },
}

impl Team {
    /// A team that has just been formed: every item pending, except those the task list marks done.
    #[must_use]
    pub fn new(
        id: TeamId,
        repo: RepoName,
        created_at: Timestamp,
        merge: MergeMode,
        task_list: TaskListText,
        crew: Crew,
    ) -> Self {
        Self {
            id,
            repo,
            created_at,
            merge,
            items: initial_items(&task_list),
            rounds: Vec::new(),
            task_list,
            crew,
            reviewers: Vec::new(),
        }
    }

    getter!("The team's ID.", pub const fn id(&self) -> &TeamId = &self.id);

    getter!("The repository the team works on.", pub const fn repo(&self) -> &RepoName = &self.repo);

    getter!("When the team was started.", pub const fn created_at(&self) -> Timestamp = self.created_at);

    getter!("What happens to an approved pull request.", pub const fn merge(&self) -> MergeMode = self.merge);

    getter!("The task list the team works through.", pub const fn task_list(&self) -> &TaskListText = &self.task_list);

    getter!("The progress of each item, in the order of the task list.", pub fn items(&self) -> &[ItemProgress] = &self.items);

    /// The progress of item `id`.
    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&ItemProgress> {
        self.items.iter().find(|item| item.id() == id)
    }

    /// The progress of item `id`, to change it through the operations of [`ItemMut`].
    pub fn item_mut(&mut self, id: ItemId) -> Option<ItemMut<'_>> {
        self.items.iter_mut().find(|item| item.id() == id).map(ItemMut::new)
    }

    getter!("The people of the team.", pub const fn crew(&self) -> &Crew = &self.crew);

    /// The people of the team, to change their tasks through the operations of [`CrewMut`].
    pub const fn crew_mut(&mut self) -> CrewMut<'_> {
        CrewMut::new(&mut self.crew)
    }

    getter!("The investigators that review the team's pull requests.", pub fn reviewers(&self) -> &[Member] = &self.reviewers);

    /// The reviewers, to change their tasks through the operations of [`MemberMut`], in the order of
    /// [`Team::reviewers`].
    pub fn reviewers_mut(&mut self) -> Vec<MemberMut<'_>> {
        self.reviewers.iter_mut().map(MemberMut::new).collect()
    }

    /// Adds a reviewer.
    pub(super) fn add_reviewer(&mut self, member: Member) -> Result<(), ReviewerProblem> {
        check_reviewer(&self.crew, &self.reviewers, &member)?;
        self.reviewers.push(member);
        Ok(())
    }

    /// The member of task `task`, in the crew or among the reviewers, with its role, to change its task.
    pub fn find_mut(&mut self, task: &TaskId) -> Option<(Role, MemberMut<'_>)> {
        if let Some(found) = self.crew.find_mut(task) {
            return Some(found);
        }
        self.reviewers
            .iter_mut()
            .find(|member| member.task() == task)
            .map(|member| (Role::Reviewer, MemberMut::new(member)))
    }

    /// The member of task `task`, in the crew or among the reviewers, with its role.
    #[must_use]
    pub fn find(&self, task: &TaskId) -> Option<(Role, &Member)> {
        self.crew.find(task).or_else(|| {
            self.reviewers
                .iter()
                .find(|member| member.task() == task)
                .map(|member| (Role::Reviewer, member))
        })
    }
}

/// A team inside a state, to change through its operations only.
///
/// It reads like a [`Team`] (it dereferences to one) but is not a `&mut Team`: the team cannot be overwritten with
/// another, so the state keeps one team for each ID and the team keeps its items, crew and reviewers as they were
/// checked. What may change is reached through the checked handles below.
pub type TeamMut<'a> = CheckedMut<'a, Team>;

impl CheckedMut<'_, Team> {
    /// See [`Team::item_mut`].
    pub fn item_mut(&mut self, id: ItemId) -> Option<ItemMut<'_>> {
        self.value.item_mut(id)
    }

    /// See [`Team::open_round`].
    ///
    /// # Errors
    /// [`OpenRoundError`].
    pub fn open_round(&mut self, id: RoundId, item: Option<ItemId>, at: Timestamp) -> Result<(), OpenRoundError> {
        self.value.open_round(id, item, at)
    }

    /// See [`Team::close_round`].
    ///
    /// # Errors
    /// [`CloseRoundError`].
    pub fn close_round(&mut self, id: &RoundId, at: Timestamp, how: RoundEnd) -> Result<(), CloseRoundError> {
        self.value.close_round(id, at, how)
    }

    /// See [`Team::advance_item`].
    ///
    /// # Errors
    /// [`AdvanceItemError`].
    pub fn advance_item(&mut self, id: ItemId, next: ItemPhase) -> Result<(), AdvanceItemError> {
        self.value.advance_item(id, next)
    }

    /// See [`Team::begin_run`].
    ///
    /// # Errors
    /// [`BeginRunError`].
    pub fn begin_run(&mut self, task: &TaskId, run: NewRun) -> Result<(), BeginRunError> {
        self.value.begin_run(task, run)
    }

    /// See [`Team::crew_mut`].
    pub const fn crew_mut(&mut self) -> CrewMut<'_> {
        self.value.crew_mut()
    }

    /// See [`Team::reviewers_mut`].
    pub fn reviewers_mut(&mut self) -> Vec<MemberMut<'_>> {
        self.value.reviewers_mut()
    }

    /// See [`Team::find_mut`].
    pub fn find_mut(&mut self, task: &TaskId) -> Option<(Role, MemberMut<'_>)> {
        self.value.find_mut(task)
    }
}

/// The progress of every item of `task_list` as a team starts: pending, or done for an item marked `[x]`.
fn initial_items(task_list: &TaskListText) -> Vec<ItemProgress> {
    task_list
        .list()
        .items()
        .map(|item| ItemProgress::new(item.id, item.mark))
        .collect()
}

/// Checks that `items` is one entry for each item of `task_list`, in order, and that an entry is done as listed if
/// and only if the list marks its item `[x]`.
fn check_items(team: &TeamId, task_list: &TaskListText, items: &[ItemProgress]) -> Result<(), TeamError> {
    let listed: Vec<(ItemId, ItemMark)> = task_list.list().items().map(|item| (item.id, item.mark)).collect();
    for position in 0..listed.len().max(items.len()) {
        let expected = listed.get(position).map(|&(id, _)| id);
        let found = items.get(position).map(ItemProgress::id);
        if expected != found {
            return Err(TeamError::ItemsMismatch {
                team: team.clone(),
                position,
                expected,
                found,
            });
        }
    }
    for (&(item, mark), progress) in listed.iter().zip(items) {
        let done_as_listed = matches!(
            progress.phase(),
            ItemPhase::Done {
                completion: Completion::Listed
            }
        );
        match (mark, done_as_listed) {
            (ItemMark::Done, true) | (ItemMark::Open, false) => {}
            (ItemMark::Done | ItemMark::Open, _) => {
                return Err(TeamError::ListedMismatch {
                    team: team.clone(),
                    item,
                    listed: mark,
                });
            }
        }
    }
    Ok(())
}

/// Checks a proposed reviewer against a crew and the reviewers already before it.
fn check_reviewer(crew: &Crew, reviewers: &[Member], member: &Member) -> Result<(), ReviewerProblem> {
    let name = member.agent().name();
    if !tier_fits(Role::Reviewer, member.agent()) {
        return Err(ReviewerProblem::NotInvestigator {
            agent: member.agent().clone(),
        });
    }
    if crew.roster().iter().any(|(_, crew)| crew.agent().name() == name) {
        return Err(ReviewerProblem::InCrew {
            agent: member.agent().clone(),
        });
    }
    if reviewers.iter().any(|reviewer| reviewer.agent().name() == name) {
        return Err(ReviewerProblem::DuplicateName {
            agent: member.agent().clone(),
        });
    }
    if crew.find(member.task()).is_some() || reviewers.iter().any(|reviewer| reviewer.task() == member.task()) {
        return Err(ReviewerProblem::DuplicateTask {
            task: member.task().clone(),
        });
    }
    Ok(())
}

/// Checks that a team's items, rounds and reviewers fit its task list, crew and one another.
fn check_team(team: &Team) -> Result<(), TeamError> {
    check_items(&team.id, &team.task_list, &team.items)?;
    rounds::check_rounds(team)?;
    for (index, reviewer) in team.reviewers.iter().enumerate() {
        let previous = team.reviewers.get(..index).unwrap_or_default();
        check_reviewer(&team.crew, previous, reviewer).map_err(|reason| TeamError::Reviewer {
            team: team.id.clone(),
            reason,
        })?;
    }
    Ok(())
}

/// Whether a participant has the tier a role needs.
#[must_use]
const fn tier_fits(role: Role, participant: &Participant) -> bool {
    matches!(
        (role, participant),
        (
            Role::Solo | Role::Coordinator | Role::Member,
            Participant::Normal(_) | Participant::Investigator(_)
        ) | (Role::Worker, Participant::Worker(_))
            | (Role::Reviewer, Participant::Investigator(_))
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ids::AgentName;
    use crate::kinds::tests::{assert_keywords, assert_refuses};
    use crate::state::fixtures::{
        LIST, assert_refused, at, inv, list, member, normal, repo, task, team, team_id, worker,
    };
    use crate::status::{ItemStatus, TaskStatus};
    use crate::task_list::ItemMark;

    /// A team's serialized shape, used to construct teams that the public constructors refuse.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TeamWire {
        /// See [`Team`].
        id: TeamId,
        /// See [`Team`].
        repo: RepoName,
        /// See [`Team`].
        created_at: Timestamp,
        /// See [`Team`].
        merge: MergeMode,
        /// See [`Team`].
        task_list: TaskListText,
        /// See [`Team`].
        items: Vec<ItemProgress>,
        /// See [`Team`].
        crew: Crew,
        /// See [`Team`].
        #[serde(default)]
        reviewers: Vec<Member>,
    }

    impl TryFrom<TeamWire> for Team {
        type Error = TeamError;

        fn try_from(wire: TeamWire) -> Result<Self, Self::Error> {
            let team = Self {
                id: wire.id,
                repo: wire.repo,
                created_at: wire.created_at,
                merge: wire.merge,
                task_list: wire.task_list,
                items: wire.items,
                rounds: Vec::new(),
                crew: wire.crew,
                reviewers: wire.reviewers,
            };
            check_team(&team)?;
            Ok(team)
        }
    }

    /// The task list item `id`, as a number.
    fn item(id: &str) -> ItemId {
        id.parse().unwrap()
    }

    // ---- the vocabulary -----------------------------------------------------------------------------------------

    #[test]
    fn merge_modes_have_their_texts_and_round_trip() {
        let modes = [(MergeMode::Autopilot, "autopilot"), (MergeMode::NoMerge, "no_merge")];

        assert_eq!(MergeMode::ALL, modes.iter().map(|&(m, _)| m).collect::<Vec<_>>());
        assert_keywords(&modes);
        assert_refuses("merge mode", &modes, &["no-merge", "manual", "merge"]);
    }

    #[test]
    fn a_task_list_text_keeps_the_text_as_given_and_what_it_parses_into() {
        let text = format!("\u{feff}{LIST}");

        let kept = TaskListText::new(text.clone()).unwrap();

        assert_eq!(kept.text(), text);
        assert_eq!(kept.list().items().count(), 3);
        assert_eq!(kept.list().coordinator().map(AgentName::as_str), Some("inv-opus"));
    }

    #[test]
    fn a_task_list_text_that_is_not_a_task_list_is_refused_with_the_parse_error() {
        let err = TaskListText::new(String::from("1. Feature\n[ ] 1.x Broken\n")).unwrap_err();

        assert_eq!(err, TaskList::parse("1. Feature\n[ ] 1.x Broken\n").unwrap_err());
        assert!(matches!(TaskListText::new(String::new()), Err(TaskListError::Empty)));
    }

    #[test]
    fn a_task_list_text_is_written_as_its_text_and_read_back_by_parsing_it() {
        let kept = list();

        let value = serde_json::to_value(&kept).unwrap();

        assert_eq!(value, json!(LIST));
        assert_eq!(serde_json::from_value::<TaskListText>(value).unwrap(), kept);
        assert_refused::<TaskListText>(&json!("1. Feature\n[ ] 1.x Broken\n"));
        assert_refused::<TaskListText>(&json!(""));
        assert_refused::<TaskListText>(&json!(5));
    }

    #[test]
    fn the_roles_need_these_tiers() {
        let table = [
            (Role::Solo, [true, false, true]),
            (Role::Coordinator, [true, false, true]),
            (Role::Member, [true, false, true]),
            (Role::Worker, [false, true, false]),
            (Role::Reviewer, [false, false, true]),
        ];
        let participants = [normal("a"), worker("b"), inv("c")];
        for (role, fits) in table {
            for (participant, expected) in participants.iter().zip(fits) {
                assert_eq!(tier_fits(role, participant), expected, "{role} {}", participant.tier());
            }
        }
    }

    // ---- the crew -----------------------------------------------------------------------------------------------

    #[test]
    fn one_agent_alone_is_a_solo_crew_whether_normal_or_investigator() {
        for agent in [normal("codex-1"), inv("inv-opus")] {
            let crew = Crew::solo(member(1, agent)).unwrap();

            let roster = crew.roster();

            assert_eq!(roster.len(), 1);
            assert_eq!(roster[0].0, Role::Solo);
            assert!(crew.coordinator().is_none());
        }
    }

    #[test]
    fn a_worker_cannot_work_alone() {
        let err = Crew::solo(member(1, worker("worker-1"))).unwrap_err();

        assert_eq!(
            err,
            CrewError::WrongTier {
                task: task(1),
                role: Role::Solo,
                tier: Tier::Worker
            }
        );
    }

    #[test]
    fn a_team_lists_its_coordinator_members_and_workers_in_that_order_with_their_roles() {
        let crew = Crew::team(TeamMembers {
            coordinator: member(1, inv("inv-opus")),
            members: vec![member(2, normal("codex-1")), member(3, inv("inv-codex"))],
            workers: vec![member(4, worker("worker-1")), member(5, worker("worker-2"))],
        })
        .unwrap();

        let roster: Vec<(Role, &str)> = crew
            .roster()
            .iter()
            .map(|(role, m)| (*role, m.agent().name()))
            .collect();

        assert_eq!(
            roster,
            [
                (Role::Coordinator, "inv-opus"),
                (Role::Member, "codex-1"),
                (Role::Member, "inv-codex"),
                (Role::Worker, "worker-1"),
                (Role::Worker, "worker-2"),
            ]
        );
        assert_eq!(crew.coordinator().map(|m| m.task().clone()), Some(task(1)));
    }

    #[test]
    fn a_team_may_have_a_normal_coordinator_and_no_workers() {
        let crew = Crew::team(TeamMembers {
            coordinator: member(1, normal("claude-1")),
            members: vec![member(2, normal("codex-1"))],
            workers: vec![],
        })
        .unwrap();

        assert_eq!(crew.roster().len(), 2);
    }

    #[test]
    fn a_role_refuses_a_participant_of_the_wrong_tier() {
        let cases = [
            (
                Crew::team(TeamMembers {
                    coordinator: member(1, worker("w")),
                    members: vec![member(2, normal("a"))],
                    workers: vec![],
                }),
                CrewError::WrongTier {
                    task: task(1),
                    role: Role::Coordinator,
                    tier: Tier::Worker,
                },
            ),
            (
                Crew::team(TeamMembers {
                    coordinator: member(1, normal("a")),
                    members: vec![member(2, worker("w"))],
                    workers: vec![],
                }),
                CrewError::WrongTier {
                    task: task(2),
                    role: Role::Member,
                    tier: Tier::Worker,
                },
            ),
            (
                Crew::team(TeamMembers {
                    coordinator: member(1, normal("a")),
                    members: vec![member(2, normal("b"))],
                    workers: vec![member(3, normal("c"))],
                }),
                CrewError::WrongTier {
                    task: task(3),
                    role: Role::Worker,
                    tier: Tier::Normal,
                },
            ),
            (
                Crew::team(TeamMembers {
                    coordinator: member(1, normal("a")),
                    members: vec![member(2, normal("b"))],
                    workers: vec![member(3, inv("i"))],
                }),
                CrewError::WrongTier {
                    task: task(3),
                    role: Role::Worker,
                    tier: Tier::Investigator,
                },
            ),
        ];
        for (result, expected) in cases {
            assert_eq!(result.unwrap_err(), expected);
        }
    }

    #[test]
    fn a_team_needs_a_member_besides_its_coordinator_and_workers_do_not_count() {
        assert_eq!(
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![],
                workers: vec![]
            })
            .unwrap_err(),
            CrewError::NoMembers
        );
        assert_eq!(
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![],
                workers: vec![member(2, worker("w"))]
            })
            .unwrap_err(),
            CrewError::NoMembers
        );
    }

    #[test]
    fn a_name_is_in_the_crew_once() {
        let cases = [
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![member(2, normal("a"))],
                workers: vec![],
            }),
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![member(2, normal("b")), member(3, normal("b"))],
                workers: vec![],
            }),
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![member(2, inv("a"))],
                workers: vec![],
            }),
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![member(2, normal("b"))],
                workers: vec![member(3, worker("a"))],
            }),
        ];
        for result in cases {
            assert!(matches!(result, Err(CrewError::DuplicateAgent { .. })), "{result:?}");
        }
        assert_eq!(
            Crew::team(TeamMembers {
                coordinator: member(1, normal("a")),
                members: vec![member(2, normal("a"))],
                workers: vec![]
            })
            .unwrap_err(),
            CrewError::DuplicateAgent { agent: normal("a") }
        );
    }

    #[test]
    fn a_task_is_in_the_crew_once() {
        let err = Crew::team(TeamMembers {
            coordinator: member(1, normal("a")),
            members: vec![member(1, normal("b"))],
            workers: vec![],
        })
        .unwrap_err();

        assert_eq!(err, CrewError::DuplicateTask { task: task(1) });
    }

    #[test]
    fn the_crew_is_checked_tiers_first_then_members_then_names_then_tasks() {
        let tier_and_name = Crew::team(TeamMembers {
            coordinator: member(1, worker("a")),
            members: vec![member(2, normal("a"))],
            workers: vec![],
        });
        let members_and_name = Crew::team(TeamMembers {
            coordinator: member(1, normal("a")),
            members: vec![],
            workers: vec![member(2, worker("a"))],
        });
        let name_and_task = Crew::team(TeamMembers {
            coordinator: member(1, normal("a")),
            members: vec![member(1, normal("a"))],
            workers: vec![],
        });

        assert!(matches!(tier_and_name, Err(CrewError::WrongTier { .. })));
        assert_eq!(members_and_name, Err(CrewError::NoMembers));
        assert!(matches!(name_and_task, Err(CrewError::DuplicateAgent { .. })));
    }

    #[test]
    fn a_member_is_found_by_its_task_with_its_role_and_can_be_changed() {
        let mut crew = Crew::team(TeamMembers {
            coordinator: member(1, inv("inv-opus")),
            members: vec![member(2, normal("codex-1"))],
            workers: vec![member(3, worker("worker-1"))],
        })
        .unwrap();

        assert_eq!(
            crew.find(&task(2)).map(|(role, m)| (role, m.agent().name())),
            Some((Role::Member, "codex-1"))
        );
        assert_eq!(crew.find(&task(3)).map(|(role, _)| role), Some(Role::Worker));
        assert_eq!(crew.find(&task(9)), None);
        let (role, mut found) = crew.find_mut(&task(2)).unwrap();
        assert_eq!(role, Role::Member);
        found.transition(TaskStatus::Running).unwrap();
        assert_eq!(crew.find(&task(2)).map(|(_, m)| m.status()), Some(TaskStatus::Running));
        assert!(crew.find_mut(&task(9)).is_none());
        let solo = Crew::solo(member(1, normal("codex-1"))).unwrap();
        assert_eq!(solo.find(&task(1)).map(|(role, _)| role), Some(Role::Solo));
        let mut solo = solo;
        assert!(solo.find_mut(&task(2)).is_none());
        assert_eq!(solo.find_mut(&task(1)).map(|(role, _)| role), Some(Role::Solo));
    }

    #[test]
    fn a_crew_is_written_by_its_kind_and_read_back() {
        let solo = Crew::solo(member(1, normal("codex-1"))).unwrap();
        let team = Crew::team(TeamMembers {
            coordinator: member(1, inv("inv-opus")),
            members: vec![member(2, normal("codex-1"))],
            workers: vec![member(3, worker("worker-1"))],
        })
        .unwrap();

        let solo_value = serde_json::to_value(&solo).unwrap();
        let team_value = serde_json::to_value(&team).unwrap();

        assert_eq!(solo_value["kind"], json!("solo"));
        assert_eq!(
            solo_value["member"]["agent"],
            json!({"tier": "normal", "name": "codex-1"})
        );
        assert_eq!(team_value["kind"], json!("team"));
        assert_eq!(team_value["coordinator"]["agent"]["name"], json!("inv-opus"));
        assert_eq!(team_value["members"].as_array().map(Vec::len), Some(1));
        assert_eq!(team_value["workers"].as_array().map(Vec::len), Some(1));
        assert_eq!(serde_json::from_value::<Crew>(solo_value).unwrap(), solo);
        assert_eq!(serde_json::from_value::<Crew>(team_value).unwrap(), team);
    }

    #[test]
    fn a_crew_that_breaks_the_rules_is_refused_when_read() {
        let one = |n: u32, tier: &str, name: &str| json!({"task": format!("task-20261111-1530{n:02}-k3x9"), "agent": {"tier": tier, "name": name}, "status": "queued"});
        let bad = [
            json!({"kind": "solo", "member": one(1, "worker", "w")}),
            json!({"kind": "team", "coordinator": one(1, "normal", "a"), "members": [], "workers": []}),
            json!({"kind": "team", "coordinator": one(1, "normal", "a"), "members": [one(2, "normal", "a")], "workers": []}),
            json!({"kind": "team", "coordinator": one(1, "normal", "a"), "members": [one(1, "normal", "b")], "workers": []}),
            json!({"kind": "pair", "members": []}),
            json!({"kind": "solo", "member": one(1, "normal", "a"), "extra": 1}),
        ];
        for value in bad {
            assert_refused::<Crew>(&value.clone());
        }
    }

    // ---- the team -----------------------------------------------------------------------------------------------

    #[test]
    fn a_new_team_has_an_item_for_each_item_of_the_task_list_and_the_listed_one_is_done() {
        let team = team(1);

        let progress: Vec<(String, ItemStatus)> =
            team.items().iter().map(|i| (i.id().to_string(), i.status())).collect();

        assert_eq!(
            progress,
            [
                (String::from("1.1"), ItemStatus::Done),
                (String::from("1.2"), ItemStatus::Pending),
                (String::from("2.1"), ItemStatus::Pending),
            ]
        );
        assert_eq!(team.item(item("1.2")).map(ItemProgress::id), Some(item("1.2")));
        assert!(team.item(item("9.9")).is_none());
        assert_eq!(
            ItemProgress::new(item("1.1"), ItemMark::Done),
            *team.item(item("1.1")).unwrap()
        );
    }

    #[test]
    fn a_new_team_has_what_it_was_given_and_no_reviewers() {
        let team = team(1);

        assert_eq!(team.id(), &team_id(1));
        assert_eq!(team.repo(), &repo());
        assert_eq!(team.created_at(), at(0));
        assert_eq!(team.merge(), MergeMode::Autopilot);
        assert_eq!(team.task_list(), &list());
        assert_eq!(team.crew().roster().len(), 3);
        assert_eq!(team.reviewers(), []);
    }

    #[test]
    fn the_progress_of_an_item_can_be_changed_through_the_team() {
        let mut team = team(1);

        team.item_mut(item("1.2"))
            .unwrap()
            .advance(crate::state::ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();

        assert_eq!(
            team.item(item("1.2")).map(ItemProgress::status),
            Some(ItemStatus::InProgress)
        );
        assert!(team.item_mut(item("9.9")).is_none());
    }

    #[test]
    fn a_team_finds_a_task_in_its_crew_or_among_its_reviewers() {
        let mut team = team(1);
        team.add_reviewer(member(7, inv("inv-codex"))).unwrap();

        assert_eq!(team.find(&task(2)).map(|(role, _)| role), Some(Role::Member));
        assert_eq!(team.find(&task(7)).map(|(role, _)| role), Some(Role::Reviewer));
        assert!(team.find(&task(9)).is_none());
        let (role, mut found) = team.find_mut(&task(7)).unwrap();
        assert_eq!(role, Role::Reviewer);
        found.transition(TaskStatus::Running).unwrap();
        assert_eq!(team.reviewers()[0].status(), TaskStatus::Running);
        team.reviewers_mut()[0].transition(TaskStatus::Waiting).unwrap();
        assert_eq!(team.reviewers()[0].status(), TaskStatus::Waiting);
        assert!(team.find_mut(&task(9)).is_none());
        assert_eq!(team.find_mut(&task(1)).map(|(role, _)| role), Some(Role::Coordinator));
    }

    #[test]
    fn a_reviewer_is_an_investigator_outside_the_crew_once() {
        let mut team = team(1);
        team.add_reviewer(member(7, inv("inv-codex"))).unwrap();
        let before = team.clone();

        let cases = [
            (
                member(8, normal("claude-1")),
                ReviewerProblem::NotInvestigator {
                    agent: normal("claude-1"),
                },
            ),
            (
                member(8, worker("worker-9")),
                ReviewerProblem::NotInvestigator {
                    agent: worker("worker-9"),
                },
            ),
            (
                member(8, inv("inv-opus")),
                ReviewerProblem::InCrew { agent: inv("inv-opus") },
            ),
            (
                member(8, inv("codex-1")),
                ReviewerProblem::InCrew { agent: inv("codex-1") },
            ),
            (
                member(8, inv("inv-codex")),
                ReviewerProblem::DuplicateName {
                    agent: inv("inv-codex"),
                },
            ),
            (
                member(7, inv("inv-gemini")),
                ReviewerProblem::DuplicateTask { task: task(7) },
            ),
            (
                member(2, inv("inv-gemini")),
                ReviewerProblem::DuplicateTask { task: task(2) },
            ),
        ];
        for (reviewer, problem) in cases {
            assert_eq!(team.add_reviewer(reviewer).unwrap_err(), problem);
            assert_eq!(team, before);
        }
    }

    // ---- reading and writing a team -----------------------------------------------------------------------------

    /// The sample team with two reviewers, as JSON, to be spoiled.
    fn team_json() -> serde_json::Value {
        let mut team = team(1);
        team.add_reviewer(member(7, inv("inv-codex"))).unwrap();
        team.add_reviewer(member(8, inv("inv-gemini"))).unwrap();
        serde_json::to_value(&team).unwrap()
    }

    #[test]
    fn a_team_round_trips_through_json_with_its_parts_in_their_places() {
        let mut team = team(1);
        team.add_reviewer(member(7, inv("inv-codex"))).unwrap();
        team.open_round("round-20261111-153001-k3x9".parse().unwrap(), Some(item("1.2")), at(1))
            .unwrap();
        team.item_mut(item("1.2"))
            .unwrap()
            .advance(crate::state::ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();

        let value = serde_json::to_value(&team).unwrap();

        assert_eq!(value["id"], json!("team-20261111-153001-t5m8"));
        assert_eq!(value["repo"], json!("team-agents"));
        assert_eq!(value["created_at"], json!("2026-11-11T19:15:00Z"));
        assert_eq!(value["merge"], json!("autopilot"));
        assert_eq!(value["task_list"], json!(LIST));
        assert_eq!(value["items"].as_array().map(Vec::len), Some(3));
        assert_eq!(value["items"][1]["phase"]["phase"], json!("in_progress"));
        assert_eq!(value["crew"]["kind"], json!("team"));
        assert_eq!(value["reviewers"].as_array().map(Vec::len), Some(1));
        assert_eq!(serde_json::from_value::<Team>(value).unwrap(), team);
    }

    #[test]
    fn a_team_without_reviewers_is_read_with_none() {
        let mut value = team_json();
        value.as_object_mut().unwrap().remove("reviewers");

        let team = serde_json::from_value::<Team>(value).unwrap();

        assert_eq!(team.reviewers(), []);
    }

    #[test]
    fn progress_that_does_not_match_the_task_list_is_refused_with_the_first_position_that_differs() {
        let items = |mutate: fn(&mut Vec<serde_json::Value>)| {
            let mut value = team_json();
            let mut list = value["items"].as_array().cloned().unwrap();
            mutate(&mut list);
            value["items"] = json!(list);
            Team::try_from(serde_json::from_value::<TeamWire>(value).unwrap())
        };
        let mismatch = |position, expected: Option<&str>, found: Option<&str>| TeamError::ItemsMismatch {
            team: team_id(1),
            position,
            expected: expected.map(item),
            found: found.map(item),
        };

        assert_eq!(items(|l| drop(l.pop())), Err(mismatch(2, Some("2.1"), None)));
        assert_eq!(items(|l| l.swap(1, 2)), Err(mismatch(1, Some("1.2"), Some("2.1"))));
        assert_eq!(items(|l| drop(l.remove(0))), Err(mismatch(0, Some("1.1"), Some("1.2"))));
        assert_eq!(items(|l| l.push(l[0].clone())), Err(mismatch(3, None, Some("1.1"))));
        assert_eq!(items(Vec::clear), Err(mismatch(0, Some("1.1"), None)));
    }

    #[test]
    fn an_item_the_list_marks_done_is_done_and_one_it_marks_open_is_not_done_listed() {
        let mut listed_but_pending = team_json();
        listed_but_pending["items"][0] = json!({"id": "1.1", "phase": {"phase": "pending"}});
        let mut open_but_listed = team_json();
        open_but_listed["items"][1] = json!({"id": "1.2", "phase": {"phase": "done", "completion": {"how": "listed"}}});

        let first = Team::try_from(serde_json::from_value::<TeamWire>(listed_but_pending).unwrap());
        let second = Team::try_from(serde_json::from_value::<TeamWire>(open_but_listed).unwrap());

        assert_eq!(
            first,
            Err(TeamError::ListedMismatch {
                team: team_id(1),
                item: item("1.1"),
                listed: ItemMark::Done
            })
        );
        assert_eq!(
            second,
            Err(TeamError::ListedMismatch {
                team: team_id(1),
                item: item("1.2"),
                listed: ItemMark::Open
            })
        );
    }

    #[test]
    fn a_listed_item_that_stays_done_by_a_merge_is_not_a_mismatch() {
        let mut value = team_json();
        value["items"][0] = json!({"id": "1.1", "phase": {"phase": "done", "completion": {"how": "listed"}}});

        serde_json::from_value::<Team>(value).unwrap();
    }

    #[test]
    fn reviewers_that_break_the_rules_are_refused_when_read() {
        let investigator = |n: u32, tier: &str, name: &str| json!({"task": format!("task-20261111-1530{n:02}-k3x9"), "agent": {"tier": tier, "name": name}, "status": "queued"});
        let cases = [
            (
                investigator(7, "normal", "claude-1"),
                ReviewerProblem::NotInvestigator {
                    agent: normal("claude-1"),
                },
            ),
            (
                investigator(7, "investigator", "inv-opus"),
                ReviewerProblem::InCrew { agent: inv("inv-opus") },
            ),
            (
                investigator(7, "investigator", "inv-gemini"),
                ReviewerProblem::DuplicateName {
                    agent: inv("inv-gemini"),
                },
            ),
            (
                investigator(8, "investigator", "inv-new"),
                ReviewerProblem::DuplicateTask { task: task(8) },
            ),
            (
                investigator(2, "investigator", "inv-new"),
                ReviewerProblem::DuplicateTask { task: task(2) },
            ),
        ];
        for (reviewer, problem) in cases {
            let mut value = team_json();
            value["reviewers"] = json!([
                investigator(7, "investigator", "inv-codex"),
                investigator(8, "investigator", "inv-gemini"),
                reviewer
            ]);
            if problem
                == (ReviewerProblem::DuplicateName {
                    agent: inv("inv-gemini"),
                })
            {
                value["reviewers"] = json!([investigator(7, "investigator", "inv-gemini"), reviewer]);
            }

            let result = Team::try_from(serde_json::from_value::<TeamWire>(value).unwrap());

            assert_eq!(
                result,
                Err(TeamError::Reviewer {
                    team: team_id(1),
                    reason: problem
                })
            );
        }
    }

    #[test]
    fn a_team_with_a_missing_part_an_unknown_field_or_a_bad_merge_mode_is_refused() {
        for key in ["id", "repo", "created_at", "merge", "task_list", "items", "crew"] {
            let mut value = team_json();
            value.as_object_mut().unwrap().remove(key);

            assert_refused::<Team>(&value);
        }
        let mut extra = team_json();
        extra["extra"] = json!(1);
        let mut merge = team_json();
        merge["merge"] = json!("whenever");
        let mut repo = team_json();
        repo["repo"] = json!("a/b");

        assert_refused::<Team>(&extra);
        assert_refused::<Team>(&merge);
        assert_refused::<Team>(&repo);
    }
}
