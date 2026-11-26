//! Investigations integration tests with the rig, real agentbus, and fake CLI.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use agent_core::kinds::Role;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

#[tokio::test]
async fn a_members_request_is_answered_by_the_free_investigator_and_the_member_is_woken() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:How to solve this problem?,done"),
            investigator("inv-r1", "inv-done:Here is the solution to your issue"),
            investigator("inv-r2", "inv-done:Here is the solution to your issue"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::Autopilot,
            Some("claude-lead"),
        )
        .await;

    // Wait until investigator task is Done
    rig.state_where("investigator task is Done", |state| {
        let Some(team_st) = state.team(&team) else {
            return false;
        };
        team_st
            .reviewers()
            .iter()
            .any(|m| m.role() == Some(Role::Investigator) && m.status() == TaskStatus::Done)
    })
    .await;

    // Wait until member is woken and finishes (run 2)
    rig.wait_for("member ran again after being woken", || rig.runs(&tasks[1]) >= 2)
        .await;

    rig.stop().await.unwrap();

    let state = rig.state();
    let team_st = state.team(&team).unwrap();
    let inv_m = team_st
        .reviewers()
        .iter()
        .find(|m| m.role() == Some(Role::Investigator))
        .expect("investigator task");
    assert_eq!(inv_m.status(), TaskStatus::Done);

    let bus_state_text = std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap();
    let bus_state: serde_json::Value = serde_json::from_str(&bus_state_text).unwrap();
    assert_eq!(bus_state["investigations"]["1"]["status"], "done");

    let inv_file_path = bus_state["investigations"]["1"]["file"].as_str().unwrap();
    let file_content = std::fs::read_to_string(inv_file_path).unwrap();
    assert!(
        file_content.contains("Here is the solution to your issue"),
        "{file_content}"
    );

    let messages = std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap();
    assert!(messages.contains("Here is the solution to your issue"), "{messages}");
}

#[tokio::test]
async fn the_investigators_worktree_is_detached_at_the_requesters_tip() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Where is the bug?"),
            investigator("inv-r1", "hold"),
            investigator("inv-r2", "hold"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::Autopilot,
            Some("claude-lead"),
        )
        .await;

    let inv_task = rig
        .state_where("investigator task is running", |state| {
            let Some(team_st) = state.team(&team) else {
                return false;
            };
            team_st
                .reviewers()
                .iter()
                .any(|m| m.role() == Some(Role::Investigator) && m.status() == TaskStatus::Running)
        })
        .await
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|m| m.role() == Some(Role::Investigator))
        .unwrap()
        .task()
        .clone();

    let requester_tip = super::rig::git(&rig.worktree(&tasks[1]), &["rev-parse", "HEAD"]);
    let inv_tip = super::rig::git(&rig.worktree(&inv_task), &["rev-parse", "HEAD"]);
    assert_eq!(inv_tip, requester_tip);

    let symbolic = Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .current_dir(rig.worktree(&inv_task))
        .output()
        .unwrap();
    assert!(!symbolic.status.success(), "HEAD should be detached");

    std::fs::write(rig.worktree(&inv_task).join(".fake-continue"), "").unwrap();
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn nothing_of_the_investigation_is_pushed() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Check safety,done"),
            investigator("inv-r1", "inv-done:Everything is safe"),
            investigator("inv-r2", "inv-done:Everything is safe"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::Autopilot,
            Some("claude-lead"),
        )
        .await;

    rig.state_where("investigator task is Done", |state| {
        let Some(team_st) = state.team(&team) else {
            return false;
        };
        team_st
            .reviewers()
            .iter()
            .any(|m| m.role() == Some(Role::Investigator) && m.status() == TaskStatus::Done)
    })
    .await;

    rig.stop().await.unwrap();

    let branches = super::rig::git(
        &rig.origin,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
    );
    assert_eq!(branches.trim(), "main");
    assert!(
        rig.github.pulls().is_empty(),
        "no pull request should be opened for investigation"
    );
}

