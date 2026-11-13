//! Loading the texts of the autopilot from a prompts directory: what loads, the language override of the notices for the
//! human, and every way a file can be unusable.

#[cfg(test)]
mod autopilot_prompts {
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::Language;
    use agent_core::prompt::{AutopilotPrompt, PromptError, RenderError, Variables};
    use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
    use agent_runtime::harness_prompts::ProblemCause;

    /// The prompts directory of this repository.
    fn repository_prompts() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn language(text: &str) -> Language {
        Language::from_str(text).unwrap()
    }

    /// A valid text for `id`: it uses exactly the variables the catalog lists.
    fn valid_text(id: AutopilotPrompt) -> String {
        let spec = id.spec();
        let variables: Vec<String> = spec.variables().iter().map(|name| format!("{{{{{name}}}}}")).collect();
        format!("Text of {}. {}\n", spec.path(), variables.join(" "))
    }

    /// A prompts directory with a valid file for every text of the catalog.
    fn catalog_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in AutopilotPrompt::ALL {
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

    /// The values of the update comment.
    fn update_values() -> Variables {
        values(&[("head", "abc1234"), ("pass", "2"), ("gate", "passed at abc1234")])
    }

    #[tokio::test]
    async fn the_texts_of_this_repository_load_and_render() {
        let loader = AutopilotPrompts::load(&repository_prompts(), &Language::default())
            .await
            .unwrap();

        let text = loader.render(AutopilotPrompt::PrUpdate, &update_values()).unwrap();

        assert!(text.contains("abc1234"), "{text}");
    }

    #[tokio::test]
    async fn a_value_is_put_in_the_text_where_its_variable_is() {
        let dir = catalog_dir();
        let loader = AutopilotPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let text = loader.render(AutopilotPrompt::PrUpdate, &update_values()).unwrap();

        assert!(text.starts_with("Text of harness/pr/update.md. "), "{text}");
        for value in ["abc1234", "passed at abc1234"] {
            assert!(text.contains(value), "{value}: {text}");
        }
        assert!(!text.contains("{{"), "{text}");
    }

    #[tokio::test]
    async fn rendering_with_the_wrong_values_is_an_error_that_names_the_variable() {
        let dir = catalog_dir();
        let loader = AutopilotPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let error = loader
            .render(AutopilotPrompt::PrUpdate, &values(&[("other", "x")]))
            .unwrap_err();

        assert!(
            matches!(
                &error,
                RenderAutopilotTextError::Render { id: AutopilotPrompt::PrUpdate, source: PromptError::Render(RenderError::Unknown { name }) }
                    if name.as_str() == "other"
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn the_notice_is_taken_from_the_language_directory_when_it_has_one() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/pr/notice-approved.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "Zatwierdzone {{team}} {{repo}} {{pr}} {{url}}\n").unwrap();
        let loader = AutopilotPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader
            .render(
                AutopilotPrompt::NoticeApproved,
                &values(&[("team", "t"), ("repo", "r"), ("pr", "7"), ("url", "u")]),
            )
            .unwrap();

        assert_eq!(text, "Zatwierdzone t r 7 u\n");
    }

    #[tokio::test]
    async fn a_text_for_github_ignores_the_language_directory() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/pr/update.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "po polsku {{head}} {{pass}} {{gate}}\n").unwrap();
        let loader = AutopilotPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader.render(AutopilotPrompt::PrUpdate, &update_values()).unwrap();

        assert!(text.starts_with("Text of harness/pr/update.md."), "{text}");
    }

    #[tokio::test]
    async fn every_problem_is_reported_not_only_the_first() {
        let dir = catalog_dir();
        std::fs::remove_file(dir.path().join("harness/review/request.md")).unwrap();
        std::fs::write(dir.path().join("harness/pr/title.md"), "uses {{undeclared}}").unwrap();
        std::fs::write(dir.path().join("harness/pr/body.md"), [0xff, 0xfe]).unwrap();

        let error = AutopilotPrompts::load(dir.path(), &Language::default())
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
                        AutopilotPrompt::PrTitle,
                        ProblemCause::Invalid(PromptError::Undeclared { .. })
                    ),
                    (AutopilotPrompt::PrBody, ProblemCause::NotUtf8(_)),
                    (AutopilotPrompt::ReviewRequest, ProblemCause::Missing { .. }),
                ]
            ),
            "{problems:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_directory_has_a_problem_for_every_text_and_the_error_names_each_file() {
        let dir = tempfile::tempdir().unwrap();

        let error = AutopilotPrompts::load(dir.path(), &Language::default())
            .await
            .unwrap_err();

        assert_eq!(error.problems().len(), AutopilotPrompt::ALL.len());
        let text = error.to_string();
        assert!(text.contains(&dir.path().display().to_string()), "{text}");
        assert!(text.contains("harness/review/request.md"), "{text}");
    }
}
