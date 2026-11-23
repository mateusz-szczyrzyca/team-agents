//! Older detailed replies retain runs while omitting tool-use and no-tool renewal metadata introduced in protocol eight.

use agent_core::ids::RunId;
use agent_core::kinds::Role;
use agent_core::protocol::{Frame, Reply, ReplyFormat, RequestId, Response, RoundDetail, RoundTask};
use agent_core::state::{RunState, SessionId, State, Timestamp};
use serde::Deserialize;

/// The strict run shape before tool-use accounting existed.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct OldRun {
    /// Run identity.
    id: RunId,
    /// Launch time.
    started_at: Timestamp,
    /// CLI session, when reported.
    session: Option<SessionId>,
    /// Associated rounds.
    #[serde(default)]
    rounds: Vec<agent_core::ids::RoundId>,
    /// The unmodified outcome.
    state: RunState,
}

#[test]
fn detailed_replies_project_no_tool_metadata_only_for_clients_before_eight() {
    for reason in ["hang", "no_tool_call"] {
        let mut source: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/state-before-no-tool-call.json")).unwrap();
        source["schema_version"] = 15.into();
        let member = &mut source["teams"][0]["crew"]["member"];
        member["runs"][0]["used_tool"] = true.into();
        member["renewals"]["reason"] = reason.into();
        let state = State::from_json(&source.to_string()).unwrap();
        let team = &state.teams()[0];
        let task = "task-20261120-144101-r7q2".parse().unwrap();
        let member = state.find_task(&task).unwrap().member;
        let round_task = RoundTask {
            task: task.clone(),
            role: Role::Solo,
            agent: member.agent().clone(),
            worktree: "/tmp/example".parse().unwrap(),
            branch: "example".parse().unwrap(),
            runs: member.runs().to_vec(),
        };
        for reply in [
            Reply::Task {
                team: team.id().clone(),
                role: Role::Solo,
                member: Box::new(member.clone()),
            },
            Reply::Team {
                team: Box::new(team.clone()),
            },
            Reply::Round {
                round: Box::new(RoundDetail::new(team, &team.rounds()[0], vec![round_task])),
            },
        ] {
            let frame = Frame::Response(Response::ok(RequestId(1), reply));
            let current = frame.to_line_for(ReplyFormat::FailedReason).unwrap();
            assert_eq!(Frame::from_line_negotiated(&current).unwrap(), frame);
            assert!(current.contains("used_tool"));
            for format in [
                ReplyFormat::Legacy,
                ReplyFormat::Split,
                ReplyFormat::Plan,
                ReplyFormat::Executor,
                ReplyFormat::Versions,
            ] {
                let wire = frame.to_line_for(format).unwrap();
                let value: serde_json::Value = serde_json::from_str(&wire).unwrap();
                let reply = &value["result"]["reply"];
                let (runs, renewal) = match reply["kind"].as_str().unwrap() {
                    "task" => (&reply["member"]["runs"], &reply["member"]["renewals"]),
                    "team" => (
                        &reply["team"]["crew"]["member"]["runs"],
                        &reply["team"]["crew"]["member"]["renewals"],
                    ),
                    "round" => (&reply["round"]["tasks"][0]["runs"], &serde_json::Value::Null),
                    other => panic!("unexpected reply {other}"),
                };
                let decoded = serde_json::from_value::<Vec<OldRun>>(runs.clone());
                assert_eq!(
                    decoded.as_ref().map_or(usize::MAX, Vec::len),
                    member.runs().len(),
                    "old clients must decode every run: {decoded:?}"
                );
                let old_runs = decoded.unwrap();
                let mut expected = source["teams"][0]["crew"]["member"]["runs"].clone();
                expected[0].as_object_mut().unwrap().remove("used_tool");
                assert_eq!(old_runs, serde_json::from_value::<Vec<OldRun>>(expected).unwrap());
                if reply["kind"] != "round" {
                    assert_eq!(renewal.is_null(), reason == "no_tool_call");
                }
                assert!(!wire.contains("no_tool_call"));
                assert!(!wire.contains("used_tool"));
                Frame::from_line_negotiated(&wire).unwrap();
            }
        }
        assert!(
            state.to_json().unwrap().contains("used_tool"),
            "wire projection must not change state"
        );
    }
}
