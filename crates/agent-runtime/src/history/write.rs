//! A record turned into rows. Every record is one transaction's worth of work: its parent row, then its child rows.
//!
//! The parent is inserted `ON CONFLICT DO NOTHING`, so a record handed in twice is one row and the first write wins; the
//! children of a parent that was there already are not written again. A child is inserted plainly, so a record that
//! names a position, a round or a path twice is refused whole, not quietly thinned.

use std::path::Path;

use agent_core::cli::ActivityEvent;
use agent_core::config::EnvName;
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    ChangeKind, ChangeScope, ClientKind, Cut, EndKind, HowChosen, Launch, MaxTextBytes, PlanEntry, Record,
    RoundOpening, RoundSubject, RunChanges, RunEndRecord, RunStart, RunWhy, Step, StepKind, TeamStart,
};
use agent_core::ids::AgentName;
use agent_core::state::RunEnd;
use rusqlite::{Transaction, params};

use super::codec::time_text;
use super::error::{RecordKind, WriteError};

/// Writes `record` in `tx`.
///
/// # Errors
/// [`WriteError`]: the record is refused, or cannot be turned into rows.
pub(super) fn insert(tx: &Transaction<'_>, record: &Record, limit: MaxTextBytes) -> Result<(), WriteError> {
    match record {
        Record::Team(team) => insert_team(tx, team, limit),
        Record::Round(round) => insert_round(tx, round, limit),
        Record::Run(run) => insert_run(tx, run, limit),
        Record::Launch(launch) => insert_launch(tx, launch, limit),
        Record::Step(step) => insert_step(tx, step, limit),
        Record::RunEnd(end) => insert_end(tx, end, limit),
        Record::Changes(changes) => insert_changes(tx, changes, limit),
    }
}

/// The error of an SQL statement of `record`.
fn sql(record: RecordKind) -> impl Fn(rusqlite::Error) -> WriteError {
    move |source| WriteError::Sqlite { record, source }
}

/// A count or a position as the number a column holds.
fn number(record: RecordKind, value: usize) -> Result<u32, WriteError> {
    u32::try_from(value).map_err(|source| WriteError::Number { record, source })
}

/// `path` as text.
fn path_text(record: RecordKind, path: &Path) -> Result<&str, WriteError> {
    path.to_str().ok_or_else(|| WriteError::PathNotText {
        record,
        path: path.to_path_buf(),
    })
}

/// A list of texts as the JSON the column holds.
fn json_list<'a>(record: RecordKind, items: impl Iterator<Item = &'a str>) -> Result<String, WriteError> {
    let list: Vec<&str> = items.collect();
    serde_json::to_string(&list).map_err(|source| WriteError::Json { record, source })
}

fn insert_team(tx: &Transaction<'_>, team: &TeamStart, limit: MaxTextBytes) -> Result<(), WriteError> {
    let record = RecordKind::Team;
    let mut cut = Cut::default();
    let task_list = cut.keep("task_list", &team.task_list, limit);
    let added = tx
        .execute(
            "INSERT INTO teams (team, at, repo, base_branch, merge_mode, task_list, cut) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (team) DO NOTHING",
            params![
                team.team.as_str(),
                time_text(team.at),
                team.repo,
                team.base_branch.as_str(),
                team.merge.as_str(),
                task_list,
                cut.to_string()
            ],
        )
        .map_err(sql(record))?;
    if added == 0 {
        return Ok(());
    }
    for entry in &team.plan {
        insert_plan_entry(tx, team, entry)?;
    }
    Ok(())
}

fn insert_plan_entry(tx: &Transaction<'_>, team: &TeamStart, entry: &PlanEntry) -> Result<(), WriteError> {
    let record = RecordKind::Team;
    let among = match &entry.how {
        HowChosen::Drawn { among } => Some(json_list(record, among.iter().map(AgentName::as_str))?),
        HowChosen::Named | HowChosen::NamedByFlag | HowChosen::NamedByLine | HowChosen::ConfiguredOrder => None,
    };
    tx.execute(
        "INSERT INTO plan (team, position, agent, role, how, drawn_among) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            team.team.as_str(),
            entry.position.get(),
            entry.agent.as_str(),
            entry.role.to_string(),
            entry.how.kind().to_string(),
            among
        ],
    )
    .map_err(sql(record))?;
    Ok(())
}

fn insert_round(tx: &Transaction<'_>, round: &RoundOpening, limit: MaxTextBytes) -> Result<(), WriteError> {
    let mut cut = Cut::default();
    let (item, title, description, expected, checks, brief) = match &round.subject {
        RoundSubject::Item(item) => (
            Some(item.item.to_string()),
            Some(cut.keep("title", &item.title, limit)),
            Some(cut.keep("description", &item.description, limit)),
            item.expected_result
                .as_deref()
                .map(|text| cut.keep("expected_result", text, limit)),
            item.checks.as_deref().map(|text| cut.keep("checks", text, limit)),
            None,
        ),
        RoundSubject::Brief(brief) => (None, None, None, None, None, Some(cut.keep("brief", brief, limit))),
    };
    tx.execute(
        "INSERT INTO rounds (round, team, at, item, title, description, expected_result, checks, brief, cut) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT (round) DO NOTHING",
        params![
            round.round.as_str(),
            round.team.as_str(),
            time_text(round.at),
            item,
            title,
            description,
            expected,
            checks,
            brief,
            cut.to_string()
        ],
    )
    .map_err(sql(RecordKind::Round))?;
    Ok(())
}

