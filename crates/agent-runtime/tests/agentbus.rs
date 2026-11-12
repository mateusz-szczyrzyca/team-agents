//! The `agentbus` wrapper against the real binary, built from `agentbus/` and run in a temporary `AGENTS_HOME`:
//! every command, and the exit statuses that become typed errors. Nothing here touches `~/agents`.

#[cfg(test)]
mod agentbus_commands {
    use std::collections::BTreeMap;
    use std::fs::{File, create_dir_all, read_to_string, write};
    use std::num::{NonZeroU32, NonZeroU64};
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::str::FromStr;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use agent_core::config::{AbsolutePath, Program, RepoName};
    use agent_core::ids::{AgentName, BranchName, TaskId, TeamId};
    use agent_runtime::agentbus::{
        AgentBus, AgentBusError, BoardItemId, BoardLevel, BoardStatus, BusCommand, BusMember, BusTier, CheckpointKey,
        Delegation, DelegationStatus, DelegationUpdate, FailureClass, FieldName, MemberState, NewBoardItem, NewMember,
        NewTeam, NoticeKind, Outgoing, Recipient, RequestId, Woken,
    };
    use agent_runtime::process::CaptureError;
    use rustix::fs::{FlockOperation, flock};
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    /// How long a wait for something another process does lasts at most.
    const PATIENCE: Duration = Duration::from_secs(20);

    const TEAM: &str = "team-20261112-134008-ab12";

