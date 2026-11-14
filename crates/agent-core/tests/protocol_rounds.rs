//! The lines of the control protocol that are about rounds, version 2: `tests/fixtures/protocol-v2-round-requests.jsonl`
//! and `protocol-v2-round-frames.jsonl`. They read and write back to the same lines; the lines of version 1 are refused as
//! the unsupported version they are, whatever they say.

#[cfg(test)]
mod protocol_rounds {
    use agent_core::kinds::Role;
    use agent_core::protocol::{
        Command, EventKind, Frame, Outcome, ProtocolError, Reply, Request, Target, UnsupportedVersion,
    };
    use agent_core::state::{RoundEnd, RoundLife};

    /// The requests about rounds, one per line.
    const REQUESTS: &str = include_str!("fixtures/protocol-v2-round-requests.jsonl");
    /// The daemon's lines about rounds, one per line.
    const FRAMES: &str = include_str!("fixtures/protocol-v2-round-frames.jsonl");
    /// The requests of version 1.
    const V1_REQUESTS: &str = include_str!("fixtures/protocol-v1-requests.jsonl");
    /// The daemon's lines of version 1.
    const V1_FRAMES: &str = include_str!("fixtures/protocol-v1-frames.jsonl");

    fn frames() -> Vec<Frame> {
        FRAMES.lines().map(|line| Frame::from_line(line).unwrap()).collect()
    }

    #[test]
    fn the_requests_about_rounds_read_and_write_back_to_the_same_lines() {
        let lines: Vec<&str> = REQUESTS.lines().collect();

        for line in &lines {
            assert_eq!(
                Request::from_line(line).unwrap().to_line().unwrap(),
                format!("{line}\n")
            );
        }
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn the_frames_about_rounds_read_and_write_back_to_the_same_lines() {
        let lines: Vec<&str> = FRAMES.lines().collect();

        for line in &lines {
            assert_eq!(Frame::from_line(line).unwrap().to_line().unwrap(), format!("{line}\n"));
        }
        assert_eq!(lines.len(), 11);
    }

    #[test]
    fn status_and_show_take_a_round_beside_a_team_and_a_task() {
        let requests: Vec<Request> = REQUESTS.lines().map(|line| Request::from_line(line).unwrap()).collect();

        let targets: Vec<&Target> = requests
            .iter()
            .filter_map(|request| match &request.command {
                Command::Status(status) => status.target.as_ref(),
                Command::Show(show) => Some(&show.target),
                _ => None,
            })
            .collect();

        assert_eq!(targets.len(), 3);
        assert!(targets.iter().all(|target| matches!(target, Target::Round(_))));
    }

    #[test]
    fn the_round_a_show_gives_has_its_item_its_state_its_tasks_its_pull_request_and_its_review_passes() {
        let Frame::Response(response) = &frames()[1] else {
            panic!("not a response");
        };
        let Outcome::Ok {
            reply: Reply::Round { round },
        } = &response.result
        else {
            panic!("not a round");
        };

        assert_eq!(round.round.item().map(|item| item.to_string()), Some("1.3".to_owned()));
        assert_eq!(round.round.life(), &RoundLife::Open);
        let task = &round.tasks[0];
        assert_eq!((task.role, task.agent.name()), (Role::Member, "codex-1"));
        assert_eq!(task.branch.as_str(), "agents/codex-1/item-1-3");
        assert!(task.worktree.as_path().ends_with("task-20261111-193002-k3x9"));
        assert_eq!(task.runs[0].rounds(), [round.round.id().clone()]);
        assert_eq!(round.pr.as_ref().map(|pr| pr.number.get().get()), Some(82));
        assert_eq!(round.review_passes.len(), 2);
        assert_eq!(round.review_passes[1].reviews.len(), 2);
    }

    #[test]
    fn a_summary_lists_the_rounds_of_its_team() {
        let Frame::Response(response) = &frames()[0] else {
            panic!("not a response");
        };
        let Outcome::Ok {
            reply: Reply::Teams { teams },
        } = &response.result
        else {
            panic!("not teams");
        };

        let rounds = &teams[0].rounds;

        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[0].ended(), Some(RoundEnd::Merged));
        assert!(rounds[1].is_open());
    }

    #[test]
    fn the_events_that_are_about_a_round_name_it() {
        let events: Vec<EventKind> = frames()
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Event(event) => Some(event.event),
                Frame::Response(_) => None,
            })
            .collect();

        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                EventKind::RoundOpened { item: Some(_), .. } => "round_opened",
                EventKind::RoundClosed {
                    how: RoundEnd::Merged, ..
                } => "round_closed merged",
                EventKind::RoundClosed {
                    how: RoundEnd::Abandoned,
                    item: None,
                    ..
                } => "round_closed abandoned",
                EventKind::ItemStatus { .. } => "item_status",
                EventKind::RunStarted { rounds, .. } if !rounds.is_empty() => "run_started",
                EventKind::PullRequestOpened { .. } => "pull_request_opened",
                EventKind::ReviewSubmitted { .. } => "review_submitted",
                EventKind::TeamUpdated { .. } => "team_updated",
                other => panic!("unexpected {other:?}"),
            })
            .collect();

        assert_eq!(
            kinds,
            [
                "round_opened",
                "item_status",
                "run_started",
                "pull_request_opened",
                "review_submitted",
                "round_closed merged",
                "round_closed abandoned",
                "team_updated"
            ]
        );
    }

    #[test]
    fn every_line_of_version_1_is_refused_as_an_unsupported_version() {
        for line in V1_REQUESTS.lines() {
            let err = Request::from_line(line).unwrap_err();

            assert!(
                matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 1 })),
                "{line}: {err:?}"
            );
        }
        for line in V1_FRAMES.lines() {
            let err = Frame::from_line(line).unwrap_err();

            assert!(
                matches!(err, ProtocolError::UnsupportedVersion(UnsupportedVersion { found: 1 })),
                "{line}: {err:?}"
            );
        }
    }
}
