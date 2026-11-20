//! The texts of the plan of a task — what a solo agent, a member and a coordinator are told, the four corrections of a
//! run sent back and the human's question after the limit — against their golden texts in `fixtures/plan-prompts/`.

#[cfg(test)]
mod harness_plan_prompts {
    use std::path::Path;

    use agent_core::plan::{StepState, parse_plan};
    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};
    use agent_core::questions::QuestionsFile;

    fn read_prompt(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
        .unwrap()
    }

    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/plan-prompts")
                .join(name),
        )
        .unwrap()
    }

    fn render(id: HarnessPrompt, pairs: &[(&str, &str)]) -> String {
        let spec = id.spec();
        let values = pairs.iter().fold(Variables::new(), |values, (name, value)| {
            values.with(name, *value).unwrap()
        });
        spec.render(&read_prompt(spec.path()), &values).unwrap()
    }

    /// The text of the first fenced `text` block of `text`.
    fn example(text: &str) -> String {
        let start = text.find("```text\n").unwrap() + "```text\n".len();
        let end = start + text[start..].find("```").unwrap();
        text[start..end].to_owned()
    }

    #[test]
    fn each_text_of_the_plan_renders_as_its_golden_text() {
        let cases: &[(HarnessPrompt, &[(&str, &str)], &str)] = &[
            (HarnessPrompt::Plan, &[], "plan.txt"),
            (HarnessPrompt::PlanMissing, &[], "plan-missing.txt"),
            (
                HarnessPrompt::PlanUnreadable,
                &[("error", "line 7: the step is not in a stage")],
                "plan-unreadable.txt",
            ),
            (
                HarnessPrompt::PlanUnfinished,
                &[("steps", "1.2, 2.1")],
                "plan-unfinished.txt",
            ),
            (HarnessPrompt::PlanLostStep, &[("steps", "1.3")], "plan-lost-step.txt"),
            (
                HarnessPrompt::NoticePlanQuestion,
                &[
                    ("task", "task-20261120-090000-abcd"),
                    ("agent", "claude-1"),
                    ("count", "3"),
                    ("reason", "steps 1.2 and 2.1 are still to do or in hand — zażółć"),
                ],
                "notice-plan-question.txt",
            ),
        ];

        for (id, pairs, file) in cases {
            assert_eq!(render(*id, pairs), golden(file), "{id:?}");
        }
    }

    #[test]
    fn the_example_of_the_rules_is_a_plan_with_every_state_of_a_step() {
        let text = render(HarnessPrompt::Plan, &[]);

        let plan = parse_plan(&example(&text)).unwrap();

        let states: Vec<StepState> = plan.steps().map(agent_core::plan::Step::state).collect();
        for state in [StepState::Todo, StepState::InHand, StepState::Done, StepState::GivenUp] {
            assert!(states.contains(&state), "{state:?} is not in the example: {states:?}");
        }
    }

    #[test]
    fn the_question_after_the_limit_is_a_questions_file_with_two_answers_and_a_recommendation() {
        let text = render(
            HarnessPrompt::NoticePlanQuestion,
            &[
                ("task", "task-20261120-090000-abcd"),
                ("agent", "claude-1"),
                ("count", "3"),
                ("reason", "there is no plan"),
            ],
        );

        let file = QuestionsFile::parse(&text).unwrap();

        let [entry] = file.entries() else {
            panic!("one question expected: {file:?}");
        };
        assert!(
            entry.body().contains("\na) ") || entry.body().starts_with("a) "),
            "{}",
            entry.body()
        );
        assert!(entry.body().contains("\nb) "), "{}", entry.body());
        assert!(entry.recommendation().is_some());
        assert_eq!(entry.verdict(), None);
    }
}
