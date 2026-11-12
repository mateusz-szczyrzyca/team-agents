//! The GitHub REST API: pull requests, reviews and comments, with the token of `secrets.toml`.
//!
//! One [`GitHub`] is one repository, one token and one base URL — injected, so that the tests reach a local fake
//! server and never `api.github.com`. Every call has a timeout; a call that is idempotent (a read, an update that
//! sets values) is retried a few times with a growing, jittered delay when the network or GitHub fails it, and one
//! that is not (creating a pull request, a review or a comment, merging) is made once: what it did on a lost
//! answer is for the caller to find out by reading.
//!
//! Text on GitHub by anyone but the trusted accounts is data, never instructions: the lists of comments and
//! reviews keep only what the trusted accounts wrote and count the rest.

use std::collections::BTreeSet;
use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

use agent_core::config::{GithubLogin, Secret};
use agent_core::ids::BranchName;
use agent_core::kinds::{Keyword, UnknownKeyword};
use agent_core::state::{CommitId, PrNumber, Timestamp};
use agent_core::status::ReviewVerdict;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{Method, Url};
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::{Value, json};

/// The base URL of the public GitHub API.
const PUBLIC_API: &str = "https://api.github.com";

/// Longest part of a repository's `owner/name`, in bytes: GitHub's own limit for a name.
const MAX_SLUG_PART_LEN: usize = 100;

/// How many attempts an idempotent call gets, the first one included.
const DEFAULT_ATTEMPTS: u32 = 3;

/// The delay before the first retry.
const DEFAULT_BASE_DELAY: Duration = Duration::from_millis(500);

/// The longest delay before a retry, and the longest `Retry-After` that is waited for.
const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(8);

/// How long one attempt may take, from sending the request to the last byte of the answer.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long connecting may take.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// What GitHub is told this client is.
const USER_AGENT: &str = "team-agents";

/// The version of the REST API the client is written for.
const API_VERSION: &str = "2022-11-28";

/// The most bytes of an answer that are read; GitHub's pages of 100 items are far below it.
const MAX_RESPONSE_BYTES: usize = 8 << 20;

/// How many items a list asks for in one page: GitHub's maximum.
const PAGE_SIZE: usize = 100;

/// How many pages a list reads before it gives up.
const MAX_PAGES: u32 = 20;

/// How many characters of GitHub's own message an error keeps.
const MAX_MESSAGE_CHARS: usize = 512;

/// Why a text is not a [`RepoSlug`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidRepoSlug {
    /// The text is not `owner/name`: it needs exactly one `/` with a part on each side.
    #[error("{value:?} is not a repository in the form owner/name")]
    NotOwnerAndName {
        /// The refused text.
        value: String,
    },
    /// A part is empty after all, too long, `.` or `..`, or has a character GitHub does not allow.
    #[error("{value:?} is not a valid {part} of a repository: 1 to 100 letters, digits, `-`, `_` or `.`")]
    BadPart {
        /// The refused part.
        value: String,
        /// Which part it was: `owner` or `name`.
        part: &'static str,
    },
}

/// A repository on GitHub, `owner/name`, such as `mateusz-szczyrzyca/team-agents`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoSlug {
    /// The account or organisation that owns it.
    owner: String,
    /// The repository's name.
    name: String,
}

impl RepoSlug {
    /// The owner.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The name of the repository.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl FromStr for RepoSlug {
    type Err = InvalidRepoSlug;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let not_owner_and_name = || InvalidRepoSlug::NotOwnerAndName { value: text.to_owned() };
        let (owner, name) = text.split_once('/').ok_or_else(not_owner_and_name)?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(not_owner_and_name());
        }
        check_slug_part(owner, "owner")?;
        check_slug_part(name, "name")?;
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// Checks one part of a repository's `owner/name`.
fn check_slug_part(part: &str, which: &'static str) -> Result<(), InvalidRepoSlug> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if part.len() > MAX_SLUG_PART_LEN || matches!(part, "." | "..") || !part.chars().all(allowed) {
        return Err(InvalidRepoSlug::BadPart {
            value: part.to_owned(),
            part: which,
        });
    }
    Ok(())
}

/// Why a text is not an [`ApiBase`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidApiBase {
    /// The text is not a URL.
    #[error("{value:?} is not a URL")]
    NotAUrl {
        /// The refused text.
        value: String,
        /// What the URL parser found wrong.
        source: <Url as FromStr>::Err,
    },
    /// The scheme is neither `https` nor `http`.
    #[error("the API base {value:?} must start with https:// (or http:// for a loopback address)")]
    Scheme {
        /// The refused text.
        value: String,
    },
    /// It is `http://` to a host that is not this machine: the token would travel in the clear.
    #[error(
        "the API base {value:?} is http:// to a host that is not a loopback address: the token would not be protected"
    )]
    InsecureHost {
        /// The refused text.
        value: String,
    },
    /// The URL has a user name, a password, a query or a fragment, none of which a base may have.
    #[error("the API base {value:?} must not have credentials, a query or a fragment")]
    Decorated {
        /// The refused text.
        value: String,
    },
}

/// The URL every path of the API is under: `https://api.github.com`, or the address of a fake server.
///
/// `https` to any host, or `http` to a loopback address only, so that the token never travels unprotected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiBase(String);

impl ApiBase {
    /// The public GitHub API.
    #[must_use]
    pub fn public() -> Self {
        Self(PUBLIC_API.to_owned())
    }

