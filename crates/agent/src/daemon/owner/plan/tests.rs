//! The plan of a task through the daemon: what a run is told of it, and a plan committed by mistake.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::ItemId;
use agent_core::questions::QuestionsFile;
use agent_core::state::{
    Crew, Member, MergeMode, PlanKeeping, PlanReason, PlanStage, Question, State, TaskListText, Team,
};
use agent_core::status::TaskStatus;

use super::super::fixture::{Daemon, PATIENCE, answer, n};
use super::PlanQuestion;
use crate::fixtures::{LIST, at, normal, repo, solo_state, task, team_id};

/// The first lines of the rules a run is told of the plan.
const RULES: &str = "## Your plan\n\nKeep the plan of your task in `.agent/plan.md`";

/// [`solo_state`] whose task 1 was sent back for `reason`, which no run has been told yet.
fn sent_back(reason: PlanReason) -> State {
    let mut state = solo_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.send_back(reason);
    }
    state
}

/// A plan whose every step is done or given up.
const FINISHED: &str = "Finish it.\n\n1. Work\n[x] 1.1 Do it\n[-] 1.2 Polish it\n    Not asked for.\n";

/// Gives the worktree of task 1 of `daemon` a finished plan, so that its fake agent's `DONE` is taken as it is.
fn finished_plan(daemon: &Daemon) {
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), FINISHED).unwrap();
}

/// A daemon of `state` whose configuration holds runs to their plan `bounces` times.
fn with_bounces(state: &State, bounces: u32) -> Daemon {
    let daemon = Daemon::new(state, None);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", &format!("[plan]\nbounces = {bounces}\n")),
    )
    .unwrap();
    daemon
}

#[tokio::test]
async fn a_run_sent_back_is_told_its_reason_and_no_later_run_is() {
    let reason = PlanReason::Unfinished {
        steps: vec!["1.2".parse::<ItemId>().unwrap(), "2.1".parse::<ItemId>().unwrap()],
    };
    let mut daemon = with_bounces(&sent_back(reason), 3);
    daemon.scenario(1, "done");
    finished_plan(&daemon);

    daemon.start().await;
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("are still to do or in hand: 1.2, 2.1. Finish each of them"),
        "{prompt}"
    );
    let keeping = state.find_task(&task(1)).unwrap().member.plan().unwrap().clone();
    assert_eq!((keeping.sent_back(), keeping.pending()), (1, None));
}

#[tokio::test]
async fn a_plan_that_does_not_parse_is_told_with_its_line() {
    let reason = PlanReason::Unreadable {
        line: Some(std::num::NonZeroU32::new(7).unwrap()),
        detail: "line 7: the step is not in a stage — zażółć".to_owned(),
    };
    let mut daemon = with_bounces(&sent_back(reason), 3);
    daemon.scenario(1, "done");
    finished_plan(&daemon);

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("is not in the form of a plan: line 7: the step is not in a stage — zażółć."),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_first_prompt_has_the_rules_of_the_plan_where_runs_are_held_to_it() {
    let mut daemon = with_bounces(&solo_state(), 3);
    daemon.scenario(1, "done");
    finished_plan(&daemon);

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(prompt.contains(RULES), "{prompt}");
}

#[tokio::test]
async fn no_rules_are_told_when_no_run_is_sent_back_for_its_plan() {
    let mut daemon = with_bounces(&solo_state(), 0);
    daemon.scenario(1, "done");

    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(!prompt.contains(RULES), "{prompt}");
}

/// What the questions number 1 of task 1 are about.
#[derive(Clone, Copy)]
enum Asked {
    /// The plan, sent back as often as allowed.
    Plan,
    /// Something the agent asked.
    OtherQuestions,
}

/// A daemon whose task 1 was sent back once for its plan and waits for the human's answers to questions number 1,
/// `asked`; its next run ends with `DONE`.
async fn waiting_for_answers(asked: Asked) -> Daemon {
    let mut daemon = with_bounces(&solo_state(), 1);
    let file = daemon.questions_file(1, 1);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        "# Questions — x\n\n## 1. Go on?\na) go on as it is\nb) send it back once more\n\n**Recommended:** a) — x\n\n**Verdict:**\n",
    )
    .unwrap();
    let mut state = solo_state();
    {
        let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Questions).unwrap();
        member
            .ask_question(Question::new(
                n(1),
                AbsolutePath::from_str(file.to_str().unwrap()).unwrap(),
                at(0),
            ))
            .unwrap();
        member.send_back(PlanReason::Missing);
        drop(member.take_plan_reason());
        if matches!(asked, Asked::Plan) {
            member.plan_asked(n(1));
        }
    }
    std::fs::write(daemon.home.state_file(), state.to_json().unwrap()).unwrap();
    finished_plan(&daemon);
    daemon.start().await;
    daemon
}

/// The plan task 1 keeps in `state`.
fn keeping(state: &State) -> PlanKeeping {
    state.find_task(&task(1)).unwrap().member.plan().unwrap().clone()
}

#[tokio::test]
async fn the_question_after_the_limit_is_the_next_questions_file_with_the_last_correction() {
    let daemon = Daemon::new(&solo_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(1)).unwrap();

    let written = owner
        .write_plan_question(
            &task(1),
            &located,
            &PlanReason::LostStep {
                steps: vec!["1.3".parse::<ItemId>().unwrap()],
            },
        )
        .await
        .unwrap();

    assert_eq!(written, PlanQuestion::Written);
    let text = std::fs::read_to_string(daemon.questions_file(1, 1)).unwrap();
    let file = QuestionsFile::parse(&text).unwrap();
    let [entry] = file.entries() else {
        panic!("one question expected: {text}");
    };
    assert!(entry.body().contains("> ## Your plan lost a step"), "{text}");
    assert!(entry.body().contains("another step now: 1.3."), "{text}");
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(
        member.plan().map(PlanKeeping::stage),
        Some(PlanStage::Asked { question: n(1) })
    );
}

#[tokio::test]
async fn going_on_as_it_is_releases_the_plan_and_the_task_goes_on_with_the_answers() {
    let mut daemon = waiting_for_answers(Asked::Plan).await;

    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert_eq!(keeping(&state).stage(), PlanStage::Released);
    assert!(daemon.recorded(1, ".fake-args").contains("questions number 1"));
}

#[tokio::test]
async fn sending_it_back_once_more_grants_and_counts_one_more_send_back() {
    let mut daemon = waiting_for_answers(Asked::Plan).await;

    daemon.ask(answer(1, 1, &[(1, "b")])).await.unwrap();
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let kept = keeping(&state);
    assert_eq!(
        (kept.sent_back(), kept.granted(), kept.stage()),
        (2, 1, PlanStage::Open)
    );
}

#[tokio::test]
async fn an_answer_to_other_questions_leaves_the_plan_as_it_is() {
    let mut daemon = waiting_for_answers(Asked::OtherQuestions).await;

    daemon.ask(answer(1, 1, &[(1, "b")])).await.unwrap();
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let kept = keeping(&state);
    assert_eq!(
        (kept.sent_back(), kept.granted(), kept.stage()),
        (1, 0, PlanStage::Open)
    );
}

/// A daemon of `state` holding runs to their plan `bounces` times, whose task 1's worktree has `plan` (none for `None`)
/// and whose fake agent ends every run with `DONE`; started.
async fn running_with_plan(state: &State, bounces: u32, plan: Option<&str>) -> Daemon {
    let mut daemon = with_bounces(state, bounces);
    let dir = daemon.scenario(1, "done");
    if let Some(plan) = plan {
        std::fs::create_dir_all(dir.join(".agent")).unwrap();
        std::fs::write(dir.join(".agent/plan.md"), plan).unwrap();
    }
    daemon.start().await;
    daemon
}

#[tokio::test]
async fn a_plan_that_does_not_parse_sends_the_run_back_and_after_the_limit_the_human_is_asked_once() {
    let mut daemon = running_with_plan(&solo_state(), 1, Some("1. Work\nnot a step — zażółć\n")).await;

    let state = daemon.status_is(1, TaskStatus::Questions).await;
    let note = daemon.notification_with("questions-1.md").await;
    daemon.stop().await.unwrap();

    let second = daemon.recorded(1, ".fake-args");
    assert!(second.contains("is not in the form of a plan: line 2:"), "{second}");
    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.questions().len(), 1);
    assert_eq!(member.runs().len(), 2);
    let keeping = member.plan().unwrap();
    assert_eq!(
        (keeping.sent_back(), keeping.stage()),
        (1, PlanStage::Asked { question: n(1) })
    );
    let text = std::fs::read_to_string(daemon.questions_file(1, 1)).unwrap();
    assert!(text.contains("> ## Your plan does not parse"), "{text}");
    assert!(note.contains(&format!("agent answer {}", task(1))), "{note}");
}

#[tokio::test]
async fn a_run_without_a_plan_is_sent_back_with_the_text_of_a_missing_plan() {
    let mut daemon = running_with_plan(&solo_state(), 1, None).await;

    daemon.status_is(1, TaskStatus::Questions).await;
    daemon.stop().await.unwrap();

    let second = daemon.recorded(1, ".fake-args");
    assert!(second.contains("## Your plan is missing"), "{second}");
}

#[tokio::test]
async fn a_role_outside_the_plans_roles_is_not_sent_back_and_its_plan_is_recorded() {
    let mut daemon = Daemon::new(&solo_state(), None);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nroles = [\"member\"]\nbounces = 3\n"),
    )
    .unwrap();
    daemon.scenario(1, "done");
    daemon.start().await;

    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.runs().len(), 1);
    let kept = agent_runtime::history::Reader::open(&daemon.home.history_file())
        .unwrap()
        .plan_versions(&task(1))
        .unwrap();
    let reads: Vec<_> = kept.into_iter().map(|record| record.read).collect();
    assert_eq!(reads, [agent_runtime::history::PlanRead::Missing]);
}

