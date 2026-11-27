//! One `SIGKILL` of the real `agent daemon` while a worker's request, a review and an investigation are all going: after the
//! start each of them goes on from the `daemon_started` checkpoint of its own task. The world is a process world, as in
//! `worker_request_process.rs`: a temporary `AGENTS_HOME`, the stand-in CLI `team-cli.sh` for every agent, the real
//! `agentbus`, a worker tier, and the fake GitHub of the daemon's own tests (a pull request is opened, so a review starts).

/// The fake GitHub of the round tests, read from where it is: it serves the routes the harness uses and merges into a bare
/// repository. Most of its controls are for tests that are not here.
#[cfg(test)]
#[path = "../src/daemon/owner/rounds/tests/fake_github.rs"]
#[expect(
    dead_code,
    reason = "the fake serves many tests; this one uses its start, its address and its account"
)]
mod fake_github;

#[cfg(test)]
mod killed_daemon_all_jobs {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use agent_core::history::{CheckpointCause, CheckpointJob, HistoryEvent, Record};
    use agent_runtime::history::Reader;
    use rustix::process::{Pid, Signal};

    use super::fake_github::{ACCOUNT, FakeGitHub};

    /// The longest any wait of this test lasts.
    const PATIENCE: Duration = Duration::from_secs(90);

    /// The task list of the team: one item, for the member.
    const LIST: &str = "Write the guide.\n\n1. Guide\n[ ] 1.1 The walkthrough\n    Write it.\n";

    /// The plan of the member: a step in hand, which its request is for.
    const MEMBER_PLAN: &str = "\
Write the walkthrough of the guide.

1. Draft
[>] 1.1 Write the walkthrough
    Files: docs/guide.md
    One section per question a reader asks.
    Check: exists docs/guide.md
";

    /// The arguments of the member's request for its step 1.1.
    const MEMBER_REQUEST: &str = r#"{"goal":"the walkthrough of the guide","workdir":"/tmp","steps":"write the file","returns":"the path of the file","for":"1.1","files":"docs/guide.md, docs/partial.txt","check":"exists docs/guide.md","do":"Write the walkthrough, one section per question."}"#;

    /// The script of the worker: its first run leaves a partial file and goes on until the test releases it; a run after it
    /// completes the request.
    const WORKER_SCRIPT: &str = r#"printf '%s\n' '.fake-*' >> "$(git rev-parse --git-path info/exclude)"
if test ! -f .fake-started; then
    mkdir -p docs
    printf 'partial\n' > docs/partial.txt
    touch .fake-started
    while test ! -f "$AGENTS_HOME/release-worker"; do sleep 0.05; done
    touch .fake-orphan-ended
    exit 1
fi
printf 'the walkthrough\n' > docs/guide.md
git add docs
git commit -qm 'Complete retained work'
"#;

    /// What each kind of job leaves before the kill, as its next run is briefed with: the file of a worker's script, the
    /// finding a reviewer wrote, the notes of an investigator. No briefing holds what another job left.
    const JOBS_LEFT: [(CheckpointJob, &str); 3] = [
        (CheckpointJob::WorkerRequest, "docs/partial.txt"),
        (CheckpointJob::Review, "a panic before the kill"),
        (CheckpointJob::Investigation, "notes.md"),
    ];

    /// The tasks of the jobs, each with its kind of job: one worker, one investigator and the reviewers.
    type Jobs = Vec<(String, CheckpointJob)>;

    /// A checkpoint of a job, as the history holds it.
    struct Taken {
        /// The kind of job.
        job: CheckpointJob,
        /// The step it was taken after.
        cause: CheckpointCause,
        /// The task whose worktree it keeps.
        task: String,
        /// The tip of the branch in that worktree.
        head: Option<String>,
    }

