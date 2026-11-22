//! `agent status --json`: the teams as JSON, with the description of each.

use agent_core::protocol::TeamSummary;
use clap::Parser;

use super::*;
use crate::commands::status::render_json;
use crate::fixtures::team_state;

/// The sample team with the description `desc`.
fn summary(desc: Option<&str>) -> TeamSummary {
    let mut summary = TeamSummary::from(&team_state().teams()[0]);
    summary.short_desc = desc.map(str::to_owned);
    summary
}

#[test]
fn the_flag_is_taken_alone_with_a_team_or_a_task_and_not_with_another_view() {
    let team = "team-20261111-153000-t5m8";

    for args in [
        vec!["agent", "status", "--json"],
        vec!["agent", "status", team, "--json"],
    ] {
        assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
    }
    for args in [
        vec!["agent", "status", "--json", "--running"],
        vec!["agent", "status", "--json", "--investigators"],
    ] {
        let refused = Cli::try_parse_from(&args).unwrap_err();
        assert_eq!(refused.kind(), clap::error::ErrorKind::ArgumentConflict, "{args:?}");
    }
}

#[test]
fn the_json_is_an_array_with_a_team_each_and_its_description() {
    let text = render_json(&[summary(Some("Fix the typo in the README")), summary(None)]).unwrap();

    let value: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert!(text.ends_with('\n'), "{text:?}");
    assert_eq!(value.as_array().map(Vec::len), Some(2));
    assert_eq!(value[0]["short_desc"], "Fix the typo in the README");
    assert_eq!(value[0]["id"], "team-20261111-153000-t5m8");
    assert!(value[1].get("short_desc").is_none(), "{value}");
}

#[test]
fn no_teams_are_an_empty_array() {
    assert_eq!(render_json(&[]).unwrap(), "[]\n");
}

#[test]
fn the_json_of_the_teams_reads_back_as_the_teams() {
    let teams = vec![summary(Some("Żółć w powiadomieniach")), summary(None)];

    let read: Vec<TeamSummary> = serde_json::from_str(&render_json(&teams).unwrap()).unwrap();

    assert_eq!(read, teams);
}

#[tokio::test]
async fn the_command_prints_what_the_daemon_lists_as_json() {
    use agent_core::protocol::{Frame, Reply, Response};

    use crate::testing::{Answer, FakeDaemon};

    let dir = tempfile::tempdir().unwrap();
    let teams = vec![summary(None)];
    let listed = teams.clone();
    let _daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), move |request| {
        let reply = Reply::Teams { teams: listed.clone() };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    });
    let environment = Environment::new(Some(std::ffi::OsString::from(dir.path())));
    let mut out = Vec::new();

    execute(
        &environment,
        Command::Status(StatusArgs {
            target: None,
            running: false,
            investigators: false,
            json: true,
        }),
        &mut out,
    )
    .await
    .unwrap();

    let read: Vec<TeamSummary> = serde_json::from_slice(&out).unwrap();
    assert_eq!(read, teams);
}

#[test]
fn the_json_is_byte_for_byte_what_it_was_before_local_times() {
    let text = render_json(&[summary(Some("Fix the typo in the README"))]).unwrap();

    assert_eq!(
        text,
        "[{\"id\":\"team-20261111-153000-t5m8\",\"repo\":\"team-agents\",\"short_desc\":\"Fix the typo in the README\",\"created_at\":\"2026-11-11T19:15:00Z\",\"merge\":\"autopilot\",\"items\":{\"pending\":2,\"in_progress\":0,\"in_review\":0,\"approved\":0,\"done\":0},\"members\":[{\"task\":\"task-20261111-153001-k3x9\",\"agent\":{\"tier\":\"investigator\",\"name\":\"inv-opus\"},\"role\":\"coordinator\",\"status\":\"queued\"},{\"task\":\"task-20261111-153002-k3x9\",\"agent\":{\"tier\":\"normal\",\"name\":\"claude-1\"},\"role\":\"member\",\"status\":\"queued\"},{\"task\":\"task-20261111-153003-k3x9\",\"agent\":{\"tier\":\"worker\",\"name\":\"glm-1\"},\"role\":\"worker\",\"status\":\"queued\"},{\"task\":\"task-20261111-153004-k3x9\",\"agent\":{\"tier\":\"investigator\",\"name\":\"inv-codex\"},\"role\":\"reviewer\",\"status\":\"queued\"}],\"activities\":[{\"investigator\":\"inv-opus\",\"task\":\"task-20261111-153001-k3x9\",\"rounds\":[],\"request\":{\"kind\":\"team_member\"},\"asked_at\":\"2026-11-11T19:15:00Z\",\"outcome\":{\"status\":\"working\"}}]}]\n"
    );
}
