use super::*;
use crate::state::fixtures::{at, drive, inv, member, round_id, task, team, team_id};
use crate::state::{ItemPhase, RoundLife, RunState};

/// The item `text` of the sample task list: 1.1 is done as listed, 1.2 and 2.1 are open.
fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

/// A new run with the ID `n`, started at 5 seconds, naming `rounds`.
fn new_run(n: u32, rounds: &[u32]) -> NewRun {
    NewRun {
        id: crate::state::fixtures::run(n),
        at: at(5),
        session: None,
        rounds: rounds.iter().map(|&round| round_id(round)).collect(),
    }
}

/// The sample team with a round 1 open for item 1.2.
fn team_with_round() -> Team {
    let mut team = team(1);
    team.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    team
}

/// The error of reading `team`: what is wrong with its rounds.
fn read_error(team: &Team) -> RoundsError {
    match check_rounds(team) {
        Err(TeamError::Rounds { team: found, source }) => {
            assert_eq!(&found, team.id());
            source
        }
        other => panic!("expected a refusal of the rounds, got {other:?}"),
    }
}

// ---- opening a round ----------------------------------------------------------------------------------------------

#[test]
fn a_round_is_opened_open_for_an_item_and_found_by_its_id_and_by_its_item() {
    let team = team_with_round();

    let round = team.round(&round_id(1)).unwrap();

    assert_eq!(
        (round.id(), round.item(), round.opened_at()),
        (&round_id(1), Some(item("1.2")), at(1))
    );
    assert!(round.is_open());
    assert_eq!(team.round_of(item("1.2")), Some(round));
    assert_eq!(team.rounds(), std::slice::from_ref(round));
    assert_eq!(team.round_of(item("2.1")), None);
    assert_eq!(team.round(&round_id(2)), None);
    assert_eq!(check_rounds(&team), Ok(()));
}

#[test]
fn rounds_are_kept_in_the_order_they_were_opened_and_the_open_ones_are_listed_in_it() {
    let mut team = team_with_round();
    team.open_round(round_id(2), Some(item("2.1")), at(2)).unwrap();
    team.close_round(&round_id(1), at(3), RoundEnd::Abandoned).unwrap();
    team.open_round(round_id(3), None, at(4)).unwrap_err();

    let all: Vec<&RoundId> = team.rounds().iter().map(Round::id).collect();
    let open: Vec<&RoundId> = team.open_rounds().map(Round::id).collect();

    assert_eq!(all, [&round_id(1), &round_id(2)]);
    assert_eq!(open, [&round_id(2)]);
}

#[test]
fn a_round_is_refused_when_the_team_has_a_round_with_the_id() {
    let mut team = team_with_round();

    let err = team.open_round(round_id(1), Some(item("2.1")), at(2)).unwrap_err();

    assert_eq!(err, OpenRoundError::DuplicateId { round: round_id(1) });
    assert_eq!(team.rounds().len(), 1);
}

#[test]
fn a_round_is_refused_for_an_item_the_team_does_not_have() {
    let mut team = team(1);

    let err = team.open_round(round_id(1), Some(item("9.9")), at(1)).unwrap_err();

    assert_eq!(err, OpenRoundError::UnknownItem { item: item("9.9") });
    assert_eq!(team.rounds(), []);
}

#[test]
fn an_item_has_one_round_even_when_the_first_is_closed() {
    let mut team = team_with_round();

    let open = team.open_round(round_id(2), Some(item("1.2")), at(2)).unwrap_err();
    team.close_round(&round_id(1), at(3), RoundEnd::Abandoned).unwrap();
    let closed = team.open_round(round_id(2), Some(item("1.2")), at(4)).unwrap_err();

    let expected = OpenRoundError::ItemHasRound {
        item: item("1.2"),
        round: round_id(1),
    };
    assert_eq!((open, closed), (expected.clone(), expected));
}

#[test]
fn a_round_is_refused_for_an_item_that_is_done() {
    let mut merged = team(1);
    merged.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    drive(&mut merged, "1.2", PhaseKind::Done);
    let mut unopened = team(2);
    drive_without_round(&mut unopened);

    let listed = merged.open_round(round_id(2), Some(item("1.1")), at(2)).unwrap_err();
    let after_merge = unopened.open_round(round_id(3), Some(item("1.2")), at(2)).unwrap_err();

    assert_eq!(listed, OpenRoundError::ItemDone { item: item("1.1") });
    assert_eq!(after_merge, OpenRoundError::ItemDone { item: item("1.2") });
}

