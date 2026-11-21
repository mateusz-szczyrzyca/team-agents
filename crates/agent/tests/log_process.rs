//! `agent log` as a process: it reads the history file of `AGENTS_HOME` and nothing else — no daemon runs, no socket or
//! state is made — and an ID the history does not know ends it with status 3, a command line it refuses with status 2.
//! Its times are those of the zone of the process (`TZ`), with the offset valid at each moment, and `--utc` prints the
//! times of the files.

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
        assert_eq!(none.status.code(), Some(0));
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

#[cfg(test)]
mod log_zone {
    use std::path::Path;
    use std::process::{Command, Output};

    use super::story_history::{Written, write_history};

    /// A POSIX zone of one hour east of UTC, with summer time of two hours from 2026-03-29 to 2026-10-25.
    const CENTRAL_EUROPE: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

    /// A POSIX zone of New York: five hours west of UTC, with summer time from March to November.
    const NEW_YORK: &str = "EST5EDT,M3.2.0,M11.1.0";

    /// Runs `agent log` with `args` in the home `home`, with `TZ` set to `zone` for the process.
    fn log(home: &Path, args: &[&str], zone: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .arg("log")
            .args(args)
            .env_clear()
            .env("AGENTS_HOME", home)
            .env("TZ", zone)
            .output()
            .unwrap()
    }

    /// Runs `agent log` and returns its text, asserting that it succeeded.
    fn text_of(home: &Path, args: &[&str], zone: &str) -> String {
        let output = log(home, args, zone);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// A home with a history, and the rounds of it.
    fn with_history() -> (tempfile::TempDir, std::path::PathBuf, Written) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let written = write_history(&home.join("history.db"));
        (dir, home, written)
    }

    #[test]
    fn the_story_of_a_round_shows_its_times_in_the_zone_of_the_process_with_the_offset_of_each_moment() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        let text = text_of(&home, &["--round", round], CENTRAL_EUROPE);

        assert!(text.contains("2026-11-11 20:30:00 +01:00"), "{text}");
        assert!(!text.contains("19:30:00Z"), "{text}");
    }

    #[test]
    fn utc_prints_the_times_of_the_files_whatever_the_zone() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        let text = text_of(&home, &["--round", round, "--utc"], CENTRAL_EUROPE);

        assert!(text.contains("2026-11-11T19:30:00Z"), "{text}");
        assert!(!text.contains("+01:00"), "{text}");
    }

    #[test]
    fn json_is_the_same_under_every_zone() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        let east = text_of(&home, &["--round", round, "--json"], CENTRAL_EUROPE);
        let utc = text_of(&home, &["--round", round, "--json"], "UTC0");

        assert_eq!(east, utc);
        assert!(east.contains("2026-11-11T19:30:00Z"), "{east}");
    }

    #[test]
    fn since_with_a_local_time_is_read_in_the_zone_of_the_process() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        // 21:00 in the zone of the process is 20:00 UTC: the same time as the one written with its zone.
        let local = log(
            &home,
            &["--round", round, "--since", "2026-11-11 21:00"],
            CENTRAL_EUROPE,
        );
        let with_zone = text_of(
            &home,
            &["--round", round, "--since", "2026-11-11T20:00:00Z"],
            CENTRAL_EUROPE,
        );

        assert_eq!(
            local.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&local.stderr)
        );
        assert_eq!(String::from_utf8(local.stdout).unwrap(), with_zone);
    }

    /// The text with every time of a person (`YYYY-MM-DD HH:MM:SS +HH:MM`) replaced by `<time>`.
    fn normalized(text: &str) -> String {
        let bytes = text.as_bytes();
        let digits = |at: usize, count: usize| {
            bytes
                .get(at..at + count)
                .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
        };
        let mut out = String::new();
        let mut at = 0;
        while at < bytes.len() {
            let shaped = digits(at, 4)
                && bytes.get(at + 4) == Some(&b'-')
                && digits(at + 5, 2)
                && bytes.get(at + 7) == Some(&b'-')
                && digits(at + 8, 2)
                && bytes.get(at + 10) == Some(&b' ')
                && digits(at + 11, 2)
                && bytes.get(at + 13) == Some(&b':')
                && digits(at + 14, 2)
                && bytes.get(at + 16) == Some(&b':')
                && digits(at + 17, 2)
                && bytes.get(at + 19) == Some(&b' ')
                && matches!(bytes.get(at + 20), Some(b'+' | b'-'))
                && digits(at + 21, 2)
                && bytes.get(at + 23) == Some(&b':')
                && digits(at + 24, 2);
            if shaped {
                out.push_str("<time>");
                at += 26;
            } else {
                let Some(next) = text.get(at..).and_then(|rest| rest.chars().next()) else {
                    break;
                };
                out.push(next);
                at += next.len_utf8();
            }
        }
        out
    }

    #[test]
    fn the_same_history_under_two_zones_differs_only_in_its_times() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        let east = text_of(&home, &["--round", round], CENTRAL_EUROPE);
        let west = text_of(&home, &["--round", round], NEW_YORK);

        assert!(east.contains(" +01:00"), "{east}");
        assert!(west.contains(" -05:00"), "{west}");
        assert_ne!(east, west);
        assert_eq!(normalized(&east), normalized(&west));
    }

    #[test]
    fn a_zone_that_cannot_be_found_is_shown_as_utc_marked() {
        let (_dir, home, written) = with_history();
        let round = written.rounds[0].as_str();

        let text = text_of(&home, &["--round", round], "Invalid/No_Such_Zone");

        assert!(text.contains("2026-11-11 19:30:00 UTC"), "{text}");
        assert!(!text.contains("+00:00"), "{text}");
    }
}
