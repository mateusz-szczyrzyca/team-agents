//! Single-subject projections of validated history records.
use super::{Format, LogError};
use agent_core::history::{EndKind, HistoryEvent, Record};
use agent_core::ids::{RoundId, TeamId};
use agent_core::log::LogView;
use agent_core::story::{Selection, json_lines};
use agent_runtime::history::Reader;
use std::collections::BTreeMap;
use std::io::Write;

/// Prints one projection. All source records are validated by the history reader.
pub(super) fn print(
    reader: &Reader,
    round: Option<RoundId>,
    team: Option<&TeamId>,
    view: LogView,
    format: Format,
    out: &mut impl Write,
) -> Result<(), LogError> {
    let (selection, records, runs) = if let Some(id) = round {
        (
            Selection::Round(id.clone()),
            reader.round_records(&id)?,
            reader.runs_of_round(&id)?,
        )
    } else if let Some(id) = team {
        (
            Selection::Team(id.clone()),
            reader.team_records(id)?,
            reader.runs_of_team(id)?,
        )
    } else {
        return Err(LogError::NoSubject);
    };
    let records: Vec<_> = records
        .into_iter()
        .filter(|r| team.is_none_or(|team| *team == r.team))
        .collect();
    let runs: Vec<_> = runs
        .into_iter()
        .filter(|r| team.is_none_or(|team| *team == r.start.value.team))
        .collect();
    if view == LogView::Usage {
        return output(
            out,
            &serde_json::json!({"message":"Tokens are not recorded yet (11.1)."}),
            format,
        );
    }
    if matches!(
        view,
        LogView::Runs | LogView::Prompts | LogView::Changes | LogView::Reviews
    ) {
        let selected: Vec<_> = records
            .into_iter()
            .filter(|r| match (&r.record, view) {
                (Record::Run(_), LogView::Runs | LogView::Prompts) | (Record::Changes(_), LogView::Changes) => true,
                (Record::Event(e), LogView::Reviews) => matches!(
                    e.event,
                    HistoryEvent::ReviewAsked { .. } | HistoryEvent::ReviewSubmitted { .. }
                ),
                _ => false,
            })
            .collect();
        if view == LogView::Prompts {
            for record in selected {
                if let Record::Run(run) = record.record {
                    output(
                        out,
                        &serde_json::json!({"run":run.run,"task":run.task,"agent":run.agent.name,"prompt":run.prompt}),
                        format,
                    )?;
                }
            }
        } else {
            for line in json_lines(&selected, &selection) {
                writeln!(out, "{line}").map_err(LogError::Output)?;
            }
        }
    } else if view == LogView::Agents {
        let mut participants = BTreeMap::new();
        for record in &records {
            if let Record::Team(start) = &record.record {
                for entry in &start.plan {
                    if matches!(selection, Selection::Team(_))
                        || runs
                            .iter()
                            .any(|run| run.start.value.agent.name == entry.agent.as_str())
                    {
                        participants.insert(entry.agent.to_string(),serde_json::json!({"agent":entry.agent,"role":entry.role,"how":entry.how.kind().to_string()}));
                    }
                }
            }
        }
        for record in &records {
            if let Record::Event(event) = &record.record
                && let HistoryEvent::ReviewAsked { reviewer, among, .. } = &event.event
            {
                participants.insert(
                    reviewer.to_string(),
                    serde_json::json!({"agent":reviewer,"role":"reviewer","how":"drawn","among":among}),
                );
            }
        }
        for value in participants.values() {
            output(out, value, format)?;
        }
    } else {
        print_latest(&runs, view, format, out)?;
    }
    out.flush().map_err(LogError::Output)
}

/// Writes a projection row without mixing diagnostics into stdout.
fn output(out: &mut impl Write, value: &serde_json::Value, format: Format) -> Result<(), LogError> {
    match format {
        Format::Json => writeln!(out, "{value}"),
        Format::Text(_) => writeln!(out, "{value:#}"),
    }
    .map_err(LogError::Output)
}

/// Projects the most recent recorded run of each task.
fn print_latest(
    runs: &[agent_runtime::history::RunRow],
    view: LogView,
    format: Format,
    out: &mut impl Write,
) -> Result<(), LogError> {
    let mut latest = BTreeMap::new();
    for run in runs {
        latest.insert(run.start.value.task.clone(), run);
    }
    for row in latest.values() {
        let run = &row.start.value;
        let value = if view == LogView::Tasks {
            serde_json::json!({"task":run.task,"agent":run.agent.name,"role":run.role,"status":row.end.as_ref().map(|end| EndKind::of(&end.value.end).to_string()),"end":row.end.as_ref().map(|end| &end.value.end)})
        } else {
            serde_json::json!({"task":run.task,"worktree":run.worktree,"branch":run.branch,"head":row.launch.as_ref().and_then(|launch| launch.value.head.as_ref())})
        };
        output(out, &value, format)?;
    }
    Ok(())
}
