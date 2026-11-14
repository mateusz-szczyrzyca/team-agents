//! The keywords the database keeps for the kinds of a record — a step, a change, the end of a run, a client, the scope
//! of a changed file — and the text of the fields that were cut. Each keyword is declared once, on its enum, and reads back
//! to the value it came from.

#[cfg(test)]
mod history_keywords {
    use std::str::FromStr;

    use agent_core::cli::ActivityEvent;
    use agent_core::config::TailscaleLogin;
    use agent_core::decide::checks::Change;
    use agent_core::history::{
        ChangeKind, ChangeScope, ClientKind, Cut, EndKind, MaxTextBytes, StepKind, UnknownCutField,
    };
    use agent_core::protocol::{Client, ControlAction};
    use agent_core::repo_path::RepoPath;
    use agent_core::state::RunEnd;
    use agent_core::status::RunOutcome;
    use strum::VariantNames;

    #[test]
    fn a_step_is_of_the_kind_of_its_event_and_the_kinds_read_back() {
        let control = ActivityEvent::Control {
            action: ControlAction::Stop,
            by: Client::Cli,
        };
        for (event, kind, text) in [
            (ActivityEvent::Text("hello".to_owned()), StepKind::Message, "message"),
            (ActivityEvent::Command("ls".to_owned()), StepKind::Command, "command"),
            (
                ActivityEvent::FileEdit { path: "a".to_owned() },
                StepKind::FileEdit,
                "file_edit",
            ),
            (
                ActivityEvent::BusCall {
                    tool: "t".to_owned(),
                    arguments: String::new(),
                },
                StepKind::BusCall,
                "bus_call",
            ),
            (
                ActivityEvent::ToolCall {
                    name: "t".to_owned(),
                    summary: String::new(),
                },
                StepKind::ToolCall,
                "tool_call",
            ),
            (ActivityEvent::Error("e".to_owned()), StepKind::Error, "error"),
            (ActivityEvent::Info("i".to_owned()), StepKind::Info, "info"),
            (control, StepKind::Control, "control"),
        ] {
            assert_eq!(StepKind::of(&event), kind, "{event:?}");
            assert_eq!(kind.to_string(), text);
            assert_eq!(StepKind::from_str(text).unwrap(), kind);
        }
        assert_eq!(StepKind::VARIANTS.len(), 8);
    }

    #[test]
    fn a_change_is_of_the_kind_the_git_status_letter_says_and_the_kinds_read_back() {
        let from = RepoPath::from_str("old.rs").unwrap();
        for (change, kind, text) in [
            (Change::Added, ChangeKind::Added, "added"),
            (Change::Deleted, ChangeKind::Deleted, "deleted"),
            (Change::Modified, ChangeKind::Modified, "modified"),
            (Change::Renamed { from }, ChangeKind::Renamed, "renamed"),
        ] {
            assert_eq!(ChangeKind::of(&change), kind, "{change:?}");
            assert_eq!(kind.to_string(), text);
            assert_eq!(ChangeKind::from_str(text).unwrap(), kind);
        }
    }

    #[test]
    fn the_end_of_a_run_is_of_a_kind_and_the_kinds_read_back() {
        for (end, kind, text) in [
            (
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
                EndKind::Reported,
                "reported",
            ),
            (RunEnd::NoStatus { exit_code: Some(1) }, EndKind::NoStatus, "no_status"),
            (
                RunEnd::QuotaExhausted { resets_at: None },
                EndKind::QuotaExhausted,
                "quota_exhausted",
            ),
            (RunEnd::RateLimited, EndKind::RateLimited, "rate_limited"),
            (RunEnd::Hung, EndKind::Hung, "hung"),
            (RunEnd::Stopped, EndKind::Stopped, "stopped"),
        ] {
            assert_eq!(EndKind::of(&end), kind, "{end:?}");
            assert_eq!(kind.to_string(), text);
            assert_eq!(EndKind::from_str(text).unwrap(), kind);
        }
    }

    #[test]
    fn a_client_is_the_command_line_or_the_dashboard_and_a_scope_is_committed_or_not() {
        let dashboard = Client::Dashboard {
            login: TailscaleLogin::from_str("you@example.com").unwrap(),
        };

        assert_eq!(ClientKind::of(&Client::Cli).to_string(), "cli");
        assert_eq!(ClientKind::of(&dashboard).to_string(), "dashboard");
        assert_eq!(ClientKind::from_str("dashboard").unwrap(), ClientKind::Dashboard);
        assert_eq!(ChangeScope::Committed.to_string(), "committed");
        assert_eq!(ChangeScope::Uncommitted.to_string(), "uncommitted");
        assert_eq!(ChangeScope::from_str("uncommitted").unwrap(), ChangeScope::Uncommitted);
    }

    /// A limit that cuts everything but one byte.
    fn tiny() -> MaxTextBytes {
        MaxTextBytes::new(1).unwrap()
    }

    #[test]
    fn the_fields_that_were_cut_are_written_sorted_and_read_back_from_the_known_names() {
        let mut cut = Cut::default();
        cut.keep("prompt", "ab", tiny());
        cut.keep("final_message", "ab", tiny());
        cut.keep("title", "a", tiny());

        let text = cut.to_string();
        let back = Cut::parse(&text, &["final_message", "prompt", "title"]).unwrap();

        assert_eq!(text, "final_message,prompt");
        assert_eq!(back, cut);
        assert_eq!(Cut::default().to_string(), "");
        assert_eq!(Cut::parse("", &["prompt"]).unwrap(), Cut::default());
    }

    #[test]
    fn a_field_nobody_cuts_is_refused_when_the_text_of_the_cut_is_read() {
        let error = Cut::parse("prompt,nonsense", &["prompt"]).unwrap_err();

        assert_eq!(
            error,
            UnknownCutField {
                field: "nonsense".to_owned()
            }
        );
    }
}
