//! The text of a story: what a person reads from the top of a round to its end.
//!
//! Nothing here reads a clock or a file: the records are the whole input. A text that a person may want to copy — a prompt
//! asked for whole, a pull request body, a review report, questions, a notification — is written as it was stored, between two
//! marker lines, never wrapped, indented or cut; a line of a story that summarizes a text (a step, the preview of a prompt)
//! is the only place text is shortened, and the line says so.

use std::collections::HashMap;

use crate::cli::ActivityEvent;
use crate::decide::checks::{Change, FileChange, Lines};
use crate::decide::round::AskReason;
use crate::history::{
    CheckOutcome, EventRecord, HistoryEvent, HowChosen, Launch, Record, RoundOpening, RoundSubject, RunChanges,
    RunEndRecord, RunSession, RunStart, RunWhy, SequencedRecord, Step, TeamStart,
};
use crate::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use crate::protocol::{Client, ControlAction, Target};
use crate::state::{AnsweredVia, CommitId, RunEnd, Timestamp};

use super::{PREVIEW_LINES, PromptView, Selection, StepView, StoryOptions};

/// How many characters of a step's text its line shows.
const STEP_SUMMARY_CHARS: usize = 160;

/// How many characters of an ID of a commit a story shows.
const SHORT_COMMIT_CHARS: usize = 7;

/// The page being written.
#[derive(Debug, Default)]
struct Page {
    /// The text so far.
    text: String,
}

impl Page {
    /// A line of `text`, indented by `indent` spaces.
    fn line(&mut self, indent: usize, text: &str) {
        self.text.push_str(&" ".repeat(indent));
        self.text.push_str(text);
        self.text.push('\n');
    }

    /// An empty line.
    fn blank(&mut self) {
        self.text.push('\n');
    }

    /// `text` as it was stored, between two marker lines that say what it is and how long.
    fn raw(&mut self, indent: usize, label: &str, text: &str) {
        self.line(indent, &format!("----- {label} ({} bytes) -----", text.len()));
        self.text.push_str(text);
        if !text.ends_with('\n') {
            self.text.push('\n');
        }
        self.line(indent, &format!("----- end of {label} -----"));
    }

    /// A note that the history cut the fields in `cut` at its limit, when it cut any.
    fn cut_note(&mut self, indent: usize, stored: &SequencedRecord) {
        let fields: Vec<&str> = stored.cut.fields().collect();
        if !fields.is_empty() {
            self.line(indent, &format!("(cut at the history limit: {})", fields.join(", ")));
        }
    }
}

/// The story of `chosen`, which are the records of `selection` in the order of their time.
pub(super) fn render(chosen: &[&SequencedRecord], selection: &Selection, options: StoryOptions) -> String {
    let mut page = Page::default();
    let team = chosen.first().map(|record| record.team.clone());
    let start = chosen.iter().find_map(|record| match &record.record {
        Record::Team(start) => Some((*record, start.as_ref())),
        _ => None,
    });
    match (selection, team) {
        (Selection::Round(round), Some(team)) => {
            round_part(&mut page, chosen, round, &team, options);
            team_part(&mut page, start, false);
            page.line(0, "Story");
            page.blank();
            blocks(&mut page, &in_round(chosen, round), options);
            no_pull_request(&mut page, &in_round(chosen, round));
        }
        (Selection::Team(team), _) => {
            page.line(0, &format!("Team {team}"));
            team_part(&mut page, start, true);
            for round in rounds_of(chosen) {
                page.blank();
                round_part(&mut page, chosen, &round, team, options);
                page.line(0, "Story");
                page.blank();
                let own = in_round(chosen, &round);
                blocks(&mut page, &own, options);
                no_pull_request(&mut page, &own);
            }
            let alone: Vec<&SequencedRecord> = chosen
                .iter()
                .copied()
                .filter(|record| record.rounds.is_empty() && !matches!(record.record, Record::Team(_)))
                .collect();
            if !alone.is_empty() {
                page.blank();
                page.line(0, "Team events");
                page.blank();
                blocks(&mut page, &alone, options);
            }
        }
        (Selection::Round(_), None) => {}
    }
    page.text
}

