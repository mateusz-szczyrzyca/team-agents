//! Where `agent status` reads the requests for workers of the tasks it shows, by the configuration of the home.

use std::str::FromStr;

use agent_core::config::AbsolutePath;

use super::*;

/// A home in a temporary directory with `config` as its `agents.toml`, or none.
fn home(dir: &tempfile::TempDir, config: Option<&str>) -> Home {
    if let Some(text) = config {
        std::fs::write(dir.path().join("agents.toml"), text).unwrap();
    }
    Home::new(&AbsolutePath::from_str(dir.path().to_str().unwrap()).unwrap())
}

#[tokio::test]
async fn a_home_without_a_configuration_reads_no_requests() {
    let dir = tempfile::tempdir().unwrap();

    let source = requests_source(&home(&dir, None)).await;

    assert!(matches!(source, RequestsSource::Unread), "{source:?}");
}

#[tokio::test]
async fn a_configuration_that_does_not_load_says_why_the_requests_are_unknown() {
    let dir = tempfile::tempdir().unwrap();

    let source = requests_source(&home(&dir, Some("this is [not toml"))).await;

    let RequestsSource::Unknown(why) = source else {
        panic!("{source:?}")
    };
    assert!(why.contains("agents.toml"), "{why}");
}

#[tokio::test]
async fn a_home_that_loads_reads_the_requests_from_its_bus() {
    let dir = tempfile::tempdir().unwrap();

    let source = requests_source(&home(
        &dir,
        Some(include_str!("../../../../config/agents.example.toml")),
    ))
    .await;

    assert!(matches!(source, RequestsSource::Bus(_)), "{source:?}");
}
