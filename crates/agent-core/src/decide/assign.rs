//! Work assignment: which task-list item goes to which member next.
//!
//! The rules, as `docs/architecture.md` states them:
//!
//! - The daemon gives one item to each member at a time. A member is busy while it is the assignee of an item that
//!   is in progress or being fixed; an item in review no longer holds its member.
//! - The items of a feature are taken in the order written: the next starts when every earlier one is merged — with
//!   `--no-merge`, when it is approved by both reviews.
//! - Features run in parallel once the features they depend on are merged: all their items are done. A dependency
//!   is released by a merge only, `--no-merge` or not: an approved item has not reached the base branch the
//!   dependent feature builds on.
//! - Workers take no items: they serve `delegate` requests, the first free one in the configured order
//!   ([`first_free_worker`]).

use std::collections::HashSet;

use crate::ids::{AgentName, ItemId};
use crate::state::{ItemPhase, ItemProgress, MergeMode};
use crate::task_list::{Feature, TaskList};

/// What [`next_assignments`] decides from.
#[derive(Debug, Clone, Copy)]
pub struct AssignmentRequest<'a> {
    /// The team's task list.
    pub list: &'a TaskList,
    /// The progress of every item of the list, as the team's state keeps it.
    pub progress: &'a [ItemProgress],
    /// What happens to an approved pull request: it decides when the next item of a feature may start.
    pub merge: MergeMode,
    /// The members that may take an item now, in the order they are offered one: not paused, stopped or asking. A
    /// member that is busy with an item is left out by [`next_assignments`] itself.
    pub available: &'a [AgentName],
}

/// An item given to a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The item that starts.
    pub item: ItemId,
    /// The member that works on it.
    pub member: AgentName,
}

/// Why no assignment can be decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AssignError {
    /// The progress has no entry for an item of the list; the state keeps one for every item, so this is a state that
    /// does not belong to the list.
    #[error("the progress has no entry for item {item} of the task list")]
    MissingProgress {
        /// The item without progress.
        item: ItemId,
    },
}

/// The items that start now and the members that take them.
///
/// Items that may start are offered to the free members in the order of `request.available`; the item of the
/// feature written first goes to the first free member. Items and members left over wait for the next call.
///
/// # Errors
/// [`AssignError::MissingProgress`] when an item of the list has no progress.
pub fn next_assignments(request: &AssignmentRequest<'_>) -> Result<Vec<Assignment>, AssignError> {
    let items = ItemSets::of(request.progress);
    if let Some(item) = request.list.items().find(|item| !items.known.contains(&item.id)) {
        return Err(AssignError::MissingProgress { item: item.id });
    }
    let features = request.list.features();
    let starting = features
        .iter()
        .filter(|feature| dependencies_merged(feature, features, &items))
        .filter_map(|feature| next_item(feature, &items, request.merge));
    let mut offered = HashSet::new();
    let free = request
        .available
        .iter()
        .filter(|member| !items.busy.contains(*member) && offered.insert(*member));
    Ok(starting
        .zip(free)
        .map(|(item, member)| Assignment {
            item,
            member: member.clone(),
        })
        .collect())
}

/// What the progress of the items says, by item, for the questions [`next_assignments`] asks.
struct ItemSets<'a> {
    /// Every item that has a progress.
    known: HashSet<ItemId>,
    /// The items not started.
    pending: HashSet<ItemId>,
    /// The items approved by their reviews and not merged.
    approved: HashSet<ItemId>,
    /// The items that are done: merged, or listed as done.
    done: HashSet<ItemId>,
    /// The members that are the assignee of an item in progress or being fixed.
    busy: HashSet<&'a AgentName>,
}

impl<'a> ItemSets<'a> {
    /// Sorts the items of `progress` by where they are.
    fn of(progress: &'a [ItemProgress]) -> Self {
        let mut sets = Self {
            known: HashSet::new(),
            pending: HashSet::new(),
            approved: HashSet::new(),
            done: HashSet::new(),
            busy: HashSet::new(),
        };
        for item in progress {
            let id = item.id();
            sets.known.insert(id);
            let set = match item.phase() {
                ItemPhase::Pending => &mut sets.pending,
                ItemPhase::Approved { .. } => &mut sets.approved,
                ItemPhase::Done { .. } => &mut sets.done,
                ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                    sets.busy.insert(assignee);
                    continue;
                }
                ItemPhase::InReview { .. } => continue,
            };
            set.insert(id);
        }
        sets
    }

    /// Whether the item no longer holds back the next one of its feature: merged, or with `--no-merge` approved.
    fn is_settled(&self, item: ItemId, merge: MergeMode) -> bool {
        self.done.contains(&item) || (merge == MergeMode::NoMerge && self.approved.contains(&item))
    }
}

