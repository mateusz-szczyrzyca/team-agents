//! The prompt of a run, section by section: which sections a run gets, in which order, filled with which values. The
//! texts are stand-ins that show their own path and variables, so each assertion shows exactly what was chosen.
//!
//! `agents.toml` without `serena` gives a prompt without the section on finding code with Serena; with it, a first
//! prompt has the section right after the working context, a resumed one does not.

#[cfg(test)]
mod run_compose {
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::str::FromStr;

    use agent_core::config::{Language, Minutes, RepoName};
    use agent_core::ids::TaskId;
    use agent_core::kinds::{Role, Tier};
    use agent_core::prompt::{HarnessPrompt, PromptId};
    use agent_core::task_list::{Item, ItemMark};
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::prompts::PromptLoader;
    use agent_runtime::run::compose::{
        PromptInput, PromptKind, PromptTexts, ResumeReason, Serena, TeamItem, compose, serena_section,
    };

    /// The stand-in for a file: its path in brackets, then each of its variables as `name={{name}}`.
    fn stand_in(path: &str, variables: &[&str]) -> String {
        let shown: Vec<String> = variables.iter().map(|name| format!("{name}={{{{{name}}}}}")).collect();
        if shown.is_empty() {
            format!("[{path}]\n")
        } else {
            format!("[{path}] {}\n", shown.join(" "))
        }
    }

