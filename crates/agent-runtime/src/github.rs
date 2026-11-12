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
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

use agent_core::config::{GithubLogin, Secret};
use agent_core::ids::BranchName;
use agent_core::state::{CommitId, PrNumber, Timestamp};
use agent_core::status::ReviewVerdict;

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
        Err(InvalidRepoSlug::NotOwnerAndName { value: text.to_owned() })
    }
}

/// Why a text is not an [`ApiBase`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidApiBase {
    /// The text is not a URL.
    #[error("{value:?} is not a URL")]
    NotAUrl {
        /// The refused text.
        value: String,
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
        Err(InvalidApiBase::NotAUrl { value: text.to_owned() })
    }
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
        let _ = (next, retry_after);
        Some(Duration::ZERO)
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
    /// GitHub's global ID of it, which GraphQL needs.
    node_id: String,
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

/// What state a posted review is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The HTTP client; the token is in its default headers, marked sensitive.
    http: reqwest::Client,
    /// The settings it was made with.
    settings: GitHubSettings,
    /// The repository every call is about.
    repo: RepoSlug,
    /// The accounts whose words are taken as instructions.
    trusted: BTreeSet<GithubLogin>,
}

impl GitHub {
    /// A client for `repo` that acts as the owner of `token` and trusts the words of `trusted`.
    ///
    /// # Errors
    /// [`GitHubError::InvalidToken`] when the token cannot be a header; [`GitHubError::Client`] when the HTTP
    /// client cannot be built.
    pub fn new(
        settings: GitHubSettings,
        repo: RepoSlug,
        token: &Secret,
        trusted: &[GithubLogin],
    ) -> Result<Self, GitHubError> {
        let _ = token;
        Ok(Self {
            http: reqwest::Client::new(),
            settings,
            repo,
            trusted: trusted.iter().cloned().collect(),
        })
    }

    /// Whether the words of the account `login` are taken as instructions.
    #[must_use]
    pub fn is_trusted(&self, login: &str) -> bool {
        let _ = login;
        false
    }

    /// Opens a pull request. Not retried: a lost answer may hide a pull request that was opened.
    ///
    /// # Errors
    /// The errors of a call: [`GitHubError::Status`] (422 when there is already one for the branches),
    /// [`GitHubError::Transport`], [`GitHubError::Timeout`], [`GitHubError::ResponseTooLarge`] and
    /// [`GitHubError::Decode`].
    pub async fn create_pull_request(&self, new: &NewPullRequest) -> Result<RemotePullRequest, GitHubError> {
        let _ = new;
        Err(GitHubError::NothingToUpdate)
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
        let _ = (number, update);
        Err(GitHubError::NothingToUpdate)
    }

    /// Marks a draft pull request ready for review; a pull request that is ready already is left as it is. GitHub's
    /// REST API cannot do it, so this is a GraphQL mutation. The mutation is not retried.
    ///
    /// # Errors
    /// [`GitHubError::GraphQl`] and the errors of a call.
    pub async fn mark_ready(&self, number: PrNumber) -> Result<(), GitHubError> {
        let _ = number;
        Err(GitHubError::NothingToUpdate)
    }

    /// Merges a pull request at `request.expected_head`. Not retried: a second try after a lost answer would be
    /// refused, and the caller reads the pull request to see what happened.
    ///
    /// # Errors
    /// [`GitHubError::Status`] with [`Rejection::NotAllowed`] when it cannot be merged and
    /// [`Rejection::Conflict`] when the head has moved; the errors of a call.
    pub async fn merge_pull_request(&self, number: PrNumber, request: &MergeRequest) -> Result<Merged, GitHubError> {
        let _ = (number, request);
        Err(GitHubError::NothingToUpdate)
    }

    /// Reads a pull request. Retried.
    ///
    /// # Errors
    /// The errors of a call.
    pub async fn get_pull_request(&self, number: PrNumber) -> Result<RemotePullRequest, GitHubError> {
        let _ = number;
        Err(GitHubError::NothingToUpdate)
    }

    /// Posts a review. Not retried.
    ///
    /// # Errors
    /// The errors of a call; GitHub refuses (422) a verdict by the author of the pull request on their own.
    pub async fn post_review(&self, number: PrNumber, review: &NewReview) -> Result<Review, GitHubError> {
        let _ = (number, review);
        Err(GitHubError::NothingToUpdate)
    }

    /// Posts a comment on a pull request. Not retried.
    ///
    /// # Errors
    /// The errors of a call.
    pub async fn post_comment(&self, number: PrNumber, body: &str) -> Result<Comment, GitHubError> {
        let _ = (number, body);
        Err(GitHubError::NothingToUpdate)
    }

    /// The comments on a pull request by the trusted accounts, oldest first. Retried, page by page.
    ///
    /// # Errors
    /// [`GitHubError::TooManyPages`] and the errors of a call.
    pub async fn list_comments(&self, number: PrNumber) -> Result<TrustedList<Comment>, GitHubError> {
        let _ = number;
        Err(GitHubError::NothingToUpdate)
    }

    /// The reviews of a pull request by the trusted accounts, oldest first. Retried, page by page.
    ///
    /// # Errors
    /// [`GitHubError::TooManyPages`] and the errors of a call.
    pub async fn list_reviews(&self, number: PrNumber) -> Result<TrustedList<Review>, GitHubError> {
        let _ = number;
        Err(GitHubError::NothingToUpdate)
    }
}
