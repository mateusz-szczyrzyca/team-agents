//! The reminder of Serena of a run that ended, put on the bus by the daemon: the run's session reads the text of the
//! catalog at its next call of the bus, and the daemon logs that it was delivered. A reminder the bus refuses is logged
//! as undelivered and leaves the end of the run as it is.

use std::path::Path;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, Language};
use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy};
use agent_core::ids::{BranchName, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{MergeMode, Timestamp};
use agent_runtime::clock::SharedClock;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Setup, TracingReporter};
use time::UtcDateTime;

use super::rig::{ONE, Rig, investigator, normal};

/// The name the harness sends its own messages to the bus with.
const BUS_SENDER: &str = "agentbus";

/// The count of searches of code a session is reminded at by default.
const SEARCHES: u64 = 40;

/// The text of the reminder of `searches` searches of code without Serena, as the catalog has it.
async fn reminder_text(searches: u64) -> String {
    let texts = HarnessPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let values = Variables::new().with("count", searches.to_string()).unwrap();
    texts.render(HarnessPrompt::SerenaRemind, &values).unwrap()
}

/// The texts of the chats the harness sent on the bus of `rig`, as its message log keeps them; none before the log is
/// there.
pub(super) fn chats_from_the_harness(rig: &Rig) -> Vec<String> {
    let file = rig.home.root().join("bus/messages.jsonl");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|message| message["from"] == BUS_SENDER)
        .map(|message| message["text"].as_str().unwrap().to_owned())
        .collect()
}

/// Writes a reminder of `searches` searches of `task` into the history of `rig`, as the engine does while the last run of
/// `task` in the state goes on.
fn record_reminder(rig: &Rig, team: &TeamId, task: &TaskId, run: &RunId, searches: u64) {
    let db = HistoryDb::open(&rig.home.history_file(), MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: std::sync::Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    started.history.record(Record::Run(Box::new(RunStart {
        run: run.clone(),
        team: team.clone(),
        task: task.clone(),
        at: Timestamp::from_utc(UtcDateTime::from_unix_timestamp(1_763_978_400).unwrap()).unwrap(),
        agent: AgentIdentity {
            name: "claude-1".to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: None,
        },
        role: Role::Solo,
        rounds: Vec::new(),
        why: RunWhy::FirstRun,
        prompt: "Do the item.".to_owned(),
        worktree: AbsolutePath::from_str(rig.repo.to_str().unwrap()).unwrap(),
        branch: BranchName::from_str("main").unwrap(),
        session: None,
    })));
    started
        .history
                                    .serena_reminded(run, task, CliKind::Claude, searches, 3, &SharedClock::new(rig.clock_of().clone()));
    started.history.flush();
    started.writer.close().unwrap();
}

/// The run of `task` the state last started: the one whose end the daemon puts a reminder out for.
fn last_run(rig: &Rig, task: &TaskId) -> RunId {
    let state = agent_core::state::State::from_json(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
    state
        .find_task(task)
        .and_then(|found| found.member.runs().last().map(|last| last.id().clone()))
        .unwrap()
}

/// Waits until the daemon has looked at its teams after the run it is told of: its looks that run a turn of each team come
/// after the runs they reach, and a look of the team is what delivers the reminders of the runs that ended. The answer to a
/// status request comes between two turns of the daemon's loop, and the look is at the start of a turn.
async fn beyond_this_run(rig: &Rig) {
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                Client::Cli,
                Command::Status(StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    }
}

/// Waits until the state says that the last run of `task` has ended.
async fn run_ended(rig: &Rig, task: &TaskId) {
    rig.state_where("the end of the last run", |state| {
        state
            .find_task(task)
            .and_then(|found| found.member.runs().last().map(|last| matches!(last.state(), agent_core::state::RunState::Ended { .. })))
            .unwrap_or(false)
    })
    .await;
}

/// A rig with a solo agent that does its item and two reviewers who say nothing; it has a bus.
async fn rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "hold"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        true,
    )
    .await
}

/// Waits for the daemon to look at its teams after `task`'s run drawn at `before` entered the daemon's state: the answer to
/// a status request comes between two turns of its loop, and a look is at the start of a turn.
async fn a_look_after(rig: &Rig, before: std::time::SystemTime) {
    let deadline = std::time::Instant::now() + super::rig::PATIENCE;
    loop {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
        let changed = std::fs::metadata(rig.home.state_file())
            .and_then(|meta| meta.modified())
            .is_ok_and(|later| later > before);
        if changed {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the daemon did not look at its teams");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_reminder_of_a_run_that_ended_is_a_chat_of_the_harness_on_the_bus() {
    let mut rig = rig().await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let task = tasks[0].clone();
    let expected = reminder_text(SEARCHES).await;

    let run = last_run(&rig, &task);
    let before = std::fs::metadata(rig.home.state_file())
        .and_then(|meta| meta.modified())
        .unwrap();
    record_reminder(&rig, &team, &task, &run, SEARCHES);
    a_look_after(&rig, before).await;

    // Signal the run to end so the daemon can record it as ended
    let run_dir = rig.home.run_dir(&task, &run);
    std::fs::write(run_dir.join(".fake-continue"), "").unwrap();

    rig.wait_for("the reminder of Serena on the bus", || {
        chats_from_the_harness(&rig).contains(&expected)
    })
    .await;
    rig.stop().await.unwrap();

    assert_eq!(chats_from_the_harness(&rig), [expected]);
}