    /// The text of the URL, without a trailing `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ApiBase {
    type Err = InvalidApiBase;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(text).map_err(|source| InvalidApiBase::NotAUrl {
            value: text.to_owned(),
            source,
        })?;
        let value = || text.to_owned();
        match url.scheme() {
            "https" => {}
            "http" if is_loopback(&url) => {}
            "http" => return Err(InvalidApiBase::InsecureHost { value: value() }),
            _ => return Err(InvalidApiBase::Scheme { value: value() }),
        }
        let decorated =
            !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some();
        if decorated {
            return Err(InvalidApiBase::Decorated { value: value() });
        }
        Ok(Self(url.as_str().trim_end_matches('/').to_owned()))
    }
}

/// Whether the host of `url` is this machine: `localhost` or a loopback address.
fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    // An IPv6 host is written in brackets.
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Whether a delay is spread out, so that clients that failed together do not retry together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jitter {
    /// The delay is exactly the growing one; for tests.
    None,
    /// The delay is between half and all of the growing one, by the sub-second part of the clock: spread, not secret.
    Spread,
}

/// How an idempotent call is retried: how many attempts, and the growing delay between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The attempts, the first one included; 1 means no retry.
    pub max_attempts: NonZeroU32,
    /// The delay before the first retry; each further retry doubles it.
    pub base_delay: Duration,
    /// The longest delay, and the longest `Retry-After` that is waited for.
    pub max_delay: Duration,
    /// Whether the delay is spread.
    pub jitter: Jitter,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: NonZeroU32::MIN.saturating_add(DEFAULT_ATTEMPTS - 1),
            base_delay: DEFAULT_BASE_DELAY,
            max_delay: DEFAULT_MAX_DELAY,
            jitter: Jitter::Spread,
        }
    }
}

impl RetryPolicy {
    /// How long to wait before attempt `next` (counted from 2) after GitHub or the network failed the one before,
    /// or `None` when there is no attempt left or GitHub asks for a longer wait (`retry_after`) than
    /// [`RetryPolicy::max_delay`].
    #[must_use]
    pub fn delay_before(&self, next: u32, retry_after: Option<Duration>) -> Option<Duration> {
        if next > self.max_attempts.get() {
            return None;
        }
        // The first retry (attempt 2) waits `base_delay`; every further one doubles it, up to `max_delay`.
        let doublings = next.saturating_sub(2).min(u32::BITS - 1);
        let growing = self.base_delay.saturating_mul(1_u32 << doublings).min(self.max_delay);
        let backoff = self.jitter.apply(growing);
        match retry_after {
            None => Some(backoff),
            Some(asked) if asked > self.max_delay => None,
            Some(asked) => Some(asked.max(backoff)),
        }
    }
}

impl Jitter {
    /// `delay`, or a delay between half and all of it.
    fn apply(self, delay: Duration) -> Duration {
        match self {
            Self::None => delay,
            Self::Spread => {
                let [a, b, c, d, ..] = RandomState::new().hash_one(0_u8).to_le_bytes();
                let permille = u32::from_le_bytes([a, b, c, d]) % 1001;
                let half = delay / 2;
                half.saturating_add(half.saturating_mul(permille) / 1000)
            }
        }
    }
}

/// The settings of a [`GitHub`] client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubSettings {
    /// Where the API is.
    pub base: ApiBase,
    /// How long one attempt may take.
    pub timeout: Duration,
    /// How long connecting may take.
    pub connect_timeout: Duration,
    /// How idempotent calls are retried.
    pub retry: RetryPolicy,
}

impl Default for GitHubSettings {
    fn default() -> Self {
        Self {
            base: ApiBase::public(),
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            retry: RetryPolicy::default(),
        }
    }
}

/// What a call of the API was for, in an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum GitHubOperation {
    /// Opening a pull request.
    #[strum(serialize = "create pull request")]
    CreatePullRequest,
    /// Changing the title, body, base or state of a pull request.
    #[strum(serialize = "update pull request")]
    UpdatePullRequest,
    /// Marking a draft pull request ready for review.
    #[strum(serialize = "mark pull request ready")]
    MarkReady,
    /// Merging a pull request.
    #[strum(serialize = "merge pull request")]
    MergePullRequest,
    /// Reading a pull request.
    #[strum(serialize = "get pull request")]
    GetPullRequest,
    /// Posting a review.
    #[strum(serialize = "post review")]
    PostReview,
    /// Posting a comment.
    #[strum(serialize = "post comment")]
    PostComment,
    /// Listing the comments of a pull request.
    #[strum(serialize = "list comments")]
    ListComments,
    /// Listing the reviews of a pull request.
    #[strum(serialize = "list reviews")]
    ListReviews,
}

/// Why GitHub refused a call, by its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// 401: the token is missing, wrong or expired.
    Unauthorized,
    /// 403: the token may not do this.
    Forbidden,
    /// 404: there is no such repository or pull request, or the token cannot see it.
    NotFound,
    /// 405: the pull request cannot be merged as it is.
    NotAllowed,
    /// 409: the head of the pull request is not the one the call named.
    Conflict,
    /// 422: GitHub refused the values, for instance a second pull request for the same branches.
    Invalid,
    /// 429, or 403 with `Retry-After`: the rate limit.
    RateLimited,
    /// 5xx: GitHub failed.
    Server,
    /// Any other status that is not a success.
    Other,
}

