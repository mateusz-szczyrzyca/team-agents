//! The story of a round as `agent log` tells it, from a history a daemon wrote with fake CLIs and a fake GitHub and read
//! after it stopped: each golden story is made by a round that really ran, with its IDs, times, paths and commits
//! normalised. Before they are, every reviewer's prompt of every review pass is compared with what the fake CLI got, and the
//! records of the round with the number of lines `--json` prints.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use agent_core::history::{HistoryEvent, Record, RunWhy};
use agent_core::ids::{RoundId, TeamId};
use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::history::Reader;
use rusqlite::Connection;

use super::rig::{ONE, Rig, investigator, normal};
use crate::environment::Environment;

/// Both outside reviewers approve every review pass.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// An approving reviewer that ends three seconds after the other, so that the order of the two is not a race.
const LATE_APPROVING: &str = "late:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes three seconds after the other has approved, and approves the second time.
const LATE_CHANGES_ONCE: &str = "late:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,late:VERDICT: APPROVE\\nSUMMARY: fixed";

/// A rig whose watchdog gives the stand-ins time: its clock ticks at every reading, so the minute it waits for a silent run is
/// a second and a half of the real time, which a stand-in that waits for the test or for another agent must outlast.
pub(super) async fn patient(agents: &[super::rig::Agent], fix_passes: u32, bus: bool) -> Rig {
    let rig = Rig::new(agents, fix_passes, bus).await;
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let patient = text.replace("hang_minutes = 1\n", "hang_minutes = 100000\n");
    assert_ne!(patient, text);
    std::fs::write(rig.home.config_file(), patient).unwrap();
    rig
}

/// Runs `agent` with `args` against the home of `rig`, which is stopped: no daemon runs, and none is asked.
fn agent(rig: &Rig, args: &[&str]) -> (ExitCode, String, String) {
    let environment = Environment::new(Some(OsString::from(rig.home.root())));
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run_in(
        &environment,
        std::iter::once("agent").chain(args.iter().copied()),
        &mut out,
        &mut err,
    )
    .unwrap();
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

/// The text `agent log` prints with `args`, which succeeds with nothing on standard error.
pub(super) fn log(rig: &Rig, args: &[&str]) -> String {
    let (code, out, err) = agent(rig, args);
    assert_eq!((code, err.as_str()), (ExitCode::SUCCESS, ""), "agent {args:?}");
    out
}

/// Asks the daemon for the gate of `task` once the run `run` of its stand-in CLI has committed, and lets the CLI go on: the
/// gate that gates the commit is the daemon's own.
pub(super) async fn gate_when_committed(rig: &Rig, task: &agent_core::ids::TaskId, run: u32) {
    use agent_core::protocol::{Client, Command, GateRequest, Reply};
    let worktree = rig.worktree(task);
    rig.wait_for("the stand-in to have committed", || {
        worktree.join(format!(".fake-committed-{run}")).exists()
    })
    .await;
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request_until_answered(Client::Cli, Command::Gate(GateRequest { task: task.clone() }))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::GateRecorded { .. }), "{reply:?}");
    std::fs::write(worktree.join(format!(".fake-gate-done-{run}")), "").unwrap();
}