/// Item 1.2 of `team` done as merged without ever having had a round: a state only version 1 could be in.
fn drive_without_round(team: &mut Team) {
    drive(team, "1.2", PhaseKind::Done);
}

#[test]
fn a_round_without_an_item_is_refused_in_a_team_whose_list_has_items() {
    let mut team = team(1);

    let err = team.open_round(round_id(1), None, at(1)).unwrap_err();

    assert_eq!(err, OpenRoundError::NoItemInTeamWithItems);
}

#[test]
fn a_list_without_items_has_one_round_for_its_whole_work_and_no_round_for_an_item() {
    let mut team = team_without_items();

    let for_item = team.open_round(round_id(1), Some(item("1.1")), at(1)).unwrap_err();
    team.open_round(round_id(1), None, at(1)).unwrap();
    let twice = team.open_round(round_id(2), None, at(2)).unwrap_err();

    assert_eq!(for_item, OpenRoundError::UnknownItem { item: item("1.1") });
    assert_eq!(twice, OpenRoundError::WholeWorkTwice { round: round_id(1) });
    assert_eq!(team.rounds().len(), 1);
    assert_eq!(team.rounds().first().and_then(Round::item), None);
    assert_eq!(check_rounds(&team), Ok(()));
}

/// A solo team whose task list has no items.
fn team_without_items() -> Team {
    use crate::state::{Crew, MergeMode, TaskListText};
    Team::new(
        team_id(3),
        crate::state::fixtures::repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Tidy up.\n".to_owned()).unwrap(),
        Crew::solo(member(40, crate::state::fixtures::normal("claude-1"))).unwrap(),
    )
}

// ---- closing a round ----------------------------------------------------------------------------------------------

#[test]
fn a_round_is_closed_with_the_time_and_the_way_it_ended() {
    let mut team = team_with_round();

    team.close_round(&round_id(1), at(9), RoundEnd::Abandoned).unwrap();

    let round = team.round(&round_id(1)).unwrap();
    assert!(!round.is_open());
    assert_eq!(round.ended(), Some(RoundEnd::Abandoned));
    assert_eq!(
        round.life(),
        &RoundLife::Closed {
            at: at(9),
            how: RoundEnd::Abandoned
        }
    );
    assert_eq!(team.open_rounds().count(), 0);
}

#[test]
fn closing_is_refused_for_a_round_that_is_not_there() {
    let mut team = team_with_round();

    let err = team.close_round(&round_id(2), at(9), RoundEnd::Abandoned).unwrap_err();

    assert_eq!(err, CloseRoundError::NoSuchRound { round: round_id(2) });
}

#[test]
fn a_round_cannot_be_closed_before_it_was_opened_and_stays_open() {
    let mut team = team_with_round();

    let err = team.close_round(&round_id(1), at(0), RoundEnd::Abandoned).unwrap_err();

    assert_eq!(
        err,
        CloseRoundError::BeforeOpening(ClosedBeforeOpened {
            round: round_id(1),
            opened_at: at(1),
            at: at(0)
        })
    );
    assert!(team.round(&round_id(1)).unwrap().is_open());
}

#[test]
fn a_round_closed_at_the_moment_it_was_opened_is_closed() {
    let mut team = team_with_round();

    team.close_round(&round_id(1), at(1), RoundEnd::Abandoned).unwrap();

    assert!(!team.round(&round_id(1)).unwrap().is_open());
}

#[test]
fn a_closed_round_is_never_opened_again() {
    let mut team = team_with_round();
    team.close_round(&round_id(1), at(2), RoundEnd::Abandoned).unwrap();
    let before = team.clone();

    for how in RoundEnd::ALL {
        let err = team.close_round(&round_id(1), at(3), *how).unwrap_err();

        assert_eq!(err, CloseRoundError::AlreadyClosed { round: round_id(1) }, "{how}");
    }
    let again = team.open_round(round_id(1), Some(item("1.2")), at(4)).unwrap_err();
    assert_eq!(again, OpenRoundError::DuplicateId { round: round_id(1) });
    assert_eq!(team, before);
}

/// The sample team with item 1.2 driven to `phase` under the round 1.
fn team_at(phase: PhaseKind) -> Team {
    let mut team = team_with_round();
    if phase != PhaseKind::Pending {
        drive(&mut team, "1.2", phase);
    }
    team
}

