//! After the last step of the ladder — a task handed over whose new holder goes bad too, or a lead given a member's work
//! that goes bad too — the human is asked once what becomes of the task, and each answer does what it says.

use std::num::NonZeroU32;

use agent_core::ids::TaskId;
use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
use agent_core::questions::Verdict;
use agent_core::state::{ItemPhase, MergeMode, Renewals};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Six runs of claude-1 without a status: both its sessions.
const BAD_IN_BOTH_SESSIONS: &str = "nostatus";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// The questions file number `number` of `task`, if it was written.
fn questions(rig: &Rig, task: &TaskId, number: u32) -> Option<String> {
    std::fs::read_to_string(rig.home.questions_file(task, NonZeroU32::new(number).unwrap())).ok()
}

/// Answers the first question of questions `number` of `task` with `text`, as `agent answer` does.
async fn answer(rig: &Rig, task: &TaskId, number: u32, text: &str) {
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text(text).unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::new(number).unwrap(),
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
}

/// A solo task whose claude-1 goes bad in both its sessions and is taken over by claude-2, which runs `scenario`
/// (its first run is the task's seventh), started and waiting for the human's answer after claude-2 went bad too.
async fn replacement_gone_bad(scenario: &'static str) -> (Rig, TaskId) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-2", scenario),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.status_is(&task, TaskStatus::Questions).await;
    (rig, task)
}

// Rule: a task handed over whose new holder goes bad too, after its own retries, asks the human once, in one questions
// file of the harness's own text.
#[tokio::test]
async fn a_replacement_that_fails_too_asks_the_human_once() {
    let (mut rig, task) = replacement_gone_bad("nostatus").await;

    let state = rig.state();
    let text = questions(&rig, &task, 1).expect("the questions file the harness wrote");
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.runs().len(), 9, "six runs of claude-1, three of claude-2");
    assert_eq!(
        member.open_question().map(agent_core::state::Question::number),
        NonZeroU32::new(1)
    );
    assert_eq!(member.asked_after_last_step(), NonZeroU32::new(1));
    assert!(text.starts_with(&format!("# Questions — {task}")), "{text}");
    assert!(text.contains("a) start it once more"), "{text}");
    assert!(text.contains("b) stop the task"), "{text}");
    assert!(
        text.contains("claude-1"),
        "it names the agent that held the task before: {text}"
    );
    assert!(questions(&rig, &task, 2).is_none(), "one questions file");
}

// Rule: the answer `a` starts the task once more, in a new session of the agent that holds it, briefed; it ends DONE.
#[tokio::test]
async fn answering_a_starts_the_task_once_more_in_a_new_session_of_its_agent() {
    let (mut rig, task) = replacement_gone_bad("done,done,done,done,done,done,nostatus,nostatus,nostatus,work").await;

    answer(&rig, &task, 1, "a").await;
    let state = rig
        .item_where("1.1", "merged after the answer", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let prompt = rig.prompt(&task, 10).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.runs().len(), 10);
    assert_eq!(
        member.renewals().map(Renewals::used),
        Some(1),
        "claude-1's one new session, no other is counted"
    );
    assert!(
        prompt.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{prompt}"
    );
    assert!(prompt.contains("You are agent **claude-2**"), "{prompt}");
    let sessions: Vec<_> = member.runs().iter().map(|run| run.session().cloned()).collect();
    assert_ne!(
        sessions[9], sessions[8],
        "the run after the answer resumes no session: {sessions:?}"
    );
}

// Rule: the answer `b`, and accepting the recommendation, stop the task: nothing more runs on it.
#[tokio::test]
async fn answering_b_or_accepting_the_recommendation_stops_the_task() {
    for verdict in ["b", "ok"] {
        let (mut rig, task) = replacement_gone_bad("nostatus").await;

        answer(&rig, &task, 1, verdict).await;
        let state = rig.status_is(&task, TaskStatus::Stopped).await;
        rig.stop().await.unwrap();

        assert_eq!(
            state.find_task(&task).unwrap().member.runs().len(),
            9,
            "{verdict}: no run after the answer"
        );
    }
}