/// Why a call of the GitHub API failed.
#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    /// The HTTP client cannot be built.
    #[error("cannot build the HTTP client for the GitHub API")]
    Client {
        /// What the client said.
        source: reqwest::Error,
    },
    /// The token has a character an HTTP header cannot carry. The token is not repeated here.
    #[error("the GitHub token cannot be sent in an HTTP header")]
    InvalidToken,
    /// The request could not be encoded.
    #[error("cannot encode the request of {operation}")]
    Encode {
        /// The call.
        operation: GitHubOperation,
        /// What the encoder said.
        source: serde_json::Error,
    },
    /// The network failed the call, or the connection broke.
    #[error("{operation} failed after {attempts} attempt(s)")]
    Transport {
        /// The call.
        operation: GitHubOperation,
        /// How many attempts were made.
        attempts: u32,
        /// What the client said.
        source: reqwest::Error,
    },
    /// The call ran over its time.
    #[error("{operation} timed out after {attempts} attempt(s) of {limit:?}")]
    Timeout {
        /// The call.
        operation: GitHubOperation,
        /// How many attempts were made.
        attempts: u32,
        /// The time of one attempt.
        limit: Duration,
    },
    /// GitHub answered with a status that is not a success.
    #[error("{operation} was refused with status {status}: {message}")]
    Status {
        /// The call.
        operation: GitHubOperation,
        /// The status code.
        status: u16,
        /// What the status means.
        reason: Rejection,
        /// How long GitHub asks to wait, when it says.
        retry_after: Option<Duration>,
        /// GitHub's own message, cut to a bounded length; empty when it gave none.
        message: String,
    },
    /// The answer is longer than the limit.
    #[error("the answer to {operation} is longer than {limit} bytes")]
    ResponseTooLarge {
        /// The call.
        operation: GitHubOperation,
        /// The limit, in bytes.
        limit: usize,
    },
    /// The answer is not the JSON that was expected.
    #[error("cannot read the answer to {operation}")]
    Decode {
        /// The call.
        operation: GitHubOperation,
        /// What the decoder said.
        source: serde_json::Error,
    },
    /// GraphQL answered with errors.
    #[error("{operation} was refused by GraphQL: {message}")]
    GraphQl {
        /// The call.
        operation: GitHubOperation,
        /// The first message GraphQL gave, cut to a bounded length.
        message: String,
    },
    /// A list has more pages than are read.
    #[error("{operation} has more than {pages} pages")]
    TooManyPages {
        /// The call.
        operation: GitHubOperation,
        /// The pages that were read.
        pages: u32,
    },
    /// An update that changes nothing.
    #[error("an update of a pull request needs at least one change")]
    NothingToUpdate,
}

/// Whether a pull request is a draft or ready for review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Not ready: it cannot be merged.
    Draft,
    /// Ready for review.
    Ready,
}

/// Where a pull request's branch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// In the repository itself.
    SameRepository,
    /// In a fork, or in a fork that was deleted: its author is not on the team.
    Fork,
}

/// The state of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullState {
    /// Open.
    Open,
    /// Closed without a merge.
    Closed,
    /// Merged, and so closed.
    Merged,
}

/// Whether GitHub finds a pull request mergeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    /// It merges without conflicts.
    Yes,
    /// It conflicts with its base.
    No,
    /// GitHub has not worked it out yet; ask again.
    Unknown,
}

/// A pull request, as GitHub reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePullRequest {
    /// Its number.
    pub number: PrNumber,
    /// Its address in the browser.
    pub url: String,
    /// Its title.
    pub title: String,
    /// Its description; empty when it has none.
    pub body: String,
    /// Open, closed or merged.
    pub state: PullState,
    /// Draft or ready.
    pub readiness: Readiness,
    /// Whether it can be merged.
    pub mergeable: Mergeable,
    /// The login of its author.
    pub author: String,
    /// The branch it merges: its head.
    pub head_branch: BranchName,
    /// The commit at its head.
    pub head: CommitId,
    /// The branch it merges into.
    pub base_branch: BranchName,
    /// Whether its head is in this repository or in a fork.
    pub origin: Origin,
}

/// A pull request to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    /// The title.
    pub title: String,
    /// The description.
    pub body: String,
    /// The branch with the changes.
    pub head: BranchName,
    /// The branch they are merged into.
    pub base: BranchName,
    /// Whether it is opened as a draft.
    pub readiness: Readiness,
}

/// What an update does to the state of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    /// Open it again.
    Reopen,
    /// Close it without merging.
    Close,
}

/// Changes to a pull request; what is `None` stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestUpdate {
    /// A new title.
    pub title: Option<String>,
    /// A new description.
    pub body: Option<String>,
    /// A new base branch.
    pub base: Option<BranchName>,
    /// Opening or closing it.
    pub state: Option<StateChange>,
}

/// How a pull request is merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    /// A merge commit.
    Merge,
    /// One squashed commit.
    Squash,
    /// Rebased commits.
    Rebase,
}

/// A merge to make: of the pull request as it is at `expected_head`, and no other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    /// How to merge.
    pub method: MergeMethod,
    /// The head the reviews and the gate saw. When the head has moved, GitHub refuses ([`Rejection::Conflict`]).
    pub expected_head: CommitId,
    /// The title of the merge commit; GitHub's default when `None`.
    pub commit_title: Option<String>,
    /// The message of the merge commit; GitHub's default when `None`.
    pub commit_message: Option<String>,
}

