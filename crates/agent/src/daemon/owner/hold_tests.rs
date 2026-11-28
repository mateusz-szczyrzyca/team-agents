//! The hold of an account at its `hold_at`: a reading at or above it starts no new run of the account until the
//! window resets, the runs going finish, the queued ones wait, the human is told once, and nothing fails. Agents of
//! other accounts are not held, and a restart of the daemon keeps the hold.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use agent_core::config::AccountName;
use agent_core::ids::AgentName;
use agent_core::limit_reading::{LimitReading, LimitWindow};
use agent_core::protocol::{Command, StatusRequest};
use agent_core::state::{Crew, MergeMode, ReadingOwner, State, TaskListText, Team};
use agent_core::status::TaskStatus;
use tracing_subscriber::layer::SubscriberExt;

use super::fixture::Daemon;
use crate::fixtures::{LIST, at, member, normal, repo, task, team_id};

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
    /// The decisions of `name` seen so far, each with its fields.
    fn decisions(&self, name: &str) -> Vec<BTreeMap<String, String>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|fields| {
                fields.get("event").map(String::as_str) == Some("decision_taken")
                    && fields.get("decision").map(String::as_str) == Some(name)
            })
            .cloned()
            .collect()
    }
}

/// A solo team with the id `id` whose task `n` is run by the normal agent `agent`.
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

/// Three queued solo tasks: 1 of `claude-1` and 2 of `claude-2`, which share the account `claude-main`, and 3 of `other-1`
/// of the account `other-main`.
fn three_tasks() -> State {
    let mut state = State::new();
    for (id, n, agent) in [(0, 1, "claude-1"), (1, 2, "claude-2"), (2, 3, "other-1")] {
        state.insert_team(solo_team(id, n, agent)).unwrap();
    }
    state
}

/// Gives the agents their accounts: `claude-1` and `claude-2` share `claude-main`, which holds at 0.5 and has room for
/// one run, so that task 2 waits for task 1; `other-1` has `other-main`, held at 0.5 too.
fn configure(daemon: &Daemon) {
    let path = daemon.home.config_file();
    let mut text = std::fs::read_to_string(&path).unwrap();
    for agent in ["claude-1", "claude-2"] {
        let header = format!("[agents.{agent}]\ncli = \"claude\"\n");
        assert!(text.contains(&header), "{text}");
        text = text.replace(&header, &format!("{header}account = \"claude-main\"\n"));
    }
    let tiers = "normal = [\"claude-1\", \"claude-2\"]";
    assert!(text.contains(tiers), "{text}");
    let text = text.replace(tiers, "normal = [\"claude-1\", \"claude-2\", \"other-1\"]");
    std::fs::write(
        &path,
        format!(
            "{text}\n[agents.other-1]\ncli = \"claude\"\naccount = \"other-main\"\n\
             [accounts.claude-main]\nowner = \"claude\"\nmax_runs = 1\nhold_at = 0.5\n\
             [accounts.other-main]\nowner = \"other\"\nhold_at = 0.5\n"
        ),
    )
    .unwrap();
}

/// Makes the repository of the worktree of task `n` one the run can work in, which the fake CLI does `scenario` in.
fn prepare(daemon: &Daemon, n: u32, scenario: &str) {
    let worktree = daemon.scenario(n, scenario);
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["commit", "-q", "--allow-empty", "-m", "Start"],
    ] {
        let done = std::process::Command::new("git")
            .current_dir(&worktree)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    }
}

/// A daemon with [`three_tasks`]: task 1 shows a reading of 0.62 of the five-hour window and the seven-day window used by
/// 0.41, both until the next day, and the other two do their work.
fn daemon_with_three_tasks() -> Daemon {
    let daemon = Daemon::new(&three_tasks(), None);
    configure(&daemon);
    prepare(&daemon, 1, "reading");
    prepare(&daemon, 2, "done");
    prepare(&daemon, 3, "done");
    daemon
}

/// A pause of the clock of the daemon beyond the reset of the readings of the stand-in: 17 hours from the clock's start.
const PAST_THE_RESET: time::Duration = time::Duration::hours(17);