/// Asks the daemon for the gate of every run of `task` that commits and waits for it, until `done` holds of the state.
pub(super) async fn gate_every_run(
    rig: &Rig,
    task: &agent_core::ids::TaskId,
    done: impl Fn(&agent_core::state::State) -> bool,
) {
    use agent_core::protocol::{Client, Command, GateRequest, Reply};
    let worktree = rig.worktree(task);
    let mut gated = 0;
    while !done(&rig.state()) {
        let next = gated + 1;
        if worktree.join(format!(".fake-committed-{next}")).exists() {
            let reply = crate::client::Connection::connect(&rig.home.socket_file())
                .await
                .unwrap()
                .request_until_answered(Client::Cli, Command::Gate(GateRequest { task: task.clone() }))
                .await
                .unwrap();
            assert!(matches!(reply, Reply::GateRecorded { .. }), "{reply:?}");
            std::fs::write(worktree.join(format!(".fake-gate-done-{next}")), "").unwrap();
            gated = next;
        } else if worktree.join(".fake-runs").exists() && gated < 20 && next_is_over(&worktree, next) {
            // A run that did not commit has no gate to wait for: the next run is the one that may.
            gated = next;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Whether the run `n` of the stand-in in `worktree` ended without committing and waiting: a later run has started.
fn next_is_over(worktree: &Path, n: u32) -> bool {
    std::fs::read_to_string(worktree.join(".fake-runs"))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .is_some_and(|runs| runs > n)
}

/// The only round of the team of `rig`.
fn the_round(rig: &Rig, team: &TeamId) -> RoundId {
    rig.state().team(team).unwrap().rounds()[0].id().clone()
}

/// The number of records the database has for `round`: those linked to it and the start of its team, counted in SQL.
fn records_of_round(rig: &Rig, round: &RoundId) -> usize {
    let raw = Connection::open(rig.home.history_file()).unwrap();
    let count: u32 = raw
        .query_row(
            "SELECT count(*) FROM history_records WHERE sequence IN \
             (SELECT sequence FROM history_record_rounds WHERE round = ?1) \
             OR (kind = 'team' AND team = (SELECT team FROM rounds WHERE round = ?1))",
            [round.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    usize::try_from(count).unwrap()
}

/// Checks what has to hold before a story is normalised: every run's prompt in the history is what its fake CLI got, every
/// reviewer's prompt of every review pass is in the whole story, and `--json` has a line for each record of the round.
pub(super) fn check_story(rig: &Rig, team: &TeamId) -> (String, String) {
    let round = the_round(rig, team);
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let state = rig.state();
    let mut reviews = 0;
    for row in reader.runs_of_round(&round).unwrap() {
        let start = &row.start.value;
        let task = &start.task;
        let number = state
            .find_task(task)
            .unwrap()
            .member
            .runs()
            .iter()
            .position(|run| *run.id() == start.run)
            .unwrap()
            + 1;
        let kept = std::fs::read_to_string(rig.worktree(task).join(format!(".fake-prompt-{number}"))).unwrap();
        assert_eq!(format!("{}\n", start.prompt), kept, "the prompt of run {}", start.run);
        if matches!(start.why, RunWhy::Review { .. }) {
            reviews += 1;
        }
    }
    assert_ne!(reviews, 0, "no review pass ran");
    let whole = log(rig, &["log", "--round", round.as_str(), "--prompts"]);
    for row in reader.runs_of_round(&round).unwrap() {
        assert!(
            whole.contains(&row.start.value.prompt),
            "the prompt of run {} is not whole",
            row.start.value.run
        );
    }
    let json = log(rig, &["log", "--round", round.as_str(), "--json"]);
    assert_eq!(json.lines().count(), records_of_round(rig, &round));
    (log(rig, &["log", "--round", round.as_str()]), json)
}

/// The story of a round with its IDs, times, paths and commits replaced by stand-ins that keep their order of appearance.
pub(super) struct Normalized<'a> {
    /// The text so far.
    text: String,
    /// The rig the story is of.
    rig: &'a Rig,
}

impl<'a> Normalized<'a> {
    pub(super) fn of(rig: &'a Rig, text: &str, json: &str) -> String {
        let mut me = Self {
            text: text.to_owned(),
            rig,
        };
        me.paths();
        me.commits(json);
        me.ids();
        me.times();
        me.volatile();
        me.text
    }

    /// The directories of the test and of the build.
    fn paths(&mut self) {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).display().to_string();
        let agentbus = super::super::fixture::agentbus_binary().display().to_string();
        for (from, to) in [
            (self.rig.home.root().display().to_string(), "<home>"),
            (self.rig.repo.display().to_string(), "<repo>"),
            (self.rig.origin.display().to_string(), "<origin>"),
            (agentbus, "<agentbus>"),
            (crate_dir, "<crate>"),
        ] {
            self.text = self.text.replace(&from, to);
        }
    }

    /// The commits, by the order they come in the records: whole in the JSON, short in the text.
    fn commits(&mut self, json: &str) {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for word in json.split(|c: char| !c.is_ascii_hexdigit()) {
            if word.len() == 40 && !seen.contains_key(word) {
                seen.insert(word.to_owned(), order.len() + 1);
                order.push(word.to_owned());
            }
        }
        for (index, commit) in order.iter().enumerate() {
            let label = format!("<commit-{}>", index + 1);
            self.text = self.text.replace(commit.as_str(), &label);
            let short: String = commit.chars().take(7).collect();
            self.text = self.text.replace(&short, &label);
        }
    }

    /// Team, round, run and task IDs, numbered in the order they first appear in.
    fn ids(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        let mut out = String::with_capacity(self.text.len());
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let rest: String = chars[at..chars.len().min(at + 32)].iter().collect();
            let kind = ["team", "round", "run", "task"]
                .into_iter()
                .find(|kind| rest.strip_prefix(&format!("{kind}-")).is_some_and(is_id_tail));
            if let Some(kind) = kind {
                let length = kind.len() + 1 + 20;
                let id: String = chars[at..at + length].iter().collect();
                let number = seen.iter().position(|known| *known == id).unwrap_or_else(|| {
                    seen.push(id.clone());
                    seen.len() - 1
                });
                write!(out, "<{kind}-{}>", number + 1).unwrap();
                at += length;
            } else {
                out.push(chars[at]);
                at += 1;
            }
        }
        self.text = out;
    }

    /// RFC 3339 times.
    fn times(&mut self) {
        let mut out = String::with_capacity(self.text.len());
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let rest: String = chars[at..chars.len().min(at + 40)].iter().collect();
            if let Some(length) = time_length(&rest) {
                out.push_str("<time>");
                at += length;
            } else {
                out.push(chars[at]);
                at += 1;
            }
        }
        self.text = out;
    }

    /// What differs between two runs of the same round: durations, the size of the prompts, session IDs.
    fn volatile(&mut self) {
        let mut lines = Vec::new();
        for line in self.text.lines() {
            let trimmed = line.trim_start();
            let indent = " ".repeat(line.chars().count() - trimmed.chars().count());
            let mut line = if trimmed.starts_with("took ") {
                format!("{indent}took <duration>")
            } else if trimmed.starts_with("prompt: ") && trimmed.contains(" lines, ") {
                format!("{indent}prompt: <n> lines, <n> bytes")
            } else if trimmed.starts_with("… ") && trimmed.ends_with(" more lines") {
                format!("{indent}… <n> more lines")
            } else if trimmed.contains(".fake-") && trimmed.contains(" (+") {
                let (before, _) = line.rsplit_once(" (+").unwrap();
                format!("{before} (+<n> -<n>)")
            } else {
                line.to_owned()
            };
            if let Some((before, rest)) = line.split_once("(took ")
                && let Some((_, after)) = rest.split_once("s);")
            {
                line = format!("{before}(took <duration>);{after}");
            }
            lines.push(uuids(&line));
        }
        let mut text = lines.join("\n");
        text.push('\n');
        self.text = text;
    }
}

/// Whether `text` is the rest of an ID after its prefix: `YYYYMMDD-HHMMSS-xxxx`.
fn is_id_tail(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 20
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[8] == b'-'
        && bytes[9..15].iter().all(u8::is_ascii_digit)
        && bytes[15] == b'-'
        && bytes[16..20]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The length of the RFC 3339 time `text` starts with, if it starts with one.
fn time_length(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let digits = |from: usize, count: usize| {
        bytes
            .get(from..from + count)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let shape = digits(0, 4)
        && bytes.get(4) == Some(&b'-')
        && digits(5, 2)
        && bytes.get(7) == Some(&b'-')
        && digits(8, 2)
        && bytes.get(10) == Some(&b'T')
        && digits(11, 2)
        && bytes.get(13) == Some(&b':')
        && digits(14, 2)
        && bytes.get(16) == Some(&b':')
        && digits(17, 2);
    if !shape {
        return None;
    }
    let mut length = 19;
    if bytes.get(length) == Some(&b'.') {
        length += 1;
        while bytes.get(length).is_some_and(u8::is_ascii_digit) {
            length += 1;
        }
    }
    (bytes.get(length) == Some(&b'Z')).then_some(length + 1)
}

/// `line` with every UUID replaced.
fn uuids(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let window: String = chars[at..chars.len().min(at + 36)].iter().collect();
        let shaped = window.len() == 36
            && window.char_indices().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_hexdigit(),
            });
        if shaped {
            out.push_str("<uuid>");
            at += 36;
        } else {
            out.push(chars[at]);
            at += 1;
        }
    }
    out
}

