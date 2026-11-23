//! Protocol compatibility for `failed_reason`: every format a client negotiated before protocol eight must not carry
//! it, because the member summaries of those clients are strict and reject an unknown field. The reason is a new
//! member of the protocol, not a change of a known one: an old daemon's reply has no `failed_reason` key, so a new
//! client reads it as the absent reason, and an old client must read a new daemon's reply too.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use agent_core::config::RepoName;
use agent_core::ids::{AgentName, ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::{Role, Tier};
use agent_core::protocol::{
    Client, Command, Event, EventKind, FAILED_REASON_PROTOCOL_VERSION, Frame, ItemCounts, MemberSummary,
    PlanStepSummary, Reply, ReplyFormat, Request, RequestId, Response, Returned, RunElsewhere, StatusRequest,
    TeamSummary,
};
use agent_core::state::split::SplitPlan;
use agent_core::state::{Activity, MergeMode, Participant, Round, Timestamp};
use agent_core::status::TaskStatus;

/// The strict member summary shape understood by clients that negotiated before protocol eight: it has every field a
/// protocol-seven client knew, and it rejects an unknown one — `failed_reason` today.
#[expect(dead_code, reason = "simulates old client fields")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OldMemberSummary {
    task: TaskId,
    agent: Participant,
    role: Role,
    status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_question: Option<NonZeroU32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new_sessions: Option<NonZeroU32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held_by: Option<AgentName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    returned: Option<Returned>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    waits_for: Option<RunElsewhere>,
}

/// The strict team summary shape understood by clients that negotiated before protocol eight.
#[expect(dead_code, reason = "simulates old client fields")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OldTeamSummary {
    id: TeamId,
    repo: RepoName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    short_desc: Option<String>,
    created_at: Timestamp,
    merge: MergeMode,
    items: ItemCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rounds: Vec<Round>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    splits: Vec<SplitPlan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    part_assignments: BTreeMap<RoundId, AgentName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tier_waits: BTreeMap<ItemId, Tier>,
    members: Vec<OldMemberSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    activities: Vec<Activity>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    task_plans: BTreeMap<TaskId, Vec<PlanStepSummary>>,
}

/// The strict reply shape understood by clients that negotiated before protocol eight.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "status", content = "reply", rename_all = "snake_case")]
enum OldOutcome {
    Ok(OldReply),
}

/// The strict reply kind understood by clients that negotiated before protocol eight.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OldReply {
    Teams { teams: Vec<OldTeamSummary> },
}

/// The strict response understood by clients that negotiated before protocol eight.
#[expect(dead_code, reason = "simulates old client fields")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OldResponse {
    protocol: u32,
    id: RequestId,
    result: OldOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split: Option<String>,
}

/// The strict frame wrapper understood by clients that negotiated before protocol eight.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OldFrame {
    Response(OldResponse),
}

/// The strict event understood by clients that negotiated before protocol eight.
#[expect(dead_code, reason = "simulates old client fields")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OldEvent {
    protocol: u32,
    at: Timestamp,
    event: OldEventKind,
}

/// The strict event kind understood by clients that negotiated before protocol eight.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OldEventKind {
    TeamUpdated { summary: Box<OldTeamSummary> },
}

/// The strict frame wrapper of a subscribed client that negotiated before protocol eight.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OldEventFrame {
    Event(OldEvent),
}

/// The formats a client can negotiate today, with the protocol number each wire line declares.
const FORMATS: &[(ReplyFormat, u32)] = &[
    (ReplyFormat::Legacy, 3),
    (ReplyFormat::Split, 4),
    (ReplyFormat::Plan, 5),
    (ReplyFormat::Executor, 6),
    (ReplyFormat::Versions, 7),
];

/// A member whose task failed with the named reason, as the daemon writes it on the status line.
#[expect(clippy::unwrap_used, reason = "fixture values that must parse in a test")]
fn member() -> MemberSummary {
    MemberSummary {
        task: "task-20261111-193002-a1b2".parse().unwrap(),
        agent: Participant::Normal("claude-1".parse().unwrap()),
        role: Role::Member,
        status: TaskStatus::Failed,
        open_question: None,
        new_sessions: None,
        held_by: None,
        returned: None,
        waits_for: None,
        failed_reason: Some("two runs ended without calling a tool".to_owned()),
    }
}

