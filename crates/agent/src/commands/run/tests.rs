use std::process::Command as Process;
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::protocol::{Client, Command, Failure, FailureKind, Frame, Reply, Response, Target};
use agent_core::state::{ItemPhase, Team};
use agent_core::task_list::TaskList;

use super::*;
use crate::fixtures::{task, team_id, team_state};
use crate::testing::{Answer, FakeDaemon};

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|word| (*word).to_owned()).collect()
}

fn agent_names(list: &AgentList) -> Vec<&str> {
    list.as_slice().iter().map(AgentName::as_str).collect()
}

// ---- the words of the command line -------------------------------------------------------------------------------

#[test]
fn with_a_file_every_word_is_an_agent() {
    let (agents, text) = split_words(&words(&["claude-1", "codex-1"]), true).unwrap();

    assert_eq!(agent_names(&agents), ["claude-1", "codex-1"]);
    assert_eq!(text, None);
}

#[test]
fn without_a_file_the_last_word_is_the_task_and_the_others_are_the_agents() {
    let (agents, text) = split_words(&words(&["claude-1", "codex-1", "Fix the typo.\nThen stop."]), false).unwrap();

    assert_eq!(agent_names(&agents), ["claude-1", "codex-1"]);
    assert_eq!(text.as_deref(), Some("Fix the typo.\nThen stop."));
}

#[test]
fn one_agent_and_a_task_are_a_solo_task() {
    let (agents, text) = split_words(&words(&["claude-1", "Żółć: fix it"]), false).unwrap();

    assert_eq!(agent_names(&agents), ["claude-1"]);
    assert_eq!(text.as_deref(), Some("Żółć: fix it"));
}

#[test]
fn no_words_name_no_agent_and_a_single_word_is_no_task() {
    assert!(matches!(split_words(&[], true), Err(WordsError::NoAgents)));
    assert!(matches!(split_words(&[], false), Err(WordsError::NoAgents)));
    assert!(matches!(
        split_words(&words(&["claude-1"]), false),
        Err(WordsError::NoTask)
    ));
}

#[test]
fn a_word_that_is_not_an_agent_name_is_named() {
    let error = split_words(&words(&["Claude One", "codex-1"]), true).unwrap_err();

    assert!(
        matches!(&error, WordsError::Agent { word, .. } if word == "Claude One"),
        "{error:?}"
    );
}

#[test]
fn an_agent_named_twice_is_refused() {
    let error = split_words(&words(&["claude-1", "claude-1", "Do it."]), false).unwrap_err();

    assert!(
        matches!(error, WordsError::Agents(AgentListError::Duplicate { .. })),
        "{error:?}"
    );
}

// ---- the name of the repository ----------------------------------------------------------------------------------

#[test]
fn the_name_of_a_repository_is_the_last_part_of_the_url_of_its_origin() {
    for (url, name) in [
        ("https://github.com/mateusz/team-agents.git", "team-agents"),
        ("https://github.com/mateusz/team-agents", "team-agents"),
        ("https://github.com/mateusz/team-agents/", "team-agents"),
        ("git@github.com:mateusz/team-agents.git", "team-agents"),
        ("ssh://git@example.com:2222/org/sub/tool.name.git", "tool.name"),
        ("/srv/git/local_repo.git", "local_repo"),
        ("file:///srv/git/local_repo", "local_repo"),
    ] {
        assert_eq!(repo_name_of(url).unwrap().as_str(), name, "{url}");
    }
}

#[test]
fn a_url_that_names_no_repository_is_refused_with_the_url() {
    for url in [
        "",
        "https://example.com/",
        "git@example.com:",
        ".git",
        "https://example.com/org/we ird.git",
    ] {
        let error = repo_name_of(url).unwrap_err();

        assert!(
            matches!(&error, RunError::RepoName { url: shown, .. } if shown == url),
            "{url}: {error:?}"
        );
    }
}

// ---- what is printed ---------------------------------------------------------------------------------------------

fn home() -> Home {
    Home::new(&AbsolutePath::from_str("/srv/agents").unwrap())
}

fn sample_team() -> Team {
    team_state().teams()[0].clone()
}

#[test]
fn what_started_is_the_team_and_a_line_for_each_task_with_its_worktree() {
    let text = render_started(&home(), &sample_team());

    assert_eq!(
        text,
        "\
team team-20261111-153000-t5m8 started on team-agents, merge: autopilot
  task-20261111-153001-k3x9  coordinator  inv-opus  /srv/agents/worktrees/team-agents/task-20261111-153001-k3x9
  task-20261111-153002-k3x9  member       claude-1  /srv/agents/worktrees/team-agents/task-20261111-153002-k3x9
  task-20261111-153003-k3x9  worker       glm-1     /srv/agents/worktrees/team-agents/task-20261111-153003-k3x9
"
    );
}

