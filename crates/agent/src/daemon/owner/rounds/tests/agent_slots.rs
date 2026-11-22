//! A queued task whose agent runs another task at its limit waits for that run, and the daemon logs the wait once as
//! `task_waits_for_agent`, naming the run it waits for.

use agent_core::ids::TaskId;
use agent_core::log::LogEvent;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The rows of `task_waits_for_agent` in the history of `rig`.
fn wait_rows(rig: &Rig) -> Vec<agent_core::log::LogEntry> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == LogEvent::TaskWaitsForAgent)
        .collect()
}

/// Records the `task_waits_for_agent` of a real wait — the second task of claude-1 behind the first — into `observed`,
/// after checking its row names the first task.
pub(super) async fn record_task_waits_for_agent(observed: &mut std::collections::BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
    let (_, first) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&first[0], TaskStatus::Running).await;
    let (_, second) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    // The loop of the daemon looks at the queue after every message: the wait is logged before the stop is served. The
    // history is read after the stop, which writes what the layer still holds.
    rig.status_is(&second[0], TaskStatus::Queued).await;
    rig.wait_for("the wait in the history", || !wait_rows(&rig).is_empty())
        .await;
    rig.stop().await.unwrap();

    let rows = wait_rows(&rig);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.level(), agent_core::log::LogLevel::Info);
    let second: &TaskId = &second[0];
    assert_eq!(row.task(), Some(second));
    assert_eq!(
        row.fields().get("holder_task"),
        Some(&serde_json::json!(first[0].as_str()))
    );
    observed.insert(LogEvent::TaskWaitsForAgent);
}

// Rule: a task that waits for its agent is logged as `task_waits_for_agent`, with the run it waits for.
#[tokio::test]
async fn a_wait_for_the_agent_is_logged_with_the_run_it_waits_for() {
    let mut observed = std::collections::BTreeSet::new();

    record_task_waits_for_agent(&mut observed).await;

    assert!(observed.contains(&LogEvent::TaskWaitsForAgent));
}

/// All text files in the `bus/` directory of `rig`.
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

