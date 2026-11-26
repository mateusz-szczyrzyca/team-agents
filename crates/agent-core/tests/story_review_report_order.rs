//! The runs of one review pass are told together in the order they were asked, also when a reviewer's report was
//! recorded before the other reviewer's run started (issue #286).

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use agent_core::history::{
        AgentIdentity, Cut, EventRecord, HistoryEvent, Record, RunStart, RunWhy, SequencedRecord,
    };
    use agent_core::ids::{RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::{PrNumber, Timestamp};
    use agent_core::status::ReviewVerdict;
    use agent_core::story::{PromptView, Selection, StepView, StoryOptions, render};

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

        /// The report of the reviewer of `review`, recorded now.
        fn submit(&mut self, review: &Review) {
            self.add(
                &review.team,
                Record::Event(Box::new(EventRecord {
                    at: self.next_time(),
                    team: review.team.clone(),
                    rounds: vec!["round-20261111-193000-q1w2".parse().unwrap()],
                    event: HistoryEvent::ReviewSubmitted {
                        pr: review.pr,
                        pass: review.pass,
                        reviewer: "inv-reviewer".parse().unwrap(),
                        task: review.task.clone(),
                        verdict: ReviewVerdict::Approve,
                        report: format!("report of {}", review.task),
                    },
                })),
            );
        }

        /// The runs and the reports of both reviewers in the order the text tells them.
        fn told(&self) -> Vec<String> {
            let text = self.text();
            let mut places: Vec<(usize, String)> = Vec::new();
            for n in 1..=2 {
                places.push((text.find(&format!("  run {}:", run(n))).unwrap(), format!("run {n}")));
                if let Some(at) = text.find(&format!("report of {}", Review::new(n).task)) {
                    places.push((at, format!("report {n}")));
                }
            }
            places.sort();
            places.into_iter().map(|(_, label)| label).collect()
        }
    }

    fn run(n: u32) -> RunId {
        format!("run-20261111-1930{n:02}-r7q2").parse().unwrap()
    }

    fn selection() -> Selection {
        Selection::Round("round-20261111-193000-q1w2".parse().unwrap())
    }

    // Rule: the runs of one review pass follow the order of their requests; a report recorded before the other
    // reviewer's run began does not split them, so the story does not depend on which reviewer ended first.
    #[test]
    fn a_report_recorded_before_the_other_run_began_does_not_split_the_runs_of_the_pass() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.submit(&Review::new(2));
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.submit(&Review::new(1));

        assert_eq!(history.told(), ["run 1", "run 2", "report 2", "report 1"]);
    }

    #[test]
    fn runs_that_overlap_are_told_as_before() {
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(1), 1, Role::Reviewer);
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.submit(&Review::new(2));
        history.submit(&Review::new(1));

        assert_eq!(history.told(), ["run 1", "run 2", "report 2", "report 1"]);
    }

    #[test]
    fn a_report_of_another_pass_still_separates_the_runs() {
        let mut other = Review::new(1);
        other.pass = 1;
        let mut history = History::default();
        history.ask(&Review::new(1));
        history.ask(&Review::new(2));
        history.start(&Review::new(2), 2, Role::Reviewer);
        history.submit(&other);
        history.start(&Review::new(1), 1, Role::Reviewer);

        assert_eq!(history.told(), ["run 2", "report 1", "run 1"]);
    }
}
