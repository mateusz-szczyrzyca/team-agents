//! The rounds of a team in a daemon with a fake GitHub: a round is opened when its item is given to a member, closed when
//! the item is merged (or approved with `--no-merge`), and every run names the rounds of its role — against stand-ins for
//! the model CLIs, a bare `origin` and the real `agentbus`.

use std::str::FromStr;

use agent_core::config::{AbsolutePath, Program};
use agent_core::ids::{RoundId, TaskId};
use agent_core::protocol::{Client, Command, EventKind, Frame, Request, RequestId, SubscribeRequest, Target};
use agent_core::state::{ItemPhase, MergeMode, RoundEnd, State};

use agent_runtime::agentbus::{AgentBus, Delegation};

use super::rig::{CHAIN, ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,done:VERDICT: APPROVE\\nSUMMARY: fixed";

/// Two features that do not wait for each other: both items start at once when two members are free.
const PARALLEL: &str = "\
Build the things.

1. First
[ ] 1.1 One
    Do one.

2. Second
[ ] 2.1 Two
    Do two.
";

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

/// The rounds of the only team of `state`: their IDs, items and ends, oldest first.
fn rounds_of_team(state: &State) -> Vec<(RoundId, Option<String>, Option<RoundEnd>)> {
    state.teams()[0]
        .rounds()
        .iter()
        .map(|round| {
            (
                round.id().clone(),
                round.item().map(|item| item.to_string()),
                round.ended(),
            )
        })
        .collect()
}

/// The rounds the run number `n` (from 0) of `task` works for.
fn rounds_of_run(state: &State, task: &TaskId, n: usize) -> Vec<RoundId> {
    state.find_task(task).unwrap().member.runs()[n].rounds().to_vec()
}

/// The events of a subscription, read as they come.
struct Events {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    _writer: tokio::net::unix::OwnedWriteHalf,
}

impl Events {
    /// Subscribes to the events of every team of the daemon of `rig`.
    async fn of(rig: &Rig) -> Self {
        let (read, mut writer) = tokio::net::UnixStream::connect(rig.home.socket_file())
            .await
            .unwrap()
            .into_split();
        let mut reader = tokio::io::BufReader::new(read);
        let line = Request::new(
            RequestId(1),
            Client::Cli,
            Command::Subscribe(SubscribeRequest { team: None }),
        )
        .to_line()
        .unwrap();
        crate::lines::write_line(&mut writer, &line).await.unwrap();
        let first = crate::lines::read_line(&mut reader, 1 << 24).await.unwrap().unwrap();
        assert!(
            matches!(Frame::from_line(&first).unwrap(), Frame::Response(_)),
            "{first}"
        );
        Self {
            reader,
            _writer: writer,
        }
    }

    /// Reads events until `wanted` of them satisfy `pick`, and returns what it picked, in order.
    async fn collect<T>(&mut self, wanted: usize, pick: impl Fn(&EventKind) -> Option<T>) -> Vec<T> {
        let mut found = Vec::new();
        while found.len() < wanted {
            let line = tokio::time::timeout(super::rig::PATIENCE, crate::lines::read_line(&mut self.reader, 1 << 24))
                .await
                .expect("timed out waiting for events")
                .unwrap()
                .expect("the connection ended while waiting for events");
            if let Frame::Event(event) = Frame::from_line(&line).unwrap()
                && let Some(picked) = pick(&event.event)
            {
                found.push(picked);
            }
        }
        found
    }
}

/// What a `round_opened` or `round_closed` event says, as text.
fn round_event(kind: &EventKind) -> Option<String> {
    match kind {
        EventKind::RoundOpened { item, .. } => Some(format!(
            "opened {}",
            item.map_or("-".to_owned(), |item| item.to_string())
        )),
        EventKind::RoundClosed { item, how, .. } => Some(format!(
            "closed {} {how}",
            item.map_or("-".to_owned(), |item| item.to_string())
        )),
        _ => None,
    }
}

#[tokio::test]
async fn a_solo_agent_on_a_two_item_list_opens_and_closes_its_two_rounds_in_order() {
    let mut rig = solo_rig().await;
    rig.start().await;
    let mut events = Events::of(&rig).await;

    let (team, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let told = events.collect(4, round_event).await;
    let state = rig
        .item_where("1.2", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert_eq!(
        told,
        ["opened 1.1", "closed 1.1 merged", "opened 1.2", "closed 1.2 merged"]
    );
    let rounds = rounds_of_team(&state);
    assert_eq!(rounds.len(), 2);
    assert_eq!(
        rounds
            .iter()
            .map(|(_, item, end)| (item.as_deref(), *end))
            .collect::<Vec<_>>(),
        [
            (Some("1.1"), Some(RoundEnd::Merged)),
            (Some("1.2"), Some(RoundEnd::Merged))
        ]
    );
    let found = state.team(&team).unwrap();
    let (first, second) = (&found.rounds()[0], &found.rounds()[1]);
    assert!(first.opened_at() <= second.opened_at());
    assert!(matches!(first.life(), agent_core::state::RoundLife::Closed { at, .. } if *at <= second.opened_at()));
    assert_eq!(rounds_of_run(&state, &tasks[0], 0), [rounds[0].0.clone()]);
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(lead.runs().last().unwrap().rounds(), [rounds[1].0.clone()]);
}

#[tokio::test]
async fn the_events_about_an_item_a_pull_request_a_review_or_a_run_name_the_round() {
    let mut rig = solo_rig().await;
    rig.start().await;
    let mut events = Events::of(&rig).await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let named = events
        .collect(9, |kind| match kind {
            EventKind::RoundOpened { round, .. } => Some(("round_opened", vec![round.clone()])),
            EventKind::ItemStatus { round, .. } => Some(("item_status", vec![round.clone()])),
            EventKind::RunStarted { rounds, .. } => Some(("run_started", rounds.clone())),
            EventKind::RunEnded { rounds, .. } => Some(("run_ended", rounds.clone())),
            EventKind::PullRequestOpened { round, .. } => Some(("pull_request_opened", vec![round.clone()])),
            EventKind::ReviewSubmitted { round, .. } => Some(("review_submitted", vec![round.clone()])),
            EventKind::RoundClosed { round, .. } => Some(("round_closed", vec![round.clone()])),
            _ => None,
        })
        .await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let round = rounds_of_team(&state)[0].0.clone();
    for (kind, rounds) in &named {
        assert!(rounds.iter().all(|found| *found == round), "{kind}: {rounds:?}");
    }
    let kinds: Vec<&str> = named.iter().map(|(kind, _)| *kind).collect();
    assert!(
        kinds.contains(&"round_opened") && kinds.contains(&"item_status") && kinds.contains(&"run_started"),
        "{kinds:?}"
    );
    assert!(
        named
            .iter()
            .any(|(kind, rounds)| *kind == "run_started" && rounds == std::slice::from_ref(&round)),
        "{named:?}"
    );
}

#[tokio::test]
async fn a_coordinator_a_member_and_the_reviewers_name_the_rounds_of_their_roles() {
    let mut rig = team_rig().await;
    rig.start().await;

    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], CHAIN, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let state = rig
        .item_where("1.2", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    let rounds = rounds_of_team(&state);
    let (first, second) = (rounds[0].0.clone(), rounds[1].0.clone());
    assert_eq!(
        rounds.iter().map(|(_, _, end)| *end).collect::<Vec<_>>(),
        [Some(RoundEnd::Merged); 2].map(Some).map(Option::flatten)
    );
    // A member: the round of its item — the first run is for item 1.1, a later one for 1.2.
    assert_eq!(rounds_of_run(&state, &tasks[1], 0), std::slice::from_ref(&first));
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.runs().last().unwrap().rounds(), std::slice::from_ref(&second));
    // The coordinator: every round that is open when its run starts — only the first at the start of the team, only the
    // second when the first is merged.
    let coordinator = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(coordinator.runs()[0].rounds(), std::slice::from_ref(&first));
    assert_eq!(
        coordinator.runs().last().unwrap().rounds(),
        std::slice::from_ref(&second)
    );
    // A reviewer: the round of the pull request it reviews, each time.
    for reviewer in found.reviewers() {
        let named: Vec<Vec<RoundId>> = reviewer.runs().iter().map(|run| run.rounds().to_vec()).collect();
        assert_eq!(
            named,
            [vec![first.clone()], vec![second.clone()]],
            "{}",
            reviewer.agent()
        );
    }
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

#[tokio::test]
async fn a_coordinator_a_member_and_a_worker_on_a_two_item_list_open_and_close_two_rounds_in_order_and_each_run_names_its_rounds()
 {
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
    // The member has a run for item 1.1 when it asks for the worker.
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
    let state = rig
        .state_where("both items done and the worker to have served the request", |state| {
            let worker_runs = state.find_task(&tasks[2]).map_or(0, |found| found.member.runs().len());
            worker_runs >= 2
                && state.teams()[0]
                    .item("1.2".parse().unwrap())
                    .is_some_and(|progress| matches!(progress.phase(), ItemPhase::Done { .. }))
        })
        .await;
    rig.stop().await.unwrap();

    let rounds = rounds_of_team(&state);
    assert_eq!(
        rounds
            .iter()
            .map(|(_, item, end)| (item.as_deref(), *end))
            .collect::<Vec<_>>(),
        [
            (Some("1.1"), Some(RoundEnd::Merged)),
            (Some("1.2"), Some(RoundEnd::Merged))
        ]
    );
    // The first run of the worker is its start; the second serves the request of the latest run of the member at that time,
    // whichever it was, and works for the rounds that run worked for.
    assert_eq!(rounds_of_run(&state, &tasks[2], 0), []);
    let served = rounds_of_run(&state, &tasks[2], 1);
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert!(
        member
            .runs()
            .iter()
            .any(|run| !run.rounds().is_empty() && run.rounds() == served),
        "{served:?}"
    );
    assert!(state.team(&team).unwrap().rounds()[0].opened_at() <= state.team(&team).unwrap().rounds()[1].opened_at());
}

#[tokio::test]
async fn a_fix_pass_stays_in_its_round() {
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

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    assert_eq!(found.rounds().len(), 1, "a fix pass is no new round");
    let round = found.rounds()[0].id().clone();
    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert!(lead.runs().len() >= 2, "the lead had a first run and a fix pass");
    assert!(lead.runs().iter().all(|run| run.rounds() == [round.clone()]));
    assert_eq!(found.rounds()[0].ended(), Some(RoundEnd::Merged));
}

#[tokio::test]
async fn with_no_merge_a_round_is_closed_as_approved_when_the_reviews_approve_and_stays_so_when_the_human_merges() {
    let mut rig = solo_rig().await;
    rig.start().await;
    let mut events = Events::of(&rig).await;

    rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let told = events.collect(2, round_event).await;
    let approved = rig
        .state_where("the round to be approved", |state| {
            rounds_of_team(state)
                .iter()
                .all(|(_, _, end)| *end == Some(RoundEnd::Approved))
        })
        .await;
    rig.notification_with("is approved").await;
    rig.github.merged_by_a_human(1);
    let merged = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert_eq!(told, ["opened 1.1", "closed 1.1 approved"]);
    assert!(matches!(
        approved.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Approved { .. }
    ));
    assert_eq!(rounds_of_team(&merged).len(), 1);
    assert_eq!(rounds_of_team(&merged)[0].2, Some(RoundEnd::Approved));
}

#[tokio::test]
async fn a_daemon_stopped_in_the_middle_of_a_round_goes_on_with_the_same_round_id() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let before = rig
        .item_where(
            "1.1",
            "asked for reviews",
            |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
        )
        .await;
    rig.stop().await.unwrap();

    rig.start().await;
    let after = rig
        .item_where("1.1", "still in review", |phase| {
            matches!(phase, ItemPhase::InReview { .. })
        })
        .await;
    rig.stop().await.unwrap();

    assert_eq!(rounds_of_team(&before).len(), 1, "the second item waits for the first");
    assert_eq!(rounds_of_team(&after), rounds_of_team(&before));
    assert_eq!(after.teams()[0].open_rounds().count(), 1);
}

#[tokio::test]
async fn agent_run_prints_the_round_of_each_item_and_agent_status_of_a_round_shows_its_item_tasks_and_pull_request() {
    let mut rig = team_rig().await;
    rig.start().await;
    let request = agent_core::protocol::RunRequest {
        repo: "example".parse().unwrap(),
        checkout: agent_core::config::AbsolutePath::try_from(rig.repo.to_str().unwrap().to_owned()).unwrap(),
        agents: agent_core::protocol::AgentList::try_from(vec![
            "inv-lead".parse().unwrap(),
            "claude-1".parse().unwrap(),
        ])
        .unwrap(),
        task_list: agent_core::state::TaskListText::new(PARALLEL.to_owned()).unwrap(),
        coordinator: Some("inv-lead".parse().unwrap()),
        merge: MergeMode::Autopilot,
    };
    let mut connection = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap();
    let mut printed = Vec::new();

    crate::commands::run::start(&mut connection, &rig.home, request, &mut printed)
        .await
        .unwrap();
    let state = rig
        .item_where("1.1", "in review", |phase| matches!(phase, ItemPhase::InReview { .. }))
        .await;
    let round = state.teams()[0].round_of("1.1".parse().unwrap()).unwrap().id().clone();
    let mut shown = Vec::new();
    crate::commands::status::status(&mut connection, Some(Target::Round(round.clone())), &mut shown)
        .await
        .unwrap();
    let mut listed = Vec::new();
    crate::commands::status::status(
        &mut connection,
        Some(Target::Team(state.teams()[0].id().clone())),
        &mut listed,
    )
    .await
    .unwrap();
    rig.stop().await.unwrap();

    let printed = String::from_utf8(printed).unwrap();
    let ids: Vec<String> = printed
        .lines()
        .filter_map(|line| line.split_whitespace().find(|word| word.starts_with("round-")))
        .map(str::to_owned)
        .collect();
    assert_eq!(ids.len(), 1, "one member takes one item at the start: {printed}");
    assert!(printed.contains(&format!("{}  item 1.1  open", ids[0])), "{printed}");
    let shown = String::from_utf8(shown).unwrap();
    assert!(shown.starts_with(&format!("{round}  item 1.1  open  team ")), "{shown}");
    assert!(shown.contains("item 1.1: One (in_review)"), "{shown}");
    assert!(shown.contains("pull request #1 on agents/inv-lead/"), "{shown}");
    assert!(shown.contains("  member  claude-1  "), "{shown}");
    assert!(shown.contains("  coordinator  inv-lead  "), "{shown}");
    let worktree = rig.worktree(&state.teams()[0].crew().roster()[1].1.task().clone());
    assert!(
        shown.contains(&format!("{}  agents/claude-1/", worktree.display())),
        "{shown}"
    );
    let listed = String::from_utf8(listed).unwrap();
    assert!(listed.contains(&format!("  {round}  item 1.1  open")), "{listed}");
}
