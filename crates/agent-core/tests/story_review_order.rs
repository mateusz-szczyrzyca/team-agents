//! Stable text presentation of adjacent parallel reviews without changing the history.

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use agent_core::history::{
        AgentIdentity, Cut, EventRecord, HistoryEvent, Record, RunStart, RunWhy, SequencedRecord,
    };
    use agent_core::ids::{RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::{PrNumber, Timestamp};
    use agent_core::story::{PromptView, Selection, StepView, StoryOptions, json_lines, render};

    /// The identity shared by one review request and all of its runs.
    #[derive(Clone, Debug)]
    struct Review {
        /// Team owning the request.
        team: TeamId,
        /// Pull request being reviewed.
        pr: PrNumber,
        /// Zero-based review pass.
        pass: u32,
        /// Task assigned to the reviewer.
        task: TaskId,
    }

    impl Review {
        fn new(n: u32) -> Self {
            Self {
                team: "team-20261111-193000-t5m8".parse().unwrap(),
                pr: PrNumber::new(NonZeroU32::new(7).unwrap()),
                pass: 0,
                task: format!("task-20261111-1930{n:02}-k3x9").parse().unwrap(),
            }
        }
    }

    /// An in-memory history with distinct times and a common round for selection.
    #[derive(Debug, Default)]
    struct History(Vec<SequencedRecord>);

    impl History {
        fn add(&mut self, team: &TeamId, record: Record) {
            let sequence = u64::try_from(self.0.len()).unwrap() + 1;
            self.0.push(SequencedRecord {
                sequence,
                at: self.next_time(),
                team: team.clone(),
                rounds: vec!["round-20261111-193000-q1w2".parse().unwrap()],
                record,
                cut: Cut::default(),
            });
        }

        fn next_time(&self) -> Timestamp {
            format!("2026-11-11T19:30:{:02}Z", self.0.len() + 1).parse().unwrap()
        }

        fn ask(&mut self, review: &Review) {
            self.add(
                &review.team,
                Record::Event(Box::new(EventRecord {
                    at: self.next_time(),
                    team: review.team.clone(),
                    rounds: vec!["round-20261111-193000-q1w2".parse().unwrap()],
                    event: HistoryEvent::ReviewAsked {
                        pr: review.pr,
                        pass: review.pass,
                        head: "1".repeat(40).parse().unwrap(),
                        reviewer: "inv-reviewer".parse().unwrap(),
                        task: review.task.clone(),
                        among: Vec::new(),
                    },
                })),
            );
        }

        fn start(&mut self, review: &Review, n: u32, role: Role) {
            self.add(
                &review.team,
                Record::Run(Box::new(RunStart {
                    run: run(n),
                    team: review.team.clone(),
                    task: review.task.clone(),
                    at: self.next_time(),
                    agent: AgentIdentity {
                        name: "inv-reviewer".to_owned(),
                        tier: Tier::Investigator,
                        cli: CliKind::Claude,
                        model: None,
                    },
                    role,
                    rounds: vec!["round-20261111-193000-q1w2".parse().unwrap()],
                    why: RunWhy::Review {
                        pr: review.pr,
                        pass: review.pass,
                    },
                    prompt: format!("Review {n} — zażółć"),
                    worktree: "/work/tree".parse().unwrap(),
                    branch: "agents/inv-reviewer/1".parse().unwrap(),
                    session: None,
                })),
            );
        }

        fn text(&self) -> String {
            render(
                &self.0,
                &selection(),
                StoryOptions {
                    prompts: PromptView::Whole,
                    steps: StepView::Listed,
                },
            )
        }

        fn assert_order(&self, expected: &[u32]) {
            let text = self.text();
            let headers: Vec<_> = text.lines().filter(|line| line.contains("  run ")).collect();
            let expected_headers: Vec<_> = expected.iter().map(|n| {
                let start = self.0.iter().find_map(|stored| match &stored.record {
                    Record::Run(start) if start.run == run(*n) => Some(start),
                    _ => None,
                }).unwrap();
                let RunWhy::Review { pr, pass } = start.why else { panic!("review fixture") };
                format!("{}  run {}: inv-reviewer (investigator, claude) as {}, because a review of pull request #{pr}, pass {}",
                    start.at, start.run, start.role, pass + 1)
            }).collect();
            assert_eq!(headers, expected_headers);
            for n in expected {
                assert_eq!(text.matches(&format!("Review {n} — zażółć")).count(), 1);
            }
        }
    }

    fn run(n: u32) -> RunId {
        format!("run-20261111-1930{n:02}-r7q2").parse().unwrap()
    }

    fn selection() -> Selection {
        Selection::Round("round-20261111-193000-q1w2".parse().unwrap())
    }

    #[test]
    fn adjacent_reviews_follow_requests_for_either_start_order() {
        for order in [[1, 2], [2, 1]] {
            let mut history = History::default();
            // Request order deliberately opposes task ID order.
            history.ask(&Review::new(2));
            history.ask(&Review::new(1));
            for n in order {
                history.start(&Review::new(n), n, Role::Reviewer);
            }
            history.assert_order(&[2, 1]);
        }
    }

    #[test]
    fn missing_any_request_preserves_the_entire_groups_start_order() {
        for requests in [vec![], vec![1], vec![2]] {
            let mut history = History::default();
            for n in requests {
                history.ask(&Review::new(n));
            }
            history.start(&Review::new(2), 2, Role::Reviewer);
            history.start(&Review::new(1), 1, Role::Reviewer);
            history.assert_order(&[2, 1]);
        }
    }

    #[test]
    fn repeated_runs_of_a_requested_task_keep_their_relative_order() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(1), 3, Role::Reviewer);
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.start(&Review::new(2), 4, Role::Reviewer);
        history.assert_order(&[3, 1, 2, 4]);
    }

    #[test]
    fn another_team_pull_request_or_pass_breaks_a_group() {
        let mut boundaries = vec![Review::new(3); 3];
        boundaries[0].team = "team-20261111-193001-t5m8".parse().unwrap();
        boundaries[1].pr = PrNumber::new(NonZeroU32::new(8).unwrap());
        boundaries[2].pass = 1;
        for boundary in boundaries {
            let mut history = History::default();
            history.ask(&Review::new(1));
            history.ask(&Review::new(2));
            history.ask(&boundary);
            history.start(&Review::new(2), 2, Role::Reviewer);
            history.start(&boundary, 3, Role::Reviewer);
            history.start(&Review::new(1), 1, Role::Reviewer);
            history.assert_order(&[2, 3, 1]);
        }
    }

    #[test]
    fn a_non_reviewer_run_breaks_a_group() {
        let mut history = History::default();
        for n in 1..=3 {
            history.ask(&Review::new(n));
        }
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(3), 3, Role::Member);
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.assert_order(&[2, 3, 1]);
    }

    #[test]
    fn an_event_keeps_its_position_and_separates_review_groups() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.ask(&Review::new(3));
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.assert_order(&[2, 1]);
        let text = history.text();
        let before = text.find(&format!("  run {}:", run(2))).unwrap();
        let event = text.find("2026-11-11T19:30:04Z  review pass 1 of #7 asked").unwrap();
        let after = text.find(&format!("  run {}:", run(1))).unwrap();
        assert!(before < event && event < after);
    }

    #[test]
    fn requests_from_another_team_pr_or_pass_do_not_order_this_group() {
        let mut requests = vec![Review::new(1); 3];
        requests[0].team = "team-20261111-193001-t5m8".parse().unwrap();
        requests[1].pr = PrNumber::new(NonZeroU32::new(8).unwrap());
        requests[2].pass = 1;
        for request in requests {
            let mut history = History::default();
            history.ask(&request);
            history.ask(&Review::new(2));
            history.start(&Review::new(2), 2, Role::Reviewer);
            history.start(&Review::new(1), 1, Role::Reviewer);
            history.assert_order(&[2, 1]);
        }
    }

    #[test]
    fn requests_with_equal_times_follow_their_recorded_sequence() {
        let mut history = History::default();
        history.ask(&Review::new(2));
        history.ask(&Review::new(1));
        let timestamp = history.0[0].at;
        history.0[1].at = timestamp;
        if let Record::Event(event) = &mut history.0[1].record {
            event.at = timestamp;
        }
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.0.reverse();
        history.assert_order(&[2, 1]);
    }

    #[test]
    fn json_keeps_chronological_records_while_text_reorders_runs() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(1), 1, Role::Reviewer);
        let before = history.0.clone();
        history.assert_order(&[1, 2]);
        let json: Vec<serde_json::Value> = json_lines(&history.0, &selection())
            .into_iter()
            .map(|line| serde_json::from_str(&line).unwrap())
            .collect();
        assert_eq!(
            json.iter()
                .map(|value| value["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(json[2]["record"]["run"], run(2).as_str());
        assert_eq!(json[3]["record"]["run"], run(1).as_str());
        assert_eq!(history.0, before);
    }

    #[test]
    fn empty_and_single_run_histories_keep_their_contents() {
        let mut history = History::default();
        assert_eq!(history.text(), "");
        history.ask(&Review::new(1));
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.assert_order(&[1]);
    }
    #[test]
    fn repeated_request_records_keep_the_first_request_order() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.ask(&Review::new(1));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.assert_order(&[1, 2]);
    }

    #[test]
    fn a_reviewer_run_for_another_reason_breaks_a_group() {
        let mut history = History::default();
        for n in 1..=3 {
            history.ask(&Review::new(n));
        }
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(3), 3, Role::Reviewer);
        if let Record::Run(start) = &mut history.0.last_mut().unwrap().record {
            start.why = RunWhy::WakeUp;
        }
        history.start(&Review::new(1), 1, Role::Reviewer);
        let text = history.text();
        let headers: Vec<_> = text
            .lines()
            .filter(|line| line.contains("  run "))
            .map(|line| line.split("  run ").nth(1).unwrap().split(':').next().unwrap())
            .collect();
        assert_eq!(headers, vec![run(2).as_str(), run(3).as_str(), run(1).as_str()]);
    }
    #[test]
    fn reordered_runs_keep_their_steps_endings_and_timestamps() {
        use agent_core::cli::ActivityEvent;
        use agent_core::history::{RunEndRecord, Step};
        use agent_core::state::RunEnd;
        use agent_core::status::RunOutcome;

        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.start(&Review::new(1), 1, Role::Reviewer);
        for n in [2, 1] {
            history.add(
                &Review::new(n).team,
                Record::Step(Box::new(Step {
                    run: run(n),
                    number: NonZeroU32::new(1).unwrap(),
                    at: history.next_time(),
                    event: ActivityEvent::Text(format!("step for {n} — żółć")),
                })),
            );
            history.add(
                &Review::new(n).team,
                Record::RunEnd(Box::new(RunEndRecord {
                    run: run(n),
                    at: history.next_time(),
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                    final_message: Some(format!("result for {n}")),
                })),
            );
        }
        history.assert_order(&[1, 2]);
        let text = history.text();
        let second = text.find(&format!("  run {}:", run(2))).unwrap();
        for (n, seconds) in [(1, "07"), (2, "05")] {
            let step = format!("2026-11-11T19:30:{seconds}Z message: step for {n} — żółć");
            let result = format!("result for {n}");
            assert_eq!(text.matches(&step).count(), 1);
            assert_eq!(text.matches(&result).count(), 1);
            assert_eq!(text.find(&step).unwrap() < second, n == 1);
            assert_eq!(text.find(&result).unwrap() < second, n == 1);
        }
    }
}
