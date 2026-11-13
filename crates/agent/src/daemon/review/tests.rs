//! The verdict of a report and the text that is posted.

use agent_core::status::ReviewVerdict;

use super::{MAX_REPORT_CHARS, VerdictProblem, parse_verdict, report_text};

#[test]
fn a_report_that_starts_with_its_verdict_has_that_verdict() {
    assert_eq!(
        parse_verdict("VERDICT: APPROVE\nSUMMARY: fine\nSTATUS: DONE"),
        Ok(ReviewVerdict::Approve)
    );
    assert_eq!(
        parse_verdict("VERDICT: REQUEST_CHANGES\nSUMMARY: not fine\nSTATUS: DONE"),
        Ok(ReviewVerdict::RequestChanges)
    );
}

#[test]
fn what_the_reviewer_said_before_its_report_does_not_matter() {
    let message = "I read the diff and ran the gate.\n\nVERDICT: REQUEST_CHANGES\nSUMMARY: one blocker\nSTATUS: DONE";

    assert_eq!(parse_verdict(message), Ok(ReviewVerdict::RequestChanges));
}

#[test]
fn markup_and_indentation_around_the_verdict_line_are_dropped() {
    for line in [
        "**VERDICT: APPROVE**",
        "**VERDICT:** APPROVE",
        "`VERDICT: APPROVE`",
        "> VERDICT: APPROVE",
        "# VERDICT: APPROVE",
        "- VERDICT: APPROVE",
        "   VERDICT:    APPROVE   ",
        "VERDICT:APPROVE",
    ] {
        assert_eq!(
            parse_verdict(&format!("{line}\nSUMMARY: fine")),
            Ok(ReviewVerdict::Approve),
            "{line}"
        );
    }
}

#[test]
fn the_verdict_may_be_written_in_any_case() {
    assert_eq!(parse_verdict("Verdict: approve"), Ok(ReviewVerdict::Approve));
    assert_eq!(
        parse_verdict("VERDICT: Request_Changes"),
        Ok(ReviewVerdict::RequestChanges)
    );
}

#[test]
fn the_first_verdict_line_decides_and_a_later_one_does_not_overrule_it() {
    let message =
        "VERDICT: REQUEST_CHANGES\nSUMMARY: a blocker\n\nIf it were fixed: VERDICT: APPROVE\nVERDICT: APPROVE";

    assert_eq!(parse_verdict(message), Ok(ReviewVerdict::RequestChanges));
}

#[test]
fn a_report_without_a_verdict_line_has_no_verdict() {
    for message in [
        "",
        "SUMMARY: fine\nSTATUS: DONE",
        "I approve of it.\nSTATUS: DONE",
        "ZVERDICT: APPROVE",
    ] {
        assert_eq!(parse_verdict(message), Err(VerdictProblem::Missing), "{message:?}");
    }
}

#[test]
fn a_verdict_line_with_both_words_or_other_words_is_unreadable_and_says_what_it_found() {
    for (line, found) in [
        ("VERDICT: APPROVE | REQUEST_CHANGES", "APPROVE | REQUEST_CHANGES"),
        ("VERDICT: APPROVE with doubts", "APPROVE with doubts"),
        ("VERDICT: LGTM", "LGTM"),
        ("VERDICT:", ""),
        ("VERDICT: żółć", "żółć"),
    ] {
        assert_eq!(
            parse_verdict(&format!("{line}\nSTATUS: DONE")),
            Err(VerdictProblem::Unreadable {
                found: found.to_owned()
            }),
            "{line}"
        );
    }
}

#[test]
fn an_unreadable_verdict_is_cut_so_that_a_huge_line_does_not_travel() {
    let line = format!("VERDICT: {}", "x".repeat(1000));

    let Err(VerdictProblem::Unreadable { found }) = parse_verdict(&line) else {
        panic!("an unreadable verdict was expected");
    };

    assert_eq!(found.chars().count(), 80);
}

#[test]
fn the_status_line_is_not_part_of_the_report() {
    assert_eq!(
        report_text("VERDICT: APPROVE\nSUMMARY: fine\n\nSTATUS: DONE\n"),
        "VERDICT: APPROVE\nSUMMARY: fine"
    );
    assert_eq!(report_text("VERDICT: APPROVE\n**STATUS: DONE**"), "VERDICT: APPROVE");
}

