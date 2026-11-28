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
    let agents = config
        .agents()
        .iter()
        .filter_map(|(name, agent)| agent.account.clone().map(|account| (name.clone(), account)));
    let investigators = config.investigators().iter().filter_map(|(name, investigator)| {
        investigator
            .account
            .clone()
            .map(|account| (AgentName::from(name), account))
    });
    agents.chain(investigators).collect()
}

/// `agent` as `agent status` writes it: followed by its account in brackets when it has one.
#[must_use]
pub fn labelled(agent: &str, account: Option<&AccountName>) -> String {
    account.map_or_else(|| agent.to_owned(), |account| format!("{agent} [{account}]"))
}

/// Every account of `config` with its agents and the names of its variables, in the order of the accounts' names.
#[must_use]
pub fn account_views(config: &Config, secrets: &Secrets) -> Vec<AccountView> {
    let of_agents = agent_accounts(config);
    config
        .accounts()
        .iter()
        .map(|(name, account)| AccountView {
            name: name.clone(),
            owner: account.owner().clone(),
            agents: of_agents
                .iter()
                .filter(|(_, of)| *of == name)
                .map(|(agent, _)| agent.clone())
                .collect(),
            env: account.env().keys().cloned().collect(),
            secrets: secrets
                .account_env()
                .get(name)
                .map_or_else(Vec::new, |variables| variables.keys().cloned().collect()),
        })
        .collect()
}
