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
