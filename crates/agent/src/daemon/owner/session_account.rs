//! The account of a session: a task whose agent has another account than the one its session began under is started
//! anew before its next run, as `agent renew` starts one. [`session_start`] is the decision; this module gathers what it
//! takes and carries it out.

use agent_core::config::AccountName;
use agent_core::decide::session_account::{AccountSession, session_start};
use agent_core::ids::{AgentName, TaskId};
use agent_runtime::history::log_decision;

use super::{Owner, OwnerError};

/// How an account is told in a log line: its name, or `none`.
fn shown(account: Option<&AccountName>) -> String {
    account.map_or_else(|| "none".to_owned(), ToString::to_string)
}

impl Owner {
    /// Starts `task` anew in a new session when its agent has another account than the one the session of the task began
    /// under, so that the briefing below is of the new session, and records the account the session runs under. Returns
    /// whether a new session began.
    ///
    /// A task with no session to resume — its first run, or a session that was just begun for another reason, which is
    /// briefed already — only has the account recorded: it is never briefed twice. Every role is started anew, a worker, a
    /// reviewer and an investigator too: a session of another account is resumed by none.
    ///
    /// # Errors
    /// [`OwnerError`] when the state refuses the new session.
    pub(super) fn renew_for_account(&mut self, task: &TaskId) -> Result<bool, OwnerError> {
        let (agent, resumable) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            (
                found.member.agent().name().to_owned(),
                found.member.last_session().is_some(),
            )
        };
        let agent: AgentName = agent.parse()?;
        // An agent the configuration no longer has is refused when the run is planned: no session is begun for it here.
        if self.participant_of(&agent).is_none() {
            return Ok(false);
        }
        let now = self.account_of(&agent);
        let recorded = self.parts.state.session_account(task).cloned();
        if session_start(recorded.as_ref(), now.as_ref()) == AccountSession::Resume {
            return Ok(false);
        }
        if !resumable {
            self.parts.state.record_session_account(task, now);
            return Ok(false);
        }
        let at = self.after_last_run(task)?;
        let begun = self.parts.state.begin_account_session(task, now.clone(), at)?;
        if begun {
            self.queue_for_task(task, agent_core::history::CheckpointCause::HolderChanged);
        }
        log_decision(
            "session_account",
            &format!(
                "task={task}, recorded={}, now={}",
                shown(recorded.as_ref()),
                shown(now.as_ref())
            ),
            &"NewSession",
        );
        Ok(begun)
    }
}
