//! The lines of the control protocol, version 2: `tests/fixtures/protocol-v2-requests.jsonl` (what a client sends)
//! and `protocol-v2-frames.jsonl` (what the daemon sends). They must keep reading — a daemon and a client of one
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
    const REQUESTS: &str = include_str!("fixtures/protocol-v2-requests.jsonl");

    /// The daemon's lines, one per line.
    const FRAMES: &str = include_str!("fixtures/protocol-v2-frames.jsonl");

    #[test]
    fn the_requests_of_version_1_read_and_write_back_to_the_same_lines() {
        let lines: Vec<&str> = REQUESTS.lines().collect();

        for line in &lines {
            let request = Request::from_line(line).unwrap();

            assert_eq!(request.to_line().unwrap(), format!("{line}\n"));
        }
        assert_eq!(lines.len(), 11);
    }

    #[test]
    fn the_frames_of_version_1_read_and_write_back_to_the_same_lines() {
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
        let other = first.replacen("\"protocol\":2", "\"protocol\":3", 1);
        assert_ne!(other, first);

        let err = Request::from_line(&other).unwrap_err();

        assert!(
            matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 3 })),
            "{err:?}"
        );
        assert_eq!(PROTOCOL_VERSION, 2);
    }

    #[test]
    fn a_frame_with_an_unknown_protocol_version_is_refused_with_a_typed_error() {
        let first = FRAMES.lines().next().unwrap();
        let other = first.replacen("\"protocol\":2", "\"protocol\":7", 1);
        assert_ne!(other, first);

        let err = Frame::from_line(&other).unwrap_err();

        assert!(
            matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 7 })),
            "{err:?}"
        );
    }
}
