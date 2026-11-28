//! No agent of a paused account is picked by the refusal-driven replacement, by a hand-over of a waiting task or by
//! the lead fallback of a takeover. The pause is written into the state before the owner is built.

use agent_core::config::AccountName;
use agent_core::decide::ladder::Trouble;
use agent_core::decide::limit::Limit;
use agent_core::decide::outcome::{Pause, PauseReason};
use agent_core::kinds::Role;
use agent_core::state::{AccountPause, State};
use agent_core::status::TaskStatus;

use super::Step;
use super::fixture::{Daemon, pair_state};
use super::takeover::Takeover;
use crate::daemon::notice::Notice;
use crate::daemon::settle::Settled;
use crate::fixtures::{at, solo_state, task};

/// A moment so far ahead that no test reaches it.
const FAR: &str = "2100-01-01T00:00:00Z";

/// Pauses `account` in `state` until [`FAR`].
fn pause(state: &mut State, account: &str) {
    let account: AccountName = account.parse().unwrap();
    state.record_account_pause(
        account,
        Some(AccountPause {
            until: FAR.parse().unwrap(),
            reason: Limit::RateLimit,
        }),
    );
}

/// Overwrites the daemon's config file with `toml`.
fn write_config(daemon: &Daemon, toml: &str) {
    std::fs::write(daemon.home.config_file(), toml).unwrap();
}

/// The name of the agent that took over `task`, after a refusal of its holder.
async fn refused_and_replaced(daemon: &Daemon, state_task: u32) -> String {
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let located = owner.locate(&task(state_task)).unwrap();
    let result = owner
        .keep_refusal(
            &task(state_task),
            &located,
            &Settled::Paused(Pause {
                until: at(10),
                reason: PauseReason::RateLimit,
                kind: agent_core::decide::refusal::Kind::RateLimit,
                evidence: String::new(),
            }),
            &agent_core::state::RunEnd::RateLimited,
            at(0),
        )
        .unwrap()
        .expect("a free agent takes the task over");
    assert_eq!(result.status, TaskStatus::Queued);
    let Notice::AgentChanged { new_agent, .. } = &result.notice else {
        panic!("expected AgentChanged, got {:?}", result.notice);
    };
    new_agent.as_str().to_owned()
}