/// The columns a reason fills besides its keyword: the item, the run that asked, the pass and the pull request.
fn why_columns(why: &RunWhy) -> (Option<String>, Option<String>, Option<u32>, Option<u32>) {
    match why {
        RunWhy::Item { item } => (Some(item.to_string()), None, None, None),
        RunWhy::Request { asked_by } => (None, asked_by.as_ref().map(ToString::to_string), None, None),
        RunWhy::FixPass { number } => (None, None, Some(number.get()), None),
        RunWhy::Review { pr, pass } => (None, None, Some(*pass), Some(pr.get().get())),
        RunWhy::FirstRun
        | RunWhy::WakeUp
        | RunWhy::AskHuman
        | RunWhy::VerdictMissing
        | RunWhy::Checks
        | RunWhy::Answers
        | RunWhy::ResumedByHuman
        | RunWhy::Continuation => (None, None, None, None),
    }
}

fn insert_run(tx: &Transaction<'_>, run: &RunStart, limit: MaxTextBytes) -> Result<(), WriteError> {
    let record = RecordKind::Run;
    let mut cut = Cut::default();
    let prompt = cut.keep("prompt", &run.prompt, limit);
    let (why_item, why_asked_by, why_pass, why_pr) = why_columns(&run.why);
    let added = tx
        .execute(
            "INSERT INTO runs (run, team, task, at, agent, tier, cli, model, role, why, why_item, why_asked_by, why_pass, \
             why_pr, prompt, worktree, branch, session, cut) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
             ON CONFLICT (run) DO NOTHING",
            params![
                run.run.as_str(),
                run.team.as_str(),
                run.task.as_str(),
                time_text(run.at),
                run.agent.name,
                run.agent.tier.to_string(),
                run.agent.cli.to_string(),
                run.agent.model.as_ref().map(|model| model.as_str().to_owned()),
                run.role.to_string(),
                run.why.kind().to_string(),
                why_item,
                why_asked_by,
                why_pass,
                why_pr,
                prompt,
                path_text(record, run.worktree.as_path())?,
                run.branch.as_str(),
                run.session.as_ref().map(|session| session.as_str().to_owned()),
                cut.to_string()
            ],
        )
        .map_err(sql(record))?;
    if added == 0 {
        return Ok(());
    }
    for (index, round) in run.rounds.iter().enumerate() {
        tx.execute(
            "INSERT INTO run_rounds (run, position, round) VALUES (?1, ?2, ?3)",
            params![
                run.run.as_str(),
                number(record, index.saturating_add(1))?,
                round.as_str()
            ],
        )
        .map_err(sql(record))?;
    }
    Ok(())
}

fn insert_launch(tx: &Transaction<'_>, launch: &Launch, limit: MaxTextBytes) -> Result<(), WriteError> {
    let record = RecordKind::Launch;
    let mut cut = Cut::default();
    let args: Vec<String> = launch.args.iter().map(|arg| cut.keep("args", arg, limit)).collect();
    let args = json_list(record, args.iter().map(String::as_str))?;
    let env_names = json_list(record, launch.env_names.iter().map(EnvName::as_str))?;
    tx.execute(
        "INSERT INTO launches (run, at, program, args, working_dir, head, env_names, prompt_in_file, cut) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (run) DO NOTHING",
        params![
            launch.run.as_str(),
            time_text(launch.at),
            launch.program,
            args,
            path_text(record, launch.working_dir.as_path())?,
            launch.head.as_ref().map(ToString::to_string),
            env_names,
            launch.prompt_in_file,
            cut.to_string()
        ],
    )
    .map_err(sql(record))?;
    Ok(())
}