/// A merge that was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The commit the merge made on the base branch.
    pub commit: CommitId,
}

/// A review to post: a verdict on the pull request as it is at `commit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReview {
    /// Approve, or request changes.
    pub verdict: ReviewVerdict,
    /// What the review says.
    pub body: String,
    /// The head the review is about.
    pub commit: CommitId,
}

/// What state a posted review is in; GitHub writes it in capitals, such as `CHANGES_REQUESTED`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<ReviewState>)]
pub enum ReviewState {
    /// Approved.
    Approved,
    /// Changes requested.
    ChangesRequested,
    /// Only comments.
    Commented,
    /// Dismissed.
    Dismissed,
    /// Not submitted yet.
    Pending,
}

impl Keyword for ReviewState {
    const WHAT: &'static str = "review state";
}

impl TryFrom<String> for ReviewState {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// A review of a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// Its ID.
    pub id: u64,
    /// The login of its author.
    pub author: String,
    /// What it says; empty when it has only a verdict.
    pub body: String,
    /// Its state.
    pub state: ReviewState,
    /// The head it is about; none when GitHub does not say.
    pub commit: Option<CommitId>,
    /// When it was submitted; none while it is pending.
    pub submitted_at: Option<Timestamp>,
}

/// A comment on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Its ID.
    pub id: u64,
    /// The login of its author.
    pub author: String,
    /// What it says.
    pub body: String,
    /// When it was written.
    pub created_at: Timestamp,
}

/// The items a list holds from the trusted accounts, and how many it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedList<T> {
    /// The items by the trusted accounts, oldest first.
    pub trusted: Vec<T>,
    /// How many items by anyone else were left out: they are data, never instructions, and are not shown.
    pub ignored: usize,
}

/// The GitHub REST API for one repository.
#[derive(Debug, Clone)]
pub struct GitHub {
    /// The HTTP client, with the headers every call carries.
    http: reqwest::Client,
    /// The settings it was made with.
    settings: GitHubSettings,
    /// The repository every call is about.
    repo: RepoSlug,
    /// The token of the account the calls are made as; its `Debug` shows nothing.
    token: Secret,
    /// The accounts whose words are taken as instructions.
    trusted: BTreeSet<GithubLogin>,
}

/// Whether a call may be made again when its answer is lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Idempotency {
    /// Making it twice does what making it once does: it is retried.
    Idempotent,
    /// It may do something a second time, or answer differently the second time: it is made once.
    Once,
}

/// A call to make.
struct Call {
    /// What it is for.
    operation: GitHubOperation,
    /// The HTTP method.
    method: Method,
    /// The path under the base URL, with its query.
    path: String,
    /// The JSON body, if it has one.
    body: Option<String>,
    /// Whether it is retried.
    idempotency: Idempotency,
}

/// Why one attempt of a call failed.
enum Failure {
    /// The connection or the answer broke.
    Transport(reqwest::Error),
    /// The attempt ran over its time.
    TimedOut,
    /// GitHub answered with a status that is not a success.
    Rejected {
        /// The status code.
        status: u16,
        /// What it means.
        reason: Rejection,
        /// How long GitHub asks to wait.
        retry_after: Option<Duration>,
        /// GitHub's message, bounded.
        message: String,
    },
    /// The answer is over the limit.
    TooLarge,
}

impl Failure {
    /// Whether a later attempt may do better: the network, the server and a rate limit that names its wait.
    fn is_transient(&self) -> bool {
        match self {
            Self::Transport(error) => !error.is_builder(),
            Self::TimedOut => true,
            Self::Rejected {
                reason, retry_after, ..
            } => matches!(reason, Rejection::Server) || (*reason == Rejection::RateLimited && retry_after.is_some()),
            Self::TooLarge => false,
        }
    }

    /// How long GitHub asks to wait before the next attempt.
    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Rejected { retry_after, .. } => *retry_after,
            Self::Transport(_) | Self::TimedOut | Self::TooLarge => None,
        }
    }

    /// The error of a call that failed on its `attempts`-th attempt, each of which had `limit`.
    fn into_error(self, operation: GitHubOperation, attempts: u32, limit: Duration) -> GitHubError {
        match self {
            Self::Transport(source) => GitHubError::Transport {
                operation,
                attempts,
                source,
            },
            Self::TimedOut => GitHubError::Timeout {
                operation,
                attempts,
                limit,
            },
            Self::Rejected {
                status,
                reason,
                retry_after,
                message,
            } => GitHubError::Status {
                operation,
                status,
                reason,
                retry_after,
                message,
            },
            Self::TooLarge => GitHubError::ResponseTooLarge {
                operation,
                limit: MAX_RESPONSE_BYTES,
            },
        }
    }
}

