//! A fake GitHub on a loopback port for the tests of the autopilot: the few routes of the REST API the harness uses,
//! with the state of a repository's pull requests. It speaks just enough HTTP/1.1 for `reqwest` (one request per
//! connection, a body given by `Content-Length`) and merges into a bare repository that stands for the remote, so that
//! what a merge changes is what the next item of a team starts from.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

/// The repository the fake serves, as GitHub names it.
pub(super) const SLUG: &str = "octo/example";

/// The login of the account the harness acts as.
pub(super) const ACCOUNT: &str = "ms-ai-agents";

/// What the fake saw of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    /// The method.
    pub(super) method: String,
    /// The path and the query.
    pub(super) target: String,
    /// The body, as text.
    pub(super) body: String,
}

impl Seen {
    /// The body as JSON; `Null` for a request without one.
    pub(super) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// A pull request of the fake.
#[derive(Debug, Clone)]
pub(super) struct Pull {
    /// Its number.
    pub(super) number: u32,
    /// Its title.
    pub(super) title: String,
    /// Its description.
    pub(super) body: String,
    /// Its head branch.
    pub(super) head: String,
    /// Its base branch.
    pub(super) base: String,
    /// Whether it is a draft.
    pub(super) draft: bool,
    /// Whether it was merged.
    pub(super) merged: bool,
    /// Whether it was closed.
    pub(super) closed: bool,
}

/// How the fake answers a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeAnswer {
    /// It merges.
    Allow,
    /// It refuses with this status.
    Refuse(u16),
}

/// What the fake keeps.
#[derive(Debug)]
struct World {
    /// The bare repository that stands for the remote.
    origin: PathBuf,
    /// The pull requests, by number from 1.
    pulls: Vec<Pull>,
    /// The reviews posted, with the number of the pull request.
    reviews: Vec<(u32, Value)>,
    /// The comments posted, with the number of the pull request.
    comments: Vec<(u32, String)>,
    /// Every request, in order.
    seen: Vec<Seen>,
    /// How a merge is answered.
    merge: MergeAnswer,
}

/// A running fake GitHub; it stops when it is dropped.
#[derive(Debug)]
pub(super) struct FakeGitHub {
    /// Where it listens.
    addr: SocketAddr,
    /// What it keeps.
    world: Arc<Mutex<World>>,
    /// The accept loop.
    accept_loop: JoinHandle<()>,
}

impl FakeGitHub {
    /// Starts a fake of the repository whose remote is the bare repository `origin`.
    pub(super) async fn start(origin: &Path) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let world = Arc::new(Mutex::new(World {
            origin: origin.to_path_buf(),
            pulls: Vec::new(),
            reviews: Vec::new(),
            comments: Vec::new(),
            seen: Vec::new(),
            merge: MergeAnswer::Allow,
        }));
        let accept_loop = tokio::spawn(accept_loop(listener, Arc::clone(&world)));
        Ok(Self {
            addr,
            world,
            accept_loop,
        })
    }

    /// The base URL of the API.
    pub(super) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every request seen so far.
    pub(super) fn requests(&self) -> Vec<Seen> {
        self.world().seen.clone()
    }

    /// The requests with `method` whose path (without its query) is `path`.
    pub(super) fn calls(&self, method: &str, path: &str) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|seen| seen.method == method && seen.target.split('?').next() == Some(path))
            .collect()
    }

    /// The pull requests.
    pub(super) fn pulls(&self) -> Vec<Pull> {
        self.world().pulls.clone()
    }

    /// The reviews posted on pull request `number`, as GitHub got them.
    pub(super) fn reviews(&self, number: u32) -> Vec<Value> {
        self.world()
            .reviews
            .iter()
            .filter(|(on, _)| *on == number)
            .map(|(_, review)| review.clone())
            .collect()
    }

    /// The comments posted on pull request `number`.
    pub(super) fn comments(&self, number: u32) -> Vec<String> {
        self.world()
            .comments
            .iter()
            .filter(|(on, _)| *on == number)
            .map(|(_, text)| text.clone())
            .collect()
    }

    /// Makes a merge be answered as `answer` from now on.
    pub(super) fn answer_merges(&self, answer: MergeAnswer) {
        self.world().merge = answer;
    }

    /// The human closes pull request `number` on GitHub without merging it.
    pub(super) fn closed_by_a_human(&self, number: u32) {
        if let Some(pull) = self.world().pulls.iter_mut().find(|pull| pull.number == number) {
            pull.closed = true;
        }
    }

    /// The human merges pull request `number` on GitHub.
    pub(super) fn merged_by_a_human(&self, number: u32) {
        let mut world = self.world();
        if let Some(sha) = merge_into_base(&mut world, number) {
            tracing::debug!(%sha, number, "a human merged a pull request");
        }
    }
}

