//! The catalog of the texts the harness composes against the files of `prompts/`: every entry must exist and render
//! with its variables, and a failure must name the file. Beside it, the shape each text must have.

#[cfg(test)]
mod harness_prompt_catalog {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, HarnessPrompt, PROMPTS_DIR, Variables};

    /// The value every variable gets in the check: the text only has to render.
    const PLACEHOLDER_VALUE: &str = "value";

    fn read(path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
    }

    #[test]
    fn every_entry_exists_and_renders_with_exactly_its_variables() {
        for id in HarnessPrompt::ALL {
            let spec = id.spec();
            let text = read(spec.path()).unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));
            let values = spec.variables().iter().fold(Variables::new(), |values, name| {
                values.with(name, PLACEHOLDER_VALUE).unwrap()
            });

            let rendered = spec
                .render(&text, &values)
                .unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));

            assert!(!rendered.trim().is_empty(), "{id:?}: {} is empty", spec.path());
        }
    }

    #[test]
    fn no_two_entries_share_a_file() {
        let paths: BTreeSet<_> = HarnessPrompt::ALL.iter().map(|id| id.spec().path()).collect();

        assert_eq!(paths.len(), HarnessPrompt::ALL.len());
    }

    #[test]
    fn every_file_of_the_run_directory_is_in_the_catalog() {
        let listed: BTreeSet<String> = HarnessPrompt::ALL
            .iter()
            .map(|id| id.spec().path().to_owned())
            .collect();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join("harness/run");

        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| format!("harness/run/{}", entry.unwrap().file_name().to_str().unwrap()))
            .collect();

        assert_eq!(on_disk, listed);
    }

    #[test]
    fn notifications_are_for_the_human_and_run_prompts_for_a_model() {
        for id in HarnessPrompt::ALL {
            let expected = if format!("{id:?}").starts_with("Notice") {
                Audience::Human
            } else {
                Audience::Model
            };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }

    #[test]
    fn a_notification_has_a_title_line_and_a_body() {
        for id in HarnessPrompt::ALL
            .iter()
            .filter(|id| id.spec().audience() == Audience::Human)
        {
            let text = read(id.spec().path()).unwrap();

            let mut lines = text.lines();
            let title = lines.next().unwrap_or_default();
            let body: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();

            assert!(!title.trim().is_empty(), "{id:?} has no title line");
            assert!(!body.is_empty(), "{id:?} has no body");
        }
    }
}
