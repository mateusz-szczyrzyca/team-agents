//! `[accounts.NAME]`: the subscriptions and keys the human has, and the checks that tie them to the agents.
//!
//! An account names its `owner` and may set variables of the environment that are not secret (`env`); its secret
//! variables are `[account_env.NAME]` of `secrets.toml`. An agent or an investigator names its account with
//! `account = "NAME"` and is then named `<owner>-…`. This module only reads and checks; nothing runs with an account yet.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use super::values::{EnvName, InvalidValue, ValueRule, check_text, keep, refuse, text_value};
use super::{AgentConfig, Config, ContextKey, InvestigatorConfig, Secrets};
use crate::ids::{AgentName, InvestigatorName};

/// Longest account name, in bytes.
const MAX_ACCOUNT_NAME_LEN: usize = 64;

/// Longest owner, in bytes.
const MAX_OWNER_LEN: usize = 32;

/// Longest value of an `env` variable, in bytes: longer than any path or setting in use.
const MAX_ENV_VALUE_LEN: usize = 4096;

/// The name of an account in `[accounts.NAME]` and `[account_env.NAME]`, such as `comp-claude`.
///
/// 1 to 64 bytes of lowercase ASCII letters, digits and `-`, starting with a letter and not ending with `-`: the
/// rule of an agent's name. Refused, with the [`ValueRule`] it breaks: the empty text, a longer one, a first digit or
/// `-`, a last `-` and any other character.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct AccountName(String);

fn check_account_name(text: &str) -> Result<(), InvalidValue> {
    const WHAT: &str = "account name";
    check_text(WHAT, text, MAX_ACCOUNT_NAME_LEN, |c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
    })?;
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(refuse(WHAT, text, ValueRule::StartsWithDigit));
    }
    if text.starts_with('-') || text.ends_with('-') {
        return Err(refuse(WHAT, text, ValueRule::HyphenAtEdge));
    }
    Ok(())
}

text_value!(AccountName, check_account_name, keep);

/// The owner of an account, a lowercase word such as `mateusz` or `comp`: the first part of the names of the agents
/// that use the account.
///
/// 1 to 32 bytes of lowercase ASCII letters and digits, starting with a letter, so that `<owner>-…` is a valid agent
/// name. Refused, with the [`ValueRule`] it breaks: the empty text, a longer one, a first digit and any other
/// character (uppercase letters, spaces, `-`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct OwnerName(String);

fn check_owner_name(text: &str) -> Result<(), InvalidValue> {
    check_text("account owner", text, MAX_OWNER_LEN, |c| {
        c.is_ascii_lowercase() || c.is_ascii_digit()
    })?;
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(refuse("account owner", text, ValueRule::StartsWithDigit));
    }
    Ok(())
}

text_value!(OwnerName, check_owner_name, keep);

/// The value of a variable in an account's `env`: not a secret, so it may be shown.
///
/// 1 to 4096 bytes without control characters. Refused, with the [`ValueRule`] it breaks: the empty text, a longer
/// one and a control character.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct EnvValue(String);

fn check_env_value(text: &str) -> Result<(), InvalidValue> {
    check_text("environment variable value", text, MAX_ENV_VALUE_LEN, |c| {
        !c.is_control()
    })
}

text_value!(EnvValue, check_env_value, keep);

/// One account of `[accounts.NAME]`, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Whose subscription or key it is.
    owner: OwnerName,
    /// The variables of the environment of a run with this account that are not secret.
    env: BTreeMap<EnvName, EnvValue>,
}

impl Account {
    /// Whose subscription or key the account is.
    #[must_use]
    pub fn owner(&self) -> &OwnerName {
        &self.owner
    }

    /// The variables of the environment of a run with this account that are not secret, by name.
    #[must_use]
    pub fn env(&self) -> &BTreeMap<EnvName, EnvValue> {
        &self.env
    }
}

/// What `[accounts.NAME]` says in the file, before the owner is checked: a missing owner and a bad one are told apart
/// from a fault of the file's format, and name the account.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAccount {
    /// The `owner` key, as written.
    #[serde(default)]
    owner: Option<String>,
    /// The `env` table.
    #[serde(default)]
    env: BTreeMap<EnvName, EnvValue>,
}

/// Why the accounts of an `agents.toml` are refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    /// An account has no `owner`.
    #[error("account {account} has no `owner`: name whose subscription or key it is")]
    OwnerMissing {
        /// The account.
        account: AccountName,
    },
    /// An account's `owner` is not a lowercase word of letters and digits.
    #[error("account {account} has an invalid `owner`")]
    OwnerInvalid {
        /// The account.
        account: AccountName,
        /// The rule the owner breaks, with the text that was written.
        #[source]
        source: InvalidValue,
    },
    /// An agent or an investigator names an account that has no `[accounts.NAME]` table.
    #[error("{} of {name}: account {account} is not defined in [accounts]", key.section())]
    UnknownAccount {
        /// The section the agent is in: `agents` or `investigators`.
        key: ContextKey,
        /// The agent or investigator that names the account.
        name: String,
        /// The account that is not defined.
        account: AccountName,
    },
    /// An agent or an investigator with an account is not named `<owner>-…`.
    #[error(
        "{} of {name}: it uses account {account} of owner {owner}, so its name must start with \"{owner}-\"",
        key.section()
    )]
    NameWithoutOwner {
        /// The section the agent is in: `agents` or `investigators`.
        key: ContextKey,
        /// The agent or investigator whose name is wrong.
        name: String,
        /// The account it uses.
        account: AccountName,
        /// The owner of that account.
        owner: OwnerName,
    },
    /// No agent and no investigator names an account.
    #[error(
        "account {account} is used by no agent or investigator: name it with `account = \"{account}\"` or remove it"
    )]
    Unused {
        /// The account nobody uses.
        account: AccountName,
    },
}