#[test]
fn a_round_ends_as_merged_only_when_its_item_is_done_as_merged() {
    for phase in [
        PhaseKind::Pending,
        PhaseKind::InProgress,
        PhaseKind::InReview,
        PhaseKind::Approved,
    ] {
        let mut team = team_at(phase);

        let err = team.close_round(&round_id(1), at(60), RoundEnd::Merged).unwrap_err();

        assert_eq!(
            err,
            CloseRoundError::EndDoesNotFit {
                round: round_id(1),
                how: RoundEnd::Merged,
                item: Some(phase)
            },
            "{phase}"
        );
        assert!(team.round(&round_id(1)).unwrap().is_open(), "{phase}");
    }
    let mut done = team_at(PhaseKind::Done);

    done.close_round(&round_id(1), at(60), RoundEnd::Merged).unwrap();

    assert_eq!(done.round(&round_id(1)).unwrap().ended(), Some(RoundEnd::Merged));
}

#[test]
fn a_round_ends_as_approved_when_its_item_is_approved_or_done_as_merged_and_not_earlier() {
    for phase in [PhaseKind::Pending, PhaseKind::InProgress, PhaseKind::InReview] {
        let mut team = team_at(phase);

        let err = team.close_round(&round_id(1), at(60), RoundEnd::Approved).unwrap_err();

        assert_eq!(
            err,
            CloseRoundError::EndDoesNotFit {
                round: round_id(1),
                how: RoundEnd::Approved,
                item: Some(phase)
            },
            "{phase}"
        );
    }
    for phase in [PhaseKind::Approved, PhaseKind::Done] {
        let mut team = team_at(phase);

        team.close_round(&round_id(1), at(60), RoundEnd::Approved).unwrap();

        assert_eq!(
            team.round(&round_id(1)).unwrap().ended(),
            Some(RoundEnd::Approved),
            "{phase}"
        );
    }
}

#[test]
fn a_round_is_abandoned_whatever_its_item_is_doing() {
    for phase in [
        PhaseKind::Pending,
        PhaseKind::InProgress,
        PhaseKind::InReview,
        PhaseKind::Approved,
        PhaseKind::Done,
    ] {
        let mut team = team_at(phase);

        team.close_round(&round_id(1), at(60), RoundEnd::Abandoned).unwrap();

        assert_eq!(
            team.round(&round_id(1)).unwrap().ended(),
            Some(RoundEnd::Abandoned),
            "{phase}"
        );
        assert_eq!(check_rounds(&team), Ok(()), "{phase}");
    }
}

#[test]
fn the_round_of_a_whole_list_cannot_end_as_merged_or_approved_but_is_abandoned_when_the_team_ends() {
    for how in [RoundEnd::Merged, RoundEnd::Approved] {
        let mut team = team_without_items();
        team.open_round(round_id(1), None, at(1)).unwrap();

        let err = team.close_round(&round_id(1), at(2), how).unwrap_err();

        assert_eq!(
            err,
            CloseRoundError::EndDoesNotFit {
                round: round_id(1),
                how,
                item: None
            }
        );
    }
    let mut team = team_without_items();
    team.open_round(round_id(1), None, at(1)).unwrap();

    team.close_round(&round_id(1), at(2), RoundEnd::Abandoned).unwrap();

    assert_eq!(team.round(&round_id(1)).unwrap().ended(), Some(RoundEnd::Abandoned));
}

// ---- moving an item on --------------------------------------------------------------------------------------------

#[test]
fn an_item_leaves_pending_only_with_a_round() {
    let mut team = team(1);
    let start = || ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
    };

    let refused = team.advance_item(item("1.2"), start()).unwrap_err();
    team.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    team.advance_item(item("1.2"), start()).unwrap();

    assert_eq!(refused, AdvanceItemError::NoRound { item: item("1.2") });
    assert_eq!(team.item(item("1.2")).unwrap().phase().kind(), PhaseKind::InProgress);
}

#[test]
fn an_item_with_a_closed_round_goes_on_and_an_unknown_item_is_refused() {
    let mut team = team_with_round();
    team.close_round(&round_id(1), at(2), RoundEnd::Abandoned).unwrap();

    team.advance_item(
        item("1.2"),
        ItemPhase::InProgress {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-2".parse().unwrap(),
        },
    )
    .unwrap();
    let unknown = team.advance_item(item("9.9"), ItemPhase::Pending).unwrap_err();

    assert_eq!(unknown, AdvanceItemError::NoSuchItem { item: item("9.9") });
}

#[test]
fn a_change_of_phase_the_item_does_not_allow_is_refused_with_its_reason() {
    let mut team = team_with_round();

    let err = team
        .advance_item(
            item("1.2"),
            ItemPhase::Done {
                completion: Completion::Merged {
                    pr: crate::state::fixtures::pr(7),
                    at: at(5),
                },
            },
        )
        .unwrap_err();

    assert!(
        matches!(err, AdvanceItemError::Advance(AdvanceError::Transition(_))),
        "{err:?}"
    );
}

