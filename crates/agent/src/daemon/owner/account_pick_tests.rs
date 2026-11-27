//! No agent of a paused account is picked by the refusal-driven replacement, by a hand-over of a waiting task or by
//! the lead fallback of a takeover. The pause is written into the state before the owner is built.

use agent_core::config::AccountName;
use agent_core::decide::ladder::Trouble;
use agent_core::decide::limit::Limit;
use agent_core::decide::outcome::{Pause, PauseReason};
use agent_core::kinds::Role;
use agent_core::state::{AccountPause, State};
use agent_core::status::TaskStatus;

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

// Rule: a waiting task is not handed over to an investigator whose account is paused, as it is not to an agent.
#[tokio::test]
async fn an_investigator_of_a_paused_account_takes_no_waiting_task_over() {
    let mut state = solo_state();
    state.start_waiting(&task(1), at(0)).unwrap();
    pause(&mut state, "inv-paused");
    let daemon = Daemon::new(&state, None);
    let path = daemon.home.config_file();
    let text = std::fs::read_to_string(&path).unwrap();
    let header = "[investigators.inv-1]\ncli = \"claude\"\n";
    assert!(text.contains(header), "{text}");
    let text = text.replace(header, &format!("{header}account = \"inv-paused\"\n"));
    std::fs::write(
        &path,
        format!(
            "{text}\n[accounts.inv-paused]\nowner = \"inv\"\n[replacement]\nagents = [\"inv-1\"]\nbusy_after = 1\n"
        ),
    )
    .unwrap();
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner.hand_over_busy_wait(&task(1)).await.unwrap();

    assert!(!changed, "an investigator of a paused account is not free");
    assert_eq!(
        owner.parts.state.find_task(&task(1)).unwrap().member.agent().name(),
        "claude-1"
    );
}

// Rule: with `mode = "lead"` the work of a member goes to its lead — unless the lead's account is paused: nobody is
// picked, and the member's task keeps its holder.
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

    assert_eq!(takeover, Takeover::Nobody);
    assert_eq!(
        owner.parts.state.find_task(&task(2)).unwrap().member.agent().name(),
        "claude-2"
    );
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
