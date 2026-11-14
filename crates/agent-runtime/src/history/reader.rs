//! The reading API of the history: typed answers to the questions about a team, a round and a run.
//!
//! A reader opens the file read-only and needs no daemon. Every row is read through the checks of the types (an ID is an
//! ID, a status is a status) and the rules of its record, so a file that was changed by something else is refused with
//! the row named, not half believed. Each answer is read in one transaction: it is a whole snapshot, never a record the
//! writer was in the middle of.

use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use agent_core::cli::ActivityEvent;
use agent_core::config::{AbsolutePath, EnvName, ModelName, TailscaleLogin};
use agent_core::decide::checks::{Change, FileChange, Lines};
use agent_core::history::{
    AgentIdentity, ChangeKind, ClientKind, CommitRecord, Cut, EndKind, HowChosen, ItemText, Launch, PlanEntry,
    RoundOpening, RoundSubject, RunChanges, RunEndRecord, RunStart, RunWhy, Step, StepKind, TeamStart, WhyKind,
};
use agent_core::ids::{AgentName, BranchName, ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::protocol::{Client, ControlAction};
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, MergeMode, PrNumber, RunEnd, SessionId, Timestamp};
use agent_core::status::RunOutcome;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, Transaction, params};

use super::codec::parse_time;
use super::error::{ReadError, ValueProblem};
use super::migrate::{latest, version_of};
use super::schema::MIGRATIONS;

/// How long a reader waits for the lock of a database that is being checkpointed.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A record as the database keeps it, with the names of the fields whose text was cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored<T> {
    /// The record.
    pub value: T,
    /// The fields that were cut at the limit of text.
    pub cut: Cut,
}

/// A run with what was recorded about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    /// How it started.
    pub start: Stored<RunStart>,
    /// How its process started; absent when the run failed before it had one.
    pub launch: Option<Stored<Launch>>,
    /// How it ended; absent while it is going, and for a run the daemon did not see end.
    pub end: Option<Stored<RunEndRecord>>,
}

/// The prompt of one run of a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundPrompt {
    /// The run.
    pub run: RunId,
    /// When it started.
    pub at: Timestamp,
    /// The agent.
    pub agent: String,
    /// Its role.
    pub role: Role,
    /// The prompt, whole unless the record says it was cut.
    pub prompt: String,
    /// Whether the prompt was cut.
    pub was_cut: bool,
}

/// The history database, open read-only.
#[derive(Debug)]
pub struct Reader {
    /// The connection.
    connection: Connection,
}

/// Which row is being read, for the error of a value that is not valid.
struct Row_ {
    /// The table.
    table: &'static str,
    /// The key of the row, as text.
    key: String,
}

impl Row_ {
    /// A row of `table` with the key `key`.
    fn of(table: &'static str, key: impl Into<String>) -> Self {
        Self { table, key: key.into() }
    }

