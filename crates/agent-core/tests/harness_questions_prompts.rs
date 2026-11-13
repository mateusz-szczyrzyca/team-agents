//! The texts of the questions and their answers against the files of `prompts/harness/questions/`: the catalog and the
//! directory list the same files, and each text says who it is for.

#[cfg(test)]
mod harness_questions_prompts {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, PROMPTS_DIR, QuestionsPrompt, Variables};

    /// The directory of the texts of the questions, in the catalog's paths.
    const QUESTIONS_DIR: &str = "harness/questions";

    fn listed() -> BTreeSet<String> {
        QuestionsPrompt::ALL
            .iter()
            .map(|id| id.spec().path().to_owned())
            .collect()
    }

    fn read(path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(path),
        )
    }

    #[test]
    fn the_catalog_has_the_texts_of_the_questions_and_their_answers() {
        let listed = listed();

        for needed in ["instructions", "accepted", "answered", "notice-incomplete"] {
            assert!(
                listed.contains(&format!("{QUESTIONS_DIR}/{needed}.md")),
                "no text {needed} in the catalog: {listed:?}"
            );
        }
    }

    #[test]
    fn every_file_of_the_questions_directory_is_in_the_catalog() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(QUESTIONS_DIR);

        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| format!("{QUESTIONS_DIR}/{}", entry.unwrap().file_name().to_str().unwrap()))
            .collect();

        assert_eq!(on_disk, listed());
    }

    #[test]
    fn what_the_human_reads_is_for_the_human_and_what_the_agent_gets_for_a_model() {
        let for_the_human = [QuestionsPrompt::Instructions, QuestionsPrompt::NoticeIncomplete];

        for id in QuestionsPrompt::ALL {
            let expected = if for_the_human.contains(id) {
                Audience::Human
            } else {
                Audience::Model
            };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }

    #[test]
    fn every_text_exists_and_renders_with_exactly_its_variables() {
        for id in QuestionsPrompt::ALL {
            let spec = id.spec();
            let text = read(spec.path()).unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));
            let values = spec
                .variables()
                .iter()
                .fold(Variables::new(), |values, name| values.with(name, "value").unwrap());

            let rendered = spec
                .render(&text, &values)
                .unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));

            assert!(!rendered.trim().is_empty(), "{id:?}: {} is empty", spec.path());
        }
    }

    #[test]
    fn the_notification_has_a_title_line_and_a_body() {
        let text = read(QuestionsPrompt::NoticeIncomplete.spec().path()).unwrap();

        let mut lines = text.lines();
        let title = lines.next().unwrap_or_default();
        let body: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();

        assert_ne!(title.trim(), "");
        assert_ne!(body.len(), 0);
    }
}
