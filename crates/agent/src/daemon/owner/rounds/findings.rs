//! Numbered findings in prompts and the single editable GitHub comment.

use agent_core::ids::TeamId;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables, VariablesError};
use agent_core::review_findings::Severity;
use agent_core::state::{FindingStatus, Findings};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};
use agent_runtime::process::error_chain;

use super::super::Owner;
use super::verdicts::Asked;
use super::{Lead, PublishError, Pulls};

/// Why a findings text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon::owner) enum TextError {
    /// A catalog variable is invalid.
    #[error("cannot set findings variables")]
    Variable(#[from] VariablesError),
    /// A catalog template could not be rendered.
    #[error("cannot render findings text")]
    Render(#[from] RenderAutopilotTextError),
}

/// The open findings, with permanent numbers and verification checks.
pub(super) fn open_text(texts: &AutopilotPrompts, ledger: Option<&Findings>) -> Result<String, TextError> {
    let mut lines = Vec::new();
    if let Some(ledger) = ledger {
        for entry in ledger
            .entries()
            .iter()
            .filter(|entry| entry.status == FindingStatus::Open)
        {
            let values = Variables::new()
                .with("number", entry.number.to_string())?
                .with("severity", entry.finding.severity.word())?
                .with("rule", &entry.finding.rule)?
                .with("place", &entry.finding.place)?
                .with("text", &entry.finding.text)?
                .with("check", &entry.finding.check)?;
            lines.push(texts.render(AutopilotPrompt::ReviewOpenFinding, &values)?);
        }
    }
    Ok(lines.join("\n"))
}

/// Open blocking findings for a human deciding whether to renew the fix budget.
pub(super) fn blocking_text(_texts: &AutopilotPrompts, _ledger: Option<&Findings>) -> Result<String, TextError> {
    Ok(String::new())
}

/// Escapes report data as one Markdown table cell without interpreting its markup.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
}

/// The complete findings table, including resolved and nonblocking entries.
fn table(texts: &AutopilotPrompts, ledger: &Findings) -> Result<String, TextError> {
    let mut rows = Vec::new();
    for entry in ledger.entries() {
        let severity = match entry.finding.severity {
            Severity::Blocker => AutopilotPrompt::ReviewSeverityBlocker,
            Severity::Major => AutopilotPrompt::ReviewSeverityMajor,
            Severity::Minor => AutopilotPrompt::ReviewSeverityMinor,
        };
        let severity = texts.render(severity, &Variables::new())?;
        let status = match entry.status {
            FindingStatus::Open => "open",
            FindingStatus::Resolved => "resolved",
        };
        let values = Variables::new()
            .with("number", entry.number.to_string())?
            .with("severity", severity.trim())?
            .with("rule", cell(&entry.finding.rule))?
            .with("place", cell(&entry.finding.place))?
            .with("text", cell(&entry.finding.text))?
            .with("status", status)?
            .with("reporter", entry.reporter.as_str())?;
        rows.push(texts.render(AutopilotPrompt::ReviewFindingsRow, &values)?);
    }
    Ok(texts.render(
        AutopilotPrompt::ReviewFindingsTable,
        &Variables::new().with("rows", rows.join("").trim_end())?,
    )?)
}

impl Owner {
    /// A failed table publication is logged; reports and decisions remain authoritative.
    pub(super) async fn post_findings(&mut self, pulls: &Pulls, lead: &Lead, team: &TeamId, asked: &Asked) {
        if let Err(error) = self.try_post_findings(pulls, lead, team, asked).await {
            tracing::warn!(event = LogEvent::GithubCallFailed.as_str(), pr = %asked.pr.number,
                error = %error_chain(&error), "cannot publish findings table: the round continues");
        }
    }

    /// Creates a findings comment once and subsequently edits its persisted ID.
    async fn try_post_findings(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        team_id: &TeamId,
        asked: &Asked,
    ) -> Result<(), PublishError> {
        let Some(ledger) = asked
            .items
            .first()
            .and_then(|id| self.parts.state.team(team_id)?.item(*id)?.findings())
            .cloned()
        else {
            return Ok(());
        };
        if ledger.entries().is_empty() {
            return Ok(());
        }
        let body = table(&pulls.texts, &ledger)?;
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let comment = match ledger.comment {
            Some(id) => github.edit_comment(id, &body).await?,
            None => github.post_comment(asked.pr.number, &body).await?,
        };
        for id in &asked.items {
            if let Some(mut team) = self.parts.state.team_mut(team_id)
                && let Some(mut item) = team.item_mut(*id)
                && let Some(ledger) = item.findings_mut()
            {
                ledger.comment = Some(comment.id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