// Rule: the human is asked once: a task started once more by the answer that goes bad again fails as before.
#[tokio::test]
async fn a_task_that_goes_bad_again_after_its_question_fails_as_before() {
    let (mut rig, task) = replacement_gone_bad("nostatus").await;

    answer(&rig, &task, 1, "a").await;
    let state = rig.status_is(&task, TaskStatus::Failed).await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(
        member.runs().len(),
        12,
        "the three runs of the session the answer began"
    );
    assert_eq!(member.questions().len(), 1, "no second question");
    assert!(questions(&rig, &task, 2).is_none());
}

// Rule: a lead given a member's work that goes bad too, after the steps of its own ladder, asks the human once.
#[tokio::test]
async fn a_lead_given_a_members_work_that_fails_too_asks_the_human() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,nostatus"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    // The only agent listed is the lead itself: nobody fits, and the member's work goes to the lead.
    configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;

    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let text = questions(&rig, &tasks[0], 1).expect("the questions file the harness wrote");
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&tasks[1]).unwrap().member.status(), TaskStatus::Stopped);
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(lead.asked_after_last_step(), NonZeroU32::new(1));
    assert!(text.contains("inv-lead"), "{text}");
}

// Rule: a restart of the daemon after a takeover keeps who holds the task, who held it and the counts: the task goes on
// with the agent that took it over, in its own session, and ends DONE.
#[tokio::test]
async fn a_restart_after_a_takeover_goes_on_with_the_agent_that_took_it_over() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            // The task's seventh run is claude-2's first: it works until the daemon stops, then the next run works.
            normal("claude-2", "work,work,work,work,work,work,hold,work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.wait_for("claude-2's first run", || rig.runs(&task) >= 7).await;
    rig.status_is(&task, TaskStatus::Running).await;

    rig.stop().await.unwrap();
    std::fs::write(rig.worktree(&task).join(".fake-continue"), "").unwrap();
    rig.start().await;
    let state = rig
        .item_where("1.1", "merged after the restart", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let args = std::fs::read_to_string(rig.worktree(&task).join(".fake-args-8")).unwrap();
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert_eq!(
        member.renewals().map(Renewals::used),
        Some(1),
        "the counts survive the restart"
    );
    assert_eq!(member.runs().len(), 8);
    assert!(
        args.lines().any(|line| line == "claude-2"),
        "the run after the restart is claude-2's: {args}"
    );
    let sessions: Vec<_> = member.runs().iter().map(|run| run.session().cloned()).collect();
    assert_eq!(
        sessions[7], sessions[6],
        "it resumes claude-2's own session: {sessions:?}"
    );
}

/// The rows of `task_handed_over` the daemon logged in the history of `rig`, after a takeover in a rig started with an
/// `info` log layer.
fn handed_over_rows(rig: &Rig) -> Vec<agent_core::log::LogEntry> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == agent_core::log::LogEvent::TaskHandedOver)
        .collect()
}

/// A daemon of `rig` that logs `info` and up into its history.
async fn start_logging(rig: &mut Rig) {
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
}

/// Records the `task_handed_over` of a real takeover — claude-1 to claude-2 — into `observed`, after checking its row.
pub(super) async fn record_task_handed_over(observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-2\"]\n");
    start_logging(&mut rig).await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "merged after the takeover", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.wait_for("the hand-over in the history", || !handed_over_rows(&rig).is_empty())
        .await;
    rig.stop().await.unwrap();

    let rows = handed_over_rows(&rig);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.level(), agent_core::log::LogLevel::Info);
    assert_eq!(row.task(), Some(&tasks[0]));
    assert_eq!(row.fields().get("from"), Some(&serde_json::json!("claude-1")));
    assert_eq!(row.fields().get("to"), Some(&serde_json::json!("claude-2")));
    observed.insert(agent_core::log::LogEvent::TaskHandedOver);
}

// Rule: a takeover is logged as `task_handed_over`, with the agent it was taken from and the agent that holds it now.
#[tokio::test]
async fn a_takeover_is_logged_with_the_agent_before_and_the_agent_after() {
    let mut observed = std::collections::BTreeSet::new();

    record_task_handed_over(&mut observed).await;

    assert!(observed.contains(&agent_core::log::LogEvent::TaskHandedOver));
}

