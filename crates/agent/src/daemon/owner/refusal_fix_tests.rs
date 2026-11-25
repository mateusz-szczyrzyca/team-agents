//! Regression coverage for refusal accounting and cooldown selection at daemon boundaries.

use agent_core::state::RunEnd;
use agent_core::status::TaskStatus;

use super::fixture::Daemon;
use crate::fixtures::{at, run, solo_state, task, team_id};

#[tokio::test]
async fn every_non_refusal_run_end_clears_the_previous_count() {
    for end in [RunEnd::NoStatus { exit_code: None }, RunEnd::Stopped] {
        let mut state = solo_state();
        {
            let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
            member.transition(TaskStatus::Running).unwrap();
            member.begin_run(run(1), at(0), None).unwrap();
        }
        state.record_limit_refusals(task(1), 1);
        let daemon = Daemon::new(&state, None);
        let (mut owner, _sinks) = daemon.bare_owner().await;

        owner
            .end_run(&task(1), &team_id(0), at(1), end, false, None, &mut Vec::new())
            .unwrap();

        assert_eq!(owner.parts.state.limit_refusals(&task(1)), 0);
    }
}

#[tokio::test]
async fn ordinary_ladder_takeover_skips_an_idle_refused_agent() {
    use super::takeover::Takeover;
    use agent_core::decide::ladder::Trouble;
    use agent_core::kinds::Role;

    let mut state = solo_state();
    state.keep_refused_until("claude-2".parse().unwrap(), at(10));
    let daemon = Daemon::new(&state, None);
    daemon.configure("[replacement]\nagents = [\"claude-2\"]\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    assert_eq!(
        owner.take_over(&task(1), Role::Solo, Trouble::NoStatus).unwrap(),
        Takeover::Nobody
    );
    assert_eq!(
        owner.parts.state.find_task(&task(1)).unwrap().member.agent().name(),
        "claude-1"
    );
}

#[tokio::test]
async fn reused_task_waiting_for_a_fresh_assignment_observes_the_cooldown() {
    use crate::daemon::team::Why;

    let mut state = solo_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(run(1), at(0), None).unwrap();
        member.end_run(at(1), RunEnd::Stopped).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    state.keep_refused_until("claude-1".parse().unwrap(), at(10));
    let daemon = Daemon::new(&state, None);
    daemon.scenario(1, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.reasons.insert(task(1), Why::Assigned("1.1".parse().unwrap()));

    owner.start_queued().await.unwrap();

    assert_eq!(owner.parts.state.find_task(&task(1)).unwrap().member.runs().len(), 1);
    assert_eq!(owner.runs.len(), 0);
}

#[tokio::test]
async fn recorded_worker_refusal_blocks_pool_requests_until_return() {
    use super::fixture::pair_state;
    use std::os::unix::fs::PermissionsExt;

    let scripts = tempfile::tempdir().unwrap();
    let bus = scripts.path().join("bus");
    let claimed = scripts.path().join("claimed");
    std::fs::write(&bus, r#"#!/bin/sh
case "$*" in
 *helper-list*) printf '%s\n' '[{"id":1,"from":"claude-1","from_task":"task-20261122-120000-aaaa","repo":"team-agents","branch":"main","status":"queued"}]' ;;
 *helper-next*) touch "$(dirname "$0")/claimed" ;;
 *) exit 1 ;;
esac
"#).unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = pair_state();
    state.keep_refused_until("glm-1".parse().unwrap(), at(10));
    let daemon = Daemon::new(&state, Some(&bus));
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace(
            "[tiers]",
            "[agents.glm-1]\ncli = \"claude\"\n[tiers]\nworkers = [\"glm-1\"]",
        ),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.serve_pool_requests().await.unwrap();
    assert!(!claimed.exists());
    daemon.clock.advance(time::Duration::seconds(11));
    owner.serve_pool_requests().await.unwrap();
    assert!(claimed.exists());
}

#[tokio::test]
async fn live_quota_cooldown_keeps_the_full_named_reset_beyond_the_task_pause_cap() {
    use crate::daemon::settle::Settled;
    use agent_core::decide::outcome::{Pause, PauseReason};

    let state = solo_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let reset = agent_core::state::Timestamp::from_utc(at(0).as_utc() + time::Duration::days(10)).unwrap();
    let day_nine = agent_core::state::Timestamp::from_utc(at(0).as_utc() + time::Duration::days(9)).unwrap();
    let capped = agent_core::state::Timestamp::from_utc(at(0).as_utc() + time::Duration::days(8)).unwrap();
    let located = owner.locate(&task(1)).unwrap();
    owner
        .keep_refusal(
            &task(1),
            &located,
            &Settled::Paused(Pause {
                until: capped,
                reason: PauseReason::Quota,
                evidence: String::new(),
            }),
            &RunEnd::QuotaExhausted { resets_at: Some(reset) },
            at(0),
        )
        .unwrap();

    assert_eq!(
        owner.parts.state.refused_until(&"claude-1".parse().unwrap()),
        Some(reset)
    );
    assert!(owner.parts.state.refused_at(&"claude-1".parse().unwrap(), day_nine));
    assert!(!owner.parts.state.refused_at(&"claude-1".parse().unwrap(), reset));
}