impl GitHub {
    /// A client for `repo` that acts as the owner of `token` and trusts the words of `trusted`.
    ///
    /// # Errors
    /// [`GitHubError::Client`] when the HTTP client cannot be built.
    pub fn new(
        settings: GitHubSettings,
        repo: RepoSlug,
        token: &Secret,
        trusted: &[GithubLogin],
    ) -> Result<Self, GitHubError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert(
            HeaderName::from_static("x-github-api-version"),
            HeaderValue::from_static(API_VERSION),
        );
        // A redirect is an error, not followed: the token must not be sent where the base URL does not point.
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(USER_AGENT)
            .connect_timeout(settings.connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|source| GitHubError::Client { source })?;
        Ok(Self {
            http,
            settings,
            repo,
            token: token.clone(),
            trusted: trusted.iter().cloned().collect(),
        })
    }

    /// Whether the words of the account `login` are taken as instructions.
    #[must_use]
    pub fn is_trusted(&self, login: &str) -> bool {
        // A name that is not a valid login (a bot such as `dependabot[bot]`) is not one of the trusted accounts.
        login
            .parse::<GithubLogin>()
            .is_ok_and(|account| self.trusted.contains(&account))
    }

    /// Opens a pull request. Not retried: a lost answer may hide a pull request that was opened.
    ///
    /// # Errors
    /// The errors of a call: [`GitHubError::Status`] (422 when there is already one for the branches),
    /// [`GitHubError::Transport`], [`GitHubError::Timeout`], [`GitHubError::ResponseTooLarge`] and
    /// [`GitHubError::Decode`].
    pub async fn create_pull_request(&self, new: &NewPullRequest) -> Result<RemotePullRequest, GitHubError> {
        let operation = GitHubOperation::CreatePullRequest;
        let body = json!({
            "title": new.title,
            "body": new.body,
            "head": new.head.as_str(),
            "base": new.base.as_str(),
            "draft": new.readiness == Readiness::Draft,
        });
        let answer = self
            .call(Call {
                operation,
                method: Method::POST,
                path: self.repo_path("pulls"),
                body: Some(body.to_string()),
                idempotency: Idempotency::Once,
            })
            .await?;
        self.pull_from(operation, &answer)
    }

    /// Changes a pull request. Retried: setting the same values twice is the same as once.
    ///
    /// # Errors
    /// [`GitHubError::NothingToUpdate`] for an update with no change, and the errors of a call.
    pub async fn update_pull_request(
        &self,
        number: PrNumber,
        update: &PullRequestUpdate,
    ) -> Result<RemotePullRequest, GitHubError> {
        let operation = GitHubOperation::UpdatePullRequest;
        let mut changes = serde_json::Map::new();
        if let Some(title) = &update.title {
            changes.insert("title".to_owned(), json!(title));
        }
        if let Some(body) = &update.body {
            changes.insert("body".to_owned(), json!(body));
        }
        if let Some(base) = &update.base {
            changes.insert("base".to_owned(), json!(base.as_str()));
        }
        if let Some(state) = update.state {
            let text = match state {
                StateChange::Reopen => "open",
                StateChange::Close => "closed",
            };
            changes.insert("state".to_owned(), json!(text));
        }
        if changes.is_empty() {
            return Err(GitHubError::NothingToUpdate);
        }
        let answer = self
            .call(Call {
                operation,
                method: Method::PATCH,
                path: self.repo_path(&format!("pulls/{number}")),
                body: Some(Value::Object(changes).to_string()),
                idempotency: Idempotency::Idempotent,
            })
            .await?;
        self.pull_from(operation, &answer)
    }

    /// Marks a draft pull request ready for review; a pull request that is ready already is left as it is. GitHub's
    /// REST API cannot do it, so this is a GraphQL mutation. The mutation is not retried.
    ///
    /// # Errors
    /// [`GitHubError::GraphQl`] and the errors of a call.
    pub async fn mark_ready(&self, number: PrNumber) -> Result<(), GitHubError> {
        let operation = GitHubOperation::MarkReady;
        let pull = self.read_pull(operation, number).await?;
        if pull.pull.readiness == Readiness::Ready {
            return Ok(());
        }
        let body = json!({
            "query": MARK_READY_MUTATION,
            "variables": {"id": pull.node_id},
        });
        let answer = self
            .call(Call {
                operation,
                method: Method::POST,
                path: "graphql".to_owned(),
                body: Some(body.to_string()),
                idempotency: Idempotency::Once,
            })
            .await?;
        let reply: GraphQlReply = decode(operation, &answer)?;
        match reply.errors.into_iter().next() {
            Some(first) => Err(GitHubError::GraphQl {
                operation,
                message: first.message.chars().take(MAX_MESSAGE_CHARS).collect(),
            }),
            None => Ok(()),
        }
    }

    /// Merges a pull request at `request.expected_head`. Not retried: a second try after a lost answer would be
    /// refused, and the caller reads the pull request to see what happened.
    ///
    /// # Errors
    /// [`GitHubError::Status`] with [`Rejection::NotAllowed`] when it cannot be merged and
    /// [`Rejection::Conflict`] when the head has moved; the errors of a call.
    pub async fn merge_pull_request(&self, number: PrNumber, request: &MergeRequest) -> Result<Merged, GitHubError> {
        let operation = GitHubOperation::MergePullRequest;
        let method = match request.method {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        };
        let mut body = serde_json::Map::new();
        body.insert("merge_method".to_owned(), json!(method));
        body.insert("sha".to_owned(), json!(request.expected_head.as_str()));
        if let Some(title) = &request.commit_title {
            body.insert("commit_title".to_owned(), json!(title));
        }
        if let Some(message) = &request.commit_message {
            body.insert("commit_message".to_owned(), json!(message));
        }
        let answer = self
            .call(Call {
                operation,
                method: Method::PUT,
                path: self.repo_path(&format!("pulls/{number}/merge")),
                body: Some(Value::Object(body).to_string()),
                idempotency: Idempotency::Once,
            })
            .await?;
        let merged: RawMerged = decode(operation, &answer)?;
        Ok(Merged { commit: merged.sha })
    }

    /// Reads a pull request. Retried.
    ///
    /// # Errors
    /// The errors of a call.
    pub async fn get_pull_request(&self, number: PrNumber) -> Result<RemotePullRequest, GitHubError> {
        let pull = self.read_pull(GitHubOperation::GetPullRequest, number).await?;
        Ok(pull.pull)
    }

    /// Posts a review. Not retried.
    ///
    /// # Errors
    /// The errors of a call; GitHub refuses (422) a verdict by the author of the pull request on their own.
    pub async fn post_review(&self, number: PrNumber, review: &NewReview) -> Result<Review, GitHubError> {
        let operation = GitHubOperation::PostReview;
        let body = json!({
            "event": review.verdict.to_string(),
            "body": review.body,
            "commit_id": review.commit.as_str(),
        });
        let answer = self
            .call(Call {
                operation,
                method: Method::POST,
                path: self.repo_path(&format!("pulls/{number}/reviews")),
                body: Some(body.to_string()),
                idempotency: Idempotency::Once,
            })
            .await?;
        let raw: RawReview = decode(operation, &answer)?;
        let author = raw.author_or_decode_error(operation)?;
        Ok(raw.into_review(author))
    }

    /// Posts a comment on a pull request. Not retried.
    ///
    /// # Errors
    /// The errors of a call.
    pub async fn post_comment(&self, number: PrNumber, body: &str) -> Result<Comment, GitHubError> {
        let operation = GitHubOperation::PostComment;
        let answer = self
            .call(Call {
                operation,
                method: Method::POST,
                path: self.repo_path(&format!("issues/{number}/comments")),
                body: Some(json!({"body": body}).to_string()),
                idempotency: Idempotency::Once,
            })
            .await?;
        let raw: RawComment = decode(operation, &answer)?;
        let author = raw.author_or_decode_error(operation)?;
        Ok(raw.into_comment(author))
    }

    /// The comments on a pull request by the trusted accounts, oldest first. Retried, page by page.
    ///
    /// # Errors
    /// [`GitHubError::TooManyPages`] and the errors of a call.
    pub async fn list_comments(&self, number: PrNumber) -> Result<TrustedList<Comment>, GitHubError> {
        let operation = GitHubOperation::ListComments;
        let raw: Vec<RawComment> = self
            .pages(operation, &self.repo_path(&format!("issues/{number}/comments")))
            .await?;
        Ok(self.keep_trusted(operation, number, raw, RawComment::into_comment))
    }

    /// The reviews of a pull request by the trusted accounts, oldest first. Retried, page by page.
    ///
    /// # Errors
    /// [`GitHubError::TooManyPages`] and the errors of a call.
    pub async fn list_reviews(&self, number: PrNumber) -> Result<TrustedList<Review>, GitHubError> {
        let operation = GitHubOperation::ListReviews;
        let raw: Vec<RawReview> = self
            .pages(operation, &self.repo_path(&format!("pulls/{number}/reviews")))
            .await?;
        Ok(self.keep_trusted(operation, number, raw, RawReview::into_review))
    }

    /// The path of `what` under this repository.
    fn repo_path(&self, what: &str) -> String {
        format!("repos/{}/{}/{what}", self.repo.owner(), self.repo.name())
    }

    /// Reads and decodes a pull request.
    async fn read_pull(&self, operation: GitHubOperation, number: PrNumber) -> Result<ReadPull, GitHubError> {
        let answer = self
            .call(Call {
                operation,
                method: Method::GET,
                path: self.repo_path(&format!("pulls/{number}")),
                body: None,
                idempotency: Idempotency::Idempotent,
            })
            .await?;
        let raw: RawPull = decode(operation, &answer)?;
        Ok(raw.into_read(&self.repo))
    }

    /// Decodes the answer to a call that returns a pull request.
    fn pull_from(&self, operation: GitHubOperation, answer: &[u8]) -> Result<RemotePullRequest, GitHubError> {
        let raw: RawPull = decode(operation, answer)?;
        Ok(raw.into_read(&self.repo).pull)
    }

    /// Reads every page of a list, at most [`MAX_PAGES`].
    async fn pages<R: DeserializeOwned>(&self, operation: GitHubOperation, path: &str) -> Result<Vec<R>, GitHubError> {
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let answer = self
                .call(Call {
                    operation,
                    method: Method::GET,
                    path: format!("{path}?per_page={PAGE_SIZE}&page={page}"),
                    body: None,
                    idempotency: Idempotency::Idempotent,
                })
                .await?;
            let mut on_page: Vec<R> = decode(operation, &answer)?;
            let last = on_page.len() < PAGE_SIZE;
            items.append(&mut on_page);
            if last {
                return Ok(items);
            }
        }
        Err(GitHubError::TooManyPages {
            operation,
            pages: MAX_PAGES,
        })
    }

    /// Keeps the items of the trusted accounts, converted with `convert`, and counts the others.
    fn keep_trusted<R: Authored, T>(
        &self,
        operation: GitHubOperation,
        number: PrNumber,
        raw: Vec<R>,
        convert: fn(R, String) -> T,
    ) -> TrustedList<T> {
        let mut trusted = Vec::new();
        let mut ignored = 0_usize;
        for item in raw {
            match item.author().map(str::to_owned) {
                Some(author) if self.is_trusted(&author) => trusted.push(convert(item, author)),
                Some(_) | None => ignored = ignored.saturating_add(1),
            }
        }
        if ignored > 0 {
            tracing::info!(%operation, pull_request = %number, ignored, "left out what accounts that are not trusted wrote");
        }
        TrustedList { trusted, ignored }
    }

    /// Makes a call, again after a transient failure when it is idempotent, and returns the body of the answer.
    async fn call(&self, call: Call) -> Result<Vec<u8>, GitHubError> {
        let allowed = match call.idempotency {
            Idempotency::Idempotent => self.settings.retry.max_attempts.get(),
            Idempotency::Once => 1,
        };
        let mut attempt = 1_u32;
        loop {
            let failure = match self.attempt(&call).await {
                Ok(body) => return Ok(body),
                Err(failure) => failure,
            };
            let next = attempt.saturating_add(1);
            let delay = if attempt < allowed && failure.is_transient() {
                self.settings.retry.delay_before(next, failure.retry_after())
            } else {
                None
            };
            let Some(delay) = delay else {
                return Err(failure.into_error(call.operation, attempt, self.settings.timeout));
            };
            tracing::debug!(operation = %call.operation, attempt, ?delay, "a call failed and is made again");
            tokio::time::sleep(delay).await;
            attempt = next;
        }
    }

    /// One attempt of a call, within the time of an attempt.
    async fn attempt(&self, call: &Call) -> Result<Vec<u8>, Failure> {
        match tokio::time::timeout(self.settings.timeout, self.exchange(call)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(Failure::TimedOut),
        }
    }

    /// Sends the request and reads the whole answer, bounded.
    async fn exchange(&self, call: &Call) -> Result<Vec<u8>, Failure> {
        let url = format!("{}/{}", self.settings.base.as_str(), call.path);
        let mut request = self
            .http
            .request(call.method.clone(), url)
            .bearer_auth(self.token.expose());
        if let Some(body) = &call.body {
            request = request.header(CONTENT_TYPE, "application/json").body(body.clone());
        }
        let mut response = request.send().await.map_err(Failure::Transport)?;
        let status = response.status();
        let retry_after = retry_after(response.headers());
        let quota_exhausted = response
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|value| value.as_bytes() == b"0");
        if response
            .content_length()
            .is_some_and(|length| length > u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX))
        {
            return Err(Failure::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Failure::Transport)? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(Failure::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if status.is_success() {
            return Ok(body);
        }
        let code = status.as_u16();
        Err(Failure::Rejected {
            status: code,
            reason: rejection(code, retry_after.is_some() || quota_exhausted),
            retry_after,
            message: error_message(&body),
        })
    }
}