// ---- the rounds of a run ------------------------------------------------------------------------------------------

/// The rounds the latest run of task `n` of the sample team names.
fn rounds_of_latest_run(team: &Team, n: u32) -> Vec<RoundId> {
    let (_, member) = team.find(&task(n)).unwrap();
    member.runs().last().unwrap().rounds().to_vec()
}

#[test]
fn a_run_names_the_rounds_it_works_for_and_they_are_kept_in_the_order_given() {
    let mut team = team_with_round();
    team.open_round(round_id(2), Some(item("2.1")), at(2)).unwrap();

    team.begin_run(&task(2), new_run(1, &[2, 1])).unwrap();

    assert_eq!(rounds_of_latest_run(&team, 2), [round_id(2), round_id(1)]);
    assert_eq!(check_rounds(&team), Ok(()));
}

#[test]
fn a_run_may_name_no_round_and_a_reviewer_may_name_one() {
    let mut team = team_with_round();
    team.add_reviewer(member(7, inv("inv-codex"))).unwrap();

    team.begin_run(&task(2), new_run(1, &[])).unwrap();
    team.begin_run(&task(7), new_run(2, &[1])).unwrap();

    assert_eq!(rounds_of_latest_run(&team, 2), []);
    assert_eq!(rounds_of_latest_run(&team, 7), [round_id(1)]);
}

#[test]
fn a_run_naming_a_round_the_team_does_not_have_is_refused_and_nothing_is_recorded() {
    let mut team = team_with_round();
    let before = team.clone();

    let err = team.begin_run(&task(2), new_run(1, &[1, 5])).unwrap_err();

    assert_eq!(err, BeginRunError::UnknownRound { round: round_id(5) });
    assert_eq!(team, before);
}

#[test]
fn a_run_naming_a_round_twice_is_refused_and_nothing_is_recorded() {
    let mut team = team_with_round();
    let before = team.clone();

    let err = team.begin_run(&task(2), new_run(1, &[1, 1])).unwrap_err();

    assert_eq!(
        err,
        BeginRunError::Run(RunError::RoundTwice {
            run: crate::state::fixtures::run(1),
            round: round_id(1)
        })
    );
    assert_eq!(team, before);
}

#[test]
fn a_run_of_a_task_the_team_does_not_have_or_while_another_goes_is_refused() {
    let mut team = team_with_round();
    team.begin_run(&task(2), new_run(1, &[1])).unwrap();

    let unknown = team.begin_run(&task(50), new_run(2, &[1])).unwrap_err();
    let going = team.begin_run(&task(2), new_run(2, &[1])).unwrap_err();

    assert_eq!(unknown, BeginRunError::UnknownTask { task: task(50) });
    assert_eq!(
        going,
        BeginRunError::Run(RunError::AlreadyRunning {
            run: crate::state::fixtures::run(1)
        })
    );
    let (_, member) = team.find(&task(2)).unwrap();
    assert!(matches!(member.runs().last().unwrap().state(), RunState::Running));
}

// ---- reading: what is wrong with the rounds -----------------------------------------------------------------------

#[test]
fn rounds_that_were_made_through_the_operations_are_read_without_complaint() {
    let mut team = team_with_round();
    team.open_round(round_id(2), Some(item("2.1")), at(2)).unwrap();
    drive(&mut team, "1.2", PhaseKind::Done);
    team.close_round(&round_id(1), at(60), RoundEnd::Merged).unwrap();
    team.begin_run(&task(2), new_run(1, &[1, 2])).unwrap();

    assert_eq!(check_rounds(&team), Ok(()));
}

#[test]
fn a_round_id_twice_in_a_team_is_refused_when_read() {
    let mut team = team_with_round();
    let copy = Round::opened(round_id(1), Some(item("2.1")), at(2));
    team.rounds.push(copy);

    assert_eq!(read_error(&team), RoundsError::DuplicateRound { round: round_id(1) });
}

#[test]
fn a_round_of_an_unknown_item_is_refused_when_read() {
    let mut team = team(1);
    team.rounds.push(Round::opened(round_id(1), Some(item("9.9")), at(1)));

    assert_eq!(
        read_error(&team),
        RoundsError::UnknownItem {
            round: round_id(1),
            item: item("9.9")
        }
    );
}