impl Drop for FakeGitHub {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

/// Runs git in the bare repository `origin`; `None` when it fails.
fn git(origin: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(origin)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The commit the branch `branch` has in the remote.
fn tip(world: &World, branch: &str) -> String {
    git(&world.origin, &["rev-parse", &format!("refs/heads/{branch}")]).unwrap_or_default()
}

/// Merges pull request `number` into its base the way a fast-forward does; the new tip of the base, or `None` when the
/// base is not part of the head.
fn merge_into_base(world: &mut World, number: u32) -> Option<String> {
    let (head, base) = {
        let pull = world.pulls.iter().find(|pull| pull.number == number)?;
        (pull.head.clone(), pull.base.clone())
    };
    let head_sha = tip(world, &head);
    git(
        &world.origin,
        &["merge-base", "--is-ancestor", &format!("refs/heads/{base}"), &head_sha],
    )?;
    git(&world.origin, &["update-ref", &format!("refs/heads/{base}"), &head_sha])?;
    if let Some(pull) = world.pulls.iter_mut().find(|pull| pull.number == number) {
        pull.merged = true;
        pull.closed = true;
    }
    Some(head_sha)
}

/// A pull request as GitHub writes it.
fn pull_json(world: &World, pull: &Pull) -> Value {
    let head_sha = tip(world, &pull.head);
    let base_sha = tip(world, &pull.base);
    json!({
        "number": pull.number,
        "html_url": format!("https://github.example.com/{SLUG}/pull/{}", pull.number),
        "node_id": format!("PR_node_{}", pull.number),
        "title": pull.title,
        "body": pull.body,
        "state": if pull.closed { "closed" } else { "open" },
        "draft": pull.draft,
        "merged": pull.merged,
        "mergeable": true,
        "user": {"login": ACCOUNT},
        "head": {"ref": pull.head, "sha": head_sha, "repo": {"full_name": SLUG}},
        "base": {"ref": pull.base, "sha": base_sha, "repo": {"full_name": SLUG}}
    })
}

/// What the fake answers: a status and a JSON body.
struct Answer {
    /// The status.
    status: u16,
    /// The body.
    body: Value,
}

fn ok(body: Value) -> Answer {
    Answer { status: 200, body }
}

fn not_found() -> Answer {
    Answer {
        status: 404,
        body: json!({"message": "Not Found"}),
    }
}

/// The number in the path `.../pulls/<n>` or `.../issues/<n>`, and what follows it.
fn numbered<'a>(path: &'a str, kind: &str) -> Option<(u32, &'a str)> {
    let rest = path.strip_prefix(&format!("/repos/{SLUG}/{kind}/"))?;
    let (number, tail) = rest.split_once('/').unwrap_or((rest, ""));
    Some((number.parse().ok()?, tail))
}

/// The branch a listing asks for: `head=<owner>:<branch>`, percent-decoded.
fn head_filter(query: &str) -> Option<String> {
    let value = query.split('&').find_map(|pair| pair.strip_prefix("head="))?;
    let (_, branch) = value.split_once(':')?;
    let mut decoded = Vec::new();
    let mut bytes = branch.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex: Vec<u8> = bytes.by_ref().take(2).collect();
            decoded.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).ok()
}

