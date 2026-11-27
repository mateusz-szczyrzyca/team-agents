use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

use super::*;

#[test]
fn a_run_keeps_a_plan_exactly_when_its_role_is_one_of_the_plan_roles() {
    let configured = Config::default().plan().roles().to_vec();
    let rows = [
        (Role::Solo, true),
        (Role::Member, true),
        (Role::Coordinator, true),
        (Role::Worker, false),
        (Role::Reviewer, false),
    ];
    for (role, kept) in rows {
        assert_eq!(keeps_plan(&configured, role), kept, "{role:?}");
    }
    // The roles come from the configuration: a home that lists none holds no run to a plan.
    assert!(!keeps_plan(&[], Role::Member));
    assert!(keeps_plan(&[Role::Member], Role::Member));
    assert!(!keeps_plan(&[Role::Member], Role::Coordinator));
}

#[test]
fn a_new_session_id_is_a_version_4_uuid() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(7);

    for _ in 0..200 {
        let id = new_session_id(&mut rng).unwrap();

        let text = id.as_str();
        assert_eq!(text.len(), 36, "{text}");
        let groups: Vec<&str> = text.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "{text}"
        );
        assert!(
            text.chars()
                .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{text}"
        );
        assert_eq!(groups[2].chars().next(), Some('4'), "the version: {text}");
        assert!(
            matches!(groups[3].chars().next(), Some('8' | '9' | 'a' | 'b')),
            "the variant: {text}"
        );
    }
}

#[test]
fn two_session_ids_differ() {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(1);

    let first = new_session_id(&mut rng).unwrap();
    let second = new_session_id(&mut rng).unwrap();

    assert_ne!(first, second);
}

#[derive(Clone, Default)]
struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_warnings(run: impl FnOnce()) -> String {
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn inconsistent_state_falls_back_to_continuation_and_logs_a_warning() {
    use crate::fixtures::{task, team_state};

    let state = team_state();
    let missing_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(9), None), RunWhy::Continuation);
    });
    assert!(
        missing_log.contains("the task of a new run is not in the state"),
        "{missing_log}"
    );

    let fix_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(1), Some(&Why::FixPass)), RunWhy::Continuation);
    });
    assert!(fix_log.contains("no item of the team is in a fix pass"), "{fix_log}");

    let review_log = capture_warnings(|| {
        assert_eq!(run_why(&state, &task(4), Some(&Why::Review)), RunWhy::Continuation);
    });
    assert!(
        review_log.contains("the reviewer has no pending review in the state"),
        "{review_log}"
    );
}

#[test]
fn turn_limit_run_why_yields_to_an_explicit_team_reason() {
    use agent_core::status::TaskStatus;

    use crate::fixtures::{at, run, task, team_id, team_state};

    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let (_, mut member) = team.find_mut(&task(2)).unwrap();
        member.begin_run(run(1), at(1), None).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(2), RunEnd::TurnLimit).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }

    assert_eq!(run_why(&state, &task(2), None), RunWhy::TurnLimit);
    assert_eq!(run_why(&state, &task(2), Some(&Why::Woken)), RunWhy::WakeUp);
}

#[test]
fn no_status_run_why_is_continuation() {
    use crate::fixtures::{task, team_state};

    let state = team_state();
    assert_eq!(run_why(&state, &task(2), Some(&Why::NoStatus)), RunWhy::Continuation);
}

#[test]
fn prompt_kind_resumes_no_status_up_to_retries() {
    use crate::fixtures::{at, run, task, team_id, team_state};
    use agent_core::config::Config;
    use agent_core::status::TaskStatus;

    let mut state = team_state();
    let session: agent_core::state::SessionId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let config = Config::default();

    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let (_, mut member) = team.find_mut(&task(2)).unwrap();
        // 1st no status
        member.begin_run(run(1), at(1), Some(session.clone())).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(2), RunEnd::NoStatus { exit_code: Some(0) }).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let found = state.find_task(&task(2)).unwrap();
    let cli_session = SessionId::from_str(session.as_str()).unwrap();
    assert_eq!(
        prompt_kind(found.member, Some(&cli_session), &config),
        PromptKind::Resume(ResumeReason::NoStatus)
    );
}

