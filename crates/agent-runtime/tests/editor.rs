//! Opening a questions file in the VS Code window connected to the host: which command and which window, what is
//! skipped, and what is said when nothing works. The remote command is a committed stand-in script.

#[cfg(test)]
mod editor {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use agent_runtime::editor::{OpenError, Opened, SocketDirs, VscodeOpener};
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
            VscodeOpener::new(vec![self.servers.clone()], self.sockets())
        }

        fn sockets(&self) -> SocketDirs {
            SocketDirs {
                runtime: self.runtime.clone(),
                fallback: self.fallback.clone(),
            }
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
            grow_old(&socket, age);
            socket
        }
    }

    /// Sets the time `path` was last changed to `age` seconds ago.
    fn grow_old(path: &Path, age: u64) {
        let then = SystemTime::now() - Duration::from_secs(age);
        let since = then.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        let stamp = Timespec {
            tv_sec: i64::try_from(since.as_secs()).unwrap(),
            tv_nsec: 0,
        };
        utimensat(
            CWD,
            path,
            &Timestamps {
                last_access: stamp,
                last_modification: stamp,
            },
            AtFlags::empty(),
        )
        .unwrap();
    }

    /// A remote command at `dir/code` that records its own path where the stand-in would record the file, with the
    /// permission bits `mode`, last changed `age` seconds ago.
    fn command_that_names_itself(dir: &Path, mode: u32, age: u64) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("code");
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$0\" >> \"$VSCODE_IPC_HOOK_CLI.opened\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        grow_old(&path, age);
        path
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
        let looking = VscodeOpener::new(vec![host.servers.join("nothing")], host.sockets());

        let opened = looking.open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoServer);
    }

    #[tokio::test]
    async fn of_two_servers_the_command_changed_last_is_the_one_that_runs() {
        let mut host = Host::new();
        let older = host.servers.join("cli/servers/Stable-1/server/bin/remote-cli");
        let newer = host.servers.join("cli/servers/Stable-2/server/bin/remote-cli");
        command_that_names_itself(&older, 0o755, 1000);
        let expected = command_that_names_itself(&newer, 0o755, 10);
        let socket = host.window(&host.runtime.clone(), "a", 10);

        host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened_through(&socket), [expected.display().to_string()]);
    }

    #[tokio::test]
    async fn a_command_that_is_not_executable_is_not_run_however_new_it_is() {
        let mut host = Host::new();
        let older = host.servers.join("cli/servers/Stable-1/server/bin/remote-cli");
        let newer = host.servers.join("cli/servers/Stable-2/server/bin/remote-cli");
        let expected = command_that_names_itself(&older, 0o755, 1000);
        command_that_names_itself(&newer, 0o644, 10);
        let socket = host.window(&host.runtime.clone(), "a", 10);

        host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened_through(&socket), [expected.display().to_string()]);
    }

    #[tokio::test]
    async fn a_server_whose_only_command_is_not_executable_is_no_server() {
        let mut host = Host::new();
        command_that_names_itself(
            &host.servers.join("cli/servers/Stable-1/server/bin/remote-cli"),
            0o644,
            10,
        );
        host.window(&host.runtime.clone(), "a", 10);

        let opened = host.opener().open(Path::new("/f.md")).await.unwrap();

        assert_eq!(opened, Opened::NoServer);
    }

    #[tokio::test]
    async fn a_directory_of_sockets_that_cannot_be_listed_is_a_list_error_that_names_it() {
        let host = Host::new();
        host.server("Stable-1", "code");
        let not_a_directory = host.runtime.join("file");
        std::fs::write(&not_a_directory, "x").unwrap();
        let looking = VscodeOpener::new(
            vec![host.servers.clone()],
            SocketDirs {
                runtime: not_a_directory.clone(),
                fallback: host.fallback.clone(),
            },
        );

        let error = looking.open(Path::new("/f.md")).await.unwrap_err();

        assert!(
            matches!(&error, OpenError::List { path, .. } if *path == not_a_directory),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_command_that_cannot_be_started_is_a_start_error_that_names_it() {
        use std::os::unix::fs::PermissionsExt;
        let mut host = Host::new();
        let dir = host.servers.join("cli/servers/Stable-1/server/bin/remote-cli");
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("code");
        std::fs::write(&program, "#!/nonexistent/interpreter\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        host.window(&host.runtime.clone(), "a", 10);

        let error = host.opener().open(Path::new("/f.md")).await.unwrap_err();

        assert!(
            matches!(&error, OpenError::Start { program: named, .. } if *named == program),
            "{error:?}"
        );
    }
}
