//! The text of a checkpoint: what is meant, what is done and what is left, against the requirements of an item.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use super::*;
use crate::plan::parse_plan;

fn id(text: &str) -> ItemId {
    text.parse().unwrap()
}

fn requirement(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.parse().unwrap(),
        text: text.to_owned(),
    }
}

fn requirements() -> Vec<Requirement> {
    vec![
        requirement("R1", "the first thing works"),
        requirement("R2", "the second thing works"),
        requirement("R3", "the third thing works"),
        requirement("K1", "run the first thing"),
    ]
}

fn plan() -> Plan {
    parse_plan(
        "Do it.\n\n1. Work\n[x] 1.1 First\n    For: R1\n[x] 1.2 Second\n    For: R2\n[ ] 1.3 Third\n    For: R2\n[ ] 1.4 Fourth\n    For: R3\n",
    )
    .unwrap()
}

fn commit(hash: &str, step: Option<&str>) -> CommitLine {
    CommitLine {
        id: hash.to_owned(),
        step: step.map(id),
    }
}

fn finding(number: u32, status: FindingStatus, place: &str, text: &str) -> FindingLine {
    FindingLine {
        number: NonZeroU32::new(number).unwrap(),
        severity: Severity::Major,
        place: place.to_owned(),
        text: text.to_owned(),
        status,
    }
}

fn pull_request() -> PrNumber {
    PrNumber::new(NonZeroU32::new(7).unwrap())
}

/// The facts of the example of the brief: R1 met, R2 partly met, R3 and K1 not started.
struct World {
    requirements: Vec<Requirement>,
    plan: Option<Plan>,
    judgements: BTreeMap<ItemId, JudgementState>,
    commits: Vec<CommitLine>,
    findings: Vec<FindingLine>,
    next: Next,
}

impl World {
    fn example() -> Self {
        Self {
            requirements: requirements(),
            plan: Some(plan()),
            judgements: BTreeMap::from([(id("1.1"), JudgementState::Confirmed)]),
            commits: vec![commit("abc1234", Some("1.1")), commit("def5678", Some("1.2"))],
            findings: vec![
                finding(1, FindingStatus::Open, "crates/a.rs:10", "Fix the reading of R2."),
                finding(2, FindingStatus::Resolved, "crates/b.rs:3", "A typo."),
            ],
            next: Next::Fix {
                pull_request: pull_request(),
            },
        }
    }

    fn content(&self) -> CheckpointContent {
        checkpoint_content(&StateInput {
            description: "Build the thing.",
            item: id("1.3"),
            title: "Say what is meant",
            requirements: &self.requirements,
            plan: self.plan.as_ref(),
            judgements: &self.judgements,
            commits: &self.commits,
            findings: &self.findings,
            passes_passed: 1,
            head: Some("0123abc"),
            next: self.next,
        })
    }
}

// Rule: `meant` is the task's description, the item and its requirements, each with its number and text.
#[test]
fn meant_tells_the_task_the_item_and_every_requirement_with_its_number() {
    assert_eq!(
        World::example().content().meant,
        "Task: Build the thing.\nItem 1.3: Say what is meant\nR1. the first thing works\nR2. the second thing works\n\
         R3. the third thing works\nK1. run the first thing"
    );
}

// Rule: a requirement is met when every step for it that is not given up is done and has a commit with its trailer, partly
// met when some are, and not started when none is.
#[test]
fn done_tells_each_requirement_with_its_state_and_its_evidence() {
    assert_eq!(
        World::example().content().done,
        "Totals: 2 commits, 2 steps\n\
         R1 met — steps 1.1 (confirmed); commits abc1234\n\
         R2 partly met — steps 1.2 (declared); commits def5678\n\
         R3 not started\n\
         K1 not started\n\
         Finding #2 resolved — crates/b.rs:3\n\
         Review passes passed: 1"
    );
}