#[test]
fn a_report_without_a_status_line_is_kept_whole_and_trimmed() {
    assert_eq!(
        report_text("\n  VERDICT: APPROVE\nSUMMARY: fine  \n\n"),
        "VERDICT: APPROVE\nSUMMARY: fine"
    );
    assert_eq!(report_text(""), "");
}

#[test]
fn only_the_last_status_line_is_taken_off() {
    assert_eq!(
        report_text("Do not write STATUS: DONE in the middle.\nVERDICT: APPROVE\nSTATUS: DONE"),
        "Do not write STATUS: DONE in the middle.\nVERDICT: APPROVE"
    );
}

#[test]
fn a_long_report_is_cut_at_a_character() {
    let message = format!("VERDICT: APPROVE\n{}", "ż".repeat(MAX_REPORT_CHARS * 2));

    let report = report_text(&message);

    assert_eq!(report.chars().count(), MAX_REPORT_CHARS);
    assert!(report.starts_with("VERDICT: APPROVE\nżż"));
}

// ---- who reviews ----------------------------------------------------------------------------------------------

mod pool {
    use std::collections::BTreeSet;
    use std::num::NonZeroU32;

    use agent_core::config::Config;
    use agent_core::ids::{InvestigatorName, ItemId};
    use agent_core::state::{
        Crew, ItemPhase, MergeMode, PrNumber, PullRequest, Review, ReviewOutcome, State, TaskListText, Team,
        TeamMembers,
    };
    use agent_core::status::{ReviewVerdict, TaskStatus};
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use crate::daemon::review::{Reviewer, Slot, choose_reviewers, free_reviewers};
    use crate::fixtures::{at, investigator, member, normal, task, team_id};

    /// A configuration with a normal agent and four investigators.
    const CONFIG: &str = r#"
[agents.claude-1]
cli = "claude"
[investigators.inv-a]
cli = "claude"
[investigators.inv-b]
cli = "codex"
[investigators.inv-c]
cli = "gemini"
[investigators.inv-d]
cli = "claude"
[tiers]
normal = ["claude-1"]
[repos.team-agents]
base_branch = "main"
gate = ["true"]
"#;

    const LIST: &str = "Build the thing.\n\n1. First\n[ ] 1.1 One\n    Do one.\n";

    fn config() -> Config {
        Config::from_toml_str(CONFIG).unwrap()
    }

    fn name(text: &str) -> InvestigatorName {
        text.parse().unwrap()
    }

    fn item() -> ItemId {
        "1.1".parse().unwrap()
    }

    /// A team `n` led by the investigator `lead` (task `n * 10 + 1`) with a member `claude-1`; its item 1.1 is pending.
    fn led_by(lead: &str, n: u32) -> Team {
        let crew = Crew::team(TeamMembers {
            coordinator: member(n * 10 + 1, investigator(lead)),
            members: vec![member(n * 10 + 2, normal("claude-1"))],
            workers: vec![],
        })
        .unwrap();
        Team::new(
            team_id(n),
            "team-agents".parse().unwrap(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            crew,
        )
    }

    /// A state with team 0 led by `inv-a`.
    fn state() -> State {
        let mut state = State::new();
        state.insert_team(led_by("inv-a", 0)).unwrap();
        state
    }

    fn names(reviewers: &[Reviewer]) -> Vec<&str> {
        reviewers.iter().map(|reviewer| reviewer.name.as_str()).collect()
    }

    /// Moves task `n` through `statuses`.
    fn go(state: &mut State, n: u32, statuses: &[TaskStatus]) {
        let (_, mut found) = state.find_task_mut(&task(n)).unwrap();
        for status in statuses {
            found.transition(*status).unwrap();
        }
    }

    /// Adds the investigator `who` as the reviewer of `team`, with task `n`.
    fn add_reviewer(state: &mut State, team: u32, who: &str, n: u32) {
        state
            .add_reviewer(&team_id(team), member(n, investigator(who)))
            .unwrap();
    }

    /// Puts item 1.1 of `team` in review with the pending reviews of the tasks `tasks`.
    fn in_review(state: &mut State, team: u32, reviewers: &[(&str, u32)]) {
        let mut found = state.team_mut(&team_id(team)).unwrap();
        let mut progress = found.item_mut(item()).unwrap();
        progress
            .advance(ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/item".parse().unwrap(),
            })
            .unwrap();
        progress
            .advance(ItemPhase::InReview {
                pr: PullRequest {
                    number: PrNumber::new(NonZeroU32::new(7).unwrap()),
                    branch: "agents/inv-a/work".parse().unwrap(),
                    opened_at: at(10),
                },
                reviews: Vec::new(),
            })
            .unwrap();
        let asked = reviewers
            .iter()
            .map(|(who, n)| Review {
                reviewer: name(who),
                task: task(*n),
                requested_at: at(11),
                outcome: ReviewOutcome::Pending,
            })
            .collect();
        progress.ask_reviews(asked).unwrap();
    }

