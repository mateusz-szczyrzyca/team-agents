//! Rotation capacity is a warning, separate from the minimum that permits a team to start.
use agent_core::{config::Config, decide::team::needs_review_rotation_warning, protocol::AgentList};

#[test]
fn rotation_warning_counts_only_investigators_outside_the_requested_team() {
    for (count, required, selected, warning) in [
        (2, 2, false, true),
        (3, 2, false, true),
        (4, 2, false, false),
        (4, 2, true, true),
        (6, 3, false, false),
        (6, 3, true, true),
    ] {
        let mut text = format!(
            "[tiers]\nnormal=['lead']\n[agents.lead]\ncli='claude'\n[review]\nmin_first_full_reviews={required}\n"
        );
        for n in 0..count {
            text.push_str(&format!("[investigators.inv-{n}]\ncli='codex'\n"));
        }
        let config = Config::from_toml_str(&text).unwrap();
        let mut agents = vec!["lead".parse().unwrap()];
        if selected {
            agents.push("inv-0".parse().unwrap());
        }
        assert_eq!(
            needs_review_rotation_warning(&config, &AgentList::try_from(agents).unwrap()),
            warning,
            "{count}, {required}, {selected}"
        );
    }
}
