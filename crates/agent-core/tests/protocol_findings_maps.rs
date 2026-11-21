//! Tests reproducing the failure to decode maps keyed by finding numbers below tagged enums
//! and verifying round trips across all protocol replies and events.

#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod protocol_findings_maps {
    use std::num::NonZeroU32;

    use agent_core::history::{HistoryHealth, HistoryState};
    use agent_core::ids::{ItemId, RoundId, TeamId};
    use agent_core::kinds::Role;
    use agent_core::protocol::{
        ControlAction, Event, EventKind, Frame, Outcome, Reply, RequestId, Response, RoundDetail, TeamSummary,
    };
    use agent_core::state::{
        Findings, GateOutcome, GateRecord, GateResult, GateTree, ItemPhase, Member, Participant, PrNumber,
        ReviewOutcome, RoundEnd, RunEnd, State,
    };
    use agent_core::status::{ItemStatus, ReviewVerdict, RunOutcome, TaskStatus};
    use serde_json::json;

    use super::world::{at, item, pr, review, round, run, state, task, team_id};

    /// Creates a test `Findings` ledger with findings in fix pass:
    /// - finding 1: explanation accepted
    /// - finding 2: opinion waiting
    /// - finding 3: opinion assigned
    /// - finding 4: opinion reported not confirmed
    /// - finding 5: opinion failed and attempted again
    /// - finding 6: fixed
    fn fix_pass_findings() -> Findings {
        let fixture_state: State = State::from_json(include_str!("fixtures/state-v0-11-second-opinions.json")).unwrap();
        let team = fixture_state.team(&team_id(1)).unwrap();
        let item = team.item(item("1.2")).unwrap();
        item.findings().unwrap().clone()
    }

    /// Builds a state containing team 1 with an active round and item 1.2 in fixing phase. With `with_findings`, its
    /// ledger is the fix pass of [`fix_pass_findings`]; without, the ledger is empty: no finding, no opinion.
    fn fixing_state(with_findings: bool) -> (State, TeamId, RoundId) {
        let mut world_state = state();
        let team = team_id(1);
        let round_id: RoundId = "round-20261111-193001-q1w2".parse().unwrap();
        let item_id: ItemId = "1.2".parse().unwrap();

        world_state
            .open_round(&team, round_id.clone(), Some(item_id), at(1))
            .unwrap();

        {
            let mut t = world_state.team_mut(&team).unwrap();
            let mut item = t.item_mut(item_id).unwrap();
            item.advance(ItemPhase::InProgress {
                assignee: "codex-1".parse().unwrap(),
                branch: "agents/codex-1/item-1-2".parse().unwrap(),
            })
            .unwrap();
            let pull = pr(1);
            item.advance(ItemPhase::InReview {
                pr: pull.clone(),
                reviews: Vec::new(),
            })
            .unwrap();
            item.ask_reviews(vec![
                review("inv-codex", 7, ReviewOutcome::Pending),
                review("inv-gemini", 8, ReviewOutcome::Pending),
            ])
            .unwrap();
            item.submit_review(&task(7), ReviewVerdict::RequestChanges, at(30))
                .unwrap();
            let ItemPhase::InReview { pr: opened, .. } = item.phase() else {
                panic!("in review");
            };
            let opened = opened.clone();
            item.advance(ItemPhase::Fixing {
                assignee: "codex-1".parse().unwrap(),
                branch: opened.branch.clone(),
                pr: opened,
            })
            .unwrap();
            item.start_findings();
            if with_findings {
                *item.findings_mut().unwrap() = fix_pass_findings();
            }
        }

        (world_state, team, round_id)
    }

    /// The fix pass of [`fixing_state`], with its findings.
    fn fix_pass_state() -> (State, TeamId, RoundId) {
        fixing_state(true)
    }

    /// Every variant of [`Reply`] that carries part of the state of `round_id` in `state`.
    fn replies(state: &State, team_id: &TeamId, round_id: &RoundId) -> Vec<Reply> {
        let team = state.team(team_id).unwrap();
        let round_state = team.rounds().iter().find(|r| r.id() == round_id).unwrap();
        let detail = RoundDetail::new(team, round_state, Vec::new());
        let summary = TeamSummary::from(team);

        let gate_res = GateResult {
            head: "1111111111111111111111111111111111111111".parse().unwrap(),
            finished_at: at(10),
            outcome: GateOutcome::Passed,
            log: "/tmp/gate.log".parse().unwrap(),
        };

        vec![
            Reply::Started {
                team: team_id.clone(),
                tasks: vec![task(1), task(2)],
                shared: Vec::new(),
            },
            Reply::Teams {
                teams: vec![summary.clone()],
            },
            Reply::Overview {
                teams: vec![summary.clone()],
                history: HistoryHealth {
                    state: HistoryState::Writing,
                    lost: 0,
                },
            },
            Reply::Team {
                team: Box::new(team.clone()),
            },
            Reply::Round {
                round: Box::new(detail),
            },
            Reply::Task {
                team: team_id.clone(),
                role: Role::Coordinator,
                member: Box::new(Member::new(task(1), Participant::Normal("claude-1".parse().unwrap()))),
            },
            Reply::Applied { tasks: vec![task(1)] },
            Reply::Gate {
                result: gate_res.clone(),
            },
            Reply::GateRecorded {
                record: GateRecord {
                    result: gate_res,
                    tree: GateTree::Clean,
                },
            },
            Reply::Subscribed { teams: vec![summary] },
        ]
    }

    /// Every variant of [`EventKind`] that carries part of the state of `round_id` in `state`.
    fn event_kinds(team_id: &TeamId, round_id: &RoundId, summary: TeamSummary) -> Vec<EventKind> {
        vec![
            EventKind::TeamStarted { team: team_id.clone() },
            EventKind::TeamUpdated {
                summary: Box::new(summary),
            },
            EventKind::TaskStatus {
                team: team_id.clone(),
                task: task(1),
                status: TaskStatus::Waiting,
            },
            EventKind::RoundOpened {
                team: team_id.clone(),
                round: round_id.clone(),
                item: Some(item("1.2")),
            },
            EventKind::RoundClosed {
                team: team_id.clone(),
                round: round_id.clone(),
                item: Some(item("1.2")),
                how: RoundEnd::Approved,
            },
            EventKind::ItemStatus {
                team: team_id.clone(),
                item: item("1.2"),
                round: round_id.clone(),
                status: ItemStatus::InProgress,
            },
            EventKind::RunStarted {
                team: team_id.clone(),
                task: task(1),
                run: run(1),
                rounds: vec![round_id.clone()],
            },
            EventKind::RunEnded {
                team: team_id.clone(),
                task: task(1),
                run: run(1),
                rounds: vec![round_id.clone()],
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            },
            EventKind::QuestionAsked {
                team: team_id.clone(),
                task: task(1),
                number: NonZeroU32::new(1).unwrap(),
            },
            EventKind::QuestionAnswered {
                team: team_id.clone(),
                task: task(1),
                number: NonZeroU32::new(1).unwrap(),
                via: agent_core::state::AnsweredVia::Command,
            },
            EventKind::PullRequestOpened {
                team: team_id.clone(),
                item: item("1.2"),
                round: round_id.clone(),
                pr: PrNumber::new(NonZeroU32::new(1).unwrap()),
            },
            EventKind::ReviewSubmitted {
                team: team_id.clone(),
                item: item("1.2"),
                round: round(1),
                reviewer: "inv-codex".parse().unwrap(),
                verdict: ReviewVerdict::RequestChanges,
            },
            EventKind::GateFinished {
                team: team_id.clone(),
                task: task(1),
                outcome: GateOutcome::Passed,
            },
            EventKind::Controlled {
                team: team_id.clone(),
                task: Some(task(1)),
                action: ControlAction::Stop,
                by: agent_core::protocol::Client::Cli,
            },
        ]
    }

    /// Writes each reply as the daemon does, wrapped in a response, reads it back as the client does, and checks it
    /// is unchanged.
    fn assert_replies_round_trip(replies: Vec<Reply>) {
        for (index, reply) in replies.into_iter().enumerate() {
            // Exhaustive match ensures any variant added later fails to compile until covered here.
            match &reply {
                Reply::Started { .. }
                | Reply::Teams { .. }
                | Reply::Overview { .. }
                | Reply::Team { .. }
                | Reply::Round { .. }
                | Reply::Task { .. }
                | Reply::Applied { .. }
                | Reply::Gate { .. }
                | Reply::GateRecorded { .. }
                | Reply::Subscribed { .. } => {}
            }

            let response = Response::ok(RequestId(u64::try_from(index).unwrap() + 1), reply);
            let frame = Frame::Response(response);
            let line = frame.to_line().unwrap();
            let decoded = Frame::from_line(&line).unwrap();
            assert_eq!(decoded, frame);
        }
    }

    /// Writes each event as the daemon does and reads it back as the client does, and checks it is unchanged.
    fn assert_event_round_trips(events: Vec<EventKind>) {
        for (index, kind) in events.into_iter().enumerate() {
            // Exhaustive match ensures any variant added later fails to compile until covered here.
            match &kind {
                EventKind::TeamStarted { .. }
                | EventKind::TeamUpdated { .. }
                | EventKind::TaskStatus { .. }
                | EventKind::RoundOpened { .. }
                | EventKind::RoundClosed { .. }
                | EventKind::ItemStatus { .. }
                | EventKind::RunStarted { .. }
                | EventKind::RunEnded { .. }
                | EventKind::QuestionAsked { .. }
                | EventKind::QuestionAnswered { .. }
                | EventKind::PullRequestOpened { .. }
                | EventKind::ReviewSubmitted { .. }
                | EventKind::GateFinished { .. }
                | EventKind::Controlled { .. } => {}
            }

            let event = Event::new(at(u32::try_from(index).unwrap()), kind);
            let frame = Frame::Event(event);
            let line = frame.to_line().unwrap();
            let decoded = Frame::from_line(&line).unwrap();
            assert_eq!(decoded, frame);
        }
    }

    /// The state of the fixture with `key` added to its `fixed` map of item 1.2, as JSON text.
    fn state_with_fixed_key(key: &str) -> String {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/state-v0-11-second-opinions.json")).unwrap();
        let fixed = &mut value["teams"][0]["items"][1]["findings"]["fixed"];
        fixed.as_object_mut().unwrap().insert(key.to_owned(), json!([]));
        serde_json::to_string(&value).unwrap()
    }

    /// Asserts that a state with `key` among the fixed answers of item 1.2 is refused, and that the refusal names
    /// the key, quoted as it was written.
    fn assert_state_refuses_key(key: &str) {
        let text = state_with_fixed_key(key);
        let Err(err) = State::from_json(&text) else {
            panic!("the key {key:?} is not a finding number and must be refused");
        };
        // The state's message does not repeat the key; the cause, which names it, is in the chain.
        let mut chain = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            chain.push_str(" / ");
            chain.push_str(&cause.to_string());
            source = cause.source();
        }
        assert!(
            chain.contains(&format!("{key:?}")),
            "error should name invalid key {key:?}: {chain}"
        );
    }

    #[test]
    fn round_detail_answer_round_trips_through_response() {
        let (state, team_id, round_id) = fix_pass_state();
        let team = state.team(&team_id).unwrap();
        let round = team.rounds().iter().find(|r| *r.id() == round_id).unwrap();
        let detail = RoundDetail::new(team, round, Vec::new());

        let response = Response::ok(
            RequestId(1),
            Reply::Round {
                round: Box::new(detail.clone()),
            },
        );
        let frame = Frame::Response(response);
        let line = frame.to_line().unwrap();

        let decoded = Frame::from_line(&line).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn team_answer_round_trips_through_response() {
        let (state, team_id, _) = fix_pass_state();
        let team = state.team(&team_id).unwrap();

        let response = Response::ok(
            RequestId(2),
            Reply::Team {
                team: Box::new(team.clone()),
            },
        );
        let frame = Frame::Response(response);
        let line = frame.to_line().unwrap();

        let decoded = Frame::from_line(&line).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn state_v0_11_fixture_with_second_opinions_loads_and_writes_back_unchanged() {
        let fixture_text = include_str!("fixtures/state-v0-11-second-opinions.json");
        let state = State::from_json(fixture_text).unwrap();
        let serialized = format!("{}\n", state.to_json().unwrap());
        assert_eq!(
            serialized,
            include_str!("fixtures/state-v0-11-second-opinions-activities.json")
                .replace("\"schema_version\": 11", "\"schema_version\": 14")
        );

        let item = &state.teams()[0].items()[1];
        let ledger = item.findings().unwrap();
        assert_eq!(ledger.entries().len(), 6);
        assert_eq!(ledger.opinions().len(), 4);
        assert_eq!(ledger.opinion_attempts(NonZeroU32::new(5).unwrap()).len(), 1);
        assert_eq!(ledger.fixed_answers(NonZeroU32::new(1).unwrap()).len(), 1);
        assert!(ledger.disputes().is_some());
    }

    #[test]
    fn daemon_answer_fixture_from_main_loads() {
        let fixture_line = include_str!("fixtures/protocol-v3-round-answer-opinions.json");
        let frame = Frame::from_line(fixture_line).unwrap();
        let Frame::Response(response) = &frame else {
            panic!("expected response");
        };
        let Outcome::Ok {
            reply: Reply::Round { round },
        } = &response.result
        else {
            panic!("expected round reply");
        };
        assert_eq!(round.opinions.len(), 4);
        assert_eq!(round.opinion_attempts.len(), 1);
    }

    #[test]
    fn every_reply_variant_round_trips_below_response() {
        let (state, team_id, round_id) = fix_pass_state();
        assert_replies_round_trip(replies(&state, &team_id, &round_id));
    }

    #[test]
    fn every_event_kind_variant_round_trips_below_event() {
        let (state, team_id, round_id) = fix_pass_state();
        let summary = TeamSummary::from(state.team(&team_id).unwrap());
        assert_event_round_trips(event_kinds(&team_id, &round_id, summary));
    }

    #[test]
    fn key_deserialization_accepts_positive_integers_and_refuses_invalid_keys() {
        for invalid in ["0", "-1", "x", ""] {
            assert_state_refuses_key(invalid);
        }
    }

    #[test]
    fn key_deserialization_refuses_keys_that_are_not_canonical_decimal_numbers() {
        for invalid in ["01", "+1", "007"] {
            assert_state_refuses_key(invalid);
        }
    }

    #[test]
    fn every_reply_variant_round_trips_with_an_empty_ledger() {
        let (state, team_id, round_id) = fixing_state(false);
        assert_replies_round_trip(replies(&state, &team_id, &round_id));
    }

    #[test]
    fn every_event_kind_variant_round_trips_with_an_empty_ledger() {
        let (state, team_id, round_id) = fixing_state(false);
        let summary = TeamSummary::from(state.team(&team_id).unwrap());
        assert_event_round_trips(event_kinds(&team_id, &round_id, summary));
    }
}