// Rule: a member's work given to its lead is logged as `task_handed_over` to `lead`.
#[tokio::test]
async fn a_members_work_given_to_its_lead_is_logged_as_handed_over_to_the_lead() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
    start_logging(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    rig.wait_for("the hand-over in the history", || !handed_over_rows(&rig).is_empty())
        .await;
    rig.stop().await.unwrap();

    let rows = handed_over_rows(&rig);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].task(), Some(&tasks[1]));
    assert_eq!(rows[0].fields().get("from"), Some(&serde_json::json!("claude-1")));
    assert_eq!(rows[0].fields().get("to"), Some(&serde_json::json!("lead")));
}

/// The texts the bus of `rig` keeps, every file of its directory read as text.
fn bus_texts(rig: &Rig) -> String {
    let mut texts = String::new();
    let mut dirs = vec![rig.home.root().join("bus")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                texts.push_str(&text);
            }
        }
    }
    texts
}

/// Stops `task` as the human does with `agent stop <task>`.
async fn stop_task(rig: &Rig, task: &TaskId) {
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(agent_core::protocol::ControlRequest {
                target: agent_core::protocol::Target::Task(task.clone()),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "unexpected reply: {reply:?}");
}

/// The heading of the message a lead is given a member's work with.
const LEAD_TEXT: &str = "WORK OF A MEMBER THAT IS GONE";

/// The heading of the briefing that follows that message.
const LEAD_BRIEFING: &str = "## What the member's runs left";

/// A team of `inv-lead` (running `lead`) and the member claude-1, which goes bad in both its sessions and is taken over
/// by claude-3, which goes bad too; started and waiting for the human's answer about the member's task.
async fn member_replacement_gone_bad(lead: &'static str) -> (Rig, Vec<TaskId>) {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", BAD_IN_BOTH_SESSIONS),
            normal("claude-3", "nostatus"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nagents = [\"claude-3\"]\n");
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[1], TaskStatus::Questions).await;
    (rig, tasks)
}

// Rule: a taken-over member whose new holder goes bad too is asked about; the answer `b` stops it, and its lead is told
// on the bus that the work is its own, with the briefing of what the member's runs left.
#[tokio::test]
async fn a_taken_over_members_answer_b_stops_it_and_its_lead_gets_the_work_with_the_briefing() {
    let (mut rig, tasks) = member_replacement_gone_bad("sleep").await;
    assert!(questions(&rig, &tasks[1], 1).is_some(), "the member's question");

    answer(&rig, &tasks[1], 1, "b").await;
    let state = rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    rig.wait_for("the lead's text on the bus", || {
        let bus = bus_texts(&rig);
        bus.contains(LEAD_TEXT) && bus.contains(LEAD_BRIEFING)
    })
    .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.agent().name(), "claude-3");
    assert_eq!(member.runs().len(), 9);
}

// Rule: a coordinator given a member's work that goes bad too, and answered `a`, starts once more in a new session; one
// answered `b` stops.
#[tokio::test]
async fn a_coordinator_answered_a_starts_once_more_and_one_answered_b_stops() {
    for (verdict, lead) in [
        ("a", "sleep,nostatus,nostatus,nostatus,nostatus,nostatus,nostatus,sleep"),
        ("b", "sleep,nostatus"),
    ] {
        let mut rig = Rig::new(
            &[
                investigator("inv-lead", lead),
                normal("claude-1", BAD_IN_BOTH_SESSIONS),
                investigator("inv-r1", APPROVE),
                investigator("inv-r2", APPROVE),
            ],
            5,
            true,
        )
        .await;
        configure(&rig, "[replacement]\nagents = [\"inv-lead\"]\n");
        rig.start().await;
        let (_, tasks) = rig
            .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
            .await;
        rig.status_is(&tasks[0], TaskStatus::Questions).await;

        answer(&rig, &tasks[0], 1, verdict).await;
        let state = if verdict == "a" {
            let lead_task = tasks[0].clone();
            rig.state_where("the lead's run after the answer", |state| {
                let lead = state.find_task(&lead_task).unwrap().member;
                lead.runs().len() == 8 && lead.current_run().is_none()
            })
            .await
        } else {
            rig.status_is(&tasks[0], TaskStatus::Stopped).await
        };
        rig.stop().await.unwrap();

        let lead = state.find_task(&tasks[0]).unwrap().member;
        if verdict == "a" {
            let sessions: Vec<_> = lead.runs().iter().map(|run| run.session().cloned()).collect();
            assert_ne!(sessions[7], sessions[6], "a new session of the lead: {sessions:?}");
            assert_ne!(lead.status(), TaskStatus::Stopped);
        } else {
            assert_eq!(lead.runs().len(), 7, "nothing more runs");
        }
    }
}