    /// The value a column holds, or the error that names this row and `column`.
    fn value<T, E>(&self, column: &'static str, parsed: Result<T, E>) -> Result<T, ReadError>
    where
        ValueProblem: From<E>,
    {
        parsed.map_err(|source| self.problem(column, ValueProblem::from(source)))
    }

    /// The error of a column of this row.
    fn problem(&self, column: &'static str, source: ValueProblem) -> ReadError {
        ReadError::Value {
            table: self.table,
            column,
            key: self.key.clone(),
            source,
        }
    }

    /// The error of a rule of this row's record that `column` breaks.
    fn rule(&self, column: &'static str, rule: &'static str) -> ReadError {
        self.problem(column, ValueProblem::Rule { rule })
    }

    /// The text of an optional column that the record needs.
    fn needed<'a>(&self, column: &'static str, value: Option<&'a str>) -> Result<&'a str, ReadError> {
        value.ok_or_else(|| self.rule(column, "the kind of this row needs this column"))
    }

    /// Checks that a column the record does not use is empty.
    fn absent<T>(&self, column: &'static str, value: Option<&T>) -> Result<(), ReadError> {
        if value.is_some() {
            return Err(self.rule(column, "the kind of this row does not use this column"));
        }
        Ok(())
    }

    /// The fields cut, from the `cut` column.
    fn cut(&self, text: &str, known: &[&'static str]) -> Result<Cut, ReadError> {
        self.value("cut", Cut::parse(text, known))
    }
}

/// The error of a query of `table`.
fn query(table: &'static str) -> impl Fn(rusqlite::Error) -> ReadError {
    move |source| ReadError::Query { table, source }
}

/// A count as the `u32` a record holds.
fn count(row: &Row_, column: &'static str, value: i64) -> Result<u32, ReadError> {
    row.value(column, u32::try_from(value))
}

impl Reader {
    /// Opens the history at `path` for reading.
    ///
    /// # Errors
    /// [`ReadError::Open`] when the file cannot be opened; [`ReadError::Empty`] when no daemon has written to it;
    /// [`ReadError::Newer`] when it is of a version this build does not know.
    pub fn open(path: &Path) -> Result<Self, ReadError> {
        let open = |source| ReadError::Open {
            path: path.to_path_buf(),
            source,
        };
        let connection =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .map_err(open)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(open)?;
        let found = version_of(&connection).map_err(open)?;
        let supported = latest(MIGRATIONS);
        if found == 0 {
            return Err(ReadError::Empty {
                path: path.to_path_buf(),
            });
        }
        if found > supported {
            return Err(ReadError::Newer {
                path: path.to_path_buf(),
                found,
                supported,
            });
        }
        Ok(Self { connection })
    }

    /// A snapshot of the database to read one answer from.
    fn snapshot(&self, table: &'static str) -> Result<Transaction<'_>, ReadError> {
        self.connection.unchecked_transaction().map_err(query(table))
    }

    /// The schema as `docs/history.schema.sql` pins it: `PRAGMA user_version` and what `sqlite_master` holds.
    ///
    /// # Errors
    /// [`ReadError::Query`].
    pub fn schema(&self) -> Result<String, ReadError> {
        let table = "sqlite_master";
        let snapshot = self.snapshot(table)?;
        let version = version_of(&snapshot).map_err(query(table))?;
        let mut statement = snapshot
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid")
            .map_err(query(table))?;
        let objects: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .map_err(query(table))?
            .collect::<Result<_, _>>()
            .map_err(query(table))?;
        let mut text = format!("PRAGMA user_version = {version};\n\n");
        for object in objects {
            text.push_str(&object);
            text.push_str(";\n\n");
        }
        Ok(text)
    }

    /// The team `team`, as it started.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn team(&self, team: &TeamId) -> Result<Option<Stored<TeamStart>>, ReadError> {
        let snapshot = self.snapshot("teams")?;
        read_team(&snapshot, team)
    }

    /// The rounds of `team`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn rounds_of_team(&self, team: &TeamId) -> Result<Vec<Stored<RoundOpening>>, ReadError> {
        let snapshot = self.snapshot("rounds")?;
        let ids = keys(
            &snapshot,
            "rounds",
            "SELECT round FROM rounds WHERE team = ?1 ORDER BY at, rowid",
            team.as_str(),
        )?;
        let mut rounds = Vec::with_capacity(ids.len());
        for id in ids {
            let round = RoundId::from_str(&id)
                .map_err(|source| Row_::of("rounds", id.clone()).problem("round", ValueProblem::from(source)))?;
            rounds.extend(read_round(&snapshot, &round)?);
        }
        Ok(rounds)
    }

    /// The round `round`, as it opened.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn round(&self, round: &RoundId) -> Result<Option<Stored<RoundOpening>>, ReadError> {
        let snapshot = self.snapshot("rounds")?;
        read_round(&snapshot, round)
    }

