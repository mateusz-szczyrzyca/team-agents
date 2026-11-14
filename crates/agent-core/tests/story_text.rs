//! What the text of a story says and in which order: the item in full, the agents by role and how each was chosen, every run
//! with its prompt and its steps, its changes, the gates and checks, the pull request from its opening to its merge, the
//! questions and the end. Long lines and non-ASCII text are kept whole, and what the options leave out is said.

#[cfg(test)]
#[path = "support/story_world.rs"]
mod story_world;

#[cfg(test)]
mod story_text {
    use agent_core::history::{Cut, HistoryEvent, MaxTextBytes, SequencedRecord};
    use agent_core::protocol::{Client, ControlAction, Target};
    use agent_core::state::{AnsweredVia, GateOutcome, GateRecord, GateResult, GateTree};
    use agent_core::story::{PromptView, Selection, StepView, StoryOptions, render};

    use super::story_world::{Written, at, head, number, round, run, task, team};

    const PREVIEW: StoryOptions = StoryOptions {
        prompts: PromptView::Preview,
        steps: StepView::Listed,
    };

    /// A prompt of seven lines.
    const PROMPT: &str = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7";

    /// One team with one round: its run, the checks and the life of its pull request.
    fn solo() -> Vec<SequencedRecord> {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.run(&team(1), &[round(1)], 1, 2, PROMPT, 2);
        written.checked(&team(1), &[round(1)], 20);
        written.pull_request(&team(1), &[round(1)], 30);
        written.records
    }

    fn story(records: &[SequencedRecord], options: StoryOptions) -> String {
        render(records, &Selection::Round(round(1)), options)
    }

    /// The index of the first occurrence of `needle` in `text`.
    fn place(text: &str, needle: &str) -> usize {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not in\n{text}"))
    }

