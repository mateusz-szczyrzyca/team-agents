//! The accounts of the configuration as the human sees them: the account of each agent, the label `agent [account]`
//! of `agent status`, and each account with its agents and the names — never the values — of its variables.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::config::{AccountName, Config, EnvName, OwnerName, Secrets};
use crate::ids::AgentName;

/// One account of `agents.toml` with who uses it and which variables it sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    /// The account's name.
    pub name: AccountName,
    /// Whose subscription or key it is.
    pub owner: OwnerName,
    /// The agents and investigators that run with it, in the order of their names.
    pub agents: Vec<AgentName>,
    /// The names of the variables its `env` sets; their values are not secret but are not listed here.
    pub env: Vec<EnvName>,
    /// The names of the variables its table of `secrets.toml` sets; the values are never read here.
    pub secrets: Vec<EnvName>,
}

/// The account each agent and investigator of `config` runs with; an agent without one is not in the map.
#[must_use]
pub fn agent_accounts(config: &Config) -> BTreeMap<AgentName, AccountName> {
    let _ = config;
    BTreeMap::new()
}

/// `agent` as `agent status` writes it: followed by its account in brackets when it has one.
#[must_use]
pub fn labelled(agent: &str, account: Option<&AccountName>) -> String {
    let _ = account;
    agent.to_owned()
}

/// Every account of `config` with its agents and the names of its variables, in the order of the accounts' names.
#[must_use]
pub fn account_views(config: &Config, secrets: &Secrets) -> Vec<AccountView> {
    let _ = (config, secrets);
    Vec::new()
}