/// The records that belong to `round` and tell what happened in it, not its opening.
fn in_round<'a>(chosen: &[&'a SequencedRecord], round: &RoundId) -> Vec<&'a SequencedRecord> {
    chosen
        .iter()
        .copied()
        .filter(|record| record.rounds.contains(round) && !matches!(record.record, Record::Round(_)))
        .collect()
}

/// The rounds of the records, in the order they were opened in, and then the ones that have records but no opening.
fn rounds_of(chosen: &[&SequencedRecord]) -> Vec<RoundId> {
    let mut rounds: Vec<RoundId> = Vec::new();
    for record in chosen {
        if let Record::Round(opening) = &record.record
            && !rounds.contains(&opening.round)
        {
            rounds.push(opening.round.clone());
        }
    }
    for record in chosen {
        for round in &record.rounds {
            if !rounds.contains(round) {
                rounds.push(round.clone());
            }
        }
    }
    rounds
}

/// The heading of a round and its item in full.
fn round_part(page: &mut Page, chosen: &[&SequencedRecord], round: &RoundId, team: &TeamId, _options: StoryOptions) {
    page.line(0, &format!("Round {round} of team {team}"));
    let opening = chosen.iter().find_map(|record| match &record.record {
        Record::Round(opening) if &opening.round == round => Some((*record, opening.as_ref())),
        _ => None,
    });
    let Some((stored, opening)) = opening else {
        page.line(0, "(the opening of the round was not recorded)");
        page.blank();
        return;
    };
    subject(page, opening);
    page.cut_note(0, stored);
    page.blank();
}

/// What a round is about, in full.
fn subject(page: &mut Page, opening: &RoundOpening) {
    match &opening.subject {
        RoundSubject::Item(item) => {
            page.line(0, &format!("Item {}: {}", item.item, item.title));
            page.blank();
            page.line(0, &item.description);
            if let Some(expected) = &item.expected_result {
                page.blank();
                page.line(0, "Expected result:");
                page.line(0, expected);
            }
            if let Some(checks) = &item.checks {
                page.blank();
                page.line(0, "Checks:");
                page.line(0, checks);
            }
        }
        RoundSubject::Brief(brief) => {
            page.line(0, "Brief:");
            page.line(0, brief);
        }
    }
}

/// The start of the team and its agents; `with_list` adds the size of its task list.
fn team_part(page: &mut Page, start: Option<(&SequencedRecord, &TeamStart)>, with_list: bool) {
    let Some((stored, start)) = start else {
        page.line(0, "(the start of the team was not recorded)");
        page.blank();
        return;
    };
    page.line(
        0,
        &format!(
            "Team started {}: repo {}, base branch {}, merge {}",
            start.at,
            start.repo,
            start.base_branch.as_str(),
            start.merge
        ),
    );
    if with_list {
        page.line(
            0,
            &format!(
                "Task list: {} lines, {} bytes",
                start.task_list.lines().count(),
                start.task_list.len()
            ),
        );
    }
    page.cut_note(0, stored);
    page.blank();
    page.line(0, "Agents");
    for entry in &start.plan {
        page.line(
            0,
            &format!(
                "{}. {} — {} — {}",
                entry.position,
                entry.agent.as_str(),
                entry.role,
                how(&entry.how)
            ),
        );
    }
    page.blank();
}

/// How an agent got its role, in words.
fn how(how: &HowChosen) -> String {
    match how {
        HowChosen::Named => "named on the command line".to_owned(),
        HowChosen::NamedByFlag => "named by --coordinator".to_owned(),
        HowChosen::NamedByLine => "named by the Coordinator: line of the task list".to_owned(),
        HowChosen::Drawn { among } => {
            let names: Vec<&str> = among.iter().map(AgentName::as_str).collect();
            format!("drawn at random among {}", names.join(", "))
        }
        HowChosen::ConfiguredOrder => "a worker, in the order of the configuration".to_owned(),
    }
}