#[tokio::test]
async fn a_task_created_before_plans_is_never_sent_back() {
    let mut state = State::new();
    let crew = Crew::solo(Member::without_plan(task(1), normal("claude-1"))).unwrap();
    state
        .insert_team(Team::new(
            team_id(0),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            crew,
        ))
        .unwrap();
    let mut daemon = running_with_plan(&state, 3, None).await;

    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!((member.runs().len(), member.plan()), (1, None));
}

#[tokio::test]
async fn a_restart_of_the_daemon_keeps_the_count_of_the_send_backs() {
    let mut daemon = running_with_plan(&solo_state(), 2, None).await;
    daemon
        .state_where("sent back once", |state| {
            state
                .find_task(&task(1))
                .and_then(|found| found.member.plan())
                .is_some_and(|keeping| keeping.sent_back() >= 1)
        })
        .await;
    daemon.stop().await.unwrap();

    daemon.start().await;
    let state = daemon.status_is(1, TaskStatus::Questions).await;
    daemon.stop().await.unwrap();

    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.plan().map(PlanKeeping::sent_back), Some(2));
    assert_eq!(member.questions().len(), 1);
}

#[tokio::test]
async fn a_done_step_whose_check_fails_is_judged_mismatch_and_recorded_in_history() {
    let mut daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists absent.txt\n",
    )
    .unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Mismatch)
    );
}

#[tokio::test]
async fn a_done_step_with_a_passing_check_is_judged_confirmed() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
}

#[tokio::test]
async fn a_refused_gate_check_is_judged_declared_and_recorded_in_history() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: make gate\n",
    )
    .unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Declared)
    );
}

#[tokio::test]
async fn unjudged_intervening_plan_causes_changed_step_to_be_rejudged() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n[ ] 1.2 Later\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    daemon
        .state_where("sent back once", |state| {
            state
                .find_task(&task(1))
                .and_then(|found| found.member.plan())
                .is_some_and(|keeping| keeping.sent_back() >= 1)
        })
        .await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    let step_id = "1.1".parse::<ItemId>().unwrap();
    assert_eq!(
        judgements.get(&step_id).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );

    let plan2_text =
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists missing.txt\n[-] 1.2 Later\n    Dropped.\n";
    {
        use agent_runtime::history::Store;
        let mut db = agent_runtime::history::HistoryDb::open(
            &daemon.home.history_file(),
            agent_core::history::MaxTextBytes::default(),
        )
        .unwrap();
        db.write_plans(&[agent_runtime::history::PlanRecord {
            task: task(1),
            team: team_id(1),
            run: Some("run-20261121-010000-abcd".parse().unwrap()),
            at: agent_core::state::Timestamp::now(&daemon.clock).unwrap(),
            read: agent_runtime::history::PlanRead::Version {
                text: plan2_text.to_owned(),
            },
        }])
        .unwrap();
    }

    std::fs::write(dir.join(".agent/plan.md"), plan2_text).unwrap();

    daemon.start().await;
    // The re-judged step is a mismatch of a run that ends DONE: with the bounce spent, the human is asked.
    wait_for_questions(&daemon).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let latest = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        latest.get(&step_id).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Mismatch),
        "changed step must be re-judged as Mismatch against missing.txt, not left as Confirmed"
    );
}

#[tokio::test]
async fn startup_cached_confirmations_do_not_show_stale_confirmed_for_edited_steps() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists edited.txt\n",
    )
    .unwrap();

    daemon.start().await;

    let mut connection = crate::client::Connection::connect(&daemon.home.socket_file())
        .await
        .unwrap();
    let reply = connection
        .request(
            agent_core::protocol::Client::Cli,
            agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
        )
        .await
        .unwrap();
    let (agent_core::protocol::Reply::Teams { teams } | agent_core::protocol::Reply::Overview { teams, .. }) = reply
    else {
        panic!("expected Reply::Teams, got {reply:?}");
    };
    let step = &teams[0].task_plans.get(&task(1)).unwrap()[0];
    assert_eq!(step.id, "1.1".parse::<ItemId>().unwrap());
    assert_eq!(
        step.state, None,
        "edited step must not show stale confirmed in startup status"
    );

    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn startup_cached_confirmations_preserved_for_deduplicated_plan_runs() {
    let mut daemon = with_bounces(&solo_state(), 2);
    let dir = daemon.scenario(1, "done-after-ok");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    let plan = "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n[ ] 1.2 Later\n";
    std::fs::write(dir.join(".agent/plan.md"), plan).unwrap();

    // Run 1: ok.txt does not exist, so 1.1 is judged Mismatch.
    daemon.start().await;
    daemon
        .state_where("sent back once", |state| {
            state
                .find_task(&task(1))
                .and_then(|f| f.member.plan())
                .is_some_and(|k| k.sent_back() >= 1)
        })
        .await;
    daemon.stop().await.unwrap();

    // Verify Run 1 kept the plan version in history
    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let versions = reader.plan_versions(&task(1)).unwrap();
    assert_eq!(versions.len(), 1);

    // Run 2: ok.txt is created, plan text is identical, 1.1 is confirmed.
    // Plan versions in history is deduplicated (still len == 1).
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;
    // The send-back counter can already hold its final value from the first start, and the task settles as waiting
    // for the human, never as Done: what this stop must observe is the run-specific persisted judgement. Wait for
    // history to record step 1.1 as Confirmed — only the run that judged the repaired check writes it, and it is
    // written when that run ends, before the settlement that follows.
    let step = "1.1".parse::<ItemId>().unwrap();
    let deadline = std::time::Instant::now() + PATIENCE;
    while !reader
        .latest_judgements(&task(1))
        .unwrap()
        .get(&step)
        .is_some_and(|record| record.state == agent_core::plan::judge::JudgementState::Confirmed)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the run that confirms step 1.1 after ok.txt appears"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    daemon.stop().await.unwrap();

    let versions = reader.plan_versions(&task(1)).unwrap();
    assert_eq!(versions.len(), 1, "history must deduplicate identical plan versions");
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    let j = judgements.get(&"1.1".parse::<ItemId>().unwrap()).unwrap();
    assert_eq!(j.state, agent_core::plan::judge::JudgementState::Confirmed);

    // Daemon restart: snapshot recovery must find the confirmation carried forward
    daemon.start().await;

    let mut connection = crate::client::Connection::connect(&daemon.home.socket_file())
        .await
        .unwrap();
    let reply = connection
        .request(
            agent_core::protocol::Client::Cli,
            agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
        )
        .await
        .unwrap();
    let (agent_core::protocol::Reply::Teams { teams } | agent_core::protocol::Reply::Overview { teams, .. }) = reply
    else {
        panic!("expected Reply::Teams, got {reply:?}");
    };
    let step = &teams[0].task_plans.get(&task(1)).unwrap()[0];
    assert_eq!(step.id, "1.1".parse::<ItemId>().unwrap());
    assert_eq!(
        step.state,
        Some(agent_core::plan::judge::JudgementState::Confirmed),
        "valid confirmation must be preserved across restart even when plan version was deduplicated"
    );

    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn plan_steps_are_judged_when_run_fails() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "fail");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Failed).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
}

#[tokio::test]
async fn plan_steps_are_judged_when_run_sleeps() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "sleep");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
}

#[tokio::test]
async fn plan_steps_are_judged_when_run_asks_questions() {
    use std::num::NonZeroU32;

    let mut daemon = with_bounces(&solo_state(), 1);
    let questions_file = daemon.home.questions_file(&task(1), NonZeroU32::new(1).unwrap());
    let dir = daemon.scenario(1, &format!("questions {}", questions_file.display()));
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Questions).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.get(&"1.1".parse::<ItemId>().unwrap()).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
}