/// The GraphQL mutation that marks a pull request ready for review.
const MARK_READY_MUTATION: &str =
    "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { isDraft } } }";

/// What a status that is not a success means. `limited` is whether GitHub said it is a rate limit: a `Retry-After`
/// or an exhausted quota, which is how it tells a rate limit from a plain 403.
fn rejection(status: u16, limited: bool) -> Rejection {
    match status {
        401 => Rejection::Unauthorized,
        403 if limited => Rejection::RateLimited,
        403 => Rejection::Forbidden,
        404 => Rejection::NotFound,
        405 => Rejection::NotAllowed,
        409 => Rejection::Conflict,
        422 => Rejection::Invalid,
        429 => Rejection::RateLimited,
        500..=599 => Rejection::Server,
        _ => Rejection::Other,
    }
}

/// How long `Retry-After` asks to wait. Only the form of a number of seconds is read, which is what GitHub sends; a
/// header in another form gives no wait, and the call fails as it is.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let text = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let Ok(seconds) = text.trim().parse::<u64>() else {
        return None;
    };
    Some(Duration::from_secs(seconds))
}

/// GitHub's own message in the body of an error, bounded. A body that is not GitHub's JSON error has none: the
/// status says what happened.
fn error_message(body: &[u8]) -> String {
    match serde_json::from_slice::<RawError>(body) {
        Ok(RawError { message: Some(message) }) => message.chars().take(MAX_MESSAGE_CHARS).collect(),
        Ok(RawError { message: None }) | Err(_) => String::new(),
    }
}