/// Says that a round has no pull request in the history, when it has none.
fn no_pull_request(page: &mut Page, records: &[&SequencedRecord]) {
    let has = records.iter().any(|record| match &record.record {
        Record::Event(event) => is_pull_request(&event.event),
        _ => false,
    });
    if !has {
        page.blank();
        page.line(0, "no pull request recorded for this round");
    }
}

/// Whether an event is about a pull request.
const fn is_pull_request(event: &HistoryEvent) -> bool {
    match event {
        HistoryEvent::PullOpened { .. }
        | HistoryEvent::PullPushed { .. }
        | HistoryEvent::PullReady { .. }
        | HistoryEvent::ReviewAsked { .. }
        | HistoryEvent::ReviewSubmitted { .. }
        | HistoryEvent::FixPassStarted { .. }
        | HistoryEvent::PullApproved { .. }
        | HistoryEvent::PullMerged { .. }
        | HistoryEvent::PullHandedOver { .. }
        | HistoryEvent::PullClosed { .. } => true,
        HistoryEvent::Gate { .. }
        | HistoryEvent::Checked { .. }
        | HistoryEvent::QuestionAsked { .. }
        | HistoryEvent::QuestionAnswered { .. }
        | HistoryEvent::Controlled { .. }
        | HistoryEvent::Notified { .. }
        | HistoryEvent::RoundClosed { .. }
        | HistoryEvent::TeamEnded {} => false,
    }
}

/// Everything recorded about one run.
#[derive(Debug, Default)]
struct RunBlock<'a> {
    /// The run, when no start is recorded.
    run: Option<&'a RunId>,
    /// How it started.
    start: Option<(&'a SequencedRecord, &'a RunStart)>,
    /// How its process started.
    launch: Option<(&'a SequencedRecord, &'a Launch)>,
    /// Its steps.
    steps: Vec<(&'a SequencedRecord, &'a Step)>,
    /// Its sessions.
    sessions: Vec<&'a RunSession>,
    /// How it ended.
    end: Option<(&'a SequencedRecord, &'a RunEndRecord)>,
    /// What it changed.
    changes: Option<&'a RunChanges>,
}

/// One thing the story tells, at its place in time.
#[derive(Debug)]
enum Item<'a> {
    /// A run, with all that was recorded about it, at its first record.
    Run(usize),
    /// An event.
    Event(&'a SequencedRecord, &'a EventRecord),
}

/// Tells `records`, in the order they come, as runs and events.
fn blocks(page: &mut Page, records: &[&SequencedRecord], options: StoryOptions) {
    let mut items: Vec<Item<'_>> = Vec::new();
    let mut runs: Vec<RunBlock<'_>> = Vec::new();
    let mut index: HashMap<&RunId, usize> = HashMap::new();
    for stored in records {
        let run_of = match &stored.record {
            Record::Run(start) => Some(&start.run),
            Record::Launch(launch) => Some(&launch.run),
            Record::Step(step) => Some(&step.run),
            Record::RunEnd(end) => Some(&end.run),
            Record::Changes(changes) => Some(&changes.run),
            Record::Session(session) => Some(&session.run),
            Record::Team(_) | Record::Round(_) | Record::Event(_) => None,
        };
        if let Some(run) = run_of {
            let at = *index.entry(run).or_insert_with(|| {
                runs.push(RunBlock {
                    run: Some(run),
                    ..RunBlock::default()
                });
                items.push(Item::Run(runs.len() - 1));
                runs.len() - 1
            });
            if let Some(entry) = runs.get_mut(at) {
                match &stored.record {
                    Record::Run(start) => entry.start = Some((stored, start)),
                    Record::Launch(launch) => entry.launch = Some((stored, launch)),
                    Record::Step(step) => entry.steps.push((stored, step)),
                    Record::RunEnd(end) => entry.end = Some((stored, end)),
                    Record::Changes(changes) => entry.changes = Some(changes),
                    Record::Session(session) => entry.sessions.push(session),
                    Record::Team(_) | Record::Round(_) | Record::Event(_) => {}
                }
            }
        } else if let Record::Event(event) = &stored.record {
            items.push(Item::Event(stored, event));
        }
    }
    for item in items {
        match item {
            Item::Run(at) => {
                if let Some(entry) = runs.get(at) {
                    run_block(page, entry, options);
                }
            }
            Item::Event(stored, event) => event_block(page, stored, event),
        }
    }
}