// Rule: a task whose agent was refused goes to the first agent of another provider whose account is not paused:
// `codex-1` would be the first listed, but its account is paused.
#[tokio::test]
async fn a_refused_task_is_not_given_to_an_agent_of_a_paused_account() {
    let mut state = solo_state();
    state.record_limit_refusals(task(1), 1);
    pause(&mut state, "codex-paused");
    let daemon = Daemon::new(&state, None);
    write_config(
        &daemon,
        "[agents.claude-1]\ncli = \"claude\"\n\
         [agents.codex-1]\ncli = \"codex\"\naccount = \"codex-paused\"\n\
         [agents.codex-2]\ncli = \"codex\"\n\
         [accounts.codex-paused]\nowner = \"codex\"\n\
         [tiers]\nnormal = [\"claude-1\", \"codex-1\", \"codex-2\"]\n\
         [binaries]\nclaude = \"fake\"\ncodex = \"fake\"\n\
         [replacement]\nagents = [\"codex-1\", \"codex-2\"]\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    );

    assert_eq!(refused_and_replaced(&daemon, 1).await, "codex-2");
}

// Rule: the investigators are the last resort of a refused coordinator; one of a paused account is not taken.
#[tokio::test]
async fn a_refused_coordinator_is_not_given_to_an_investigator_of_a_paused_account() {
    let mut state = pair_state();
    state.record_limit_refusals(task(1), 1);
    pause(&mut state, "inv-paused");
    let daemon = Daemon::new(&state, None);
    write_config(
        &daemon,
        "[agents.claude-1]\ncli = \"claude\"\n\
         [agents.claude-2]\ncli = \"claude\"\n\
         [investigators.inv-2]\ncli = \"codex\"\naccount = \"inv-paused\"\n\
         [investigators.inv-3]\ncli = \"codex\"\n\
         [accounts.inv-paused]\nowner = \"inv\"\n\
         [tiers]\nnormal = [\"claude-1\", \"claude-2\"]\n\
         [binaries]\nclaude = \"fake\"\ncodex = \"fake\"\n\
         [replacement]\nagents = [\"inv-2\", \"inv-3\"]\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    );

    assert_eq!(refused_and_replaced(&daemon, 1).await, "inv-3");
}

/// A solo task held by the investigator `inv-held`, waiting past `busy_after` for its busy agent, with the free
/// investigator `inv-1` as the only agent of `[replacement]`. Tiers must match for a hand-over, so the holder is an
/// investigator too. With `paused` the account of `inv-1` is paused; returns whether the task changed hands and the name
/// of its holder.
async fn busy_wait_of_an_investigator(paused: bool) -> (bool, String) {
    let crew =
        agent_core::state::Crew::solo(crate::fixtures::member(1, crate::fixtures::investigator("inv-held"))).unwrap();
    let team = agent_core::state::Team::new(
        crate::fixtures::team_id(0),
        crate::fixtures::repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        agent_core::state::TaskListText::new(crate::fixtures::LIST.to_owned()).unwrap(),
        crew,
    );
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state.start_waiting(&task(1), at(0)).unwrap();
    if paused {
        pause(&mut state, "inv-paused");
    }
    let daemon = Daemon::new(&state, None);
    write_config(
        &daemon,
        "[investigators.inv-held]\ncli = \"claude\"\n\
         [investigators.inv-1]\ncli = \"claude\"\naccount = \"inv-paused\"\n\
         [accounts.inv-paused]\nowner = \"inv\"\n\
         [binaries]\nclaude = \"fake\"\n\
         [replacement]\nagents = [\"inv-1\"]\nbusy_after = 1\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
    );
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task(1)).await.unwrap();

    let holder = owner
        .parts
        .state
        .find_task(&task(1))
        .unwrap()
        .member
        .agent()
        .name()
        .to_owned();
    (changed, holder)
}

// Rule: a waiting task is handed over to a free investigator of the holder's tier: the setup of the next test.
#[tokio::test]
async fn a_waiting_task_is_handed_to_a_free_investigator_of_an_account_that_runs() {
    assert_eq!(busy_wait_of_an_investigator(false).await, (true, "inv-1".to_owned()));
}

// Rule: a waiting task is not handed over to an investigator whose account is paused, as it is not to an agent.
#[tokio::test]
async fn an_investigator_of_a_paused_account_takes_no_waiting_task_over() {
    assert_eq!(busy_wait_of_an_investigator(true).await, (false, "inv-held".to_owned()));
}

// Rule: with `mode = "lead"` the work of a member goes to its lead — unless the lead's account is paused: nobody is
// picked, the member's task keeps its holder and waits until the reset, not failed.
#[tokio::test]
async fn a_lead_of_a_paused_account_is_not_given_the_work_of_a_member() {
    let mut state = pair_state();
    pause(&mut state, "claude-paused");
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.claude-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"claude-paused\"\n"));
    std::fs::write(
        &path,
        format!("{text}\n[accounts.claude-paused]\nowner = \"claude\"\n[replacement]\nmode = \"lead\"\n"),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let takeover = owner.take_over(&task(2), Role::Member, Trouble::NoStatus).unwrap();

    assert_eq!(takeover, Takeover::Wait);
    assert_eq!(owner.wakes.get(&task(2)), Some(&FAR.parse().unwrap()));
    assert_eq!(
        owner.parts.state.find_task(&task(2)).unwrap().member.agent().name(),
        "claude-2"
    );
}