    /// The runs of `team`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn runs_of_team(&self, team: &TeamId) -> Result<Vec<RunRow>, ReadError> {
        let snapshot = self.snapshot("runs")?;
        let ids = keys(
            &snapshot,
            "runs",
            "SELECT run FROM runs WHERE team = ?1 ORDER BY at, rowid",
            team.as_str(),
        )?;
        read_runs(&snapshot, &ids)
    }

    /// The runs that worked for `round`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn runs_of_round(&self, round: &RoundId) -> Result<Vec<RunRow>, ReadError> {
        let snapshot = self.snapshot("runs")?;
        let ids = keys(
            &snapshot,
            "run_rounds",
            "SELECT runs.run FROM run_rounds JOIN runs ON runs.run = run_rounds.run WHERE run_rounds.round = ?1 \
             ORDER BY runs.at, runs.rowid",
            round.as_str(),
        )?;
        read_runs(&snapshot, &ids)
    }

    /// The run `run`.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn run(&self, run: &RunId) -> Result<Option<RunRow>, ReadError> {
        let snapshot = self.snapshot("runs")?;
        read_run(&snapshot, run)
    }

    /// The steps of `run`, in their order.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn steps(&self, run: &RunId) -> Result<Vec<Stored<Step>>, ReadError> {
        let snapshot = self.snapshot("steps")?;
        let mut statement = snapshot
            .prepare(
                "SELECT n, at, kind, text, path, tool, detail, action, client, login, cut FROM steps WHERE run = ?1 ORDER BY n",
            )
            .map_err(query("steps"))?;
        let mut rows = statement.query(params![run.as_str()]).map_err(query("steps"))?;
        let mut steps = Vec::new();
        while let Some(row) = rows.next().map_err(query("steps"))? {
            steps.push(decode_step(run, row)?);
        }
        Ok(steps)
    }

    /// What `run` changed; `None` when nothing was recorded about it.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn changes(&self, run: &RunId) -> Result<Option<Stored<RunChanges>>, ReadError> {
        let snapshot = self.snapshot("run_changes")?;
        read_changes(&snapshot, run)
    }

    /// The prompts of the runs of `round`, oldest first.
    ///
    /// # Errors
    /// [`ReadError`].
    pub fn prompts_of_round(&self, round: &RoundId) -> Result<Vec<RoundPrompt>, ReadError> {
        let snapshot = self.snapshot("runs")?;
        let mut statement = snapshot
            .prepare(
                "SELECT runs.run, runs.at, runs.agent, runs.role, runs.prompt, runs.cut FROM run_rounds \
                 JOIN runs ON runs.run = run_rounds.run WHERE run_rounds.round = ?1 ORDER BY runs.at, runs.rowid",
            )
            .map_err(query("runs"))?;
        let mut rows = statement.query(params![round.as_str()]).map_err(query("runs"))?;
        let mut prompts = Vec::new();
        while let Some(row) = rows.next().map_err(query("runs"))? {
            let id: String = row.get(0).map_err(query("runs"))?;
            let here = Row_::of("runs", id.clone());
            let cut: String = row.get(5).map_err(query("runs"))?;
            prompts.push(RoundPrompt {
                run: here.value("run", RunId::from_str(&id))?,
                at: here.value("at", parse_time(&row.get::<_, String>(1).map_err(query("runs"))?))?,
                agent: row.get(2).map_err(query("runs"))?,
                role: here.value("role", Role::from_str(&row.get::<_, String>(3).map_err(query("runs"))?))?,
                prompt: row.get(4).map_err(query("runs"))?,
                was_cut: here.cut(&cut, RUN_CUTS)?.has("prompt"),
            });
        }
        Ok(prompts)
    }
}

/// The fields a team row can cut.
const TEAM_CUTS: &[&str] = &["task_list"];
/// The fields a round row can cut.
const ROUND_CUTS: &[&str] = &["title", "description", "expected_result", "checks", "brief"];
/// The fields a run row can cut.
const RUN_CUTS: &[&str] = &["prompt"];
/// The fields a launch row can cut.
const LAUNCH_CUTS: &[&str] = &["args"];
/// The fields a step row can cut.
const STEP_CUTS: &[&str] = &["text", "path", "tool", "detail"];
/// The fields a run end row can cut.
const END_CUTS: &[&str] = &["final_message"];
/// The fields a commit row can cut.
const COMMIT_CUTS: &[&str] = &["subject"];

/// The first column of the rows `sql` gives for `key`.
fn keys(snapshot: &Transaction<'_>, table: &'static str, sql: &str, key: &str) -> Result<Vec<String>, ReadError> {
    let mut statement = snapshot.prepare(sql).map_err(query(table))?;
    let rows = statement
        .query_map(params![key], |row| row.get(0))
        .map_err(query(table))?
        .collect::<Result<_, _>>()
        .map_err(query(table))?;
    Ok(rows)
}

/// The agent names of a JSON list column.
fn names(row: &Row_, column: &'static str, text: &str) -> Result<Vec<AgentName>, ReadError> {
    let list: Vec<String> = row.value(column, serde_json::from_str(text))?;
    list.iter()
        .map(|name| row.value(column, AgentName::from_str(name)))
        .collect()
}

/// A position as the number it is.
fn position(row: &Row_, value: i64) -> Result<NonZeroU32, ReadError> {
    let number = count(row, "position", value)?;
    NonZeroU32::new(number).ok_or_else(|| row.rule("position", "a position counts from 1"))
}