/// The time and the text that open an entry of the story.
fn entry(page: &mut Page, at: Timestamp, text: &str) {
    page.line(0, &format!("{at}  {text}"));
}

/// A run: who, why, where, how long, its prompt, its command, its steps, its end and its changes.
fn run_block(page: &mut Page, block: &RunBlock<'_>, options: StoryOptions) {
    let Some((stored, start)) = block.start else {
        let run = block.run.map_or_else(String::new, ToString::to_string);
        let at = block
            .launch
            .map(|(stored, _)| stored.at)
            .or_else(|| block.end.map(|(stored, _)| stored.at));
        match at {
            Some(at) => entry(page, at, &format!("run {run}: the start of the run was not recorded")),
            None => page.line(0, &format!("run {run}: the start of the run was not recorded")),
        }
        run_rest(page, block, None, options);
        return;
    };
    let model = start
        .agent
        .model
        .as_ref()
        .map_or_else(String::new, |model| format!(", {model}"));
    entry(
        page,
        start.at,
        &format!(
            "run {}: {} ({}, {}{model}) as {}, because {}",
            start.run,
            start.agent.name,
            start.agent.tier,
            start.agent.cli,
            start.role,
            why(&start.why)
        ),
    );
    page.line(
        4,
        &format!(
            "task {}; worktree {}, branch {}",
            start.task,
            start.worktree.as_path().display(),
            start.branch.as_str()
        ),
    );
    page.cut_note(4, stored);
    run_rest(page, block, Some(start), options);
}

/// Why a run started, in words.
fn why(why: &RunWhy) -> String {
    match why {
        RunWhy::FirstRun => "it is the first run of the task".to_owned(),
        RunWhy::Item { item } => format!("the task was given item {item}"),
        RunWhy::WakeUp => "a message or a board item woke the task".to_owned(),
        RunWhy::Request { asked_by: Some(run) } => format!("a worker serves a request of run {run}"),
        RunWhy::Request { asked_by: None } => "a worker serves a request".to_owned(),
        RunWhy::FixPass { number } => format!("fix pass {number} of the item"),
        RunWhy::Review { pr, pass } => format!("a review of pull request #{pr}, pass {}", pass.saturating_add(1)),
        RunWhy::AskHuman => "the fix passes are used up: the human is asked".to_owned(),
        RunWhy::VerdictMissing => "the report had no verdict: it is asked again".to_owned(),
        RunWhy::Checks => "the checks before a review sent the round back".to_owned(),
        RunWhy::Answers => "the human answered the questions of the task".to_owned(),
        RunWhy::ResumedByHuman => "the human stopped the task and let it go on".to_owned(),
        RunWhy::Continuation => "the task goes on after a restart, a hang or a limit".to_owned(),
    }
}