/// Decodes the JSON body of an answer.
fn decode<T: DeserializeOwned>(operation: GitHubOperation, body: &[u8]) -> Result<T, GitHubError> {
    serde_json::from_slice(body).map_err(|source| GitHubError::Decode { operation, source })
}

/// The body of an error answer.
#[derive(Deserialize)]
struct RawError {
    /// GitHub's message.
    message: Option<String>,
}

/// The answer to a GraphQL call.
#[derive(Deserialize)]
struct GraphQlReply {
    /// The errors it reports; none when it worked.
    #[serde(default)]
    errors: Vec<GraphQlMessage>,
}

/// One error of a GraphQL answer.
#[derive(Deserialize)]
struct GraphQlMessage {
    /// What went wrong.
    message: String,
}

/// The answer to a merge.
#[derive(Deserialize)]
struct RawMerged {
    /// The merge commit.
    sha: CommitId,
}

/// An account in an answer.
#[derive(Deserialize)]
struct RawUser {
    /// Its login.
    login: String,
}

/// An item of a list that someone wrote.
trait Authored {
    /// The login of its author; none when the account is gone.
    fn author(&self) -> Option<&str>;
}

/// A comment as GitHub writes it.
#[derive(Deserialize)]
struct RawComment {
    /// Its ID.
    id: u64,
    /// Its author; none for an account that is gone.
    user: Option<RawUser>,
    /// What it says.
    body: Option<String>,
    /// When it was written.
    created_at: Timestamp,
}

