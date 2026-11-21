//! Wire compatibility before executor-aware status negotiation.

use super::*;
use crate::protocol::{Client, Command, Event, EventKind, Frame, ReplyFormat, Request, StatusRequest};
use crate::state::fixtures::{at, round_id, team};

/// The strict summary shape understood before executor and wait metadata.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OldTeamSummary {
    id: TeamId,
    repo: RepoName,
    created_at: Timestamp,
    merge: MergeMode,
    items: ItemCounts,
    #[serde(default)]
    rounds: Vec<Round>,
    #[serde(default)]
    splits: Vec<crate::state::split::SplitPlan>,
    members: Vec<MemberSummary>,
    #[serde(default)]
    activities: Vec<crate::state::Activity>,
    #[serde(default)]
    task_plans: std::collections::BTreeMap<TaskId, Vec<PlanStepSummary>>,
}

fn populated_summary() -> TeamSummary {
    let mut summary = TeamSummary::from(&team(1));
    summary
        .part_assignments
        .insert(round_id(1), "claude-1".parse().unwrap());
    summary
        .tier_waits
        .insert("1.2".parse().unwrap(), crate::kinds::Tier::Investigator);
    summary
}

#[test]
fn older_protocols_decode_populated_summaries_with_the_strict_old_shape() {
    let summary = populated_summary();
    let mut expected = serde_json::to_value(&summary).unwrap();
    expected.as_object_mut().unwrap().remove("part_assignments");
    expected.as_object_mut().unwrap().remove("tier_waits");
    let expected: OldTeamSummary = serde_json::from_value(expected).unwrap();
    let frames = [
        (
            Frame::Response(Response::ok(
                RequestId(1),
                Reply::Teams {
                    teams: vec![summary.clone()],
                },
            )),
            "/result/reply/teams/0",
        ),
        (
            Frame::Event(Event::new(
                at(1),
                EventKind::TeamUpdated {
                    summary: Box::new(summary),
                },
            )),
            "/event/summary",
        ),
    ];
    for (frame, path) in frames {
        for format in [ReplyFormat::Legacy, ReplyFormat::Split, ReplyFormat::Plan] {
            let wire: serde_json::Value = serde_json::from_str(&frame.to_line_for(format).unwrap()).unwrap();
            let old = serde_json::from_value::<OldTeamSummary>(wire.pointer(path).unwrap().clone());
            assert!(matches!(&old, Ok(found) if found == &expected), "{format:?}: {old:?}");
        }
    }
}

#[test]
fn executor_protocol_round_trips_requests_responses_and_events_with_both_metadata_maps() {
    let request = Request::new(
        RequestId(1),
        Client::Cli,
        Command::Status(StatusRequest { target: None }),
    );
    let wire = request.to_line_for(ReplyFormat::Executor).unwrap();
    let value: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(value["protocol"], crate::protocol::EXECUTOR_PROTOCOL_VERSION);
    assert_eq!(
        Request::from_line_negotiated(&wire).unwrap(),
        (request, ReplyFormat::Executor)
    );
    let summary = populated_summary();
    for frame in [
        Frame::Response(Response::ok(
            RequestId(1),
            Reply::Teams {
                teams: vec![summary.clone()],
            },
        )),
        Frame::Event(Event::new(
            at(1),
            EventKind::TeamUpdated {
                summary: Box::new(summary),
            },
        )),
    ] {
        let wire = frame.to_line_for(ReplyFormat::Executor).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(value["protocol"], 6);
        assert_eq!(Frame::from_line_negotiated(&wire).unwrap(), frame);
    }
}

#[test]
fn full_team_replies_hide_asked_wait_markers_from_older_clients() {
    let mut value = serde_json::to_value(team(1)).unwrap();
    value["tier_waits"] = serde_json::json!({"1.2": "investigator"});
    let mut team: crate::state::Team = serde_json::from_value(value).unwrap();
    team.mark_tier_questions();
    let frame = Frame::Response(Response::ok(RequestId(1), Reply::Team { team: Box::new(team) }));
    let marker = serde_json::json!({"1.2": "investigator"});
    for (format, expected) in [
        (ReplyFormat::Legacy, None),
        (ReplyFormat::Split, None),
        (ReplyFormat::Plan, None),
        (ReplyFormat::Executor, Some(&marker)),
    ] {
        let wire: serde_json::Value = serde_json::from_str(&frame.to_line_for(format).unwrap()).unwrap();
        assert_eq!(
            wire.pointer("/result/reply/team/tier_questions"),
            expected,
            "{format:?}"
        );
    }
}