/// The rest of a run: how long, the prompt, the command, the steps, the end and the changes.
fn run_rest(page: &mut Page, block: &RunBlock<'_>, start: Option<&RunStart>, options: StoryOptions) {
    match (start, block.end) {
        (Some(start), Some((_, end))) => page.line(4, &took(start.at, end.at)),
        (None, Some(_)) => {}
        (_, None) => page.line(
            4,
            "no end recorded (the run is going, or the daemon did not see it end)",
        ),
    }
    if let Some(start) = start {
        prompt(page, start, block, options);
    }
    if let Some((stored, launch)) = block.launch {
        command(page, stored, launch, start);
    }
    steps(page, block, options);
    for session in &block.sessions {
        page.line(
            4,
            &format!("session {} observed at {}", session.session.as_str(), session.at),
        );
    }
    if let Some((stored, end)) = block.end {
        page.line(4, &format!("ended: {}", run_end(&end.end)));
        if let Some(message) = &end.final_message {
            page.raw(4, "final message", message);
        }
        page.cut_note(4, stored);
    }
    if let Some(changes) = block.changes {
        changed(page, changes);
    }
}

/// How long a run took, in words; a clock that went back is said, not hidden.
fn took(from: Timestamp, to: Timestamp) -> String {
    let span = to.as_utc() - from.as_utc();
    if span.is_negative() {
        return "took no time that can be told: the clock went back".to_owned();
    }
    let seconds = span.whole_seconds();
    if seconds == 0 && span.subsec_nanoseconds() > 0 {
        return "took under 1s".to_owned();
    }
    let (hours, minutes, rest) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    match (hours, minutes) {
        (0, 0) => format!("took {rest}s"),
        (0, _) => format!("took {minutes}m {rest:02}s"),
        _ => format!("took {hours}h {minutes:02}m {rest:02}s"),
    }
}

/// The prompt of a run: its size and its first lines, or all of it.
fn prompt(page: &mut Page, start: &RunStart, block: &RunBlock<'_>, options: StoryOptions) {
    let lines = start.prompt.lines().count();
    page.line(4, &format!("prompt: {lines} lines, {} bytes", start.prompt.len()));
    if let Some((stored, _)) = block.start {
        page.cut_note(4, stored);
    }
    match options.prompts {
        PromptView::Whole => page.raw(4, "prompt", &start.prompt),
        PromptView::Preview => {
            for line in start.prompt.lines().take(PREVIEW_LINES) {
                page.line(6, line);
            }
            if lines > PREVIEW_LINES {
                page.line(6, &format!("… {} more lines", lines - PREVIEW_LINES));
            }
        }
    }
}

/// The command line a process was started with; an argument that is the prompt is shown as `<the prompt>`.
fn command(page: &mut Page, stored: &SequencedRecord, launch: &Launch, start: Option<&RunStart>) {
    let args: Vec<String> = launch
        .args
        .iter()
        .map(|arg| match start {
            Some(start) if !arg.is_empty() && *arg == start.prompt => "<the prompt>".to_owned(),
            _ => quoted(arg),
        })
        .collect();
    let mut words = vec![quoted(&launch.program)];
    words.extend(args);
    page.line(4, &format!("command: {}", words.join(" ")));
    page.cut_note(4, stored);
    let head = launch
        .head
        .as_ref()
        .map_or_else(|| "unknown (git could not say)".to_owned(), short);
    page.line(
        4,
        &format!(
            "started {} in {}, at commit {head}{}",
            launch.at,
            launch.working_dir.as_path().display(),
            if launch.prompt_in_file {
                ", the prompt in a file"
            } else {
                ""
            }
        ),
    );
}

/// `text` as a shell would need it written, when that differs.
fn quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '=' | '@' | '%' | '+' | ',' | '-')
        });
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// The first characters of a commit's ID.
fn short(commit: &CommitId) -> String {
    commit.as_str().chars().take(SHORT_COMMIT_CHARS).collect()
}

