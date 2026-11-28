//! The queue of tasks whose account has its `max_runs` going: one pass of the loop starts no more runs than the account
//! has room for, a task held back by its account does not make the tasks behind it wait for it, a task handed to another
//! agent is judged with that agent's account, and each wait is told once, in the log record of its own.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_core::state::{Crew, MergeMode, State, TaskListText, Team, TeamMembers};
use agent_core::status::TaskStatus;
use tracing_subscriber::layer::SubscriberExt;

use super::fixture::Daemon;
use crate::fixtures::{LIST, at, investigator, member, normal, repo, run, task, team_id};

/// The structured fields of the events a test looks at, as the subscriber receives them.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        /// The fields of one event.
        #[derive(Default)]
        struct Fields(BTreeMap<String, String>);
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
        self.0.lock().unwrap().push(fields.0);
    }
}

impl Events {
    /// The events of `name` seen so far, each with its fields.
    fn named(&self, name: &str) -> Vec<BTreeMap<String, String>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|fields| fields.get("event").map(String::as_str) == Some(name))
            .cloned()
            .collect()
    }
}

/// A state of one team: the coordinator `claude-1` as task 1 and one member of the agent `agent` for each number of
/// `members`; every task is queued.
fn team_of(agent: &str, members: &[u32]) -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: member(1, normal("claude-1")),
        members: members
            .iter()
            .map(|n| agent_core::state::Member::new(task(*n), normal(agent)))
            .collect(),
        workers: vec![],
    })
    .unwrap();
    let team = Team::new(
        team_id(0),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(LIST.to_owned()).unwrap(),
        crew,
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state
}

/// A team of one solo task, the number `n`, of the normal agent `agent`, with the team number `id`.
fn solo_team(id: u32, n: u32, agent: &str) -> Team {
    Team::new(
        team_id(id),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new(LIST.to_owned()).unwrap(),
        Crew::solo(member(n, normal(agent))).unwrap(),
    )
}

/// Puts the task `n` of `state` into a run that goes.
fn run_task(state: &mut State, n: u32) {
    let (_, mut member) = state.find_task_mut(&task(n)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member.begin_run(run(n), at(0), None).unwrap();
}

/// Appends `text` to the configuration of `daemon`.
fn configure(daemon: &Daemon, text: &str) {
    let path = daemon.home.config_file();
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{old}\n{text}\n")).unwrap();
}

/// Gives `agents` (of the fixture's configuration) the account `claude-main`, whose `max_runs` is 1.
fn share_one_run(daemon: &Daemon, agents: &[&str]) {
    let path = daemon.home.config_file();
    let text = agents
        .iter()
        .fold(std::fs::read_to_string(&path).unwrap(), |text, agent| {
            let header = format!("[agents.{agent}]\ncli = \"claude\"\n");
            assert!(text.contains(&header), "{text}");
            text.replace(&header, &format!("{header}account = \"claude-main\"\n"))
        });
    std::fs::write(
        &path,
        format!("{text}\n[accounts.claude-main]\nowner = \"claude\"\nmax_runs = 1\n"),
    )
    .unwrap();
}

// Rule: when no run goes, two queued tasks of an account whose `max_runs` is 1 start exactly one run in one pass.
#[tokio::test]
async fn two_queued_tasks_of_an_account_with_one_run_start_one() {
    let daemon = Daemon::new(&team_of("claude-2", &[2]), None);
    share_one_run(&daemon, &["claude-1", "claude-2"]);
    daemon.scenario(1, "done");
    daemon.scenario(2, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    let started: Vec<usize> = [1, 2]
        .iter()
        .map(|n| owner.parts.state.find_task(&task(*n)).unwrap().member.runs().len())
        .collect();
    assert_eq!(started, vec![1, 0]);
}

// Rule: a task held back by its account does not count as running: the next queued task of the same agent is held back by
// the account too, not told to wait for the agent's run that does not go.
#[tokio::test]
async fn a_task_held_back_by_its_account_is_no_holder_of_its_agent() {
    let mut state = team_of("claude-2", &[2]);
    state.insert_team(solo_team(1, 3, "claude-2")).unwrap();
    run_task(&mut state, 1);
    let daemon = Daemon::new(&state, None);
    share_one_run(&daemon, &["claude-1", "claude-2"]);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let events = Events::default();

    {
        let _logged = tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));
        owner.start_queued().await.unwrap();
    }

    assert_eq!(
        events.named("task_waits_for_agent"),
        Vec::<BTreeMap<String, String>>::new()
    );
    let waits: Vec<String> = events
        .named("decision_taken")
        .iter()
        .map(|fields| fields["task"].clone())
        .collect();
    assert_eq!(waits, vec![task(2).to_string(), task(3).to_string()]);
}