#[tokio::test]
async fn judgement_write_failure_keeps_the_settlement_and_cached_steps() {
    use agent_runtime::history::{History, RecordKind, Setup, Store, TracingReporter, WriteError};
    struct Failing;
    impl Store for Failing {
        fn write(&mut self, _: &[agent_core::history::Record]) -> Result<(), WriteError> {
            Ok(())
        }
        fn write_judgements(&mut self, _: &[agent_runtime::history::JudgementRecord]) -> Result<(), WriteError> {
            Err(WriteError::Rule {
                record: RecordKind::Judgement,
                rule: "injected judgement failure",
            })
        }
    }
    let daemon = with_bounces(&solo_state(), 1);
    finished_plan(&daemon);
    let started = History::start(
        Failing,
        Setup {
            reporter: std::sync::Arc::new(TracingReporter),
            capacity: agent_runtime::history::DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let (owner, _sinks) = daemon.bare_owner().await;
    let mut owner = owner.with_history(started.history.clone());
    let located = owner.locate(&task(1)).unwrap();
    let end = agent_core::state::RunEnd::Reported {
        outcome: agent_core::status::RunOutcome::Done,
    };
    let result = owner
        .plan_at_end(
            &task(1),
            &located,
            &end,
            crate::daemon::settle::Settled::Status(TaskStatus::Done),
        )
        .await;
    assert!(
        matches!(
            result,
            Ok((crate::daemon::settle::Settled::Status(TaskStatus::Done), _))
        ),
        "{result:?}"
    );
    assert_eq!(
        owner.task_plans.get(&task(1)).unwrap()[0].state,
        Some(agent_core::plan::judge::JudgementState::Declared)
    );
    assert_eq!(started.history.health().lost, 1);
    started.writer.close().unwrap();
}

#[tokio::test]
async fn stopped_run_keeps_its_cached_plan_steps() {
    use agent_core::protocol::{Client, Command, ControlRequest, StatusRequest, Target};
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "hang");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), FINISHED).unwrap();
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    let mut connection = crate::client::Connection::connect(&daemon.home.socket_file())
        .await
        .unwrap();
    connection
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(task(1)),
            }),
        )
        .await
        .unwrap();
    daemon.status_is(1, TaskStatus::Stopped).await;
    let reply = connection
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let (agent_core::protocol::Reply::Teams { teams } | agent_core::protocol::Reply::Overview { teams, .. }) = reply
    else {
        panic!("{reply:?}")
    };
    assert_eq!(teams[0].task_plans.get(&task(1)).map(Vec::len), Some(2));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn reopened_step_is_checked_again_when_marked_done() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists ok.txt\n[ ] 1.2 Later\n",
    )
    .unwrap();
    std::fs::write(dir.join("ok.txt"), "content").unwrap();
    daemon.start().await;

    daemon
        .state_where("sent back once", |state| {
            state
                .find_task(&task(1))
                .and_then(|found| found.member.plan())
                .is_some_and(|keeping| keeping.sent_back() >= 1)
        })
        .await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    let step_id = "1.1".parse::<ItemId>().unwrap();
    assert_eq!(
        judgements.get(&step_id).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );

    let plan2_text = "Finish it.\n\n1. Work\n[>] 1.1 Do it\n    Check: exists ok.txt\n[-] 1.2 Later\n    Dropped.\n";
    {
        use agent_runtime::history::Store;
        let mut db = agent_runtime::history::HistoryDb::open(
            &daemon.home.history_file(),
            agent_core::history::MaxTextBytes::default(),
        )
        .unwrap();
        db.write_plans(&[agent_runtime::history::PlanRecord {
            task: task(1),
            team: team_id(1),
            run: Some("run-20261121-010000-abcd".parse().unwrap()),
            at: agent_core::state::Timestamp::now(&daemon.clock).unwrap(),
            read: agent_runtime::history::PlanRead::Version {
                text: plan2_text.to_owned(),
            },
        }])
        .unwrap();
    }

    std::fs::remove_file(dir.join("ok.txt")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), plan2_text.replace("[>]", "[x]")).unwrap();

    daemon.start().await;
    // The re-judged step is a mismatch of a run that ends DONE: with the bounce spent, the human is asked.
    wait_for_questions(&daemon).await;
    daemon.stop().await.unwrap();

    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let latest = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        latest.get(&step_id).map(|j| j.state),
        Some(agent_core::plan::judge::JudgementState::Mismatch),
        "changed step must be re-judged as Mismatch against missing.txt, not left as Confirmed"
    );
}

#[test]
fn edited_recovered_steps_discard_every_previous_judgement_state() {
    use agent_core::plan::{judge::JudgementState, parse_plan};
    use agent_runtime::history::JudgementRecord;
    use std::collections::HashMap;

    let old = parse_plan(FINISHED).unwrap();
    let changed = parse_plan(&FINISHED.replace("Do it", "Write the guide")).unwrap();
    let id = "1.1".parse::<ItemId>().unwrap();
    let run = "run-20261121-010000-abcd".parse::<agent_core::ids::RunId>().unwrap();
    let earlier = HashMap::from([(run.clone(), old.clone())]);
    for state in [
        JudgementState::Confirmed,
        JudgementState::Declared,
        JudgementState::Mismatch,
    ] {
        let judgements = HashMap::from([(
            id,
            JudgementRecord {
                task: task(1),
                team: team_id(1),
                run: Some(run.clone()),
                at: at(0),
                step: id,
                state,
                check: None,
                reason: Some("previous reason".to_owned()),
                duration_ms: 0,
                output_path: None,
            },
        )]);
        let summary = super::step_summary(changed.step(&id).unwrap(), &judgements, &earlier);
        assert_eq!((summary.state, summary.reason), (None, None), "{state:?}");
    }
}

/// The judgement the history kept for step `id` of task 1, read after the daemon stopped.
fn judgement_of(daemon: &Daemon, id: &str) -> agent_runtime::history::JudgementRecord {
    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    reader
        .latest_judgements(&task(1))
        .unwrap()
        .get(&id.parse::<ItemId>().unwrap())
        .cloned()
        .expect("a judgement of the step")
}

#[tokio::test]
async fn a_done_step_with_a_passing_command_is_judged_confirmed_by_the_daemon() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'exit 0'\n"),
    )
    .await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_done_step_with_a_failing_command_is_judged_mismatch_and_keeps_its_output() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'echo failing-output; exit 3'\n"),
    )
    .await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let judgement = judgement_of(&daemon, "1.1");
    assert_eq!(judgement.state, agent_core::plan::judge::JudgementState::Mismatch);
    assert!(
        judgement.reason.as_deref().unwrap_or_default().contains("exit code 3"),
        "reason: {:?}",
        judgement.reason
    );
    let output = judgement.output_path.expect("the output of the check is kept");
    let text = std::fs::read_to_string(daemon.home.task_dir(&task(1)).join(output)).unwrap();
    assert!(text.contains("failing-output"), "output: {text:?}");
}

#[tokio::test]
async fn a_command_check_runs_with_the_environment_of_the_agents_runs() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'test -n \"$AGENTS_HOME\"'\n"),
    )
    .await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_command_check_is_declared_when_check_seconds_is_zero() {
    let state = solo_state();
    let daemon = with_bounces(&state, 0);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 0\ncheck_seconds = 0\n"),
    )
    .unwrap();
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'exit 3'\n",
    )
    .unwrap();
    let mut daemon = daemon;
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Declared
    );
}

/// The process group ID a command check of task 1 wrote to its record while it runs, once it is written.
async fn running_check_group(daemon: &Daemon) -> u32 {
    let record = daemon.home.task_dir(&task(1)).join("checks").join("running");
    loop {
        if let Ok(text) = std::fs::read_to_string(&record)
            && let Some((pid, ticks)) = text.trim().split_once(' ')
            && let (Ok(pid), Ok(_ticks)) = (pid.parse::<u32>(), ticks.parse::<u64>())
        {
            return pid;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Reads the kernel start time (field 22 of `/proc/<pid>/stat`) of `pid`.
fn process_start_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let (_, rest) = stat.rsplit_once(')').unwrap();
    rest.split_whitespace().nth(19).unwrap().parse().unwrap()
}

/// Whether a process with `pid` is still running (a zombie has ended).
fn process_runs(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.trim_start().chars().next());
        !matches!(state, None | Some('Z' | 'X'))
    })
}

#[tokio::test]
async fn the_daemon_answers_while_a_command_check_runs() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'sleep 5'\n"),
    )
    .await;

    let _group = running_check_group(&daemon).await;
    let answered = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        daemon.ask(agent_core::protocol::Command::Status(
            agent_core::protocol::StatusRequest { target: None },
        )),
    )
    .await;
    assert!(answered.is_ok(), "the status request is answered while the check runs");

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();
    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn stopping_the_daemon_ends_the_process_of_a_running_command_check() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: /bin/sh -c 'sleep 300'\n"),
    )
    .await;

    let group = running_check_group(&daemon).await;
    daemon.stop().await.unwrap();

    assert!(
        !process_runs(group),
        "the process group {group} of the check outlives the daemon"
    );
    assert!(!daemon.home.task_dir(&task(1)).join("checks").join("running").exists());
}

#[tokio::test]
async fn a_daemon_that_starts_ends_the_check_a_dead_daemon_left_running() {
    use std::os::unix::process::CommandExt;

    let mut daemon = with_bounces(&solo_state(), 0);
    let checks = daemon.home.task_dir(&task(1)).join("checks");
    std::fs::create_dir_all(&checks).unwrap();
    let mut left = std::process::Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .unwrap();
    let ticks = process_start_ticks(left.id());
    std::fs::write(checks.join("running"), format!("{} {ticks}\n", left.id())).unwrap();

    daemon.start().await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while process_runs(left.id()) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let ended = !process_runs(left.id());
    left.wait().unwrap();
    daemon.stop().await.unwrap();

    assert!(
        ended,
        "the check a dead daemon left running is ended when the daemon starts"
    );
    assert!(!checks.join("running").exists());
}

#[tokio::test]
async fn two_done_steps_with_commands_run_one_after_the_other_and_show_in_status() {
    let mut daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    let plan = concat!(
        "Finish it.\n\n1. Work\n",
        "[x] 1.1 First\n",
        "    Check: /bin/sh -c 'echo first > order.txt'\n",
        "[x] 1.2 Second\n",
        "    Check: /bin/sh -c 'test \"$(cat order.txt)\" = first && echo second >> order.txt && seq 1 60 && exit 7'\n",
    );
    std::fs::write(dir.join(".agent/plan.md"), plan).unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    let reply = daemon
        .ask(agent_core::protocol::Command::Status(
            agent_core::protocol::StatusRequest { target: None },
        ))
        .await
        .unwrap();
    daemon.stop().await.unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join("order.txt")).unwrap(),
        "first\nsecond\n"
    );
    let (agent_core::protocol::Reply::Teams { teams } | agent_core::protocol::Reply::Overview { teams, .. }) = reply
    else {
        panic!("expected Teams or Overview reply, got {reply:?}");
    };
    let summaries = teams[0].task_plans.get(&task(1)).expect("task plan summaries");
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        summaries[0].state,
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
    assert_eq!(summaries[0].reason, None);
    assert_eq!(
        summaries[1].state,
        Some(agent_core::plan::judge::JudgementState::Mismatch)
    );
    assert!(
        summaries[1]
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("exit code 7"),
        "reason: {:?}",
        summaries[1].reason
    );
    let second = judgement_of(&daemon, "1.2");
    let output_rel = second.output_path.expect("output path of failing second step");
    let output_text = std::fs::read_to_string(daemon.home.task_dir(&task(1)).join(output_rel)).unwrap();
    assert_eq!(output_text.lines().count(), 60);
}