#[test]
fn human_resumed_workers_and_reviewers_keep_the_full_prompt_after_no_status() {
    use crate::fixtures::{at, run, task, team_state};
    use agent_core::status::TaskStatus;

    for (role, number, expected) in [
        (Role::Solo, 1, PromptKind::Resume(ResumeReason::NoStatus)),
        (Role::Coordinator, 1, PromptKind::Resume(ResumeReason::NoStatus)),
        (Role::Member, 2, PromptKind::Resume(ResumeReason::NoStatus)),
        (Role::Worker, 3, PromptKind::First),
        (Role::Reviewer, 4, PromptKind::First),
    ] {
        let mut state = team_state();
        let session: agent_core::state::SessionId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let (_, mut member) = state.find_task_mut(&task(number)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(run(1), at(1), Some(session.clone())).unwrap();
        member.end_run(at(2), RunEnd::NoStatus { exit_code: Some(0) }).unwrap();
        member.transition(TaskStatus::Failed).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        let found = state.find_task(&task(number)).unwrap();
        let cli_session = SessionId::from_str(session.as_str()).unwrap();
        assert_eq!(
            prompt_kind_for_role(role, found.member, Some(&cli_session), &Config::default()),
            expected,
            "{role:?}"
        );
    }
}

#[test]
fn reported_sessions_resume_for_every_report_outcome_and_role() {
    use crate::fixtures::{at, run, task, team_state};
    use agent_core::status::TaskStatus;

    for outcome in [RunOutcome::Done, RunOutcome::Questions] {
        for (role, number) in [
            (Role::Solo, 1),
            (Role::Coordinator, 1),
            (Role::Member, 2),
            (Role::Worker, 3),
            (Role::Reviewer, 4),
        ] {
            let mut state = team_state();
            let session = "00000000-0000-4000-8000-000000000001".parse().unwrap();
            let (_, mut member) = state.find_task_mut(&task(number)).unwrap();
            member.transition(TaskStatus::Running).unwrap();
            member.begin_run(run(1), at(1), Some(session)).unwrap();
            member.end_run(at(2), RunEnd::Reported { outcome }).unwrap();
            let found = state.find_task(&task(number)).unwrap();
            let cli_session = "00000000-0000-4000-8000-000000000001".parse().unwrap();
            assert_eq!(
                prompt_kind_for_role(role, found.member, Some(&cli_session), &Config::default()),
                PromptKind::Resume(ResumeReason::Reported),
                "{role:?} {outcome:?}"
            );
            assert_eq!(prompt_kind(found.member, None, &Config::default()), PromptKind::First);
        }
    }
}

#[test]
fn stopped_sessions_restart_and_hung_sessions_keep_the_hang_prompt() {
    use crate::fixtures::{at, run, task, team_state};
    use agent_core::status::TaskStatus;

    let config = Config::default();
    for (end, expected) in [
        (RunEnd::Stopped, PromptKind::First),
        (
            RunEnd::Hung,
            PromptKind::Resume(ResumeReason::Hang {
                after: config.watchdog().hang_minutes,
            }),
        ),
    ] {
        let mut state = team_state();
        let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(run(1), at(1), None).unwrap();
        member.end_run(at(2), end).unwrap();
        let session = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        assert_eq!(
            prompt_kind(state.find_task(&task(2)).unwrap().member, Some(&session), &config),
            expected
        );
    }
}

#[test]
fn no_status_beyond_the_retry_bound_gets_the_whole_prompt() {
    use crate::fixtures::{at, run, task, team_state};
    use agent_core::status::TaskStatus;

    let mut state = team_state();
    let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
    for number in 1..=NO_STATUS_RETRIES + 1 {
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run(run(u32::try_from(number).unwrap()), at(1), None)
            .unwrap();
        member.end_run(at(2), RunEnd::NoStatus { exit_code: Some(0) }).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let session = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    assert_eq!(
        prompt_kind(
            state.find_task(&task(2)).unwrap().member,
            Some(&session),
            &Config::default()
        ),
        PromptKind::First
    );
}

#[test]
fn a_session_replaced_by_its_bound_keeps_priority_over_a_reported_end() {
    use crate::fixtures::{at, run, task, team_state};
    use agent_core::status::TaskStatus;
    let mut state = team_state();
    let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(
            run(1),
            at(1),
            Some("00000000-0000-4000-8000-000000000001".parse().unwrap()),
        )
        .unwrap();
    member
        .end_run(
            at(2),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();
    assert!(state.begin_context_session(&task(2), at(3)).unwrap());
    let member = state.find_task(&task(2)).unwrap().member;
    assert!(replaced_by_bound(member));
    let session = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    assert_eq!(
        prompt_kind(member, Some(&session), &Config::default()),
        PromptKind::First
    );
}
