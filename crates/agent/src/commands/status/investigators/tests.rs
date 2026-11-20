//! What `agent status --investigators` says of each configured investigator: the last two activities in the teams that are
//! not closed, newest first, or that it is free.

use std::num::NonZeroU32;

use agent_core::state::{ActivityEnd, ActivityRequest};

use super::*;
use crate::commands::status::tests::team;
use crate::fixtures::{at, task};

/// An activity of `investigator` in task `n`, about the item `item`, asked `asked` seconds after 19:15:00 UTC and, when
/// `end` is given, finished at `end`. The round it serves is that of `item`.
fn activity(
    investigator: &str,
    n: u32,
    item: u32,
    asked: u32,
    request: &ActivityRequest,
    end: Option<(u32, ActivityEnd)>,
) -> Activity {
    serde_json::from_value(serde_json::json!({
        "investigator": investigator,
        "task": task(n),
        "rounds": [format!("round-20261111-1530{item:02}-q1w2")],
        "request": request,
        "asked_at": at(asked),
        "outcome": match end {
            Some((ended, end)) => serde_json::json!({"status": "finished", "at": at(ended), "end": end}),
            None => serde_json::json!({"status": "working"}),
        },
    }))
    .unwrap()
}

/// The configured investigators, as the names `--investigators` shows.
fn configured(names: &[&str]) -> Vec<InvestigatorName> {
    names.iter().map(|name| name.parse().unwrap()).collect()
}

// Rule: one line for each of the last two activities of each configured investigator in the teams that are not closed,
// the newest first: its task, the kind of the activity, the investigator and the team it serves, then what it did; an
// investigator with no activity there is free.
#[test]
fn the_investigators_show_their_last_two_activities_newest_first_or_are_free() {
    let open = team(
        0,
        false,
        vec![
            activity(
                "inv-codex",
                4,
                1,
                10,
                &ActivityRequest::TeamMember,
                Some((20, ActivityEnd::Reported)),
            ),
            activity(
                "inv-codex",
                21,
                1,
                30,
                &ActivityRequest::FirstReview,
                Some((40, ActivityEnd::Reported)),
            ),
            activity(
                "inv-codex",
                22,
                1,
                50,
                &ActivityRequest::Verification { pass: NonZeroU32::MIN },
                Some((60, ActivityEnd::Reported)),
            ),
            activity("inv-opus", 5, 1, 15, &ActivityRequest::FirstReview, None),
        ],
    );
    let closed = team(
        1,
        true,
        vec![activity(
            "inv-codex",
            23,
            1,
            70,
            &ActivityRequest::FirstReview,
            Some((80, ActivityEnd::Reported)),
        )],
    );

    let text = render(
        &[open, closed],
        &configured(&["inv-codex", "inv-opus", "inv-gemini"]),
        UtcOffset::UTC,
    );

    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "task-20261111-153022-k3x9  reviewer  inv-codex  for team-20261111-153000-t5m8  done at 19:16:00, 2nd review finished",
            "task-20261111-153021-k3x9  reviewer  inv-codex  for team-20261111-153000-t5m8  done at 19:15:40, 1st review finished",
            "task-20261111-153005-k3x9  reviewer  inv-opus  for team-20261111-153000-t5m8  working, 1st review",
            "inv-gemini  free, no activity in an open team",
        ]
    );
}

// Rule: a finished activity says `done at <time>` and what was done, in the words of its kind; one that ended without
// its report says `stopped at <time>` and the reason the daemon kept, as the line of a reviewer does.
#[test]
fn each_kind_of_activity_says_what_it_did_and_an_ended_one_says_why_it_stopped() {
    let cases = [
        (
            "inv-alloc",
            ActivityRequest::TaskAllocation,
            ActivityEnd::Reported,
            "task-allocator  inv-alloc  for team-20261111-153000-t5m8  done at 19:15:30, tasks allocated",
        ),
        (
            "inv-crew",
            ActivityRequest::TeamMember,
            ActivityEnd::Reported,
            "team-member  inv-crew  for team-20261111-153000-t5m8  done at 19:15:30, work in the team finished",
        ),
        (
            "inv-asked",
            ActivityRequest::Investigation,
            ActivityEnd::Reported,
            "investigator  inv-asked  for team-20261111-153000-t5m8  done at 19:15:30, investigation finished",
        ),
        (
            "inv-halted",
            ActivityRequest::FirstReview,
            ActivityEnd::Stopped,
            "reviewer  inv-halted  for team-20261111-153000-t5m8  stopped at 19:15:30, reason: the human stopped it",
        ),
        (
            "inv-hung",
            ActivityRequest::FirstReview,
            ActivityEnd::Failed {
                reason: Some(agent_core::state::RunEnd::Hung),
            },
            "reviewer  inv-hung  for team-20261111-153000-t5m8  stopped at 19:15:30, reason: the run showed no activity for too long",
        ),
    ];
    for (investigator, request, end, said) in cases {
        let open = team(
            0,
            false,
            vec![activity(investigator, 4, 1, 20, &request, Some((30, end)))],
        );

        let text = render(&[open], &configured(&[investigator]), UtcOffset::UTC);

        assert_eq!(text, format!("task-20261111-153004-k3x9  {said}\n"));
    }
}

// Rule: the ordinal counts the reviews the investigator gave: a review of the round that ended without its report is not
// one of them.
#[test]
fn a_review_that_ended_without_its_report_is_not_counted_in_the_ordinal() {
    let open = team(
        0,
        false,
        vec![
            activity(
                "inv-codex",
                21,
                1,
                30,
                &ActivityRequest::FirstReview,
                Some((40, ActivityEnd::Failed { reason: None })),
            ),
            activity(
                "inv-codex",
                22,
                1,
                50,
                &ActivityRequest::Verification { pass: NonZeroU32::MIN },
                Some((60, ActivityEnd::Reported)),
            ),
        ],
    );

    let text = render(&[open], &configured(&["inv-codex"]), UtcOffset::UTC);

    assert_eq!(
        text.lines().next(),
        Some(
            "task-20261111-153022-k3x9  reviewer  inv-codex  for team-20261111-153000-t5m8  done at 19:16:00, 1st review finished"
        )
    );
}

// Rule: passes of one task asked in the same second are told in the order of their passes: the newest pass comes
// first, and a verification that goes is shown before the reports of the earlier passes.
#[test]
fn passes_asked_in_one_second_are_shown_newest_pass_first() {
    let open = team(
        0,
        false,
        vec![
            activity(
                "inv-codex",
                21,
                1,
                20,
                &ActivityRequest::FirstReview,
                Some((30, ActivityEnd::Reported)),
            ),
            activity(
                "inv-codex",
                21,
                1,
                20,
                &ActivityRequest::Verification { pass: NonZeroU32::MIN },
                Some((40, ActivityEnd::Reported)),
            ),
            activity(
                "inv-codex",
                21,
                1,
                20,
                &ActivityRequest::Verification {
                    pass: NonZeroU32::new(2).unwrap(),
                },
                None,
            ),
        ],
    );

    let text = render(&[open], &configured(&["inv-codex"]), UtcOffset::UTC);

    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "task-20261111-153021-k3x9  reviewer  inv-codex  for team-20261111-153000-t5m8  working, 3rd review",
            "task-20261111-153021-k3x9  reviewer  inv-codex  for team-20261111-153000-t5m8  done at 19:15:40, 2nd review finished",
        ]
    );
}