/// Captured structured fields of refusal decisions, shared with the tracing layer.
#[derive(Clone, Default)]
struct Decisions(std::sync::Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Decisions {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        /// Fields of one event, preserving the values the subscriber receives.
        #[derive(Default)]
        struct Fields(std::collections::BTreeMap<String, String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().to_owned(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.insert(field.name().to_owned(), value.to_owned());
            }
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.0.get("decision").map(String::as_str) == Some("limit") {
            self.0.lock().unwrap().push(fields.0);
        }
    }
}

#[tokio::test]
async fn refusal_decisions_emit_pause_and_change_agent_log_records() {
    use crate::daemon::settle::Settled;
    use agent_core::decide::outcome::{Pause, PauseReason};
    use tracing_subscriber::layer::SubscriberExt;

    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(1)).unwrap();
    let decisions = Decisions::default();
    let subscriber = tracing_subscriber::registry().with(decisions.clone());
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..2 {
            owner
                .keep_refusal(
                    &task(1),
                    &located,
                    &Settled::Paused(Pause {
                        until: at(10),
                        reason: PauseReason::RateLimit,
                        evidence: String::new(),
                    }),
                    &RunEnd::RateLimited,
                    at(0),
                )
                .unwrap();
        }
    });
    let records = decisions.0.lock().unwrap();
    let actual: Vec<_> = records
        .iter()
        .map(|record| {
            ["event", "decision", "result", "agent", "refusals", "return_at", "task"]
                .map(|key| record.get(key).cloned())
        })
        .collect();
    let expected: Vec<_> = [("pause", "1"), ("change_agent", "2")]
        .into_iter()
        .map(|(result, count)| {
            [
                "decision_taken".to_owned(),
                "limit".to_owned(),
                result.to_owned(),
                "claude-1".to_owned(),
                count.to_owned(),
                at(10).to_string(),
                task(1).to_string(),
            ]
            .map(Some)
        })
        .collect();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn failure_interruption_and_commit_cut_each_restart_the_refusal_sequence() {
    use super::commit_reminder::CommitOrder;
    use crate::daemon::control::Halt;
    use crate::daemon::settle::Settled;
    use agent_core::decide::outcome::{Pause, PauseReason};

    for path in ["failed", "interrupted", "stopped", "commit_cut"] {
        let mut state = solo_state();
        {
            let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
            member.transition(TaskStatus::Running).unwrap();
            member.begin_run(run(1), at(0), None).unwrap();
        }
        state.record_limit_refusals(task(1), 1);
        let daemon = Daemon::new(&state, None);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        daemon.clock.advance(time::Duration::seconds(1));
        match path {
            "failed" => owner
                .failed(&task(1), "a process could not finish".to_owned())
                .await
                .unwrap(),
            "interrupted" => owner.interrupted(&task(1), None).await.unwrap(),
            "stopped" => {
                owner.halts.insert(task(1), Halt::Stop);
                owner.interrupted(&task(1), None).await.unwrap();
            }
            "commit_cut" => {
                owner.commit_cuts.insert(task(1), CommitOrder { lines: 10, minutes: 1 });
                owner.interrupted(&task(1), None).await.unwrap();
            }
            _ => panic!("table lists the four daemon paths"),
        }
        assert_eq!(owner.parts.state.limit_refusals(&task(1)), 0, "{path}");
        let located = owner.locate(&task(1)).unwrap();
        owner
            .keep_refusal(
                &task(1),
                &located,
                &Settled::Paused(Pause {
                    until: at(10),
                    reason: PauseReason::RateLimit,
                    evidence: String::new(),
                }),
                &RunEnd::RateLimited,
                at(1),
            )
            .unwrap();
        assert_eq!(owner.parts.state.limit_refusals(&task(1)), 1, "{path}");
    }
}

#[tokio::test]
async fn keep_refusal_returns_some_when_the_rule_decides_to_change_agent() {
    use crate::daemon::settle::Settled;
    use agent_core::decide::outcome::{Pause, PauseReason};

    let mut state = solo_state();
    // One prior refusal: the next one with retries=1 triggers ChangeAgent.
    state.record_limit_refusals(task(1), 1);
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(1)).unwrap();

    let result = owner
        .keep_refusal(
            &task(1),
            &located,
            &Settled::Paused(Pause {
                until: at(10),
                reason: PauseReason::RateLimit,
                evidence: String::new(),
            }),
            &RunEnd::RateLimited,
            at(0),
        )
        .unwrap();

    assert!(
        result.is_some(),
        "keep_refusal should return Some when ChangeAgent is decided (retries=1, this is the second refusal)"
    );
}