#[test]
fn a_member_that_has_an_item_says_which() {
    let mut state = team_state();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let mut item = team.item_mut("1.1".parse().unwrap()).unwrap();
        item.advance(ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/task-20261111-153002-k3x9-1.1".parse().unwrap(),
        })
        .unwrap();
    }

    let text = render_started(&home(), &state.teams()[0]);

    assert!(
        text.contains("/srv/agents/worktrees/team-agents/task-20261111-153002-k3x9  item 1.1\n"),
        "{text}"
    );
    assert!(
        text.contains("task-20261111-153001-k3x9  coordinator  inv-opus  /srv"),
        "{text}"
    );
}

#[test]
fn a_team_that_merges_by_hand_says_so() {
    let mut team = sample_team();
    team = rebuild_with(&team, MergeMode::NoMerge);

    let text = render_started(&home(), &team);

    assert!(
        text.starts_with("team team-20261111-153000-t5m8 started on team-agents, merge: no_merge\n"),
        "{text}"
    );
}

/// The same team that merges as `merge` says.
fn rebuild_with(team: &Team, merge: MergeMode) -> Team {
    let crew = team.crew().clone();
    Team::new(
        team.id().clone(),
        team.repo().clone(),
        team.created_at(),
        merge,
        team.task_list().clone(),
        crew,
    )
}

// ---- the command -------------------------------------------------------------------------------------------------

/// A git checkout in a temporary directory, with `origin` at `origin`.
fn checkout(origin: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap().join("work");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    for args in [vec!["init", "-q", "-b", "main"]]
        .into_iter()
        .chain(origin.map(|url| vec!["remote", "add", "origin", url]))
    {
        let status = Process::new("git")
            .args(&args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    (dir, repo)
}

/// A daemon that starts the sample team for a `run` and shows it for a `show`.
fn daemon_that_starts(dir: &tempfile::TempDir) -> FakeDaemon {
    FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        let reply = match &request.command {
            Command::Run(_) => Reply::Started {
                team: team_id(0),
                tasks: vec![task(1), task(2), task(3)],
            },
            Command::Show(show) => {
                assert_eq!(show.target, Target::Team(team_id(0)));
                Reply::Team {
                    team: Box::new(sample_team()),
                }
            }
            other => panic!("{other:?}"),
        };
        Answer::Lines(vec![
            Frame::Response(Response::ok(request.id, reply)).to_line().unwrap(),
        ])
    })
}

fn input(words: &[&str], file: Option<PathBuf>) -> RunInput {
    RunInput {
        words: self::words(words),
        file,
        coordinator: None,
        merge: MergeMode::Autopilot,
    }
}

fn run_request(daemon: &FakeDaemon) -> agent_core::protocol::RunRequest {
    let Command::Run(request) = daemon
        .requests()
        .into_iter()
        .find_map(|request| match request.command {
            command @ Command::Run(_) => Some(command),
            _ => None,
        })
        .unwrap()
    else {
        unreachable!()
    };
    request
}

#[tokio::test]
async fn a_task_given_as_the_last_word_is_run_in_the_repository_the_command_is_in_and_what_started_is_printed() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout(Some("https://example.com/org/team-agents.git"));
    let daemon = daemon_that_starts(&dir);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    run(
        &mut connection,
        &home(),
        &repo.join("sub"),
        input(&["claude-1", "codex-1", "Fix the typo."], None),
        &mut out,
    )
    .await
    .unwrap();

    let request = run_request(&daemon);
    assert_eq!(request.repo.as_str(), "team-agents");
    assert_eq!(request.checkout.as_path(), repo);
    assert_eq!(agent_names(&request.agents), ["claude-1", "codex-1"]);
    assert_eq!(request.task_list.text(), "Fix the typo.");
    assert_eq!(request.coordinator, None);
    assert_eq!(request.merge, MergeMode::Autopilot);
    assert_eq!(String::from_utf8(out).unwrap(), render_started(&home(), &sample_team()));
}

#[tokio::test]
async fn a_task_list_file_the_flags_and_the_merge_mode_are_sent_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout(Some("git@example.com:org/example-repo.git"));
    let list = dir.path().join("list.md");
    std::fs::write(
        &list,
        "Coordinator: codex-1\n\nBuild.\n\n1. First\n[ ] 1.1 One\n    Do one.\n",
    )
    .unwrap();
    let daemon = daemon_that_starts(&dir);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut asked = input(&["claude-1", "codex-1"], Some(list.clone()));
    asked.coordinator = Some("claude-1".parse().unwrap());
    asked.merge = MergeMode::NoMerge;

    run(&mut connection, &home(), &repo, asked, &mut Vec::new())
        .await
        .unwrap();

    let request = run_request(&daemon);
    assert_eq!(request.repo.as_str(), "example-repo");
    assert_eq!(agent_names(&request.agents), ["claude-1", "codex-1"]);
    assert_eq!(request.task_list.text(), std::fs::read_to_string(&list).unwrap());
    assert_eq!(request.task_list.list().items().count(), 1);
    assert_eq!(request.coordinator, Some("claude-1".parse().unwrap()));
    assert_eq!(request.merge, MergeMode::NoMerge);
}