/// A state of three solo tasks: task 1 of `inv-held` is queued and has waited since the start of time, task 2 of
/// `inv-held` runs, and task 3 of `inv-2` runs.
fn busy_investigator_and_a_running_account() -> State {
    let mut state = State::new();
    for (n, agent) in [(1, "inv-held"), (2, "inv-held"), (3, "inv-2")] {
        let crew = Crew::solo(member(n, investigator(agent))).unwrap();
        let team = Team::new(
            team_id(n - 1),
            repo(),
            at(0),
            MergeMode::Autopilot,
            TaskListText::new(LIST.to_owned()).unwrap(),
            crew,
        );
        state.insert_team(team).unwrap();
    }
    state.start_waiting(&task(1), at(0)).unwrap();
    run_task(&mut state, 2);
    run_task(&mut state, 3);
    state
}

// Rule: a task handed over to a free agent is judged with the account of that agent: when its account has its most runs
// going, the task stays queued, with no run, and does not start beside them.
#[tokio::test]
async fn a_task_handed_over_to_an_agent_of_a_full_account_waits() {
    let daemon = Daemon::new(&busy_investigator_and_a_running_account(), None);
    configure(
        &daemon,
        "[investigators.inv-held]\ncli = \"claude\"\n\
         [investigators.inv-2]\ncli = \"claude\"\naccount = \"inv-shared\"\n\
         [accounts.inv-shared]\nowner = \"inv\"\nmax_runs = 1\n\
         [limits]\nruns_per_agent = 1\n\
         [replacement]\nagents = [\"inv-1\"]\nbusy_after = 1\n",
    );
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("[investigators.inv-1]\ncli = \"claude\"\n"), "{text}");
    std::fs::write(
        &path,
        text.replace(
            "[investigators.inv-1]\ncli = \"claude\"\n",
            "[investigators.inv-1]\ncli = \"claude\"\naccount = \"inv-shared\"\n",
        ),
    )
    .unwrap();
    daemon.scenario(1, "done");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    let found = owner.parts.state.find_task(&task(1)).unwrap();
    assert_eq!(
        (
            found.member.agent().name().to_owned(),
            found.member.status(),
            found.member.runs().len()
        ),
        ("inv-1".to_owned(), TaskStatus::Queued, 0)
    );
}

// Rule: the wait of a task for its account is a `decision_taken` record, `account_slot`, with the account, the runs
// that go and its limit; it is told once, not at every pass of the loop.
#[tokio::test]
async fn the_wait_for_an_account_is_told_once_as_a_decision() {
    let mut state = team_of("claude-2", &[2]);
    run_task(&mut state, 1);
    let daemon = Daemon::new(&state, None);
    share_one_run(&daemon, &["claude-1", "claude-2"]);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let events = Events::default();

    {
        let _logged = tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));
        owner.start_queued().await.unwrap();
        owner.start_queued().await.unwrap();
    }

    let told: Vec<[String; 6]> = events
        .named("decision_taken")
        .iter()
        .map(|fields| ["decision", "account", "task", "running", "max_runs", "result"].map(|key| fields[key].clone()))
        .collect();
    assert_eq!(
        told,
        vec![[
            "account_slot".to_owned(),
            "claude-main".to_owned(),
            task(2).to_string(),
            "1".to_owned(),
            "1".to_owned(),
            "wait".to_owned(),
        ]]
    );
}

// Rule: the wait for the account and the wait for the busy agent are told each on its own: a task that waited for its
// account and then waits for its agent logs `task_waits_for_agent`.
#[tokio::test]
async fn a_task_that_waited_for_its_account_still_tells_that_it_waits_for_its_agent() {
    let mut state = team_of("claude-2", &[2]);
    run_task(&mut state, 1);
    let daemon = Daemon::new(&state, None);
    share_one_run(&daemon, &["claude-1", "claude-2"]);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let events = Events::default();
    let _logged = tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));
    owner.start_queued().await.unwrap();
    assert_eq!(events.named("decision_taken").len(), 1);

    // The run of task 1 ends and a run of the agent of task 2 goes in another place: now its agent is the holder.
    {
        let (_, mut first) = owner.parts.state.find_task_mut(&task(1)).unwrap();
        first.end_run(at(1), agent_core::state::RunEnd::Stopped).unwrap();
        first.transition(TaskStatus::Done).unwrap();
    }
    let mut other = State::new();
    other.insert_team(solo_team(5, 9, "claude-2")).unwrap();
    run_task(&mut other, 9);
    let team = other.teams().first().unwrap().clone();
    owner.parts.state.insert_team(team).unwrap();
    owner.start_queued().await.unwrap();

    let holders: Vec<String> = events
        .named("task_waits_for_agent")
        .iter()
        .map(|fields| fields["holder_task"].clone())
        .collect();
    assert_eq!(holders, vec![task(9).to_string()]);
}
