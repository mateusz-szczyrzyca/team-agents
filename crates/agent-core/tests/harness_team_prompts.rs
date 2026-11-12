//! The texts of a team's life against the files of `prompts/harness/team/`: the catalog and the directory list the
//! same files, and each text says who it is for.

#[cfg(test)]
mod harness_team_prompts {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, PROMPTS_DIR, TeamPrompt, Variables};

    /// The directory of the texts of a team's life, in the catalog's paths.
    const TEAM_DIR: &str = "harness/team";

    fn listed() -> BTreeSet<String> {
        TeamPrompt::ALL.iter().map(|id| id.spec().path().to_owned()).collect()
    }

    #[test]
    fn the_catalog_has_the_texts_of_a_teams_life() {
        let listed = listed();

        for needed in ["context", "assignment", "wake", "delegation", "notice-team-done"] {
            assert!(
                listed.contains(&format!("{TEAM_DIR}/{needed}.md")),
                "no text {needed} in the catalog: {listed:?}"
            );
        }
    }

    #[test]
    fn every_file_of_the_team_directory_is_in_the_catalog() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(TEAM_DIR);

        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| format!("{TEAM_DIR}/{}", entry.unwrap().file_name().to_str().unwrap()))
            .collect();

        assert_eq!(on_disk, listed());
    }

    #[test]
    fn what_the_team_tells_a_model_is_for_a_model_and_the_notification_for_the_human() {
        for id in TeamPrompt::ALL {
            let expected = if format!("{id:?}").starts_with("Notice") {
                Audience::Human
            } else {
                Audience::Model
            };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }

    #[test]
    fn every_text_exists_and_renders_with_exactly_its_variables() {
        for id in TeamPrompt::ALL {
            let spec = id.spec();
            let text = std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../")
                    .join(PROMPTS_DIR)
                    .join(spec.path()),
            )
            .unwrap_or_else(|error| panic!("{id:?}: {}: {error}", spec.path()));
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
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(PROMPTS_DIR)
                .join(TeamPrompt::NoticeTeamDone.spec().path()),
        )
        .unwrap();

        let mut lines = text.lines();
        let title = lines.next().unwrap_or_default();
        let body: Vec<_> = lines.filter(|line| !line.trim().is_empty()).collect();

        assert!(!title.trim().is_empty());
        assert!(!body.is_empty());
    }
}
