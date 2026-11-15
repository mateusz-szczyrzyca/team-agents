//! `agent log` as a process: it reads the history file of `AGENTS_HOME` and nothing else — no daemon runs, no socket or
//! state is made — and an ID the history does not know ends it with status 3, a command line it refuses with status 2.

#[cfg(test)]
#[path = "support/story_history.rs"]
mod story_history;

#[cfg(test)]
mod log_process {
    use std::path::Path;
    use std::process::{Command, Output};

    use super::story_history::{Written, write_history, write_old_history};

    /// Runs `agent log` with `args` in the home `home`, with nothing else in its environment.
    fn log(home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .arg("log")
            .args(args)
            .env_clear()
            .env("AGENTS_HOME", home)
            .output()
            .unwrap()
    }

    /// A home with a history, and what is in it.
    fn with_history() -> (tempfile::TempDir, std::path::PathBuf, Written) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let written = write_history(&home.join("history.db"));
        (dir, home, written)
    }

    /// The names of the files in `home`.
    fn files(home: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(home)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_story_of_a_round_is_printed_with_no_daemon_and_nothing_else_is_made_in_the_home() {
        let (_dir, home, written) = with_history();
        let before = files(&home);

        let output = log(&home, &["--round", written.rounds[0].as_str()]);

        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.starts_with(&format!("Round {} of team {}\n", written.rounds[0], written.team)),
            "{text}"
        );
        assert!(text.contains("pull request #7 merged as 4444444\n"), "{text}");
        assert_eq!(output.stderr, b"");
        let after = files(&home);
        assert!(after.iter().all(|name| name.starts_with("history.db")), "{after:?}");
        assert!(after.len() >= before.len());
        assert!(
            !after.iter().any(|name| name.contains("sock") || name == "state.json"),
            "{after:?}"
        );
    }

    #[test]
    fn an_unknown_round_ends_with_status_3_a_bad_command_line_with_status_2_and_a_closed_history_with_status_1() {
        let (_dir, home, _) = with_history();

        let unknown = log(&home, &["--round", "round-20990101-000000-zzzz"]);
        let bad = log(&home, &["--round", "nonsense"]);
        let none = log(&home, &[]);
        let empty = tempfile::tempdir().unwrap();
        let missing = log(
            &empty.path().canonicalize().unwrap(),
            &["--team", "team-20261111-193001-t5m8"],
        );

        assert_eq!(unknown.status.code(), Some(3));
        assert!(
            String::from_utf8(unknown.stderr)
                .unwrap()
                .contains("round-20990101-000000-zzzz")
        );
        assert_eq!(bad.status.code(), Some(2));
        assert_eq!(none.status.code(), Some(2));
        assert_eq!(missing.status.code(), Some(1));
        assert_eq!(unknown.stdout, b"");
    }

    #[test]
    fn a_history_of_item_8_4_is_told_by_the_binary_and_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let written = write_old_history(&home.join("history.db"));
        let before = std::fs::read(home.join("history.db")).unwrap();

        let output = log(&home, &["--team", written.team.as_str()]);

        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("no pull request recorded for this round\n"), "{text}");
        assert_eq!(std::fs::read(home.join("history.db")).unwrap(), before);
    }
}
