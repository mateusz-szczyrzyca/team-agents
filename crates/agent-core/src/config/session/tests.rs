//! `[session]` and each agent's `max_context`: the default, the values, the refusals, and the bound an agent gets.

use super::super::{AgentName, Config, ConfigError, InvestigatorName};
use super::{ContextKey, DEFAULT_MAX_CONTEXT, context_bound};

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

/// The smallest configuration of one agent (`agent` is its extra lines) and optionally one investigator (`investigator`
/// is its extra lines, empty for none), which a test adds the `context` key to.
fn with_context(agent: &str, investigator: &str) -> String {
    let investigator = if investigator.is_empty() {
        String::new()
    } else {
        format!("\n[investigators.inv-1]\ncli = \"codex\"{investigator}\n")
    };
    format!(
        "[agents.claude-1]\ncli = \"claude\"{agent}\n{investigator}\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
    )
}

#[test]
fn a_context_is_read_for_an_agent_and_for_an_investigator_and_is_absent_without_it() {
    let config = Config::from_toml_str(&with_context("\ncontext = 200000", "\ncontext = 300000")).unwrap();
    let plain = Config::from_toml_str(&with_context("", "\n")).unwrap();
    let agent: AgentName = "claude-1".parse().unwrap();
    let investigator: InvestigatorName = "inv-1".parse().unwrap();

    assert_eq!(config.agent(&agent).unwrap().context, Some(200_000));
    assert_eq!(config.investigator(&investigator).unwrap().context, Some(300_000));
    assert_eq!(plain.agent(&agent).unwrap().context, None);
    assert_eq!(plain.investigator(&investigator).unwrap().context, None);
}

#[test]
fn a_context_that_is_not_a_number_is_refused() {
    let error = Config::from_toml_str(&with_context("\ncontext = true", "")).unwrap_err();

    assert!(
        matches!(&error, ConfigError::Parse(parse) if parse.span().is_some()),
        "{error:?}"
    );
}

#[test]
fn a_context_of_zero_is_refused_naming_the_agent_and_the_key() {
    for (agent, investigator) in [("\ncontext = 0", ""), ("", "\ncontext = 0")] {
        let error = Config::from_toml_str(&with_context(agent, investigator)).unwrap_err();
        let name = if agent.is_empty() { "inv-1" } else { "claude-1" };

        assert!(
            matches!(&error, ConfigError::AgentContextZero { name: refused, .. } if refused.as_str() == name),
            "{error:?}"
        );
        let text = error.to_string();
        assert!(text.contains(name) && text.contains("context"), "{text}");
    }
}

#[test]
fn a_context_below_the_agent_s_bound_is_refused_naming_it() {
    let section = Config::from_toml_str(&with_context("\ncontext = 1000", "")).unwrap_err();
    assert!(
        matches!(&section, ConfigError::AgentContextBelowBound { key: ContextKey::Agent, name, context: 1_000, bound: 150_000 }
            if name.as_str() == "claude-1"),
        "{section:?}"
    );

    let own = Config::from_toml_str(&with_context("\nmax_context = 5000\ncontext = 4000", "")).unwrap_err();
    assert!(
        matches!(
            &own,
            ConfigError::AgentContextBelowBound {
                key: ContextKey::Agent,
                context: 4_000,
                bound: 5_000,
                ..
            }
        ),
        "{own:?}"
    );
}

#[test]
fn a_context_is_accepted_when_the_bound_is_off_or_the_window_is_above_it() {
    let off = Config::from_toml_str(
        "[agents.claude-1]\ncli = \"claude\"\ncontext = 1\n\n[session]\nmax_context = 0\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n",
    )
    .unwrap();
    let above = Config::from_toml_str(&with_context("\ncontext = 200001", "")).unwrap();
    let equal = Config::from_toml_str(&with_context("\ncontext = 150000", "")).unwrap();

    assert_eq!(off.agent(&"claude-1".parse().unwrap()).unwrap().context, Some(1));
    assert_eq!(
        above.agent(&"claude-1".parse().unwrap()).unwrap().context,
        Some(200_001)
    );
    assert_eq!(
        equal.agent(&"claude-1".parse().unwrap()).unwrap().context,
        Some(150_000)
    );
}
