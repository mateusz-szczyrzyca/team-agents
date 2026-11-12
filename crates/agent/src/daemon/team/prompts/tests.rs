use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::ids::BranchName;
use agent_core::state::Team;
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus, RequestId};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::prompts::PromptLoader;

use super::*;
use crate::daemon::team::support::{CHAIN, crew_of, solo, task_id};

/// The three loaded catalogs of this repository.
struct Texts {
    team: TeamPrompts,
    harness: HarnessPrompts,
    rules: PromptLoader,
}

async fn texts() -> Texts {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = Language::default();
    Texts {
        team: TeamPrompts::load(&dir, &language).await.unwrap(),
        harness: HarnessPrompts::load(&dir, &language).await.unwrap(),
        rules: PromptLoader::load(&dir, &language).await.unwrap(),
    }
}

fn branch(text: &str) -> BranchName {
    text.parse().unwrap()
}

fn request() -> DelegationRequest {
    DelegationRequest {
        id: RequestId::new(NonZeroU64::new(7).unwrap()),
        from: "claude-1".parse().unwrap(),
        from_task: task_id(2),
        repo: "example".parse().unwrap(),
        branch: branch("agents/claude-1/task-20261111-153002-k3x9-1.1"),
        worktree: PathBuf::from("/srv/agents/worktrees/example/task-2"),
        request: "List the files under src/.".to_owned(),
        context: "The module is small.".to_owned(),
        status: DelegationStatus::Starting,
        worker: None,
        helper_task: None,
        summary: String::new(),
    }
}

/// Shapes the prompt of the task `n` of `team`.
fn shaped(texts: &Texts, team: &Team, n: u32, why: Option<&Why>, new_session: bool) -> String {
    let (role, member) = team.find(&task_id(n)).unwrap();
    let work = branch("agents/claude-1/task-20261111-153002-k3x9-1.1");
    let base = branch("main");
    shape(
        &texts.team,
        &PromptTexts {
            harness: &texts.harness,
            rules: &texts.rules,
        },
        &TeamRun {
            team,
            role,
            member,
            branch: &work,
            base: &base,
            why,
            new_session,
        },
        "THE COMPOSED PROMPT\n".to_owned(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_task_without_a_team_keeps_its_prompt() {
    let texts = texts().await;
    let team = solo(CHAIN);

    let prompt = shaped(&texts, &team, 1, Some(&Why::Woken), true);

    assert_eq!(prompt, "THE COMPOSED PROMPT\n");
}

#[tokio::test]
async fn the_first_run_of_a_member_gets_the_team_after_its_prompt() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 1);

    let prompt = shaped(&texts, &team, 2, None, true);

    assert!(prompt.starts_with("THE COMPOSED PROMPT\n"), "{prompt}");
    assert!(prompt.contains("## Your team"), "{prompt}");
    assert!(prompt.contains(team.id().as_str()), "the team: {prompt}");
    assert!(prompt.contains("inv-opus"), "the coordinator: {prompt}");
    assert!(prompt.contains("claude-1"), "the members: {prompt}");
    assert!(prompt.contains("glm-1"), "the workers: {prompt}");
    assert!(
        prompt.contains("`agents/claude-1/task-20261111-153002-k3x9-1.1`"),
        "the branch: {prompt}"
    );
    assert!(prompt.contains("`main`"), "the base: {prompt}");
}

#[tokio::test]
async fn a_team_without_workers_says_so() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, None, true);

    assert!(
        prompt.contains("Workers, cheap models for small jobs: none."),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_resumed_session_is_not_told_the_team_again() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, None, false);

    assert_eq!(prompt, "THE COMPOSED PROMPT\n");
}

#[tokio::test]
async fn a_member_that_is_woken_for_the_first_time_gets_its_prompt_the_team_and_the_wake() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, Some(&Why::Woken), true);

    assert!(prompt.starts_with("THE COMPOSED PROMPT\n"), "{prompt}");
    let team_at = prompt.find("## Your team").expect("the team");
    let wake_at = prompt.find("YOU HAVE BEEN WOKEN UP").expect("the wake");
    assert!(team_at < wake_at, "{prompt}");
}