fn read_team(snapshot: &Transaction<'_>, team: &TeamId) -> Result<Option<Stored<TeamStart>>, ReadError> {
    let here = Row_::of("teams", team.as_str());
    let found = snapshot
        .query_row(
            "SELECT at, repo, base_branch, merge_mode, task_list, cut FROM teams WHERE team = ?1",
            params![team.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(query("teams"))?;
    let Some((at, repo, base, merge, task_list, cut)) = found else {
        return Ok(None);
    };
    let plan = read_plan(snapshot, team)?;
    let value = TeamStart {
        team: team.clone(),
        at: here.value("at", parse_time(&at))?,
        repo,
        base_branch: here.value("base_branch", BranchName::from_str(&base))?,
        merge: here.value("merge_mode", MergeMode::from_str(&merge))?,
        task_list,
        plan,
    };
    Ok(Some(Stored {
        value,
        cut: here.cut(&cut, TEAM_CUTS)?,
    }))
}

fn read_plan(snapshot: &Transaction<'_>, team: &TeamId) -> Result<Vec<PlanEntry>, ReadError> {
    let mut statement = snapshot
        .prepare("SELECT position, agent, role, how, drawn_among FROM plan WHERE team = ?1 ORDER BY position")
        .map_err(query("plan"))?;
    let mut rows = statement.query(params![team.as_str()]).map_err(query("plan"))?;
    let mut plan = Vec::new();
    while let Some(row) = rows.next().map_err(query("plan"))? {
        let number: i64 = row.get(0).map_err(query("plan"))?;
        let here = Row_::of("plan", format!("{team} {number}"));
        let agent: String = row.get(1).map_err(query("plan"))?;
        let role: String = row.get(2).map_err(query("plan"))?;
        let how: String = row.get(3).map_err(query("plan"))?;
        let among: Option<String> = row.get(4).map_err(query("plan"))?;
        let how = match here.value("how", agent_core::history::HowKind::from_str(&how))? {
            agent_core::history::HowKind::Named => HowChosen::Named,
            agent_core::history::HowKind::NamedByFlag => HowChosen::NamedByFlag,
            agent_core::history::HowKind::NamedByLine => HowChosen::NamedByLine,
            agent_core::history::HowKind::ConfiguredOrder => HowChosen::ConfiguredOrder,
            agent_core::history::HowKind::Drawn => HowChosen::Drawn {
                among: names(&here, "drawn_among", here.needed("drawn_among", among.as_deref())?)?,
            },
        };
        plan.push(PlanEntry {
            agent: here.value("agent", AgentName::from_str(&agent))?,
            position: position(&here, number)?,
            role: here.value("role", Role::from_str(&role))?,
            how,
        });
    }
    Ok(plan)
}

fn read_round(snapshot: &Transaction<'_>, round: &RoundId) -> Result<Option<Stored<RoundOpening>>, ReadError> {
    let here = Row_::of("rounds", round.as_str());
    let found = snapshot
        .query_row(
            "SELECT team, at, item, title, description, expected_result, checks, brief, cut FROM rounds WHERE round = ?1",
            params![round.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()
        .map_err(query("rounds"))?;
    let Some((team, at, item, title, description, expected, checks, brief, cut)) = found else {
        return Ok(None);
    };
    let subject = match (item, brief) {
        (Some(item), None) => RoundSubject::Item(ItemText {
            item: here.value("item", ItemId::from_str(&item))?,
            title: here.needed("title", title.as_deref())?.to_owned(),
            description: here.needed("description", description.as_deref())?.to_owned(),
            expected_result: expected,
            checks,
        }),
        (None, Some(brief)) => {
            here.absent("title", title.as_ref())?;
            here.absent("description", description.as_ref())?;
            here.absent("expected_result", expected.as_ref())?;
            here.absent("checks", checks.as_ref())?;
            RoundSubject::Brief(brief)
        }
        (Some(_), Some(_)) | (None, None) => {
            return Err(here.rule(
                "brief",
                "a round is about an item or has a brief, not both and not neither",
            ));
        }
    };
    Ok(Some(Stored {
        value: RoundOpening {
            round: round.clone(),
            team: here.value("team", TeamId::from_str(&team))?,
            at: here.value("at", parse_time(&at))?,
            subject,
        },
        cut: here.cut(&cut, ROUND_CUTS)?,
    }))
}

fn read_runs(snapshot: &Transaction<'_>, ids: &[String]) -> Result<Vec<RunRow>, ReadError> {
    let mut runs = Vec::with_capacity(ids.len());
    for id in ids {
        let run = RunId::from_str(id).map_err(|source| Row_::of("runs", id.clone()).problem("run", source.into()))?;
        runs.extend(read_run(snapshot, &run)?);
    }
    Ok(runs)
}

fn read_run(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<RunRow>, ReadError> {
    let Some(start) = read_run_start(snapshot, run)? else {
        return Ok(None);
    };
    let launch = read_launch(snapshot, run)?;
    let end = read_end(snapshot, run)?;
    if let Some(end) = &end
        && end.value.at < start.value.at
    {
        return Err(Row_::of("run_ends", run.as_str()).rule("at", "a run cannot end before it started"));
    }
    Ok(Some(RunRow { start, launch, end }))
}

/// The reason of a run, from its columns.
fn read_why(
    here: &Row_,
    why: &str,
    item: Option<&str>,
    asked_by: Option<&str>,
    pass: Option<i64>,
    pr: Option<i64>,
) -> Result<RunWhy, ReadError> {
    let pass_number = pass.map(|value| count(here, "why_pass", value)).transpose()?;
    let kind = here.value("why", WhyKind::from_str(why))?;
    let item = item.map(ItemId::from_str);
    let item = |here: &Row_| -> Result<ItemId, ReadError> {
        let value = item.ok_or_else(|| here.rule("why_item", "this reason names an item"))?;
        here.value("why_item", value)
    };
    let none_extra = |here: &Row_| -> Result<(), ReadError> {
        if asked_by.is_some() {
            return Err(here.rule("why_asked_by", "this reason has no run that asked"));
        }
        if pass.is_some() {
            return Err(here.rule("why_pass", "this reason has no pass"));
        }
        if pr.is_some() {
            return Err(here.rule("why_pr", "this reason has no pull request"));
        }
        Ok(())
    };
    match kind {
        WhyKind::Item => {
            let named = item(here)?;
            none_extra(here)?;
            Ok(RunWhy::Item { item: named })
        }
        WhyKind::Request => Ok(RunWhy::Request {
            asked_by: asked_by
                .map(|id| here.value("why_asked_by", RunId::from_str(id)))
                .transpose()?,
        }),
        WhyKind::FixPass => {
            let number = pass_number
                .and_then(NonZeroU32::new)
                .ok_or_else(|| here.rule("why_pass", "a fix pass counts from 1"))?;
            Ok(RunWhy::FixPass { number })
        }
        WhyKind::Review => {
            let pass = pass_number.ok_or_else(|| here.rule("why_pass", "a review has a pass"))?;
            let pr = pr
                .map(|value| count(here, "why_pr", value))
                .transpose()?
                .and_then(NonZeroU32::new)
                .ok_or_else(|| here.rule("why_pr", "a review has a pull request"))?;
            Ok(RunWhy::Review {
                pr: PrNumber::new(pr),
                pass,
            })
        }
        WhyKind::FirstRun
        | WhyKind::WakeUp
        | WhyKind::AskHuman
        | WhyKind::VerdictMissing
        | WhyKind::Checks
        | WhyKind::Answers
        | WhyKind::ResumedByHuman
        | WhyKind::Continuation => {
            none_extra(here)?;
            Ok(match kind {
                WhyKind::FirstRun => RunWhy::FirstRun,
                WhyKind::WakeUp => RunWhy::WakeUp,
                WhyKind::AskHuman => RunWhy::AskHuman,
                WhyKind::VerdictMissing => RunWhy::VerdictMissing,
                WhyKind::Checks => RunWhy::Checks,
                WhyKind::Answers => RunWhy::Answers,
                WhyKind::ResumedByHuman => RunWhy::ResumedByHuman,
                WhyKind::Continuation | WhyKind::Item | WhyKind::Request | WhyKind::FixPass | WhyKind::Review => {
                    RunWhy::Continuation
                }
            })
        }
    }
}

fn read_run_start(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<Stored<RunStart>>, ReadError> {
    let here = Row_::of("runs", run.as_str());
    let found = snapshot
        .query_row(
            "SELECT team, task, at, agent, tier, cli, model, role, why, why_item, why_asked_by, why_pass, why_pr, prompt, \
             worktree, branch, session, cut FROM runs WHERE run = ?1",
            params![run.as_str()],
            |row| {
                Ok(RunColumns {
                    team: row.get(0)?,
                    task: row.get(1)?,
                    at: row.get(2)?,
                    agent: row.get(3)?,
                    tier: row.get(4)?,
                    cli: row.get(5)?,
                    model: row.get(6)?,
                    role: row.get(7)?,
                    why: row.get(8)?,
                    why_item: row.get(9)?,
                    why_asked_by: row.get(10)?,
                    why_pass: row.get(11)?,
                    why_pr: row.get(12)?,
                    prompt: row.get(13)?,
                    worktree: row.get(14)?,
                    branch: row.get(15)?,
                    session: row.get(16)?,
                    cut: row.get(17)?,
                })
            },
        )
        .optional()
        .map_err(query("runs"))?;
    let Some(columns) = found else {
        return Ok(None);
    };
    let rounds = read_run_rounds(snapshot, run)?;
    let value = RunStart {
        run: run.clone(),
        team: here.value("team", TeamId::from_str(&columns.team))?,
        task: here.value("task", TaskId::from_str(&columns.task))?,
        at: here.value("at", parse_time(&columns.at))?,
        agent: AgentIdentity {
            name: columns.agent,
            tier: here.value("tier", Tier::from_str(&columns.tier))?,
            cli: here.value("cli", CliKind::from_str(&columns.cli))?,
            model: columns
                .model
                .as_deref()
                .map(|model| here.value("model", ModelName::from_str(model)))
                .transpose()?,
        },
        role: here.value("role", Role::from_str(&columns.role))?,
        rounds,
        why: read_why(
            &here,
            &columns.why,
            columns.why_item.as_deref(),
            columns.why_asked_by.as_deref(),
            columns.why_pass,
            columns.why_pr,
        )?,
        prompt: columns.prompt,
        worktree: here.value("worktree", AbsolutePath::from_str(&columns.worktree))?,
        branch: here.value("branch", BranchName::from_str(&columns.branch))?,
        session: columns
            .session
            .as_deref()
            .map(|session| here.value("session", SessionId::from_str(session)))
            .transpose()?,
    };
    Ok(Some(Stored {
        value,
        cut: here.cut(&columns.cut, RUN_CUTS)?,
    }))
}

/// The columns of a row of `runs`, as text and numbers.
struct RunColumns {
    team: String,
    task: String,
    at: String,
    agent: String,
    tier: String,
    cli: String,
    model: Option<String>,
    role: String,
    why: String,
    why_item: Option<String>,
    why_asked_by: Option<String>,
    why_pass: Option<i64>,
    why_pr: Option<i64>,
    prompt: String,
    worktree: String,
    branch: String,
    session: Option<String>,
    cut: String,
}

fn read_run_rounds(snapshot: &Transaction<'_>, run: &RunId) -> Result<Vec<RoundId>, ReadError> {
    let rounds = keys(
        snapshot,
        "run_rounds",
        "SELECT round FROM run_rounds WHERE run = ?1 ORDER BY position",
        run.as_str(),
    )?;
    let here = Row_::of("run_rounds", run.as_str());
    rounds
        .iter()
        .map(|round| here.value("round", RoundId::from_str(round)))
        .collect()
}

fn read_launch(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<Stored<Launch>>, ReadError> {
    let here = Row_::of("launches", run.as_str());
    let found = snapshot
        .query_row(
            "SELECT at, program, args, working_dir, head, env_names, prompt_in_file, cut FROM launches WHERE run = ?1",
            params![run.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(query("launches"))?;
    let Some((at, program, args, working_dir, head, env_names, in_file, cut)) = found else {
        return Ok(None);
    };
    let env: Vec<String> = here.value("env_names", serde_json::from_str(&env_names))?;
    let value = Launch {
        run: run.clone(),
        at: here.value("at", parse_time(&at))?,
        program,
        args: here.value("args", serde_json::from_str(&args))?,
        working_dir: here.value("working_dir", AbsolutePath::from_str(&working_dir))?,
        head: head
            .as_deref()
            .map(|head| here.value("head", CommitId::from_str(head)))
            .transpose()?,
        env_names: env
            .iter()
            .map(|name| here.value("env_names", EnvName::from_str(name)))
            .collect::<Result<_, _>>()?,
        prompt_in_file: in_file,
    };
    Ok(Some(Stored {
        value,
        cut: here.cut(&cut, LAUNCH_CUTS)?,
    }))
}

fn read_end(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<Stored<RunEndRecord>>, ReadError> {
    let here = Row_::of("run_ends", run.as_str());
    let found = snapshot
        .query_row(
            "SELECT at, how, outcome, exit_code, resets_at, final_message, cut FROM run_ends WHERE run = ?1",
            params![run.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(query("run_ends"))?;
    let Some((at, how, outcome, exit_code, resets_at, final_message, cut)) = found else {
        return Ok(None);
    };
    let end = match here.value("how", EndKind::from_str(&how))? {
        EndKind::Reported => {
            here.absent("exit_code", exit_code.as_ref())?;
            here.absent("resets_at", resets_at.as_ref())?;
            let outcome = here.needed("outcome", outcome.as_deref())?;
            RunEnd::Reported {
                outcome: here.value("outcome", RunOutcome::from_str(outcome))?,
            }
        }
        EndKind::NoStatus => {
            here.absent("outcome", outcome.as_ref())?;
            here.absent("resets_at", resets_at.as_ref())?;
            RunEnd::NoStatus {
                exit_code: exit_code
                    .map(|code| here.value("exit_code", i32::try_from(code)))
                    .transpose()?,
            }
        }
        EndKind::QuotaExhausted => {
            here.absent("outcome", outcome.as_ref())?;
            here.absent("exit_code", exit_code.as_ref())?;
            RunEnd::QuotaExhausted {
                resets_at: resets_at
                    .as_deref()
                    .map(|text| here.value("resets_at", parse_time(text)))
                    .transpose()?,
            }
        }
        kind @ (EndKind::RateLimited | EndKind::Hung | EndKind::Stopped) => {
            here.absent("outcome", outcome.as_ref())?;
            here.absent("exit_code", exit_code.as_ref())?;
            here.absent("resets_at", resets_at.as_ref())?;
            match kind {
                EndKind::RateLimited => RunEnd::RateLimited,
                EndKind::Hung => RunEnd::Hung,
                EndKind::Reported | EndKind::NoStatus | EndKind::QuotaExhausted | EndKind::Stopped => RunEnd::Stopped,
            }
        }
    };
    Ok(Some(Stored {
        value: RunEndRecord {
            run: run.clone(),
            at: here.value("at", parse_time(&at))?,
            end,
            final_message,
        },
        cut: here.cut(&cut, END_CUTS)?,
    }))
}

fn decode_step(run: &RunId, row: &Row<'_>) -> Result<Stored<Step>, ReadError> {
    let table = query("steps");
    let number: i64 = row.get(0).map_err(&table)?;
    let here = Row_::of("steps", format!("{run} {number}"));
    let at: String = row.get(1).map_err(&table)?;
    let kind: String = row.get(2).map_err(&table)?;
    let text: Option<String> = row.get(3).map_err(&table)?;
    let path: Option<String> = row.get(4).map_err(&table)?;
    let tool: Option<String> = row.get(5).map_err(&table)?;
    let detail: Option<String> = row.get(6).map_err(&table)?;
    let action: Option<String> = row.get(7).map_err(&table)?;
    let client: Option<String> = row.get(8).map_err(&table)?;
    let login: Option<String> = row.get(9).map_err(&table)?;
    let cut: String = row.get(10).map_err(&table)?;
    let kind = here.value("kind", StepKind::from_str(&kind))?;
    let event = match kind {
        StepKind::Message | StepKind::Command | StepKind::Error | StepKind::Info => {
            let own = here.needed("text", text.as_deref())?.to_owned();
            for (column, value) in [
                ("path", &path),
                ("tool", &tool),
                ("detail", &detail),
                ("action", &action),
            ] {
                here.absent(column, value.as_ref())?;
            }
            match kind {
                StepKind::Message => ActivityEvent::Text(own),
                StepKind::Command => ActivityEvent::Command(own),
                StepKind::Error => ActivityEvent::Error(own),
                StepKind::Info | StepKind::FileEdit | StepKind::BusCall | StepKind::ToolCall | StepKind::Control => {
                    ActivityEvent::Info(own)
                }
            }
        }
        StepKind::FileEdit => {
            here.absent("text", text.as_ref())?;
            ActivityEvent::FileEdit {
                path: here.needed("path", path.as_deref())?.to_owned(),
            }
        }
        StepKind::BusCall | StepKind::ToolCall => {
            here.absent("text", text.as_ref())?;
            let name = here.needed("tool", tool.as_deref())?.to_owned();
            let detail = here.needed("detail", detail.as_deref())?.to_owned();
            if matches!(kind, StepKind::BusCall) {
                ActivityEvent::BusCall {
                    tool: name,
                    arguments: detail,
                }
            } else {
                ActivityEvent::ToolCall { name, summary: detail }
            }
        }
        StepKind::Control => {
            here.absent("text", text.as_ref())?;
            let action = here.value(
                "action",
                ControlAction::from_str(here.needed("action", action.as_deref())?),
            )?;
            let by = match here.value(
                "client",
                ClientKind::from_str(here.needed("client", client.as_deref())?),
            )? {
                ClientKind::Cli => {
                    here.absent("login", login.as_ref())?;
                    Client::Cli
                }
                ClientKind::Dashboard => Client::Dashboard {
                    login: here.value(
                        "login",
                        TailscaleLogin::from_str(here.needed("login", login.as_deref())?),
                    )?,
                },
            };
            ActivityEvent::Control { action, by }
        }
    };
    let number = here.value("n", u32::try_from(number))?;
    Ok(Stored {
        value: Step {
            run: run.clone(),
            number: NonZeroU32::new(number).ok_or_else(|| here.rule("n", "steps count from 1"))?,
            at: here.value("at", parse_time(&at))?,
            event,
        },
        cut: here.cut(&cut, STEP_CUTS)?,
    })
}

fn read_changes(snapshot: &Transaction<'_>, run: &RunId) -> Result<Option<Stored<RunChanges>>, ReadError> {
    let here = Row_::of("run_changes", run.as_str());
    let found = snapshot
        .query_row(
            "SELECT head_before, head_after FROM run_changes WHERE run = ?1",
            params![run.as_str()],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(query("run_changes"))?;
    let Some((before, after)) = found else {
        return Ok(None);
    };
    let head = |column: &'static str, text: Option<String>| -> Result<Option<CommitId>, ReadError> {
        text.as_deref()
            .map(|text| here.value(column, CommitId::from_str(text)))
            .transpose()
    };
    let (commits, cut) = read_commits(snapshot, run)?;
    let (files, uncommitted) = read_files(snapshot, run)?;
    Ok(Some(Stored {
        value: RunChanges {
            run: run.clone(),
            head_before: head("head_before", before)?,
            head_after: head("head_after", after)?,
            commits,
            files,
            uncommitted,
        },
        cut,
    }))
}

fn read_commits(snapshot: &Transaction<'_>, run: &RunId) -> Result<(Vec<CommitRecord>, Cut), ReadError> {
    let mut statement = snapshot
        .prepare("SELECT n, hash, subject, authored_at, cut FROM commits WHERE run = ?1 ORDER BY n")
        .map_err(query("commits"))?;
    let mut rows = statement.query(params![run.as_str()]).map_err(query("commits"))?;
    let mut commits = Vec::new();
    let mut any_cut = false;
    while let Some(row) = rows.next().map_err(query("commits"))? {
        let number: i64 = row.get(0).map_err(query("commits"))?;
        let here = Row_::of("commits", format!("{run} {number}"));
        let hash: String = row.get(1).map_err(query("commits"))?;
        let authored: String = row.get(3).map_err(query("commits"))?;
        let cut: String = row.get(4).map_err(query("commits"))?;
        any_cut |= here.cut(&cut, COMMIT_CUTS)?.has("subject");
        commits.push(CommitRecord {
            id: here.value("hash", CommitId::from_str(&hash))?,
            subject: row.get(2).map_err(query("commits"))?,
            authored_at: here.value("authored_at", parse_time(&authored))?,
        });
    }
    let all = Cut::parse(if any_cut { "subject" } else { "" }, COMMIT_CUTS)
        .map_err(|source| Row_::of("commits", run.as_str()).problem("cut", source.into()))?;
    Ok((commits, all))
}

fn read_files(snapshot: &Transaction<'_>, run: &RunId) -> Result<(Vec<FileChange>, Vec<FileChange>), ReadError> {
    let mut statement = snapshot
        .prepare(
            "SELECT scope, path, change, renamed_from, is_binary, lines_added, lines_removed FROM changed_files \
             WHERE run = ?1 ORDER BY scope, path",
        )
        .map_err(query("changed_files"))?;
    let mut rows = statement.query(params![run.as_str()]).map_err(query("changed_files"))?;
    let (mut committed, mut uncommitted) = (Vec::new(), Vec::new());
    while let Some(row) = rows.next().map_err(query("changed_files"))? {
        let scope: String = row.get(0).map_err(query("changed_files"))?;
        let path: String = row.get(1).map_err(query("changed_files"))?;
        let here = Row_::of("changed_files", format!("{run} {path}"));
        let change: String = row.get(2).map_err(query("changed_files"))?;
        let from: Option<String> = row.get(3).map_err(query("changed_files"))?;
        let binary: bool = row.get(4).map_err(query("changed_files"))?;
        let added: Option<i64> = row.get(5).map_err(query("changed_files"))?;
        let removed: Option<i64> = row.get(6).map_err(query("changed_files"))?;
        let change = match here.value("change", ChangeKind::from_str(&change))? {
            ChangeKind::Added => Change::Added,
            ChangeKind::Deleted => Change::Deleted,
            ChangeKind::Modified => Change::Modified,
            ChangeKind::Renamed => Change::Renamed {
                from: here.value(
                    "renamed_from",
                    RepoPath::from_str(here.needed("renamed_from", from.as_deref())?),
                )?,
            },
        };
        if !matches!(change, Change::Renamed { .. }) {
            here.absent("renamed_from", from.as_ref())?;
        }
        let lines = if binary {
            here.absent("lines_added", added.as_ref())?;
            here.absent("lines_removed", removed.as_ref())?;
            Lines::Binary
        } else {
            let added = added.ok_or_else(|| here.rule("lines_added", "a text file has its lines counted"))?;
            let removed = removed.ok_or_else(|| here.rule("lines_removed", "a text file has its lines counted"))?;
            Lines::Text {
                added: count(&here, "lines_added", added)?,
                removed: count(&here, "lines_removed", removed)?,
            }
        };
        let file = FileChange {
            path: here.value("path", RepoPath::from_str(&path))?,
            change,
            lines,
        };
        match here.value("scope", agent_core::history::ChangeScope::from_str(&scope))? {
            agent_core::history::ChangeScope::Committed => committed.push(file),
            agent_core::history::ChangeScope::Uncommitted => uncommitted.push(file),
        }
    }
    Ok((committed, uncommitted))
}