// Rule: what is left is the requirements not met with their steps not done, the open findings, and the next line.
#[test]
fn left_tells_the_steps_and_findings_that_remain_and_what_comes_next() {
    assert_eq!(
        World::example().content().left,
        "Totals: 2 steps, 1 findings\n\
         R2 partly met — steps left: 1.3 (not done)\n\
         R3 not started — steps left: 1.4 (not done)\n\
         K1 not started — no step names it\n\
         Finding #1 [MAJOR] crates/a.rs:10 — Fix the reading of R2. (R2)\n\
         next: fix the 1 open findings in pull request #7 and answer each one"
    );
}

// Rule: a commit with no trailer, or one that names no step of a requirement, is listed under "not tied to a requirement".
#[test]
fn a_commit_with_no_trailer_is_listed_last_under_not_tied_to_a_requirement() {
    let mut world = World::example();
    world.commits.push(commit("9999999", None));
    world.commits.push(commit("8888888", Some("7.7")));

    let done = world.content().done;

    assert_eq!(
        done.lines().skip(5).collect::<Vec<_>>(),
        [
            "Finding #2 resolved — crates/b.rs:3",
            "Review passes passed: 1",
            "Not tied to a requirement:",
            "commits 9999999, 8888888",
        ]
    );
    assert_eq!(done.lines().next(), Some("Totals: 4 commits, 2 steps"));
}

// Rule: a requirement whose steps are all confirmed is told with its check at the head.
#[test]
fn a_requirement_whose_steps_are_all_confirmed_names_the_head_its_check_passed_at() {
    let mut world = World::example();
    world.requirements = vec![requirement("K1", "run the first thing")];
    world.plan = Some(parse_plan("Do it.\n\n1. Work\n[x] 1.1 First\n    For: K1\n").unwrap());
    world.commits = vec![commit("abc1234", Some("1.1"))];
    world.findings.clear();

    assert_eq!(
        world.content().done,
        "Totals: 1 commits, 1 steps\nK1 met — its check passed at 0123abc\nReview passes passed: 1"
    );
}

// Rule: a step marked done with no commit that names it does not meet its requirement.
#[test]
fn a_done_step_without_a_commit_of_its_trailer_does_not_meet_its_requirement() {
    let mut world = World::example();
    world.commits = vec![commit("abc1234", Some("1.1"))];

    let done = world.content().done;

    assert_eq!(done.lines().nth(2), Some("R2 not started"));
}

// Rule: a plan with no `For:` line is legacy: its steps are listed under "not tied to a requirement", in done and in left.
#[test]
fn the_steps_of_a_legacy_plan_are_listed_under_not_tied_to_a_requirement() {
    let mut world = World::example();
    world.plan = Some(parse_plan("Do it.\n\n1. Work\n[x] 1.1 First\n[ ] 1.2 Second\n").unwrap());
    world.commits.clear();
    world.findings.clear();

    let content = world.content();

    assert_eq!(
        content.done.lines().skip(1).collect::<Vec<_>>(),
        [
            "R1 not started",
            "R2 not started",
            "R3 not started",
            "K1 not started",
            "Review passes passed: 1",
            "Not tied to a requirement:",
            "steps 1.1 (confirmed)",
        ]
    );
    assert_eq!(
        content.left.lines().skip(5).collect::<Vec<_>>(),
        [
            "Not tied to a requirement:",
            "steps left: 1.2 (not done)",
            "next: fix the 0 open findings in pull request #7 and answer each one",
        ]
    );
}

// Rule: a step marked done whose check failed is left, as a mismatch.
#[test]
fn a_step_whose_check_failed_is_left_as_a_mismatch() {
    let mut world = World::example();
    world.judgements.insert(id("1.2"), JudgementState::Mismatch);

    let content = world.content();

    assert_eq!(
        content.left.lines().nth(1),
        Some("R2 not started — steps left: 1.2 (mismatch), 1.3 (not done)")
    );
    assert_eq!(content.done.lines().next(), Some("Totals: 2 commits, 1 steps"));
}

