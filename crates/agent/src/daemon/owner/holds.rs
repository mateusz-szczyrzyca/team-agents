//! The hold of an account at its `hold_at`: while a window of its limits is used up to that share, no new run of its
//! agents starts, until the window resets. The runs going finish, the queued ones wait in the queue, nothing fails.
//!
//! [`agent_core::decide::account_hold::held_accounts`] is the rule and follows from the readings the state keeps, so a
//! restart of the daemon keeps a hold. This module asks it, keeps the agents of a held account from being picked, and
//! tells the human once when a hold begins and the log when it begins and ends.

use std::collections::{BTreeMap, BTreeSet};

use agent_core::config::AccountName;
use agent_core::decide::account_hold::{Hold, held_accounts};
use agent_core::ids::AgentName;
use agent_core::limit_reading::LimitWindow;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::state::Timestamp;

use super::{Owner, error_chain};

/// The holds the daemon has told, by account, with the end of each. Boxed in the owner: a plain map there would make the
/// futures of the code that holds an owner across an await larger than clippy's limit.
#[derive(Debug, Default)]
pub(super) struct KnownHolds(BTreeMap<AccountName, Timestamp>);

impl Owner {
    /// The accounts that are held at `now`.
    pub(super) fn held_account_names(&self, now: Timestamp) -> BTreeSet<AccountName> {
        held_accounts(&self.parts.state, &self.parts.settings.config, now)
            .into_keys()
            .collect()
    }

    /// Whether `agent` has an account that is in `held`: none of its runs starts.
    pub(super) fn agent_is_held(&self, agent: &str, held: &BTreeSet<AccountName>) -> bool {
        self.account_named(agent).is_some_and(|account| held.contains(&account))
    }

    /// The agents and the investigators of the configuration whose account is held at `now`, agents first, each in name
    /// order. They are picked for nothing, like those of a paused account.
    pub(super) fn agents_of_held_accounts(&self, now: Timestamp) -> Vec<AgentName> {
        let held = self.held_account_names(now);
        if held.is_empty() {
            return Vec::new();
        }
        let config = &self.parts.settings.config;
        let agents = config
            .agents()
            .iter()
            .filter(|(_, agent)| agent.account.as_ref().is_some_and(|account| held.contains(account)))
            .map(|(name, _)| name.clone());
        let investigators = config
            .investigators()
            .iter()
            .filter(|(_, agent)| agent.account.as_ref().is_some_and(|account| held.contains(account)))
            .map(|(name, _)| AgentName::from(name));
        agents.chain(investigators).collect()
    }

    /// Takes the holds of the state as the daemon knows them, without telling anyone: a hold that began before a
    /// restart was told then.
    pub(super) fn remember_holds(&mut self, now: Timestamp) {
        self.account_holds.0 = held_accounts(&self.parts.state, &self.parts.settings.config, now)
            .into_iter()
            .filter_map(|(account, hold)| match hold {
                Hold::Open => None,
                Hold::Held { until, .. } => Some((account, until)),
            })
            .collect();
    }

    /// Compares the holds at `now` with the ones the daemon knows: logs each hold that ended, and logs and tells the human
    /// of each that began.
    pub(super) async fn update_holds(&mut self, now: Timestamp) {
        let current = held_accounts(&self.parts.state, &self.parts.settings.config, now);
        let ended: Vec<AccountName> = self
            .account_holds
            .0
            .keys()
            .filter(|account| !current.contains_key(*account))
            .cloned()
            .collect();
        for account in ended {
            self.account_holds.0.remove(&account);
            tracing::debug!(
                event = LogEvent::DecisionTaken.as_str(),
                decision = "account_released",
                account = %account,
                result = "release",
                "the hold of an account ended: its queued runs start"
            );
        }
        for (account, hold) in current {
            match hold {
                Hold::Open => {}
                Hold::Held { window, used, until } => {
                    if self.account_holds.0.insert(account.clone(), until).is_none() {
                        Box::pin(self.begin_hold(&account, window, used, until)).await;
                    }
                }
            }
        }
    }

    /// Logs the hold of `account` that begins and tells the human, once.
    async fn begin_hold(&mut self, account: &AccountName, window: LimitWindow, used: f64, until: Timestamp) {
        let agents = self.agents_of(account);
        let hold_at = self
            .parts
            .settings
            .config
            .accounts()
            .get(account)
            .and_then(agent_core::config::Account::hold_at)
            .map_or(0.0, agent_core::config::HoldAt::share);
        tracing::debug!(
            event = LogEvent::DecisionTaken.as_str(),
            decision = "account_held",
            account = %account,
            window = window_words(window),
            used = %percent(used),
            hold_at = %percent(hold_at),
            until = %until,
            agents = %agents.join(","),
            result = "hold",
            "an account is held: no run of its agents starts before its window resets"
        );
        let values = Variables::new()
            .with("account", account.as_str())
            .and_then(|v| v.with("agents", agents.join(", ")))
            .and_then(|v| v.with("until", self.parts.zone.stamp_minute(until)))
            .and_then(|v| v.with("window", window_words(window)))
            .and_then(|v| v.with("used", percent(used)))
            .and_then(|v| v.with("hold_at", percent(hold_at)));
        let values = match values {
            Ok(values) => values,
            Err(error) => {
                tracing::warn!(event = LogEvent::NotificationFailed.as_str(), error = %error_chain(&error), "cannot build the variables of the hold notice");
                return;
            }
        };
        let rendered = match self.parts.texts.render(HarnessPrompt::NoticeAccountHeld, &values) {
            Ok(rendered) => rendered,
            Err(error) => {
                tracing::warn!(event = LogEvent::NotificationFailed.as_str(), error = %error_chain(&error), "cannot render the hold notice");
                return;
            }
        };
        let (title, body) = rendered.split_once('\n').unwrap_or((rendered.as_str(), ""));
        self.deliver(title.trim(), body.trim(), "hold notice").await;
    }
}

/// The window as the notice and the log say it.
const fn window_words(window: LimitWindow) -> &'static str {
    match window {
        LimitWindow::FiveHour => "five-hour",
        LimitWindow::SevenDay => "seven-day",
    }
}

/// A share as a whole percent: `62%`.
fn percent(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
}