#[tokio::test]
async fn a_busy_member_in_a_team_is_handed_to_the_least_loaded_cli_tells_the_bus_and_delivers_the_round() {
    use agent_core::state::ItemPhase;

    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate"),
            normal("claude-1", "hold"),
            normal("claude-2", "work"),
            normal("codex-1", "hold"),
            normal("codex-2", "hold"),
            normal("codex-3", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    // Give codex-1, codex-2 and codex-3 the `codex` CLI binary so `codex` carries two runs and `claude` carries one.
    let config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let team_cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team-cli.sh");
    let updated = config_text
        .replace(
            "[agents.codex-1]\ncli = \"claude\"",
            "[agents.codex-1]\ncli = \"codex\"",
        )
        .replace(
            "[agents.codex-2]\ncli = \"claude\"",
            "[agents.codex-2]\ncli = \"codex\"",
        )
        .replace(
            "[agents.codex-3]\ncli = \"claude\"",
            "[agents.codex-3]\ncli = \"codex\"",
        )
        .replace(
            "[binaries]\n",
            &format!("[binaries]\ncodex = \"{}\"\n", team_cli.display()),
        )
        + "\n[replacement]\nagents = [\"codex-3\", \"claude-2\"]\nbusy_after = 3600\n";
    std::fs::write(rig.home.config_file(), updated).unwrap();

    rig.start().await;
    let (_, busy_a) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (_, busy_c) = rig.run(&["codex-1"], ONE, MergeMode::NoMerge, None).await;
    let (_, busy_d) = rig.run(&["codex-2"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy_a[0], TaskStatus::Running).await;
    rig.status_is(&busy_c[0], TaskStatus::Running).await;
    rig.status_is(&busy_d[0], TaskStatus::Running).await;

    let (team_b, team_b_tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let lead_task = team_b_tasks[0].clone();
    let member_task = team_b_tasks[1].clone();
    rig.state_where("lead sleeps and member task is queued waiting for claude-1", |state| {
        let lead = state.find_task(&lead_task).unwrap().member;
        let member = state.find_task(&member_task).unwrap().member;
        lead.status() == TaskStatus::Waiting
            && member.status() == TaskStatus::Queued
            && member.waiting_since().is_some()
    })
    .await;

    // Advance the injected clock past busy_after without real sleep.
    rig.advance_history_days(1);

    let item_id = "1.1".parse().unwrap();
    let state = rig
        .state_where(
            "item 1.1 of team B delivered after busy hand-over to claude-2",
            |state| {
                state
                    .team(&team_b)
                    .and_then(|team| team.item(item_id))
                    .is_some_and(|progress| matches!(progress.phase(), ItemPhase::Done { .. }))
            },
        )
        .await;
    let prompt = rig.kept_prompt(&member_task, 1);
    let bus = bus_texts(&rig);

    // Release claude-1 in team A and verify it is not woken for the task it lost in team B.
    std::fs::write(rig.worktree(&busy_a[0]).join(".fake-continue"), "").unwrap();
    rig.status_is(&busy_a[0], TaskStatus::Waiting).await;
    let after_free = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&member_task).unwrap().member;
    assert_eq!(
        member.agent().name(),
        "claude-2",
        "least-loaded CLI claude (1 run) is chosen over first-listed codex-3 on codex (2 runs)"
    );
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert!(
        prompt.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{prompt}"
    );
    assert!(prompt.contains("`busy`"), "{prompt}");
    assert!(
        bus.contains("is held by claude-2 now: claude-1 is busy in another team."),
        "the bus tells the team who holds the item now: {bus}"
    );
    assert_eq!(
        after_free.find_task(&member_task).unwrap().member.runs().len(),
        member.runs().len(),
        "the first holder is not woken for the task when it becomes free"
    );
}

#[tokio::test]
async fn a_restart_during_the_busy_hand_over_keeps_the_new_holder_and_briefing() {
    use agent_core::decide::ladder::Trouble;
    use agent_core::state::{ItemPhase, Participant, Timestamp};

    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 300\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    rig.start().await;
    let (_, busy_a) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy_a[0], TaskStatus::Running).await;
    let (team_b, tasks_b) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let waiting_task = tasks_b[0].clone();
    rig.status_is(&waiting_task, TaskStatus::Queued).await;
    rig.stop().await.unwrap();

    // Persist a busy hand-over that has just begun before its first run starts, then restart the daemon.
    let mut state = rig.state();
    let at: Timestamp = "2026-11-22T18:00:00Z".parse().unwrap();
    let claude_2: agent_core::ids::AgentName = "claude-2".parse().unwrap();
    state
        .hand_over_task(&waiting_task, Participant::Normal(claude_2.clone()), at)
        .unwrap();
    let branch = match state
        .team(&team_b)
        .unwrap()
        .item("1.1".parse().unwrap())
        .unwrap()
        .phase()
    {
        ItemPhase::InProgress { branch, .. } => branch.clone(),
        other => panic!("unexpected phase: {other:?}"),
    };
    state
        .team_mut(&team_b)
        .unwrap()
        .item_mut("1.1".parse().unwrap())
        .unwrap()
        .reassign(claude_2, branch)
        .unwrap();
    state.begin_holder_session(&waiting_task, Trouble::Busy, at).unwrap();
    std::fs::write(rig.home.state_file(), state.to_json().unwrap()).unwrap();

    rig.start().await;
    let item_id = "1.1".parse().unwrap();
    let finished = rig
        .state_where(
            "item 1.1 of team B delivered by claude-2 after restart during hand-over",
            |state| {
                state
                    .team(&team_b)
                    .and_then(|team| team.item(item_id))
                    .is_some_and(|progress| matches!(progress.phase(), ItemPhase::Done { .. }))
            },
        )
        .await;
    let prompt = rig.kept_prompt(&waiting_task, 1);
    rig.stop().await.unwrap();

    let member = finished.find_task(&waiting_task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert!(prompt.contains("`busy`"), "{prompt}");
}

/// A waiting member and an item of another member in review, with a free investigator as candidate.
fn state_with_another_item_in_review() -> (agent_core::state::State, agent_core::ids::TeamId, TaskId) {
    let mut state = crate::fixtures::team_state();
    let team = crate::fixtures::team_id(0);
    let task = crate::fixtures::task(2);
    {
        let mut found = state.team_mut(&team).unwrap();
        for (round, item, agent, branch) in [
            ("round-20261111-153101-q1w1", "1.1", "claude-1", "agents/claude-1/1.1"),
            (
                "round-20261111-153102-q1w2",
                "1.2",
                "inv-opus",
                "agents/codex-1/item-1-2",
            ),
        ] {
            found
                .open_round(
                    round.parse().unwrap(),
                    Some(item.parse().unwrap()),
                    crate::fixtures::at(1),
                )
                .unwrap();
            found
                .advance_item(
                    item.parse().unwrap(),
                    agent_core::state::ItemPhase::InProgress {
                        assignee: agent.parse().unwrap(),
                        branch: branch.parse().unwrap(),
                    },
                )
                .unwrap();
        }
        found
            .advance_item(
                "1.2".parse().unwrap(),
                agent_core::state::ItemPhase::InReview {
                    pr: crate::fixtures::pr(1),
                    reviews: Vec::new(),
                },
            )
            .unwrap();
    }
    state.start_waiting(&task, crate::fixtures::at(0));
    (state, team, task)
}

#[tokio::test]
async fn a_review_of_another_members_item_does_not_block_a_busy_hand_over() {
    use super::super::fixture::Daemon;

    let (state, _team, task) = state_with_another_item_in_review();
    let daemon = Daemon::new(&state, None);
    daemon.configure("\n[replacement]\nagents = [\"inv-1\"]\nbusy_after = 1\n");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(changed, "another item's review does not stop the hand-over");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "inv-1"
    );
}

#[tokio::test]
async fn a_waiting_members_own_review_is_not_handed_over() {
    use super::super::fixture::Daemon;
    use agent_core::state::{ItemPhase, NewRun, RunEnd};
    use agent_core::status::RunOutcome;

    let (mut state, team, task) = state_with_another_item_in_review();
    {
        let mut found = state.team_mut(&team).unwrap();
        found
            .begin_run(
                &task,
                NewRun {
                    id: crate::fixtures::run(1),
                    at: crate::fixtures::at(2),
                    session: None,
                    rounds: vec!["round-20261111-153101-q1w1".parse().unwrap()],
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(&task).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .end_run(
                crate::fixtures::at(3),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
        member.transition(TaskStatus::Waiting).unwrap();
        let mut pr = crate::fixtures::pr(2);
        pr.branch = "agents/claude-1/1.1".parse().unwrap();
        found
            .advance_item(
                "1.1".parse().unwrap(),
                ItemPhase::InReview {
                    pr,
                    reviews: Vec::new(),
                },
            )
            .unwrap();
    }
    let daemon = Daemon::new(&state, None);
    daemon.configure("\n[replacement]\nagents = [\"inv-1\"]\nbusy_after = 1\n");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "a task waiting for its item's review stays with its holder");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "claude-1"
    );
}

/// A waiting ordinary member in a team with a candidate-free investigator available as a replacement.
fn waiting_team_state() -> (agent_core::state::State, agent_core::ids::TeamId, TaskId) {
    let mut state = crate::fixtures::team_state();
    let task = crate::fixtures::task(2);
    state.start_waiting(&task, crate::fixtures::at(0));
    (state, crate::fixtures::team_id(0), task)
}

/// Enables a replacement candidate by name; the sample investigator inv-codex needs its config entry too.
fn configure_candidate(daemon: &super::super::fixture::Daemon, name: &str) {
    let extra = if name == "inv-codex" {
        "[investigators.inv-codex]\ncli = \"claude\"\n"
    } else {
        ""
    };
    daemon.configure(&format!(
        "\n{extra}[replacement]\nagents = [\"{name}\"]\nbusy_after = 1\n"
    ));
}

/// Gives the sample reviewer task 4 a state that maps to availability free, running or paused.
fn candidate_with_status(state: &mut agent_core::state::State, status: TaskStatus) {
    let task = crate::fixtures::task(4);
    let mut team = state.team_mut(&crate::fixtures::team_id(0)).unwrap();
    let (_, mut member) = team.find_mut(&task).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    if status != TaskStatus::Running {
        member.transition(status).unwrap();
    }
}

/// Puts `claude-2` in a solo team of its own, outside the waiting member's team and with the requested status.
fn candidate_outside_with_status(state: &mut agent_core::state::State, status: TaskStatus) {
    use agent_core::state::{Crew, Member, MergeMode, TaskListText, Team};

    let task = crate::fixtures::task(5);
    let team = crate::fixtures::team_id(1);
    state
        .insert_team(Team::new(
            team.clone(),
            crate::fixtures::repo(),
            crate::fixtures::at(0),
            MergeMode::NoMerge,
            TaskListText::new("Solo task.\n".to_owned()).unwrap(),
            Crew::solo(Member::new(task.clone(), crate::fixtures::normal("claude-2"))).unwrap(),
        ))
        .unwrap();
    let mut solo = state.team_mut(&team).unwrap();
    let (_, mut member) = solo.find_mut(&task).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    if status != TaskStatus::Running {
        member.transition(status).unwrap();
    }
}

#[tokio::test]
async fn a_lower_tier_candidate_is_not_given_the_waiting_task() {
    use super::super::fixture::Daemon;

    let (mut state, _team, task) = waiting_team_state();
    state
        .hand_over_task(&task, crate::fixtures::investigator("inv-1"), crate::fixtures::at(0))
        .unwrap();
    state.start_waiting(&task, crate::fixtures::at(0));
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "claude-2");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "a normal agent cannot take an investigator's task");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "inv-1"
    );
}

