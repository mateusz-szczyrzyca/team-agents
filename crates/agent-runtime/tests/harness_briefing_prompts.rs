//! The briefing of a task started in a new session, against its golden texts in `fixtures/briefing/`: with and without
//! a plan, with and without uncommitted files and commits, with unread messages, and with a non-ASCII text.

#[cfg(test)]
mod harness_briefing_prompts {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR};
    use agent_runtime::run::compose::{Briefing, BriefingCommit, BriefingMessage, BriefingStep, briefing_variables};

    /// The briefing of `facts`, rendered through the real text of `prompts/harness/run/briefing.md`.
    fn render(facts: &Briefing<'_>) -> String {
        let spec = HarnessPrompt::Briefing.spec();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROMPTS_DIR)
            .join(spec.path());
        let text = std::fs::read_to_string(path).unwrap();
        spec.render(&text, &briefing_variables(facts).unwrap()).unwrap()
    }

    /// The golden text `name` of the briefing.
    fn golden(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/briefing")
                .join(name),
        )
        .unwrap()
    }

    /// The reason a task of these tests is started anew.
    fn reason() -> String {
        "its last three runs ended without a `STATUS:` line".to_owned()
    }

    /// A commit of the branch of these tests.
    fn commit(id: &'static str, subject: &'static str) -> BriefingCommit<'static> {
        BriefingCommit { id, subject }
    }

    /// A step of the plan of these tests.
    fn step(id: &'static str, title: &'static str, state: &'static str) -> BriefingStep<'static> {
        BriefingStep { id, title, state }
    }

    /// The message of the team of these tests.
    fn message(id: &'static str, from: &'static str, text: &'static str) -> BriefingMessage<'static> {
        BriefingMessage { id, from, text }
    }

    #[test]
    fn a_new_session_is_briefed_with_the_commits_the_files_and_the_plan() {
        let commits = [
            commit("97c3d79e", "Add the ladder decision"),
            commit("9d630293", "Decide the ladder of a task"),
        ];
        let uncommitted = ["crates/agent-core/src/decide/ladder.rs", "notes.md"];
        let plan = [
            step("1.1", "Read the code", "done"),
            step("1.2", "Write the test", "in hand"),
        ];
        let facts = Briefing {
            reason: &reason(),
            task: "task-20261120-144101-r7q2",
            prompt: "Fix the parser of \u{17c}ó\u{142}\u{107}.\nIt drops the last line.",
            base: "main",
            branch: "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1",
            commits: &commits,
            uncommitted: &uncommitted,
            plan: Some(&plan),
            messages: &[message("1508", "inv-opus", "Work on item 1.1 only.")],
            snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
        };

        let text = render(&facts);

        assert_eq!(text, golden("briefing-full.txt"));
    }

    #[test]
    fn a_new_session_of_a_task_without_plan_commits_or_files_is_briefed_from_fewer_facts() {
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

        let text = render(&facts);

        assert_eq!(text, golden("briefing-sparse.txt"));
    }

    #[test]
    fn a_new_session_is_told_the_messages_of_its_team_it_did_not_read() {
        let uncommitted = ["notes.md"];
        let facts = Briefing {
            reason: &reason(),
            task: "task-20261120-144101-r7q2",
            prompt: "Fix the parser.\n",
            base: "main",
            branch: "agents/deepinfra-deepseek/task-20261120-191109-qesn-1.1",
            commits: &[],
            uncommitted: &uncommitted,
            plan: None,
            messages: &[message("1508", "inv-opus", "Work on item 1.1 only.")],
            snapshot: "refs/agents/snapshots/round-1/task-20261120-144101-r7q2/1",
        };

        let text = render(&facts);

        assert_eq!(text, golden("briefing-messages.txt"));
    }
}
