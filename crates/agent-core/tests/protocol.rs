//! The lines of the control protocol, version 3: `tests/fixtures/protocol-v3-requests.jsonl` (what a client sends)
//! and `protocol-v3-frames.jsonl` (what the daemon sends). They must keep reading — a daemon and a client of one
//! version understand each other — and writing them back must give the same text.

#[cfg(test)]
mod protocol_lines {
    use agent_core::ids::AgentName;
    use agent_core::protocol::{
        Client, Command, ControlAction, EventKind, Frame, Outcome, PROTOCOL_VERSION, ProtocolError, Reply, Request,
        RequestId, Target, UnsupportedVersion,
    };
    use agent_core::questions::Verdict;

    /// The requests, one per line.
    const REQUESTS: &str = include_str!("fixtures/protocol-v3-requests.jsonl");

    /// The daemon's lines, one per line.
    const FRAMES: &str = include_str!("fixtures/protocol-v3-frames.jsonl");

    #[test]
    fn the_requests_of_version_3_read_and_write_back_to_the_same_lines() {
        let lines: Vec<&str> = REQUESTS.lines().collect();

        for line in &lines {
            let request = Request::from_line(line).unwrap();

            assert_eq!(request.to_line().unwrap(), format!("{line}\n"));
        }
        assert_eq!(lines.len(), 11);
    }