    #[test]
    fn every_investigator_outside_the_crew_is_free_and_new_in_a_team_with_nothing_else_going() {
        let free = free_reviewers(&state(), &config(), &team_id(0));

        assert_eq!(names(&free), ["inv-b", "inv-c", "inv-d"]);
        assert!(free.iter().all(|reviewer| reviewer.slot == Slot::New));
    }

    #[test]
    fn the_coordinator_of_a_team_does_not_review_its_own_pull_requests() {
        let free = free_reviewers(&state(), &config(), &team_id(0));

        assert!(!names(&free).contains(&"inv-a"));
    }

    #[test]
    fn an_investigator_that_leads_another_team_that_is_not_finished_is_busy() {
        let mut state = state();
        state.insert_team(led_by("inv-b", 1)).unwrap();
        go(&mut state, 11, &[TaskStatus::Running, TaskStatus::Waiting]);

        let free = free_reviewers(&state, &config(), &team_id(0));

        assert_eq!(names(&free), ["inv-c", "inv-d"]);
    }

    #[test]
    fn an_investigator_whose_task_is_going_or_about_to_start_or_asking_or_paused_is_busy() {
        for statuses in [
            vec![],
            vec![TaskStatus::Running],
            vec![TaskStatus::Running, TaskStatus::Questions],
            vec![TaskStatus::Paused],
        ] {
            let mut state = state();
            state.insert_team(led_by("inv-b", 1)).unwrap();
            go(&mut state, 11, &statuses);

            let free = free_reviewers(&state, &config(), &team_id(0));

            assert!(!names(&free).contains(&"inv-b"), "{statuses:?}");
        }
    }

    #[test]
    fn an_investigator_whose_task_ended_is_free_again() {
        for statuses in [
            vec![TaskStatus::Running, TaskStatus::Done],
            vec![TaskStatus::Running, TaskStatus::Failed],
            vec![TaskStatus::Stopped],
        ] {
            let mut state = state();
            state.insert_team(led_by("inv-b", 1)).unwrap();
            go(&mut state, 11, &statuses);

            let free = free_reviewers(&state, &config(), &team_id(0));

            assert!(names(&free).contains(&"inv-b"), "{statuses:?}");
        }
    }

    #[test]
    fn an_investigator_whose_review_is_asked_for_in_another_team_is_busy_even_while_it_waits() {
        let mut state = state();
        state.insert_team(led_by("inv-b", 1)).unwrap();
        go(&mut state, 11, &[TaskStatus::Running, TaskStatus::Done]);
        add_reviewer(&mut state, 1, "inv-c", 13);
        go(&mut state, 13, &[TaskStatus::Running, TaskStatus::Waiting]);
        in_review(&mut state, 1, &[("inv-c", 13), ("inv-d", 14)]);

        let free = free_reviewers(&state, &config(), &team_id(0));

        assert_eq!(names(&free), ["inv-b"]);
    }

    #[test]
    fn an_investigator_that_waits_after_a_review_in_another_team_is_free_and_new_here() {
        let mut state = state();
        state.insert_team(led_by("inv-b", 1)).unwrap();
        go(&mut state, 11, &[TaskStatus::Running, TaskStatus::Done]);
        add_reviewer(&mut state, 1, "inv-c", 13);
        go(&mut state, 13, &[TaskStatus::Running, TaskStatus::Waiting]);

        let free = free_reviewers(&state, &config(), &team_id(0));

        assert!(free.contains(&Reviewer {
            name: name("inv-c"),
            slot: Slot::New
        }));
    }

    #[test]
    fn a_review_task_of_the_team_that_waits_with_nothing_to_review_is_used_again() {
        let mut state = state();
        add_reviewer(&mut state, 0, "inv-c", 5);
        go(&mut state, 5, &[TaskStatus::Running, TaskStatus::Waiting]);

        let free = free_reviewers(&state, &config(), &team_id(0));

        assert!(free.contains(&Reviewer {
            name: name("inv-c"),
            slot: Slot::Task(task(5))
        }));
        assert_eq!(names(&free), ["inv-b", "inv-c", "inv-d"]);
    }

