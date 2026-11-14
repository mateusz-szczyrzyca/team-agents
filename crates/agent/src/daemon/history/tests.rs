//! What the daemon makes of a team, a round and the health of the history: the plan it records agrees with the team it
//! formed for any draw (a seeded source: a failure names its seed), the item of a round is the item of the list in full.

use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::{AbsolutePath, Config};
use agent_core::history::{HistoryHealth, HistoryState, HowChosen, Record, RoundSubject};
use agent_core::ids::AgentName;
use agent_core::kinds::Role;
use agent_core::protocol::{AgentList, RunRequest};
use agent_core::state::{MergeMode, TaskListText};
use agent_runtime::history::{History, OpenError, Problem, RecordKind, Reporter, Setup, Store, WriteError};
use agent_runtime::store::Home;
use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;

use super::{health_to_tell, round_opening, team_start};
use crate::daemon::team::{Formed, form};
use crate::testing::ManualClock;

/// Two normal agents, a worker and three investigators.
const CONFIG: &str = "\
[agents.a1]\ncli = \"claude\"\n[agents.a2]\ncli = \"codex\"\n[agents.w1]\ncli = \"opencode\"\n\
[investigators.i1]\ncli = \"claude\"\n[investigators.i2]\ncli = \"claude\"\n[investigators.i3]\ncli = \"claude\"\n[investigators.i4]\ncli = \"claude\"\n\
[tiers]\nnormal = [\"a1\", \"a2\"]\nworkers = [\"w1\"]\n[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n";

/// A list with two items, the first with every part.
const LIST: &str = "\
Build the thing.

1. First
[ ] 1.1 The first item
    Do the first thing.
    Expected result:
    - the first thing works
    Checks:
    - run it
[ ] 1.2 The second item
    Do the second thing.
";

/// A list without items.
const PLAIN: &str = "Fix the typo in the README.\n";

fn config() -> Config {
    Config::from_toml_str(CONFIG).unwrap()
}

