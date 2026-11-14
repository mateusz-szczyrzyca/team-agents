//! The JSON of a record: one object for each record, with its place in the order of writes, its time, team, rounds, kind and
//! the fields that were cut, and the record itself under `record`.
//!
//! Every field of the record is there, whole: a prompt or a report is the string that was stored.

use serde_json::{Value, json};

use crate::cli::ActivityEvent;
use crate::config::{EnvName, ModelName};
use crate::decide::checks::{Change, FileChange, Lines};
use crate::history::{
    EndKind, HistoryEvent, HowChosen, Launch, PlanEntry, Record, RoundOpening, RoundSubject, RunChanges, RunEndRecord,
    RunSession, RunStart, RunWhy, SequencedRecord, Step, StepKind, TeamStart,
};
use crate::ids::{AgentName, RoundId, RunId};
use crate::protocol::Client;
use crate::state::{CommitId, RunEnd, SessionId};

/// The keywords of the records that have tables of their own, as the ledger keeps them.
#[derive(Debug, Clone, Copy, strum::Display)]
#[strum(serialize_all = "snake_case")]
enum PlainKind {
    /// A team starting.
    Team,
    /// A round opening.
    Round,
    /// A run starting.
    Run,
    /// The process of a run.
    Launch,
    /// A step.
    Step,
    /// A run ending.
    RunEnd,
    /// What a run changed.
    Changes,
    /// A session observed.
    Session,
}

/// The record as one JSON object.
pub(super) fn record_json(stored: &SequencedRecord) -> Value {
    let (kind, record) = match &stored.record {
        Record::Team(team) => (PlainKind::Team.to_string(), team_json(team)),
        Record::Round(round) => (PlainKind::Round.to_string(), round_json(round)),
        Record::Run(run) => (PlainKind::Run.to_string(), run_json(run)),
        Record::Launch(launch) => (PlainKind::Launch.to_string(), launch_json(launch)),
        Record::Step(step) => (PlainKind::Step.to_string(), step_json(step)),
        Record::RunEnd(end) => (PlainKind::RunEnd.to_string(), end_json(end)),
        Record::Changes(changes) => (PlainKind::Changes.to_string(), changes_json(changes)),
        Record::Session(session) => (PlainKind::Session.to_string(), session_json(session)),
        Record::Event(event) => (event.event.kind().to_string(), event_json(&event.event)),
    };
    json!({
        "sequence": stored.sequence,
        "at": stored.at.to_string(),
        "team": stored.team.as_str(),
        "rounds": stored.rounds.iter().map(RoundId::as_str).collect::<Vec<_>>(),
        "kind": kind,
        "cut": stored.cut.fields().collect::<Vec<_>>(),
        "record": record,
    })
}

/// The event as JSON: the payload the ledger keeps.
#[expect(
    clippy::expect_used,
    reason = "an event is made of strings, numbers, lists and keywords, which always have a JSON form; the history_events test serializes every kind"
)]
fn event_json(event: &HistoryEvent) -> Value {
    serde_json::to_value(event).expect("an event has a JSON form")
}

fn team_json(team: &TeamStart) -> Value {
    json!({
        "team": team.team.as_str(),
        "at": team.at.to_string(),
        "repo": team.repo,
        "base_branch": team.base_branch.as_str(),
        "merge": team.merge.to_string(),
        "task_list": team.task_list,
        "plan": team.plan.iter().map(entry_json).collect::<Vec<_>>(),
    })
}

fn entry_json(entry: &PlanEntry) -> Value {
    let among = match &entry.how {
        HowChosen::Drawn { among } => Some(among.iter().map(AgentName::as_str).collect::<Vec<_>>()),
        HowChosen::Named | HowChosen::NamedByFlag | HowChosen::NamedByLine | HowChosen::ConfiguredOrder => None,
    };
    json!({
        "agent": entry.agent.as_str(),
        "position": entry.position.get(),
        "role": entry.role.to_string(),
        "how": entry.how.kind().to_string(),
        "among": among,
    })
}

fn round_json(round: &RoundOpening) -> Value {
    let (item, brief) = match &round.subject {
        RoundSubject::Item(item) => (
            json!({
                "item": item.item.to_string(),
                "title": item.title,
                "description": item.description,
                "expected_result": item.expected_result,
                "checks": item.checks,
            }),
            Value::Null,
        ),
        RoundSubject::Brief(brief) => (Value::Null, Value::String(brief.clone())),
    };
    json!({
        "round": round.round.as_str(),
        "team": round.team.as_str(),
        "at": round.at.to_string(),
        "item": item,
        "brief": brief,
    })
}

fn why_json(why: &RunWhy) -> Value {
    let kind = why.kind().to_string();
    match why {
        RunWhy::Item { item } => json!({"kind": kind, "item": item.to_string()}),
        RunWhy::Request { asked_by } => json!({"kind": kind, "asked_by": asked_by.as_ref().map(RunId::as_str)}),
        RunWhy::FixPass { number } => json!({"kind": kind, "number": number.get()}),
        RunWhy::Review { pr, pass } => json!({"kind": kind, "pr": pr.get().get(), "pass": pass}),
        RunWhy::FirstRun
        | RunWhy::WakeUp
        | RunWhy::AskHuman
        | RunWhy::VerdictMissing
        | RunWhy::Checks
        | RunWhy::Answers
        | RunWhy::ResumedByHuman
        | RunWhy::Continuation => json!({"kind": kind}),
    }
}

