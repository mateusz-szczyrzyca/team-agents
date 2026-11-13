//! The texts of the autopilot against the files of `prompts/harness/pr/` and `prompts/harness/review/`: the catalog and
//! the two directories list the same files, each text says who it is for, and each renders with exactly its variables.

#[cfg(test)]
mod harness_autopilot_prompts {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, AutopilotPrompt, PROMPTS_DIR, Variables};

    /// The directories of the texts of the autopilot, in the catalog's paths.
    const DIRS: [&str; 2] = ["harness/pr", "harness/review"];

    fn listed() -> BTreeSet<String> {
        AutopilotPrompt::ALL
            .iter()
            .map(|id| id.spec().path().to_owned())
            .collect()
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

    #[test]
    fn the_catalog_has_the_texts_of_a_pull_request_and_its_reviews() {
        let listed = listed();

        for needed in [
            "harness/pr/title.md",
            "harness/pr/body.md",
            "harness/pr/update.md",
            "harness/pr/notice-approved.md",
            "harness/review/request.md",
            "harness/review/comment.md",
            "harness/review/fix-pass.md",
            "harness/review/ask-human.md",
            "harness/review/verdict-missing.md",
        ] {
            assert!(listed.contains(needed), "no text {needed} in the catalog: {listed:?}");
        }
    }

    #[test]
    fn every_file_of_the_two_directories_is_in_the_catalog() {
        let mut on_disk = BTreeSet::new();
        for dir in DIRS {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(dir);
            for entry in std::fs::read_dir(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())) {
                on_disk.insert(format!("{dir}/{}", entry.unwrap().file_name().to_str().unwrap()));
            }
        }

        assert_eq!(on_disk, listed());
    }

    #[test]
    fn what_goes_to_the_human_is_a_notice_and_the_rest_is_in_english_for_models_and_github() {
        for id in AutopilotPrompt::ALL {
            let expected = if id.spec().path().contains("/notice-") {
                Audience::Human
            } else {
                Audience::Model
            };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }

    #[test]
    fn every_text_exists_and_renders_with_exactly_its_variables() {
        for id in AutopilotPrompt::ALL {
            let spec = id.spec();
            let values = spec
                .variables()
                .iter()
                .fold(Variables::new(), |values, name| values.with(name, "value").unwrap());

            let rendered = spec
                .render(&read(spec.path()), &values)
                .unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));

            assert!(!rendered.trim().is_empty(), "{id:?}: {} is empty", spec.path());
        }
    }

    #[test]
    fn every_notice_has_a_title_line_and_a_body() {
        for id in AutopilotPrompt::ALL
            .iter()
            .filter(|id| id.spec().audience() == Audience::Human)
        {
            let text = read(id.spec().path());

            let mut lines = text.lines();
            let title = lines.next().unwrap_or_default();
            let body: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();

            assert_ne!(title.trim(), "", "{id:?}");
            assert_ne!(body.len(), 0, "{id:?}");
        }
    }

    #[test]
    fn the_request_for_a_review_names_the_pull_request_the_branch_and_the_report_it_wants() {
        let spec = AutopilotPrompt::ReviewRequest.spec();

        for name in ["pr", "branch", "base", "head"] {
            assert!(spec.variables().contains(&name), "{name}");
        }
        let text = read(spec.path());
        assert!(text.contains("VERDICT: APPROVE"), "{text}");
        assert!(text.contains("VERDICT: REQUEST_CHANGES"), "{text}");
        assert!(text.contains("STATUS: DONE"), "{text}");
    }

    #[test]
    fn the_fix_pass_gives_the_lead_the_findings_and_says_how_the_pass_ends() {
        let spec = AutopilotPrompt::ReviewFixPass.spec();

        for name in ["pr", "pass", "findings"] {
            assert!(spec.variables().contains(&name), "{name}");
        }
        assert!(read(spec.path()).contains("agent gate"));
    }
}