// Rule: a member's task that has no step of the ladder left and whose lead's account is paused waits — the ladder
// answers `Waits`, the task is not failed and is woken at the end of the lead's pause.
#[tokio::test]
async fn a_member_with_no_step_left_waits_for_a_lead_of_a_paused_account() {
    let mut state = pair_state();
    pause(&mut state, "claude-paused");
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.claude-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"claude-paused\"\n"));
    std::fs::write(
        &path,
        format!(
            "{text}\n[accounts.claude-paused]\nowner = \"claude\"\n[replacement]\nmode = \"lead\"\nnew_sessions = 0\n"
        ),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let step = owner.climb(&task(2), Role::Member, Trouble::Failed, 1).unwrap();

    assert_eq!(step, Step::Waits);
    assert_eq!(owner.wakes.get(&task(2)), Some(&FAR.parse().unwrap()));
}

// Rule: the agents that take nothing new at a moment are those of paused accounts, investigators included.
#[tokio::test]
async fn the_unavailable_agents_include_the_investigators_of_a_paused_account() {
    let mut state = solo_state();
    pause(&mut state, "inv-paused");
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[investigators.inv-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"inv-paused\"\n"));
    std::fs::write(&path, format!("{text}\n[accounts.inv-paused]\nowner = \"inv\"\n")).unwrap();
    let (owner, _sinks) = daemon.bare_owner().await;

    let unavailable = owner.unavailable_agents_at(at(0));

    let names: Vec<&str> = unavailable.iter().map(agent_core::ids::AgentName::as_str).collect();
    assert_eq!(names, ["inv-1"]);
}

// Rule: a member's task that waits for the account of its lead is woken, after a restart of the daemon, at the end of
// the lead's pause — not at once because its own agent's account is not paused.
#[tokio::test]
async fn a_task_waiting_for_a_lead_of_a_paused_account_is_woken_at_the_reset_after_a_restart() {
    let mut state = pair_state();
    pause(&mut state, "claude-paused");
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[agents.claude-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"claude-paused\"\n"));
    std::fs::write(
        &path,
        format!("{text}\n[accounts.claude-paused]\nowner = \"claude\"\n[replacement]\nmode = \"lead\"\n"),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    assert_eq!(
        owner.take_over(&task(2), Role::Member, Trouble::NoStatus).unwrap(),
        Takeover::Wait
    );
    let team = owner.locate(&task(2)).unwrap().team;
    owner
        .move_to(&task(2), &team, TaskStatus::Paused, &mut Vec::new())
        .unwrap();
    owner.wakes.clear();

    owner.wake_held_tasks(at(0));

    assert_eq!(owner.wakes.get(&task(2)), Some(&FAR.parse().unwrap()));
}

// Rule: a member's task that its own agent's account holds is woken, after a restart of the daemon, at the end of that
// pause, also when the account of its lead is paused until later: the lead's pause matters only to a task that waits
// for the lead.
#[tokio::test]
async fn a_task_held_for_its_own_account_is_woken_at_its_own_reset_after_a_restart() {
    let mut state = pair_state();
    pause(&mut state, "claude-paused");
    state.record_account_pause(
        "claude-own".parse().unwrap(),
        Some(AccountPause {
            until: at(10),
            reason: Limit::RateLimit,
        }),
    );
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let lead = "[agents.claude-1]\ncli = \"claude\"\n";
    let member = "[agents.claude-2]\ncli = \"claude\"\n";
    assert!(text.contains(lead) && text.contains(member), "{text}");
    let text = text
        .replace(lead, &format!("{lead}account = \"claude-paused\"\n"))
        .replace(member, &format!("{member}account = \"claude-own\"\n"));
    std::fs::write(
        &path,
        format!("{text}\n[accounts.claude-paused]\nowner = \"claude\"\n[accounts.claude-own]\nowner = \"claude\"\n"),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.hold(&task(2), at(10));
    let team = owner.locate(&task(2)).unwrap().team;
    owner
        .move_to(&task(2), &team, TaskStatus::Paused, &mut Vec::new())
        .unwrap();
    owner.wakes.clear();

    owner.wake_held_tasks(at(0));

    assert_eq!(owner.wakes.get(&task(2)), Some(&at(10)));
}