#[tokio::test]
async fn a_command_check_past_its_limit_is_ended_with_its_children() {
    let daemon = with_bounces(&solo_state(), 0);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[plan]\nbounces = 0\n", "[plan]\nbounces = 0\ncheck_seconds = 1\n"),
    )
    .unwrap();
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Slow\n    Check: /bin/sh -c 'sleep 300 & echo $! > child.pid; sleep 300'\n",
    )
    .unwrap();
    let mut daemon = daemon;
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let child_pid: u32 = std::fs::read_to_string(dir.join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        !process_runs(child_pid),
        "child process {child_pid} of timed-out check must be killed"
    );
    let judgement = judgement_of(&daemon, "1.1");
    assert_eq!(judgement.state, agent_core::plan::judge::JudgementState::Mismatch);
    assert_eq!(judgement.reason.as_deref(), Some("ran past 1 seconds"));
}

#[tokio::test]
async fn a_command_check_sees_the_agents_secret_and_not_the_daemons_secrets() {
    use std::os::unix::fs::PermissionsExt;

    let mut daemon = with_bounces(&solo_state(), 0);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        format!("{config}\n[build]\nincremental = true\n"),
    )
    .unwrap();
    std::fs::write(
        daemon.home.secrets_file(),
        "github_token = \"ghp_example\"\n[agent_env]\nAGENT_ONLY = \"for-the-agent\"\n[notification_env]\nDAEMON_ONLY = \"for-notifications\"\n",
    )
    .unwrap();
    std::fs::set_permissions(daemon.home.secrets_file(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Secrets\n    Check: /bin/sh -c 'test \"$AGENT_ONLY\" = \"for-the-agent\" && test -z \"$DAEMON_ONLY\" && test -z \"$GITHUB_TOKEN\" && test \"$CARGO_INCREMENTAL\" = \"1\"'\n",
    )
    .unwrap();
    daemon.start().await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_daemon_restarted_while_a_command_check_runs_reruns_the_check_and_settles_the_run_once() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Recoverable\n    Check: /bin/sh -c 'if [ ! -f restarted.txt ]; then sleep 300; fi; touch attempt-2.txt'\n",
    )
    .unwrap();
    daemon.start().await;

    let group = running_check_group(&daemon).await;
    daemon.stop().await.unwrap();

    assert!(
        !process_runs(group),
        "the check process group {group} must not outlive daemon.stop()"
    );
    let saved = State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
    assert!(
        saved.find_task(&task(1)).unwrap().member.plan_running_check().is_some(),
        "state.json records the unsettled finished run while its check is running"
    );

    std::fs::write(dir.join("restarted.txt"), "yes").unwrap();
    daemon.start().await;
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert!(dir.join("attempt-2.txt").exists(), "the check ran again on restart");
    let member = state.find_task(&task(1)).unwrap().member;
    assert_eq!(
        member.runs().len(),
        1,
        "the finished run is settled once without starting a second agent CLI run"
    );
    assert_eq!(member.plan_running_check(), None);
    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_restart_after_the_judgement_is_recorded_settles_the_run_without_rerunning_the_check() {
    use agent_runtime::history::Store;

    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Judged\n    Check: /bin/sh -c 'echo ran >> count.txt; sleep 300'\n",
    )
    .unwrap();
    daemon.start().await;

    let _group = running_check_group(&daemon).await;
    daemon.stop().await.unwrap();

    let saved = State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
    let run_id = saved
        .find_task(&task(1))
        .unwrap()
        .member
        .current_run()
        .expect("the finished run stays open in state.json until its plan settlement commits")
        .id()
        .clone();
    {
        let mut db = agent_runtime::history::HistoryDb::open(
            &daemon.home.history_file(),
            agent_core::history::MaxTextBytes::default(),
        )
        .unwrap();
        db.write_judgements(&[agent_runtime::history::JudgementRecord {
            task: task(1),
            team: team_id(0),
            run: Some(run_id),
            at: at(0),
            step: "1.1".parse().unwrap(),
            state: agent_core::plan::judge::JudgementState::Confirmed,
            check: Some("/bin/sh -c 'echo ran >> count.txt; sleep 300'".to_owned()),
            reason: None,
            duration_ms: 12,
            output_path: None,
        }])
        .unwrap();
    }

    if dir.join("count.txt").exists() {
        std::fs::remove_file(dir.join("count.txt")).unwrap();
    }

    daemon.start().await;
    let settled_state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    assert!(
        !dir.join("count.txt").exists(),
        "a step already judged for the finished run must not be executed again on restart"
    );
    let member = settled_state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.runs().len(), 1);
    assert_eq!(member.plan_running_check(), None);
    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

/// Waits for `path` to hold a process ID and returns it.
async fn wait_for_pid_file(path: &std::path::Path) -> u32 {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            return pid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for pid file {}",
            path.display()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Ends process group `pid` if a test failure left it running.
fn cleanup_group_if_alive(pid: u32) {
    if process_runs(pid)
        && let Ok(raw) = i32::try_from(pid)
        && let Some(group) = rustix::process::Pid::from_raw(raw)
        && rustix::process::kill_process_group(group, rustix::process::Signal::KILL).is_err()
    {}
}

#[tokio::test]
async fn a_refused_gate_command_check_is_judged_declared_with_its_refused_reason() {
    let mut daemon = running_with_plan(
        &solo_state(),
        0,
        Some("Finish it.\n\n1. Work\n[x] 1.1 Refused\n    Check: agent gate\n"),
    )
    .await;

    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let judgement = judgement_of(&daemon, "1.1");
    assert_eq!(judgement.state, agent_core::plan::judge::JudgementState::Declared);
    assert_eq!(judgement.reason.as_deref(), Some("refused gate command"));
}

#[tokio::test]
async fn stopping_or_pausing_a_task_while_its_command_check_runs_ends_the_check_and_halts_the_task() {
    use agent_core::protocol::{Command, ControlRequest, Target};

    for (control, expected_status) in [
        (
            Command::Pause(ControlRequest {
                target: Target::Task(task(1)),
            }),
            TaskStatus::Paused,
        ),
        (
            Command::Stop(ControlRequest {
                target: Target::Task(task(1)),
            }),
            TaskStatus::Stopped,
        ),
    ] {
        let mut daemon = with_bounces(&solo_state(), 0);
        let dir = daemon.scenario(1, "done");
        std::fs::create_dir_all(dir.join(".agent")).unwrap();
        std::fs::write(
            dir.join(".agent/plan.md"),
            "Finish it.\n\n1. Work\n[x] 1.1 Long\n    Check: /bin/sh -c 'echo $$ > check.pid; sleep 300'\n",
        )
        .unwrap();
        daemon.start().await;

        let group = wait_for_pid_file(&dir.join("check.pid")).await;
        let reply = daemon.ask(control).await.unwrap();
        assert_eq!(reply, agent_core::protocol::Reply::Applied { tasks: vec![task(1)] });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while process_runs(group) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let check_still_running = process_runs(group);
        let saved = State::from_json(&std::fs::read_to_string(daemon.home.state_file()).unwrap()).unwrap();
        let member = saved.find_task(&task(1)).unwrap().member.clone();
        cleanup_group_if_alive(group);
        daemon.stop().await.unwrap();

        assert!(
            !check_still_running,
            "the running command check group {group} must be ended by stop/pause"
        );
        assert_eq!(member.status(), expected_status);
        assert_eq!(member.plan_running_check(), None);
        assert!(!daemon.home.task_dir(&task(1)).join("checks").join("running").exists());
    }
}

#[tokio::test]
async fn a_legacy_task_restarted_while_its_command_check_runs_settles_once_without_rerunning_cli() {
    let mut state = State::new();
    let crew = Crew::solo(Member::without_plan(task(1), normal("claude-1"))).unwrap();
    state
        .insert_team(Team::new(
            team_id(0),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            crew,
        ))
        .unwrap();
    let mut daemon = with_bounces(&state, 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Legacy\n    Check: /bin/sh -c 'echo $$ > check.pid; if [ ! -f restarted.txt ]; then sleep 300; fi; touch legacy-checked.txt'\n",
    )
    .unwrap();
    daemon.start().await;

    let group = wait_for_pid_file(&dir.join("check.pid")).await;
    cleanup_group_if_alive(group);
    daemon.stop().await.unwrap();

    std::fs::write(dir.join("restarted.txt"), "yes").unwrap();
    daemon.start().await;
    let settled = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let member = settled.find_task(&task(1)).unwrap().member;
    assert_eq!(member.plan(), None, "legacy member still has no PlanKeeping");
    assert_eq!(
        member.runs().len(),
        1,
        "legacy member's finished run must settle once without rerunning the CLI"
    );
    assert_eq!(member.plan_running_check(), None);
    assert!(dir.join("legacy-checked.txt").exists());
    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_restart_during_a_second_command_check_keeps_the_first_judgement_and_does_not_rerun_it() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    let plan = concat!(
        "Finish it.\n\n1. Work\n",
        "[x] 1.1 First\n",
        "    Check: /bin/sh -c 'echo ran >> first.txt'\n",
        "[x] 1.2 Second\n",
        "    Check: /bin/sh -c 'echo $$ > second.pid; if [ ! -f restarted.txt ]; then sleep 300; fi; echo ran >> second.txt'\n",
    );
    std::fs::write(dir.join(".agent/plan.md"), plan).unwrap();
    daemon.start().await;

    let group = wait_for_pid_file(&dir.join("second.pid")).await;
    cleanup_group_if_alive(group);
    daemon.stop().await.unwrap();

    std::fs::write(dir.join("restarted.txt"), "yes").unwrap();
    daemon.start().await;
    let _state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let first_runs = std::fs::read_to_string(dir.join("first.txt")).unwrap();
    assert_eq!(
        first_runs.lines().count(),
        1,
        "the first command check was already persisted before the restart and must not rerun"
    );
    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.step_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements.len(),
        2,
        "exactly two step judgement rows exist across the restart"
    );
    assert_eq!(
        judgements
            .iter()
            .filter(|row| row.step == "1.1".parse::<ItemId>().unwrap())
            .count(),
        1,
        "step 1.1 must have exactly one judgement row for the run"
    );
    assert_eq!(
        judgements
            .iter()
            .filter(|row| row.step == "1.2".parse::<ItemId>().unwrap())
            .count(),
        1,
        "step 1.2 must have exactly one judgement row for the run"
    );
    assert_eq!(
        reader.plan_versions(&task(1)).unwrap().len(),
        1,
        "exactly one plan version row exists across the restart"
    );
}