#[tokio::test]
async fn a_request_waits_while_no_investigator_is_free() {
    let mut rig = Rig::new(
        &[
            normal("claude-solo", "work"),
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Help me,sleep"),
            investigator("inv-r1", "inv-review-hold,inv-done:Resolved"),
            investigator("inv-r2", "inv-review-hold,inv-done:Resolved"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;

    // Start claude-solo which opens a PR and triggers review by inv-r1 and inv-r2
    let (review_team, _) = rig.run(&["claude-solo"], ONE, MergeMode::Autopilot, None).await;

    // Wait until both reviewers are running in review_team
    rig.state_where("both reviewers are running", |state| {
        let Some(team_st) = state.team(&review_team) else {
            return false;
        };
        let running_count = team_st
            .reviewers()
            .iter()
            .filter(|m| m.status() == TaskStatus::Running)
            .count();
        running_count == 2
    })
    .await;

    // Now start team with claude-lead and claude-1 which asks for an investigation
    let (team, _) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-lead"),
        )
        .await;

    rig.wait_for("request queued while investigators review", || {
        bus_state(&rig)["investigations"]["1"]["status"] == "queued"
    })
    .await;

    let state = rig.state();
    let team_st = state.team(&team).unwrap();
    assert!(
        !team_st.reviewers().iter().any(|m| m.role() == Some(Role::Investigator)),
        "no investigator task should be added while every investigator is busy"
    );

    let bus_state_text = std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap();
    let bus_state: serde_json::Value = serde_json::from_str(&bus_state_text).unwrap();
    assert_eq!(bus_state["investigations"]["1"]["status"], "queued");

    // Release reviewers in review_team
    let review_team_st = state.team(&review_team).unwrap();
    for reviewer in review_team_st.reviewers() {
        std::fs::write(rig.worktree(reviewer.task()).join(".fake-continue"), "").unwrap();
    }

    // After review finishes, an investigator becomes free and investigation request is served
    rig.state_where("investigation task is created after reviewer is free", |state| {
        let Some(team_st) = state.team(&team) else {
            return false;
        };
        team_st.reviewers().iter().any(|m| m.role() == Some(Role::Investigator))
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_request_of_another_team_is_left_to_that_team() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Own question,done"),
            investigator("inv-r1", "done"),
            investigator("inv-r2", "done"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;

    let bus_dir = rig.home.root().join("bus");
    let bus_bin = super::super::fixture::agentbus_binary();
    let status = Command::new(bus_bin)
        .args([
            "inv-add",
            "--dir",
            bus_dir.to_str().unwrap(),
            "--from-agent",
            "other-agent",
            "--from-task",
            "task-20260101-000000-othr",
            "--repo",
            "example",
            "--branch",
            "main",
            "--tasks-dir",
            rig.home.tasks_dir().to_str().unwrap(),
            "--question",
            "Foreign question",
            "--kind",
            "",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let (_team, _) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-lead"),
        )
        .await;

    rig.wait_for("own request answered beside foreign request", || {
        bus_state(&rig)["investigations"]["2"]["status"] == "done"
    })
    .await;

    let bus_state_text = std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap();
    let bus_state: serde_json::Value = serde_json::from_str(&bus_state_text).unwrap();
    assert_eq!(bus_state["investigations"]["1"]["status"], "queued");

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn the_least_recently_used_of_two_investigators_answers() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:First question,inv-add:Second question,done"),
            investigator("inv-r1", "inv-done:Answer from r1,inv-done:Answer from r1"),
            investigator("inv-r2", "inv-done:Answer from r2,inv-done:Answer from r2"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;

    let (team, tasks) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-lead"),
        )
        .await;

    // First investigation served and done
    let first_investigator = rig
        .state_where("first investigator done", |state| {
            let Some(team_st) = state.team(&team) else {
                return false;
            };
            team_st
                .reviewers()
                .iter()
                .any(|m| m.role() == Some(Role::Investigator) && m.status() == TaskStatus::Done)
        })
        .await
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|m| m.role() == Some(Role::Investigator))
        .unwrap()
        .agent()
        .clone();

    // Member wakes up for run 2, asks second question, and sleeps
    rig.wait_for("member asks second question", || rig.runs(&tasks[1]) >= 2)
        .await;

    // Second investigation assigned: must be the other (least recently used) investigator
    let second_investigator = rig
        .state_where("second investigator assigned", |state| {
            let Some(team_st) = state.team(&team) else {
                return false;
            };
            team_st
                .reviewers()
                .iter()
                .filter(|m| m.role() == Some(Role::Investigator))
                .count()
                >= 2
        })
        .await
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .rfind(|m| m.role() == Some(Role::Investigator))
        .unwrap()
        .agent()
        .clone();

    assert_ne!(
        first_investigator, second_investigator,
        "the other (LRU) investigator must be chosen for the second request"
    );

    rig.stop().await.unwrap();
}

/// Reads the real bus state after a complete atomic save.
fn bus_state(rig: &Rig) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap()).unwrap()
}

/// Provokes the three investigation events through real queued requests.
pub(super) async fn record_investigation_events(observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>) {
    use agent_core::log::{LogEvent, LogLevel};
    use agent_runtime::history::Reader;

    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add:Unknown request,sleep"),
            investigator("inv-r1", "inv-done:Resolved"),
            investigator("inv-r2", "inv-done:Resolved"),
        ],
        1,
        true,
    )
    .await;
    // Model a newer bus response at the process boundary; persisted bus state stays valid.
    let wrapper = rig.home.root().join("future-kind-bus");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
real=REAL_BUS
if [[ "$1" == inv-next ]]; then
    output=$("$real" "$@")
    if [[ "$output" == *'Unknown request'* ]]; then
        printf '{"kind":"future-kind",%s\n' "${output#\{}"
    else
        printf '%s\n' "$output"
    fi
else
    exec "$real" "$@"
fi
"#
    .replace("REAL_BUS", &format!("{:?}", super::super::fixture::agentbus_binary()));
    std::fs::write(&wrapper, script).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let config = config.replace(
        &format!(r#"agentbus = "{}""#, super::super::fixture::agentbus_binary().display()),
        &format!(r#"agentbus = "{}""#, wrapper.display()),
    );
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-lead"),
        )
        .await;
    rig.wait_for("unknown kind failed", || {
        matches!(
            bus_state(&rig)["investigations"]["1"]["status"].as_str(),
            Some("failed" | "done")
        )
    })
    .await;
    assert_eq!(bus_state(&rig)["investigations"]["1"]["status"], "failed");
    assert_eq!(rig.state().team(&team).unwrap().reviewers().len(), 0);
    let branch = super::rig::git(&rig.worktree(&tasks[1]), &["symbolic-ref", "--short", "HEAD"]);
    let output = Command::new(super::super::fixture::agentbus_binary())
        .args(["inv-add", "--dir"])
        .arg(rig.home.root().join("bus"))
        .args([
            "--from-agent",
            "claude-1",
            "--from-task",
            tasks[1].as_str(),
            "--repo",
            "example",
            "--branch",
            branch.trim(),
            "--tasks-dir",
        ])
        .arg(rig.home.tasks_dir())
        .args(["--question", "Known request", "--kind", ""])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    rig.wait_for("known request answered", || {
        bus_state(&rig)["investigations"]["2"]["status"] == "done"
    })
    .await;
    rig.stop().await.unwrap();
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    for (event, level, id) in [
        (LogEvent::InvestigationKindUnknown, LogLevel::Warn, "1"),
        (LogEvent::InvestigationStarted, LogLevel::Info, "2"),
        (LogEvent::InvestigationAnswered, LogLevel::Info, "2"),
    ] {
        let rows: Vec<_> = logs.iter().filter(|row| row.event() == event).collect();
        assert_eq!(rows.len(), 1, "{event:?}: {rows:?}");
        assert_eq!(rows[0].level(), level);
        assert_eq!(rows[0].team(), Some(&team));
        assert_eq!(rows[0].fields()["id"], id);
        observed.insert(event);
    }
}

#[tokio::test]
async fn an_unknown_kind_is_failed_once_without_starting_an_investigator() {
    let mut observed = std::collections::BTreeSet::new();
    record_investigation_events(&mut observed).await;
    assert_eq!(observed.len(), 3);
}