/// The steps of a run, numbered, or how many they were.
fn steps(page: &mut Page, block: &RunBlock<'_>, options: StoryOptions) {
    match options.steps {
        StepView::LeftOut => page.line(4, &format!("steps: {} (left out)", block.steps.len())),
        StepView::Listed => {
            if block.steps.is_empty() {
                page.line(4, "steps: none");
                return;
            }
            page.line(4, &format!("steps ({}):", block.steps.len()));
            let mut listed: Vec<&(&SequencedRecord, &Step)> = block.steps.iter().collect();
            listed.sort_by_key(|(_, step)| step.number);
            for (stored, step) in listed {
                let cut: Vec<&str> = stored.cut.fields().collect();
                let note = if cut.is_empty() {
                    String::new()
                } else {
                    format!(" (cut: {})", cut.join(", "))
                };
                page.line(
                    6,
                    &format!("{}. {} {}{note}", step.number, step.at, step_text(&step.event)),
                );
            }
        }
    }
}

/// A step as `kind: what`, on one line.
fn step_text(event: &ActivityEvent) -> String {
    let (kind, text) = match event {
        ActivityEvent::Text(text) => ("message", text.clone()),
        ActivityEvent::Command(text) => ("command", text.clone()),
        ActivityEvent::FileEdit { path } => ("file edit", path.clone()),
        ActivityEvent::BusCall { tool, arguments } => ("bus call", format!("{tool} {arguments}")),
        ActivityEvent::ToolCall { name, summary } => ("tool call", format!("{name} {summary}")),
        ActivityEvent::Error(text) => ("error", text.clone()),
        ActivityEvent::Info(text) => ("info", text.clone()),
        ActivityEvent::Control { action, by } => ("control", format!("{action} {}", client(by))),
    };
    let first = text.lines().next().unwrap_or_default();
    let mut shown: String = first.chars().take(STEP_SUMMARY_CHARS).collect();
    if first.chars().count() > STEP_SUMMARY_CHARS || text.lines().count() > 1 {
        shown.push('…');
    }
    format!("{kind}: {shown}")
}

/// Who used a control, in words.
fn client(client: &Client) -> String {
    match client {
        Client::Cli => "from the terminal".to_owned(),
        Client::Dashboard { login } => format!("through the dashboard as {}", login.as_str()),
    }
}

/// How a run ended, in words.
fn run_end(end: &RunEnd) -> String {
    match end {
        RunEnd::Reported { outcome } => format!("reported {outcome}"),
        RunEnd::NoStatus { exit_code: Some(code) } => format!("no STATUS line (exit code {code})"),
        RunEnd::NoStatus { exit_code: None } => "no STATUS line (ended by a signal)".to_owned(),
        RunEnd::QuotaExhausted { resets_at: Some(at) } => format!("quota exhausted (back at {at})"),
        RunEnd::QuotaExhausted { resets_at: None } => "quota exhausted".to_owned(),
        RunEnd::RateLimited => "rate limited".to_owned(),
        RunEnd::Hung => "hung: stopped by the watchdog".to_owned(),
        RunEnd::Stopped => "stopped by the human".to_owned(),
    }
}

/// What a run changed: the heads, the commits, the files and what was left uncommitted.
fn changed(page: &mut Page, changes: &RunChanges) {
    let head = |head: &Option<CommitId>| head.as_ref().map_or_else(|| "unknown".to_owned(), short);
    page.line(
        4,
        &format!(
            "changes: head {} → {}",
            head(&changes.head_before),
            head(&changes.head_after)
        ),
    );
    for commit in &changes.commits {
        page.line(6, &format!("commit {} {}", short(&commit.id), commit.subject));
    }
    for file in &changes.files {
        page.line(6, &file_text(file));
    }
    for file in &changes.uncommitted {
        page.line(6, &format!("uncommitted: {}", file_text(file)));
    }
}

/// A changed file as `how path (+added -removed)`.
fn file_text(file: &FileChange) -> String {
    let lines = match file.lines {
        Lines::Text { added, removed } => format!("(+{added} -{removed})"),
        Lines::Binary => "(binary)".to_owned(),
    };
    match &file.change {
        Change::Added => format!("added {} {lines}", file.path.as_str()),
        Change::Modified => format!("modified {} {lines}", file.path.as_str()),
        Change::Deleted => format!("deleted {} {lines}", file.path.as_str()),
        Change::Renamed { from } => format!("renamed {} → {} {lines}", from.as_str(), file.path.as_str()),
    }
}