/// Whether every feature `feature` depends on is merged: all its items are done.
fn dependencies_merged(feature: &Feature, features: &[Feature], items: &ItemSets<'_>) -> bool {
    features
        .iter()
        .filter(|other| feature.depends_on.contains(&other.number))
        .all(|dependency| dependency.items.iter().all(|item| items.done.contains(&item.id)))
}

/// The item of `feature` that starts next: the first that is not settled, if it is not started yet.
fn next_item(feature: &Feature, items: &ItemSets<'_>, merge: MergeMode) -> Option<ItemId> {
    let first = feature.items.iter().find(|item| !items.is_settled(item.id, merge))?;
    items.pending.contains(&first.id).then_some(first.id)
}

/// The worker that serves the next `delegate` request: the first of `workers`, in their configured order, that is
/// not in `busy`.
#[must_use]
pub fn first_free_worker<'a>(workers: &'a [AgentName], busy: &[AgentName]) -> Option<&'a AgentName> {
    workers.iter().find(|worker| !busy.contains(worker))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A feature after the other: 1 has three items, 2 depends on 1, 3 depends on nothing, 4 on 2 and 3.
    const LIST: &str = "\
Brief.

1. First
[ ] 1.1 One
[ ] 1.2 Two
[ ] 1.3 Three

2. Second
Depends on: 1
[ ] 2.1 Four
[ ] 2.2 Five

3. Third
Depends on: none
[ ] 3.1 Six

4. Fourth
Depends on: 2, 3
[ ] 4.1 Seven
";

    /// Two independent features, to be worked in parallel.
    const PARALLEL: &str = "\
Brief.

1. First
[ ] 1.1 One
[ ] 1.2 Two

2. Second
[ ] 2.1 Three
[ ] 2.2 Four

