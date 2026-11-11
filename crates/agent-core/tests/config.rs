//! The example configuration, `config/agents.example.toml`, through the public API: what users copy must load.

use agent_core::config::{Config, ConfigError};
use agent_core::ids::{AgentName, InvestigatorName};
use agent_core::kinds::{CliKind, Tier};

/// The example file, as shipped.
const EXAMPLE: &str = include_str!("../../../config/agents.example.toml");

#[test]
fn the_example_configuration_loads() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    assert!(config.language().is_english());
    let names: Vec<_> = config.agents().keys().map(AgentName::as_str).collect();
    assert_eq!(names, ["claude-1", "codex-1", "gemini-1"]);
    let gemini: AgentName = "gemini-1".parse().unwrap();
    assert_eq!(config.agent(&gemini).unwrap().cli, CliKind::Gemini);
    let normal: Vec<AgentName> = ["claude-1", "codex-1", "gemini-1"]
        .iter()
        .map(|name| name.parse().unwrap())
        .collect();
    assert_eq!(config.normal_agents(), normal);
    assert_eq!(config.worker_agents(), Vec::<AgentName>::new());
}

#[test]
fn the_example_has_the_two_investigators_every_review_needs() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    let names: Vec<_> = config.investigators().keys().map(InvestigatorName::as_str).collect();
    assert_eq!(names, ["inv-codex", "inv-opus"]);
}

#[test]
fn the_example_describes_a_repository_with_its_gate() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    let repo = config.repo(&"team-agents".parse().unwrap()).unwrap();
    assert_eq!(repo.base_branch.as_str(), "main");
    assert_eq!(repo.gate.program().as_path().to_str(), Some("make"));
    assert_eq!(repo.gate.args().as_slice(), ["gate"]);
}

#[test]
fn the_examples_dashboard_listens_on_loopback() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    assert!(config.web().listen.as_socket_addr().ip().is_loopback());
    assert_eq!(config.web().allowed_logins.len(), 1);
}

#[test]
fn a_copy_with_one_name_in_both_tier_lists_fails_naming_that_name() {
    let both = EXAMPLE.replace("workers = []", "workers = [\"codex-1\"]");
    assert_ne!(both, EXAMPLE, "the example has no `workers = []` line to change");

    let err = Config::from_toml_str(&both).unwrap_err();

    assert!(
        matches!(&err, ConfigError::NameInTwoLists { name, first: Tier::Normal, second: Tier::Worker } if name == "codex-1"),
        "{err:?}"
    );
}
