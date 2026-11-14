//! What the daemon records in the history for a team in a daemon with a fake GitHub, a bare `origin`, stand-ins for the
//! model CLIs and the real `agentbus`: the team with its plan, the rounds with their items, every run with the prompt
//! and the command line its CLI got, its steps, what it changed and how it ended — read back from `history.db` through
//! the reading API after the daemon stopped, and compared with what the stand-ins recorded of themselves and with the
//! state.

use std::collections::BTreeSet;
use std::str::FromStr;

use agent_core::cli::ActivityEvent;
use agent_core::config::{AbsolutePath, Program};
use agent_core::decide::checks::{Change, Lines};
use agent_core::history::MaxTextBytes;
use agent_core::history::{HowChosen, RoundSubject, RunWhy};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::protocol::{Client, Command, ControlAction, ControlRequest, Reply, StatusRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, PrNumber, RunEnd, State};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_runtime::agentbus::{AgentBus, Delegation};
use agent_runtime::history::{HistoryDb, Migration, Reader, RunRow};

use super::rig::{CHAIN, ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,done:VERDICT: APPROVE\\nSUMMARY: fixed";

/// A rig with a solo agent that works on every item and two reviewers who approve.
async fn solo_rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await
}

/// A rig with a coordinator that sleeps until its member is done and then integrates, a member and two reviewers who
/// approve.
async fn team_rig() -> Rig {
    Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate"),
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await
}

/// Adds the worker `name`, whose stand-in does what `scenario` says, to the configuration of the home of `rig`.
fn add_worker(rig: &Rig, name: &str, scenario: &str) {
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let root = rig.home.root().display();
    let table = format!(
        "[agents.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \"--home\", \"{root}\", \
         \"--bus-bin\", \"{}\", \"--bus-dir\", \"{root}/bus\"]\n",
        super::super::fixture::agentbus_binary().display(),
    );
    let with_tier = text.replace(
        "[tiers]\nnormal = [",
        &format!("[tiers]\nworkers = [\"{name}\"]\nnormal = ["),
    );
    assert_ne!(with_tier, text);
    std::fs::write(rig.home.config_file(), format!("{with_tier}{table}")).unwrap();
}

/// The reader of the history the daemon of `rig` wrote; the daemon has stopped, so everything is in it.
fn reader(rig: &Rig) -> Reader {
    Reader::open(&rig.home.history_file()).unwrap()
}