// Rule: in mode `off` a coordinator holding the work of a member the human stopped fails as before, unasked.
#[tokio::test]
async fn in_mode_off_a_coordinator_holding_a_stopped_members_work_fails_unasked() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,nostatus"),
            normal("claude-1", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[replacement]\nmode = \"off\"\n");
    rig.wake_sleeping_teams();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[1], TaskStatus::Running).await;

    stop_task(&rig, &tasks[1]).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Failed).await;
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&tasks[0]).unwrap().member.questions().len(), 0);
    assert!(questions(&rig, &tasks[0], 1).is_none(), "no questions file");
}

// Rule: a restart of the daemon while the question after the last step is open keeps it open, and its answer still does
// what it says.
#[tokio::test]
async fn a_restart_while_the_question_is_open_keeps_it_and_its_answer_still_applies() {
    let (mut rig, task) = replacement_gone_bad("done,done,done,done,done,done,nostatus,nostatus,nostatus,work").await;

    rig.stop().await.unwrap();
    rig.start().await;
    rig.status_is(&task, TaskStatus::Questions).await;
    answer(&rig, &task, 1, "a").await;
    let state = rig
        .item_where("1.1", "merged after the answer", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    let member = state.find_task(&task).unwrap().member;
    assert_eq!(member.runs().len(), 10);
    assert_eq!(member.asked_after_last_step(), NonZeroU32::new(1));
}

/// Points the `[binaries] agentbus` of `rig` at `program`, before the daemon starts; the agents keep their own.
fn use_agentbus(rig: &Rig, program: &std::path::Path) {
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    // The `[binaries]` table is the only one with a line `agentbus = "…"`.
    let changed: Vec<String> = config
        .lines()
        .map(|line| {
            if line.starts_with("agentbus = ") {
                format!("agentbus = \"{}\"", program.display())
            } else {
                line.to_owned()
            }
        })
        .collect();
    assert_ne!(changed.join("\n"), config.trim_end(), "the config names an agentbus");
    std::fs::write(rig.home.config_file(), changed.join("\n") + "\n").unwrap();
}

// Rule: a member's work owed to its lead survives a restart until the lead is told: the bus fails when the member is
// answered `b`, the daemon restarts with the bus back, and the lead is told then, with the briefing.
#[tokio::test]
async fn a_members_work_owed_to_its_lead_is_told_after_a_restart_when_the_bus_failed_before() {
    let (mut rig, tasks) = member_replacement_gone_bad("sleep").await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    rig.stop().await.unwrap();
    let failing = rig.home.root().join("failing-agentbus");
    std::fs::write(&failing, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&failing, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    use_agentbus(&rig, &failing);
    rig.start().await;

    answer(&rig, &tasks[1], 1, "b").await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    rig.stop().await.unwrap();
    assert!(
        !bus_texts(&rig).contains(LEAD_TEXT),
        "the bus could not take the message"
    );
    assert!(
        rig.state().find_task(&tasks[1]).unwrap().member.owes_lead(),
        "the work stays owed to the lead in the state"
    );
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start().await;
    rig.wait_for("the lead's text on the bus after the restart", || {
        let bus = bus_texts(&rig);
        bus.contains(LEAD_TEXT) && bus.contains(LEAD_BRIEFING)
    })
    .await;
    let member_task = tasks[1].clone();
    let state = rig
        .state_where("nothing owed any more", |state| {
            !state.find_task(&member_task).unwrap().member.owes_lead()
        })
        .await;
    rig.stop().await.unwrap();

    assert_eq!(state.find_task(&tasks[1]).unwrap().member.status(), TaskStatus::Stopped);
}
