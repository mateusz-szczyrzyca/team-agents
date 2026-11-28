//! The view of the accounts: who runs with which, the label of an agent, and the names of the variables.

use super::{AccountView, account_views, agent_accounts, labelled};
use crate::config::{AccountName, Config, Secrets};
use crate::ids::AgentName;

/// A secret that must never show in the view.
const MARKER: &str = "MARKER-7f3a";

/// The mode of a file only its owner can use.
const OWNER_ONLY: u32 = 0o600;

fn account(text: &str) -> AccountName {
    text.parse().unwrap()
}

fn agent(text: &str) -> AgentName {
    text.parse().unwrap()
}

/// Two accounts, an agent and an investigator of the first, an agent of the second and one with none.
const CONFIG: &str = r#"
[accounts.comp-claude]
owner = "comp"
[accounts.comp-claude.env]
CLAUDE_CONFIG_DIR = "/home/x/.claude-comp"

[accounts.mateusz-codex]
owner = "mateusz"

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[agents.mateusz-codex-1]
cli = "codex"
account = "mateusz-codex"

[agents.plain-1]
cli = "claude"

[investigators.comp-inv-opus]
cli = "claude"
model = "opus"
account = "comp-claude"

[tiers]
normal = ["comp-claude-1", "mateusz-codex-1", "plain-1"]
"#;

#[test]
fn each_agent_and_investigator_with_an_account_is_mapped_to_it() {
    let config = Config::from_toml_str(CONFIG).unwrap();

    let found: Vec<(String, String)> = agent_accounts(&config)
        .into_iter()
        .map(|(agent, account)| (agent.to_string(), account.to_string()))
        .collect();

    assert_eq!(
        found,
        [
            ("comp-claude-1".to_owned(), "comp-claude".to_owned()),
            ("comp-inv-opus".to_owned(), "comp-claude".to_owned()),
            ("mateusz-codex-1".to_owned(), "mateusz-codex".to_owned()),
        ]
    );
}

#[test]
fn a_configuration_without_accounts_maps_no_agent() {
    let config = Config::from_toml_str("[agents.codex-1]\ncli = \"codex\"\n[tiers]\nnormal = [\"codex-1\"]\n").unwrap();

    assert!(agent_accounts(&config).is_empty());
}

#[test]
fn the_label_puts_the_account_in_brackets_after_the_agent() {
    assert_eq!(
        labelled("comp-inv-opus", Some(&account("comp-claude"))),
        "comp-inv-opus [comp-claude]"
    );
}

#[test]
fn the_label_of_an_agent_without_an_account_is_its_name() {
    assert_eq!(labelled("plain-1", None), "plain-1");
}

#[test]
fn an_account_view_names_its_owner_agents_and_variables_without_values() {
    let config = Config::from_toml_str(CONFIG).unwrap();
    let secrets = Secrets::parse(
        &format!("[account_env.comp-claude]\nCLAUDE_CODE_OAUTH_TOKEN = \"{MARKER}\"\n"),
        OWNER_ONLY,
    )
    .unwrap();

    let views = account_views(&config, &secrets);

    assert_eq!(
        views,
        [
            AccountView {
                name: account("comp-claude"),
                owner: "comp".parse().unwrap(),
                agents: vec![agent("comp-claude-1"), agent("comp-inv-opus")],
                env: vec!["CLAUDE_CONFIG_DIR".parse().unwrap()],
                secrets: vec!["CLAUDE_CODE_OAUTH_TOKEN".parse().unwrap()],
            },
            AccountView {
                name: account("mateusz-codex"),
                owner: "mateusz".parse().unwrap(),
                agents: vec![agent("mateusz-codex-1")],
                env: Vec::new(),
                secrets: Vec::new(),
            },
        ]
    );
    assert!(!format!("{views:?}").contains(MARKER));
}

#[test]
fn a_configuration_without_accounts_has_no_account_view() {
    let config = Config::from_toml_str("[agents.codex-1]\ncli = \"codex\"\n[tiers]\nnormal = [\"codex-1\"]\n").unwrap();

    assert_eq!(account_views(&config, &Secrets::default()), Vec::<AccountView>::new());
}
