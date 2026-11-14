//! The events of a round — gates, checks, the pull request and its reviews, questions, controls, notifications and the
//! ends of a round and a team — as values: their keywords, the keys that make a repeat the same event, the texts the
//! history redacts and cuts, and the JSON the ledger keeps.

#[cfg(test)]
mod history_events {
    use std::num::NonZeroU32;
    use std::str::FromStr;

    use agent_core::config::TailscaleLogin;
    use agent_core::decide::round::AskReason;
    use agent_core::history::{CheckOutcome, EventKind, HistoryEvent};
    use agent_core::ids::{BranchName, InvestigatorName};
    use agent_core::protocol::{Client, ControlAction, Target};
    use agent_core::state::{AnsweredVia, CommitId, GateOutcome, GateRecord, GateResult, GateTree, PrNumber, RoundEnd};
    use agent_core::status::ReviewVerdict;
    use strum::VariantNames;

    use agent_core::ids::{RoundId, TaskId, TeamId};
    use agent_core::state::Timestamp;

    /// The task, team and round `n` (0 to 59), and the time `n` seconds after 19:30:00 UTC.
    mod world {
        use super::*;

        pub(super) fn task(n: u32) -> TaskId {
            format!("task-20261111-1930{n:02}-k3x9").parse().unwrap()
        }

        pub(super) fn team_id(n: u32) -> TeamId {
            format!("team-20261111-1930{n:02}-t5m8").parse().unwrap()
        }

        pub(super) fn round(n: u32) -> RoundId {
            format!("round-20261111-1930{n:02}-q1w2").parse().unwrap()
        }

        pub(super) fn at(seconds: u32) -> Timestamp {
            let start: Timestamp = "2026-11-11T19:30:00Z".parse().unwrap();
            Timestamp::from_utc(start.as_utc() + time::Duration::seconds(i64::from(seconds))).unwrap()
        }
    }

    fn pr(n: u32) -> PrNumber {
        PrNumber::new(NonZeroU32::new(n).unwrap())
    }

