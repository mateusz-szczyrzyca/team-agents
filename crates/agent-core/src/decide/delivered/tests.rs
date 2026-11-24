use std::str::FromStr;

use super::*;

fn commit(digit: char) -> CommitId {
    CommitId::from_str(&digit.to_string().repeat(40)).unwrap()
}

fn file(path: &str) -> RepoPath {
    RepoPath::try_from(path.to_owned()).unwrap()
}

/// A failed member, a lead that is done with a passed gate, and nothing lacking: the delivered case.
fn delivered<'a>() -> Delivery<'a> {
    Delivery {
        member: TaskStatus::Failed,
        lead_done: true,
        gate_passed: true,
        commits: &[],
        files: &[],
        told: None,
        asked: false,
    }
}

// Rule: the member's branch is part of the lead's and its worktree is clean: the member's task is closed.
#[test]
fn a_failed_member_whose_branch_the_lead_contains_is_released() {
    assert_eq!(delivered_step(&delivered()), Delivered::Release);
}

// Rule: a commit the lead lacks, and was not told of, is told to the lead.
#[test]
fn a_commit_the_lead_lacks_is_told_to_the_lead() {
    let commits = [commit('a')];

    let step = delivered_step(&Delivery {
        commits: &commits,
        ..delivered()
    });

    assert_eq!(step, Delivered::TellLead);
}

// Rule: an uncommitted file the lead was not told of is told to the lead.
#[test]
fn an_uncommitted_file_the_lead_was_not_told_of_is_told_to_the_lead() {
    let files = [file("src/lib.rs")];

    let step = delivered_step(&Delivery {
        files: &files,
        ..delivered()
    });

    assert_eq!(step, Delivered::TellLead);
}

// Rule: an uncommitted file the lead was told of does not hold the round, when no commit is lacking.
#[test]
fn an_uncommitted_file_the_lead_was_told_of_does_not_hold_the_round() {
    let files = [file("src/lib.rs")];

    let step = delivered_step(&Delivery {
        files: &files,
        told: Some(Told {
            commits: &[],
            files: &files,
            answered: true,
        }),
        ..delivered()
    });

    assert_eq!(step, Delivered::Release);
}

// Rule: the lead ended again without the commits it was told of: the human is asked.
#[test]
fn a_lead_that_ended_again_without_the_commits_it_was_told_of_has_the_human_asked() {
    let commits = [commit('a'), commit('b')];

    let step = delivered_step(&Delivery {
        commits: &commits,
        told: Some(Told {
            commits: &commits,
            files: &[],
            answered: true,
        }),
        ..delivered()
    });

    assert_eq!(step, Delivered::AskHuman);
}

// Rule: a lead that was told and has not ended again is waited for.
#[test]
fn a_lead_that_was_told_and_has_not_ended_again_is_waited_for() {
    let commits = [commit('a')];

    let step = delivered_step(&Delivery {
        commits: &commits,
        told: Some(Told {
            commits: &commits,
            files: &[],
            answered: false,
        }),
        ..delivered()
    });

    assert_eq!(step, Delivered::Nothing);
}

// Rule: the human is asked once.
#[test]
fn the_human_is_asked_once() {
    let commits = [commit('a')];

    let step = delivered_step(&Delivery {
        commits: &commits,
        told: Some(Told {
            commits: &commits,
            files: &[],
            answered: true,
        }),
        asked: true,
        ..delivered()
    });

    assert_eq!(step, Delivered::Nothing);
}

// Rule: a commit that came after the lead was told is told to it too.
#[test]
fn a_commit_that_came_after_the_lead_was_told_is_told_to_it() {
    let told = [commit('a')];
    let commits = [commit('a'), commit('b')];

    let step = delivered_step(&Delivery {
        commits: &commits,
        told: Some(Told {
            commits: &told,
            files: &[],
            answered: true,
        }),
        ..delivered()
    });

    assert_eq!(step, Delivered::TellLead);
}

// Rule: only a failed member is looked at: the others are never bypassed, and the human's stop is not this decision.
#[test]
fn a_member_that_did_not_fail_is_never_bypassed() {
    for member in [
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Waiting,
        TaskStatus::Questions,
        TaskStatus::Paused,
        TaskStatus::Done,
        TaskStatus::Stopped,
    ] {
        let step = delivered_step(&Delivery { member, ..delivered() });

        assert_eq!(step, Delivered::Nothing, "member {member:?}");
    }
}

// Rule: nothing is decided before the lead is done with a passed gate.
#[test]
fn nothing_is_decided_before_the_lead_is_done_with_a_passed_gate() {
    for (lead_done, gate_passed) in [(false, false), (false, true), (true, false)] {
        let step = delivered_step(&Delivery {
            lead_done,
            gate_passed,
            ..delivered()
        });

        assert_eq!(step, Delivered::Nothing, "done {lead_done}, gate {gate_passed}");
    }
}

// Rule: a member the human stopped is looked at for work the lead lacks as a failed one is: a commit the lead lacks is told.
#[test]
fn a_stopped_member_with_a_commit_the_lead_lacks_has_it_told_to_the_lead() {
    let commits = [commit('a')];

    let step = delivered_step(&Delivery {
        member: TaskStatus::Stopped,
        commits: &commits,
        ..delivered()
    });

    assert_eq!(step, Delivered::TellLead);
}

// Rule: an uncommitted file of a stopped member that the lead was not told of is told to it.
#[test]
fn a_stopped_member_with_an_uncommitted_file_the_lead_was_not_told_of_has_it_told_to_the_lead() {
    let files = [file("src/lib.rs")];

    let step = delivered_step(&Delivery {
        member: TaskStatus::Stopped,
        files: &files,
        ..delivered()
    });

    assert_eq!(step, Delivered::TellLead);
}

// Rule: a stopped member whose work the lead holds is closed already: nothing is done, and its file the lead was told of
// does not hold the round.
#[test]
fn a_stopped_member_whose_work_the_lead_holds_needs_nothing() {
    let files = [file("src/lib.rs")];

    let clean = delivered_step(&Delivery {
        member: TaskStatus::Stopped,
        ..delivered()
    });
    let told = delivered_step(&Delivery {
        member: TaskStatus::Stopped,
        files: &files,
        told: Some(Told {
            commits: &[],
            files: &files,
            answered: true,
        }),
        ..delivered()
    });

    assert_eq!((clean, told), (Delivered::Nothing, Delivered::Nothing));
}

// Rule: the lead ended again without the commits of a stopped member it was told of: the human is asked.
#[test]
fn a_lead_that_ended_again_without_the_commits_of_a_stopped_member_has_the_human_asked() {
    let commits = [commit('a')];

    let step = delivered_step(&Delivery {
        member: TaskStatus::Stopped,
        commits: &commits,
        told: Some(Told {
            commits: &commits,
            files: &[],
            answered: true,
        }),
        ..delivered()
    });

    assert_eq!(step, Delivered::AskHuman);
}
