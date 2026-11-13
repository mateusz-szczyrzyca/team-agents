//! Loading the texts of the questions and their answers from a prompts directory: what loads, the language override of
//! the texts for the human, and every way a file can be unusable.

#[cfg(test)]
mod questions_prompts {
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::Language;
    use agent_core::prompt::{PromptError, QuestionsPrompt, RenderError, Variables};
    use agent_runtime::harness_prompts::ProblemCause;
    use agent_runtime::questions_prompts::{QuestionsPrompts, RenderQuestionsTextError};

    /// The prompts directory of this repository.
    fn repository_prompts() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn language(text: &str) -> Language {
        Language::from_str(text).unwrap()
    }

    /// A valid text for `id`: it uses exactly the variables the catalog lists.
    fn valid_text(id: QuestionsPrompt) -> String {
        let spec = id.spec();
        let variables: Vec<String> = spec.variables().iter().map(|name| format!("{{{{{name}}}}}")).collect();
        format!("Text of {}. {}\n", spec.path(), variables.join(" "))
    }

    /// A prompts directory with a valid file for every text of the catalog.
    fn catalog_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in QuestionsPrompt::ALL {
            let path = dir.path().join(id.spec().path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, valid_text(*id)).unwrap();
        }
        dir
    }

    fn values(pairs: &[(&str, &str)]) -> Variables {
        pairs.iter().fold(Variables::new(), |values, (name, value)| {
            values.with(name, *value).unwrap()
        })
    }

    #[tokio::test]
    async fn the_texts_of_this_repository_load_and_render() {
        let loader = QuestionsPrompts::load(&repository_prompts(), &Language::default())
            .await
            .unwrap();

        let text = loader
            .render(QuestionsPrompt::Instructions, &values(&[("task_id", "task-x")]))
            .unwrap();

        assert!(text.contains("agent answer task-x"), "{text}");
    }

    #[tokio::test]
    async fn a_value_is_put_in_the_text_where_its_variable_is() {
        let dir = catalog_dir();
        let loader = QuestionsPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let text = loader
            .render(QuestionsPrompt::Accepted, &values(&[("recommendation", "a) memory")]))
            .unwrap();

        assert_eq!(text, "Text of harness/questions/accepted.md. a) memory\n");
    }

    #[tokio::test]
    async fn rendering_with_the_wrong_values_is_an_error_that_names_the_variable() {
        let dir = catalog_dir();
        let loader = QuestionsPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let error = loader
            .render(QuestionsPrompt::Accepted, &values(&[("other", "x")]))
            .unwrap_err();

        assert!(
            matches!(
                &error,
                RenderQuestionsTextError::Render {
                    id: QuestionsPrompt::Accepted,
                    source: PromptError::Render(RenderError::Unknown { name })
                } if name.as_str() == "other"
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn the_notification_is_taken_from_the_language_directory_when_it_has_one() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/questions/notice-incomplete.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "Odpowiedzi {{agent}} {{task}} {{file}} {{problems}}\n").unwrap();
        let loader = QuestionsPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader
            .render(
                QuestionsPrompt::NoticeIncomplete,
                &values(&[("agent", "a"), ("task", "t"), ("file", "f"), ("problems", "p")]),
            )
            .unwrap();

        assert_eq!(text, "Odpowiedzi a t f p\n");
    }

    #[tokio::test]
    async fn a_text_for_a_model_ignores_the_language_directory() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/questions/answered.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "po polsku {{number}} {{answers}}\n").unwrap();
        let loader = QuestionsPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader
            .render(QuestionsPrompt::Answered, &values(&[("number", "1"), ("answers", "x")]))
            .unwrap();

        assert_eq!(text, "Text of harness/questions/answered.md. 1 x\n");
    }

    #[tokio::test]
    async fn every_problem_is_reported_not_only_the_first() {
        let dir = catalog_dir();
        std::fs::remove_file(dir.path().join("harness/questions/answered.md")).unwrap();
        std::fs::write(
            dir.path().join("harness/questions/instructions.md"),
            "uses {{undeclared}}",
        )
        .unwrap();
        std::fs::write(dir.path().join("harness/questions/accepted.md"), [0xff, 0xfe]).unwrap();

        let error = QuestionsPrompts::load(dir.path(), &Language::default())
            .await
            .unwrap_err();

        let problems: Vec<_> = error
            .problems()
            .iter()
            .map(|problem| (problem.id, &problem.cause))
            .collect();
        assert!(
            matches!(
                problems.as_slice(),
                [
                    (
                        QuestionsPrompt::Instructions,
                        ProblemCause::Invalid(PromptError::Undeclared { .. })
                    ),
                    (QuestionsPrompt::Accepted, ProblemCause::NotUtf8(_)),
                    (QuestionsPrompt::Answered, ProblemCause::Missing { .. }),
                ]
            ),
            "{problems:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_directory_has_a_problem_for_every_text_and_the_error_names_each_file() {
        let dir = tempfile::tempdir().unwrap();

        let error = QuestionsPrompts::load(dir.path(), &Language::default())
            .await
            .unwrap_err();

        assert_eq!(error.problems().len(), QuestionsPrompt::ALL.len());
        let text = error.to_string();
        assert!(text.contains(&dir.path().display().to_string()), "{text}");
        assert!(text.contains("harness/questions/answered.md"), "{text}");
    }
}