    fn number(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    /// A commit of forty copies of `digit`.
    fn head(digit: char) -> CommitId {
        digit.to_string().repeat(40).parse().unwrap()
    }

    fn reviewer(name: &str) -> InvestigatorName {
        name.parse().unwrap()
    }

    fn gate() -> HistoryEvent {
        HistoryEvent::Gate {
            task: world::task(1),
            started_at: world::at(5),
            record: GateRecord {
                tree: GateTree::Clean,
                result: GateResult {
                    head: head('a'),
                    finished_at: world::at(9),
                    outcome: GateOutcome::Passed,
                    log: "/tmp/gate.log".parse().unwrap(),
                },
            },
        }
    }

    /// One event of every kind, with texts that are not ASCII and a text of a thousand characters.
    fn every_event() -> Vec<HistoryEvent> {
        vec![
            gate(),
            HistoryEvent::Checked {
                pr: Some(pr(7)),
                head: head('b'),
                issues: vec![
                    "test file changed: tests/a.rs".to_owned(),
                    "zażółć gęślą jaźń".to_owned(),
                ],
                outcome: CheckOutcome::SentBack { pass: number(1), of: 2 },
            },
            HistoryEvent::PullOpened {
                number: pr(7),
                branch: "agents/a1/20261111-193001".parse::<BranchName>().unwrap(),
                head: head('b'),
                title: "Add the thing — 日本語".to_owned(),
                body: "x".repeat(1000),
            },
            HistoryEvent::PullPushed {
                pr: pr(7),
                head: head('c'),
            },
            HistoryEvent::PullReady { pr: pr(7) },
            HistoryEvent::ReviewAsked {
                pr: pr(7),
                pass: 1,
                head: head('c'),
                reviewer: reviewer("i1"),
                task: world::task(2),
            },
            HistoryEvent::ReviewSubmitted {
                pr: pr(7),
                pass: 1,
                reviewer: reviewer("i1"),
                task: world::task(2),
                verdict: ReviewVerdict::RequestChanges,
                report: "Blocker: ❌ no test\nSTATUS: DONE".to_owned(),
            },
            HistoryEvent::FixPassStarted {
                pr: pr(7),
                pass: number(1),
                of: 5,
                findings: "Blocker: no test".to_owned(),
            },
            HistoryEvent::PullApproved { pr: pr(7) },
            HistoryEvent::PullMerged {
                pr: pr(7),
                commit: Some(head('d')),
            },
            HistoryEvent::PullHandedOver { pr: pr(7) },
            HistoryEvent::PullClosed { pr: pr(7) },
            HistoryEvent::QuestionAsked {
                task: world::task(1),
                number: number(2),
                text: "## 1. Which?\n**Verdict:**".to_owned(),
            },
            HistoryEvent::QuestionAnswered {
                task: world::task(1),
                number: number(2),
                text: "## 1. Which?\n**Verdict:** ok".to_owned(),
                via: AnsweredVia::Dashboard,
            },
            HistoryEvent::Controlled {
                action: ControlAction::Stop,
                target: Target::Team(world::team_id(1)),
                by: Client::Dashboard {
                    login: TailscaleLogin::from_str("human@example.com").unwrap(),
                },
                tasks: vec![world::task(1), world::task(2)],
            },
            HistoryEvent::Notified {
                task: Some(world::task(1)),
                title: "Questions".to_owned(),
                body: "Please answer.".to_owned(),
            },
            HistoryEvent::RoundClosed {
                round: world::round(1),
                end: RoundEnd::Merged,
            },
            HistoryEvent::TeamEnded {},
        ]
    }

    #[test]
    fn every_event_has_the_keyword_of_its_variant_and_the_keywords_read_back() {
        let expected = [
            "gate",
            "checked",
            "pull_opened",
            "pull_pushed",
            "pull_ready",
            "review_asked",
            "review_submitted",
            "fix_pass_started",
            "pull_approved",
            "pull_merged",
            "pull_handed_over",
            "pull_closed",
            "question_asked",
            "question_answered",
            "controlled",
            "notified",
            "round_closed",
            "team_ended",
        ];
        let events = every_event();
        assert_eq!(events.len(), expected.len());
        assert_eq!(EventKind::VARIANTS, expected);
        for (event, text) in events.iter().zip(expected) {
            assert_eq!(event.kind().to_string(), text, "{event:?}");
            assert_eq!(EventKind::from_str(text).unwrap(), event.kind());
        }
    }

    #[test]
    fn an_event_reads_back_equal_from_its_json_and_the_tag_is_its_keyword() {
        for event in every_event() {
            let json = serde_json::to_string(&event).unwrap();
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["event"], event.kind().to_string(), "{json}");
            assert_eq!(serde_json::from_str::<HistoryEvent>(&json).unwrap(), event, "{json}");
        }
    }

    #[test]
    fn an_event_with_an_unknown_field_or_an_unknown_name_is_refused() {
        let mut value = serde_json::to_value(HistoryEvent::PullReady { pr: pr(7) }).unwrap();
        value["extra"] = serde_json::json!(1);
        let error = serde_json::from_value::<HistoryEvent>(value).unwrap_err();
        assert_eq!(error.classify(), serde_json::error::Category::Data);
        let unknown = serde_json::json!({"event": "pull_teleported", "pr": 7});
        let error = serde_json::from_value::<HistoryEvent>(unknown).unwrap_err();
        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }

    #[test]
    fn what_can_happen_twice_has_no_key_and_what_cannot_has_one_that_names_it() {
        let keys: Vec<(String, Option<String>)> = every_event()
            .iter()
            .map(|event| (event.kind().to_string(), event.key()))
            .collect();
        let none = |kind: &str| keys.iter().find(|(name, _)| name == kind).unwrap().1.is_none();
        for kind in ["gate", "checked", "controlled", "notified"] {
            assert!(none(kind), "{kind} can happen again");
        }
        let mut seen = std::collections::BTreeSet::new();
        for (kind, key) in &keys {
            if let Some(key) = key {
                assert!(seen.insert((kind.clone(), key.clone())), "{kind} {key} is not unique");
            }
        }
        assert_eq!(seen.len(), keys.len() - 4);
    }

