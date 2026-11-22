//! A member that holds one item after another is handed over to by the runs of the item's own round only: the `DONE` of
//! the first item is no hand-over of the second.

use super::*;
use crate::state::fixtures::drive;
use crate::state::fixtures::{at, round_id, run, task, team};
use crate::state::{NewRun, PhaseKind, RunEnd, Team};
use crate::status::RunOutcome;

/// A team whose member `codex-1` ran once for the round of item 1.2 (ending `DONE` at second 11) and once for the round
/// of item 2.1 (ending `QUESTIONS`), and the runs it has.
fn member_runs() -> Vec<crate::state::Run> {
    let mut team: Team = team(1);
    for (id, item) in [(1, "1.2"), (2, "2.1")] {
        team.open_round(round_id(id), Some(item.parse().unwrap()), at(1))
            .unwrap();
    }
    drive(&mut team, "1.2", PhaseKind::InProgress);
    drive(&mut team, "2.1", PhaseKind::InProgress);
    for (n, round, outcome) in [(1, 1, RunOutcome::Done), (2, 2, RunOutcome::Questions)] {
        team.begin_run(
            &task(2),
            NewRun {
                id: run(n),
                at: at(10 * n),
                session: None,
                rounds: vec![round_id(round)],
            },
        )
        .unwrap();
        let (_, mut member) = team.crew_mut().find_mut(&task(2)).unwrap();
        member.end_run(at(10 * n + 1), RunEnd::Reported { outcome }).unwrap();
    }
    team.crew().find(&task(2)).unwrap().1.runs().to_vec()
}

/// The ids of `runs`, as numbers.
fn ids(runs: &[crate::state::Run]) -> Vec<String> {
    runs.iter().map(|run| run.id().to_string()).collect()
}

#[test]
fn the_runs_of_a_round_are_the_runs_that_worked_for_it() {
    let runs = member_runs();

    assert_eq!(ids(&runs_of_round(&runs, &round_id(1))), [run(1).to_string()]);
    assert_eq!(ids(&runs_of_round(&runs, &round_id(2))), [run(2).to_string()]);
    assert_eq!(runs_of_round(&runs, &round_id(3)), Vec::new());
}

#[test]
fn the_done_of_an_earlier_item_is_no_hand_over_of_the_next_one() {
    let runs = member_runs();
    let (second, first): (ItemId, ItemId) = ("2.1".parse().unwrap(), "1.2".parse().unwrap());
    let facts = |runs: &[crate::state::Run]| Facts::from_runs_before(Held::ByTheMember, Answered::No, runs, at(100));

    // Item 2.1 was never handed over: its run asked a question.
    assert_eq!(
        classify(
            &ReturnLine::returned(&second),
            facts(&runs_of_round(&runs, &round_id(2)))
        ),
        Some(Rejection::NotHandedOver)
    );
    // Item 1.2 was.
    assert_eq!(
        classify(
            &ReturnLine::returned(&first),
            facts(&runs_of_round(&runs, &round_id(1)))
        ),
        Some(Rejection::Counted)
    );
}