/// Runs the command in a checkout with a daemon, and returns its error and the requests the daemon got.
async fn refused_before_asking(
    origin: Option<&str>,
    input: RunInput,
    in_repository: bool,
) -> (RunError, Vec<agent_core::protocol::Request>) {
    let dir = tempfile::tempdir().unwrap();
    let (work, repo) = checkout(origin);
    let daemon = daemon_that_starts(&dir);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let directory = if in_repository { repo } else { work.path().to_path_buf() };

    let error = run(&mut connection, &home(), &directory, input, &mut Vec::new())
        .await
        .unwrap_err();

    (error, daemon.requests())
}

#[tokio::test]
async fn a_task_list_file_that_is_not_there_is_named_and_the_daemon_is_not_asked() {
    let (error, requests) = refused_before_asking(
        Some("https://example.com/org/team-agents.git"),
        input(&["claude-1"], Some(PathBuf::from("/nonexistent/list.md"))),
        true,
    )
    .await;

    assert!(
        matches!(&error, RunError::NoListFile { path } if path == Path::new("/nonexistent/list.md")),
        "{error:?}"
    );
    assert_eq!(requests, []);
}

#[tokio::test]
async fn a_task_that_is_not_a_task_list_is_refused_with_its_line_and_the_daemon_is_not_asked() {
    let (error, requests) = refused_before_asking(
        Some("https://example.com/org/team-agents.git"),
        input(&["claude-1", "Brief.\n\n[ ] 1.1 An item before its feature\n"], None),
        true,
    )
    .await;

    assert!(
        matches!(&error, RunError::List(TaskListError::ItemBeforeFeature { .. })),
        "{error:?}"
    );
    assert_eq!(requests, []);
}

#[tokio::test]
async fn a_directory_that_is_in_no_repository_is_refused_and_the_daemon_is_not_asked() {
    let (error, requests) = refused_before_asking(None, input(&["claude-1", "Do it."], None), false).await;

    assert!(matches!(&error, RunError::Checkout { .. }), "{error:?}");
    assert_eq!(requests, []);
}

#[tokio::test]
async fn a_repository_without_an_origin_is_refused_and_the_daemon_is_not_asked() {
    let (error, requests) = refused_before_asking(None, input(&["claude-1", "Do it."], None), true).await;

    assert!(matches!(&error, RunError::NoOrigin(_)), "{error:?}");
    assert_eq!(requests, []);
}

#[tokio::test]
async fn words_that_make_no_task_are_refused_before_anything_is_asked() {
    let (error, requests) = refused_before_asking(
        Some("https://example.com/org/team-agents.git"),
        input(&["claude-1"], None),
        true,
    )
    .await;

    assert!(matches!(error, RunError::Words(WordsError::NoTask)), "{error:?}");
    assert_eq!(requests, []);
}

#[tokio::test]
async fn a_refusal_of_the_daemon_is_the_error_with_what_the_daemon_said() {
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout(Some("https://example.com/org/team-agents.git"));
    let daemon = FakeDaemon::serve(&dir.path().join("agent.sock"), |request| {
        let refusal = Failure {
            kind: FailureKind::Refused,
            message: "too many investigators in the team".to_owned(),
        };
        Answer::Lines(vec![
            Frame::Response(Response::error(request.id, refusal)).to_line().unwrap(),
        ])
    });
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();
    let mut out = Vec::new();

    let error = run(
        &mut connection,
        &home(),
        &repo,
        input(&["inv-opus", "inv-codex", "Do it."], None),
        &mut out,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(&error, RunError::Client(ClientError::Refused(failure)) if failure.kind == FailureKind::Refused && failure.message == "too many investigators in the team"),
        "{error:?}"
    );
    assert_eq!(out, b"");
}

#[tokio::test]
async fn a_task_list_the_daemon_receives_parses_as_the_one_that_was_given() {
    let list = "Brief.\n\n1. First\n[ ] 1.1 One\n    Do one.\n";
    let dir = tempfile::tempdir().unwrap();
    let (_work, repo) = checkout(Some("https://example.com/org/team-agents.git"));
    let daemon = daemon_that_starts(&dir);
    let mut connection = Connection::connect(daemon.socket()).await.unwrap();

    run(
        &mut connection,
        &home(),
        &repo,
        input(&["claude-1", list], None),
        &mut Vec::new(),
    )
    .await
    .unwrap();

    let sent = run_request(&daemon).task_list;
    assert_eq!(&TaskList::parse(sent.text()).unwrap(), sent.list());
    assert_eq!(Client::Cli, daemon.requests()[0].client);
}