    #[test]
    fn a_story_opens_with_the_round_its_team_and_the_item_in_full() {
        let text = story(&solo(), PREVIEW);

        assert!(
            text.starts_with(&format!("Round {} of team {}\n", round(1), team(1))),
            "{text}"
        );
        assert!(text.contains("Item 1.1: The first item — zażółć\n"), "{text}");
        assert!(text.contains("Do the thing.\nThen the other thing.\n"), "{text}");
        assert!(text.contains("Expected result:\nThe thing is done.\n"), "{text}");
        assert!(text.contains("Checks:\nRun it.\n"), "{text}");
        assert!(
            text.contains("Team started 2026-11-11T19:30:00Z: repo demo, base branch main, merge autopilot"),
            "{text}"
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn the_agents_are_listed_by_role_with_how_each_was_chosen() {
        let text = story(&solo(), PREVIEW);

        assert!(
            text.contains("1. inv-a — coordinator — drawn at random among inv-a, inv-b\n"),
            "{text}"
        );
        assert!(
            text.contains("2. dev-1 — member — named on the command line\n"),
            "{text}"
        );
        assert!(place(&text, "Agents") < place(&text, "1. inv-a"));
    }

    #[test]
    fn a_run_says_who_why_where_how_long_and_how_it_ended() {
        let text = story(&solo(), PREVIEW);

        assert!(
            text.contains(&format!(
                "run {}: dev-1 (normal, claude) as member, because the task was given item 1.1\n",
                run(1)
            )),
            "{text}"
        );
        assert!(text.contains("worktree /work/tree, branch agents/dev-1/1\n"), "{text}");
        assert!(text.contains("took 12s\n"), "{text}");
        assert!(text.contains("command: claude -p <the prompt> --verbose\n"), "{text}");
        assert!(text.contains("ended: reported DONE\n"), "{text}");
        assert!(text.contains("All done.\nSTATUS: DONE\n"), "{text}");
    }

    #[test]
    fn the_prompt_is_its_first_lines_and_its_size_unless_the_whole_is_asked_for() {
        let preview = story(&solo(), PREVIEW);
        let whole = story(
            &solo(),
            StoryOptions {
                prompts: PromptView::Whole,
                steps: StepView::Listed,
            },
        );

        assert!(
            preview.contains(&format!("prompt: 7 lines, {} bytes\n", PROMPT.len())),
            "{preview}"
        );
        assert!(
            preview.contains(
                "      line 1\n      line 2\n      line 3\n      line 4\n      line 5\n      … 2 more lines\n"
            ),
            "{preview}"
        );
        assert!(!preview.contains("line 6"), "{preview}");
        assert!(whole.contains(&format!("{PROMPT}\n")), "{whole}");
    }

    #[test]
    fn the_steps_are_numbered_with_their_kind_and_time_unless_they_are_left_out() {
        let listed = story(&solo(), PREVIEW);
        let left_out = story(
            &solo(),
            StoryOptions {
                prompts: PromptView::Preview,
                steps: StepView::LeftOut,
            },
        );

        assert!(listed.contains("steps (2):\n"), "{listed}");
        assert!(
            listed.contains("      1. 2026-11-11T19:30:04Z message: step 1 of the run\n"),
            "{listed}"
        );
        assert!(
            listed.contains("      2. 2026-11-11T19:30:05Z message: step 2 of the run\n"),
            "{listed}"
        );
        assert!(left_out.contains("steps: 2 (left out)\n"), "{left_out}");
        assert!(!left_out.contains("step 1 of the run"), "{left_out}");
    }

    #[test]
    fn the_changes_of_a_run_follow_it_with_its_commits_files_and_what_it_left_uncommitted() {
        let text = story(&solo(), PREVIEW);

        let changes = place(&text, "changes: head 1111111 → 2222222\n");
        assert!(place(&text, "ended: reported DONE") < changes, "{text}");
        assert!(text.contains("commit 2222222 Add the thing\n"), "{text}");
        assert!(text.contains("added src/new.rs (+10 -0)\n"), "{text}");
        assert!(text.contains("renamed src/old.rs → src/moved.rs (+2 -1)\n"), "{text}");
        assert!(text.contains("modified logo.png (binary)\n"), "{text}");
        assert!(text.contains("uncommitted: deleted notes.txt (+0 -3)\n"), "{text}");
    }

    #[test]
    fn the_checks_and_the_pull_request_are_told_from_its_opening_to_its_merge_with_texts_whole() {
        let text = story(&solo(), PREVIEW);

        let steps = [
            "checks before a review found 1 issue at 2222222:\n  - the round changes production code and adds no test\n  → sent back to the team (pass 1 of 2)\n",
            "pull request #7 opened: branch agents/dev-1/1 at 2222222\n",
            "The body of the pull request.\nSecond line — 日本語.\n",
            "pull request #7 marked ready for review\n",
            "review pass 1 of #7 asked of inv-c",
            "review pass 1 of #7 by inv-c: REQUEST_CHANGES\n",
            "Blocker: no test.\nVERDICT: REQUEST_CHANGES\n",
            "fix pass 1 of 5 on #7 started\n",
            "pull request #7: pushed 3333333\n",
            "review pass 2 of #7 by inv-c: APPROVE\n",
            "pull request #7 approved by both reviews\n",
            "pull request #7 merged as 4444444\n",
            "round {round} closed: merged\n",
        ];
        let mut last = 0;
        for step in steps {
            let step = step.replace("{round}", round(1).as_str());
            let found = place(&text, &step);
            assert!(found >= last, "{step:?} is out of order in\n{text}");
            last = found;
        }
        assert!(!text.contains("team ended"), "{text}");
    }

    #[test]
    fn a_gate_a_question_a_control_and_a_notification_are_told_in_the_order_of_time() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        let gate = HistoryEvent::Gate {
            task: task(1),
            started_at: at(10),
            record: GateRecord {
                tree: GateTree::Clean,
                result: GateResult {
                    head: head('2'),
                    finished_at: at(22),
                    outcome: GateOutcome::Passed,
                    log: "/tmp/gate.log".parse().unwrap(),
                },
            },
        };
        written.event(&team(1), 22, &[round(1)], gate);
        written.event(
            &team(1),
            23,
            &[round(1)],
            HistoryEvent::QuestionAsked {
                task: task(1),
                number: number(1),
                text: "## 1. Which?\n**Verdict:**".to_owned(),
            },
        );
        written.event(
            &team(1),
            24,
            &[round(1)],
            HistoryEvent::QuestionAnswered {
                task: task(1),
                number: number(1),
                text: "## 1. Which?\n**Verdict:** a".to_owned(),
                via: AnsweredVia::SavedFile,
            },
        );
        written.event(
            &team(1),
            25,
            &[round(1)],
            HistoryEvent::Controlled {
                action: ControlAction::Pause,
                target: Target::Task(task(1)),
                by: Client::Cli,
                tasks: vec![task(1)],
            },
        );
        written.event(
            &team(1),
            26,
            &[round(1)],
            HistoryEvent::Notified {
                task: Some(task(1)),
                title: "Questions".to_owned(),
                body: "Please answer.".to_owned(),
            },
        );

        let text = story(&written.records, PREVIEW);

        let told = [
            "gate for task",
            "passed at 2222222",
            "took 12s",
            "asked the human (questions 1)",
            "## 1. Which?\n**Verdict:**\n",
            "answered questions 1 (saved file)",
            "**Verdict:** a\n",
            "paused task",
            "notification sent: Questions",
            "Please answer.\n",
        ];
        let mut last = 0;
        for step in told {
            let found = place(&text, step);
            assert!(found >= last, "{step:?} is out of order in\n{text}");
            last = found;
        }
    }

    #[test]
    fn a_round_without_a_pull_request_says_so() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.run(&team(1), &[round(1)], 1, 2, "p", 0);

        let text = story(&written.records, PREVIEW);

        assert!(text.contains("no pull request recorded for this round\n"), "{text}");
    }