// Rule: a reading at `hold_at` holds the account: the run that showed it finishes, the next run of the account waits
// and is not failed, one notification tells the hold, an agent of another account starts, and the held run starts after
// the reset.
#[tokio::test]
async fn reading_above_hold_at_holds_the_account() {
    let mut daemon = daemon_with_three_tasks();
    daemon.start().await;

    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.status_is(3, TaskStatus::Waiting).await;
    let told = daemon.notification_with("claude-main is held").await;
    after_an_admission_pass(&daemon).await;

    let state = daemon.state();
    assert_eq!(state.find_task(&task(2)).unwrap().member.status(), TaskStatus::Queued);
    assert_eq!(state.find_task(&task(2)).unwrap().member.runs().len(), 0);
    assert!(told.contains("claude-1") && told.contains("claude-2"), "{told}");
    let told_once = daemon
        .notifications()
        .iter()
        .filter(|text| text.contains("claude-main is held"))
        .count();
    assert_eq!(
        told_once,
        1,
        "one notification tells the hold: {:?}",
        daemon.notifications()
    );
    daemon.clock.advance(PAST_THE_RESET);
    daemon.status_is(2, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();
}

// Rule: the hold follows from the readings the state keeps, so a daemon that starts again keeps it, tells it once
// only, and lets the held run start after the reset.
#[tokio::test]
async fn restart_keeps_the_hold() {
    let mut daemon = daemon_with_three_tasks();
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.notification_with("claude-main is held").await;
    daemon.stop().await.unwrap();

    daemon.start().await;
    after_an_admission_pass(&daemon).await;

    let state = daemon.state();
    assert_eq!(state.find_task(&task(2)).unwrap().member.status(), TaskStatus::Queued);
    let told = daemon
        .notifications()
        .iter()
        .filter(|text| text.contains("claude-main is held"))
        .count();
    assert_eq!(told, 1, "{:?}", daemon.notifications());
    daemon.clock.advance(PAST_THE_RESET);
    daemon.status_is(2, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();
}

/// A state of one task of `claude-1` with a reading of `used` of the five-hour window of the account `claude-main` that
/// resets an hour after the clock's start.
fn state_with_reading(used: f64) -> State {
    let mut state = State::new();
    state.insert_team(solo_team(0, 1, "claude-1")).unwrap();
    state.keep_limit_readings(
        &ReadingOwner::Account("claude-main".parse().unwrap()),
        &[LimitReading::new(LimitWindow::FiveHour, used, at(3600)).unwrap()],
        at(0),
        at(0),
    );
    state
}

/// The agents of `names` as the owner of `daemon` would pick them at the start of the clock.
async fn pickable(daemon: &Daemon, names: &[&str]) -> Vec<String> {
    let (owner, _sinks) = daemon.bare_owner().await;
    let names: Vec<AgentName> = names.iter().map(|name| name.parse().unwrap()).collect();
    owner
        .not_of_paused_accounts(&names, at(0))
        .iter()
        .map(|name| name.as_str().to_owned())
        .collect()
}

// Rule: an agent of a held account is picked for nothing — no replacement, no hand-over, no worker for a request —
// like one of a paused account; an agent of another account or without one is.
#[tokio::test]
async fn an_agent_of_a_held_account_is_not_picked() {
    let daemon = Daemon::new(&state_with_reading(0.6), None);
    configure(&daemon);

    let free = pickable(&daemon, &["claude-1", "claude-2", "other-1"]).await;

    assert_eq!(free, ["other-1"]);
    let (owner, _sinks) = daemon.bare_owner().await;
    let unavailable: Vec<String> = owner
        .unavailable_agents_at(at(0))
        .iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(unavailable, ["claude-1", "claude-2"]);
}

// Rule: a reading below `hold_at` holds nobody.
#[tokio::test]
async fn a_reading_below_hold_at_picks_everybody() {
    let daemon = Daemon::new(&state_with_reading(0.49), None);
    configure(&daemon);

    let free = pickable(&daemon, &["claude-1", "claude-2", "other-1"]).await;

    assert_eq!(free, ["claude-1", "claude-2", "other-1"]);
}

// Rule: the log says once when a hold begins and once when it ends, and the human is told once, whatever number of
// times the holds are looked at.
#[tokio::test]
async fn a_hold_is_logged_when_it_begins_and_when_it_ends_and_told_once() {
    let daemon = Daemon::new(&state_with_reading(0.6), None);
    configure(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let events = Events::default();
    let _logged = tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));

    owner.update_holds(at(0)).await;
    owner.update_holds(at(1)).await;
    owner.update_holds(at(3600)).await;
    owner.update_holds(at(3601)).await;

    let begun = events.decisions("account_held");
    assert_eq!(begun.len(), 1);
    assert_eq!(begun[0]["account"], "claude-main");
    let released = events.decisions("account_released");
    assert_eq!(released.len(), 1);
    assert_eq!(released[0]["account"], "claude-main");
    let told: Vec<String> = daemon
        .notifications()
        .into_iter()
        .filter(|text| text.contains("claude-main is held"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].contains("claude-1") && told[0].contains("claude-2"), "{told:?}");
}

// Rule: a daemon that starts on a state that records the hold of an account does not tell the hold again: it began
// before.
#[tokio::test]
async fn a_hold_that_began_before_a_restart_is_not_told_again() {
    let mut state = state_with_reading(0.6);
    state.record_account_hold(claude_main(), Some(at(3600)));
    let daemon = Daemon::new(&state, None);
    configure(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.update_holds(at(1)).await;

    assert_eq!(daemon.notifications(), Vec::<String>::new());
}

/// The account of these tests.
fn claude_main() -> AccountName {
    "claude-main".parse().unwrap()
}

/// Lets the daemon go through its queue once more after the state a test has seen: a request is answered between two
/// passes over the queue, so after two answers a pass that began after that state has been made.
async fn after_an_admission_pass(daemon: &Daemon) {
    for _ in 0..2 {
        daemon
            .ask(Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
    }
}

// Rule: a hold begins when a reading reaches `hold_at` and lasts until the window resets, whatever the readings read
// after it say; the state records it.
#[tokio::test]
async fn a_lower_reading_does_not_end_a_hold_before_its_reset() {
    let daemon = Daemon::new(&state_with_reading(0.6), None);
    configure(&daemon);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let events = Events::default();
    let _logged = tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));

    owner.update_holds(at(0)).await;
    assert_eq!(owner.parts.state.account_hold_until(&claude_main()), Some(at(3600)));
    owner.parts.state.keep_limit_readings(
        &ReadingOwner::Account(claude_main()),
        &[LimitReading::new(LimitWindow::FiveHour, 0.4, at(3600)).unwrap()],
        at(1),
        at(1),
    );
    owner.update_holds(at(1)).await;

    assert_eq!(owner.held_account_names(at(1)), BTreeSet::from([claude_main()]));
    assert_eq!(events.decisions("account_released").len(), 0);
    owner.update_holds(at(3600)).await;
    assert_eq!(owner.held_account_names(at(3600)), BTreeSet::new());
    assert_eq!(owner.parts.state.account_hold_until(&claude_main()), None);
    assert_eq!(events.decisions("account_released").len(), 1);
}

// Rule: a hold the daemon meets for the first time when it starts — `hold_at` was added to a configuration that has a
// reading above it — is told once, and not again when the daemon starts once more.
#[tokio::test]
async fn a_hold_first_met_at_startup_is_told_once_and_not_again_after_a_restart() {
    let mut daemon = Daemon::new(&state_with_reading(0.6), None);
    configure(&daemon);
    daemon.start().await;
    daemon.notification_with("claude-main is held").await;
    daemon.stop().await.unwrap();

    daemon.start().await;
    after_an_admission_pass(&daemon).await;

    let told = daemon
        .notifications()
        .iter()
        .filter(|text| text.contains("claude-main is held"))
        .count();
    assert_eq!(told, 1, "{:?}", daemon.notifications());
    assert_eq!(
        daemon.state().find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Queued
    );
    assert_eq!(daemon.state().account_hold_until(&claude_main()), Some(at(3600)));
    daemon.stop().await.unwrap();
}