    #[test]
    fn a_review_task_of_the_team_with_a_review_to_give_is_busy() {
        let mut state = state();
        add_reviewer(&mut state, 0, "inv-c", 5);
        add_reviewer(&mut state, 0, "inv-d", 6);
        go(&mut state, 5, &[TaskStatus::Running, TaskStatus::Waiting]);
        go(&mut state, 6, &[TaskStatus::Running, TaskStatus::Waiting]);
        in_review(&mut state, 0, &[("inv-c", 5), ("inv-d", 6)]);

        let free = free_reviewers(&state, &config(), &team_id(0));

        assert_eq!(names(&free), ["inv-b"]);
    }

    #[test]
    fn a_review_task_of_the_team_that_ended_is_not_offered_and_no_second_one_is_made_for_its_investigator() {
        for statuses in [
            vec![TaskStatus::Running, TaskStatus::Done],
            vec![TaskStatus::Running, TaskStatus::Failed],
            vec![TaskStatus::Stopped],
        ] {
            let mut state = state();
            add_reviewer(&mut state, 0, "inv-c", 5);
            go(&mut state, 5, &statuses);

            let free = free_reviewers(&state, &config(), &team_id(0));

            assert_eq!(names(&free), ["inv-b", "inv-d"], "{statuses:?}");
        }
    }

    #[test]
    fn a_reviewer_that_gave_its_review_is_free_while_the_other_one_of_the_round_is_still_busy() {
        let mut state = state();
        add_reviewer(&mut state, 0, "inv-c", 5);
        add_reviewer(&mut state, 0, "inv-d", 6);
        go(&mut state, 5, &[TaskStatus::Running, TaskStatus::Waiting]);
        go(&mut state, 6, &[TaskStatus::Running, TaskStatus::Waiting]);
        in_review(&mut state, 0, &[("inv-c", 5), ("inv-d", 6)]);
        {
            let mut found = state.team_mut(&team_id(0)).unwrap();
            found
                .item_mut(item())
                .unwrap()
                .submit_review(&task(5), ReviewVerdict::Approve, at(20))
                .unwrap();
        }

        let free = free_reviewers(&state, &config(), &team_id(0));

        // The pending review of inv-d keeps it busy; inv-c has given its review and is free for another pull request.
        assert!(free.iter().any(|reviewer| reviewer.name == name("inv-c")));
        assert!(!free.iter().any(|reviewer| reviewer.name == name("inv-d")));
    }

    fn pool(count: usize) -> Vec<Reviewer> {
        ["inv-a", "inv-b", "inv-c", "inv-d"]
            .iter()
            .take(count)
            .map(|who| Reviewer {
                name: name(who),
                slot: Slot::New,
            })
            .collect()
    }

    #[test]
    fn two_of_the_free_reviewers_are_drawn() {
        let drawn = choose_reviewers(pool(4), &mut StdRng::seed_from_u64(1)).unwrap();

        assert_eq!(drawn.len(), 2);
        let distinct: BTreeSet<&str> = drawn.iter().map(|reviewer| reviewer.name.as_str()).collect();
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn with_exactly_two_free_both_review() {
        let drawn = choose_reviewers(pool(2), &mut StdRng::seed_from_u64(1)).unwrap();

        let drawn: BTreeSet<&str> = drawn.iter().map(|reviewer| reviewer.name.as_str()).collect();
        assert_eq!(drawn, BTreeSet::from(["inv-a", "inv-b"]));
    }

    #[test]
    fn fewer_than_two_free_reviewers_leave_the_pull_request_waiting() {
        assert_eq!(choose_reviewers(pool(1), &mut StdRng::seed_from_u64(1)), None);
        assert_eq!(choose_reviewers(pool(0), &mut StdRng::seed_from_u64(1)), None);
    }

    #[test]
    fn the_draw_is_a_function_of_the_random_source_and_reaches_every_reviewer() {
        let draw = |seed| choose_reviewers(pool(4), &mut StdRng::seed_from_u64(seed)).unwrap();
        assert_eq!(draw(7), draw(7));

        let mut seen = BTreeSet::new();
        for seed in 0..64 {
            for reviewer in draw(seed) {
                seen.insert(reviewer.name.as_str().to_owned());
            }
        }

        assert_eq!(seen.len(), 4, "{seen:?}");
    }
}
