//! Reported resumes retain session context and repeat only changed assignments.

use super::*;
use agent_core::config::Language;
use agent_core::task_list::ItemMark;

#[tokio::test]
async fn reported_resumes_repeat_only_changed_items_and_the_coordinators_team() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = Language::default();
    let harness = HarnessPrompts::load(&directory, &language).await.unwrap();
    let rules = PromptLoader::load(&directory, &language).await.unwrap();
    let task = "task-20261111-153601-k3x9".parse().unwrap();
    let repo = "team-agents".parse().unwrap();
    let item = Item {
        id: "2.3".parse().unwrap(),
        title: "The daemon".to_owned(),
        mark: ItemMark::Open,
        description: "Build it.".to_owned(),
        expected_result: None,
        checks: None,
    };
    let team_items = [TeamItem {
        item: &item,
        assignee: Some("claude-2"),
    }];
    for (role, item_changed) in [(Role::Solo, false), (Role::Solo, true), (Role::Coordinator, false)] {
        let input = PromptInput {
            kind: PromptKind::Resume(ResumeReason::Reported),
            agent: "claude-1",
            tier: Tier::Normal,
            role,
            task: &task,
            repo: &repo,
            worktree: Path::new("/w/task"),
            questions_file: Path::new("/h/questions-2.md"),
            number: NonZeroU32::new(2).unwrap(),
            brief: "UNIQUE BRIEF TEXT",
            item: Some(&item),
            item_changed,
            session_prompt: Some(Path::new("/h/tasks/task/session-prompt.md")),
            team_items: if role == Role::Coordinator { &team_items } else { &[] },
            checkpoint: None,
            serena: Serena::Given,
        };
        let prompt = compose(
            &PromptTexts {
                harness: &harness,
                rules: &rules,
            },
            &input,
        )
        .unwrap();
        assert!(prompt.starts_with("# YOUR SESSION GOES ON\n"), "{prompt}");
        assert!(prompt.contains("`/h/tasks/task/session-prompt.md`"), "{prompt}");
        assert_eq!(prompt.contains("## Your item: 2.3"), item_changed, "{prompt}");
        assert_eq!(
            prompt.contains("## The items of your team"),
            role == Role::Coordinator,
            "{prompt}"
        );
        for omitted in [
            "UNIQUE BRIEF TEXT",
            "# Rust developer rules",
            "# Working context",
            "# Finding code",
        ] {
            assert!(!prompt.contains(omitted), "{omitted}: {prompt}");
        }
    }
}