    /// The `agentbus` built from `agentbus/`, once for this file.
    fn agentbus_binary() -> &'static Path {
        static BINARY: OnceLock<PathBuf> = OnceLock::new();
        BINARY.get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agentbus");
            let binary = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agentbus-under-test-of-killed-daemon");
            let built = Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(&binary)
                .arg(".")
                .current_dir(&source)
                .output()
                .unwrap();
            assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
            binary
        })
    }

    /// Runs git for the setup of a test.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The fake GitHub, served by the runtime of a thread of its own: the test thread blocks while it waits, and the daemon
    /// calls the fake all that time.
    struct Github {
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Github {
        /// Starts the fake for the repository whose remote is the bare repository `origin`; its base URL is the second value.
        fn start(origin: &Path) -> (Self, String) {
            let (found, wait) = std::sync::mpsc::channel();
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let origin = origin.to_owned();
            let thread = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let fake = FakeGitHub::start(&origin).await.unwrap();
                    found.send(fake.url()).unwrap();
                    stopped.await.unwrap();
                });
            });
            let url = wait.recv().unwrap();
            (
                Self {
                    stop: Some(stop),
                    thread: Some(thread),
                },
                url,
            )
        }
    }

    impl Drop for Github {
        fn drop(&mut self) {
            if let (Some(stop), Some(thread)) = (self.stop.take(), self.thread.take()) {
                stop.send(()).unwrap();
                thread.join().unwrap();
            }
        }
    }

    /// A home with a scratch repository `example` whose remote the fake GitHub serves, a lead, a member, the reviewers and
    /// an investigator, and one worker; the daemon is started by [`World::start`].
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
        _github: Github,
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let origin = root.join("octo/example.git");
            std::fs::create_dir_all(&origin).unwrap();
            git(&origin, &["init", "-q", "--bare", "-b", "main"]);
            let repo = root.join("example");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
            std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
            std::fs::write(repo.join("list.md"), LIST).unwrap();
            git(&repo, &["add", "README.md"]);
            git(&repo, &["commit", "-q", "-m", "First"]);
            git(&repo, &["push", "-q", "origin", "main"]);
            let (github, url) = Github::start(&origin);
            let mut config = String::new();
            for (name, kind, scenario) in [
                ("inv-codex", "investigators", "inv-review-hold"),
                ("inv-gemini", "investigators", "inv-review-hold"),
                ("inv-extra", "investigators", "inv-review-hold"),
                (
                    "claude-lead",
                    "agents",
                    "sleep,worker-investigation-integrate:walk:Why does the guide fail?,sleep",
                ),
                ("claude-1", "agents", "work"),
                ("w-one", "agents", "script:killed-restart"),
            ] {
                write!(
                    config,
                    "[{kind}.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \
                     \"--bus-bin\", \"{bus}\", \"--bus-dir\", \"{home}/bus\", \"--home\", \"{home}\"]\n",
                    bus = agentbus_binary().display(),
                    home = home.display(),
                )
                .unwrap();
            }
            write!(
                config,
                "[plan]\nbounces = 0\n[tiers]\nnormal = [\"claude-lead\", \"claude-1\"]\nworkers = [\"w-one\"]\n[binaries]\nclaude = \"{}\"\n\
                 agentbus = \"{}\"\n[github]\napi_base = \"{url}\"\ntrusted_accounts = [\"{ACCOUNT}\"]\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[disk]\nlow_free_gb = 0\nmin_free_gb = 0\n",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team-cli.sh").display(),
                agentbus_binary().display(),
            )
            .unwrap();
            std::fs::write(home.join("agents.toml"), config).unwrap();
            let secrets = home.join("secrets.toml");
            std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
            std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
            for (name, text) in [
                ("fake-plans/walk.md", MEMBER_PLAN),
                ("fake-requests/walk.jsonl", MEMBER_REQUEST),
            ] {
                let path = home.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, format!("{text}\n")).unwrap();
            }
            std::fs::create_dir(home.join("fake-scripts")).unwrap();
            std::fs::write(home.join("fake-scripts/killed-restart.sh"), WORKER_SCRIPT).unwrap();
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
                _github: github,
            }
        }

        fn start(&mut self) {
            let socket = self.home.join("agent.sock");
            let daemon = self
                .command()
                .arg("daemon")
                .arg("--prompts-dir")
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            self.daemon = Some(daemon);
            eventually("the socket", || socket.exists());
        }

        /// Ends the daemon with `signal` and waits for it to end.
        fn end(&mut self, signal: Signal) {
            if let Some(mut daemon) = self.daemon.take() {
                let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
                rustix::process::kill_process(pid, signal).unwrap();
                daemon.wait().unwrap();
            }
        }

        /// The `agent` binary in the scratch repository, in this home.
        fn command(&self) -> Command {
            let mut agent = Command::new(env!("CARGO_BIN_EXE_agent"));
            agent.current_dir(&self.repo).env("AGENTS_HOME", &self.home);
            agent
        }

        /// The worktree of `task`.
        fn worktree(&self, task: &str) -> PathBuf {
            self.home.join("worktrees/example").join(task)
        }

        /// The prompt the stand-in recorded for run `n` of `task`: it stays in the repository when the worktree goes.
        fn prompt(&self, task: &str, n: u32) -> Option<String> {
            let path = self
                .repo
                .join(".git/fake-records")
                .join(task)
                .join(format!(".fake-prompt-{n}"));
            std::fs::read_to_string(path).ok()
        }

        /// The bus's state, as JSON.
        fn bus(&self) -> serde_json::Value {
            let text = std::fs::read_to_string(self.home.join("bus/state.json")).unwrap();
            serde_json::from_str(&text).unwrap()
        }

        /// The checkpoints of the jobs other than an item, oldest first.
        fn job_checkpoints(&self) -> Vec<Taken> {
            let Ok(reader) = Reader::open(&self.home.join("history.db")) else {
                return Vec::new();
            };
            let mut taken = Vec::new();
            for team in reader.teams().unwrap() {
                for stored in reader.team_records(&team.value.team).unwrap() {
                    if let Record::Event(event) = stored.record
                        && let HistoryEvent::Checkpoint {
                            cause, task, job, head, ..
                        } = event.event
                        && job != CheckpointJob::Item
                    {
                        taken.push(Taken {
                            job,
                            cause,
                            task: task.to_string(),
                            head: head.map(|head| head.to_string()),
                        });
                    }
                }
            }
            taken
        }

        /// The tasks that took a checkpoint of `job` because of `cause`, once each.
        fn tasks_with(&self, job: CheckpointJob, cause: CheckpointCause) -> Vec<String> {
            let mut tasks: Vec<String> = (self.job_checkpoints().into_iter())
                .filter(|taken| taken.job == job && taken.cause == cause)
                .map(|taken| taken.task)
                .collect();
            tasks.dedup();
            tasks
        }

        /// The `daemon_started` checkpoints of `task`.
        fn daemon_started(&self, task: &str) -> Vec<Taken> {
            let all = self.job_checkpoints().into_iter();
            all.filter(|taken| taken.task == task && taken.cause == CheckpointCause::DaemonStarted)
                .collect()
        }

        /// Whether the bus has the investigation held by `task` answered.
        fn investigation_answered(&self, task: &str) -> bool {
            self.bus()["investigations"].as_object().is_some_and(|all| {
                all.values()
                    .any(|investigation| investigation["inv_task"] == task && investigation["status"] == "done")
            })
        }

        /// Starts the team of the member and its lead on the task list, without a merge.
        fn run(&self) {
            let args = ["run", "claude-lead", "claude-1", "--coordinator", "claude-lead"];
            let output = self
                .command()
                .args(args)
                .args(["-f", "list.md", "--no-merge"])
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }

        /// Waits until the request, the investigation and the reviews are going, each with its first run begun.
        fn jobs_going(&self) -> Jobs {
            let mut jobs = Jobs::new();
            eventually("the request, the review and the investigation to go", || {
                let tasks = |job, cause| self.tasks_with(job, cause);
                jobs = [
                    (
                        CheckpointJob::WorkerRequest,
                        tasks(CheckpointJob::WorkerRequest, CheckpointCause::RequestStarted),
                    ),
                    (
                        CheckpointJob::Review,
                        tasks(CheckpointJob::Review, CheckpointCause::ReviewStarted),
                    ),
                    (
                        CheckpointJob::Investigation,
                        tasks(CheckpointJob::Investigation, CheckpointCause::InvestigationStarted),
                    ),
                ]
                .into_iter()
                .flat_map(|(job, tasks)| tasks.into_iter().map(move |task| (task, job)))
                .collect();
                let begun = |(task, job): &(String, CheckpointJob)| match job {
                    CheckpointJob::WorkerRequest => self.worktree(task).join(".fake-started").exists(),
                    _ => self.prompt(task, 1).is_some(),
                };
                jobs.len() >= 3 && jobs.iter().all(begun)
            });
            jobs
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            self.end(Signal::TERM);
        }
    }

    #[test]
    fn daemon_killed_with_request_review_and_investigation_going() {
        let mut world = World::new();
        world.start();
        world.run();
        let jobs = world.jobs_going();
        let task_of = |wanted: CheckpointJob| jobs.iter().find(|(_, job)| *job == wanted).unwrap().0.as_str();
        let count = |wanted: CheckpointJob| jobs.iter().filter(|(_, job)| *job == wanted).count();
        assert_eq!(
            (count(CheckpointJob::WorkerRequest), count(CheckpointJob::Investigation)),
            (1, 1)
        );
        for (task, _) in &jobs {
            assert!(
                world.daemon_started(task).is_empty(),
                "{task} had no daemon_started before the kill"
            );
        }
        // What the reviewers and the investigator did before the kill: a file in the worktree, which the start of the daemon
        // finds changed, and a reviewer's finding in its findings file.
        for (task, job) in &jobs {
            let file = if *job == CheckpointJob::Review {
                "scratch.md"
            } else {
                "notes.md"
            };
            if *job != CheckpointJob::WorkerRequest {
                std::fs::write(world.worktree(task).join(file), "looked at before the kill\n").unwrap();
            }
            if *job == CheckpointJob::Review {
                let findings = world.home.join("tasks").join(task).join("review-pr1-pass1-findings.md");
                std::fs::write(findings, "1. [MAJOR] ERR-3 a.rs:4 — a panic before the kill\n").unwrap();
            }
        }

        world.end(Signal::KILL);
        // The stand-in the killed daemon left ends before the daemon starts again, so that it cannot race the next run.
        std::fs::write(world.home.join("release-worker"), "").unwrap();
        eventually("the orphaned worker has ended", || {
            world
                .worktree(task_of(CheckpointJob::WorkerRequest))
                .join(".fake-orphan-ended")
                .exists()
        });
        world.start();

        eventually("a briefed second run of every job", || {
            jobs.iter().all(|(task, _)| world.prompt(task, 2).is_some())
        });
        // The request needs the round open, which the approved reviews close: the held runs end after the request is done.
        eventually("the request done", || world.bus()["helpers"]["1"]["status"] == "done");
        for (task, _) in jobs.iter().filter(|(_, job)| *job != CheckpointJob::WorkerRequest) {
            std::fs::write(world.worktree(task).join(".fake-continue"), "").unwrap();
        }
        eventually("the investigation answered", || {
            world.investigation_answered(task_of(CheckpointJob::Investigation))
        });
        eventually("the reviews recorded", || {
            let submitted = world.tasks_with(CheckpointJob::Review, CheckpointCause::ReviewSubmitted);
            jobs.iter()
                .filter(|(_, job)| *job == CheckpointJob::Review)
                .all(|(task, _)| submitted.contains(task))
        });
        world.end(Signal::TERM);

        for taken in world.job_checkpoints() {
            let held = jobs.iter().find(|(task, _)| *task == taken.task);
            assert_eq!(
                held.map(|(_, job)| *job),
                Some(taken.job),
                "{} is the task of a job",
                taken.task
            );
        }
        for (task, job) in &jobs {
            let started = world.daemon_started(task);
            assert_eq!(started.len(), 1, "{task}");
            assert_eq!(started[0].job, *job, "{task}");
            let prompt = world.prompt(task, 2).unwrap();
            let briefing = prompt.split_once("## Checkpoint: ").unwrap().1;
            let (title, _) = briefing.split_once('\n').unwrap();
            assert!(
                title.ends_with(&format!(", head {}", started[0].head.as_ref().unwrap())),
                "{title}"
            );
            // A worker's request is handed over to its next run by the checkpoint that tells what the start took.
            assert!(
                title.starts_with("daemon_started at ") || title.starts_with("holder_changed at "),
                "{title}"
            );
            for (kind, left) in JOBS_LEFT {
                assert_eq!(
                    briefing.contains(left),
                    kind == *job,
                    "{task} is briefed with {left}: {briefing}"
                );
            }
            for (other, _) in jobs.iter().filter(|(other, _)| other != task) {
                assert!(!prompt.contains(other), "the prompt of {task} names {other}");
            }
        }
    }
}