#[tokio::test]
async fn mixed_inline_and_command_checks_keep_inline_states_in_status_when_history_is_disabled() {
    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_core::state::{NewRun, RunEnd};
    use agent_core::status::RunOutcome;
    use agent_runtime::run::{FinishedRun, Progress};

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join("ok.txt"), "present").unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Exists\n    Check: exists ok.txt\n[x] 1.2 Command\n    Check: /bin/sh -c 'exit 0'\n",
    )
    .unwrap();

    let (mut owner, _sinks) = daemon.bare_owner().await;
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id,
                at: at(0),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    owner
        .parts
        .state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();

    owner
        .finished(
            &task(1),
            FinishedRun {
                classified: Classified {
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                    pause: None,
                },
                finish: RunFinish {
                    exit: ExitStatus::Code(0),
                    stopped_by: StoppedBy::Nobody,
                },
                progress: Progress::Made,
                used_tool: true,
                session: None,
                final_message: Some("STATUS: DONE".to_owned()),
            },
        )
        .await
        .unwrap();

    let joined = owner.checks.join_next_with_id().await.expect("command check batch");
    owner.check_batch_done(joined).await.unwrap();

    let summaries = owner.task_plans.get(&task(1)).expect("cached task plan summaries");
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        summaries[0].state,
        Some(agent_core::plan::judge::JudgementState::Confirmed),
        "inline check judgement must be preserved in cached status summaries when history is disabled"
    );
    assert_eq!(
        summaries[1].state,
        Some(agent_core::plan::judge::JudgementState::Confirmed)
    );
}

#[tokio::test]
async fn a_recovered_run_preserves_its_progress_and_pause_across_restart() {
    use agent_core::decide::outcome::{Pause, PauseReason};
    use agent_core::state::{NewRun, RunEnd, RunningPlanCheck};

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), FINISHED).unwrap();

    // Case 1: pause until timestamp is preserved as classified when the run finished, not recomputed from restart time.
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id.clone(),
                at: at(0),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.set_plan_running_check(Some(RunningPlanCheck {
            run: run_id,
            end: RunEnd::RateLimited,
            pause: Some(Pause {
                until: at(1200),
                reason: PauseReason::RateLimit,
                evidence: "HTTP 429 Too Many Requests".to_owned(),
            }),
            made_progress: true,
            final_message: None,
            ended_at: None,
        }));
    }
    owner.recover_plan_checks().await.unwrap();
    assert_eq!(
        owner.wakes.get(&task(1)),
        Some(&at(1200)),
        "recovered pause wake time must match the run's classified pause"
    );
    drop(owner);

    // Case 2: a hung run that made no progress preserves prior hang strikes from recovery and increments to 2.
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let session = Some("sess-1".parse().unwrap());
    let first_hang_id: agent_core::ids::RunId = "run-20261122-060200-hang".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: first_hang_id,
                at: at(10),
                session: session.clone(),
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.end_run(at(20), RunEnd::Hung).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    let run_id: agent_core::ids::RunId = "run-20261122-060500-efgh".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id.clone(),
                at: at(30),
                session,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.set_plan_running_check(Some(RunningPlanCheck {
            run: run_id,
            end: RunEnd::Hung,
            pause: None,
            made_progress: false,
            final_message: None,
            ended_at: None,
        }));
    }
    owner.recover().await.unwrap();
    assert_eq!(
        owner.hangs.get(&task(1)),
        Some(&2),
        "a recovered hang without progress must restore prior hang strike (1) and increment to 2"
    );
}

#[tokio::test]
async fn deferred_and_reviewer_target_cleanup_waits_until_parked_command_checks_finish() {
    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_core::state::{NewRun, RunEnd};
    use agent_core::status::RunOutcome;
    use agent_runtime::run::{FinishedRun, Progress};

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::write(dir.join("target/built.txt"), "artifact").unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Build\n    Check: /bin/sh -c 'test -f target/built.txt'\n",
    )
    .unwrap();

    let (mut owner, _sinks) = daemon.bare_owner().await;
    let round_id = agent_core::ids::RoundId::derive(&team_id(0), 1);
    owner
        .parts
        .state
        .team_mut(&team_id(0))
        .unwrap()
        .open_round(round_id.clone(), Some("1.1".parse().unwrap()), at(0))
        .unwrap();
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id,
                at: at(0),
                session: None,
                rounds: vec![round_id.clone()],
            },
        )
        .unwrap();
    owner
        .parts
        .state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();

    owner
        .finished(
            &task(1),
            FinishedRun {
                classified: Classified {
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                    pause: None,
                },
                finish: RunFinish {
                    exit: ExitStatus::Code(0),
                    stopped_by: StoppedBy::Nobody,
                },
                progress: Progress::Made,
                used_tool: true,
                session: None,
                final_message: Some("STATUS: DONE".to_owned()),
            },
        )
        .await
        .unwrap();
    assert!(owner.parked.contains_key(&task(1)));

    // Closing a round while the task's command check is parked must defer target removal until the check finishes.
    owner.cleanup_round_targets(&team_id(0), &round_id);
    assert!(
        owner.deferred_targets.contains_key(&task(1)),
        "round target cleanup must defer while a task's command check is parked"
    );

    let joined = owner.checks.join_next_with_id().await.expect("command check batch");
    owner.check_batch_done(joined).await.unwrap();
    assert!(
        !owner.deferred_targets.contains_key(&task(1)),
        "deferred target cleanup is released once the parked command check settles"
    );
}

#[tokio::test]
async fn unreadable_history_during_recovery_does_not_rerun_command_checks() {
    use agent_core::state::{NewRun, RunEnd, RunningPlanCheck};
    use agent_core::status::RunOutcome;

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Check\n    Check: /bin/sh -c 'touch reran.txt'\n",
    )
    .unwrap();
    // Write a corrupt history.db file so Reader::open fails with a read error.
    std::fs::write(daemon.home.history_file(), b"corrupt-sqlite-header").unwrap();

    let (mut owner, _sinks) = daemon.bare_owner().await;
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id.clone(),
                at: at(0),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.set_plan_running_check(Some(RunningPlanCheck {
            run: run_id,
            end: RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            pause: None,
            made_progress: true,
            final_message: Some("STATUS: DONE".to_owned()),
            ended_at: None,
        }));
    }

    let result = owner.recover_plan_checks().await;
    for (_id, _task) in owner.checking.drain() {}
    owner.checks.abort_all();
    assert!(
        result.is_err() || owner.parked.is_empty(),
        "recovery must not spawn command checks when an existing history.db cannot be read"
    );
    assert!(
        !dir.join("reran.txt").exists(),
        "command check must not rerun when existing judgements in history.db are unreadable"
    );
}

#[tokio::test]
async fn check_environment_failure_and_leftover_check_errors_are_reported() {
    use super::super::Located;
    use agent_core::decide::plan::PlanRead;
    use agent_core::kinds::Role;
    use agent_core::plan::parse_plan;

    let daemon = with_bounces(&solo_state(), 0);
    let checks_dir = daemon.home.task_dir(&task(1)).join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    std::fs::write(checks_dir.join("running"), "not-a-pid\n").unwrap();

    let (mut owner, _sinks) = daemon.bare_owner().await;
    let leftover = owner.end_leftover_checks();
    assert!(
        leftover.is_err(),
        "end_leftover_checks must propagate an invalid or unkillable running check record"
    );
    std::fs::remove_file(checks_dir.join("running")).unwrap();

    let bad_located = Located {
        team: team_id(0),
        role: Role::Solo,
        agent: "invalid agent name".to_owned(),
    };
    let plan = parse_plan("Finish it.\n\n1. Work\n[x] 1.1 Cmd\n    Check: /bin/sh -c 'exit 0'\n").unwrap();
    let found = super::Found {
        read: PlanRead::Parsed(plan),
        text: None,
    };
    let worktree = owner.worktree_of(&task(1), &bad_located).unwrap();
    let judged = owner
        .judge_steps_at_end(
            &task(1),
            &bad_located,
            None,
            &found,
            &worktree,
            (None, &std::collections::HashMap::new()),
        )
        .await
        .unwrap();
    let super::Judged::Known(records) = judged else {
        panic!("expected Known judgements when check_environment fails");
    };
    let step_id: ItemId = "1.1".parse().unwrap();
    let record = records.get(&step_id).expect("mismatch record for step 1.1");
    assert_eq!(record.state, agent_core::plan::judge::JudgementState::Mismatch);
    assert!(
        record
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("cannot prepare the environment of the check: invalid agent name \"invalid agent name\""),
        "reason: {:?}",
        record.reason
    );
}

#[tokio::test]
async fn completed_check_batch_after_halt_parked_check_is_ignored_without_unknown_check_error() {
    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_core::state::{NewRun, RunEnd};
    use agent_core::status::RunOutcome;
    use agent_runtime::run::{FinishedRun, Progress};

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Quick\n    Check: /bin/sh -c 'exit 0'\n",
    )
    .unwrap();

    let (mut owner, _sinks) = daemon.bare_owner().await;
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id,
                at: at(0),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    owner
        .parts
        .state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();

    owner
        .finished(
            &task(1),
            FinishedRun {
                classified: Classified {
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                    pause: None,
                },
                finish: RunFinish {
                    exit: ExitStatus::Code(0),
                    stopped_by: StoppedBy::Nobody,
                },
                progress: Progress::Made,
                used_tool: true,
                session: None,
                final_message: Some("STATUS: DONE".to_owned()),
            },
        )
        .await
        .unwrap();
    let joined = owner.checks.join_next_with_id().await.expect("completed check batch");
    owner.halt_parked_check(&task(1)).await.unwrap();
    let res = owner.check_batch_done(joined).await;
    assert!(
        res.is_ok(),
        "check_batch_done must ignore a batch that completed right before halt_parked_check: {res:?}"
    );
    assert_eq!(
        owner.parts.state.find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Stopped
    );
}