    #[test]
    fn two_events_of_one_kind_are_the_same_event_when_their_keys_are_equal() {
        let pushed = |n: u32, digit: char| {
            HistoryEvent::PullPushed {
                pr: pr(n),
                head: head(digit),
            }
            .key()
        };
        assert_eq!(pushed(7, 'c'), pushed(7, 'c'));
        assert_ne!(pushed(7, 'c'), pushed(7, 'd'));
        assert_ne!(pushed(7, 'c'), pushed(8, 'c'));
        let asked = |pass: u32, name: &str| {
            HistoryEvent::ReviewAsked {
                pr: pr(7),
                pass,
                head: head('c'),
                reviewer: reviewer(name),
                task: world::task(2),
            }
            .key()
        };
        assert_eq!(asked(1, "i1"), asked(1, "i1"));
        assert_ne!(asked(1, "i1"), asked(2, "i1"));
        assert_ne!(asked(1, "i1"), asked(1, "i2"));
        let answered = |n: u32| {
            HistoryEvent::QuestionAnswered {
                task: world::task(1),
                number: number(n),
                text: String::new(),
                via: AnsweredVia::Command,
            }
            .key()
        };
        assert_ne!(answered(1), answered(2));
    }

    #[test]
    fn the_texts_of_an_event_are_the_free_texts_it_has_each_under_the_name_of_its_field() {
        let expected: Vec<(&str, Vec<&str>)> = vec![
            ("gate", vec![]),
            ("checked", vec!["issues", "issues"]),
            ("pull_opened", vec!["title", "body"]),
            ("pull_pushed", vec![]),
            ("pull_ready", vec![]),
            ("review_asked", vec![]),
            ("review_submitted", vec!["report"]),
            ("fix_pass_started", vec!["findings"]),
            ("pull_approved", vec![]),
            ("pull_merged", vec![]),
            ("pull_handed_over", vec![]),
            ("pull_closed", vec![]),
            ("question_asked", vec!["text"]),
            ("question_answered", vec!["text"]),
            ("controlled", vec![]),
            ("notified", vec!["title", "body"]),
            ("round_closed", vec![]),
            ("team_ended", vec![]),
        ];
        for (mut event, (kind, names)) in every_event().into_iter().zip(expected) {
            assert_eq!(event.kind().to_string(), kind);
            let found: Vec<&str> = event.texts_mut().iter().map(|(name, _)| *name).collect();
            assert_eq!(found, names, "{kind}");
        }
    }

    #[test]
    fn changing_a_text_through_texts_mut_changes_the_event() {
        let mut total = 0;
        for mut event in every_event() {
            let mut count = 0;
            for (_, text) in event.texts_mut() {
                *text = "changed".to_owned();
                count += 1;
            }
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(json.matches("\"changed\"").count(), count, "{json}");
            total += count;
        }
        assert_eq!(total, 10);
    }

    #[test]
    fn the_identities_of_an_event_are_the_ids_and_the_login_a_credential_must_not_occur_in() {
        let events = every_event();
        let controlled = events
            .iter()
            .find(|event| event.kind() == EventKind::Controlled)
            .unwrap();
        let identities = controlled.identities();
        assert!(identities.contains(&"human@example.com"), "{identities:?}");
        assert!(identities.contains(&world::task(1).as_str()), "{identities:?}");
        assert!(identities.contains(&world::team_id(1).as_str()), "{identities:?}");
        let asked = events
            .iter()
            .find(|event| event.kind() == EventKind::ReviewAsked)
            .unwrap();
        assert!(asked.identities().contains(&"i1"));
        assert!(asked.identities().contains(&head('c').as_str()));
        let ended = events
            .iter()
            .find(|event| event.kind() == EventKind::TeamEnded)
            .unwrap();
        assert_eq!(ended.identities(), Vec::<&str>::new());
    }

    #[test]
    fn every_outcome_of_the_checks_reads_back_from_its_json() {
        for outcome in [
            CheckOutcome::SentBack { pass: number(2), of: 3 },
            CheckOutcome::AskedHuman {
                reason: AskReason::OnlyTheHumanMayAllow,
                question: number(4),
            },
            CheckOutcome::AskedHuman {
                reason: AskReason::PassesUsed,
                question: number(1),
            },
            CheckOutcome::Allowed,
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(serde_json::from_str::<CheckOutcome>(&json).unwrap(), outcome, "{json}");
        }
    }
}