    #[test]
    fn the_frames_of_version_3_read_and_write_back_to_the_same_lines() {
        let lines: Vec<&str> = FRAMES.lines().collect();

        for line in &lines {
            let frame = Frame::from_line(line).unwrap();

            assert_eq!(frame.to_line().unwrap(), format!("{line}\n"));
        }
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn every_command_of_the_protocol_has_a_line_in_the_fixture() {
        let commands: Vec<Command> = REQUESTS
            .lines()
            .map(|l| Request::from_line(l).unwrap().command)
            .collect();

        let names: Vec<&str> = commands
            .iter()
            .map(|c| match c {
                Command::Run(_) => "run",
                Command::Status(_) => "status",
                Command::Show(_) => "show",
                Command::Answer(_) => "answer",
                Command::Stop(_) => "stop",
                Command::Pause(_) => "pause",
                Command::Unpause(_) => "unpause",
                Command::Resume(_) => "resume",
                Command::Say(_) => "say",
                Command::Gate(_) => "gate",
                Command::Subscribe(_) => "subscribe",
            })
            .collect();

        assert_eq!(
            names,
            [
                "run",
                "status",
                "show",
                "answer",
                "stop",
                "pause",
                "unpause",
                "resume",
                "say",
                "gate",
                "subscribe"
            ]
        );
    }

    #[test]
    fn the_run_and_the_answer_of_the_fixture_carry_what_the_lines_say() {
        let requests: Vec<Request> = REQUESTS.lines().map(|l| Request::from_line(l).unwrap()).collect();

        let Command::Run(run) = &requests[0].command else {
            panic!("not a run");
        };
        assert_eq!(requests[0].id, RequestId(1));
        assert_eq!(requests[0].client, Client::Cli);
        assert_eq!(run.agents.as_slice().len(), 2);
        assert_eq!(run.coordinator.as_ref().map(AgentName::as_str), Some("inv-opus"));
        assert_eq!(run.task_list.list().items().count(), 1);
        let Command::Answer(answer) = &requests[3].command else {
            panic!("not an answer");
        };
        assert!(matches!(requests[3].client, Client::Dashboard { .. }));
        assert_eq!(answer.answers.as_slice()[0].verdict, Verdict::Accept);
        assert!(matches!(answer.answers.as_slice()[1].verdict, Verdict::Own(_)));
        let Command::Stop(stop) = &requests[4].command else {
            panic!("not a stop");
        };
        assert!(matches!(stop.target, Target::Task(_)));
    }

    #[test]
    fn the_replies_and_events_of_the_fixture_carry_what_the_lines_say() {
        let frames: Vec<Frame> = FRAMES.lines().map(|l| Frame::from_line(l).unwrap()).collect();

        let Frame::Response(started) = &frames[0] else {
            panic!("not a response");
        };
        assert!(matches!(&started.result, Outcome::Ok { reply: Reply::Started { tasks, .. } } if tasks.len() == 2));
        let Frame::Response(failed) = &frames[2] else {
            panic!("not a response");
        };
        assert!(matches!(&failed.result, Outcome::Error { error } if error.message.contains("running")));
        let Frame::Event(paused) = &frames[5] else {
            panic!("not an event");
        };
        assert!(matches!(
            &paused.event,
            EventKind::Controlled {
                action: ControlAction::Pause,
                task: None,
                ..
            }
        ));
    }

    #[test]
    fn a_request_with_an_unknown_protocol_version_is_refused_with_a_typed_error() {
        let first = REQUESTS.lines().next().unwrap();
        let other = first.replacen("\"protocol\":3", "\"protocol\":4", 1);
        assert_ne!(other, first);

        let err = Request::from_line(&other).unwrap_err();

        assert!(
            matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 4 })),
            "{err:?}"
        );
        assert_eq!(PROTOCOL_VERSION, 3);
    }

    #[test]
    fn a_frame_with_an_unknown_protocol_version_is_refused_with_a_typed_error() {
        let first = FRAMES.lines().next().unwrap();
        let other = first.replacen("\"protocol\":3", "\"protocol\":7", 1);
        assert_ne!(other, first);

        let err = Frame::from_line(&other).unwrap_err();

        assert!(
            matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 7 })),
            "{err:?}"
        );
    }

    #[test]
    fn old_protocol_four_client_decodes_cleanly_without_task_plans() {
        use agent_core::config::RepoName;
        use agent_core::ids::{ItemId, TaskId, TeamId};
        use agent_core::protocol::{ItemCounts, PlanStepSummary, ReplyFormat, RequestId, Response, TeamSummary};
        use agent_core::state::{MergeMode, Timestamp};

        #[expect(dead_code, reason = "simulates old client fields")]
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OldTeamSummary {
            id: TeamId,
            repo: RepoName,
            created_at: Timestamp,
            merge: MergeMode,
            items: ItemCounts,
            #[serde(default)]
            rounds: Vec<agent_core::state::Round>,
            #[serde(default)]
            splits: Vec<agent_core::state::split::SplitPlan>,
            members: Vec<agent_core::protocol::MemberSummary>,
            #[serde(default)]
            activities: Vec<agent_core::state::Activity>,
        }

        #[expect(dead_code, reason = "simulates old client fields")]
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OldResponse {
            r#type: String,
            protocol: u32,
            id: RequestId,
            result: OldOutcome,
        }

        #[expect(dead_code, reason = "simulates old client fields")]
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        #[serde(tag = "status", content = "reply", rename_all = "snake_case")]
        enum OldOutcome {
            Ok(OldReply),
        }

        #[expect(dead_code, reason = "simulates old client fields")]
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum OldReply {
            Teams { teams: Vec<OldTeamSummary> },
        }

        let task = "task-20261121-010000-abcd".parse::<TaskId>().unwrap();
        let step = PlanStepSummary {
            id: "1.1".parse::<ItemId>().unwrap(),
            mark: serde_json::from_value(serde_json::json!("x")).unwrap(),
            title: "First step".to_owned(),
            state: Some(agent_core::plan::judge::JudgementState::Confirmed),
            reason: None,
        };
        let mut task_plans = std::collections::BTreeMap::new();
        task_plans.insert(task.clone(), vec![step]);

        let summary = TeamSummary {
            id: "team-20261121-010000-abcd".parse::<TeamId>().unwrap(),
            repo: "repo".parse::<RepoName>().unwrap(),
            created_at: Timestamp::from_utc(time::OffsetDateTime::UNIX_EPOCH.into()).unwrap(),
            merge: MergeMode::Autopilot,
            items: ItemCounts::default(),
            rounds: Vec::new(),
            splits: Vec::new(),
            members: Vec::new(),
            activities: Vec::new(),
            task_plans,
            tier_waits: std::collections::BTreeMap::new(),
            part_assignments: std::collections::BTreeMap::new(),
        };

        let frame = Frame::Response(Response::ok(RequestId(1), Reply::Teams { teams: vec![summary] }));

        let split_line = frame.to_line_for(ReplyFormat::Split).unwrap();
        assert!(
            !split_line.contains("task_plans"),
            "split protocol 4 line must omit task_plans: {split_line}"
        );

        let parsed: OldResponse =
            serde_json::from_str(&split_line).expect("old protocol 4 client must decode line cleanly");
        assert_eq!(parsed.protocol, 4);
    }

    #[test]
    fn new_protocol_five_client_receives_task_plans() {
        use agent_core::config::RepoName;
        use agent_core::ids::{ItemId, TaskId, TeamId};
        use agent_core::protocol::{ItemCounts, PlanStepSummary, ReplyFormat, RequestId, Response, TeamSummary};
        use agent_core::state::{MergeMode, Timestamp};

        let task = "task-20261121-010000-abcd".parse::<TaskId>().unwrap();
        let step = PlanStepSummary {
            id: "1.1".parse::<ItemId>().unwrap(),
            mark: serde_json::from_value(serde_json::json!("x")).unwrap(),
            title: "First step".to_owned(),
            state: Some(agent_core::plan::judge::JudgementState::Confirmed),
            reason: None,
        };
        let mut task_plans = std::collections::BTreeMap::new();
        task_plans.insert(task.clone(), vec![step]);

        let summary = TeamSummary {
            id: "team-20261121-010000-abcd".parse::<TeamId>().unwrap(),
            repo: "repo".parse::<RepoName>().unwrap(),
            created_at: Timestamp::from_utc(time::OffsetDateTime::UNIX_EPOCH.into()).unwrap(),
            merge: MergeMode::Autopilot,
            items: ItemCounts::default(),
            rounds: Vec::new(),
            splits: Vec::new(),
            members: Vec::new(),
            activities: Vec::new(),
            task_plans,
            tier_waits: std::collections::BTreeMap::new(),
            part_assignments: std::collections::BTreeMap::new(),
        };

        let frame = Frame::Response(Response::ok(RequestId(1), Reply::Teams { teams: vec![summary] }));

        let plan_line = frame.to_line_for(ReplyFormat::Plan).unwrap();
        assert!(
            plan_line.contains("\"protocol\":5"),
            "plan protocol line must have protocol 5: {plan_line}"
        );
        assert!(
            plan_line.contains("task_plans"),
            "plan protocol line must retain task_plans: {plan_line}"
        );

        let decoded = Frame::from_line_negotiated(&plan_line).expect("must decode protocol 5 frame");
        let Frame::Response(Response {
            result: Outcome::Ok {
                reply: Reply::Teams { teams },
            },
            ..
        }) = decoded
        else {
            panic!("expected Reply::Teams");
        };
        assert_eq!(teams[0].task_plans.len(), 1);
        assert_eq!(teams[0].task_plans.get(&task).unwrap()[0].title, "First step");
    }

    #[test]
    fn new_protocol_five_client_preserves_both_split_fields_and_task_plans() {
        use agent_core::config::RepoName;
        use agent_core::ids::{ItemId, TaskId, TeamId};
        use agent_core::protocol::{ItemCounts, PlanStepSummary, ReplyFormat, RequestId, Response, TeamSummary};
        use agent_core::state::split::SplitPlan;
        use agent_core::state::{MergeMode, Timestamp};

        let task = "task-20261121-010000-abcd".parse::<TaskId>().unwrap();
        let step = PlanStepSummary {
            id: "1.1".parse::<ItemId>().unwrap(),
            mark: serde_json::from_value(serde_json::json!("x")).unwrap(),
            title: "First step".to_owned(),
            state: Some(agent_core::plan::judge::JudgementState::Confirmed),
            reason: None,
        };
        let mut task_plans = std::collections::BTreeMap::new();
        task_plans.insert(task.clone(), vec![step]);

        let split_plan: SplitPlan = serde_json::from_value(serde_json::json!({
            "parent": "1.1",
            "parts": [
                { "title": "Part 1", "delivers": "Delivers 1", "checks": "Checks 1", "depends_on": [] },
                { "title": "Part 2", "delivers": "Delivers 2", "checks": "Checks 2", "depends_on": [] }
            ]
        }))
        .unwrap();

        let summary = TeamSummary {
            id: "team-20261121-010000-abcd".parse::<TeamId>().unwrap(),
            repo: "repo".parse::<RepoName>().unwrap(),
            created_at: Timestamp::from_utc(time::OffsetDateTime::UNIX_EPOCH.into()).unwrap(),
            merge: MergeMode::Autopilot,
            items: ItemCounts::default(),
            rounds: Vec::new(),
            splits: vec![split_plan],
            members: Vec::new(),
            activities: Vec::new(),
            task_plans,
            tier_waits: std::collections::BTreeMap::new(),
            part_assignments: std::collections::BTreeMap::new(),
        };

        let frame = Frame::Response(Response::ok(RequestId(1), Reply::Teams { teams: vec![summary] }));

        let plan_line = frame.to_line_for(ReplyFormat::Plan).unwrap();
        assert!(
            plan_line.contains("\"protocol\":5"),
            "plan protocol line must have protocol 5: {plan_line}"
        );
        assert!(
            plan_line.contains("splits"),
            "plan protocol line must retain splits: {plan_line}"
        );
        assert!(
            plan_line.contains("task_plans"),
            "plan protocol line must retain task_plans: {plan_line}"
        );

        let decoded = Frame::from_line_negotiated(&plan_line).expect("must decode protocol 5 frame");
        let Frame::Response(Response {
            result: Outcome::Ok {
                reply: Reply::Teams { teams },
            },
            ..
        }) = decoded
        else {
            panic!("expected Reply::Teams");
        };
        assert_eq!(teams[0].splits.len(), 1);
        assert_eq!(teams[0].task_plans.len(), 1);
        assert_eq!(teams[0].task_plans.get(&task).unwrap()[0].title, "First step");
    }
}

#[test]
fn plan_step_summary_rejects_unknown_marks() {
    for mark in ["q", "", "xx"] {
        let result = serde_json::from_value::<agent_core::protocol::PlanStepSummary>(serde_json::json!({
            "id": "1.1", "mark": mark, "title": "Write guide", "state": null, "reason": null
        }));
        match result {
            Err(error) => assert_eq!(error.classify(), serde_json::error::Category::Data),
            Ok(summary) => panic!("invalid mark accepted: {summary:?}"),
        }
    }
}