#[tokio::test]
async fn recovered_run_preserves_original_ended_at_and_judgement_read_error_preserves_source() {
    use std::error::Error as _;

    use agent_core::state::{NewRun, RunEnd, RunState, RunningPlanCheck};
    use agent_core::status::RunOutcome;

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), FINISHED).unwrap();

    // Part 1: typed source on PlanJudgementsRead.
    std::fs::write(daemon.home.history_file(), b"corrupt-sqlite-header").unwrap();
    let (owner, _sinks) = daemon.bare_owner().await;
    let read_err = owner
        .latest_judgements(&task(1))
        .await
        .expect_err("corrupt history.db must fail");
    assert!(
        read_err.source().is_some(),
        "PlanJudgementsRead must preserve the typed underlying source error: {read_err:?}"
    );
    drop(owner);
    std::fs::remove_file(daemon.home.history_file()).unwrap();

    // Part 2: original ended_at timestamp in RunningPlanCheck is preserved when settling after recovery.
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let recovered_run_id: agent_core::ids::RunId = "run-20261122-060000-end1".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: recovered_run_id.clone(),
                at: at(10),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.set_plan_running_check(Some(RunningPlanCheck {
            run: recovered_run_id,
            end: RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
            pause: None,
            made_progress: true,
            final_message: Some("STATUS: DONE".to_owned()),
            ended_at: Some(at(123)),
        }));
    }
    owner.recover_plan_checks().await.unwrap();
    let ended_state = owner
        .parts
        .state
        .find_task(&task(1))
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .state()
        .clone();
    assert_eq!(
        ended_state,
        RunState::Ended {
            at: at(123),
            end: RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        },
        "settled recovered run must record its original ended_at timestamp"
    );
}

#[tokio::test]
async fn command_check_batch_propagates_clock_error_and_preserves_typed_judgement_read_source() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_core::ids::Clock;
    use agent_core::state::{NewRun, RunEnd};
    use agent_core::status::RunOutcome;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::run::{FinishedRun, Progress};

    struct FailOnThirdRead(AtomicUsize);
    impl Clock for FailOnThirdRead {
        fn now(&self) -> time::UtcDateTime {
            let call = self.0.fetch_add(1, Ordering::SeqCst);
            if call < 2 {
                at(210).as_utc()
            } else {
                time::UtcDateTime::MIN
            }
        }
    }

    let daemon = with_bounces(&solo_state(), 0);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Check\n    Check: /bin/sh -c 'exit 0'\n",
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let run_id: agent_core::ids::RunId = "run-20261122-060500-clk1".parse().unwrap();
    owner
        .parts
        .state
        .begin_run(
            &task(1),
            NewRun {
                id: run_id,
                at: at(200),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
    {
        let (_, mut member) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
    }
    owner.parts.clock = SharedClock::new(FailOnThirdRead(AtomicUsize::new(0)));
    owner
        .finished(
            &task(1),
            FinishedRun {
                classified: Classified {
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done,
                    },
                    pause: None,
                },
                finish: RunFinish {
                    exit: ExitStatus::Code(0),
                    stopped_by: StoppedBy::Nobody,
                },
                progress: Progress::Made,
                used_tool: true,
                session: None,
                final_message: Some("STATUS: DONE".to_owned()),
            },
        )
        .await
        .unwrap();
    let joined = owner.checks.join_next_with_id().await.expect("command check batch");
    let batch_res = owner.check_batch_done(joined).await;
    assert!(
        matches!(batch_res, Err(super::super::OwnerError::Clock(_))),
        "clock failure during command check judgement must propagate as OwnerError::Clock, got {batch_res:?}"
    );
}

#[tokio::test]
async fn async_recover_task_checks_cleans_up_group_on_blocking_pool() {
    use std::os::unix::process::CommandExt;

    let daemon = with_bounces(&solo_state(), 0);
    let checks_dir = daemon.home.task_dir(&task(1)).join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record_path = checks_dir.join("running");

    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("rec=\"$1\"; read -r s < \"/proc/$$/stat\" && set -- ${s#*)} && printf '%s %s\\n' \"$$\" \"${20}\" > \"$rec\" && exec sleep 30")
        .arg("sh")
        .arg(&record_path)
        .process_group(0)
        .spawn()
        .expect("spawn check group");
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::fs::read_to_string(&record_path)
        .unwrap_or_default()
        .trim()
        .is_empty()
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let (owner, _sinks) = daemon.bare_owner().await;
    let recovered = owner.recover_task_checks(&task(1)).await.expect("recover_task_checks");
    if recovered.is_none() {
        drop(child.kill());
    }
    drop(child.wait());
    assert_eq!(
        recovered,
        Some(std::num::NonZeroU32::new(pid).unwrap()),
        "recover_task_checks must end the recorded check process group via spawn_blocking"
    );
}

#[tokio::test]
async fn async_recover_leftover_checks_ends_recorded_group_on_blocking_pool() {
    use std::os::unix::process::CommandExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let daemon = with_bounces(&solo_state(), 0);
    let checks_dir = daemon.home.task_dir(&task(1)).join("checks");
    std::fs::create_dir_all(&checks_dir).unwrap();
    let record_path = checks_dir.join("running");

    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exec sleep 30")
        .process_group(0)
        .spawn()
        .expect("spawn leftover check group");
    let pid = child.id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read child stat");
    let (_, after_comm) = stat.rsplit_once(')').expect("child stat parens");
    let ticks = after_comm.split_whitespace().nth(19).expect("child starttime ticks");
    let record_line = format!("{pid} {ticks}\n");

    let mkfifo = std::process::Command::new("mkfifo")
        .arg(&record_path)
        .status()
        .expect("create running FIFO");
    assert!(mkfifo.success(), "mkfifo failed: {mkfifo}");

    let (owner, _sinks) = daemon.bare_owner().await;

    let executor_progressed = Arc::new(AtomicBool::new(false));
    let failsafe_triggered = Arc::new(AtomicBool::new(false));
    let failsafe_flag = Arc::clone(&failsafe_triggered);
    let failsafe_fifo = record_path.clone();
    let failsafe_line = record_line.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let failsafe = std::thread::spawn(move || {
        if matches!(
            done_rx.recv_timeout(std::time::Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            failsafe_flag.store(true, Ordering::SeqCst);
            drop(std::fs::write(&failsafe_fifo, failsafe_line));
        }
    });

    let (recover_res, ()) = tokio::join!(owner.recover_leftover_checks(), async {
        if !failsafe_triggered.load(Ordering::SeqCst) {
            executor_progressed.store(true, Ordering::SeqCst);
            drop(std::fs::write(&record_path, &record_line));
        }
    });
    drop(done_tx);
    failsafe.join().expect("join FIFO hang-failsafe thread");
    recover_res.expect("recover_leftover_checks");
    let record_removed = !record_path.exists();
    drop(child.kill());
    drop(child.wait());
    assert!(
        executor_progressed.load(Ordering::SeqCst),
        "recover_leftover_checks blocked the async executor thread instead of offloading to spawn_blocking"
    );
    assert!(
        record_removed,
        "recover_leftover_checks must end leftover check process groups and remove checks/running on the blocking pool"
    );
}

/// A plan of task 1 whose only step is done and checks `exists absent.txt`: its check fails until the file is there.
const MISSING_FILE_PLAN: &str = "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: exists absent.txt\n";

/// Waits until the state of task 1 has `count` send-backs of its plan, on the daemon that runs.
async fn sent_back_count(daemon: &Daemon, count: u32) {
    daemon
        .state_where(&format!("{count} send-backs of the plan"), |state| {
            state
                .find_task(&task(1))
                .and_then(|found| found.member.plan().map(|keeping| keeping.sent_back() == count))
                .unwrap_or(false)
        })
        .await;
}

#[tokio::test]
async fn a_mismatch_of_a_done_run_sends_it_back_and_the_repaired_step_settles() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done-on-resume-after absent.txt");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), MISSING_FILE_PLAN).unwrap();
    daemon.start().await;

    sent_back_count(&daemon, 1).await;
    // The agent repairs what the check looks for before its next run ends.
    std::fs::write(dir.join("absent.txt"), "now here").unwrap();
    let state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();

    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("the check of step 1.1 (`exists absent.txt`) failed"),
        "{prompt}"
    );
    let keeping = state.find_task(&task(1)).unwrap().member.plan().unwrap().clone();
    assert_eq!(keeping.sent_back(), 1);
    assert_eq!(
        judgement_of(&daemon, "1.1").state,
        agent_core::plan::judge::JudgementState::Confirmed
    );
}

#[tokio::test]
async fn a_mismatch_that_stays_uses_the_bounces_and_asks_the_human_about_the_step_and_its_check() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), MISSING_FILE_PLAN).unwrap();
    daemon.start().await;

    wait_for_questions(&daemon).await;
    let text = std::fs::read_to_string(daemon.questions_file(1, 1)).unwrap();
    daemon.stop().await.unwrap();

    assert!(text.contains("1.1") && text.contains("exists absent.txt"), "{text}");
    let prompt = daemon.recorded(1, ".fake-args");
    assert!(
        prompt.contains("the check of step 1.1 (`exists absent.txt`) failed"),
        "{prompt}"
    );
    let reader = agent_runtime::history::Reader::open(&daemon.home.history_file()).unwrap();
    assert_eq!(
        reader.latest_judgements(&task(1)).unwrap()[&"1.1".parse::<ItemId>().unwrap()].state,
        agent_core::plan::judge::JudgementState::Mismatch
    );
}

