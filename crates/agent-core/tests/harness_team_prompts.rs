//! The texts of a team's life against the files of `prompts/harness/team/`: the catalog and the directory list the
//! same files, and each text says who it is for.

#[cfg(test)]
mod harness_team_prompts {
    use std::collections::BTreeSet;
    use std::path::Path;

    use agent_core::prompt::{Audience, HarnessPrompt, PROMPTS_DIR};

    /// The directory of the texts of a team's life, in the catalog's paths.
    const TEAM_DIR: &str = "harness/team";

    fn listed() -> BTreeSet<String> {
        HarnessPrompt::ALL
            .iter()
            .map(|id| id.spec().path().to_owned())
            .filter(|path| path.starts_with(TEAM_DIR))
            .collect()
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
        for id in HarnessPrompt::ALL.iter().filter(|id| id.spec().path().starts_with(TEAM_DIR)) {
            let expected = if id.spec().path().ends_with("notice-team-done.md") {
                Audience::Human
            } else {
                Audience::Model
            };

            assert_eq!(id.spec().audience(), expected, "{id:?}");
        }
    }
}