impl Authored for RawComment {
    fn author(&self) -> Option<&str> {
        self.user.as_ref().map(|user| user.login.as_str())
    }
}

impl RawComment {
    /// The comment, by `author`.
    fn into_comment(self, author: String) -> Comment {
        Comment {
            id: self.id,
            author,
            body: self.body.unwrap_or_default(),
            created_at: self.created_at,
        }
    }

    /// The login of the author of a comment this client just posted: it always has one.
    fn author_or_decode_error(&self, operation: GitHubOperation) -> Result<String, GitHubError> {
        author_or_decode_error(self, operation)
    }
}

/// A review as GitHub writes it.
#[derive(Deserialize)]
struct RawReview {
    /// Its ID.
    id: u64,
    /// Its author; none for an account that is gone.
    user: Option<RawUser>,
    /// What it says.
    body: Option<String>,
    /// Its state.
    state: ReviewState,
    /// The head it is about.
    commit_id: Option<CommitId>,
    /// When it was submitted.
    submitted_at: Option<Timestamp>,
}

impl Authored for RawReview {
    fn author(&self) -> Option<&str> {
        self.user.as_ref().map(|user| user.login.as_str())
    }
}

impl RawReview {
    /// The review, by `author`.
    fn into_review(self, author: String) -> Review {
        Review {
            id: self.id,
            author,
            body: self.body.unwrap_or_default(),
            state: self.state,
            commit: self.commit_id,
            submitted_at: self.submitted_at,
        }
    }

    /// The login of the author of a review this client just posted: it always has one.
    fn author_or_decode_error(&self, operation: GitHubOperation) -> Result<String, GitHubError> {
        author_or_decode_error(self, operation)
    }
}

/// The author of an item the answer to a post describes; an answer without one is not what GitHub sends.
fn author_or_decode_error(item: &impl Authored, operation: GitHubOperation) -> Result<String, GitHubError> {
    item.author().map(str::to_owned).ok_or_else(|| GitHubError::Decode {
        operation,
        source: serde_json::Error::custom("the answer has no author"),
    })
}

/// The state of a pull request as GitHub writes it.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawState {
    /// Open.
    Open,
    /// Closed, merged or not.
    Closed,
}

/// A repository in an answer.
#[derive(Deserialize)]
struct RawRepo {
    /// Its `owner/name`.
    full_name: String,
}

/// One side of a pull request.
#[derive(Deserialize)]
struct RawSide {
    /// The branch.
    #[serde(rename = "ref")]
    branch: BranchName,
    /// The commit at its tip.
    sha: CommitId,
    /// Its repository; none when a fork was deleted.
    repo: Option<RawRepo>,
}

/// A pull request as GitHub writes it.
#[derive(Deserialize)]
struct RawPull {
    /// Its number.
    number: PrNumber,
    /// Its address in the browser.
    html_url: String,
    /// GitHub's global ID of it.
    node_id: String,
    /// Its title.
    title: String,
    /// Its description.
    body: Option<String>,
    /// Open or closed.
    state: RawState,
    /// Whether it is a draft.
    #[serde(default)]
    draft: bool,
    /// Whether it was merged.
    #[serde(default)]
    merged: bool,
    /// Whether it can be merged; none while GitHub works it out.
    #[serde(default)]
    mergeable: Option<bool>,
    /// Its author.
    user: RawUser,
    /// The branch it merges.
    head: RawSide,
    /// The branch it merges into.
    base: RawSide,
}

/// A pull request read, with what GraphQL needs of it.
struct ReadPull {
    /// The pull request.
    pull: RemotePullRequest,
    /// GitHub's global ID of it.
    node_id: String,
}

impl RawPull {
    /// The pull request of `repo`.
    fn into_read(self, repo: &RepoSlug) -> ReadPull {
        let state = match (self.state, self.merged) {
            (RawState::Open, _) => PullState::Open,
            (RawState::Closed, true) => PullState::Merged,
            (RawState::Closed, false) => PullState::Closed,
        };
        let same_repository = self
            .head
            .repo
            .as_ref()
            .is_some_and(|head| head.full_name.eq_ignore_ascii_case(&repo.to_string()));
        ReadPull {
            node_id: self.node_id,
            pull: RemotePullRequest {
                number: self.number,
                url: self.html_url,
                title: self.title,
                body: self.body.unwrap_or_default(),
                state,
                readiness: if self.draft { Readiness::Draft } else { Readiness::Ready },
                mergeable: match self.mergeable {
                    Some(true) => Mergeable::Yes,
                    Some(false) => Mergeable::No,
                    None => Mergeable::Unknown,
                },
                author: self.user.login,
                head_branch: self.head.branch,
                head: self.head.sha,
                base_branch: self.base.branch,
                origin: if same_repository {
                    Origin::SameRepository
                } else {
                    Origin::Fork
                },
            },
        }
    }
}