    /// A prompts directory with a stand-in for every text of both catalogs.
    fn prompts_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let specs = HarnessPrompt::ALL
            .iter()
            .map(|id| id.spec())
            .chain(PromptId::ALL.iter().map(|id| id.spec()));
        for spec in specs {
            let path = dir.path().join(spec.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, stand_in(spec.path(), spec.variables())).unwrap();
        }
        dir
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        harness: HarnessPrompts,
        rules: PromptLoader,
        task: TaskId,
        repo: RepoName,
    }

    async fn fixture() -> Fixture {
        let dir = prompts_dir();
        let language = Language::default();
        Fixture {
            harness: HarnessPrompts::load(dir.path(), &language).await.unwrap(),
            rules: PromptLoader::load(dir.path(), &language).await.unwrap(),
            _dir: dir,
            task: TaskId::from_str("task-20261111-153601-k3x9").unwrap(),
            repo: RepoName::from_str("team-agents").unwrap(),
        }
    }

    impl Fixture {
        fn input(&self, role: Role, tier: Tier) -> PromptInput<'_> {
            PromptInput {
                kind: PromptKind::First,
                agent: "claude-1",
                tier,
                role,
                task: &self.task,
                repo: &self.repo,
                worktree: Path::new("/w/team-agents/task-1"),
                questions_file: Path::new("/h/tasks/task-1/questions-2.md"),
                number: NonZeroU32::new(2).unwrap(),
                brief: "",
                item: None,
                team_items: &[],
                checkpoint: None,
                serena: Serena::Without,
            }
        }

        fn compose(&self, input: &PromptInput<'_>) -> String {
            compose(
                &PromptTexts {
                    harness: &self.harness,
                    rules: &self.rules,
                },
                input,
            )
            .unwrap()
        }
    }

    const CONTEXT: &str = "[harness/run/context.md] agent=claude-1 task=task-20261111-153601-k3x9 repo=team-agents worktree=/w/team-agents/task-1 questions_file=/h/tasks/task-1/questions-2.md number=2";

    /// The stand-in of the section on finding code with Serena.
    const SERENA: &str = "[harness/run/serena.md]";

    fn item(expected: Option<&str>, checks: Option<&str>) -> Item {
        Item {
            id: "2.3".parse().unwrap(),
            title: "The daemon".to_owned(),
            mark: ItemMark::Open,
            description: "Build it.".to_owned(),
            expected_result: expected.map(str::to_owned),
            checks: checks.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn a_solo_agents_first_prompt_is_its_context_its_role_and_its_rules() {
        let f = fixture().await;

        let prompt = f.compose(&f.input(Role::Solo, Tier::Normal));

        assert_eq!(
            prompt,
            format!("{CONTEXT}\n\n[harness/run/role-solo.md]\n\n[agent/rust-developer.md]\n")
        );
    }

    #[tokio::test]
    async fn the_role_and_the_tier_choose_the_sections() {
        let f = fixture().await;
        let cases = [
            (
                Role::Coordinator,
                Tier::Investigator,
                "role-coordinator",
                "investigator/rust-developer",
            ),
            (Role::Member, Tier::Normal, "role-member", "agent/rust-developer"),
            (
                Role::Member,
                Tier::Investigator,
                "role-member",
                "investigator/rust-developer",
            ),
            (Role::Worker, Tier::Worker, "role-worker", "worker-agent/rust-developer"),
            (
                Role::Reviewer,
                Tier::Investigator,
                "role-reviewer",
                "investigator/code-review",
            ),
            (
                Role::Coordinator,
                Tier::Normal,
                "role-coordinator",
                "agent/rust-developer",
            ),
        ];

        for (role, tier, role_file, rules_file) in cases {
            let prompt = f.compose(&f.input(role, tier));

            assert_eq!(
                prompt,
                format!("{CONTEXT}\n\n[harness/run/{role_file}.md]\n\n[{rules_file}.md]\n"),
                "{role:?} {tier:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_brief_comes_after_the_rules_and_only_when_there_is_one() {
        let f = fixture().await;
        let mut input = f.input(Role::Solo, Tier::Normal);
        input.brief = "Build the thing.";

        let with = f.compose(&input);
        input.brief = " \n";
        let blank = f.compose(&input);

        assert!(
            with.ends_with("[agent/rust-developer.md]\n\n[harness/run/brief.md] brief=Build the thing.\n"),
            "{with}"
        );
        assert!(!blank.contains("brief.md"), "{blank}");
    }

    // Rule: a coordinator has the items of its team in full, each with who has it; nobody else is given them.
    #[tokio::test]
    async fn a_coordinator_is_given_every_item_of_its_team_in_full_with_who_has_it() {
        let f = fixture().await;
        let first = item(Some("it works"), Some("run it"));
        let second = Item {
            id: "2.4".parse().unwrap(),
            title: "The socket".to_owned(),
            ..item(None, None)
        };
        let items = [
            TeamItem {
                item: &first,
                assignee: Some("gemini-1"),
            },
            TeamItem {
                item: &second,
                assignee: None,
            },
        ];
        let mut input = f.input(Role::Coordinator, Tier::Normal);
        input.team_items = &items;

        let prompt = f.compose(&input);

        assert!(
            prompt.ends_with(
                "[harness/run/team-items.md]\n\n\
                 [harness/run/team-item.md] item_id=2.3 item_title=The daemon item_description=Build it. \
                 who=gemini-1 works on it\n\n\
                 [harness/run/item-expected.md] expected=it works\n\n\
                 [harness/run/item-checks.md] checks=run it\n\n\
                 [harness/run/team-item.md] item_id=2.4 item_title=The socket item_description=Build it. \
                 who=nobody has it yet\n"
            ),
            "{prompt}"
        );
        // Without items of a team the prompt has no such part.
        let plain = f.compose(&f.input(Role::Coordinator, Tier::Normal));
        assert!(!plain.contains("team-item"), "{plain}");
    }

    #[tokio::test]
    async fn an_item_is_followed_by_its_expected_result_and_its_checks_when_it_has_them() {
        let f = fixture().await;
        let full = item(Some("it works"), Some("run it"));
        let bare = item(None, None);
        let mut input = f.input(Role::Member, Tier::Normal);
        input.item = Some(&full);

        let with = f.compose(&input);
        input.item = Some(&bare);
        let without = f.compose(&input);

        assert!(
            with.ends_with(
                "[harness/run/item.md] item_id=2.3 item_title=The daemon item_description=Build it.\n\n\
                 [harness/run/item-expected.md] expected=it works\n\n\
                 [harness/run/item-checks.md] checks=run it\n"
            ),
            "{with}"
        );
        assert!(
            without.ends_with("[harness/run/item.md] item_id=2.3 item_title=The daemon item_description=Build it.\n"),
            "{without}"
        );
    }

    #[tokio::test]
    async fn the_checkpoint_is_the_last_section() {
        let f = fixture().await;
        let it = item(None, None);
        let mut input = f.input(Role::Member, Tier::Normal);
        input.brief = "B";
        input.item = Some(&it);
        input.checkpoint = Some("done: A; next: B");

        let prompt = f.compose(&input);

        assert!(
            prompt.ends_with("[harness/run/checkpoint.md] checkpoint=done: A; next: B\n"),
            "{prompt}"
        );
        let order: Vec<_> = [
            "context",
            "role-member",
            "rust-developer",
            "brief",
            "item.md",
            "checkpoint",
        ]
        .iter()
        .map(|part| prompt.find(part).unwrap())
        .collect();
        assert!(order.is_sorted(), "{order:?}");
    }

    #[tokio::test]
    async fn a_value_that_looks_like_a_placeholder_is_not_expanded() {
        let f = fixture().await;
        let mut input = f.input(Role::Solo, Tier::Normal);
        input.brief = "{{task}} and \\{{agent}}";

        let prompt = f.compose(&input);

        assert!(prompt.contains("brief={{task}} and \\{{agent}}\n"), "{prompt}");
    }

    #[tokio::test]
    async fn a_resumed_run_is_told_what_happened_and_nothing_else() {
        let f = fixture().await;
        let after: Minutes = serde_json::from_str("30").unwrap();
        let cases = [
            (
                ResumeReason::Hang { after },
                "[harness/run/resume-hang.md] minutes=30\n",
            ),
            (ResumeReason::Restart, "[harness/run/resume-restart.md]\n"),
            (ResumeReason::Limit, "[harness/run/resume-limit.md]\n"),
            (ResumeReason::NoStatus, "[harness/run/resume-no-status.md]\n"),
        ];

        for (reason, expected) in cases {
            let mut input = f.input(Role::Member, Tier::Normal);
            input.kind = PromptKind::Resume(reason);
            input.brief = "ignored";

            assert_eq!(f.compose(&input), expected, "{reason:?}");
        }
    }

    #[tokio::test]
    async fn a_resumed_run_gets_the_checkpoint_after_the_news() {
        let f = fixture().await;
        let mut input = f.input(Role::Member, Tier::Normal);
        input.kind = PromptKind::Resume(ResumeReason::Restart);
        input.checkpoint = Some("next: tests");

        let prompt = f.compose(&input);

        assert_eq!(
            prompt,
            "[harness/run/resume-restart.md]\n\n[harness/run/checkpoint.md] checkpoint=next: tests\n"
        );
    }

    /// Each role, a first run or a resumed one, with Serena or without: whether its prompt shows the section on finding
    /// code with Serena.
    #[test]
    fn only_a_first_prompt_of_a_run_with_serena_shows_the_section() {
        let cases = [
            (PromptKind::First, true, true),
            (PromptKind::First, false, false),
            (PromptKind::Resume(ResumeReason::Restart), true, false),
            (PromptKind::Resume(ResumeReason::Hang { after: minutes() }), true, false),
            (PromptKind::Resume(ResumeReason::Limit), true, false),
            (PromptKind::Resume(ResumeReason::TurnLimit), true, false),
            (PromptKind::Resume(ResumeReason::NoStatus), true, false),
            (PromptKind::Resume(ResumeReason::Restart), false, false),
        ];
        let roles = [
            Role::Solo,
            Role::Coordinator,
            Role::Member,
            Role::Reviewer,
            Role::Worker,
        ];

        for role in roles {
            for (kind, with_serena, wanted) in cases {
                let found = serena_section(role, kind, Serena::of(with_serena));
                assert_eq!(
                    found.is_some(),
                    wanted,
                    "{role:?} {kind:?} serena={with_serena}: {found:?}"
                );
                assert_eq!(
                    found,
                    wanted.then_some(HarnessPrompt::Serena),
                    "{role:?} {kind:?} serena={with_serena}"
                );
            }
        }
    }

    fn minutes() -> Minutes {
        serde_json::from_str("30").unwrap()
    }

    #[tokio::test]
    async fn a_run_with_serena_has_the_section_right_after_the_working_context() {
        let f = fixture().await;
        let it = item(None, None);
        let mut input = f.input(Role::Member, Tier::Normal);
        input.serena = Serena::Given;
        input.brief = "B";
        input.item = Some(&it);

        let prompt = f.compose(&input);

        assert_eq!(
            prompt,
            format!(
                "{CONTEXT}\n\n{SERENA}\n\n[harness/run/role-member.md]\n\n[agent/rust-developer.md]\n\n\
                 [harness/run/brief.md] brief=B\n\n\
                 [harness/run/item.md] item_id=2.3 item_title=The daemon item_description=Build it.\n"
            )
        );
    }

    #[tokio::test]
    async fn a_run_without_serena_has_the_prompt_it_always_had() {
        let f = fixture().await;
        let with_context = f.compose(&f.input(Role::Reviewer, Tier::Investigator));

        assert!(!with_context.contains("serena"), "{with_context}");
        assert_eq!(
            with_context,
            format!("{CONTEXT}\n\n[harness/run/role-reviewer.md]\n\n[investigator/code-review.md]\n")
        );
    }

    #[tokio::test]
    async fn a_resumed_run_with_serena_is_not_told_again() {
        let f = fixture().await;
        let mut input = f.input(Role::Member, Tier::Normal);
        input.serena = Serena::Given;
        input.kind = PromptKind::Resume(ResumeReason::Restart);

        let prompt = f.compose(&input);

        assert_eq!(prompt, "[harness/run/resume-restart.md]\n");
    }
}