/// An event: one line with its time, and the texts it carries whole.
fn event_block(page: &mut Page, stored: &SequencedRecord, record: &EventRecord) {
    let at = record.at;
    match &record.event {
        HistoryEvent::Gate {
            task,
            started_at,
            record: gate,
        } => {
            let log = gate.result.log.as_path().display();
            let how = took(*started_at, gate.result.finished_at);
            entry(
                page,
                at,
                &format!("gate for task {task}: {} ({how}); output {log}", gate.summary()),
            );
        }
        HistoryEvent::Checked {
            head, issues, outcome, ..
        } => checked(page, at, head, issues, outcome),
        HistoryEvent::Controlled {
            action,
            target,
            by,
            tasks,
        } => controlled(page, at, *action, target, by, tasks),
        HistoryEvent::QuestionAsked { task, number, text } => {
            entry(page, at, &format!("task {task} asked the human (questions {number})"));
            page.raw(2, "questions", text);
        }
        HistoryEvent::QuestionAnswered {
            task,
            number,
            text,
            via,
        } => {
            entry(
                page,
                at,
                &format!("task {task}: the human answered questions {number} ({})", channel(*via)),
            );
            page.raw(2, "answered questions", text);
        }
        HistoryEvent::Notified { task, title, body } => {
            let about = task.as_ref().map_or_else(String::new, |task| format!(" (task {task})"));
            entry(page, at, &format!("notification sent: {title}{about}"));
            page.raw(2, "notification", body);
        }
        HistoryEvent::RoundClosed { round, end } => entry(page, at, &format!("round {round} closed: {end}")),
        HistoryEvent::TeamEnded {} => entry(page, at, "team ended"),
        HistoryEvent::PullOpened { .. }
        | HistoryEvent::PullPushed { .. }
        | HistoryEvent::PullReady { .. }
        | HistoryEvent::ReviewAsked { .. }
        | HistoryEvent::ReviewSubmitted { .. }
        | HistoryEvent::FixPassStarted { .. }
        | HistoryEvent::PullApproved { .. }
        | HistoryEvent::PullMerged { .. }
        | HistoryEvent::PullHandedOver { .. }
        | HistoryEvent::PullClosed { .. } => pull_request_event(page, at, &record.event),
    }
    page.cut_note(2, stored);
}

/// The checks before a review found issues.
fn checked(page: &mut Page, at: Timestamp, head: &CommitId, issues: &[String], outcome: &CheckOutcome) {
    let plural = if issues.len() == 1 { "issue" } else { "issues" };
    entry(
        page,
        at,
        &format!(
            "checks before a review found {} {plural} at {}:",
            issues.len(),
            short(head)
        ),
    );
    for issue in issues {
        page.line(2, &format!("- {issue}"));
    }
    page.line(2, &outcome_text(outcome));
}

/// The human used a control.
fn controlled(page: &mut Page, at: Timestamp, action: ControlAction, target: &Target, by: &Client, tasks: &[TaskId]) {
    let reached: Vec<String> = tasks.iter().map(|task| format!("task {task}")).collect();
    let reached = if reached.is_empty() {
        "no task".to_owned()
    } else {
        reached.join(", ")
    };
    entry(
        page,
        at,
        &format!(
            "the human {} {} ({}); reached {reached}",
            control_verb(action),
            target_text(target),
            client(by)
        ),
    );
}