/// Why the accounts of `secrets.toml` do not fit the accounts of `agents.toml`. Names only: never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountSecretsError {
    /// `[account_env.NAME]` is there, but `agents.toml` has no `[accounts.NAME]`.
    #[error("[account_env.{account}] belongs to no account: define [accounts.{account}] in agents.toml")]
    NoAccount {
        /// The account `secrets.toml` names.
        account: AccountName,
    },
    /// A variable is in the `env` of an account and in its `[account_env.NAME]`.
    #[error("account {account} sets {variable} twice: in `env` of agents.toml and in secrets.toml; keep one")]
    VariableTwice {
        /// The account.
        account: AccountName,
        /// The variable set in both places.
        variable: EnvName,
    },
}

/// Checks the accounts the file defines against the agents and the investigators that use them and returns them.
///
/// The first rule broken is the error. Checked in this order: every account's owner, account by account; then every
/// agent and investigator that names an account, agents first: the account must exist and the name must start with
/// `<owner>-`; at last that every account is named by one of them.
pub(super) fn check(
    accounts: BTreeMap<AccountName, RawAccount>,
    agents: &BTreeMap<AgentName, AgentConfig>,
    investigators: &BTreeMap<InvestigatorName, InvestigatorConfig>,
) -> Result<BTreeMap<AccountName, Account>, AccountError> {
    let accounts = accounts
        .into_iter()
        .map(|(name, raw)| check_account(&name, raw).map(|account| (name, account)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let users = agents
        .iter()
        .map(|(name, agent)| (ContextKey::Agent, name.as_str(), agent.account.as_ref()))
        .chain(
            investigators
                .iter()
                .map(|(name, agent)| (ContextKey::Investigator, name.as_str(), agent.account.as_ref())),
        );
    for (key, name, account) in users {
        if let Some(account) = account {
            check_user(&accounts, key, name, account)?;
        }
    }
    let used = |account: &AccountName| {
        agents.values().any(|agent| agent.account.as_ref() == Some(account))
            || investigators
                .values()
                .any(|agent| agent.account.as_ref() == Some(account))
    };
    match accounts.keys().find(|account| !used(account)) {
        Some(account) => Err(AccountError::Unused {
            account: account.clone(),
        }),
        None => Ok(accounts),
    }
}

/// Checks the owner of the account `name` and makes the [`Account`].
fn check_account(name: &AccountName, raw: RawAccount) -> Result<Account, AccountError> {
    let owner = raw
        .owner
        .ok_or_else(|| AccountError::OwnerMissing { account: name.clone() })?;
    let owner = owner
        .parse::<OwnerName>()
        .map_err(|source| AccountError::OwnerInvalid {
            account: name.clone(),
            source,
        })?;
    Ok(Account { owner, env: raw.env })
}

/// Checks that the agent or investigator `name` of `key`, which names `account`, has an account to use and is named
/// after its owner.
fn check_user(
    accounts: &BTreeMap<AccountName, Account>,
    key: ContextKey,
    name: &str,
    account: &AccountName,
) -> Result<(), AccountError> {
    let found = accounts.get(account).ok_or_else(|| AccountError::UnknownAccount {
        key,
        name: name.to_owned(),
        account: account.clone(),
    })?;
    let owner = found.owner();
    let named_after_owner = name
        .strip_prefix(owner.as_str())
        .is_some_and(|rest| rest.starts_with('-'));
    if named_after_owner {
        return Ok(());
    }
    Err(AccountError::NameWithoutOwner {
        key,
        name: name.to_owned(),
        account: account.clone(),
        owner: owner.clone(),
    })
}

/// Checks the `[account_env.NAME]` tables of `secrets` against the accounts of `config`, table by table in the order
/// of the account names, variable by variable in the order of their names.
pub(super) fn check_secrets(config: &Config, secrets: &Secrets) -> Result<(), AccountSecretsError> {
    for (account, variables) in secrets.account_env() {
        let Some(found) = config.accounts().get(account) else {
            return Err(AccountSecretsError::NoAccount {
                account: account.clone(),
            });
        };
        if let Some(variable) = variables.keys().find(|variable| found.env().contains_key(*variable)) {
            return Err(AccountSecretsError::VariableTwice {
                account: account.clone(),
                variable: variable.clone(),
            });
        }
    }
    Ok(())
}