#[tokio::test]
async fn an_agent_of_the_tasks_team_is_not_given_the_waiting_task() {
    use super::super::fixture::Daemon;

    let (mut state, _team, task) = waiting_team_state();
    candidate_with_status(&mut state, TaskStatus::Waiting);
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "inv-codex");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "a free agent of the same team still cannot take the task");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "claude-1"
    );
}

#[tokio::test]
async fn a_running_candidate_is_not_given_the_waiting_task() {
    use super::super::fixture::Daemon;

    let (mut state, _team, task) = waiting_team_state();
    candidate_outside_with_status(&mut state, TaskStatus::Running);
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "claude-2");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "a running candidate stays untouched");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "claude-1"
    );
}

#[tokio::test]
async fn a_paused_candidate_is_not_given_the_waiting_task() {
    use super::super::fixture::Daemon;

    let (mut state, _team, task) = waiting_team_state();
    candidate_outside_with_status(&mut state, TaskStatus::Paused);
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "claude-2");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "a candidate paused for its quota stays untouched");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "claude-1"
    );
}

#[tokio::test]
async fn a_task_that_waits_for_an_answer_is_not_handed_over() {
    use super::super::fixture::Daemon;

    let (mut state, team, task) = waiting_team_state();
    {
        let mut found = state.team_mut(&team).unwrap();
        let (_, mut member) = found.find_mut(&task).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Questions).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "inv-1");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task).await.unwrap();

    assert!(!changed, "the human's answer does not change the holder");
    assert_eq!(
        owner.parts.state.find_task(&task).unwrap().member.agent().name(),
        "claude-1"
    );
}

