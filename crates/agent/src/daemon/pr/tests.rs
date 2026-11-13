//! Which branches are published, and the text of a pull request.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::config::{AbsolutePath, Language};
use agent_core::ids::{BranchName, ItemId};
use agent_core::state::{
    CommitId, Crew, GateOutcome, GateRecord, GateResult, GateTree, ItemPhase, MergeMode, PrNumber, PullRequest,
    TaskListText, Team, TeamMembers,
};
use agent_core::status::TaskStatus;
use agent_runtime::autopilot_prompts::AutopilotPrompts;

use super::{Joining, MAX_TITLE_CHARS, Publication, Publish, PullTextError, plan_publications, pull_text};
use crate::fixtures::{LIST, at, investigator, member, normal, repo, task, team_id};

/// Two features that do not wait for each other, with one item each.
const PARALLEL: &str = "\
Build the things.

1. First
[ ] 1.1 One
    Do one.

2. Second
[ ] 2.1 Two
    Do two.
";

fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

fn branch(text: &str) -> BranchName {
    text.parse().unwrap()
}

fn pr(number: u32, head: &str) -> PullRequest {
    PullRequest {
        number: PrNumber::new(NonZeroU32::new(number).unwrap()),
        branch: branch(head),
        opened_at: at(100),
    }
}

/// A solo team (`claude-1`, task 1) on `list`.
fn solo(list: &str) -> Team {
    Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(list.to_owned()).unwrap(),
        Crew::solo(member(1, normal("claude-1"))).unwrap(),
    )
}

/// A team on `list` led by the coordinator `inv-opus` (task 1) with the members `claude-1` (task 2) and `claude-2`
/// (task 3).
fn led(list: &str) -> Team {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: vec![member(2, normal("claude-1")), member(3, normal("claude-2"))],
        workers: vec![],
    })
    .unwrap();
    Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(list.to_owned()).unwrap(),
        crew,
    )
}

/// Moves the task `n` through `statuses`.
fn go(team: &mut Team, n: u32, statuses: &[TaskStatus]) {
    let (_, mut found) = team.find_mut(&task(n)).unwrap();
    for status in statuses {
        found.transition(*status).unwrap();
    }
}

/// Task `n` ran and waits.
fn waits(team: &mut Team, n: u32) {
    go(team, n, &[TaskStatus::Running, TaskStatus::Waiting]);
}

/// Gives `id` to `assignee`, on `on`.
fn start(team: &mut Team, id: &str, assignee: &str, on: &str) {
    team.item_mut(item(id))
        .unwrap()
        .advance(ItemPhase::InProgress {
            assignee: assignee.parse().unwrap(),
            branch: branch(on),
        })
        .unwrap();
}

/// Puts `id`, which is in progress, in review on `number`.
fn review(team: &mut Team, id: &str, number: u32, head: &str) {
    team.item_mut(item(id))
        .unwrap()
        .advance(ItemPhase::InReview {
            pr: pr(number, head),
            reviews: Vec::new(),
        })
        .unwrap();
}

/// Sends `id`, which is in review with a request for changes, back to `assignee` on `on`.
fn fix(team: &mut Team, id: &str, assignee: &str, on: &str, number: u32, head: &str) {
    use agent_core::state::{Review, ReviewOutcome};
    use agent_core::status::ReviewVerdict;
    let mut found = team.item_mut(item(id)).unwrap();
    let asked = ["inv-codex", "inv-gemini"]
        .iter()
        .zip([20, 21])
        .map(|(who, n)| Review {
            reviewer: who.parse().unwrap(),
            task: task(n),
            requested_at: at(101),
            outcome: ReviewOutcome::Pending,
        })
        .collect();
    found.ask_reviews(asked).unwrap();
    found
        .submit_review(&task(20), ReviewVerdict::RequestChanges, at(102))
        .unwrap();
    found
        .advance(ItemPhase::Fixing {
            assignee: assignee.parse().unwrap(),
            branch: branch(on),
            pr: pr(number, head),
        })
        .unwrap();
}

fn joining(id: &str, on: &str) -> Joining {
    Joining {
        item: item(id),
        branch: branch(on),
    }
}