/// Answers `request` as GitHub would.
fn route(world: &mut World, method: &str, target: &str, body: &str) -> Answer {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let pulls_path = format!("/repos/{SLUG}/pulls");
    match (method, path) {
        ("GET", path) if path == pulls_path => {
            let wanted = head_filter(query);
            let listed: Vec<Value> = world
                .pulls
                .iter()
                .filter(|pull| !pull.closed && wanted.as_ref().is_none_or(|head| *head == pull.head))
                .map(|pull| pull_json(world, pull))
                .collect();
            ok(Value::Array(listed))
        }
        ("POST", path) if path == pulls_path => {
            let number = u32::try_from(world.pulls.len().saturating_add(1)).unwrap_or(u32::MAX);
            let text = |key: &str| body[key].as_str().unwrap_or_default().to_owned();
            world.pulls.push(Pull {
                number,
                title: text("title"),
                body: text("body"),
                head: text("head"),
                base: text("base"),
                draft: body["draft"].as_bool().unwrap_or(false),
                merged: false,
                closed: false,
            });
            let pull = world.pulls.last().cloned();
            Answer {
                status: 201,
                body: pull.map_or(Value::Null, |pull| pull_json(world, &pull)),
            }
        }
        ("POST", "/graphql") => {
            let id = body["variables"]["id"].as_str().unwrap_or_default();
            if let Some(number) = id.strip_prefix("PR_node_").and_then(|n| n.parse::<u32>().ok())
                && let Some(pull) = world.pulls.iter_mut().find(|pull| pull.number == number)
            {
                pull.draft = false;
            }
            ok(json!({"data": {"markPullRequestReadyForReview": {"pullRequest": {"isDraft": false}}}}))
        }
        (method, path) => {
            if let Some((number, tail)) = numbered(path, "pulls") {
                return pull_route(world, method, number, tail, &body);
            }
            if let Some((number, "comments")) = numbered(path, "issues")
                && method == "POST"
            {
                world
                    .comments
                    .push((number, body["body"].as_str().unwrap_or_default().to_owned()));
                return Answer {
                    status: 201,
                    body: json!({"id": 1, "user": {"login": ACCOUNT}, "body": body["body"], "created_at": "2026-11-13T10:00:00Z"}),
                };
            }
            not_found()
        }
    }
}

/// The routes under `/repos/<slug>/pulls/<number>`.
fn pull_route(world: &mut World, method: &str, number: u32, tail: &str, body: &Value) -> Answer {
    let Some(pull) = world.pulls.iter().find(|pull| pull.number == number).cloned() else {
        return not_found();
    };
    match (method, tail) {
        ("GET", "") => ok(pull_json(world, &pull)),
        ("PATCH", "") => {
            if let Some(found) = world.pulls.iter_mut().find(|found| found.number == number) {
                if let Some(title) = body["title"].as_str() {
                    title.clone_into(&mut found.title);
                }
                if let Some(text) = body["body"].as_str() {
                    text.clone_into(&mut found.body);
                }
            }
            let updated = world.pulls.iter().find(|found| found.number == number).cloned();
            ok(updated.map_or(Value::Null, |updated| pull_json(world, &updated)))
        }
        ("POST", "reviews") => {
            let id = u64::try_from(world.reviews.len().saturating_add(1)).unwrap_or(u64::MAX);
            let review = json!({
                "id": id,
                "user": {"login": ACCOUNT},
                "body": body["body"],
                "state": "COMMENTED",
                "commit_id": body["commit_id"],
                "submitted_at": "2026-11-13T10:00:00Z",
                "event": body["event"],
            });
            world.reviews.push((number, review.clone()));
            Answer {
                status: 200,
                body: review,
            }
        }
        ("PUT", "merge") => match world.merge {
            MergeAnswer::Refuse(status) => Answer {
                status,
                body: json!({"message": "Pull Request is not mergeable"}),
            },
            MergeAnswer::Allow => match merge_into_base(world, number) {
                Some(sha) => ok(json!({"sha": sha, "merged": true, "message": "merged"})),
                None => Answer {
                    status: 405,
                    body: json!({"message": "Base branch was modified"}),
                },
            },
        },
        _ => not_found(),
    }
}

async fn accept_loop(listener: TcpListener, world: Arc<Mutex<World>>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        connections.spawn(serve_connection(stream, Arc::clone(&world)));
    }
}

async fn serve_connection(mut stream: TcpStream, world: Arc<Mutex<World>>) {
    let Some((method, target, body)) = read_request(&mut stream).await else {
        return;
    };
    let answer = {
        let mut world = world.lock().unwrap_or_else(PoisonError::into_inner);
        world.seen.push(Seen {
            method: method.clone(),
            target: target.clone(),
            body: body.clone(),
        });
        route(&mut world, &method, &target, &body)
    };
    let text = answer.body.to_string();
    let wire = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        answer.status,
        text.len()
    );
    if stream.write_all(wire.as_bytes()).await.is_ok() {
        // A failed shutdown means the client has gone; there is nobody to tell.
        drop(stream.shutdown().await);
    }
}

/// Reads one request: its method, target and body; `None` when the client closes before it is complete.
async fn read_request(stream: &mut TcpStream) -> Option<(String, String, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
    };
    let head = String::from_utf8_lossy(buffer.get(..head_end)?).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    let body_start = head_end + 4;
    while buffer.len() < body_start + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
    }
    let body = String::from_utf8_lossy(buffer.get(body_start..body_start + length)?).into_owned();
    Some((method, target, body))
}
