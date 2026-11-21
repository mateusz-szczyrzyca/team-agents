//! The plan of a task through the daemon: what a run is told of it, and a plan committed by mistake.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::ItemId;
use agent_core::questions::QuestionsFile;
use agent_core::state::{
    Crew, Member, MergeMode, PlanKeeping, PlanReason, PlanStage, Question, State, TaskListText, Team,
};
use agent_core::status::TaskStatus;

use super::super::fixture::{Daemon, answer, n};
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
    let mut daemon = with_bounces(&solo_state(), 1);
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
    let _state = daemon.status_is(1, TaskStatus::Done).await;
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