#[test]
fn a_solo_agent_that_waits_with_its_item_in_progress_publishes_the_branch_of_the_item() {
    let mut team = solo(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-1-1.1");
    waits(&mut team, 1);

    let plan = plan_publications(&team).unwrap();

    assert_eq!(
        plan,
        [Publication {
            head_branch: branch("agents/claude-1/task-1-1.1"),
            publish: Publish::Open {
                items: vec![joining("1.1", "agents/claude-1/task-1-1.1")]
            },
        }]
    );
}

#[test]
fn a_solo_agent_that_is_still_working_publishes_nothing() {
    let mut team = solo(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-1-1.1");
    go(&mut team, 1, &[TaskStatus::Running]);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

#[test]
fn a_team_publishes_the_branch_of_its_coordinator_for_the_item_of_a_member_that_waits() {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    waits(&mut team, 2);
    waits(&mut team, 1);

    let plan = plan_publications(&team).unwrap();

    assert_eq!(
        plan,
        [Publication {
            head_branch: branch(&format!("agents/inv-opus/{}", task(1))),
            publish: Publish::Open {
                items: vec![joining("1.1", "agents/claude-1/task-2-1.1")]
            },
        }]
    );
}

#[test]
fn a_member_that_is_still_working_is_not_published() {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    go(&mut team, 2, &[TaskStatus::Running]);
    waits(&mut team, 1);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

#[test]
fn a_coordinator_that_is_still_working_publishes_nothing() {
    let mut team = led(LIST);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    waits(&mut team, 2);
    go(&mut team, 1, &[TaskStatus::Running]);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

#[test]
fn two_finished_members_go_in_one_pull_request_of_the_coordinators_branch() {
    let mut team = led(PARALLEL);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    start(&mut team, "2.1", "claude-2", "agents/claude-2/task-3-2.1");
    waits(&mut team, 2);
    waits(&mut team, 3);
    waits(&mut team, 1);

    let plan = plan_publications(&team).unwrap();

    assert_eq!(
        plan,
        [Publication {
            head_branch: branch(&format!("agents/inv-opus/{}", task(1))),
            publish: Publish::Open {
                items: vec![
                    joining("1.1", "agents/claude-1/task-2-1.1"),
                    joining("2.1", "agents/claude-2/task-3-2.1")
                ]
            },
        }]
    );
}

#[test]
fn while_a_pull_request_of_the_coordinators_branch_is_in_review_a_finished_item_waits() {
    let mut team = led(PARALLEL);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    let head = format!("agents/inv-opus/{}", task(1));
    review(&mut team, "1.1", 7, &head);
    start(&mut team, "2.1", "claude-2", "agents/claude-2/task-3-2.1");
    waits(&mut team, 2);
    waits(&mut team, 3);
    waits(&mut team, 1);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

#[test]
fn a_fix_pass_that_is_over_updates_the_pull_request_and_the_finished_items_join_it() {
    let mut team = led(PARALLEL);
    let head = format!("agents/inv-opus/{}", task(1));
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    review(&mut team, "1.1", 7, &head);
    fix(&mut team, "1.1", "inv-opus", &head, 7, &head);
    start(&mut team, "2.1", "claude-2", "agents/claude-2/task-3-2.1");
    waits(&mut team, 2);
    waits(&mut team, 3);
    waits(&mut team, 1);

    let plan = plan_publications(&team).unwrap();

    assert_eq!(
        plan,
        [Publication {
            head_branch: branch(&head),
            publish: Publish::Update {
                pr: pr(7, &head),
                fixing: vec![item("1.1")],
                joining: vec![joining("2.1", "agents/claude-2/task-3-2.1")],
            },
        }]
    );
}

#[test]
fn a_fix_pass_the_coordinator_is_still_working_on_publishes_nothing() {
    let mut team = led(LIST);
    let head = format!("agents/inv-opus/{}", task(1));
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-2-1.1");
    review(&mut team, "1.1", 7, &head);
    fix(&mut team, "1.1", "inv-opus", &head, 7, &head);
    waits(&mut team, 2);
    go(&mut team, 1, &[TaskStatus::Running]);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

#[test]
fn a_solo_agent_with_its_first_item_in_review_publishes_the_branch_of_its_second_on_its_own() {
    let mut team = solo(PARALLEL);
    start(&mut team, "1.1", "claude-1", "agents/claude-1/task-1-1.1");
    review(&mut team, "1.1", 7, "agents/claude-1/task-1-1.1");
    start(&mut team, "2.1", "claude-1", "agents/claude-1/task-1-2.1");
    waits(&mut team, 1);

    let plan = plan_publications(&team).unwrap();

    assert_eq!(
        plan,
        [Publication {
            head_branch: branch("agents/claude-1/task-1-2.1"),
            publish: Publish::Open {
                items: vec![joining("2.1", "agents/claude-1/task-1-2.1")]
            },
        }]
    );
}

#[test]
fn items_that_are_pending_approved_or_done_publish_nothing() {
    let team = solo(LIST);

    assert_eq!(plan_publications(&team).unwrap(), []);
}

// ---- the text -------------------------------------------------------------------------------------------------

/// The texts of this repository's `prompts/`.
async fn texts() -> AutopilotPrompts {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    AutopilotPrompts::load(&dir, &Language::default()).await.unwrap()
}

fn gate(outcome: GateOutcome) -> GateRecord {
    GateRecord {
        tree: GateTree::Clean,
        result: GateResult {
            head: "26b65de0123456789abcdef0123456789abcdef0".parse::<CommitId>().unwrap(),
            finished_at: at(90),
            outcome,
            log: AbsolutePath::try_from("/home/dev/agents/tasks/gate-1.log".to_owned()).unwrap(),
        },
    }
}

#[tokio::test]
async fn the_title_names_the_item_and_the_description_has_its_text_its_expected_result_and_its_checks() {
    let team = solo(LIST);

    let text = pull_text(
        &texts().await,
        &team,
        &repo(),
        &[item("1.1")],
        &gate(GateOutcome::Passed),
    )
    .unwrap();

    assert_eq!(text.title, "Item 1.1: The first item");
    assert!(text.body.contains("## 1.1 — The first item"), "{}", text.body);
    assert!(text.body.contains("Do it."), "{}", text.body);
    assert!(text.body.contains("### Expected result\n\n- it works"), "{}", text.body);
    assert!(text.body.contains("### Checks\n\n- run it"), "{}", text.body);
}

#[tokio::test]
async fn the_description_names_the_team_the_repository_and_what_the_gate_said() {
    let team = solo(LIST);

    let text = pull_text(
        &texts().await,
        &team,
        &repo(),
        &[item("1.1")],
        &gate(GateOutcome::Passed),
    )
    .unwrap();

    assert!(text.body.contains(team_id(0).as_str()), "{}", text.body);
    assert!(text.body.contains("team-agents"), "{}", text.body);
    assert!(text.body.contains("passed at 26b65de"), "{}", text.body);
}

#[tokio::test]
async fn an_item_without_an_expected_result_or_checks_has_neither_heading() {
    let team = solo(LIST);

    let text = pull_text(
        &texts().await,
        &team,
        &repo(),
        &[item("1.2")],
        &gate(GateOutcome::Passed),
    )
    .unwrap();

    assert!(text.body.contains("## 1.2 — The second item"), "{}", text.body);
    assert!(!text.body.contains("Expected result"), "{}", text.body);
    assert!(!text.body.contains("### Checks"), "{}", text.body);
}

#[tokio::test]
async fn two_items_share_one_title_and_one_description_in_their_order() {
    let team = solo(LIST);

    let text = pull_text(
        &texts().await,
        &team,
        &repo(),
        &[item("1.1"), item("1.2")],
        &gate(GateOutcome::Passed),
    )
    .unwrap();

    assert_eq!(text.title, "Item 1.1, 1.2: The first item; The second item");
    let first = text.body.find("## 1.1").unwrap();
    let second = text.body.find("## 1.2").unwrap();
    assert!(first < second, "{}", text.body);
}

#[tokio::test]
async fn a_long_title_is_cut_at_a_character_and_stays_on_one_line() {
    let long = format!("{}\n", "ż".repeat(MAX_TITLE_CHARS * 2));
    let list = format!("Build.\n\n1. First\n[ ] 1.1 {long}    Do it.\n");
    let team = solo(&list);

    let text = pull_text(
        &texts().await,
        &team,
        &repo(),
        &[item("1.1")],
        &gate(GateOutcome::Passed),
    )
    .unwrap();

    assert_eq!(text.title.chars().count(), MAX_TITLE_CHARS);
    assert!(!text.title.contains('\n'));
    assert!(text.title.starts_with("Item 1.1: żż"));
}

#[tokio::test]
async fn a_pull_request_for_no_item_or_an_item_the_list_does_not_have_is_refused() {
    let team = solo(LIST);
    let texts = texts().await;
    let gate = gate(GateOutcome::Passed);

    let none = pull_text(&texts, &team, &repo(), &[], &gate).unwrap_err();
    let unknown = pull_text(&texts, &team, &repo(), &[item("9.9")], &gate).unwrap_err();

    assert!(matches!(none, PullTextError::NoItems), "{none:?}");
    assert!(
        matches!(unknown, PullTextError::NoSuchItem { item: found } if found == item("9.9")),
        "{unknown:?}"
    );
}