fn insert_step(tx: &Transaction<'_>, step: &Step, limit: MaxTextBytes) -> Result<(), WriteError> {
    let mut cut = Cut::default();
    let mut text = None;
    let mut path = None;
    let mut tool = None;
    let mut detail = None;
    let mut action = None;
    let mut client = None;
    let mut login = None;
    match &step.event {
        ActivityEvent::Text(value)
        | ActivityEvent::Command(value)
        | ActivityEvent::Error(value)
        | ActivityEvent::Info(value) => {
            text = Some(cut.keep("text", value, limit));
        }
        ActivityEvent::FileEdit { path: edited } => path = Some(cut.keep("path", edited, limit)),
        ActivityEvent::BusCall { tool: name, arguments } => {
            tool = Some(cut.keep("tool", name, limit));
            detail = Some(cut.keep("detail", arguments, limit));
        }
        ActivityEvent::ToolCall { name, summary } => {
            tool = Some(cut.keep("tool", name, limit));
            detail = Some(cut.keep("detail", summary, limit));
        }
        ActivityEvent::Control { action: done, by } => {
            action = Some(done.to_string());
            client = Some(ClientKind::of(by).to_string());
            login = match by {
                agent_core::protocol::Client::Cli => None,
                agent_core::protocol::Client::Dashboard { login } => Some(login.as_str().to_owned()),
            };
        }
    }
    tx.execute(
        "INSERT INTO steps (run, n, at, kind, text, path, tool, detail, action, client, login, cut) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) ON CONFLICT (run, n) DO NOTHING",
        params![
            step.run.as_str(),
            step.number.get(),
            time_text(step.at),
            StepKind::of(&step.event).to_string(),
            text,
            path,
            tool,
            detail,
            action,
            client,
            login,
            cut.to_string()
        ],
    )
    .map_err(sql(RecordKind::Step))?;
    Ok(())
}

fn insert_end(tx: &Transaction<'_>, end: &RunEndRecord, limit: MaxTextBytes) -> Result<(), WriteError> {
    let record = RecordKind::RunEnd;
    let ended = time_text(end.at);
    let started: Option<String> = rusqlite::OptionalExtension::optional(tx.query_row(
        "SELECT at FROM runs WHERE run = ?1",
        params![end.run.as_str()],
        |row| row.get(0),
    ))
    .map_err(sql(record))?;
    // The run's own row is missing when the foreign key refuses the end below; a run that has one cannot end before it
    // began (the state refuses the same).
    if started.is_some_and(|started| ended < started) {
        return Err(WriteError::Rule {
            record,
            rule: "a run cannot end before it started",
        });
    }
    let mut cut = Cut::default();
    let final_message = end
        .final_message
        .as_deref()
        .map(|text| cut.keep("final_message", text, limit));
    let (outcome, exit_code, resets_at) = match &end.end {
        RunEnd::Reported { outcome } => (Some(outcome.to_string()), None, None),
        RunEnd::NoStatus { exit_code } => (None, *exit_code, None),
        RunEnd::QuotaExhausted { resets_at } => (None, None, resets_at.map(time_text)),
        RunEnd::RateLimited | RunEnd::Hung | RunEnd::Stopped => (None, None, None),
    };
    tx.execute(
        "INSERT INTO run_ends (run, at, how, outcome, exit_code, resets_at, final_message, cut) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (run) DO NOTHING",
        params![
            end.run.as_str(),
            ended,
            EndKind::of(&end.end).to_string(),
            outcome,
            exit_code,
            resets_at,
            final_message,
            cut.to_string()
        ],
    )
    .map_err(sql(record))?;
    Ok(())
}

fn insert_changes(tx: &Transaction<'_>, changes: &RunChanges, limit: MaxTextBytes) -> Result<(), WriteError> {
    let record = RecordKind::Changes;
    let added = tx
        .execute(
            "INSERT INTO run_changes (run, head_before, head_after) VALUES (?1, ?2, ?3) ON CONFLICT (run) DO NOTHING",
            params![
                changes.run.as_str(),
                changes.head_before.as_ref().map(ToString::to_string),
                changes.head_after.as_ref().map(ToString::to_string)
            ],
        )
        .map_err(sql(record))?;
    if added == 0 {
        return Ok(());
    }
    for (index, commit) in changes.commits.iter().enumerate() {
        let mut cut = Cut::default();
        let subject = cut.keep("subject", &commit.subject, limit);
        tx.execute(
            "INSERT INTO commits (run, n, hash, subject, authored_at, cut) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                changes.run.as_str(),
                number(record, index.saturating_add(1))?,
                commit.id.to_string(),
                subject,
                time_text(commit.authored_at),
                cut.to_string()
            ],
        )
        .map_err(sql(record))?;
    }
    for file in &changes.files {
        insert_file(tx, changes, ChangeScope::Committed, file)?;
    }
    for file in &changes.uncommitted {
        insert_file(tx, changes, ChangeScope::Uncommitted, file)?;
    }
    Ok(())
}

fn insert_file(
    tx: &Transaction<'_>,
    changes: &RunChanges,
    scope: ChangeScope,
    file: &FileChange,
) -> Result<(), WriteError> {
    let from = match &file.change {
        Change::Renamed { from } => Some(from.as_str().to_owned()),
        Change::Added | Change::Modified | Change::Deleted => None,
    };
    let (is_binary, added, removed) = match file.lines {
        Lines::Text { added, removed } => (false, Some(added), Some(removed)),
        Lines::Binary => (true, None, None),
    };
    tx.execute(
        "INSERT INTO changed_files (run, scope, path, change, renamed_from, is_binary, lines_added, lines_removed) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            changes.run.as_str(),
            scope.to_string(),
            file.path.as_str(),
            ChangeKind::of(&file.change).to_string(),
            from,
            is_binary,
            added,
            removed
        ],
    )
    .map_err(sql(RecordKind::Changes))?;
    Ok(())
}