#[tokio::test]
async fn a_resumed_member_that_is_woken_gets_only_the_wake() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, Some(&Why::Woken), false);

    assert!(prompt.starts_with("# YOU HAVE BEEN WOKEN UP (team "), "{prompt}");
    assert!(prompt.contains(team.id().as_str()), "{prompt}");
    assert!(!prompt.contains("THE COMPOSED PROMPT"), "{prompt}");
}

#[tokio::test]
async fn a_resumed_member_that_is_given_an_item_gets_the_item_its_branch_and_its_checks() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, Some(&Why::Assigned("1.2".parse().unwrap())), false);

    assert!(prompt.starts_with("# A NEW ITEM FOR YOU: 1.2"), "{prompt}");
    assert!(
        prompt.contains("`agents/claude-1/task-20261111-153002-k3x9-1.1`"),
        "the branch: {prompt}"
    );
    assert!(prompt.contains("## Your item: 1.2 — Two"), "the item: {prompt}");
    assert!(prompt.contains("Do two."), "its description: {prompt}");
    assert!(!prompt.contains("THE COMPOSED PROMPT"), "{prompt}");
}

#[tokio::test]
async fn a_new_session_given_an_item_keeps_its_prompt_which_has_the_item_and_adds_the_team() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);

    let prompt = shaped(&texts, &team, 2, Some(&Why::Assigned("1.1".parse().unwrap())), true);

    assert!(prompt.starts_with("THE COMPOSED PROMPT\n"), "{prompt}");
    assert!(prompt.contains("## Your team"), "{prompt}");
    assert!(
        !prompt.contains("A NEW ITEM FOR YOU"),
        "the item is in the composed prompt: {prompt}"
    );
}

#[tokio::test]
async fn an_item_that_is_not_in_the_list_is_an_error_not_an_empty_prompt() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 0);
    let (role, member) = team.find(&task_id(2)).unwrap();
    let work = branch("agents/claude-1/task");
    let base = branch("main");

    let result = shape(
        &texts.team,
        &PromptTexts {
            harness: &texts.harness,
            rules: &texts.rules,
        },
        &TeamRun {
            team: &team,
            role,
            member,
            branch: &work,
            base: &base,
            why: Some(&Why::Assigned("9.9".parse().unwrap())),
            new_session: false,
        },
        String::new(),
    );

    assert!(matches!(result, Err(TeamPromptError::NoSuchItem { .. })), "{result:?}");
}

#[tokio::test]
async fn a_worker_asked_to_serve_a_request_gets_the_request_the_context_and_the_requesters_worktree() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 1);
    let why = Why::Delegated(Box::new(request()));

    let prompt = shaped(&texts, &team, 3, Some(&why), false);

    assert!(prompt.starts_with("# A REQUEST FROM claude-1 (H7)"), "{prompt}");
    assert!(prompt.contains("List the files under src/."), "{prompt}");
    assert!(prompt.contains("The module is small."), "{prompt}");
    assert!(prompt.contains("`/srv/agents/worktrees/example/task-2`"), "{prompt}");
    assert!(
        prompt.contains("`agents/claude-1/task-20261111-153002-k3x9-1.1`"),
        "the requester's branch: {prompt}"
    );
    assert!(!prompt.contains("THE COMPOSED PROMPT"), "{prompt}");
}

#[tokio::test]
async fn a_worker_that_starts_a_session_for_a_request_gets_its_prompt_the_team_and_the_request() {
    let texts = texts().await;
    let team = crew_of(CHAIN, 1, 1);
    let why = Why::Delegated(Box::new(request()));

    let prompt = shaped(&texts, &team, 3, Some(&why), true);

    let team_at = prompt.find("## Your team").expect("the team");
    let request_at = prompt.find("A REQUEST FROM claude-1").expect("the request");
    assert!(prompt.starts_with("THE COMPOSED PROMPT\n"), "{prompt}");
    assert!(team_at < request_at, "{prompt}");
}