/// Every run of every task of the only team of `state`: its task, the role of the task, the run's number in its task
/// counted from 1, and its ID.
fn runs_of(state: &State) -> Vec<(TaskId, Role, usize, RunId)> {
    let team = &state.teams()[0];
    let crew = team.crew().roster();
    let reviewers = team.reviewers().iter().map(|member| (Role::Reviewer, member));
    crew.into_iter()
        .chain(reviewers)
        .flat_map(|(role, member)| {
            member
                .runs()
                .iter()
                .enumerate()
                .map(|(index, run)| (member.task().clone(), role, index + 1, run.id().clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The rows of the runs of `team`, by run.
fn rows(reader: &Reader, team: &TeamId) -> Vec<RunRow> {
    reader.runs_of_team(team).unwrap()
}

#[tokio::test]
async fn a_solo_round_is_in_the_history_from_its_team_to_its_reviews_with_every_run_once() {
    let mut rig = solo_rig().await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reader = reader(&rig);
    let started = reader.team(&team).unwrap().unwrap().value;
    assert_eq!(started.task_list, ONE);
    assert_eq!(
        (started.repo.as_str(), started.base_branch.as_str(), started.merge),
        ("example", "main", MergeMode::Autopilot)
    );
    assert_eq!(started.plan.len(), 1);
    assert_eq!(started.plan[0].agent.as_str(), "claude-1");
    assert_eq!(started.plan[0].role, Role::Solo);
    assert_eq!(started.plan[0].how, HowChosen::Named);
    let rounds = reader.rounds_of_team(&team).unwrap();
    assert_eq!(rounds.len(), 1);
    let RoundSubject::Item(item) = &rounds[0].value.subject else {
        panic!("{:?}", rounds[0])
    };
    let listed = state.teams()[0]
        .task_list()
        .list()
        .item("1.1".parse().unwrap())
        .unwrap()
        .clone();
    assert_eq!(
        (
            item.title.as_str(),
            item.description.as_str(),
            item.expected_result.clone(),
            item.checks.clone()
        ),
        (
            listed.title.as_str(),
            listed.description.as_str(),
            listed.expected_result.clone(),
            listed.checks.clone()
        )
    );
    // A row for every run of the state, and none twice.
    let expected: BTreeSet<RunId> = runs_of(&state).into_iter().map(|(_, _, _, run)| run).collect();
    let found: Vec<RunId> = rows(&reader, &team)
        .iter()
        .map(|row| row.start.value.run.clone())
        .collect();
    assert_eq!(found.iter().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(found.len(), expected.len(), "a run is in the history twice: {found:?}");
    assert!(
        expected.len() >= 3,
        "the solo agent and two reviewers ran: {expected:?}"
    );
    // The reasons: the first run of the agent, and the reviews of the pull request.
    for row in rows(&reader, &team) {
        let start = &row.start.value;
        match start.role {
            Role::Solo => assert_eq!(start.why, RunWhy::FirstRun),
            Role::Reviewer => assert_eq!(
                start.why,
                RunWhy::Review {
                    pr: PrNumber::new(std::num::NonZeroU32::MIN),
                    pass: 0
                }
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(start.team, team);
        assert!(tasks.contains(&start.task) || start.role == Role::Reviewer);
        assert_eq!(start.rounds.len(), 1);
        assert!(row.end.is_some(), "run {} has no end", start.run);
    }
}

#[tokio::test]
async fn the_prompt_and_the_command_line_of_every_run_equal_what_its_cli_got_byte_for_byte() {
    let mut rig = team_rig().await;
    add_worker(&rig, "glm-1", "sleep,done:Result: looked");
    rig.start().await;

    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "glm-1"],
            CHAIN,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    rig.state_where("the member to have run", |state| {
        state
            .find_task(&tasks[1])
            .is_some_and(|found| !found.member.runs().is_empty())
    })
    .await;
    let bus = AgentBus::new(
        Program::from_str(super::super::fixture::agentbus_binary().to_str().unwrap()).unwrap(),
        AbsolutePath::from_str(rig.home.root().to_str().unwrap()).unwrap(),
    );
    bus.delegate(&Delegation {
        team: team.clone(),
        requester: "claude-1".parse().unwrap(),
        task: tasks[1].clone(),
        repo: "example".parse().unwrap(),
        branch: format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap(),
        worktree: AbsolutePath::from_str(rig.worktree(&tasks[1]).to_str().unwrap()).unwrap(),
        dispatcher: Program::from_str("true").unwrap(),
        request: "Look at the module.".to_owned(),
        context: "None.".to_owned(),
    })
    .await
    .unwrap();
    rig.state_where("both items done and the worker to have served the request", |state| {
        state
            .find_task(&tasks[2])
            .is_some_and(|found| found.member.runs().len() >= 2)
            && state.teams()[0]
                .item("1.2".parse().unwrap())
                .is_some_and(|progress| matches!(progress.phase(), ItemPhase::Done { .. }))
    })
    .await;
    rig.stop().await.unwrap();
    let state = rig.state();

    let reader = reader(&rig);
    let mut compared = 0;
    for (task, role, n, run) in runs_of(&state) {
        let row = reader
            .run(&run)
            .unwrap()
            .unwrap_or_else(|| panic!("run {run} is not in the history"));
        let worktree = rig.worktree(&task);
        let prompt = std::fs::read_to_string(worktree.join(format!(".fake-prompt-{n}"))).unwrap();
        let args = std::fs::read_to_string(worktree.join(format!(".fake-args-{n}"))).unwrap();
        assert_eq!(
            format!("{}\n", row.start.value.prompt),
            prompt,
            "the prompt of {role} run {n} of {task}"
        );
        let launch = row.launch.unwrap_or_else(|| panic!("run {run} has no launch")).value;
        assert_eq!(
            format!("{}\n", launch.args.join("\n")),
            args,
            "the command line of {role} run {n} of {task}"
        );
        assert_eq!(launch.working_dir.as_path(), worktree.as_path());
        compared += 1;
    }
    assert!(
        compared >= 5,
        "coordinator, member, worker and reviewers ran: {compared}"
    );
}

#[tokio::test]
async fn the_plan_of_a_team_says_how_each_participant_got_its_role() {
    let mut rig = team_rig().await;
    add_worker(&rig, "glm-1", "sleep");
    rig.start().await;

    let (team, _) = rig
        .run(
            &["inv-lead", "claude-1", "glm-1"],
            ONE,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    rig.stop().await.unwrap();

    let plan = reader(&rig).team(&team).unwrap().unwrap().value.plan;
    let seen: Vec<(String, u32, Role, HowChosen)> = plan
        .into_iter()
        .map(|entry| (entry.agent.to_string(), entry.position.get(), entry.role, entry.how))
        .collect();
    assert_eq!(
        seen,
        [
            ("inv-lead".to_owned(), 1, Role::Coordinator, HowChosen::NamedByFlag),
            ("claude-1".to_owned(), 2, Role::Member, HowChosen::Named),
            ("glm-1".to_owned(), 3, Role::Worker, HowChosen::ConfiguredOrder),
        ]
    );
}

#[tokio::test]
async fn a_coordinator_nobody_named_is_drawn_and_the_history_says_among_whom() {
    let mut rig = Rig::new(
        &[
            investigator("inv-a", "sleep"),
            investigator("inv-b", "sleep"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;

    let (team, _) = rig
        .run(&["claude-1", "inv-a", "inv-b"], ONE, MergeMode::NoMerge, None)
        .await;
    let state = rig.state();
    rig.stop().await.unwrap();

    let drawn = state.team(&team).unwrap().crew().roster()[0]
        .1
        .agent()
        .name()
        .to_owned();
    let plan = reader(&rig).team(&team).unwrap().unwrap().value.plan;
    let coordinator = plan.iter().find(|entry| entry.role == Role::Coordinator).unwrap();
    assert_eq!(coordinator.agent.as_str(), drawn);
    let among = ["inv-a", "inv-b"].map(|name| name.parse().unwrap());
    assert_eq!(coordinator.how, HowChosen::Drawn { among: among.to_vec() });
    let members: Vec<(&str, &HowChosen)> = plan
        .iter()
        .filter(|entry| entry.role == Role::Member)
        .map(|entry| (entry.agent.as_str(), &entry.how))
        .collect();
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|(_, how)| **how == HowChosen::Named));
}

#[tokio::test]
async fn a_worker_that_serves_a_request_has_a_run_that_names_the_run_that_asked() {
    let mut rig = team_rig().await;
    add_worker(&rig, "glm-1", "sleep,done:Result: looked");
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["inv-lead", "claude-1", "glm-1"],
            CHAIN,
            MergeMode::Autopilot,
            Some("inv-lead"),
        )
        .await;
    let asking = rig
        .state_where("the member to have a run", |state| {
            state
                .find_task(&tasks[1])
                .is_some_and(|found| !found.member.runs().is_empty())
        })
        .await
        .find_task(&tasks[1])
        .unwrap()
        .member
        .runs()
        .last()
        .unwrap()
        .id()
        .clone();
    let bus = AgentBus::new(
        Program::from_str(super::super::fixture::agentbus_binary().to_str().unwrap()).unwrap(),
        AbsolutePath::from_str(rig.home.root().to_str().unwrap()).unwrap(),
    );
    bus.delegate(&Delegation {
        team: team.clone(),
        requester: "claude-1".parse().unwrap(),
        task: tasks[1].clone(),
        repo: "example".parse().unwrap(),
        branch: format!("agents/claude-1/{}-1.1", tasks[1]).parse().unwrap(),
        worktree: AbsolutePath::from_str(rig.worktree(&tasks[1]).to_str().unwrap()).unwrap(),
        dispatcher: Program::from_str("true").unwrap(),
        request: "Look at the module.".to_owned(),
        context: "None.".to_owned(),
    })
    .await
    .unwrap();
    rig.state_where("the worker to have served the request", |state| {
        state
            .find_task(&tasks[2])
            .is_some_and(|found| found.member.runs().len() >= 2)
    })
    .await;
    rig.stop().await.unwrap();

    let reader = reader(&rig);
    let worker_runs: Vec<RunRow> = rows(&reader, &team)
        .into_iter()
        .filter(|row| row.start.value.role == Role::Worker)
        .collect();
    assert!(worker_runs.len() >= 2, "{worker_runs:?}");
    assert_eq!(worker_runs[0].start.value.why, RunWhy::FirstRun);
    assert_eq!(
        worker_runs[1].start.value.why,
        RunWhy::Request { asked_by: Some(asking) }
    );
}

#[tokio::test]
async fn the_steps_of_a_run_are_what_its_cli_printed_in_order() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "stream:steps"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let lines = [
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"starting"}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"src/lib.rs"}}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t3","name":"mcp__agentbus__board_add","input":{"title":"x"}}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t4","name":"Task","input":{"pattern":"look around"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}}"#,
    ];
    std::fs::write(scripts.join("steps.jsonl"), format!("{}\n", lines.join("\n"))).unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    let run = state.find_task(&tasks[0]).unwrap().member.runs()[0].id().clone();
    let steps = reader(&rig).steps(&run).unwrap();
    let seen: Vec<(u32, ActivityEvent)> = steps
        .into_iter()
        .map(|step| (step.value.number.get(), step.value.event))
        .collect();
    assert_eq!(
        seen,
        [
            (1, ActivityEvent::Info("session started, model fake-model".to_owned())),
            (2, ActivityEvent::Text("starting".to_owned())),
            (3, ActivityEvent::Command("cargo test".to_owned())),
            (
                4,
                ActivityEvent::FileEdit {
                    path: "src/lib.rs".to_owned()
                }
            ),
            (
                5,
                ActivityEvent::BusCall {
                    tool: "board_add".to_owned(),
                    arguments: r#"{"title":"x"}"#.to_owned()
                }
            ),
            (
                6,
                ActivityEvent::ToolCall {
                    name: "Task".to_owned(),
                    summary: "look around".to_owned()
                }
            ),
            (7, ActivityEvent::Error("boom".to_owned())),
            (8, ActivityEvent::Info("result: success".to_owned())),
        ]
    );
}

#[tokio::test]
async fn the_changes_of_a_run_are_what_its_cli_did_a_file_added_deleted_modified_renamed_binary_and_one_left_uncommitted()
 {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:changes"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    // The repository the worktrees start from has the files the run is to change.
    std::fs::write(rig.repo.join("edited.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(rig.repo.join("gone.txt"), "a\nb\n").unwrap();
    std::fs::write(
        rig.repo.join("moved.txt"),
        "this file will move and keep its contents,\nso git calls it a rename\n",
    )
    .unwrap();
    git(&rig.repo, &["add", "-A"]);
    git(&rig.repo, &["commit", "-q", "-m", "More files"]);
    git(&rig.repo, &["push", "-q", "origin", "main"]);
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("changes.sh"),
        "set -e\n\
         printf 'one\\nTWO\\nthree\\nfour\\n' > edited.txt\n\
         git rm -q gone.txt\n\
         git mv moved.txt renamed.txt\n\
         printf 'x\\ny\\n' > added.txt\n\
         printf '\\000\\001\\002\\003' > blob.bin\n\
         git add edited.txt added.txt blob.bin\n\
         git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m 'Change many things'\n\
         printf 'left\\n' > left.txt\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    let run = state.find_task(&tasks[0]).unwrap().member.runs()[0].id().clone();
    let reader = reader(&rig);
    let changes = reader.changes(&run).unwrap().unwrap().value;
    let launch_head = reader.run(&run).unwrap().unwrap().launch.unwrap().value.head;
    assert_eq!(changes.head_before, launch_head);
    assert!(changes.head_before.is_some() && changes.head_after.is_some() && changes.head_before != changes.head_after);
    assert_eq!(changes.commits.len(), 1);
    assert_eq!(changes.commits[0].subject, "Change many things");
    let files: Vec<(String, Change, Lines)> = changes
        .files
        .into_iter()
        .map(|file| (file.path.to_string(), file.change, file.lines))
        .collect();
    assert_eq!(
        files,
        [
            (
                "added.txt".to_owned(),
                Change::Added,
                Lines::Text { added: 2, removed: 0 }
            ),
            ("blob.bin".to_owned(), Change::Added, Lines::Binary),
            (
                "edited.txt".to_owned(),
                Change::Modified,
                Lines::Text { added: 2, removed: 1 }
            ),
            (
                "gone.txt".to_owned(),
                Change::Deleted,
                Lines::Text { added: 0, removed: 2 }
            ),
            (
                "renamed.txt".to_owned(),
                Change::Renamed {
                    from: "moved.txt".parse().unwrap()
                },
                Lines::Text { added: 0, removed: 0 }
            ),
        ]
    );
    let left: Vec<String> = changes.uncommitted.iter().map(|file| file.path.to_string()).collect();
    assert!(left.contains(&"left.txt".to_owned()), "{left:?}");
    let left_file = changes
        .uncommitted
        .iter()
        .find(|file| file.path.as_str() == "left.txt")
        .unwrap();
    assert_eq!(
        (&left_file.change, left_file.lines),
        (&Change::Added, Lines::Text { added: 1, removed: 0 })
    );
}

#[tokio::test]
async fn the_review_passes_and_the_fix_passes_of_a_pull_request_are_numbered() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reader = reader(&rig);
    let whys: Vec<(Role, RunWhy)> = rows(&reader, &team)
        .into_iter()
        .map(|row| (row.start.value.role, row.start.value.why))
        .collect();
    let pr = PrNumber::new(std::num::NonZeroU32::MIN);
    let fix = RunWhy::FixPass {
        number: std::num::NonZeroU32::MIN,
    };
    assert!(whys.contains(&(Role::Solo, fix)), "{whys:?}");
    assert!(
        whys.contains(&(Role::Reviewer, RunWhy::Review { pr, pass: 0 })),
        "{whys:?}"
    );
    assert!(
        whys.contains(&(Role::Reviewer, RunWhy::Review { pr, pass: 1 })),
        "{whys:?}"
    );
}

#[tokio::test]
async fn a_daemon_stopped_in_the_middle_of_a_run_leaves_one_row_for_each_run_and_the_next_run_goes_on() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "slowdone,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the first run to have started", || rig.runs(&tasks[0]) >= 1)
        .await;

    rig.stop().await.unwrap();
    rig.start().await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reader = reader(&rig);
    let found: Vec<RunRow> = rows(&reader, &team);
    let expected: BTreeSet<RunId> = runs_of(&state).into_iter().map(|(_, _, _, run)| run).collect();
    assert_eq!(
        found
            .iter()
            .map(|row| row.start.value.run.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(found.len(), expected.len());
    let solo: Vec<&RunRow> = found.iter().filter(|row| row.start.value.role == Role::Solo).collect();
    assert!(solo.len() >= 2, "{solo:?}");
    assert_eq!(
        solo[0].end.as_ref().unwrap().value.end,
        RunEnd::NoStatus { exit_code: None }
    );
    assert_eq!(solo[1].start.value.why, RunWhy::Continuation);
    assert_eq!(
        solo[1].end.as_ref().unwrap().value.end,
        RunEnd::Reported {
            outcome: RunOutcome::Done
        }
    );
}

#[tokio::test]
async fn a_stop_and_a_resume_by_the_human_are_a_control_step_of_the_run_and_the_reason_of_the_next() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "slowdone,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.wait_for("the first run to have started", || rig.runs(&tasks[0]) >= 1)
        .await;

    for command in [Command::Stop(ControlRequest {
        target: Target::Task(tasks[0].clone()),
    })] {
        crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, command)
            .await
            .unwrap();
    }
    rig.status_is(&tasks[0], TaskStatus::Stopped).await;
    crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Resume(ControlRequest {
                target: Target::Task(tasks[0].clone()),
            }),
        )
        .await
        .unwrap();
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reader = reader(&rig);
    let solo: Vec<RunRow> = rows(&reader, &team)
        .into_iter()
        .filter(|row| row.start.value.role == Role::Solo)
        .collect();
    assert_eq!(solo[0].end.as_ref().unwrap().value.end, RunEnd::Stopped);
    assert_eq!(solo[1].start.value.why, RunWhy::ResumedByHuman);
    let steps = reader.steps(&solo[0].start.value.run).unwrap();
    assert!(
        steps.iter().any(|step| matches!(
            &step.value.event,
            ActivityEvent::Control {
                action: ControlAction::Stop,
                by: Client::Cli
            }
        )),
        "{steps:?}"
    );
}

#[tokio::test]
async fn no_value_of_a_secret_or_of_the_environment_of_the_clis_is_in_the_files_of_the_history() {
    let mut rig = solo_rig().await;
    let secrets = rig.home.root().join("secrets.toml");
    std::fs::write(
        &secrets,
        "github_token = \"ghp_marker_7d31f5a2c9\"\n[agent_env]\nMARKER_API_KEY = \"marker-key-83b2e6f1a4\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let launch = rows(&reader(&rig), &team)[0].launch.clone().unwrap().value;
    assert!(
        launch.env_names.iter().any(|name| name.as_str() == "MARKER_API_KEY"),
        "the name is recorded"
    );
    let mut files = 0;
    for entry in std::fs::read_dir(rig.home.root()).unwrap() {
        let path = entry.unwrap().path();
        if !path.file_name().unwrap().to_string_lossy().starts_with("history.db") {
            continue;
        }
        files += 1;
        let bytes = std::fs::read(&path).unwrap();
        for marker in ["ghp_marker_7d31f5a2c9", "marker-key-83b2e6f1a4"] {
            assert!(
                !bytes.windows(marker.len()).any(|window| window == marker.as_bytes()),
                "{marker} is in {path:?}"
            );
        }
    }
    assert!(files >= 1);
}

#[tokio::test]
async fn a_history_in_the_way_does_not_stop_a_round_and_status_shows_what_was_lost() {
    let mut rig = solo_rig().await;
    std::fs::create_dir(rig.home.history_file()).unwrap();
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    rig.stop().await.unwrap();

    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: agent_core::state::Completion::Merged { .. }
        }
    ));
    let Reply::Overview { teams, history } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(teams.len(), 1);
    assert!(
        matches!(history.state, agent_core::history::HistoryState::Unavailable { .. }),
        "{history:?}"
    );
    assert!(history.lost >= 3, "{history:?}");
}

#[tokio::test]
async fn a_history_of_a_newer_version_is_left_as_it_is_and_the_round_goes_on() {
    let mut rig = solo_rig().await;
    // A database of the version after the one this build writes: made with a list of migrations that is one longer.
    let migrations = [
        Migration {
            version: 1,
            sql: "CREATE TABLE from_the_past (x TEXT);",
        },
        Migration {
            version: 2,
            sql: "CREATE TABLE from_the_future (x TEXT);",
        },
    ];
    drop(HistoryDb::open_with(&rig.home.history_file(), &migrations, MaxTextBytes::default()).unwrap());
    let before = std::fs::read(rig.home.history_file()).unwrap();
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    rig.stop().await.unwrap();

    assert_eq!(std::fs::read(rig.home.history_file()).unwrap(), before);
    let Reply::Overview { history, .. } = reply else {
        panic!("{reply:?}")
    };
    assert!(
        matches!(&history.state, agent_core::history::HistoryState::Unavailable { reason } if reason.contains("version 2")),
        "{history:?}"
    );
}