fn request(agents: &[&str], list: &str, coordinator: Option<&str>) -> RunRequest {
    RunRequest {
        repo: "example".parse().unwrap(),
        checkout: AbsolutePath::from_str("/work/example").unwrap(),
        agents: AgentList::try_from(
            agents
                .iter()
                .map(|name| name.parse::<AgentName>().unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap(),
        task_list: TaskListText::new(list.to_owned()).unwrap(),
        coordinator: coordinator.map(|name| name.parse().unwrap()),
        merge: MergeMode::Autopilot,
    }
}

fn formed(request: &RunRequest, seed: u64) -> Formed {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
    let home = Home::new(&AbsolutePath::from_str("/srv/agents").unwrap());
    let clock = ManualClock::new(crate::fixtures::at(0));
    form(request, &config(), &home, &clock, &mut rng).unwrap()
}

#[test]
fn the_plan_of_a_team_names_the_coordinator_it_has_and_the_agents_it_was_drawn_among_for_any_draw() {
    let request = request(&["a1", "i1", "a2", "i2"], LIST, None);
    for seed in 0..200 {
        let formed = formed(&request, seed);

        let start = team_start(&request, &formed.team, &formed.base, &config()).unwrap();

        let coordinator = formed.team.crew().roster()[0].1.agent().name().to_owned();
        let entry = start.plan.iter().find(|entry| entry.role == Role::Coordinator).unwrap();
        assert_eq!(entry.agent.as_str(), coordinator, "seed {seed}");
        assert_eq!(
            entry.how,
            HowChosen::Drawn {
                among: ["i1", "i2"].map(|name| name.parse().unwrap()).to_vec()
            },
            "seed {seed}"
        );
        assert_eq!(start.plan.len(), 4, "seed {seed}");
        assert_eq!(start.team, *formed.team.id());
        assert_eq!(start.at, formed.team.created_at());
        assert_eq!(start.base_branch.as_str(), "main");
        assert_eq!((start.repo.as_str(), start.merge), ("example", MergeMode::Autopilot));
        assert_eq!(start.task_list, LIST);
    }
}

#[test]
fn the_flag_wins_over_the_line_and_a_solo_agent_is_named() {
    let flagged = request(&["a1", "a2"], LIST, Some("a2"));
    let solo = request(&["a1"], LIST, None);

    let by_flag = team_start(
        &flagged,
        &formed(&flagged, 1).team,
        &formed(&flagged, 1).base,
        &config(),
    )
    .unwrap();
    let alone = team_start(&solo, &formed(&solo, 1).team, &formed(&solo, 1).base, &config()).unwrap();

    let coordinator = by_flag
        .plan
        .iter()
        .find(|entry| entry.role == Role::Coordinator)
        .unwrap();
    assert_eq!(
        (coordinator.agent.as_str(), &coordinator.how),
        ("a2", &HowChosen::NamedByFlag)
    );
    assert_eq!(
        (alone.plan[0].role, &alone.plan[0].how),
        (Role::Solo, &HowChosen::Named)
    );
}

#[test]
fn a_round_of_an_item_is_that_item_in_full_and_the_round_of_a_list_without_items_is_its_brief() {
    let with_items = formed(&request(&["a1"], LIST, None), 3);
    let without = formed(&request(&["a1"], PLAIN, None), 3);

    let item = round_opening(&with_items.team, &with_items.team.rounds()[0]).unwrap();
    let brief = round_opening(&without.team, &without.team.rounds()[0]).unwrap();

    let RoundSubject::Item(item_text) = &item.subject else {
        panic!("{item:?}")
    };
    assert_eq!(item_text.item.to_string(), "1.1");
    assert_eq!(item_text.title, "The first item");
    assert_eq!(item_text.description, "Do the first thing.");
    assert_eq!(item_text.expected_result.as_deref(), Some("- the first thing works"));
    assert_eq!(item_text.checks.as_deref(), Some("- run it"));
    assert_eq!(
        (item.round.clone(), item.team.clone(), item.at),
        (
            with_items.team.rounds()[0].id().clone(),
            with_items.team.id().clone(),
            with_items.team.rounds()[0].opened_at()
        )
    );
    assert_eq!(
        brief.subject,
        RoundSubject::Brief("Fix the typo in the README.".to_owned())
    );
}

/// A store that refuses everything.
struct Refusing;

impl Store for Refusing {
    fn write(&mut self, _records: &[Record]) -> Result<(), WriteError> {
        Err(WriteError::Rule {
            record: RecordKind::Step,
            rule: "refused for the test",
        })
    }
}

/// A reporter that says nothing.
#[derive(Debug)]
struct Silent;

impl Reporter for Silent {
    fn report(&self, _problem: &Problem) {}
}

fn setup() -> Setup {
    Setup {
        reporter: Arc::new(Silent),
        capacity: std::num::NonZeroUsize::new(8).unwrap(),
    }
}

#[test]
fn status_tells_of_the_history_only_when_it_lost_records_or_cannot_be_written() {
    let whole = History::start(Accepting, setup()).unwrap();
    let broken = History::start(Refusing, setup()).unwrap();
    let error: OpenError = OpenError::MigrationOrder { found: 2, expected: 1 };
    let unavailable = History::unavailable(&error, Arc::new(Silent));
    let record = whole_record();
    broken.history.record(record);
    broken.history.flush();
    whole.history.flush();

    assert_eq!(health_to_tell(&History::disabled()), None);
    assert_eq!(health_to_tell(&whole.history), None);
    assert_eq!(
        health_to_tell(&broken.history),
        Some(HistoryHealth {
            state: HistoryState::Writing,
            lost: 1
        })
    );
    assert!(matches!(
        health_to_tell(&unavailable),
        Some(HistoryHealth {
            state: HistoryState::Unavailable { .. },
            lost: 0
        })
    ));
    whole.writer.close().unwrap();
    broken.writer.close().unwrap();
}

/// A store that accepts everything.
struct Accepting;

impl Store for Accepting {
    fn write(&mut self, _records: &[Record]) -> Result<(), WriteError> {
        Ok(())
    }
}

/// Any record.
fn whole_record() -> Record {
    use agent_core::history::Step;
    Record::Step(Box::new(Step {
        run: "run-20261112-100001-r1r1".parse().unwrap(),
        number: std::num::NonZeroU32::MIN,
        at: crate::fixtures::at(1),
        event: agent_core::cli::ActivityEvent::Info("x".to_owned()),
    }))
}
