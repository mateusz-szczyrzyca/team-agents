//! Opening a questions file in the VS Code window connected to the host: which command and which window, what is
//! skipped, and what is said when nothing works. The remote command is a committed stand-in script.

#[cfg(test)]
mod editor {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use agent_runtime::editor::{OpenError, Opened, VscodeOpener};
    use rustix::fs::{AtFlags, CWD, Timespec, Timestamps, utimensat};

    /// The stand-in for the remote command of a server.
    fn fake_cli() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor/fake-vscode-cli.sh")
    }

    /// A host: a directory for the servers, one for the runtime directory of the session and one for the others.
    struct Host {
        _dir: tempfile::TempDir,
        servers: PathBuf,
        runtime: PathBuf,
        fallback: PathBuf,
        /// The sockets made, kept open so that they exist as sockets.
        listeners: Vec<UnixListener>,
    }

    impl Host {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let make = |name: &str| {
                let path = dir.path().join(name);
                std::fs::create_dir(&path).unwrap();
                path
            };
            Self {
                servers: make("server"),
                runtime: make("runtime"),
                fallback: make("fallback"),
                _dir: dir,
                listeners: Vec::new(),
            }
        }

        fn opener(&self) -> VscodeOpener {
            VscodeOpener::new(vec![self.servers.clone()], self.runtime.clone(), self.fallback.clone())
        }

        /// Puts the remote command where a server of the `cli/servers` layout has it, as `name` under `build`.
        fn server(&self, build: &str, name: &str) -> PathBuf {
            let dir = self
                .servers
                .join("cli/servers")
                .join(build)
                .join("server/bin/remote-cli");
            std::fs::create_dir_all(&dir).unwrap();
            let link = dir.join(name);
            symlink(fake_cli(), &link).unwrap();
            link
        }

        /// A socket of a window named `name` in `dir`, last changed `age` seconds ago.
        fn window(&mut self, dir: &Path, name: &str, age: u64) -> PathBuf {
            let socket = dir.join(format!("vscode-ipc-{name}.sock"));
            self.listeners.push(UnixListener::bind(&socket).unwrap());
            let then = SystemTime::now() - Duration::from_secs(age);
            let since = then.duration_since(SystemTime::UNIX_EPOCH).unwrap();
            let stamp = Timespec {
                tv_sec: i64::try_from(since.as_secs()).unwrap(),
                tv_nsec: 0,
            };
            utimensat(
                CWD,
                &socket,
                &Timestamps {
                    last_access: stamp,
                    last_modification: stamp,
                },
                AtFlags::empty(),
            )
            .unwrap();
            socket
        }
    }

    /// The files the stand-in command opened through `socket`.
    fn opened_through(socket: &Path) -> Vec<String> {
        let mut name = socket.as_os_str().to_owned();
        name.push(".opened");
        std::fs::read_to_string(PathBuf::from(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn the_file_is_opened_in_the_window_of_the_session_through_the_command_of_the_server() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        let socket = host.window(&host.runtime.clone(), "a", 10);

        let opened = host
            .opener()
            .open(Path::new("/h/tasks/t/questions-1.md"))
            .await
            .unwrap();

        assert_eq!(opened, Opened::Window { socket: socket.clone() });
        assert_eq!(opened_through(&socket), ["/h/tasks/t/questions-1.md"]);
    }

    #[tokio::test]
    async fn the_command_is_found_under_both_layouts_of_a_server() {
        let mut host = Host::new();
        let dir = host.servers.join("bin/abc123/bin/remote-cli");
        std::fs::create_dir_all(&dir).unwrap();
        symlink(fake_cli(), dir.join("code")).unwrap();
        let socket = host.window(&host.runtime.clone(), "a", 10);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::Window { socket });
    }

    #[tokio::test]
    async fn without_a_server_there_is_nothing_to_open_the_file_in() {
        let mut host = Host::new();
        host.window(&host.runtime.clone(), "a", 10);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoServer);
    }

    #[tokio::test]
    async fn a_server_without_a_window_is_told_apart_from_no_server() {
        let host = Host::new();
        host.server("Stable-1", "code");

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoWindow);
    }

    #[tokio::test]
    async fn the_newest_window_is_tried_first() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        let older = host.window(&host.runtime.clone(), "older", 100);
        let newer = host.window(&host.runtime.clone(), "newer", 5);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::Window { socket: newer });
        assert_eq!(opened_through(&older), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_window_that_was_closed_is_skipped_for_the_next_one() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        host.window(&host.runtime.clone(), "dead-newest", 5);
        let alive = host.window(&host.runtime.clone(), "alive", 100);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::Window { socket: alive });
    }

    #[tokio::test]
    async fn the_windows_of_the_session_are_tried_before_the_others() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        let own = host.window(&host.runtime.clone(), "own", 100);
        let other = host.window(&host.fallback.clone(), "other", 5);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::Window { socket: own });
        assert_eq!(opened_through(&other), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_window_of_the_other_directory_is_used_when_the_session_has_none_that_works() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        host.window(&host.runtime.clone(), "dead-own", 5);
        let other = host.window(&host.fallback.clone(), "other", 100);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::Window { socket: other });
    }

    #[tokio::test]
    async fn files_that_are_not_sockets_are_not_windows() {
        let host = Host::new();
        host.server("Stable-1", "code");
        std::fs::write(host.runtime.join("vscode-ipc-a.sock"), "not a socket").unwrap();
        std::fs::write(host.runtime.join("other.sock"), "not a window").unwrap();

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoWindow);
    }

    #[tokio::test]
    async fn when_every_window_refuses_the_last_one_tried_is_named_with_what_it_said() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        host.window(&host.runtime.clone(), "dead-a", 100);
        let newest = host.window(&host.runtime.clone(), "dead-b", 5);

        let error = host.opener().open(Path::new("/f.md")).await.unwrap_err();

        assert!(
            matches!(&error, OpenError::Refused { file, socket, said }
                if file == Path::new("/f.md") && *socket != newest && said.contains("ECONNREFUSED")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_window_that_does_not_answer_is_given_up_after_the_time_it_has() {
        let mut host = Host::new();
        host.server("Stable-1", "code");
        host.window(&host.runtime.clone(), "slow", 5);
        let alive = host.window(&host.runtime.clone(), "alive", 100);

        let opened = host
            .opener()
            .with_timeout(Duration::from_millis(300))
            .open(Path::new("/f.md"))
            .await
            .unwrap();

        assert_eq!(opened, Opened::Window { socket: alive });
    }

    #[tokio::test]
    async fn a_directory_of_servers_that_is_not_there_is_no_server() {
        let host = Host::new();
        let opener = VscodeOpener::new(
            vec![host.servers.join("nothing")],
            host.runtime.clone(),
            host.fallback.clone(),
        );

        let opened = opener.open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoServer);
    }
}