/// Compares `actual` with the golden file `name`; with `UPDATE_GOLDEN` in the environment, writes it instead.
pub(super) fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no golden file {}", path.display()));
    assert_eq!(actual, expected, "the story differs from {name}");
}

#[tokio::test]
async fn a_solo_round_merged_at_the_first_review_pass_is_told_from_its_item_to_its_merge() {
    let mut rig = patient(
        &[
            normal("claude-1", "gated"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", LATE_APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    gate_when_committed(&rig, &tasks[0], 1).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.wait_for("the team to end", || {
        Reader::open(&rig.home.history_file())
            .unwrap()
            .team_records(&team)
            .unwrap()
            .iter()
            .any(|stored| matches!(&stored.record, Record::Event(event) if matches!(event.event, HistoryEvent::TeamEnded {})))
    })
    .await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    golden("story-solo-merged.txt", &Normalized::of(&rig, &text, &json));
}

/// Adds the worker `name`, whose stand-in does what `scenario` says, to the configuration of the home of `rig`.
fn add_worker(rig: &Rig, name: &str, scenario: &str) {
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let root = rig.home.root().display();
    let table = format!(
        "[agents.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \"--home\", \"{root}\", \
         \"--bus-bin\", \"{}\", \"--bus-dir\", \"{root}/bus\"]\n",
        super::super::fixture::agentbus_binary().display(),
    );
    let with_tier = text.replace(
        "[tiers]\nnormal = [",
        &format!("[tiers]\nworkers = [\"{name}\"]\nnormal = ["),
    );
    assert_ne!(with_tier, text);
    std::fs::write(rig.home.config_file(), format!("{with_tier}{table}")).unwrap();
}

/// Waits for the team of `rig` to end: its end is the last record of a story.
async fn team_ended(rig: &Rig, team: &TeamId) {
    rig.wait_for("the team to end", || {
        Reader::open(&rig.home.history_file())
            .unwrap()
            .team_records(team)
            .unwrap()
            .iter()
            .any(|stored| matches!(&stored.record, Record::Event(event) if matches!(event.event, HistoryEvent::TeamEnded {})))
    })
    .await;
}

#[tokio::test]
async fn a_team_round_with_a_worker_one_fix_pass_and_a_second_review_pass_that_approves_is_told_whole() {
    use std::str::FromStr;

    use agent_core::config::{AbsolutePath, Program};
    use agent_runtime::agentbus::{AgentBus, Delegation};

    let mut rig = patient(
        &[
            // The coordinator waits for its member, integrates, and then fixes: each of its two commits is gated by the daemon.
            investigator("inv-lead", "sleep,lead-gated"),
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", LATE_CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    add_worker(&rig, "glm-1", "sleep,done:Result: looked");
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;

    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "glm-1"],
            ONE,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    rig.state_where("the member to have run", |state| {
        state
            .find_task(&tasks[1])
            .is_some_and(|found| !found.member.runs().is_empty())
    })
    .await;
    let bus = AgentBus::new(
        Program::from_str(super::super::fixture::agentbus_binary().to_str().unwrap()).unwrap(),
        AbsolutePath::from_str(rig.home.root().to_str().unwrap()).unwrap(),
    );
    bus.delegate(&Delegation {
        team: team.clone(),
        requester: "claude-1".parse().unwrap(),
        task: tasks[1].clone(),
        repo: "example".parse().unwrap(),
        branch: format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap(),
        worktree: AbsolutePath::from_str(rig.worktree(&tasks[1]).to_str().unwrap()).unwrap(),
        dispatcher: Program::from_str("true").unwrap(),
        request: "Look at the module.".to_owned(),
        context: "None.".to_owned(),
    })
    .await
    .unwrap();
    gate_every_run(&rig, &tasks[0], |state| {
        state.teams()[0]
            .item("1.1".parse().unwrap())
            .is_some_and(|progress| matches!(progress.phase(), ItemPhase::Done { .. }))
    })
    .await;
    team_ended(&rig, &team).await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    assert_eq!(text.matches("fix pass 1 of 5 on #1 started").count(), 1, "{text}");
    assert_eq!(text.matches("review pass 2 of #1 by").count(), 2, "{text}");
    assert_eq!(text.matches("gate for task").count(), 2, "{text}");
    assert!(text.contains("as worker, because a worker serves a request"), "{text}");
    golden("story-team-fix-pass.txt", &Normalized::of(&rig, &text, &json));
}

/// A solo agent that works, and the two reviewers: one approves at once, the other three seconds later.
async fn solo_rig(scenario: &'static str) -> Rig {
    patient(
        &[
            normal("claude-1", scenario),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", LATE_APPROVING),
        ],
        5,
        false,
    )
    .await
}

#[tokio::test]
async fn a_no_merge_round_is_told_to_its_approval_and_hand_over_and_the_human_merge_after_it() {
    let mut rig = solo_rig("work").await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.notification_with("is approved").await;
    rig.github.merged_by_a_human(1);
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    team_ended(&rig, &team).await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    assert!(text.contains("approved and handed to the human (--no-merge)"), "{text}");
    assert!(text.contains("closed: approved"), "{text}");
    assert!(text.contains("merged (the merge commit was not recorded)"), "{text}");
    golden("story-no-merge.txt", &Normalized::of(&rig, &text, &json));
}

#[tokio::test]
async fn an_abandoned_round_is_told_with_the_stop_of_the_human_that_ended_it() {
    use agent_core::protocol::{Client, Command, ControlRequest, Target};
    use agent_core::status::TaskStatus;

    let mut rig = solo_rig("sleep").await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;

    crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Team(team.clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[0], TaskStatus::Stopped).await;
    rig.stop().await.unwrap();

    let round = the_round(&rig, &team);
    let text = log(&rig, &["log", "--round", round.as_str()]);
    let json = log(&rig, &["log", "--round", round.as_str(), "--json"]);

    assert!(text.contains("the human stopped team"), "{text}");
    assert!(text.contains("closed: abandoned"), "{text}");
    assert!(text.contains("no pull request recorded for this round"), "{text}");
    assert_eq!(json.lines().count(), records_of_round(&rig, &round));
    golden("story-abandoned.txt", &Normalized::of(&rig, &text, &json));
}

#[tokio::test]
async fn a_round_with_a_question_answered_is_told_with_the_file_the_human_saw_and_their_answers() {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::status::TaskStatus;

    let mut rig = solo_rig("ask,gated").await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: std::num::NonZeroU32::MIN,
        verdict: Verdict::from_text("a").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: std::num::NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    gate_when_committed(&rig, &tasks[0], 2).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    team_ended(&rig, &team).await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    assert!(text.contains("asked the human (questions 1)"), "{text}");
    assert!(text.contains("the human answered questions 1 (command)"), "{text}");
    assert!(text.contains("**Verdict:** a"), "{text}");
    golden("story-question-answered.txt", &Normalized::of(&rig, &text, &json));
}

/// An item whose text is not ASCII and has a line of three thousand characters.
fn unicode_list() -> String {
    format!(
        "Zbuduj rzecz — 日本語 🦀.\n\n1. Pierwsza\n[ ] 1.1 Pierwsza pozycja: zażółć gęślą jaźń\n    Opis: {}\n    Expected result:\n    - działa — très bien\n    Checks:\n    - uruchom to\n",
        "żółć ".repeat(600)
    )
}

#[tokio::test]
async fn a_round_with_text_that_is_not_ascii_and_a_very_long_line_is_told_whole_and_never_wrapped() {
    let mut rig = patient(
        &[
            normal("claude-1", "gated"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: très bien — 日本語"),
            investigator("inv-r2", "late:VERDICT: APPROVE\\nSUMMARY: zażółć gęślą jaźń"),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_rng(rand::SeedableRng::seed_from_u64(7)).await;
    let list = unicode_list();
    let (team, tasks) = rig.run(&["claude-1"], &list, MergeMode::Autopilot, None).await;
    gate_when_committed(&rig, &tasks[0], 1).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    team_ended(&rig, &team).await;
    rig.stop().await.unwrap();

    let (text, json) = check_story(&rig, &team);

    let long = "żółć ".repeat(600);
    // The item, and the pull request body that carries it, hold the long line whole; no line of the story is wrapped.
    assert!(
        text.lines().filter(|line| line.contains(long.trim_end())).count() >= 2,
        "{}",
        text.len()
    );
    assert!(text.contains("Pierwsza pozycja: zażółć gęślą jaźń"), "{text}");
    assert!(text.contains("très bien — 日本語"), "{text}");
    let first: serde_json::Value = serde_json::from_str(json.lines().next().unwrap()).unwrap();
    assert_eq!(first["kind"], "team");
    golden("story-unicode.txt", &Normalized::of(&rig, &text, &json));
}

#[tokio::test]
async fn two_teams_that_ran_at_once_have_stories_that_share_no_record_and_hold_every_record() {
    let mut rig = patient(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
            investigator("inv-r3", APPROVING),
            investigator("inv-r4", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (first, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (second, _) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    // Each pull request waits for the human, so the teams do not collide in the one base branch of the fake GitHub.
    rig.wait_for("both rounds to be closed", || {
        let reader = Reader::open(&rig.home.history_file()).unwrap();
        [&first, &second].iter().all(|team| {
            reader.team_records(team).unwrap().iter().any(|stored| {
                matches!(&stored.record, Record::Event(event) if matches!(event.event, HistoryEvent::RoundClosed { .. }))
            })
        })
    })
    .await;
    rig.stop().await.unwrap();

    let sequences = |team: &TeamId| -> Vec<u64> {
        log(&rig, &["log", "--team", team.as_str(), "--json"])
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["sequence"]
                    .as_u64()
                    .unwrap()
            })
            .collect()
    };
    let (one, two) = (sequences(&first), sequences(&second));

    let raw = Connection::open(rig.home.history_file()).unwrap();
    let total: u32 = raw
        .query_row("SELECT count(*) FROM history_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(one.len() + two.len(), usize::try_from(total).unwrap());
    assert!(one.iter().all(|sequence| !two.contains(sequence)));
}
