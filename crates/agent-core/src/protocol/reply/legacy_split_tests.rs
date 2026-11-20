//! A protocol-three decoder rejects fields introduced for split execution.

use super::*;
use crate::ids::RoundId;
use crate::state::fixtures::team;
use crate::state::split::{PartDefinition, SplitPlan};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum OldFrame {
    Response {
        protocol: ProtocolVersion,
        id: RequestId,
        result: OldOutcome,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum OldOutcome {
    Ok { reply: OldReply },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum OldReply {
    Applied { tasks: Vec<TaskId> },
    Teams { teams: Vec<OldSummary> },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldSummary {
    id: TeamId,
    repo: RepoName,
    created_at: Timestamp,
    merge: MergeMode,
    items: ItemCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rounds: Vec<OldRound>,
    members: Vec<MemberSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    activities: Vec<crate::state::Activity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldRound {
    id: RoundId,
    #[serde(skip_serializing_if = "Option::is_none")]
    item: Option<crate::ids::ItemId>,
    opened_at: Timestamp,
    state: crate::state::RoundLife,
}

#[test]
fn protocol_three_answers_and_summaries_remain_readable_by_strict_old_types() {
    let mut answer = Response::ok(RequestId(1), Reply::Applied { tasks: vec![] });
    answer.split = Some("Recorded split parts: 1.2a — Guide.".into());
    let mut summary = TeamSummary::from(&team(1));
    summary.splits = vec![
        SplitPlan::new(
            "1.2".parse().unwrap(),
            vec![
                PartDefinition {
                    title: "Guide".into(),
                    delivers: "A guide".into(),
                    checks: "Read it".into(),
                    depends_on: vec![],
                },
                PartDefinition {
                    title: "Index".into(),
                    delivers: "An index".into(),
                    checks: "Read it".into(),
                    depends_on: vec![],
                },
            ],
        )
        .unwrap(),
    ];
    let status = Response::ok(RequestId(2), Reply::Teams { teams: vec![summary] });
    for response in [answer, status] {
        let line = crate::protocol::Frame::Response(response).to_line().unwrap();
        let old = serde_json::from_str::<OldFrame>(&line);
        assert!(
            matches!(
                old,
                Ok(OldFrame::Response {
                    protocol: ProtocolVersion,
                    ..
                })
            ),
            "{old:?}"
        );
    }
}

#[test]
fn protocol_four_preserves_split_details_and_selects_matching_replies() {
    use crate::protocol::{Client, Command, Frame, ProtocolError, ReplyFormat, Request, StatusRequest};
    let request = Request::new(
        RequestId(7),
        Client::Cli,
        Command::Status(StatusRequest { target: None }),
    );
    let line = request.to_line_for(ReplyFormat::Split).unwrap();
    let wire: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(wire["protocol"], 4);
    assert_eq!(
        Request::from_line_negotiated(&line).unwrap(),
        (request.clone(), ReplyFormat::Split)
    );
    assert_eq!(
        Request::from_line_negotiated(&request.to_line().unwrap()).unwrap(),
        (request, ReplyFormat::Legacy)
    );
    let mut response = Response::ok(RequestId(7), Reply::Applied { tasks: vec![] });
    response.split = Some("Recorded split parts: 1.2a — Guía.".into());
    let frame = Frame::Response(response);
    let line = frame.to_line_for(ReplyFormat::Split).unwrap();
    let wire: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(wire["protocol"], 4);
    assert_eq!(Frame::from_line_negotiated(&line).unwrap(), frame);
    assert!(matches!(
        Frame::from_line(&line),
        Err(ProtocolError::UnsupportedVersion(crate::protocol::UnsupportedVersion {
            found: 4
        }))
    ));
}

#[test]
fn negotiated_frames_check_size_version_and_malformed_data() {
    use crate::protocol::{Frame, MAX_FRAME_BYTES, MAX_REQUEST_BYTES, ProtocolError, Request};
    for version in [1, 2, 5] {
        let line = format!(r#"{{"protocol":{version},"type":"unknown"}}"#);
        assert!(
            matches!(Frame::from_line_negotiated(&line), Err(ProtocolError::UnsupportedVersion(crate::protocol::UnsupportedVersion { found })) if found == version)
        );
        assert!(
            matches!(Request::from_line_negotiated(&line), Err(ProtocolError::UnsupportedVersion(crate::protocol::UnsupportedVersion { found })) if found == version)
        );
    }
    assert!(matches!(
        Frame::from_line_negotiated(r#"{"protocol":4}"#),
        Err(ProtocolError::Malformed { .. })
    ));
    assert!(matches!(
        Request::from_line_negotiated(r#"{"protocol":4}"#),
        Err(ProtocolError::Malformed { .. })
    ));
    assert!(matches!(
        Frame::from_line_negotiated(&" ".repeat(MAX_FRAME_BYTES + 1)),
        Err(ProtocolError::TooLong { .. })
    ));
    assert!(matches!(
        Request::from_line_negotiated(&" ".repeat(MAX_REQUEST_BYTES + 1)),
        Err(ProtocolError::TooLong { .. })
    ));
}

fn working_split_team() -> Team {
    use crate::state::fixtures::{at, drive, round_id};
    let mut found = team(1);
    let parent = "1.2".parse().unwrap();
    found.open_round(round_id(1), Some(parent), at(1)).unwrap();
    drive(&mut found, "1.2", crate::state::PhaseKind::InProgress);
    let plan = SplitPlan::new(
        parent,
        ["Guide", "Index"]
            .into_iter()
            .map(|title| PartDefinition {
                title: title.into(),
                delivers: title.into(),
                checks: "Read it".into(),
                depends_on: vec![],
            })
            .collect(),
    )
    .unwrap();
    found.activate_split(plan).unwrap();
    found
}

#[test]
fn split_summary_events_and_round_identities_use_the_negotiated_version() {
    use crate::protocol::{Event, EventKind, Frame, ReplyFormat};
    use crate::state::fixtures::at;
    let summary = TeamSummary::from(&working_split_team());
    let response = Frame::Response(Response::ok(
        RequestId(9),
        Reply::Teams {
            teams: vec![summary.clone()],
        },
    ));
    let legacy = response.to_line().unwrap();
    assert!(matches!(
        serde_json::from_str::<OldFrame>(&legacy),
        Ok(OldFrame::Response { .. })
    ));
    let event = Frame::Event(Event::new(
        at(2),
        EventKind::TeamUpdated {
            summary: Box::new(summary),
        },
    ));
    for frame in [response, event] {
        let line = frame.to_line_for(ReplyFormat::Split).unwrap();
        assert_eq!(Frame::from_line_negotiated(&line).unwrap(), frame);
        assert!(line.contains("\"part\""));
        assert!(line.contains("\"splits\""));
        let line = frame.to_line_for(ReplyFormat::Legacy).unwrap();
        assert!(!line.contains("\"part\""));
        assert!(!line.contains("\"splits\""));
    }
}

#[test]
fn legacy_full_team_requests_explain_that_split_state_needs_protocol_four() {
    use crate::protocol::Frame;
    let response = Frame::Response(Response::ok(
        RequestId(9),
        Reply::Team {
            team: Box::new(working_split_team()),
        },
    ));
    let decoded = Frame::from_line(&response.to_line().unwrap()).unwrap();
    assert_eq!(
        decoded,
        Frame::Response(Response::error(
            RequestId(9),
            Failure {
                kind: FailureKind::Invalid,
                message: "showing a split team requires protocol 4".into(),
            }
        ))
    );
}
