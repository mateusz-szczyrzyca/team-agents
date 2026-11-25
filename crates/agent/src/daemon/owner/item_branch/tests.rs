//! Which branch a run is put on, and when the worktree is left alone.

use std::path::PathBuf;

use agent_core::ids::{AgentName, BranchName};
use agent_core::state::{Completion, ItemPhase};
use agent_runtime::git::{StatusEntry, TreeState};

use super::{Checkout, decide, target_branch};
use crate::fixtures::pr;

/// A branch name of the tests.
fn branch(name: &str) -> BranchName {
    name.parse().unwrap()
}

/// An agent name of the tests.
fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// An item `assignee` works on, on `name`.
fn in_progress(assignee: &str, name: &str) -> ItemPhase {
    ItemPhase::InProgress {
        assignee: agent(assignee),
        branch: branch(name),
    }
}

/// An item `assignee` fixes, on `name`.
fn fixing(assignee: &str, name: &str) -> ItemPhase {
    ItemPhase::Fixing {
        assignee: agent(assignee),
        branch: branch(name),
        pr: pr(1),
    }
}

/// A tree with one changed path, as `git status --porcelain` shows it.
fn changed(index: char, worktree: char) -> TreeState {
    TreeState::Dirty(vec![StatusEntry {
        index,
        worktree,
        path: PathBuf::from("src/lib.rs"),
    }])
}

// Rule: the item the agent is fixing decides, before any item it only has in progress.
#[test]
fn the_item_being_fixed_is_the_target_before_one_in_progress() {
    let phases = [
        in_progress("claude-1", "agents/claude-1/b"),
        fixing("claude-1", "agents/claude-1/a"),
    ];

    let target = target_branch(phases.iter(), "claude-1");

    assert_eq!(target, Some(&branch("agents/claude-1/a")));
}

// Rule: an item another agent fixes is not this agent's target.
#[test]
fn an_item_another_agent_fixes_is_not_the_target() {
    let phases = [fixing("claude-2", "agents/claude-2/a")];

    assert_eq!(target_branch(phases.iter(), "claude-1"), None);
}

// Rule: the only item in progress of the agent is its target.
#[test]
fn the_only_item_in_progress_is_the_target() {
    let phases = [
        ItemPhase::Pending,
        in_progress("claude-2", "agents/claude-2/c"),
        in_progress("claude-1", "agents/claude-1/b"),
    ];

    let target = target_branch(phases.iter(), "claude-1");

    assert_eq!(target, Some(&branch("agents/claude-1/b")));
}

// Rule: two items in progress name no branch: the harness does not guess.
#[test]
fn two_items_in_progress_name_no_target() {
    let phases = [
        in_progress("claude-1", "agents/claude-1/a"),
        in_progress("claude-1", "agents/claude-1/b"),
    ];

    assert_eq!(target_branch(phases.iter(), "claude-1"), None);
}

// Rule: an agent with nothing in progress or being fixed has no target.
#[test]
fn an_agent_with_no_item_in_progress_or_being_fixed_has_no_target() {
    let phases = [
        ItemPhase::Pending,
        ItemPhase::Done {
            completion: Completion::Listed,
        },
    ];

    assert_eq!(target_branch(phases.iter(), "claude-1"), None);
}

// Rule: the worktree that has the target branch out is left alone, whatever its tree holds.
#[test]
fn the_target_branch_out_stays_even_with_changes() {
    let target = branch("agents/claude-1/a");

    assert_eq!(decide(&target, Some(&target), &TreeState::Clean), Checkout::Stay);
    assert_eq!(decide(&target, Some(&target), &changed(' ', 'M')), Checkout::Stay);
}

// Rule: another branch with a clean tree is switched.
#[test]
fn another_branch_with_a_clean_tree_is_switched() {
    let other = branch("agents/claude-1/b");

    assert_eq!(
        decide(&branch("agents/claude-1/a"), Some(&other), &TreeState::Clean),
        Checkout::Switch
    );
}

// Rule: a detached HEAD is switched like another branch.
#[test]
fn a_detached_head_with_a_clean_tree_is_switched() {
    assert_eq!(
        decide(&branch("agents/claude-1/a"), None, &TreeState::Clean),
        Checkout::Switch
    );
}

// Rule: untracked files are kept by a switch, so they do not stop it.
#[test]
fn untracked_files_do_not_stop_a_switch() {
    let other = branch("agents/claude-1/b");

    assert_eq!(
        decide(&branch("agents/claude-1/a"), Some(&other), &changed('?', '?')),
        Checkout::Switch
    );
}

// Rule: a change to a tracked file, staged or not, stops the switch.
#[test]
fn a_changed_tracked_file_stops_a_switch_staged_or_not() {
    let other = branch("agents/claude-1/b");
    let target = branch("agents/claude-1/a");

    assert_eq!(decide(&target, Some(&other), &changed(' ', 'M')), Checkout::Refuse);
    assert_eq!(decide(&target, Some(&other), &changed('A', ' ')), Checkout::Refuse);
    assert_eq!(decide(&target, None, &changed('M', 'M')), Checkout::Refuse);
}
