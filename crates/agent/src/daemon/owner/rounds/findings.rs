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
    /// The answers a verifier may give could not be rendered.
    #[error("cannot render the answers of a finding")]
    Answers(#[from] super::responses::ResponseTextError),
}

/// The open findings, with permanent numbers and verification checks.
pub(super) fn open_text(texts: &AutopilotPrompts, ledger: Option<&Findings>) -> Result<String, TextError> {
    selected_text(texts, ledger, |_| true)
}

/// The open findings for a verifier, each with the answers it may get beside it.
pub(super) fn verification_text(texts: &AutopilotPrompts, ledger: Option<&Findings>) -> Result<String, TextError> {
    let Some(ledger) = ledger else {
        return Ok(String::new());
    };
    let answers = ledger.verifier_answers();
    let mut blocks = Vec::new();
    for offered in answers {
        let finding = selected_text(texts, Some(ledger), |entry| entry.number == offered.number)?;
        let allowed = super::responses::verifier_allowed(texts, offered)?;
        let line = texts.render(
            AutopilotPrompt::ReviewVerifierAnswer,
            &Variables::new()
                .with("number", offered.number.to_string())?
                .with("allowed", allowed)?,
        )?;
        blocks.push(format!("{}\n{}", finding.trim_end(), line.trim_end()));
    }
    Ok(blocks.join("\n"))
}

/// Renders the selected open entries with their verification checks.
pub(super) fn selected_text(
    texts: &AutopilotPrompts,
    ledger: Option<&Findings>,
    selected: impl Fn(&agent_core::state::RecordedFinding) -> bool,
) -> Result<String, TextError> {
    let mut lines = Vec::new();
    if let Some(ledger) = ledger {
        for entry in ledger
            .entries()
            .iter()
            .filter(|entry| entry.status == FindingStatus::Open && selected(entry))
        {
            let values = Variables::new()
                .with("number", entry.number.to_string())?
                .with("severity", entry.finding.severity.word())?
                .with("rule", &entry.finding.rule)?
                .with("place", &entry.finding.place)?
                .with("text", &entry.finding.text)?
                .with("check", &entry.finding.check)?;
            lines.push(texts.render(AutopilotPrompt::ReviewOpenFinding, &values)?);
            let opinion = opinion_text(texts, ledger, entry.number)?;
            if !opinion.is_empty() {
                lines.push(opinion);
            }
            let explanation = explanation_text(texts, ledger, entry.number)?;
            if !explanation.is_empty() {
                lines.push(explanation);
            }
        }
    }
    Ok(lines.join("\n"))
}

/// The retained explanation and each verifier's evidence, rendered from catalog texts.
fn explanation_text(
    texts: &AutopilotPrompts,
    ledger: &Findings,
    number: std::num::NonZeroU32,
) -> Result<String, TextError> {
    use agent_core::review_findings::Answer;
    use agent_core::state::ExplanationStatus;
    let Some(explanation) = ledger.disputes().and_then(|d| d.explanations.get(&number)) else {
        return Ok(String::new());
    };
    let status = match explanation.status {
        ExplanationStatus::Pending => "pending",
        ExplanationStatus::Accepted => "accepted",
        ExplanationStatus::Rejected => "rejected",
    };
    let mut votes = Vec::new();
    for (reviewer, answer) in &explanation.votes {
        let (status, why) = match answer {
            Answer::Accepted { why } => ("ACCEPTED", why.as_str()),
            Answer::Rejected { why } => ("REJECTED", why.as_str()),
            Answer::Raised { why, .. } => ("RAISED", why.as_str()),
            Answer::Open { why } => ("OPEN", why.as_str()),
            Answer::Resolved => ("RESOLVED", ""),
        };
        votes.push(
            texts.render(
                AutopilotPrompt::ReviewExplanationVote,
                &Variables::new()
                    .with("reviewer", reviewer.as_str())?
                    .with("status", status)?
                    .with("why", why)?,
            )?,
        );
    }
    Ok(texts.render(
        AutopilotPrompt::ReviewExplanation,
        &Variables::new()
            .with("status", status)?
            .with("why", &explanation.why)?
            .with("votes", votes.join(""))?,
    )?)
}

/// Open blocking findings for a human deciding whether to renew the fix budget.
pub(super) fn blocking_text(texts: &AutopilotPrompts, ledger: Option<&Findings>) -> Result<String, TextError> {
    selected_text(texts, ledger, |entry| entry.finding.severity.blocks())
}

/// The longest finding text the comment shows, in characters; a longer text is cut at a word boundary.
const COMMENT_TEXT_LIMIT: usize = 160;

/// Collapses every run of whitespace to one space, so that a text stays on one Markdown line.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escapes the characters of report data that would open markup or break a table bar in a comment line.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
}

/// Cuts `text` to at most `limit` characters, at the last space before the limit when there is one, with `…` when cut.
fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    let kept = head.rsplit_once(' ').map_or(head.as_str(), |(kept, _)| kept);
    format!("{kept}…")
}

/// The finding text as a comment line: a leading character that Markdown reads as a block start is written as a
/// character reference, so the line stays a paragraph line of its entry.
fn text_line(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_punctuation() || first.is_ascii_digit() => {
            format!("&#{};{}", u32::from(first), escape(chars.as_str()))
        }
        _ => escape(text),
    }
}

