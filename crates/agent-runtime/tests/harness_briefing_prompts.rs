//! The briefing of a task started in a new session, against its golden texts in `fixtures/briefing/`: with and without
//! a plan, with and without uncommitted files and commits, and with a non-ASCII text.

#[cfg(test)]
mod harness_briefing_prompts {
    use std::path::Path;

    use agent_core::config::Language;
    use agent_core::prompt::{HarnessPrompt, PromptId};
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::run::compose::{Briefing, BriefingCommit, BriefingMessage, BriefingStep, briefing};

    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/briefing")
                .join(name),
        )
        .unwrap()
    }

    /// A prompts directory with a stand-in for every text of both catalogs; the briefing's stand-in shows its path and
    /// each of its variables as `name={{name}}`.
    fn prompts() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let specs = HarnessPrompt::ALL
            .iter()
            .map(|id| id.spec())
            .chain(PromptId::ALL.iter().map(|id| id.spec()));
        for spec in specs {
            let path = dir.path().join(spec.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let shown: Vec<String> = spec
                .variables()
                .iter()
                .map(|name| format!("{name}={{{{{name}}}}}"))
                .collect();
            std::fs::write(&path, format!("[{}] {}\n", spec.path(), shown.join(" "))).unwrap();
        }
        dir
    }

    /// The briefing of `facts`, rendered with the stand-in text and a section for each fact it is made of.
    async fn render(facts: &Briefing<'_>) -> String {
        let dir = prompts();
        let texts = HarnessPrompts::load(dir.path(), &Language::default()).await.unwrap();
        briefing(
            &|id, values| {
                texts
                    .render(HarnessPrompt::Briefing, values)
                    .unwrap_or_else(|_| id.to_owned())
            },
            facts,
        )
    }

    /// The reason a task of these tests is started anew.
    fn reason() -> String {
        "its last three runs ended without a `STATUS:` line".to_owned()
    }

    #[tokio::test]
    async fn a_new_session_is_briefed_with_the_commits_the_files_and_the_plan() {
        let facts = Briefing {
            reason: &reason(),
            task: "task-20261120-144101-r7q2",
            prompt: "Fix the parser of \u{17c}ół\u{107}.\nIt drops the last line.",
            base: "main",
            branch: "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1",
            commits: &[
                BriefingCommit {
                    id: "97c3d79e",
                    subject: "Add the ladder decision",
                },
                BriefingCommit {
                    id: "9d630293",
                    subject: "Decide the ladder of a task",
                },
            ],
            uncommitted: &["crates/agent-core/src/decide/ladder.rs", "notes.md"],
            plan: Some(&[
                BriefingStep {
                    id: "1.1",
                    title: "Read the code",
                    state: "done",
                },
                BriefingStep {
                    id: "1.2",
                    title: "Write the test",
                    state: "in hand",
                },
            ]),
            messages: &[],
            snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
        };

        let text = render(&facts).await;

        assert_eq!(text, golden("briefing-full.txt"));
    }

    #[tokio::test]
    async fn a_new_session_of_a_task_without_plan_commits_or_files_is_briefed_from_fewer_facts() {
        let facts = Briefing {
            reason: &reason(),
            task: "task-20261120-144101-r7q2",
            prompt: "Fix the parser.\n",
            base: "main",
            branch: "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1",
            commits: &[],
            uncommitted: &[],
            plan: None,
            messages: &[],
            snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
        };

        let text = render(&facts).await;

        assert_eq!(text, golden("briefing-sparse.txt"));
    }

    #[tokio::test]
    async fn a_new_session_is_told_the_messages_of_its_team_it_did_not_read() {
        let facts = Briefing {
            reason: &reason(),
            task: "task-20261120-144101-r7q2",
            prompt: "Fix the parser.\n",
            base: "main",
            branch: "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1",
            commits: &[],
            uncommitted: &["notes.md"],
            plan: None,
            messages: &[BriefingMessage {
                id: "1508",
                from: "inv-opus",
                text: "Work on item 1.1 only.",
            }],
            snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
        };

        let text = render(&facts).await;

        assert_eq!(text, golden("briefing-messages.txt"));
    }
}
