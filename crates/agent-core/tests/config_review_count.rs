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