#[tokio::test]
async fn a_refused_gate_check_is_told_to_the_next_run_once_and_not_after_a_restart_of_the_daemon() {
    let plan = "Finish it.\n\n1. Work\n[x] 1.1 Do it\n    Check: make gate\n[ ] 1.2 Later\n";
    let mut daemon = running_with_plan(&solo_state(), 1, Some(plan)).await;

    // The next run takes the notice when it starts. Wait until that run has told it and has ended.
    sent_back_count(&daemon, 1).await;
    wait_for_run_with_notice(&daemon).await;
    daemon
        .state_where("the second run to end", |state| {
            state.find_task(&task(1)).is_some_and(|found| {
                let runs = found.member.runs();
                runs.len() >= 2 && matches!(runs[runs.len() - 1].state(), agent_core::state::RunState::Ended { .. })
            })
        })
        .await;
    daemon.stop().await.unwrap();

    // After a restart the state holds no notice left to tell, and the run that had it told it exactly once.
    daemon.start().await;
    daemon.stop().await.unwrap();
    let prompt = daemon.recorded(1, ".fake-args");
    assert_eq!(prompt.matches("## Step 1.1 check was refused").count(), 1, "{prompt}");
    let state = daemon.state();
    let keeping = state.find_task(&task(1)).unwrap().member.plan().unwrap();
    assert_eq!(keeping.refused_gate(), &[]);
    assert_eq!(
        judgement_of(&daemon, "1.1").reason.as_deref(),
        Some("refused gate command")
    );
}

/// Waits until the prompt of the run of task 1 that was last started holds the refused gate notice.
async fn wait_for_run_with_notice(daemon: &Daemon) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !daemon
        .recorded(1, ".fake-args")
        .contains("## Step 1.1 check was refused")
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        daemon
            .recorded(1, ".fake-args")
            .contains("## Step 1.1 check was refused"),
        "timed out waiting for refused gate notice in prompt"
    );
}

/// Waits until the questions file of task 1, number 1, is written.
async fn wait_for_questions(daemon: &Daemon) {
    let questions = daemon.questions_file(1, 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !questions.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(questions.exists(), "timed out waiting for {}", questions.display());
}

#[test]
fn refused_gates_lists_only_the_refused_commands_in_order() {
    use agent_core::plan::judge::JudgementState;
    use agent_runtime::plan::check::JudgementResult;

    let judged = |state, check: Option<&str>, refused: bool| JudgementResult {
        state,
        check_line: check.map(str::to_owned),
        reason: None,
        duration_ms: 0,
        output_path: None,
        tail: String::new(),
        gate_refused: refused,
    };
    let one: ItemId = "1.1".parse().unwrap();
    let two: ItemId = "1.2".parse().unwrap();
    let three: ItemId = "1.3".parse().unwrap();
    let results = [
        (one, judged(JudgementState::Declared, Some("make gate"), true)),
        (two, judged(JudgementState::Confirmed, Some("exists ok.txt"), false)),
        (
            three,
            judged(JudgementState::Declared, Some("agent gate --locked"), true),
        ),
        (one, judged(JudgementState::Declared, None, false)),
    ];

    assert_eq!(
        super::refused_gates(&results),
        vec![(one, "make gate".to_owned()), (three, "agent gate --locked".to_owned())]
    );
    assert_eq!(super::refused_gates(&[]), Vec::new());
}

#[tokio::test]
async fn mismatch_and_fault_bounce_once_with_40_line_tail_and_human_answers_work() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done-on-resume-after run2-go.txt");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    let plan_with_unfinished_and_failing_cmd = concat!(
        "Finish it.\n\n1. Work\n",
        "[x] 1.1 Run command\n",
        "    Check: /bin/sh -c 'i=1; while [ \"$i\" -le 50 ]; do printf \"line-%s\\n\" \"$i\"; i=$((i + 1)); done; [ -f fixed.txt ]'\n",
        "[ ] 1.2 Finish later\n",
    );
    std::fs::write(dir.join(".agent/plan.md"), plan_with_unfinished_and_failing_cmd).unwrap();
    daemon.start().await;

    // Run 1 ends DONE with both an unfinished step (1.2) and a command check mismatch (1.1).
    // It counts as one bounce and Run 2's prompt names both reasons and at most 40 lines of output.
    sent_back_count(&daemon, 1).await;
    // Run 2 waits on run2-go.txt. Mark step 1.2 done and release Run 2 so it only fails on step 1.1.
    let plan_only_mismatch = concat!(
        "Finish it.\n\n1. Work\n",
        "[x] 1.1 Run command\n",
        "    Check: /bin/sh -c 'i=1; while [ \"$i\" -le 50 ]; do printf \"line-%s\\n\" \"$i\"; i=$((i + 1)); done; [ -f fixed.txt ]'\n",
        "[x] 1.2 Finish later\n",
    );
    std::fs::write(dir.join(".agent/plan.md"), plan_only_mismatch).unwrap();
    std::fs::write(dir.join("run2-go.txt"), "go").unwrap();
    let _state_at_q1 = daemon.status_is(1, TaskStatus::Questions).await;

    let run2_prompt = daemon.recorded(1, ".fake-args");
    assert!(
        run2_prompt.contains("## Your plan is not finished") && run2_prompt.contains("1.2"),
        "run 2 prompt must name the unfinished step: {run2_prompt}"
    );
    assert!(
        run2_prompt.contains("## Step 1.1 (Run command) check failed"),
        "run 2 prompt must also name the step check mismatch in the same bounce: {run2_prompt}"
    );
    assert!(
        run2_prompt.contains("line-11\n") && run2_prompt.contains("line-50") && !run2_prompt.contains("line-10\n"),
        "mismatch tail in prompt must hold at most the last 40 lines: {run2_prompt}"
    );

    let q1_text = std::fs::read_to_string(daemon.questions_file(1, 1)).expect("read questions-1.md");
    assert!(
        q1_text.contains("Step 1.1 (Run command) check failed") && q1_text.contains("fixed.txt"),
        "questions-1.md must name the step and its check: {q1_text}"
    );

    // Answer b) ("send it back once more"): grants and counts one more bounce, sending the task back.
    // Wait for questions-2.md when that extra bounce also ends with fixed.txt still absent.
    daemon.ask(answer(1, 1, &[(1, "b")])).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !daemon.questions_file(1, 2).exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        daemon.questions_file(1, 2).exists(),
        "timed out waiting for questions-2.md"
    );
    let state_at_q2 = daemon.status_is(1, TaskStatus::Questions).await;
    let keeping_at_q2 = keeping(&state_at_q2);
    assert_eq!(
        (keeping_at_q2.sent_back(), keeping_at_q2.granted()),
        (2, 1),
        "answering b) must grant and count one additional send-back"
    );

    // Answer a) / "ok" ("go on as it is") on questions-2.md: releases the plan so the next DONE settles as Done.
    daemon.ask(answer(1, 2, &[(1, "ok")])).await.unwrap();
    let final_state = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();
    assert_eq!(keeping(&final_state).stage(), PlanStage::Released);
}

#[tokio::test]
async fn mismatch_across_roles_statuses_and_refused_gate_across_restart() {
    use crate::fixtures::{investigator, member};
    use agent_core::state::TeamMembers;

    let mut team_done = State::new();
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, investigator("inv-opus")),
        members: vec![member(2, normal("claude-1"))],
        workers: Vec::new(),
    })
    .unwrap();
    let list = TaskListText::new("Finish it.".to_owned()).unwrap();
    team_done
        .insert_team(Team::new(team_id(0), repo(), at(0), MergeMode::Autopilot, list, crew))
        .unwrap();

    let combined_plan = "Finish it.\n\n1. Work\n[x] 1.1 Check file\n    Check: exists absent.txt\n[ ] 1.2 Remaining\n";
    let end_done = agent_core::state::RunEnd::Reported {
        outcome: agent_core::status::RunOutcome::Done,
    };
    for (label, state, task_num) in [
        ("solo", solo_state(), 1_u32),
        ("coordinator", team_done.clone(), 1_u32),
        ("member", team_done, 2_u32),
    ] {
        let daemon = with_bounces(&state, 2);
        let dir = daemon.scenario(task_num, "done");
        std::fs::create_dir_all(dir.join(".agent")).unwrap();
        std::fs::write(dir.join(".agent/plan.md"), combined_plan).unwrap();

        let (mut owner, _sinks) = daemon.bare_owner().await;
        let t = task(task_num);
        let located = owner.locate(&t).unwrap();
        let (_, held) = owner
            .plan_at_end(
                &t,
                &located,
                &end_done,
                crate::daemon::settle::Settled::Status(TaskStatus::Done),
            )
            .await
            .unwrap();
        assert_eq!(held, super::PlanHeld::Yes, "{label} must be held on mismatch");
        let kept = owner.parts.state.find_task(&t).unwrap().member.plan().unwrap();
        assert_eq!(kept.sent_back(), 1, "{label} must count 1 bounce");
        assert!(
            matches!(kept.pending(), Some(PlanReason::FaultAndMismatch { .. })),
            "{label} must record FaultAndMismatch, got {:?}",
            kept.pending()
        );
    }

    for (scenario, expected_status) in [("sleep", TaskStatus::Waiting), ("fail-tool", TaskStatus::Failed)] {
        let mut daemon = with_bounces(&solo_state(), 2);
        // A declared FAILED ends the task here, as it did before the ladder took it: `mode = "off"`.
        let mut config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
        config.push_str("\n[replacement]\nmode = \"off\"\n");
        std::fs::write(daemon.home.config_file(), config).unwrap();
        let dir = daemon.scenario(1, scenario);
        std::fs::create_dir_all(dir.join(".agent")).unwrap();
        std::fs::write(dir.join(".agent/plan.md"), MISSING_FILE_PLAN).unwrap();
        daemon.start().await;
        let settled_state = daemon.status_is(1, expected_status).await;
        daemon.stop().await.unwrap();

        let m = settled_state.find_task(&task(1)).unwrap().member;
        assert_eq!((m.runs().len(), m.plan().unwrap().sent_back()), (1, 0), "{scenario}");
        assert_eq!(
            judgement_of(&daemon, "1.1").state,
            agent_core::plan::judge::JudgementState::Mismatch
        );
    }

    let gate_plan = "Finish it.\n\n1. Work\n[x] 1.1 Gate step\n    Check: make gate\n[ ] 1.2 Next step\n";
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "sleep");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), gate_plan).unwrap();
    daemon.start().await;
    let mut waiting_state = daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    let (_, mut m) = waiting_state.find_task_mut(&task(1)).unwrap();
    m.send_back(PlanReason::Unfinished {
        steps: vec!["1.2".parse().unwrap()],
    });
    m.transition(TaskStatus::Queued).unwrap();
    std::fs::write(daemon.home.state_file(), waiting_state.to_json().unwrap()).unwrap();
    daemon.scenario(1, "done");

    daemon.start().await;
    let _at_questions = daemon.status_is(1, TaskStatus::Questions).await;
    let first_after_restart = daemon.recorded(1, ".fake-args");
    assert_eq!(first_after_restart.matches("## Step 1.1 check was refused").count(), 1);

    daemon.ask(answer(1, 1, &[(1, "ok")])).await.unwrap();
    let _at_done = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();
    let second_after_restart = daemon.recorded(1, ".fake-args");
    assert_eq!(second_after_restart.matches("## Step 1.1 check was refused").count(), 0);
}

