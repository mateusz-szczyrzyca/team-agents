//! `[session]` and each agent's `max_context`: the default, the values, the refusals, and the bound an agent gets.

use super::super::{AgentName, Config, ConfigError};
use super::{DEFAULT_MAX_CONTEXT, context_bound};

/// The smallest configuration: one normal agent, which is all a task needs.
fn one_agent() -> &'static str {
    "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
}

fn with(table: &str) -> String {
    format!("{}{table}", one_agent())
}

#[test]
fn a_file_without_session_bounds_a_context_at_the_default() {
    let config = Config::from_toml_str(one_agent()).unwrap();

    assert_eq!(config.session().max_context, DEFAULT_MAX_CONTEXT);
}

#[test]
fn the_session_table_sets_the_bound_and_zero_switches_it_off() {
    let set = Config::from_toml_str(&with("[session]\nmax_context = 300000\n")).unwrap();
    assert_eq!(set.session().max_context, 300_000);

    let off = Config::from_toml_str(&with("[session]\nmax_context = 0\n")).unwrap();
    assert_eq!(off.session().max_context, 0);
}

#[test]
fn a_bound_that_is_not_a_number_is_refused() {
    let error = Config::from_toml_str(&with("[session]\nmax_context = \"big\"\n")).unwrap_err();

    assert!(
        matches!(&error, ConfigError::Parse(parse) if parse.span().is_some() && parse.message().contains("big")),
        "{error:?}"
    );
}

#[test]
fn a_negative_bound_is_refused() {
    let error = Config::from_toml_str(&with("[session]\nmax_context = -1\n")).unwrap_err();

    assert!(
        matches!(&error, ConfigError::Parse(parse) if parse.span().is_some()),
        "{error:?}"
    );
}

#[test]
fn an_agent_reads_its_own_bound_and_one_without_has_none() {
    let config = Config::from_toml_str(
        "[agents.claude-1]\ncli = \"claude\"\nmax_context = 5000\n\n[agents.claude-2]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\", \"claude-2\"]\nworkers = []\n",
    )
    .unwrap();
    let own: AgentName = "claude-1".parse().unwrap();
    let none: AgentName = "claude-2".parse().unwrap();

    assert_eq!(config.agent(&own).unwrap().max_context, Some(5_000));
    assert_eq!(config.agent(&none).unwrap().max_context, None);
}

#[test]
fn the_bound_of_an_agent_is_its_own_else_the_section_else_off_at_zero() {
    for (agent, section, expected) in [
        (None, DEFAULT_MAX_CONTEXT, Some(DEFAULT_MAX_CONTEXT)),
        (Some(5_000), DEFAULT_MAX_CONTEXT, Some(5_000)),
        (Some(0), DEFAULT_MAX_CONTEXT, None),
        (None, 0, None),
        (Some(7), 0, Some(7)),
    ] {
        assert_eq!(
            context_bound(agent, section),
            expected,
            "agent {agent:?}, section {section}"
        );
    }
}