/// Whether the comment folds a finding among the closed ones: fixed, not confirmed, or explained and accepted.
fn is_closed(entry: &agent_core::state::RecordedFinding, ledger: &Findings) -> bool {
    use agent_core::state::ExplanationStatus;
    entry.status != FindingStatus::Open
        || ledger
            .disputes()
            .and_then(|disputes| disputes.explanations.get(&entry.number))
            .is_some_and(|explanation| explanation.status == ExplanationStatus::Accepted)
}

/// One finding of the comment: its first line, its text and, when there is one, its explanation and second opinion
/// on a third line.
fn entry_text(
    texts: &AutopilotPrompts,
    ledger: &Findings,
    entry: &agent_core::state::RecordedFinding,
) -> Result<String, TextError> {
    let severity = match entry.finding.severity {
        Severity::Blocker => AutopilotPrompt::ReviewSeverityBlocker,
        Severity::Major => AutopilotPrompt::ReviewSeverityMajor,
        Severity::Minor => AutopilotPrompt::ReviewSeverityMinor,
    };
    let severity = texts.render(severity, &Variables::new())?;
    let state = match entry.status {
        FindingStatus::Open => "open",
        FindingStatus::Resolved => "resolved",
        FindingStatus::NotConfirmed => "not confirmed",
    };
    let mut notes = Vec::new();
    for note in [
        explanation_text(texts, ledger, entry.number)?,
        opinion_text(texts, ledger, entry.number)?,
    ] {
        if !note.is_empty() {
            notes.push(escape(&collapse(&note)));
        }
    }
    let note = if notes.is_empty() {
        String::new()
    } else {
        format!("\n{}", notes.join(" · "))
    };
    let values = Variables::new()
        .with("number", entry.number.to_string())?
        .with("severity", severity.trim())?
        .with("rule", escape(&collapse(&entry.finding.rule)))?
        .with("place", escape(&collapse(&entry.finding.place)))?
        .with("state", state)?
        .with("reporter", escape(&collapse(entry.reporter.as_str())))?
        .with(
            "text",
            text_line(&cut(&collapse(&entry.finding.text), COMMENT_TEXT_LIMIT)),
        )?
        .with("note", note)?;
    Ok(texts
        .render(AutopilotPrompt::ReviewFindingEntry, &values)?
        .trim_end()
        .to_owned())
}

/// The findings comment: the open entries in number order, then the closed ones folded in one `<details>` block.
fn comment(texts: &AutopilotPrompts, ledger: &Findings) -> Result<String, TextError> {
    let mut open_entries = Vec::new();
    let mut closed_entries = Vec::new();
    let (mut blockers, mut majors, mut minors) = (0_usize, 0_usize, 0_usize);
    for entry in ledger.entries() {
        let text = entry_text(texts, ledger, entry)?;
        if is_closed(entry, ledger) {
            closed_entries.push(text);
            continue;
        }
        match entry.finding.severity {
            Severity::Blocker => blockers = blockers.saturating_add(1),
            Severity::Major => majors = majors.saturating_add(1),
            Severity::Minor => minors = minors.saturating_add(1),
        }
        open_entries.push(text);
    }
    let mut sections = Vec::new();
    if !open_entries.is_empty() {
        sections.push(open_entries.join("\n\n"));
    }
    if !closed_entries.is_empty() {
        let block = texts.render(
            AutopilotPrompt::ReviewFindingsClosed,
            &Variables::new()
                .with("count", closed_entries.len().to_string())?
                .with("entries", closed_entries.join("\n\n"))?,
        )?;
        sections.push(block.trim_end().to_owned());
    }
    Ok(texts.render(
        AutopilotPrompt::ReviewFindingsList,
        &Variables::new()
            .with("blockers", blockers.to_string())?
            .with("majors", majors.to_string())?
            .with("minors", minors.to_string())?
            .with("closed", closed_entries.len().to_string())?
            .with("entries", sections.join("\n\n"))?,
    )?)
}

impl Owner {
    /// A failed comment publication is logged; reports and decisions remain authoritative.
    pub(super) async fn post_findings(&mut self, pulls: &Pulls, lead: &Lead, team: &TeamId, asked: &Asked) {
        if let Err(error) = self.try_post_findings(pulls, lead, team, asked).await {
            tracing::warn!(event = LogEvent::GithubCallFailed.as_str(), pr = %asked.pr.number,
                error = %error_chain(&error), "cannot publish findings comment: the round continues");
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
        let body = comment(&pulls.texts, &ledger)?;
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

/// Investigator and evidence of a settled second opinion.
fn opinion_text(
    texts: &AutopilotPrompts,
    ledger: &Findings,
    number: std::num::NonZeroU32,
) -> Result<String, TextError> {
    let Some(agent_core::state::SecondOpinion {
        stage: agent_core::state::OpinionStage::Reported {
            investigator, report, ..
        },
        ..
    }) = ledger.opinions().get(&number)
    else {
        return Ok(String::new());
    };
    let outcome = match report.outcome {
        agent_core::review_response::opinion::Outcome::Confirmed => "CONFIRMED",
        agent_core::review_response::opinion::Outcome::NotConfirmed => "NOT CONFIRMED",
    };
    Ok(texts.render(
        AutopilotPrompt::ReviewOpinionResult,
        &Variables::new()
            .with("reviewer", investigator.as_str())?
            .with("outcome", outcome)?
            .with("evidence", report.evidence.as_str())?,
    )?)
}