    #[test]
    fn a_text_that_was_cut_at_the_limit_says_which_field() {
        let mut records = solo();
        let mut cut = Cut::default();
        cut.keep("prompt", "too long", MaxTextBytes::new(3).unwrap());
        let run_start = records
            .iter_mut()
            .find(|record| matches!(record.record, agent_core::history::Record::Run(_)))
            .unwrap();
        run_start.cut = cut;

        let text = story(&records, PREVIEW);

        assert!(text.contains("(cut at the history limit: prompt)\n"), "{text}");
    }

    #[test]
    fn non_ascii_text_and_a_line_of_fifty_thousand_characters_are_kept_whole_and_not_wrapped() {
        let long = "é".repeat(50_000);
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.run(&team(1), &[round(1)], 1, 2, &long, 1);
        written.event(
            &team(1),
            30,
            &[round(1)],
            HistoryEvent::PullOpened {
                number: super::story_world::pr(7),
                branch: "agents/dev-1/1".parse().unwrap(),
                head: head('2'),
                title: "日本語".to_owned(),
                body: long.clone(),
            },
        );

        let whole = story(
            &written.records,
            StoryOptions {
                prompts: PromptView::Whole,
                steps: StepView::Listed,
            },
        );
        let preview = story(&written.records, PREVIEW);

        assert!(whole.contains(&format!("\n{long}\n")), "the prompt is not whole");
        assert_eq!(whole.matches(&long).count(), 2, "the prompt and the body");
        assert!(preview.contains(&format!("\n{long}\n")), "the body is not whole");
        assert!(preview.lines().any(|line| line.contains(&long)));
    }

    #[test]
    fn the_story_of_a_team_tells_its_rounds_in_order_and_the_team_start_and_end_once() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.round(&team(1), &round(2), 50);
        written.run(&team(1), &[round(1)], 1, 2, "first", 0);
        written.run(&team(1), &[round(2)], 2, 60, "second", 0);
        written.pull_request(&team(1), &[round(1)], 30);

        let text = render(&written.records, &Selection::Team(team(1)), PREVIEW);

        assert_eq!(text.matches("Team started").count(), 1, "{text}");
        assert!(
            place(&text, &format!("Round {} of team", round(1))) < place(&text, &format!("Round {} of team", round(2))),
            "{text}"
        );
        assert_eq!(text.matches("team ended\n").count(), 1, "{text}");
        assert!(text.contains(&format!("run {}:", run(2))), "{text}");
    }

    #[test]
    fn a_selection_without_records_has_no_story() {
        assert_eq!(render(&solo(), &Selection::Round(round(9)), PREVIEW), "");
        assert_eq!(render(&solo(), &Selection::Team(team(9)), PREVIEW), "");
    }

    #[test]
    fn a_record_the_history_lost_is_told_in_its_place_with_what_and_why() {
        let mut written = Written::default();
        written.team(&team(1), 0);
        written.round(&team(1), &round(1), 1);
        written.event(
            &team(1),
            10,
            &[round(1)],
            HistoryEvent::Lost {
                what: "step".to_owned(),
                cause: "write_disk_full".to_owned(),
                detail: "the disk is full — zażółć".to_owned(),
            },
        );
        written.event(
            &team(1),
            20,
            &[round(1)],
            HistoryEvent::PullReady {
                pr: super::story_world::pr(7),
            },
        );

        let text = story(&written.records, PREVIEW);

        let lost = place(
            &text,
            "a step record is missing (write_disk_full): the disk is full — zażółć\n",
        );
        assert!(lost < place(&text, "pull request #7 marked ready for review"), "{text}");
    }

    #[test]
    fn a_run_that_ended_with_no_changes_recorded_says_so() {
        let mut records = solo();
        records.retain(|record| !matches!(record.record, agent_core::history::Record::Changes(_)));

        let text = story(&records, PREVIEW);

        assert!(text.contains("changes: not recorded\n"), "{text}");
        assert!(!text.contains("changes: head"), "{text}");
    }
}
