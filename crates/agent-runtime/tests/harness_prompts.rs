//! Loading the texts of the harness's own catalog from a prompts directory: what loads, the language override of the
//! texts for the human, and every way a file can be unusable.

#[cfg(test)]
mod harness_prompts {
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::Language;
    use agent_core::prompt::{HarnessPrompt, PromptError, RenderError, Variables};
    use agent_runtime::harness_prompts::{HarnessPrompts, MAX_TEXT_BYTES, ProblemCause, RenderTextError};

    /// The prompts directory of this repository.
    fn repository_prompts() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn language(text: &str) -> Language {
        Language::from_str(text).unwrap()
    }

    /// A valid text for `id`: it uses exactly the variables the catalog lists.
    fn valid_text(id: HarnessPrompt) -> String {
        let spec = id.spec();
        let variables: Vec<String> = spec.variables().iter().map(|name| format!("{{{{{name}}}}}")).collect();
        format!("Text of {}. {}\n", spec.path(), variables.join(" "))
    }

    /// A prompts directory with a valid file for every text of the catalog.
    fn catalog_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in HarnessPrompt::ALL {
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
    async fn the_prompts_of_this_repository_load_and_render() {
        let loader = HarnessPrompts::load(&repository_prompts(), &Language::default())
            .await
            .unwrap();

        let text = loader
            .render(HarnessPrompt::PromptFile, &values(&[("path", "/tmp/p.md")]))
            .unwrap();

        assert!(text.contains("/tmp/p.md"), "{text}");
    }

    #[tokio::test]
    async fn a_value_is_put_in_the_text_where_its_variable_is() {
        let dir = catalog_dir();
        let loader = HarnessPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let text = loader
            .render(HarnessPrompt::Brief, &values(&[("brief", "Build the thing.")]))
            .unwrap();

        assert_eq!(text, "Text of harness/run/brief.md. Build the thing.\n");
    }

    #[tokio::test]
    async fn rendering_with_the_wrong_values_is_an_error_that_names_the_variable() {
        let dir = catalog_dir();
        let loader = HarnessPrompts::load(dir.path(), &Language::default()).await.unwrap();

        let error = loader
            .render(HarnessPrompt::Brief, &values(&[("other", "x")]))
            .unwrap_err();

        assert!(
            matches!(
                &error,
                RenderTextError::Render { id: HarnessPrompt::Brief, source: PromptError::Render(RenderError::Unknown { name }) }
                    if name.as_str() == "other"
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_text_for_the_human_is_taken_from_the_language_directory_when_it_has_one() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/run/notice-hung.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "Zawieszony {{agent}} {{task}} {{minutes}} {{strikes}}\n").unwrap();
        let loader = HarnessPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader
            .render(
                HarnessPrompt::NoticeHung,
                &values(&[("agent", "a"), ("task", "t"), ("minutes", "3"), ("strikes", "2")]),
            )
            .unwrap();

        assert_eq!(text, "Zawieszony a t 3 2\n");
    }

    #[tokio::test]
    async fn a_text_for_a_model_ignores_the_language_directory() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/run/role-solo.md");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "po polsku\n").unwrap();
        let loader = HarnessPrompts::load(dir.path(), &language("Polish")).await.unwrap();

        let text = loader.render(HarnessPrompt::RoleSolo, &Variables::new()).unwrap();

        assert_eq!(text, "Text of harness/run/role-solo.md. \n");
    }

    #[tokio::test]
    async fn every_problem_is_reported_not_only_the_first() {
        let dir = catalog_dir();
        std::fs::remove_file(dir.path().join("harness/run/brief.md")).unwrap();
        std::fs::write(dir.path().join("harness/run/role-solo.md"), "{{ broken").unwrap();
        std::fs::write(dir.path().join("harness/run/role-member.md"), "uses {{undeclared}}").unwrap();
        std::fs::write(dir.path().join("harness/run/role-worker.md"), [0xff, 0xfe]).unwrap();
        std::fs::write(
            dir.path().join("harness/run/role-reviewer.md"),
            "x".repeat(MAX_TEXT_BYTES + 1),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("harness/run/role-coordinator.md")).unwrap();
        std::fs::create_dir(dir.path().join("harness/run/role-coordinator.md")).unwrap();

        let error = HarnessPrompts::load(dir.path(), &Language::default())
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
                    (HarnessPrompt::RoleSolo, ProblemCause::Invalid(PromptError::Template(_))),
                    (HarnessPrompt::RoleCoordinator, ProblemCause::Read(_)),
                    (
                        HarnessPrompt::RoleMember,
                        ProblemCause::Invalid(PromptError::Undeclared { .. })
                    ),
                    (HarnessPrompt::RoleWorker, ProblemCause::NotUtf8(_)),
                    (HarnessPrompt::RoleReviewer, ProblemCause::TooLarge),
                    (HarnessPrompt::Brief, ProblemCause::Missing { .. }),
                ]
            ),
            "{problems:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_text_lists_the_files_that_were_tried() {
        let dir = catalog_dir();
        std::fs::remove_file(dir.path().join("harness/run/notice-failed.md")).unwrap();

        let error = HarnessPrompts::load(dir.path(), &language("Polish")).await.unwrap_err();

        let [problem] = error.problems() else {
            panic!("one problem expected: {:?}", error.problems())
        };
        let ProblemCause::Missing { tried } = &problem.cause else {
            panic!("{problem:?}")
        };
        assert_eq!(
            tried,
            &[
                Path::new("lang/polish/harness/run/notice-failed.md").to_path_buf(),
                Path::new("harness/run/notice-failed.md").to_path_buf()
            ]
        );
    }

    #[tokio::test]
    async fn a_translation_that_cannot_be_read_does_not_fall_back_to_english() {
        let dir = catalog_dir();
        let own = dir.path().join("lang/polish/harness/run/notice-hung.md");
        std::fs::create_dir_all(&own).unwrap();

        let error = HarnessPrompts::load(dir.path(), &language("Polish")).await.unwrap_err();

        let [problem] = error.problems() else {
            panic!("one problem expected: {:?}", error.problems())
        };
        assert!(matches!(problem.cause, ProblemCause::Read(_)), "{problem:?}");
        assert_eq!(problem.path, own);
    }

    #[tokio::test]
    async fn an_empty_directory_has_a_problem_for_every_text() {
        let dir = tempfile::tempdir().unwrap();

        let error = HarnessPrompts::load(dir.path(), &Language::default())
            .await
            .unwrap_err();

        assert_eq!(error.problems().len(), HarnessPrompt::ALL.len());
    }
}