#[test]
fn an_item_with_two_rounds_is_refused_when_read() {
    let mut team = team_with_round();
    team.rounds.push(Round::opened(round_id(2), Some(item("1.2")), at(2)));

    assert_eq!(
        read_error(&team),
        RoundsError::ItemWithTwoRounds {
            item: item("1.2"),
            first: round_id(1),
            second: round_id(2)
        }
    );
}

#[test]
fn a_round_of_a_listed_item_is_refused_when_read() {
    let mut team = team(1);
    team.rounds.push(Round::opened(round_id(1), Some(item("1.1")), at(1)));

    assert_eq!(
        read_error(&team),
        RoundsError::RoundOfListedItem {
            round: round_id(1),
            item: item("1.1")
        }
    );
}

#[test]
fn a_round_without_an_item_is_refused_when_read_unless_it_is_the_one_round_of_a_list_without_items() {
    let mut with_items = team(1);
    with_items.rounds.push(Round::opened(round_id(1), None, at(1)));
    let mut twice = team_without_items();
    twice.rounds.push(Round::opened(round_id(1), None, at(1)));
    twice.rounds.push(Round::opened(round_id(2), None, at(2)));
    let mut once = team_without_items();
    once.rounds.push(Round::opened(round_id(1), None, at(1)));

    assert_eq!(
        read_error(&with_items),
        RoundsError::NoItemInTeamWithItems { round: round_id(1) }
    );
    assert_eq!(
        read_error(&twice),
        RoundsError::WholeWorkTwice {
            first: round_id(2),
            second: round_id(1)
        }
    );
    assert_eq!(check_rounds(&once), Ok(()));
}

#[test]
fn an_item_that_left_pending_without_a_round_is_refused_when_read_in_every_phase() {
    for phase in [
        PhaseKind::InProgress,
        PhaseKind::InReview,
        PhaseKind::Approved,
        PhaseKind::Done,
    ] {
        let mut team = team(1);
        drive(&mut team, "1.2", phase);

        assert_eq!(
            read_error(&team),
            RoundsError::ItemWithoutRound {
                item: item("1.2"),
                phase
            },
            "{phase}"
        );
    }
}

#[test]
fn an_item_that_is_pending_or_listed_needs_no_round() {
    assert_eq!(check_rounds(&team(1)), Ok(()));
    let mut with_round = team_with_round();
    with_round.open_round(round_id(2), Some(item("2.1")), at(2)).unwrap();
    assert_eq!(check_rounds(&with_round), Ok(()));
}

#[test]
fn an_ending_that_does_not_fit_the_item_is_refused_when_read() {
    let mut merged_early = team_at(PhaseKind::InReview);
    merged_early.rounds[0].close(at(60), RoundEnd::Merged).unwrap();
    let mut approved_early = team_at(PhaseKind::InProgress);
    approved_early.rounds[0].close(at(60), RoundEnd::Approved).unwrap();
    let mut whole_work = team_without_items();
    whole_work.rounds.push(Round::opened(round_id(1), None, at(1)));
    whole_work.rounds[0].close(at(2), RoundEnd::Merged).unwrap();

    assert_eq!(
        read_error(&merged_early),
        RoundsError::EndDoesNotFit {
            round: round_id(1),
            how: RoundEnd::Merged,
            item: Some(PhaseKind::InReview)
        }
    );
    assert_eq!(
        read_error(&approved_early),
        RoundsError::EndDoesNotFit {
            round: round_id(1),
            how: RoundEnd::Approved,
            item: Some(PhaseKind::InProgress)
        }
    );
    assert_eq!(
        read_error(&whole_work),
        RoundsError::EndDoesNotFit {
            round: round_id(1),
            how: RoundEnd::Merged,
            item: None
        }
    );
}

#[test]
fn a_run_naming_a_round_its_team_does_not_have_is_refused_when_read_for_a_member_and_for_a_reviewer() {
    let mut member_run = team_with_round();
    member_run.begin_run(&task(2), new_run(1, &[1])).unwrap();
    member_run.rounds.clear();
    let mut reviewer_run = team_with_round();
    reviewer_run.add_reviewer(member(7, inv("inv-codex"))).unwrap();
    reviewer_run.begin_run(&task(7), new_run(2, &[1])).unwrap();
    reviewer_run.rounds.clear();

    for (team, owner, run) in [(&member_run, 2, 1), (&reviewer_run, 7, 2)] {
        assert_eq!(
            read_error(team),
            RoundsError::RunNamesUnknownRound {
                task: task(owner),
                run: crate::state::fixtures::run(run),
                round: round_id(1)
            }
        );
    }
}
