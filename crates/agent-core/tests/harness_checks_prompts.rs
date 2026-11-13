//! The texts of the checks before a review against the files of `prompts/harness/checks/`: the catalog and the directory
//! list the same files, each text says who it is for, and each renders with exactly its variables.

#[cfg(test)]
mod harness_checks_prompts {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, ChecksPrompt, PROMPTS_DIR, Variables};

    /// The directory of the texts of the checks, in the catalog's paths.
    const DIR: &str = "harness/checks";

    fn listed() -> BTreeSet<String> {
        ChecksPrompt::ALL.iter().map(|id| id.spec().path().to_owned()).collect()
    }

    fn read(path: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
        .unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    fn values(names: &[&str]) -> Variables {
        names
            .iter()
            .fold(Variables::new(), |values, name| values.with(name, "value").unwrap())
    }

    #[test]
    fn the_catalog_has_the_reasons_the_messages_and_the_questions_of_the_checks() {
        let listed = listed();

        for needed in [
            "issue-test-edited",
            "issue-protected",
            "issue-red-edited",
            "issue-no-tests",
            "issue-skips-test",
            "team-pass",
            "ask-relay",
            "answered",
            "question-body",
            "notice-not-asked",
        ] {
            assert!(
                listed.contains(&format!("{DIR}/{needed}.md")),
                "no text {needed} in the catalog: {listed:?}"
            );
        }
    }

    #[test]
    fn every_file_of_the_directory_is_in_the_catalog() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(DIR);

        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
            .map(|entry| format!("{DIR}/{}", entry.unwrap().file_name().to_str().unwrap()))
            .collect();

        assert_eq!(on_disk, listed());
    }

    #[test]
    fn what_a_model_reads_is_in_english_and_what_the_human_reads_may_be_in_their_language() {
        for id in ChecksPrompt::ALL {
            let name = format!("{id:?}");
            let human = name.starts_with("Question") || name.starts_with("Why") || name.starts_with("Notice");

            let expected = if human { Audience::Human } else { Audience::Model };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }

    #[test]
    fn every_text_exists_and_renders_with_exactly_its_variables() {
        for id in ChecksPrompt::ALL {
            let spec = id.spec();

            let rendered = spec
                .render(&read(spec.path()), &values(spec.variables()))
                .unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));

            assert!(!rendered.trim().is_empty(), "{id:?}: {} is empty", spec.path());
        }
    }

    #[test]
    fn every_notice_has_a_title_line_and_a_body() {
        let text = read(ChecksPrompt::NoticeNotAsked.spec().path());

        let mut lines = text.lines();
        let title = lines.next().unwrap_or_default();
        let body: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();

        assert_ne!(title.trim(), "");
        assert_ne!(body.len(), 0);
    }

    #[test]
    fn a_title_of_the_questions_file_is_one_line() {
        for id in [
            ChecksPrompt::QuestionsTitle,
            ChecksPrompt::QuestionTitlePath,
            ChecksPrompt::QuestionTitleNoTests,
        ] {
            let spec = id.spec();

            let rendered = spec.render(&read(spec.path()), &values(spec.variables())).unwrap();

            assert_eq!(rendered.trim().lines().count(), 1, "{id:?}: {rendered:?}");
        }
    }

    // Rule: what the team is told says how the round goes on: the gate, the end of the turn, and no push by the team.
    #[test]
    fn the_message_that_sends_the_round_back_says_how_the_team_goes_on() {
        let text = read(ChecksPrompt::TeamPass.spec().path());

        assert!(text.contains("agent gate"), "{text}");
        assert!(text.contains("STATUS: DONE"), "{text}");
        assert!(text.contains("Do not push"), "{text}");
    }

    #[test]
    fn what_the_lead_is_told_to_put_the_question_ends_its_turn_with_questions() {
        let text = read(ChecksPrompt::AskRelay.spec().path());

        assert!(text.contains("STATUS: QUESTIONS"), "{text}");
        assert!(ChecksPrompt::AskRelay.spec().variables().contains(&"questions_file"));
    }

    #[test]
    fn what_the_lead_is_told_after_the_answers_ends_its_turn_with_done() {
        let text = read(ChecksPrompt::Answered.spec().path());

        assert!(text.contains("STATUS: DONE"), "{text}");
        assert!(text.contains("agent gate"), "{text}");
    }

    // Rule: a reason tells the team how to settle what it names: the rules it cites are the ones the checks apply.
    #[test]
    fn the_reasons_name_the_way_out_the_rules_of_the_checks_give() {
        assert!(read(ChecksPrompt::IssueRedEdited.spec().path()).contains("TEST-FIX:"));
        assert!(read(ChecksPrompt::IssueRedEdited.spec().path()).contains("## Tests changed after RED"));
        assert!(read(ChecksPrompt::IssueNoTests.spec().path()).contains("Test-First: not possible"));
        for id in [
            ChecksPrompt::IssueSkipsTest,
            ChecksPrompt::IssueSuppressesLint,
            ChecksPrompt::IssueChecksUnderTest,
        ] {
            assert!(
                read(id.spec().path()).contains("description of the pull request"),
                "{id:?}"
            );
        }
    }

    // Rule: the body of a question holds no line that a questions file would read as structure.
    #[test]
    fn the_text_of_a_question_has_no_line_a_questions_file_reads_as_structure() {
        for id in [
            ChecksPrompt::QuestionBody,
            ChecksPrompt::WhyOnlyHuman,
            ChecksPrompt::WhyPassesUsed,
        ] {
            let spec = id.spec();
            let rendered = spec.render(&read(spec.path()), &values(spec.variables())).unwrap();

            for line in rendered.lines() {
                let line = line.trim_start();
                assert!(
                    !line.starts_with("## ") && !line.starts_with("**") && !line.starts_with("# "),
                    "{id:?}: {line}"
                );
            }
        }
    }
}
