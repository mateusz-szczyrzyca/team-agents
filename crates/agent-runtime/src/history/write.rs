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
use agent_core::ids::{AgentName, RunId};
use agent_core::log::{LogLevel, LogRecord, LogTaskScope, LogTeamScope};
use agent_core::state::RunEnd;
use rusqlite::{Transaction, params};

use super::codec::time_text;
use super::error::{RecordKind, WriteError};
use super::ledger;

/// Whether a database has the ledger. A daemon always migrates the database it opens, so only a test that opens an older
/// version on purpose writes to one that has not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ledger {
    /// The ledger is there: every record is entered in it.
    Kept,
    /// The database is of a version before the ledger: the records have their tables and nothing else.
    Absent,
}

/// Whether a database has the daemon log tables (`daemon_logs` and `daemon_log_rounds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DaemonLogs {
    /// The daemon log tables are there (`user_version >= 4`).
    Kept,
    /// The database is of a version before migration 4.
    Absent,
}

/// Writes `record` in `tx`.
///
/// # Errors
/// [`WriteError`]: the record is refused, or cannot be turned into rows.
pub(super) fn insert(
    tx: &Transaction<'_>,
    record: &Record,
    limit: MaxTextBytes,
    ledger: Ledger,
) -> Result<(), WriteError> {
    match record {
        Record::Team(team) => insert_team(tx, team, limit)?,
        Record::Round(round) => insert_round(tx, round, limit)?,
        Record::Run(run) => insert_run(tx, run, limit)?,
        Record::Launch(launch) => insert_launch(tx, launch, limit)?,
        Record::Step(step) => insert_step(tx, step, limit)?,
        Record::RunEnd(end) => insert_end(tx, end, limit)?,
        Record::Changes(changes) => insert_changes(tx, changes, limit)?,
        // An event has no table of its own: the ledger holds it.
        Record::Event(_) => {}
        Record::Session(session) => {
            tx.execute(
                "INSERT INTO run_sessions (run, n, at, session) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (run, n) DO NOTHING",
                params![
                    session.run.as_str(),
                    session.number.get(),
                    time_text(session.at),
                    session.session.as_str()
                ],
            )
            .map_err(sql(RecordKind::Session))?;
        }
    }
    match ledger {
        Ledger::Kept => ledger::append(tx, record, limit),
        Ledger::Absent => match record {
            Record::Event(_) => Err(WriteError::Rule {
                record: RecordKind::Event,
                rule: "the database was made before the ledger, which keeps events: it is migrated when it is opened",
            }),
            Record::Team(_)
            | Record::Round(_)
            | Record::Run(_)
            | Record::Launch(_)
            | Record::Step(_)
            | Record::RunEnd(_)
            | Record::Changes(_)
            | Record::Session(_) => Ok(()),
        },
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
        | RunWhy::Continuation
        | RunWhy::TurnLimit => (None, None, None, None),
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
        RunEnd::TurnLimit | RunEnd::RateLimited | RunEnd::Hung | RunEnd::Stopped => (None, None, None),
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

/// Writes `record` into `daemon_logs` (and `daemon_log_rounds`), coalescing repeated identical warnings.
///
/// # Errors
/// [`WriteError`]: the database predates migration 4, or the record is refused or cannot be turned into rows.
pub(super) fn insert_log(
    tx: &Transaction<'_>,
    record: &LogRecord,
    limit: MaxTextBytes,
    logs: DaemonLogs,
) -> Result<(), WriteError> {
    let kind = RecordKind::Event;
    if logs == DaemonLogs::Absent {
        return Err(WriteError::Rule {
            record: kind,
            rule: "the database was made before migration 4, which keeps daemon logs: it is migrated when it is opened",
        });
    }
    if record.module.is_empty() {
        return Err(WriteError::Rule {
            record: kind,
            rule: "a daemon log module cannot be empty",
        });
    }
    let record = record.clone().keep(limit);
    let fields_json =
        serde_json::to_string(&record.fields).map_err(|source| WriteError::Json { record: kind, source })?;
    let at_text = time_text(record.at);
    let cut_text = record.cut.to_string();
    let team = record.team.as_ref().map(|scope| scope.team().as_str());
    let rounds = record.team.as_ref().map_or(&[][..], LogTeamScope::rounds);
    let task = record.task.as_ref().map(|scope| scope.task().as_str());
    let run = record.task.as_ref().and_then(LogTaskScope::run).map(RunId::as_str);

    if record.level() == LogLevel::Warn
        && let Some(existing_id) = find_matching_warn_id(tx, &record, &fields_json, &cut_text)?
    {
        tx.execute(
            "UPDATE daemon_logs SET count = count + 1, \
             first_at = CASE WHEN ?1 < first_at THEN ?1 ELSE first_at END, \
             at = CASE WHEN ?1 > at THEN ?1 ELSE at END WHERE id = ?2",
            params![at_text, existing_id],
        )
        .map_err(sql(kind))?;
        return Ok(());
    }

    let id = if super::migrate::version_of(tx).map_err(sql(kind))? >= 6 {
        let previous: i64 = tx
            .query_row(
                "SELECT value FROM history_cursors WHERE name = 'daemon_logs'",
                [],
                |row| row.get(0),
            )
            .map_err(sql(kind))?;
        Some(previous.checked_add(1).ok_or(WriteError::Rule {
            record: kind,
            rule: "daemon log cursor exhausted",
        })?)
    } else {
        None
    };
    tx.execute(
        "INSERT INTO daemon_logs \
         (first_at, at, level, module, event, team, task, run, message, fields, count, cut, id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12)",
        params![
            at_text,
            at_text,
            record.level().as_str(),
            record.module,
            record.event.as_str(),
            team,
            task,
            run,
            record.message,
            fields_json,
            cut_text,
            id,
        ],
    )
    .map_err(sql(kind))?;
    let log_id = tx.last_insert_rowid();
    for (index, round) in rounds.iter().enumerate() {
        tx.execute(
            "INSERT INTO daemon_log_rounds (log, position, round) VALUES (?1, ?2, ?3)",
            params![log_id, number(kind, index.saturating_add(1))?, round.as_str()],
        )
        .map_err(sql(kind))?;
    }
    Ok(())
}

/// Finds the `id` of an existing `warn` row with identical `(event, team, rounds, task, run, message, fields, cut)`.
/// The index covers every scalar criterion; round lists are compared exactly to disambiguate rows sharing that prefix.
fn find_matching_warn_id(
    tx: &Transaction<'_>,
    record: &LogRecord,
    fields_json: &str,
    cut_text: &str,
) -> Result<Option<i64>, WriteError> {
    let kind = RecordKind::Event;
    let team = record.team.as_ref().map(|scope| scope.team().as_str());
    let rounds = record.team.as_ref().map_or(&[][..], LogTeamScope::rounds);
    let task = record.task.as_ref().map(|scope| scope.task().as_str());
    let run = record.task.as_ref().and_then(LogTaskScope::run).map(RunId::as_str);
    let mut statement = tx
        .prepare(
            "SELECT id FROM daemon_logs \
             WHERE level = 'warn' AND event = ?1 AND team IS ?2 AND task IS ?3 AND run IS ?4 \
               AND message = ?5 AND fields = ?6 AND cut = ?7 \
             ORDER BY id DESC",
        )
        .map_err(sql(kind))?;
    let ids: Vec<i64> = statement
        .query_map(
            params![
                record.event.as_str(),
                team,
                task,
                run,
                record.message,
                fields_json,
                cut_text
            ],
            |row| row.get(0),
        )
        .map_err(sql(kind))?
        .collect::<Result<_, _>>()
        .map_err(sql(kind))?;
    let mut round_statement = tx
        .prepare("SELECT round FROM daemon_log_rounds WHERE log = ?1 ORDER BY position ASC")
        .map_err(sql(kind))?;
    for id in ids {
        let existing_rounds: Vec<String> = round_statement
            .query_map(params![id], |row| row.get(0))
            .map_err(sql(kind))?
            .collect::<Result<_, _>>()
            .map_err(sql(kind))?;
        if existing_rounds.len() == rounds.len()
            && existing_rounds
                .iter()
                .zip(rounds.iter())
                .all(|(existing, target)| existing == target.as_str())
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}