/// An event of a pull request, from its opening to its end; any other event is not told here.
fn pull_request_event(page: &mut Page, at: Timestamp, event: &HistoryEvent) {
    match event {
        HistoryEvent::PullOpened {
            number,
            branch,
            head,
            title,
            body,
        } => {
            let opened = format!(
                "pull request #{number} opened: branch {} at {}",
                branch.as_str(),
                short(head)
            );
            entry(page, at, &opened);
            page.line(2, &format!("title: {title}"));
            page.raw(2, "pull request body", body);
        }
        HistoryEvent::PullPushed { pr, head } => {
            entry(page, at, &format!("pull request #{pr}: pushed {}", short(head)));
        }
        HistoryEvent::PullReady { pr } => entry(page, at, &format!("pull request #{pr} marked ready for review")),
        HistoryEvent::ReviewAsked {
            pr,
            pass,
            head,
            reviewer,
            task,
        } => {
            let pass = pass.saturating_add(1);
            let reviewer = reviewer.as_str();
            let head = short(head);
            entry(
                page,
                at,
                &format!("review pass {pass} of #{pr} asked of {reviewer} (task {task}) at {head}"),
            );
        }
        HistoryEvent::ReviewSubmitted {
            pr,
            pass,
            reviewer,
            verdict,
            report,
            ..
        } => {
            let pass = pass.saturating_add(1);
            entry(
                page,
                at,
                &format!("review pass {pass} of #{pr} by {}: {verdict}", reviewer.as_str()),
            );
            page.raw(2, "review report", report);
        }
        HistoryEvent::FixPassStarted { pr, pass, of, findings } => {
            entry(page, at, &format!("fix pass {pass} of {of} on #{pr} started"));
            page.raw(2, "findings sent to the team", findings);
        }
        HistoryEvent::PullApproved { pr } => entry(page, at, &format!("pull request #{pr} approved by both reviews")),
        HistoryEvent::PullMerged {
            pr,
            commit: Some(commit),
        } => entry(page, at, &format!("pull request #{pr} merged as {}", short(commit))),
        HistoryEvent::PullMerged { pr, commit: None } => {
            entry(
                page,
                at,
                &format!("pull request #{pr} merged (the merge commit was not recorded)"),
            );
        }
        HistoryEvent::PullHandedOver { pr } => {
            entry(
                page,
                at,
                &format!("pull request #{pr} approved and handed to the human (--no-merge)"),
            );
        }
        HistoryEvent::PullClosed { pr } => entry(page, at, &format!("pull request #{pr} closed without a merge")),
        HistoryEvent::Gate { .. }
        | HistoryEvent::Checked { .. }
        | HistoryEvent::QuestionAsked { .. }
        | HistoryEvent::QuestionAnswered { .. }
        | HistoryEvent::Controlled { .. }
        | HistoryEvent::Notified { .. }
        | HistoryEvent::RoundClosed { .. }
        | HistoryEvent::TeamEnded {} => {}
    }
}

/// What followed the checks, in words.
fn outcome_text(outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::SentBack { pass, of } => format!("→ sent back to the team (pass {pass} of {of})"),
        CheckOutcome::AskedHuman { reason, question } => format!(
            "→ the human is asked in questions {question} ({})",
            match reason {
                AskReason::OnlyTheHumanMayAllow => "only the human may allow these issues",
                AskReason::PassesUsed => "the team fixed nothing in the passes the checks allow",
            }
        ),
        CheckOutcome::Allowed => "→ allowed by the human: the reviews go on".to_owned(),
    }
}

/// The channel an answer came through, in words.
const fn channel(via: AnsweredVia) -> &'static str {
    match via {
        AnsweredVia::Command => "command",
        AnsweredVia::SavedFile => "saved file",
        AnsweredVia::Dashboard => "dashboard",
    }
}

/// What a control did, in the past tense.
const fn control_verb(action: ControlAction) -> &'static str {
    match action {
        ControlAction::Stop => "stopped",
        ControlAction::Pause => "paused",
        ControlAction::Unpause => "unpaused",
        ControlAction::Resume => "resumed",
        ControlAction::Say => "said something to",
    }
}

/// What a control was aimed at, in words.
fn target_text(target: &Target) -> String {
    match target {
        Target::Team(team) => format!("team {team}"),
        Target::Task(task) => format!("task {task}"),
        Target::Round(round) => format!("round {round}"),
    }
}