// Rule: a step given up is not asked for.
#[test]
fn a_step_given_up_is_not_asked_for() {
    let mut world = World::example();
    world.plan = Some(
        parse_plan("Do it.\n\n1. Work\n[x] 1.1 First\n    For: R1\n[-] 1.2 Second\n    For: R2\n    Not needed.\n")
            .unwrap(),
    );

    let content = world.content();

    assert_eq!(content.left.lines().nth(1), Some("R2 not started — no step names it"));
}

fn next_line(next: Next) -> String {
    let mut world = World::example();
    world.next = next;
    world.findings.truncate(1);
    world.content().left.lines().last().unwrap().to_owned()
}

// Rule: with no pull request the next line says to hand the round over.
#[test]
fn the_next_line_before_a_pull_request_says_to_hand_the_round_over() {
    assert_eq!(
        next_line(Next::HandOver),
        "next: commit what is finished, run the gate and hand the round over"
    );
}

// Rule: in review the next line says to wait for the reviews of the pass.
#[test]
fn the_next_line_in_review_says_to_wait_for_the_reviews_of_the_pass() {
    assert_eq!(
        next_line(Next::WaitForReviews { pass: 2 }),
        "next: wait for the reviews of pass 2"
    );
}

// Rule: when fixing the next line counts the open findings and names the pull request.
#[test]
fn the_next_line_when_fixing_counts_the_open_findings() {
    let mut world = World::example();
    world.findings = vec![
        finding(1, FindingStatus::Open, "a", "x"),
        finding(2, FindingStatus::Open, "b", "y"),
        finding(3, FindingStatus::NotConfirmed, "c", "z"),
    ];
    world.next = Next::Fix {
        pull_request: pull_request(),
    };

    assert_eq!(
        world.content().left.lines().last(),
        Some("next: fix the 2 open findings in pull request #7 and answer each one")
    );
}

// Rule: when the round waits for the human the next line says to read the human's messages.
#[test]
fn the_next_line_when_the_round_waits_for_the_human_says_to_read_the_messages() {
    assert_eq!(
        next_line(Next::WaitForHuman),
        "next: wait for the human's instruction; read the human's messages"
    );
}

// Rule: an approved pull request waits for its merge.
#[test]
fn the_next_line_of_an_approved_pull_request_says_to_wait_for_the_merge() {
    assert_eq!(next_line(Next::WaitForMerge), "next: wait for the merge");
}

// Rule: a list longer than 40 lines keeps the first 40 and says how many it left out.
#[test]
fn a_list_of_more_than_forty_lines_is_cut_with_the_number_left_out() {
    let mut world = World::example();
    world.findings = (1..=45)
        .map(|number| finding(number, FindingStatus::Open, "place", "text"))
        .collect();

    let left = world.content().left;
    let lines: Vec<&str> = left.lines().collect();

    // The totals line, the three requirements, then 36 findings make the first 40 lines of the list.
    assert_eq!(lines.len(), 1 + 40 + 1 + 1);
    assert_eq!(lines.get(40), Some(&"Finding #37 [MAJOR] place — text"));
    assert_eq!(lines.get(41), Some(&"8 more left out"));
    assert_eq!(lines.first(), Some(&"Totals: 2 steps, 45 findings"));
}

// Rule: the numbers in the first lines are read back for the listing; a content without them has none.
#[test]
fn the_counts_are_read_from_the_first_lines_of_the_parts() {
    let content = World::example().content();

    assert_eq!(
        content.counts(),
        Some(Counts {
            commits: 2,
            steps_done: 2,
            steps_left: 2,
            findings_open: 1,
        })
    );
    let plain = CheckpointContent {
        meant: "m".to_owned(),
        done: "d".to_owned(),
        left: "l".to_owned(),
    };
    assert_eq!(plain.counts(), None);
}