    /// The `agentbus` built from `agentbus/`, once for all the tests of this file.
    fn agentbus_binary() -> &'static Path {
        static BINARY: OnceLock<PathBuf> = OnceLock::new();
        BINARY.get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test");
            let built = std::process::Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(&binary)
                .arg(".")
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "go build of {source:?} failed: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            binary
        })
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn agent(name: &str) -> AgentName {
        name.parse().unwrap()
    }

    fn team_id() -> TeamId {
        TEAM.parse().unwrap()
    }

    fn task(suffix: &str) -> TaskId {
        format!("task-20261112-134008-{suffix}").parse().unwrap()
    }

    fn branch(name: &str) -> BranchName {
        name.parse().unwrap()
    }

    fn repo() -> RepoName {
        "team-agents".parse().unwrap()
    }

    fn item(id: u32) -> BoardItemId {
        BoardItemId::new(NonZeroU32::new(id).unwrap())
    }

    /// An `AGENTS_HOME` in a temporary directory, and the bus of it.
    struct Fixture {
        /// The home; removed when the fixture is dropped.
        dir: tempfile::TempDir,
        /// The bus under test.
        bus: AgentBus,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let program = Program::from_str(agentbus_binary().to_str().unwrap()).unwrap();
        let bus = AgentBus::new(program, abs(dir.path()));
        Fixture { dir, bus }
    }

    fn member(suffix: &str, tier: BusTier, state: MemberState) -> NewMember {
        NewMember {
            task: task(suffix),
            branch: branch(&format!("agents/{suffix}/item-1")),
            worktree: abs(Path::new("/tmp/example-worktrees").join(suffix).as_path()),
            tier,
            state,
            provider: Some("anthropic".to_owned()),
        }
    }

    /// A team with a coordinator, a normal member and a worker.
    fn new_team() -> NewTeam {
        NewTeam {
            id: team_id(),
            repo: repo(),
            lead: Some(agent("claude-1")),
            members: BTreeMap::from([
                (agent("claude-1"), member("aaa1", BusTier::Normal, MemberState::Active)),
                (agent("codex-1"), member("bbb2", BusTier::Normal, MemberState::Active)),
                (agent("gemini-1"), member("ccc3", BusTier::Worker, MemberState::Active)),
            ]),
        }
    }

    /// A fixture with the team of [`new_team`] created.
    async fn with_team() -> Fixture {
        let fixture = fixture();
        fixture.bus.team_create(&new_team()).await.unwrap();
        fixture
    }

    fn board_item(title: &str, level: BoardLevel, designated_for: Option<&str>) -> NewBoardItem {
        NewBoardItem {
            title: title.to_owned(),
            details: "Do it\nand report.".to_owned(),
            level,
            designated_for: designated_for.map(agent),
        }
    }

    fn failure_class(result: Result<impl std::fmt::Debug, AgentBusError>) -> Option<FailureClass> {
        result.unwrap_err().class()
    }

    // ------------------------------------------------------------ teams

    /// The requests that make an MCP client read its messages: initialise, then call `read_messages`.
    const READ_MESSAGES: &str = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_messages","arguments":{}}}"#,
        "\n",
    );

    /// Makes `member` read the messages that wait for it, as its model would through the MCP server: what it has
    /// read no longer wakes it. The bus has no command for that, which is why this talks to `agentbus serve`.
    async fn read_messages_as(fixture: &Fixture, member: &str, suffix: &str) {
        let mut serve = Command::new(agentbus_binary())
            .arg("serve")
            .args(["--agent", member, "--team", TEAM, "--task"])
            .arg(task(suffix).as_str())
            .args(["--repo", "team-agents", "--branch", "agents/x/item-1", "--dir"])
            .arg(fixture.bus.bus_dir())
            .env("AGENTS_HOME", fixture.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = serve.stdin.take().unwrap();
        stdin.write_all(READ_MESSAGES.as_bytes()).await.unwrap();
        // The server ends when its input does.
        drop(stdin);
        let output = tokio::time::timeout(PATIENCE, serve.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        let said = String::from_utf8(output.stdout).unwrap();
        assert!(output.status.success() && said.contains(r#""isError":false"#), "{said}");
    }

    #[tokio::test]
    async fn team_create_makes_a_team_that_team_get_reads_back() {
        let fixture = fixture();

        fixture.bus.team_create(&new_team()).await.unwrap();
        let team = fixture.bus.team_get(&team_id()).await.unwrap();

        assert_eq!(team.id, team_id());
        assert_eq!(team.repo, repo());
        assert_eq!(team.lead, Some(agent("claude-1")));
        assert!(!team.done);
        assert!(team.board.is_empty());
        assert_eq!(
            team.members.get(&agent("gemini-1")),
            Some(&BusMember {
                task: task("ccc3"),
                branch: branch("agents/ccc3/item-1"),
                worktree: PathBuf::from("/tmp/example-worktrees/ccc3"),
                state: MemberState::Active,
                tier: BusTier::Worker,
                provider: Some("anthropic".to_owned()),
            })
        );
        let names: Vec<_> = team.members.keys().map(AgentName::as_str).collect();
        assert_eq!(names, ["claude-1", "codex-1", "gemini-1"]);
        assert!(fixture.bus.bus_dir().starts_with(fixture.dir.path()));
    }

    #[tokio::test]
    async fn team_create_without_a_coordinator_or_a_provider_reads_back_as_none() {
        let fixture = fixture();
        let mut team = new_team();
        team.lead = None;
        for new in team.members.values_mut() {
            new.provider = None;
        }

        fixture.bus.team_create(&team).await.unwrap();
        let read = fixture.bus.team_get(&team_id()).await.unwrap();

        assert_eq!(read.lead, None);
        assert!(read.members.values().all(|member| member.provider.is_none()));
    }

    #[tokio::test]
    async fn team_create_refuses_a_team_without_members() {
        let fixture = fixture();
        let mut team = new_team();
        team.members.clear();

        let result = fixture.bus.team_create(&team).await;

        assert_eq!(failure_class(result), Some(FailureClass::InvalidArgument));
    }

    #[tokio::test]
    async fn team_get_of_a_team_that_does_not_exist_is_not_found() {
        let fixture = fixture();

        let result = fixture.bus.team_get(&team_id()).await;

        assert_eq!(failure_class(result), Some(FailureClass::NotFound));
    }

    #[tokio::test]
    async fn team_set_changes_the_state_of_a_member() {
        let fixture = with_team().await;

        let woken = fixture
            .bus
            .team_set(&team_id(), &agent("codex-1"), MemberState::Paused)
            .await
            .unwrap();

        assert_eq!(woken, Woken::No);
        let team = fixture.bus.team_get(&team_id()).await.unwrap();
        assert_eq!(team.members.get(&agent("codex-1")).unwrap().state, MemberState::Paused);
        assert_eq!(team.members.get(&agent("claude-1")).unwrap().state, MemberState::Active);
    }

    #[tokio::test]
    async fn team_set_lets_a_member_sleep_when_nothing_waits_for_it() {
        let fixture = with_team().await;
        read_messages_as(&fixture, "codex-1", "bbb2").await;

        let woken = fixture
            .bus
            .team_set(&team_id(), &agent("codex-1"), MemberState::Sleeping)
            .await
            .unwrap();

        assert_eq!(woken, Woken::No);
        let team = fixture.bus.team_get(&team_id()).await.unwrap();
        assert_eq!(
            team.members.get(&agent("codex-1")).unwrap().state,
            MemberState::Sleeping
        );
    }

    #[tokio::test]
    async fn team_set_wakes_a_member_that_goes_to_sleep_with_a_message_waiting_for_it() {
        let fixture = with_team().await;
        read_messages_as(&fixture, "codex-1", "bbb2").await;
        let to = Recipient::Agent(agent("codex-1"));
        fixture
            .bus
            .send(&agent("claude-1"), &to, &chat("please review"))
            .await
            .unwrap();

        let woken = fixture
            .bus
            .team_set(&team_id(), &agent("codex-1"), MemberState::Sleeping)
            .await
            .unwrap();

        assert_eq!(woken, Woken::Message);
        let team = fixture.bus.team_get(&team_id()).await.unwrap();
        assert_eq!(team.members.get(&agent("codex-1")).unwrap().state, MemberState::Waking);
    }

    #[tokio::test]
    async fn team_set_wakes_a_member_that_goes_to_sleep_with_work_on_the_board() {
        let fixture = with_team().await;
        read_messages_as(&fixture, "codex-1", "bbb2").await;
        let work = board_item("Parse the thing", BoardLevel::Normal, Some("codex-1"));
        fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &work)
            .await
            .unwrap();

        let woken = fixture
            .bus
            .team_set(&team_id(), &agent("codex-1"), MemberState::Sleeping)
            .await
            .unwrap();

        assert_eq!(woken, Woken::Board);
    }

    #[tokio::test]
    async fn team_set_does_not_wake_a_member_of_a_finished_team() {
        let fixture = with_team().await;
        fixture.bus.team_done(&team_id()).await.unwrap();

        let woken = fixture
            .bus
            .team_set(&team_id(), &agent("codex-1"), MemberState::Sleeping)
            .await
            .unwrap();

        assert_eq!(woken, Woken::No);
    }

    #[tokio::test]
    async fn team_set_names_each_state() {
        for state in [
            MemberState::Done,
            MemberState::Excluded,
            MemberState::Hung,
            MemberState::Paused,
            MemberState::Stopped,
        ] {
            // An excluded member stays excluded, so each state is set on a team of its own.
            let fixture = with_team().await;

            fixture
                .bus
                .team_set(&team_id(), &agent("gemini-1"), state)
                .await
                .unwrap();

            let team = fixture.bus.team_get(&team_id()).await.unwrap();
            assert_eq!(team.members.get(&agent("gemini-1")).unwrap().state, state);
        }
    }

    #[tokio::test]
    async fn team_set_of_a_member_that_is_not_in_the_team_is_not_found() {
        let fixture = with_team().await;

        let result = fixture
            .bus
            .team_set(&team_id(), &agent("opencode-1"), MemberState::Done)
            .await;

        assert_eq!(failure_class(result), Some(FailureClass::NotFound));
    }

    #[tokio::test]
    async fn team_done_marks_the_team_finished_and_is_harmless_again_or_for_no_team() {
        let fixture = with_team().await;

        fixture.bus.team_done(&team_id()).await.unwrap();
        fixture.bus.team_done(&team_id()).await.unwrap();
        let other: TeamId = "team-20261112-134008-zzzz".parse().unwrap();
        fixture.bus.team_done(&other).await.unwrap();

        assert!(fixture.bus.team_get(&team_id()).await.unwrap().done);
    }

    // ------------------------------------------------------------ the board

    #[tokio::test]
    async fn board_add_puts_items_on_the_board_numbered_in_order() {
        let fixture = with_team().await;
        let first = board_item("Zażółć gęślą jaźń", BoardLevel::Normal, None);
        let second = board_item("-rf looks like a flag", BoardLevel::Easy, None);

        let a = fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &first)
            .await
            .unwrap();
        let b = fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &second)
            .await
            .unwrap();

        assert_eq!((a, b), (item(1), item(2)));
        let board = fixture.bus.board_dump(&team_id()).await.unwrap().board;
        let seen: Vec<_> = board
            .values()
            .map(|added| (added.title.as_str(), added.level))
            .collect();
        assert_eq!(
            seen,
            [
                ("Zażółć gęślą jaźń", BoardLevel::Normal),
                ("-rf looks like a flag", BoardLevel::Easy)
            ]
        );
    }

    #[tokio::test]
    async fn board_add_for_a_member_designates_the_item_to_it() {
        let fixture = with_team().await;
        let work = board_item("Fix it", BoardLevel::Normal, Some("codex-1"));

        fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &work)
            .await
            .unwrap();

        let board = fixture.bus.board_dump(&team_id()).await.unwrap().board;
        let added = board.get(&item(1)).unwrap();
        assert_eq!(added.designated_for, Some(agent("codex-1")));
        assert_eq!(added.creator, agent("claude-1"));
        assert_eq!((added.level, added.status), (BoardLevel::Normal, BoardStatus::Todo));
        assert_eq!(added.details, "Do it\nand report.");
        assert_eq!((added.assignee.clone(), added.previous.clone()), (None, None));
        assert_eq!((added.result.as_str(), added.progress.as_str()), ("", ""));
    }

    #[tokio::test]
    async fn board_add_refuses_what_the_bus_refuses_by_class() {
        let fixture = with_team().await;
        let cases = [
            (
                board_item("  ", BoardLevel::Normal, None),
                "claude-1",
                FailureClass::InvalidArgument,
            ),
            (
                board_item("t", BoardLevel::Normal, Some("gemini-1")),
                "claude-1",
                FailureClass::InvalidArgument,
            ),
            (
                board_item("t", BoardLevel::Normal, Some("opencode-1")),
                "claude-1",
                FailureClass::NotFound,
            ),
        ];
        for (new, member, class) in cases {
            let result = fixture.bus.board_add(&team_id(), &agent(member), &new).await;

            assert_eq!(failure_class(result), Some(class), "{new:?} as {member}");
        }
    }

    // The bus does not check that the caller of `board-add` and `board-claim` is a member of the team: an agent
    // that is none can add to the board and claim an item. The wrapper passes it on, and this test makes a change of
    // the bus show up here.
    #[tokio::test]
    async fn board_add_and_board_claim_do_not_check_that_the_caller_is_a_member() {
        let fixture = with_team().await;
        let outsider = agent("opencode-1");
        let work = board_item("t", BoardLevel::Normal, None);

        fixture.bus.board_add(&team_id(), &outsider, &work).await.unwrap();
        fixture.bus.board_claim(&team_id(), &outsider, item(1)).await.unwrap();

        let board = fixture.bus.board_dump(&team_id()).await.unwrap().board;
        let claimed = board.get(&item(1)).unwrap();
        assert_eq!(claimed.status, BoardStatus::Claimed);
        assert_eq!(claimed.assignee, Some(outsider));
    }

    #[tokio::test]
    async fn board_add_to_a_team_that_does_not_exist_is_not_in_a_team() {
        let fixture = fixture();
        let work = board_item("t", BoardLevel::Normal, None);

        let result = fixture.bus.board_add(&team_id(), &agent("claude-1"), &work).await;

        assert_eq!(failure_class(result), Some(FailureClass::NotInTeam));
    }

    #[tokio::test]
    async fn board_dump_of_an_empty_board_is_empty_and_of_a_team_that_does_not_exist_is_not_found() {
        let fixture = with_team().await;
        let other: TeamId = "team-20261112-134008-zzzz".parse().unwrap();

        let empty = fixture.bus.board_dump(&team_id()).await.unwrap();
        let missing = fixture.bus.board_dump(&other).await;

        assert!(empty.board.is_empty());
        assert_eq!(failure_class(missing), Some(FailureClass::NotFound));
    }

    #[tokio::test]
    async fn board_claim_gives_the_item_to_the_member_and_the_board_says_so() {
        let fixture = with_team().await;
        let work = board_item("Parse it", BoardLevel::Normal, None);
        fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &work)
            .await
            .unwrap();

        fixture
            .bus
            .board_claim(&team_id(), &agent("codex-1"), item(1))
            .await
            .unwrap();

        let board = fixture.bus.board_dump(&team_id()).await.unwrap().board;
        let claimed = board.get(&item(1)).unwrap();
        assert_eq!(claimed.status, BoardStatus::Claimed);
        assert_eq!(claimed.assignee, Some(agent("codex-1")));
    }

    #[tokio::test]
    async fn board_claim_refuses_what_the_bus_refuses_by_class() {
        let fixture = with_team().await;
        let normal = board_item("Parse it", BoardLevel::Normal, None);
        let reserved = board_item("For claude", BoardLevel::Normal, Some("claude-1"));
        fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &normal)
            .await
            .unwrap();
        fixture
            .bus
            .board_add(&team_id(), &agent("claude-1"), &reserved)
            .await
            .unwrap();
        fixture
            .bus
            .board_claim(&team_id(), &agent("codex-1"), item(1))
            .await
            .unwrap();
        let cases = [
            ("claude-1", item(1), FailureClass::Conflict),
            ("codex-1", item(2), FailureClass::Permission),
            ("gemini-1", item(2), FailureClass::Permission),
            ("codex-1", item(9), FailureClass::NotFound),
        ];
        for (member, id, class) in cases {
            let result = fixture.bus.board_claim(&team_id(), &agent(member), id).await;

            assert_eq!(failure_class(result), Some(class), "{member} claims {id}");
        }
    }

    #[tokio::test]
    async fn board_release_agent_puts_the_items_of_an_agent_back_and_lists_them() {
        let fixture = with_team().await;
        let work = board_item("Parse it", BoardLevel::Normal, None);
        let for_codex = board_item("For codex", BoardLevel::Normal, Some("codex-1"));
        let other = board_item("Not codex's", BoardLevel::Normal, None);
        for new in [&work, &for_codex, &other] {
            fixture
                .bus
                .board_add(&team_id(), &agent("claude-1"), new)
                .await
                .unwrap();
        }
        fixture
            .bus
            .board_claim(&team_id(), &agent("codex-1"), item(1))
            .await
            .unwrap();
        fixture
            .bus
            .board_claim(&team_id(), &agent("claude-1"), item(3))
            .await
            .unwrap();

        let released = fixture
            .bus
            .board_release_agent(&team_id(), &agent("codex-1"))
            .await
            .unwrap();

        assert_eq!(released, [item(1), item(2)]);
        let board = fixture.bus.board_dump(&team_id()).await.unwrap().board;
        assert_eq!(board.get(&item(1)).unwrap().status, BoardStatus::Todo);
        assert_eq!(board.get(&item(1)).unwrap().previous, Some(agent("codex-1")));
        assert_eq!(board.get(&item(2)).unwrap().designated_for, None);
        assert_eq!(board.get(&item(3)).unwrap().status, BoardStatus::Claimed);
    }

    #[tokio::test]
    async fn board_release_agent_of_an_agent_without_items_lists_nothing_and_of_no_team_is_not_found() {
        let fixture = with_team().await;
        let other: TeamId = "team-20261112-134008-zzzz".parse().unwrap();

        let released = fixture
            .bus
            .board_release_agent(&team_id(), &agent("codex-1"))
            .await
            .unwrap();
        let missing = fixture.bus.board_release_agent(&other, &agent("codex-1")).await;

        assert_eq!(released, []);
        assert_eq!(failure_class(missing), Some(FailureClass::NotFound));
    }

    // ------------------------------------------------------------ the queue of requests for workers

    fn delegation(request: &str) -> Delegation {
        Delegation {
            team: team_id(),
            requester: agent("claude-1"),
            task: task("aaa1"),
            repo: repo(),
            branch: branch("agents/aaa1/item-1"),
            worktree: abs(Path::new("/tmp/example-worktrees/aaa1")),
            // The bus runs it with `_dispatch-helpers` once the request is queued.
            dispatcher: Program::from_str("true").unwrap(),
            request: request.to_owned(),
            context: "See README.md".to_owned(),
        }
    }

    fn request_id(number: u64) -> RequestId {
        RequestId::new(NonZeroU64::new(number).unwrap())
    }

    #[tokio::test]
    async fn delegate_queues_a_request_that_helper_get_reads() {
        let fixture = with_team().await;

        let id = fixture.bus.delegate(&delegation("Run the linter: żółć")).await.unwrap();
        let again = fixture.bus.delegate(&delegation("Second")).await.unwrap();

        assert_eq!((id, again), (request_id(1), request_id(2)));
        let read = fixture.bus.helper_get(id).await.unwrap();
        assert_eq!(read.id, id);
        assert_eq!(read.from, agent("claude-1"));
        assert_eq!(read.from_task, task("aaa1"));
        assert_eq!(read.repo, repo());
        assert_eq!(read.branch, branch("agents/aaa1/item-1"));
        assert_eq!(read.worktree, PathBuf::from("/tmp/example-worktrees/aaa1"));
        assert_eq!(read.request, "Run the linter: żółć");
        assert_eq!(read.context, "See README.md");
        assert_eq!(read.status, DelegationStatus::Queued);
        assert_eq!((read.worker, read.helper_task, read.summary.as_str()), (None, None, ""));
    }

    /// A dispatcher that records how it was run: its arguments and `AGENTS_HOME`, one per line, in `dispatched` in
    /// that home. Written once and closed before any test runs it, so that no `exec` meets a file still open for
    /// writing (`ETXTBSY`).
    fn recording_dispatcher() -> &'static Path {
        static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
        SCRIPT.get_or_init(|| {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;

            let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("recording-dispatcher");
            let script = "#!/bin/sh\n\
                { printf '%s\\n' \"$AGENTS_HOME\"; printf '%s\\n' \"$@\"; } > \"$AGENTS_HOME/dispatched.part\"\n\
                mv \"$AGENTS_HOME/dispatched.part\" \"$AGENTS_HOME/dispatched\"\n";
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o755)
                .open(&path)
                .unwrap();
            file.write_all(script.as_bytes()).unwrap();
            file.sync_all().unwrap();
            drop(file);
            path
        })
    }

    #[tokio::test]
    async fn delegate_runs_the_dispatcher_it_is_given_with_dispatch_helpers_in_its_home() {
        let fixture = with_team().await;
        let delegation = Delegation {
            dispatcher: Program::from_str(recording_dispatcher().to_str().unwrap()).unwrap(),
            ..delegation("Run the linter")
        };

        fixture.bus.delegate(&delegation).await.unwrap();

        // The bus starts it detached and does not wait for it: wait for what it wrote.
        let record = fixture.dir.path().join("dispatched");
        let deadline = Instant::now() + PATIENCE;
        let written = loop {
            if let Ok(written) = read_to_string(&record) {
                break written;
            }
            assert!(Instant::now() < deadline, "the dispatcher was not run");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let home = fixture.dir.path().to_str().unwrap();
        assert_eq!(written, format!("{home}\n_dispatch-helpers\n"));
    }

    #[tokio::test]
    async fn delegate_with_an_empty_request_is_invalid() {
        let fixture = with_team().await;

        let result = fixture.bus.delegate(&delegation("   ")).await;

        assert_eq!(failure_class(result), Some(FailureClass::InvalidArgument));
    }

    #[tokio::test]
    async fn helper_next_gives_the_oldest_request_to_the_worker_once() {
        let fixture = with_team().await;
        fixture.bus.delegate(&delegation("First")).await.unwrap();
        fixture.bus.delegate(&delegation("Second")).await.unwrap();

        let first = fixture.bus.helper_next(&agent("gemini-1")).await.unwrap().unwrap();
        let second = fixture.bus.helper_next(&agent("gemini-1")).await.unwrap().unwrap();
        let none = fixture.bus.helper_next(&agent("gemini-1")).await.unwrap();

        assert_eq!((first.id, first.request.as_str()), (request_id(1), "First"));
        assert_eq!(
            (first.status, first.worker),
            (DelegationStatus::Starting, Some(agent("gemini-1")))
        );
        assert_eq!(second.id, request_id(2));
        assert_eq!(none, None);
    }

    #[tokio::test]
    async fn helper_next_on_an_empty_queue_gives_none() {
        let fixture = with_team().await;

        let next = fixture.bus.helper_next(&agent("gemini-1")).await.unwrap();

        assert_eq!(next, None);
    }

    #[tokio::test]
    async fn helper_set_changes_only_what_it_is_given() {
        let fixture = with_team().await;
        let id = fixture.bus.delegate(&delegation("First")).await.unwrap();
        let running = DelegationUpdate {
            status: Some(DelegationStatus::Running),
            task: Some(task("ddd4")),
            summary: None,
        };
        let done = DelegationUpdate {
            status: Some(DelegationStatus::Done),
            task: None,
            summary: Some("All green. Łódź.".to_owned()),
        };

        fixture.bus.helper_set(id, &running).await.unwrap();
        let after_running = fixture.bus.helper_get(id).await.unwrap();
        fixture.bus.helper_set(id, &done).await.unwrap();
        fixture.bus.helper_set(id, &DelegationUpdate::default()).await.unwrap();
        let after_done = fixture.bus.helper_get(id).await.unwrap();

        assert_eq!(after_running.status, DelegationStatus::Running);
        assert_eq!(after_running.helper_task, Some(task("ddd4")));
        assert_eq!(after_running.summary, "");
        assert_eq!(after_done.status, DelegationStatus::Done);
        assert_eq!(after_done.helper_task, Some(task("ddd4")));
        assert_eq!(after_done.summary, "All green. Łódź.");
    }

    #[tokio::test]
    async fn helper_set_and_get_of_a_request_that_does_not_exist_are_not_found() {
        let fixture = with_team().await;

        let set = fixture
            .bus
            .helper_set(request_id(9), &DelegationUpdate::default())
            .await;
        let get = fixture.bus.helper_get(request_id(9)).await;

        assert_eq!(failure_class(set), Some(FailureClass::NotFound));
        assert_eq!(failure_class(get), Some(FailureClass::NotFound));
    }

    #[tokio::test]
    async fn every_delegation_status_is_set_and_read_back() {
        let fixture = with_team().await;
        let id = fixture.bus.delegate(&delegation("First")).await.unwrap();
        for status in [
            DelegationStatus::Starting,
            DelegationStatus::Running,
            DelegationStatus::Failed,
            DelegationStatus::Unclear,
            DelegationStatus::Done,
            DelegationStatus::Queued,
        ] {
            let update = DelegationUpdate {
                status: Some(status),
                ..DelegationUpdate::default()
            };

            fixture.bus.helper_set(id, &update).await.unwrap();

            assert_eq!(fixture.bus.helper_get(id).await.unwrap().status, status);
        }
    }

    // ------------------------------------------------------------ checkpoints, messages, status

    #[tokio::test]
    async fn checkpoint_get_of_a_key_without_a_checkpoint_is_none() {
        let fixture = with_team().await;

        let checkpoint = fixture
            .bus
            .checkpoint_get(&CheckpointKey::Team(team_id()))
            .await
            .unwrap();

        assert_eq!(checkpoint, None);
    }

    #[tokio::test]
    async fn checkpoint_get_reads_the_checkpoint_of_a_team_and_of_a_task() {
        let fixture = with_team().await;
        let directory = fixture.bus.bus_dir().join("checkpoints");
        create_dir_all(&directory).unwrap();
        write(
            directory.join(format!("team-{TEAM}.md")),
            "\n## Checkpoint\n\nZrobione: łódź\n",
        )
        .unwrap();
        write(directory.join(format!("task-{}.md", task("aaa1"))), "own notes").unwrap();

        let team = fixture
            .bus
            .checkpoint_get(&CheckpointKey::Team(team_id()))
            .await
            .unwrap();
        let own = fixture
            .bus
            .checkpoint_get(&CheckpointKey::Task(task("aaa1")))
            .await
            .unwrap();

        assert_eq!(team.as_deref(), Some("\n## Checkpoint\n\nZrobione: łódź\n"));
        assert_eq!(own.as_deref(), Some("own notes"));
    }

    fn chat(text: &str) -> Outgoing {
        Outgoing::Chat { text: text.to_owned() }
    }

    #[tokio::test]
    async fn send_puts_messages_on_the_bus_numbered_in_order() {
        let fixture = fixture();
        let from = agent("claude-1");

        let first = fixture
            .bus
            .send(&from, &Recipient::All, &chat("hello all"))
            .await
            .unwrap();
        let second = fixture
            .bus
            .send(
                &from,
                &Recipient::Team(team_id()),
                &chat("-v is not a flag\nsecond line: łódź"),
            )
            .await
            .unwrap();

        assert_eq!((first.id.get(), second.id.get()), (1, 2));
        let log = read_to_string(fixture.bus.bus_dir().join("messages.jsonl")).unwrap();
        assert_eq!(log.lines().count(), 2);
        assert!(log.contains("hello all") && log.contains("-v is not a flag"), "{log}");
        assert!(log.contains(&format!("team:{TEAM}")), "{log}");
    }

    #[tokio::test]
    async fn send_puts_an_event_of_the_harness_with_its_details_on_the_bus() {
        let fixture = with_team().await;
        let fields = BTreeMap::from([
            ("member".parse::<FieldName>().unwrap(), "codex-1".to_owned()),
            ("until".parse::<FieldName>().unwrap(), "2026-11-12T12:00:00Z".to_owned()),
        ]);
        let notice = Outgoing::Notice {
            kind: NoticeKind::MemberPaused,
            fields,
            text: "codex-1 is paused".to_owned(),
        };

        let sent = fixture
            .bus
            .send(&agent("daemon"), &Recipient::Team(team_id()), &notice)
            .await
            .unwrap();

        assert_eq!(sent.id.get(), 2, "the team's creation is message 1");
        let log = read_to_string(fixture.bus.bus_dir().join("messages.jsonl")).unwrap();
        let last = log.lines().last().unwrap();
        assert!(
            last.contains("member_paused") && last.contains("2026-11-12T12:00:00Z"),
            "{last}"
        );
    }

    #[tokio::test]
    async fn send_of_an_empty_text_is_invalid() {
        let fixture = fixture();

        let result = fixture
            .bus
            .send(&agent("claude-1"), &Recipient::All, &chat("  \n"))
            .await;

        assert_eq!(failure_class(result), Some(FailureClass::InvalidArgument));
    }

    #[tokio::test]
    async fn status_of_an_empty_bus_has_no_agents_and_no_reservations() {
        let fixture = fixture();

        let status = fixture.bus.status().await.unwrap();

        assert_eq!((status.agents, status.reservations), (Vec::new(), Vec::<String>::new()));
    }

    #[tokio::test]
    async fn status_lists_an_agent_that_is_running() {
        let fixture = fixture();
        let mut serve = Command::new(agentbus_binary())
            .arg("serve")
            .args(["--agent", "claude-1", "--task"])
            .arg(task("aaa1").as_str())
            .args(["--repo", "team-agents", "--branch", "agents/aaa1/item-1", "--dir"])
            .arg(fixture.bus.bus_dir())
            .env("AGENTS_HOME", fixture.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            let status = fixture.bus.status().await.unwrap();
            if !status.agents.is_empty() {
                break status;
            }
            assert!(Instant::now() < deadline, "the agent did not show up in the status");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let pid = serve.id().unwrap();
        serve.kill().await.unwrap();

        let listed = status.agents.first().unwrap();
        assert_eq!(status.agents.len(), 1);
        assert_eq!(listed.agent, agent("claude-1"));
        assert_eq!(listed.task, task("aaa1"));
        assert_eq!(listed.branch, branch("agents/aaa1/item-1"));
        assert_eq!(listed.pid, pid);
    }

    /// Starts the `agentbus serve` of `member` on the bus of `fixture`, sends it `requests` (one JSON-RPC message
    /// per line, after the initialisation) and keeps its input open: an agent that has ended is no longer listed.
    async fn serve_and_call(fixture: &Fixture, member: &str, suffix: &str, requests: &[&str]) -> tokio::process::Child {
        let mut serve = Command::new(agentbus_binary())
            .arg("serve")
            .args(["--agent", member, "--task"])
            .arg(task(suffix).as_str())
            .args(["--repo", "team-agents", "--branch", "agents/x/item-1", "--dir"])
            .arg(fixture.bus.bus_dir())
            .env("AGENTS_HOME", fixture.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = serve.stdin.as_mut().unwrap();
        let initialise = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
        for line in [initialise, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#]
            .into_iter()
            .chain(requests.iter().copied())
        {
            stdin.write_all(line.as_bytes()).await.unwrap();
            stdin.write_all(b"\n").await.unwrap();
        }
        stdin.flush().await.unwrap();
        serve
    }

    #[tokio::test]
    async fn status_does_not_hide_an_agent_that_the_bus_lists_without_a_task() {
        let fixture = fixture();
        let _first = serve_and_call(&fixture, "claude-1", "aaa1", &[]).await;
        // No `--task` and no `--branch`: the bus lists this agent with an empty task and branch.
        let _second = Command::new(agentbus_binary())
            .arg("serve")
            .args(["--agent", "codex-1", "--dir"])
            .arg(fixture.bus.bus_dir())
            .env("AGENTS_HOME", fixture.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + PATIENCE;
        loop {
            match fixture.bus.status().await {
                Err(AgentBusError::Unexpected {
                    command: BusCommand::Status,
                    ..
                }) => break,
                Err(other) => panic!("{other:?}"),
                Ok(status) => {
                    let glued = status.agents.iter().any(|listed| listed.status.contains(" pid "));
                    assert!(!glued, "an agent was added to the status of another: {status:?}");
                }
            }
            assert!(Instant::now() < deadline, "the status never showed the second agent");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn status_keeps_the_line_breaks_of_an_agents_status_and_of_a_reservation() {
        let fixture = fixture();
        let set_status = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"set_status","arguments":{"status":"Reviewing\nWaiting for tests: żółć"}}}"#;
        let reserve = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"reserve","arguments":{"resource":"lab","note":"flashing\nthe second board"}}}"#;
        let mut serve = serve_and_call(&fixture, "claude-1", "aaa1", &[set_status, reserve]).await;
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            let status = fixture.bus.status().await.unwrap();
            if !status.agents.is_empty() && !status.reservations.is_empty() {
                break status;
            }
            assert!(Instant::now() < deadline, "the agent did not show up in the status");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        serve.kill().await.unwrap();

        assert_eq!(status.agents.len(), 1);
        assert_eq!(
            status.agents.first().map(|agent| agent.status.as_str()),
            Some("Reviewing\nWaiting for tests: żółć")
        );
        assert_eq!(status.reservations.len(), 1);
        let reservation = status.reservations.first().unwrap();
        assert!(reservation.starts_with("- lab: claude-1"), "{reservation}");
        assert!(
            reservation.ends_with("purpose: flashing\nthe second board"),
            "{reservation}"
        );
    }

    // ------------------------------------------------------------ what can go wrong with the process

    #[tokio::test]
    async fn a_program_that_is_not_there_is_a_run_error() {
        let dir = tempfile::tempdir().unwrap();
        let program = Program::from_str("/nonexistent/agentbus-test-program").unwrap();
        let bus = AgentBus::new(program, abs(dir.path()));

        let error = bus.status().await.unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Run {
                    source: CaptureError::Spawn { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(error.class(), None);
    }

    #[tokio::test]
    async fn an_exit_status_that_is_no_class_is_unclassified() {
        let dir = tempfile::tempdir().unwrap();
        let bus = AgentBus::new(Program::from_str("false").unwrap(), abs(dir.path()));

        let error = bus.status().await.unwrap_err();

        assert!(
            matches!(
                &error,
                AgentBusError::Failed {
                    exit_code: 1,
                    class: FailureClass::Unclassified,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_command_that_prints_what_is_no_json_is_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let bus = AgentBus::new(Program::from_str("echo").unwrap(), abs(dir.path()));

        let error = bus.team_get(&team_id()).await.unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Decode {
                    command: BusCommand::TeamGet,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_command_that_waits_for_the_lock_longer_than_its_time_is_killed_and_is_a_timeout() {
        let fixture = fixture();
        let bus = fixture.bus.clone().with_timeout(Duration::from_secs(30));
        // Another process holds the lock of the bus, as one that is in the middle of an operation does.
        create_dir_all(bus.bus_dir()).unwrap();
        let lock = File::create(bus.bus_dir().join("bus.lock")).unwrap();
        flock(&lock, FlockOperation::LockExclusive).unwrap();

        let error = bus.status().await.unwrap_err();

        assert!(
            matches!(&error, AgentBusError::Run { source: CaptureError::Timeout { limit }, .. } if *limit == Duration::from_secs(30)),
            "{error:?}"
        );
        flock(&lock, FlockOperation::Unlock).unwrap();
    }
}
