//! The places of the daemon's home that item 6.1 adds: the socket, the settings files, the prompts, the worktrees and
//! the questions files. Each is derived from the root and validated IDs, so no input can name a place outside it.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::TaskId;
use agent_runtime::store::Home;

fn home() -> Home {
    Home::new(&AbsolutePath::from_str("/srv/agents-home").unwrap())
}

fn task() -> TaskId {
    "task-20261111-153601-k3x9".parse().unwrap()
}

#[test]
fn the_socket_and_the_settings_files_are_in_the_root() {
    let home = home();

    assert_eq!(home.socket_file(), PathBuf::from("/srv/agents-home/agent.sock"));
    assert_eq!(home.config_file(), PathBuf::from("/srv/agents-home/agents.toml"));
    assert_eq!(home.secrets_file(), PathBuf::from("/srv/agents-home/secrets.toml"));
    assert_eq!(home.prompts_dir(), PathBuf::from("/srv/agents-home/prompts"));
}

#[test]
fn a_worktree_is_one_directory_of_its_task_under_its_repository() {
    let repo = RepoName::from_str("team-agents").unwrap();

    assert_eq!(
        home().worktree_dir(&repo, &task()),
        PathBuf::from("/srv/agents-home/worktrees/team-agents/task-20261111-153601-k3x9")
    );
}

#[test]
fn a_questions_file_is_numbered_by_its_round_in_the_directory_of_its_task() {
    let round = NonZeroU32::new(3).unwrap();

    assert_eq!(
        home().questions_file(&task(), round),
        PathBuf::from("/srv/agents-home/tasks/task-20261111-153601-k3x9/questions-3.md")
    );
}

#[test]
fn the_questions_file_of_a_round_lies_in_the_task_directory() {
    let home = home();
    let file = home.questions_file(&task(), NonZeroU32::new(1).unwrap());

    assert_eq!(file.parent(), Some(home.task_dir(&task()).as_path()));
}