/// A team summary with one failed member; its split executor and plan metadata exercise every strip.
#[expect(clippy::unwrap_used, reason = "fixture values that must parse in a test")]
fn summary() -> TeamSummary {
    TeamSummary {
        id: "team-20261111-193000-a1b2".parse().unwrap(),
        repo: "sample".parse().unwrap(),
        short_desc: None,
        created_at: "2026-11-11T19:30:00Z".parse().unwrap(),
        merge: MergeMode::Autopilot,
        items: ItemCounts::default(),
        rounds: Vec::new(),
        splits: Vec::new(),
        part_assignments: BTreeMap::from([(
            "round-20261111-193001-a1b2".parse().unwrap(),
            "claude-1".parse().unwrap(),
        )]),
        tier_waits: BTreeMap::new(),
        members: vec![member()],
        activities: Vec::new(),
        task_plans: BTreeMap::new(),
    }
}

#[test]
fn old_member_summaries_decode_status_replies_of_every_format_before_eight() {
    let frame = Frame::Response(Response::ok(RequestId(1), Reply::Teams { teams: vec![summary()] }));

    for &(format, protocol) in FORMATS {
        let wire = frame.to_line_for(format).unwrap();
        assert!(
            !wire.contains("failed_reason"),
            "{format:?} must not leak the failed reason: {wire}"
        );
        let decoded: OldFrame = serde_json::from_str(&wire)
            .unwrap_or_else(|error| panic!("{format:?} must decode with a strict old member summary: {error}: {wire}"));
        let OldFrame::Response(OldResponse {
            protocol: read,
            result: OldOutcome::Ok(OldReply::Teams { teams }),
            ..
        }) = decoded;
        assert_eq!(read, protocol);
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].members.len(), 1);
        assert_eq!(teams[0].members[0].status, TaskStatus::Failed);
    }
}

#[test]
fn old_member_summaries_decode_team_updated_events_of_every_format_before_eight() {
    let event = Event::new(
        "2026-11-11T19:31:00Z".parse().unwrap(),
        EventKind::TeamUpdated {
            summary: Box::new(summary()),
        },
    );
    let frame = Frame::Event(event);

    for &(format, protocol) in FORMATS {
        let wire = frame.to_line_for(format).unwrap();
        assert!(
            !wire.contains("failed_reason"),
            "{format:?} must not leak the failed reason in an event: {wire}"
        );
        let decoded: OldEventFrame = serde_json::from_str(&wire).unwrap_or_else(|error| {
            panic!("{format:?} event must decode with a strict old member summary: {error}: {wire}")
        });
        let OldEventFrame::Event(OldEvent {
            protocol: read,
            event: OldEventKind::TeamUpdated { summary },
            ..
        }) = decoded;
        assert_eq!(read, protocol);
        assert_eq!(summary.members.len(), 1);
        assert_eq!(summary.members[0].status, TaskStatus::Failed);
    }
}

#[test]
fn protocol_eight_carries_the_failed_reason_and_round_trips() {
    let request = Request::new(
        RequestId(1),
        Client::Cli,
        Command::Status(StatusRequest { target: None }),
    );
    let request_wire = request.to_line_for(ReplyFormat::FailedReason).unwrap();
    let request_json: serde_json::Value = serde_json::from_str(&request_wire).unwrap();
    assert_eq!(request_json["protocol"], FAILED_REASON_PROTOCOL_VERSION);
    assert_eq!(
        Request::from_line_negotiated(&request_wire).unwrap(),
        (request, ReplyFormat::FailedReason)
    );

    let frame = Frame::Response(Response::ok(RequestId(1), Reply::Teams { teams: vec![summary()] }));
    let frame_wire = frame.to_line_for(ReplyFormat::FailedReason).unwrap();
    let frame_json: serde_json::Value = serde_json::from_str(&frame_wire).unwrap();
    assert_eq!(frame_json["protocol"], FAILED_REASON_PROTOCOL_VERSION);
    assert_eq!(
        frame_json
            .pointer("/result/reply/teams/0/members/0/failed_reason")
            .and_then(serde_json::Value::as_str),
        Some("two runs ended without calling a tool")
    );
    assert_eq!(Frame::from_line_negotiated(&frame_wire).unwrap(), frame);
}