#[tokio::test]
async fn legacy_member_and_roles_deliver_refused_gate_and_plain_mismatch_end_to_end() {
    // 1. Legacy member (Member::without_plan) with `Check: make gate`, ends SLEEPING, restarts, and queues next run.
    let mut legacy_state = State::new();
    let legacy_crew = Crew::solo(Member::without_plan(task(1), normal("claude-1"))).unwrap();
    legacy_state
        .insert_team(Team::new(
            team_id(0),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            legacy_crew,
        ))
        .unwrap();
    let mut daemon = with_bounces(&legacy_state, 2);
    let dir = daemon.scenario(1, "sleep");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(
        dir.join(".agent/plan.md"),
        "Finish it.\n\n1. Work\n[x] 1.1 Gate step\n    Check: make gate\n",
    )
    .unwrap();
    daemon.start().await;
    let mut waiting = daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    waiting
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Queued)
        .unwrap();
    std::fs::write(daemon.home.state_file(), waiting.to_json().unwrap()).unwrap();
    daemon.scenario(1, "done");
    daemon.start().await;
    let settled = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();
    let legacy_prompt = daemon.recorded(1, ".fake-args");
    assert_eq!(
        legacy_prompt.matches("## Step 1.1 check was refused").count(),
        1,
        "legacy member must receive refused gate notice once: {legacy_prompt}"
    );
    assert_eq!(settled.find_task(&task(1)).unwrap().member.plan(), None);

    // 2. End-to-end daemon.start() plain mismatch bounce and repair for both coordinator (task 1) and member (task 2).
    for (active_task, inactive_task, settled_status) in
        [(1_u32, 2_u32, TaskStatus::Done), (2_u32, 1_u32, TaskStatus::Waiting)]
    {
        let mut team_done = super::super::fixture::pair_state();
        {
            let (_, mut other) = team_done.find_task_mut(&task(inactive_task)).unwrap();
            other.transition(TaskStatus::Running).unwrap();
            other.transition(TaskStatus::Stopped).unwrap();
        }
        let mut role_daemon = with_bounces(&team_done, 1);
        let role_dir = role_daemon.scenario(active_task, "done-on-resume-after absent.txt");
        std::fs::create_dir_all(role_dir.join(".agent")).unwrap();
        std::fs::write(role_dir.join(".agent/plan.md"), MISSING_FILE_PLAN).unwrap();
        role_daemon.start().await;
        role_daemon
            .state_where("1 send-back of the active role", |s| {
                s.find_task(&task(active_task))
                    .and_then(|f| f.member.plan().map(|k| k.sent_back() == 1))
                    .unwrap_or(false)
            })
            .await;
        std::fs::write(role_dir.join("absent.txt"), "repaired").unwrap();
        let role_settled = role_daemon.status_is(active_task, settled_status).await;
        role_daemon.stop().await.unwrap();
        let role_prompt = role_daemon.recorded(active_task, ".fake-args");
        assert!(
            role_prompt.contains("the check of step 1.1 (`exists absent.txt`) failed"),
            "{role_prompt}"
        );
        assert_eq!(
            role_settled
                .find_task(&task(active_task))
                .unwrap()
                .member
                .plan()
                .unwrap()
                .sent_back(),
            1
        );
    }
}

#[tokio::test]
async fn aborted_unpolled_run_preserves_refused_gate_across_restart_and_missing_output_is_reported() {
    let mut daemon = with_bounces(&solo_state(), 1);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), FINISHED).unwrap();

    let mut initial = daemon.state();
    initial
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .add_plan_refused_gate("1.1".parse().unwrap(), "make gate".to_owned());
    std::fs::write(daemon.home.state_file(), initial.to_json().unwrap()).unwrap();

    {
        let (mut owner, _sinks) = daemon.bare_owner().await;
        owner.start(task(1)).await.unwrap();
        owner.runs.abort_all();
    }

    daemon.start().await;
    let settled = daemon.status_is(1, TaskStatus::Done).await;
    daemon.stop().await.unwrap();
    let prompt = daemon.recorded(1, ".fake-args");
    assert_eq!(
        prompt.matches("## Step 1.1 check was refused").count(),
        1,
        "recovered run after aborted unpolled engine task must receive refused gate notice once: {prompt}"
    );
    assert_eq!(
        settled.find_task(&task(1)).unwrap().member.plan_refused_gate(),
        &[],
        "refused gate notice must be cleared once the recovered run finishes"
    );

    let parsed = agent_core::plan::parse_plan("Finish.\n\n1. Work\n[x] 1.1 Run cmd\n    Check: cargo test\n").unwrap();
    let run_id: agent_core::ids::RunId = "run-20261122-060000-abcd".parse().unwrap();
    let mut judgements = std::collections::HashMap::new();
    judgements.insert(
        "1.1".parse().unwrap(),
        agent_runtime::history::JudgementRecord {
            task: task(1),
            team: team_id(0),
            run: Some(run_id.clone()),
            at: at(10),
            step: "1.1".parse().unwrap(),
            check: Some("cargo test".to_owned()),
            state: agent_core::plan::judge::JudgementState::Mismatch,
            reason: Some("exit status 1".to_owned()),
            output_path: Some("checks/run-20261122-060000-abcd/1.1.log".to_owned()),
            duration_ms: 15,
        },
    );
    let reason = super::first_mismatch(&daemon.home.task_dir(&task(1)), &parsed, &judgements, Some(&run_id))
        .await
        .expect("mismatch reason when output file is missing");
    let (owner, _sinks) = daemon.bare_owner().await;
    let rendered = super::correction(&owner.parts.texts, &reason).unwrap();
    assert!(
        rendered.contains("cannot read the check output"),
        "missing output file must be reported in correction tail: {rendered}"
    );
}

#[tokio::test]
async fn pre_judging_task_with_plan_keeping_is_judged_and_never_sent_back_for_mismatch() {
    use agent_core::plan::judge::JudgementState;
    use agent_runtime::history::Reader;

    let mut pre_judging_state = State::new();
    let crew = Crew::solo(Member::before_judging(task(1), normal("claude-1"))).unwrap();
    pre_judging_state
        .insert_team(Team::new(
            team_id(0),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            crew,
        ))
        .unwrap();
    let json = pre_judging_state.to_json().unwrap();
    let reloaded = State::from_json(&json).unwrap();
    assert!(
        reloaded
            .find_task(&task(1))
            .unwrap()
            .member
            .plan()
            .is_some_and(|k| !k.judging()),
        "pre-judging task must keep a PlanKeeping without the judging marker"
    );

    let mut daemon = with_bounces(&reloaded, 3);
    let dir = daemon.scenario(1, "done");
    std::fs::create_dir_all(dir.join(".agent")).unwrap();
    std::fs::write(dir.join(".agent/plan.md"), MISSING_FILE_PLAN).unwrap();

    daemon.start().await;
    let settled = daemon
        .state_where("pre-judging run 1 settled", |s| {
            s.find_task(&task(1)).is_some_and(|f| {
                f.member.runs().first().is_some_and(|r| !r.is_running())
                    && f.member.status() != TaskStatus::Running
                    && f.member.plan_running_check().is_none()
            })
        })
        .await;
    daemon.stop().await.unwrap();

    let member = settled.find_task(&task(1)).unwrap().member;
    let keeping = member.plan().expect("pre-judging task keeps its plan record");
    assert!(!keeping.judging());
    assert_eq!(
        keeping.sent_back(),
        0,
        "pre-judging task with PlanKeeping must not be sent back for a step mismatch"
    );
    assert_eq!(member.status(), TaskStatus::Done);
    assert_eq!(member.runs().len(), 1);

    let reader = Reader::open(&daemon.home.history_file()).unwrap();
    let judgements = reader.latest_judgements(&task(1)).unwrap();
    assert_eq!(
        judgements[&ItemId::from_str("1.1").unwrap()].state,
        JudgementState::Mismatch
    );
}