#[tokio::test]
async fn an_active_coordinator_is_not_handed_over() {
    use super::super::fixture::Daemon;

    let (mut state, team, _member) = waiting_team_state();
    {
        let task = crate::fixtures::task(2);
        let mut found = state.team_mut(&team).unwrap();
        let (_, mut member) = found.find_mut(&task).unwrap();
        member.transition(TaskStatus::Running).unwrap();
    }
    let lead = crate::fixtures::task(1);
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "inv-1");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&lead).await.unwrap();

    assert!(!changed, "a lead stays while one of its members runs");
    assert_eq!(
        owner.parts.state.find_task(&lead).unwrap().member.agent().name(),
        "inv-opus"
    );
}

#[tokio::test]
async fn a_reviewer_is_not_handed_over_for_a_busy_wait() {
    use super::super::fixture::Daemon;

    let (state, _team, _member) = waiting_team_state();
    let reviewer = crate::fixtures::task(4);
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "inv-1");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&reviewer).await.unwrap();

    assert!(!changed, "reviewer replacement does not use the busy hand-over path");
    assert_eq!(
        owner.parts.state.find_task(&reviewer).unwrap().member.agent().name(),
        "inv-codex"
    );
}

/// Reads the messages an agent of a team sees, without moving its bus read cursor.
fn history_of(home: &agent_runtime::store::Home, agent: &str, team: &agent_core::ids::TeamId, task: &TaskId) -> String {
    use std::io::Write as _;

    let mut child = std::process::Command::new(super::super::fixture::agentbus_binary())
        .arg("serve")
        .arg("--agent")
        .arg(agent)
        .arg("--task")
        .arg(task.as_str())
        .arg("--team")
        .arg(team.as_str())
        .arg("--dir")
        .arg(home.root().join("bus"))
        .arg("--home")
        .arg(home.root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"read_messages\",\"arguments\":{{\"history\":50}}}}}}"
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

/// A saved hand-over with its team notice still owed, its item on the former holder's original branch.
fn pre_send_busy_notice() -> (agent_core::state::State, agent_core::ids::TeamId, TaskId) {
    use agent_core::state::{ItemPhase, Participant};

    let mut state = crate::fixtures::team_state();
    let team = crate::fixtures::team_id(0);
    let task = crate::fixtures::task(2);
    let at = crate::fixtures::at(1);
    {
        let mut found = state.team_mut(&team).unwrap();
        found
            .open_round(
                "round-20261111-153101-q1w1".parse().unwrap(),
                Some("1.1".parse().unwrap()),
                at,
            )
            .unwrap();
        found
            .advance_item(
                "1.1".parse().unwrap(),
                ItemPhase::InProgress {
                    assignee: "claude-1".parse().unwrap(),
                    branch: "agents/claude-1/1.1".parse().unwrap(),
                },
            )
            .unwrap();
    }
    let holder: agent_core::ids::AgentName = "inv-1".parse().unwrap();
    state
        .hand_over_task(&task, Participant::Investigator("inv-1".parse().unwrap()), at)
        .unwrap();
    state
        .team_mut(&team)
        .unwrap()
        .item_mut("1.1".parse().unwrap())
        .unwrap()
        .reassign(holder, "agents/claude-1/1.1".parse().unwrap())
        .unwrap();
    assert!(
        state
            .begin_holder_session(&task, agent_core::decide::ladder::Trouble::Busy, at)
            .unwrap()
    );
    state.owe_busy_notice(&task).unwrap();
    (state, team, task)
}

#[tokio::test]
async fn an_owed_busy_notice_is_retried_for_the_whole_team_after_a_restart() {
    use super::super::fixture::Daemon;
    let (state, team, task) = pre_send_busy_notice();

    let flaky = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    let daemon = Daemon::new(&state, Some(&flaky));
    std::fs::write(
        daemon.home.root().join("real-agentbus"),
        super::super::fixture::agentbus_binary().to_str().unwrap(),
    )
    .unwrap();
    let down = daemon.home.root().join("flaky-bus-down");
    std::fs::write(&down, "").unwrap();

    let (mut first, _sinks) = daemon.bare_owner().await;
    first.look_at_every_team();
    first.team_life().await.unwrap();

    let refused = daemon.home.root().join("flaky-bus-refused");
    let calls = std::fs::read_to_string(&refused).unwrap();
    assert!(
        calls.contains("send"),
        "the recovery did not try the owed send: {calls}"
    );
    assert!(first.parts.state.find_task(&task).unwrap().member.busy_notice_owed());
    drop(first);

    std::fs::remove_file(&down).unwrap();
    let (mut recovered, _sinks) = daemon.bare_owner().await;
    recovered.look_at_every_team();
    recovered.team_life().await.unwrap();

    assert!(
        !recovered
            .parts
            .state
            .find_task(&task)
            .unwrap()
            .member
            .busy_notice_owed()
    );
    assert_eq!(
        recovered
            .parts
            .state
            .find_task(&task)
            .unwrap()
            .member
            .past_holders()
            .len(),
        1,
        "recovery delivers the notice without another hand-over"
    );
    let messages = std::fs::read_to_string(daemon.home.root().join("bus/messages.jsonl")).unwrap();
    let notice = messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|message| {
            message["text"]
                .as_str()
                .is_some_and(|text| text.contains("is held by inv-1 now: claude-1 is busy in another team"))
        });
    let Some(notice) = notice else {
        panic!("the team notice is missing: {messages}");
    };
    assert_eq!(notice["to"].as_str(), Some(format!("team:{team}").as_str()));
    assert!(
        history_of(&daemon.home, "inv-opus", &team, &crate::fixtures::task(1))
            .contains("is held by inv-1 now: claude-1 is busy in another team"),
        "the coordinator did not receive the holder-change notice"
    );
}

#[tokio::test]
async fn busy_notice_rendering_failures_are_typed_and_retained() {
    use super::super::fixture::Daemon;
    use crate::daemon::owner::takeover::BusyNoticeError;

    let (state, _team, task) = waiting_team_state();
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    let missing_branch = owner.busy_handover_text(&task, "1.1".parse().unwrap()).unwrap_err();
    assert!(matches!(
        missing_branch,
        BusyNoticeError::MissingBranch { item } if item == "1.1".parse().unwrap()
    ));

    let (state, _team, task) = state_with_another_item_in_review();
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    let missing_holder = owner.busy_handover_text(&task, "1.1".parse().unwrap()).unwrap_err();
    assert!(matches!(
        missing_holder,
        BusyNoticeError::MissingPreviousHolder { task: failed } if failed == task
    ));
}

#[tokio::test]
async fn an_owed_notice_with_no_matching_item_is_settled() {
    use super::super::fixture::Daemon;

    let (mut state, _team, task) = waiting_team_state();
    state
        .begin_holder_session(&task, agent_core::decide::ladder::Trouble::Busy, crate::fixtures::at(0))
        .unwrap();
    state.owe_busy_notice(&task).unwrap();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.team_life().await.unwrap();

    assert!(!owner.parts.state.find_task(&task).unwrap().member.busy_notice_owed());
    let saved =
        agent_core::state::State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
    assert!(!saved.find_task(&task).unwrap().member.busy_notice_owed());
}

/// Queues a normal notice whose accepted text proves that the outbox got past a settled busy notice.
async fn tell_later_marker(owner: &mut crate::daemon::owner::Owner, team: &agent_core::ids::TeamId) {
    use crate::daemon::team::{BusJob, BusNotice};
    use agent_runtime::agentbus::{BusTier, MemberState, NewMember, NewTeam};

    owner
        .parts
        .bus
        .team_create(&NewTeam {
            id: team.clone(),
            repo: crate::fixtures::repo(),
            lead: None,
            members: std::collections::BTreeMap::from([(
                "claude-1".parse().unwrap(),
                NewMember {
                    task: crate::fixtures::task(2),
                    branch: "agents/claude-1/1.1".parse().unwrap(),
                    worktree: owner
                        .parts
                        .store
                        .home()
                        .worktree_dir(&crate::fixtures::repo(), &crate::fixtures::task(2))
                        .to_str()
                        .unwrap()
                        .parse()
                        .unwrap(),
                    tier: BusTier::Normal,
                    state: MemberState::Sleeping,
                    provider: Some("claude".to_owned()),
                },
            )]),
        })
        .await
        .unwrap();
    owner.tell_bus(
        team,
        BusJob::Tell {
            to: "claude-1".parse().unwrap(),
            notice: BusNotice::MemberFinished {
                member: "claude-1".parse().unwrap(),
                item: Some("1.1".parse().unwrap()),
                branch: "agents/claude-1/1.1".parse().unwrap(),
            },
        },
    );
}

#[tokio::test]
async fn a_permanent_bus_refusal_settles_the_notice_and_lets_later_jobs_run() {
    use super::super::fixture::Daemon;

    let (state, team, task) = pre_send_busy_notice();
    let flaky = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    let daemon = Daemon::new(&state, Some(&flaky));
    std::fs::write(
        daemon.home.root().join("real-agentbus"),
        super::super::fixture::agentbus_binary().to_str().unwrap(),
    )
    .unwrap();
    std::fs::write(daemon.home.root().join("flaky-bus-refuse-once"), "send").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.queue_busy_handover_notices().await.unwrap();
    tell_later_marker(&mut owner, &team).await;
    owner.look_at_every_team();

    owner.team_life().await.unwrap();

    let messages = std::fs::read_to_string(daemon.home.root().join("bus/messages.jsonl")).unwrap_or_default();
    assert!(messages.contains("claude-1 reported DONE on item 1.1"), "{messages}");
    assert!(!owner.parts.state.find_task(&task).unwrap().member.busy_notice_owed());
    assert!(!daemon.home.root().join("flaky-bus-refuse-once").exists());
}

#[tokio::test]
async fn an_unrenderable_notice_settles_the_obligation_and_lets_later_jobs_run() {
    use super::super::fixture::Daemon;

    let (state, team, task) = pre_send_busy_notice();
    let flaky = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    let daemon = Daemon::new(&state, Some(&flaky));
    std::fs::write(
        daemon.home.root().join("real-agentbus"),
        super::super::fixture::agentbus_binary().to_str().unwrap(),
    )
    .unwrap();
    let down = daemon.home.root().join("flaky-bus-down");
    std::fs::write(&down, "").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.look_at_every_team();
    owner.team_life().await.unwrap();
    assert!(owner.parts.state.find_task(&task).unwrap().member.busy_notice_owed());
    owner
        .parts
        .state
        .team_mut(&team)
        .unwrap()
        .item_mut("1.1".parse().unwrap())
        .unwrap()
        .advance(agent_core::state::ItemPhase::Pending)
        .unwrap();
    std::fs::remove_file(&down).unwrap();
    tell_later_marker(&mut owner, &team).await;
    owner.look_at_every_team();

    owner.team_life().await.unwrap();

    let messages = std::fs::read_to_string(daemon.home.root().join("bus/messages.jsonl")).unwrap_or_default();
    assert!(messages.contains("claude-1 reported DONE on item 1.1"), "{messages}");
    assert!(!owner.parts.state.find_task(&task).unwrap().member.busy_notice_owed());
}

/// A reviewer that never gives a verdict: it ends its turn and waits.
const SILENT: &str = "sleep";

#[tokio::test]
async fn a_member_whose_item_waits_for_a_review_is_not_handed_over_while_a_free_candidate_exists() {
    use agent_core::state::ItemPhase;

    let mut rig = Rig::new(
        &[
            normal("claude-2", "work-or-hold"),
            normal("claude-3", "work"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    let config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config_text + "\n[replacement]\nagents = [\"claude-3\"]\nbusy_after = 3600\n",
    )
    .unwrap();
    rig.start().await;

    // The item of claude-2 is done and waits for its review: the reviewers never give a verdict.
    let (review_team, review_tasks) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;
    let reviewed = review_tasks[0].clone();
    let item_id = "1.1".parse().unwrap();
    rig.state_where("the item of claude-2 to wait for its review", |state| {
        state
            .team(&review_team)
            .and_then(|team| team.item(item_id))
            .is_some_and(|progress| matches!(progress.phase(), ItemPhase::InReview { .. }))
    })
    .await;

    // claude-2 is busy with a run of its own, so the owner of the reviewed item is occupied elsewhere too.
    let holder_list = ONE.replace("The first item", "Holds the agent");
    let (_, holder) = rig.run(&["claude-2"], &holder_list, MergeMode::NoMerge, None).await;
    rig.status_is(&holder[0], TaskStatus::Running).await;
    // The positive control: a second task waits for claude-2, which that run holds, and goes to claude-3.
    let (_, waiting) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    let waiting = waiting[0].clone();
    rig.state_where("the second task to wait for claude-2", |state| {
        state
            .find_task(&waiting)
            .is_some_and(|found| found.member.waiting_since().is_some())
    })
    .await;
    rig.advance_history_days(1);
    let state = rig
        .state_where("the waiting task to be handed over to claude-3", |state| {
            state
                .find_task(&waiting)
                .is_some_and(|found| found.member.agent().name() == "claude-3")
        })
        .await;

    let reviewed_member = state.find_task(&reviewed).unwrap().member;
    assert_eq!(reviewed_member.agent().name(), "claude-2");
    assert!(
        reviewed_member.past_holders().is_empty(),
        "the reviewed item keeps its holder"
    );
    assert!(
        matches!(
            state
                .team(&review_team)
                .and_then(|team| team.item(item_id))
                .map(|progress| progress.phase().clone()),
            Some(ItemPhase::InReview { .. })
        ),
        "the item still waits for its review"
    );

    std::fs::write(rig.worktree(&holder[0]).join(".fake-continue"), "").unwrap();
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_busy_investigator_is_not_given_a_review_while_free_investigators_exist() {
    use agent_core::state::ItemPhase;

    let mut rig = Rig::new(
        &[
            normal("claude-2", "work"),
            investigator("inv-r1", "hold"),
            investigator("inv-r2", SILENT),
            investigator("inv-r3", SILENT),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    // inv-r1 runs its own task, so it is busy when the review of claude-2's item is asked for.
    let (_, holder) = rig.run(&["inv-r1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&holder[0], TaskStatus::Running).await;
    // inv-r2 and inv-r3 finish their own tasks after it: the oldest activity of the three is inv-r1's.
    for free in ["inv-r2", "inv-r3"] {
        let (_, own) = rig.run(&[free], ONE, MergeMode::NoMerge, None).await;
        rig.status_is(&own[0], TaskStatus::Waiting).await;
    }
    let (review_team, _) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;
    let item_id = "1.1".parse().unwrap();
    let state = rig
        .state_where(
            "the item of claude-2 to be asked for two reviews of free investigators",
            |state| {
                state.team(&review_team).is_some_and(|team| {
                    team.reviewers().len() == 2
                        && team
                            .item(item_id)
                            .is_some_and(|progress| matches!(progress.phase(), ItemPhase::InReview { .. }))
                })
            },
        )
        .await;

    let reviewers: Vec<String> = state
        .team(&review_team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| member.agent().name().to_owned())
        .collect();
    assert_eq!(
        reviewers,
        ["inv-r2", "inv-r3"],
        "the busy investigator is not a reviewer"
    );

    std::fs::write(rig.worktree(&holder[0]).join(".fake-continue"), "").unwrap();
    rig.stop().await.unwrap();
}