fn run_json(run: &RunStart) -> Value {
    json!({
        "run": run.run.as_str(),
        "team": run.team.as_str(),
        "task": run.task.as_str(),
        "at": run.at.to_string(),
        "agent": {
            "name": run.agent.name,
            "tier": run.agent.tier.to_string(),
            "cli": run.agent.cli.to_string(),
            "model": run.agent.model.as_ref().map(ModelName::as_str),
        },
        "role": run.role.to_string(),
        "rounds": run.rounds.iter().map(RoundId::as_str).collect::<Vec<_>>(),
        "why": why_json(&run.why),
        "prompt": run.prompt,
        "worktree": run.worktree.as_path().to_string_lossy(),
        "branch": run.branch.as_str(),
        "session": run.session.as_ref().map(SessionId::as_str),
    })
}

fn launch_json(launch: &Launch) -> Value {
    json!({
        "run": launch.run.as_str(),
        "at": launch.at.to_string(),
        "program": launch.program,
        "args": launch.args,
        "working_dir": launch.working_dir.as_path().to_string_lossy(),
        "head": launch.head.as_ref().map(CommitId::as_str),
        "env_names": launch.env_names.iter().map(EnvName::as_str).collect::<Vec<_>>(),
        "prompt_in_file": launch.prompt_in_file,
    })
}

fn client_json(client: &Client) -> Value {
    match client {
        Client::Cli => json!({"kind": "cli"}),
        Client::Dashboard { login } => json!({"kind": "dashboard", "login": login.as_str()}),
    }
}

fn step_json(step: &Step) -> Value {
    let kind = StepKind::of(&step.event).to_string();
    let event = match &step.event {
        ActivityEvent::Text(text)
        | ActivityEvent::Command(text)
        | ActivityEvent::Error(text)
        | ActivityEvent::Info(text) => json!({"kind": kind, "text": text}),
        ActivityEvent::FileEdit { path } => json!({"kind": kind, "path": path}),
        ActivityEvent::BusCall { tool, arguments } => json!({"kind": kind, "tool": tool, "arguments": arguments}),
        ActivityEvent::ToolCall { name, summary } => json!({"kind": kind, "name": name, "summary": summary}),
        ActivityEvent::Control { action, by } => {
            json!({"kind": kind, "action": action.to_string(), "by": client_json(by)})
        }
    };
    json!({
        "run": step.run.as_str(),
        "number": step.number.get(),
        "at": step.at.to_string(),
        "event": event,
    })
}

fn end_json(end: &RunEndRecord) -> Value {
    let kind = EndKind::of(&end.end).to_string();
    let how = match &end.end {
        RunEnd::Reported { outcome } => json!({"kind": kind, "outcome": outcome.to_string()}),
        RunEnd::NoStatus { exit_code } => json!({"kind": kind, "exit_code": exit_code}),
        RunEnd::QuotaExhausted { resets_at } => {
            json!({"kind": kind, "resets_at": resets_at.map(|at| at.to_string())})
        }
        RunEnd::RateLimited | RunEnd::Hung | RunEnd::Stopped => json!({"kind": kind}),
    };
    json!({
        "run": end.run.as_str(),
        "at": end.at.to_string(),
        "end": how,
        "final_message": end.final_message,
    })
}

fn file_json(file: &FileChange) -> Value {
    let (change, from) = match &file.change {
        Change::Added => ("added", None),
        Change::Modified => ("modified", None),
        Change::Deleted => ("deleted", None),
        Change::Renamed { from } => ("renamed", Some(from.as_str())),
    };
    let lines = match file.lines {
        Lines::Text { added, removed } => json!({"added": added, "removed": removed}),
        Lines::Binary => Value::Null,
    };
    json!({
        "path": file.path.as_str(),
        "change": change,
        "from": from,
        "binary": matches!(file.lines, Lines::Binary),
        "lines": lines,
    })
}

fn changes_json(changes: &RunChanges) -> Value {
    json!({
        "run": changes.run.as_str(),
        "head_before": changes.head_before.as_ref().map(CommitId::as_str),
        "head_after": changes.head_after.as_ref().map(CommitId::as_str),
        "commits": changes.commits.iter().map(|commit| json!({
            "id": commit.id.as_str(),
            "subject": commit.subject,
            "authored_at": commit.authored_at.to_string(),
        })).collect::<Vec<_>>(),
        "files": changes.files.iter().map(file_json).collect::<Vec<_>>(),
        "uncommitted": changes.uncommitted.iter().map(file_json).collect::<Vec<_>>(),
    })
}

fn session_json(session: &RunSession) -> Value {
    json!({
        "run": session.run.as_str(),
        "number": session.number.get(),
        "at": session.at.to_string(),
        "session": session.session.as_str(),
    })
}
