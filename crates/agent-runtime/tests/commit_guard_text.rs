//! What an agent is told when its commit is refused, rendered from the texts of this repository.

#[cfg(test)]
mod commit_guard_text {
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::Language;
    use agent_core::decide::commit_guard::{Refusal, StepRefusal, Taken};
    use agent_core::state::CommitId;
    use agent_runtime::commit_guard::text::{refusal_text, step_refusal_text};
    use agent_runtime::harness_prompts::HarnessPrompts;

    async fn texts() -> HarnessPrompts {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
        HarnessPrompts::load(&dir, &Language::from_str("English").unwrap())
            .await
            .unwrap()
    }

    fn red() -> CommitId {
        "2".repeat(40).parse().unwrap()
    }

    fn edited(path: &str, taken: Taken) -> Refusal {
        Refusal::FrozenTestEdited {
            path: path.parse().unwrap(),
            red: red(),
            taken,
        }
    }

    // Rule: each finding is a line of its own with the file, the RED commit and what was taken, in the frame that says
    // the commit was not made and what to do.
    #[tokio::test]
    async fn each_finding_is_a_line_in_the_frame_that_says_what_to_do() {
        let refusals = [
            edited("tests/a.rs", Taken::Lines(3)),
            edited("tests/b.rs", Taken::WholeFile),
        ];

        let text = refusal_text(&texts().await, &refusals).unwrap();

        assert!(text.starts_with("This commit was not made."), "{text}");
        assert!(text.contains(
            "- `tests/a.rs`: this commit takes 3 line(s) out of it (removes or replaces them); the `RED:` commit 2222222 froze it.\n"
        ), "{text}");
        assert!(text.contains(
            "- `tests/b.rs`: this commit deletes it or replaces it as a binary file; the `RED:` commit 2222222 froze it.\n"
        ), "{text}");
        assert!(text.contains("`TEST-FIX:`"), "{text}");
        assert!(text.contains("Never rewrite a commit that is already pushed"), "{text}");
        assert!(!text.contains("{{"), "{text}");
    }

    // Rule: a TEST-FIX commit that also changes other files names them.
    #[tokio::test]
    async fn a_test_fix_commit_that_changes_other_files_names_them() {
        let refusals = [Refusal::TestFixTouchesOtherFiles {
            others: vec!["src/a.rs".parse().unwrap(), "docs/b.md".parse().unwrap()],
        }];

        let text = refusal_text(&texts().await, &refusals).unwrap();

        assert!(text.contains(
            "- This commit is a `TEST-FIX:` commit and also changes files that are not tests: `src/a.rs`, `docs/b.md`. A `TEST-FIX:` commit holds test files only.\n"
        ), "{text}");
    }
    fn steps() -> Vec<agent_core::ids::ItemId> {
        vec!["1.1".parse().unwrap(), "1.2".parse().unwrap()]
    }

    // Rule: a commit with no trailer is told what the trailer is, and which steps it may name.
    #[tokio::test]
    async fn a_commit_with_no_trailer_is_told_to_add_one_and_which_steps_it_may_name() {
        let refusal = StepRefusal::Missing { steps: steps() };

        let text = step_refusal_text(&texts().await, &refusal).unwrap();

        assert!(text.starts_with("This commit was not made."), "{text}");
        assert!(
            text.contains("The `Step:` lines of this message name: nothing."),
            "{text}"
        );
        assert!(text.contains("The steps of your plan are: `1.1`, `1.2`."), "{text}");
        assert!(text.contains("Step: 1.2"), "{text}");
        assert!(!text.contains("{{"), "{text}");
    }

    // Rule: a commit whose trailer names no step of the plan is told what it named.
    #[tokio::test]
    async fn a_commit_that_names_no_step_of_the_plan_is_told_what_it_named() {
        let refusal = StepRefusal::NotInPlan {
            named: vec!["3.1".to_owned(), "zażółć".to_owned()],
            steps: steps(),
        };

        let text = step_refusal_text(&texts().await, &refusal).unwrap();

        assert!(
            text.contains("The `Step:` lines of this message name: `3.1`, `zażółć`."),
            "{text}"
        );
        assert!(text.contains("The steps of your plan are: `1.1`, `1.2`."), "{text}");
    }
}
