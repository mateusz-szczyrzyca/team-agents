//! The hold of an account at its `hold_at`: while a window of its limits is used up to that share, no new run of its
//! agents starts, until the window resets. The runs going finish, the queued ones wait in the queue, nothing fails.
//!
//! [`agent_core::decide::account_hold::held_accounts`] is the rule that a kept reading holds an account; the state records
//! the hold when it begins, so that it lasts until the window resets whatever is read after it, and a restart of the
//! daemon keeps it. This module asks the rule, keeps the agents of a held account from being picked, and tells the human
//! once when a hold begins and the log when it begins and ends.

use std::collections::BTreeSet;

use agent_core::config::AccountName;
use agent_core::decide::account_hold::{Hold, held_accounts, held_until};
use agent_core::ids::AgentName;
use agent_core::limit_reading::LimitWindow;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::state::Timestamp;

use super::{Owner, error_chain};

impl Owner {
    /// The accounts that are held at `now`: those the state records a hold of and those a kept reading holds.
    pub(super) fn held_account_names(&self, now: Timestamp) -> BTreeSet<AccountName> {
        held_until(&self.parts.state, &self.parts.settings.config, now)
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

    /// Brings the holds the state records to those at `now`: a hold that a reading begins is recorded, logged and told to
    /// the human, once; a recorded hold whose window has reset, or whose account has no `hold_at` any more, is forgotten
    /// and logged. A hold is recorded when it begins, so a later reading below `hold_at` does not end it and a daemon that
    /// starts again does not tell it again.
    ///
    /// Returns whether the state changed, so that its caller saves it.
    pub(super) async fn update_holds(&mut self, now: Timestamp) -> bool {
        let (state, config) = (&self.parts.state, &self.parts.settings.config);
        let live = held_until(state, config, now);
        let ended: Vec<AccountName> = state
            .account_holds()
            .map(|(account, _)| account.clone())
            .filter(|account| !live.contains_key(account))
            .collect();
        let by_readings = held_accounts(state, config, now);
        let mut changed = !ended.is_empty();
        for account in ended {
            self.parts.state.record_account_hold(account.clone(), None);
            tracing::debug!(
                event = LogEvent::DecisionTaken.as_str(),
                decision = "account_released",
                account = %account,
                result = "release",
                "the hold of an account ended: its queued runs start"
            );
        }
        for (account, hold) in by_readings {
            let Hold::Held { window, used, until } = hold else {
                continue;
            };
            match self.parts.state.account_hold_until(&account) {
                None => {
                    self.parts.state.record_account_hold(account.clone(), Some(until));
                    changed = true;
                    Box::pin(self.begin_hold(&account, window, used, until)).await;
                }
                Some(recorded) if recorded < until => {
                    // A second window at `hold_at` that resets later: the same hold, longer.
                    self.parts.state.record_account_hold(account, Some(until));
                    changed = true;
                }
                Some(_) => {}
            }
        }
        changed
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