3. Third
[ ] 3.1 Five
";

    /// The item `id` as the list of `text` marks it: the progress of every item at the start of a team.
    fn fresh(list: &TaskList) -> Vec<ItemProgress> {
        list.items().map(|item| ItemProgress::new(item.id, item.mark)).collect()
    }

    fn name(text: &str) -> AgentName {
        text.parse().unwrap()
    }

    fn id(text: &str) -> ItemId {
        text.parse().unwrap()
    }

    fn names(texts: &[&str]) -> Vec<AgentName> {
        texts.iter().map(|text| name(text)).collect()
    }

    /// The progress of item `item` in `phase`, which is the JSON of the phase.
    fn progress(item: &str, phase: Value) -> ItemProgress {
        let mut value = json!({"id": item});
        value["phase"] = phase;
        serde_json::from_value(value).unwrap()
    }

    fn pull_request() -> Value {
        json!({"number": 7, "branch": "agents/codex-1/item", "opened_at": "2026-11-11T19:40:00Z"})
    }

    fn review(verdict: &str) -> Value {
        json!({
            "reviewer": "inv-opus",
            "task": "task-20261111-193007-k3x9",
            "requested_at": "2026-11-11T19:41:00Z",
            "outcome": {"status": "submitted", "verdict": verdict, "at": "2026-11-11T19:50:00Z"}
        })
    }

    fn in_progress(item: &str, assignee: &str) -> ItemProgress {
        progress(
            item,
            json!({"phase": "in_progress", "assignee": assignee, "branch": "agents/x/item"}),
        )
    }

    fn in_review(item: &str) -> ItemProgress {
        progress(item, json!({"phase": "in_review", "pr": pull_request(), "reviews": []}))
    }

    fn approved(item: &str) -> ItemProgress {
        progress(
            item,
            json!({"phase": "approved", "pr": pull_request(), "reviews": [review("APPROVE")]}),
        )
    }

    fn merged(item: &str) -> ItemProgress {
        progress(
            item,
            json!({"phase": "done", "completion": {
                "how": "merged", "pr": pull_request(), "at": "2026-11-11T20:00:00Z"
            }}),
        )
    }

    fn fixing(item: &str, assignee: &str) -> ItemProgress {
        serde_json::from_value(json!({
            "id": item,
            "phase": {"phase": "fixing", "assignee": assignee, "branch": "agents/x/item", "pr": pull_request()},
            "fix_passes": [{"requested_by": [review("REQUEST_CHANGES")]}]
        }))
        .unwrap()
    }

    /// Replaces the progress of the item numbered like `new` in `all`.
    fn with(mut all: Vec<ItemProgress>, new: ItemProgress) -> Vec<ItemProgress> {
        let slot = all.iter_mut().find(|p| p.id() == new.id()).unwrap();
        *slot = new;
        all
    }

    /// The assignments of `text` for `progress`, `available` members and `merge`.
    fn decide(
        text: &str,
        progress: &[ItemProgress],
        merge: MergeMode,
        available: &[&str],
    ) -> Result<Vec<Assignment>, AssignError> {
        let list = TaskList::parse(text).unwrap();
        let available = names(available);
        next_assignments(&AssignmentRequest {
            list: &list,
            progress,
            merge,
            available: &available,
        })
    }

    /// The assignments as `(item, member)` pairs of text.
    fn pairs(result: Result<Vec<Assignment>, AssignError>) -> Vec<(String, String)> {
        result
            .unwrap()
            .into_iter()
            .map(|a| (a.item.to_string(), a.member.to_string()))
            .collect()
    }

    fn pair(item: &str, member: &str) -> (String, String) {
        (item.to_owned(), member.to_owned())
    }

    fn start(text: &str) -> Vec<ItemProgress> {
        fresh(&TaskList::parse(text).unwrap())
    }

    // Rule: the first item of the first feature goes to the first available member.
    #[test]
    fn the_first_item_goes_to_the_first_member() {
        let all = start(PARALLEL);
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1"])),
            vec![pair("1.1", "codex-1")]
        );
    }

    // Rule: one item to each member at a time; features run in parallel, in the order written, and the feature
    // written last waits when there are fewer members than features.
    #[test]
    fn independent_features_go_to_different_members_in_the_order_written() {
        let all = start(PARALLEL);
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("1.1", "codex-1"), pair("2.1", "claude-1")]
        );
    }

    // Rule: more members than items that may start: the others get nothing.
    #[test]
    fn members_beyond_the_items_get_nothing() {
        let all = start(LIST);
        // Only features 1 and 3 may start: 2 waits for 1 and 4 for 2 and 3.
        assert_eq!(
            pairs(decide(LIST, &all, MergeMode::Autopilot, &["a", "b", "c", "d"])),
            vec![pair("1.1", "a"), pair("3.1", "b")]
        );
    }

    // Rule: the items of a feature are taken in order: while 1.1 is in progress, 1.2 does not start.
    #[test]
    fn a_feature_gives_one_item_at_a_time() {
        let all = with(start(PARALLEL), in_progress("1.1", "codex-1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["claude-1", "gemini-1"])),
            vec![pair("2.1", "claude-1"), pair("3.1", "gemini-1")]
        );
    }

    // Rule: an item in review, being fixed or approved holds back the next one of its feature (autopilot).
    #[test]
    fn the_next_item_waits_while_the_previous_is_in_review_fixed_or_approved() {
        for previous in [in_review("1.1"), fixing("1.1", "codex-1"), approved("1.1")] {
            let all = with(start(PARALLEL), previous);
            assert_eq!(
                pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["claude-1"])),
                vec![pair("2.1", "claude-1")]
            );
        }
    }

    // Rule: the next item of a feature starts when the previous one is merged.
    #[test]
    fn the_next_item_starts_when_the_previous_is_merged() {
        let all = with(start(PARALLEL), merged("1.1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["claude-1"])),
            vec![pair("1.2", "claude-1")]
        );
    }

    // Rule: with --no-merge the next item starts when the previous one is approved by both reviews.
    #[test]
    fn with_no_merge_the_next_item_starts_when_the_previous_is_approved() {
        let all = with(start(PARALLEL), approved("1.1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::NoMerge, &["claude-1"])),
            vec![pair("1.2", "claude-1")]
        );
    }

    // Rule: with --no-merge an item in review is not approved yet: the next one still waits.
    #[test]
    fn with_no_merge_an_item_in_review_still_holds_the_next() {
        for previous in [
            in_review("1.1"),
            fixing("1.1", "codex-1"),
            in_progress("1.1", "codex-1"),
        ] {
            let all = with(start(PARALLEL), previous);
            assert_eq!(
                pairs(decide(PARALLEL, &all, MergeMode::NoMerge, &["claude-1"])),
                vec![pair("2.1", "claude-1")]
            );
        }
    }

    // Rule: items the list marks done are skipped; the next open item of the feature starts.
    #[test]
    fn items_marked_done_in_the_list_are_skipped() {
        let text = "Brief.\n\n1. First\n[x] 1.1 One\n[ ] 1.2 Two\n[x] 1.3 Three\n[ ] 1.4 Four\n";
        let all = start(text);
        assert_eq!(
            pairs(decide(text, &all, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("1.2", "codex-1")]
        );
        let after = with(all, merged("1.2"));
        assert_eq!(
            pairs(decide(text, &after, MergeMode::Autopilot, &["codex-1"])),
            vec![pair("1.4", "codex-1")]
        );
    }

    // Rule: a feature whose items are all done starts nothing.
    #[test]
    fn a_finished_list_assigns_nothing() {
        let all = vec![
            merged("1.1"),
            merged("1.2"),
            merged("1.3"),
            merged("2.1"),
            merged("2.2"),
            merged("3.1"),
            merged("4.1"),
        ];
        assert_eq!(pairs(decide(LIST, &all, MergeMode::Autopilot, &["codex-1"])), vec![]);
    }

    // Rule: a feature starts when the features it depends on are merged: all their items done.
    #[test]
    fn a_feature_waits_for_the_features_it_depends_on() {
        let one_left = vec![
            merged("1.1"),
            merged("1.2"),
            progress("1.3", json!({"phase": "pending"})),
            ItemProgress::new(id("2.1"), crate::task_list::ItemMark::Open),
            ItemProgress::new(id("2.2"), crate::task_list::ItemMark::Open),
            merged("3.1"),
            ItemProgress::new(id("4.1"), crate::task_list::ItemMark::Open),
        ];
        assert_eq!(
            pairs(decide(LIST, &one_left, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("1.3", "codex-1")]
        );
        let done = with(one_left, merged("1.3"));
        assert_eq!(
            pairs(decide(LIST, &done, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("2.1", "codex-1")]
        );
    }

    // Rule: a feature that depends on two waits for both.
    #[test]
    fn a_feature_with_two_dependencies_waits_for_both() {
        let mut all = start(LIST);
        for done in ["1.1", "1.2", "1.3", "2.1", "2.2"] {
            all = with(all, merged(done));
        }
        // Feature 3 is not merged yet: 4 waits, and 3.1 is the one to start.
        assert_eq!(
            pairs(decide(LIST, &all, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("3.1", "codex-1")]
        );
        let all = with(all, merged("3.1"));
        assert_eq!(
            pairs(decide(LIST, &all, MergeMode::Autopilot, &["codex-1"])),
            vec![pair("4.1", "codex-1")]
        );
    }

    // Rule: a dependency is released by a merge only: with --no-merge an approved item does not release the features
    // that depend on its feature.
    #[test]
    fn an_approved_item_does_not_release_a_dependent_feature() {
        let mut all = start(LIST);
        for done in ["1.1", "1.2"] {
            all = with(all, merged(done));
        }
        let all = with(all, approved("1.3"));
        assert_eq!(
            pairs(decide(LIST, &all, MergeMode::NoMerge, &["codex-1", "claude-1"])),
            vec![pair("3.1", "codex-1")]
        );
    }

    // Rule: one item to each member at a time: a member busy with an item gets none.
    #[test]
    fn a_member_with_an_item_in_progress_is_busy() {
        let all = with(start(PARALLEL), in_progress("1.1", "codex-1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("2.1", "claude-1")]
        );
    }

    // Rule: a member whose item is being fixed is busy as well.
    #[test]
    fn a_member_fixing_an_item_is_busy() {
        let all = with(start(PARALLEL), fixing("1.1", "codex-1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1", "claude-1"])),
            vec![pair("2.1", "claude-1")]
        );
    }

    // Rule: an item in review no longer holds its member, who may take an item of another feature.
    #[test]
    fn a_member_whose_item_is_in_review_is_free() {
        let all = with(start(PARALLEL), in_review("1.1"));
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1"])),
            vec![pair("2.1", "codex-1")]
        );
    }

    // Rule: only the members offered take items: one that is paused or asking is left out by the caller.
    #[test]
    fn only_the_available_members_take_items() {
        let all = start(PARALLEL);
        assert_eq!(pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &[])), vec![]);
    }

    // Rule: a member named twice still gets one item.
    #[test]
    fn a_member_listed_twice_gets_one_item() {
        let all = start(PARALLEL);
        assert_eq!(
            pairs(decide(PARALLEL, &all, MergeMode::Autopilot, &["codex-1", "codex-1"])),
            vec![pair("1.1", "codex-1")]
        );
    }

    // Rule: a progress that lacks an item of the list is an error naming the item.
    #[test]
    fn a_missing_progress_is_refused() {
        let all = start(PARALLEL);
        let short = all.get(..all.len() - 1).unwrap().to_vec();
        assert_eq!(
            decide(PARALLEL, &short, MergeMode::Autopilot, &["codex-1"]),
            Err(AssignError::MissingProgress { item: id("3.1") })
        );
    }

    // Rule: workers take no items; a delegate request goes to the first free worker in the configured order.
    #[test]
    fn a_delegation_goes_to_the_first_free_worker_in_the_configured_order() {
        let workers = names(&["glm", "qwen", "kimi"]);
        assert_eq!(first_free_worker(&workers, &[]), Some(&name("glm")));
        assert_eq!(first_free_worker(&workers, &names(&["glm"])), Some(&name("qwen")));
        assert_eq!(
            first_free_worker(&workers, &names(&["qwen", "glm"])),
            Some(&name("kimi"))
        );
        assert_eq!(first_free_worker(&workers, &names(&["glm", "qwen", "kimi"])), None);
        assert_eq!(first_free_worker(&[], &[]), None);
    }
}
