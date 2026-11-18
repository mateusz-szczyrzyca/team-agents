//! Configurable independent full review counts and compatibility of omitted settings.

use agent_core::config::{Config, ReviewCount, ReviewCountError};

/// Three configured investigators allow either supported pass size in these cases.
const INVESTIGATORS: &str =
    "[investigators.a]\ncli = 'claude'\n[investigators.b]\ncli = 'codex'\n[investigators.c]\ncli = 'claude'\n";

#[test]
fn the_review_count_defaults_to_two_and_accepts_two_or_three() {
    assert_eq!(
        Config::from_toml_str("")
            .unwrap()
            .review()
            .min_first_full_reviews()
            .get(),
        2
    );
    for count in [2, 3] {
        let config = Config::from_toml_str(&format!(
            "{INVESTIGATORS}\n[review]\nmin_first_full_reviews = {count}\n"
        ))
        .unwrap();
        assert_eq!(config.review().min_first_full_reviews().get(), count);
    }
}

#[test]
fn a_review_count_below_two_is_refused_by_its_type() {
    for value in [0, 1] {
        assert_eq!(ReviewCount::try_from(value), Err(ReviewCountError { value }));
    }
}

#[test]
fn explicit_counts_cannot_exceed_the_configured_investigators() {
    use agent_core::config::ConfigError;
    let error = Config::from_toml_str(&format!("{INVESTIGATORS}\n[review]\nmin_first_full_reviews = 4\n")).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::ReviewersUnavailable {
            required: 4,
            configured: 3
        }
    ));
    for count in [0, 1] {
        let error = Config::from_toml_str(&format!(
            "{INVESTIGATORS}\n[review]\nmin_first_full_reviews = {count}\n"
        ))
        .unwrap_err();
        assert!(matches!(error, ConfigError::Parse(_)));
    }
}

#[test]
fn a_team_must_leave_the_configured_count_outside_it() {
    use agent_core::decide::team::{TeamPlanError, TeamRequest, plan_team};
    use rand::SeedableRng;
    let config = Config::from_toml_str(&format!("{INVESTIGATORS}\n[review]\nmin_first_full_reviews = 3\n")).unwrap();
    let agents = agent_core::protocol::AgentList::try_from(vec!["a".parse().unwrap()]).unwrap();
    let request = TeamRequest {
        agents: &agents,
        flag_coordinator: None,
        listed_coordinator: None,
    };
    let error = plan_team(&request, &config, &mut rand::rngs::Xoshiro256PlusPlus::seed_from_u64(1)).unwrap_err();
    assert_eq!(
        error,
        TeamPlanError::TooManyInvestigators {
            in_team: 1,
            configured: 3,
            required: 3
        }
    );
}
