use super::Outlook;
use crate::github::{GitHubError, GitHubOperation, Rejection};
use std::time::Duration;

fn status(status: u16, reason: Rejection, retry_after: Option<Duration>) -> GitHubError {
    GitHubError::Status {
        operation: GitHubOperation::GetPullRequest,
        status,
        reason,
        retry_after,
        message: "Not Found".to_owned(),
    }
}

#[test]
fn the_network_a_timeout_and_github_failing_may_pass() {
    let cases = [
        GitHubError::Timeout {
            operation: GitHubOperation::GetPullRequest,
            attempts: 3,
            limit: Duration::from_secs(30),
        },
        status(500, Rejection::Server, None),
        status(503, Rejection::Server, None),
        status(429, Rejection::RateLimited, Some(Duration::from_secs(60))),
        status(429, Rejection::RateLimited, None),
        status(403, Rejection::RateLimited, Some(Duration::from_secs(60))),
    ];
    for error in cases {
        assert_eq!(error.outlook(), Outlook::WillNotPass, "{error:?}");
    }
}

#[test]
fn a_refusal_that_will_be_given_again_will_not_pass() {
    let cases = [
        status(401, Rejection::Unauthorized, None),
        status(403, Rejection::Forbidden, None),
        status(404, Rejection::NotFound, None),
        status(405, Rejection::NotAllowed, None),
        status(409, Rejection::Conflict, None),
        status(422, Rejection::Invalid, None),
        status(301, Rejection::Other, None),
        GitHubError::GraphQl {
            operation: GitHubOperation::MarkReady,
            message: "not found".to_owned(),
        },
        GitHubError::StillDraft {
            operation: GitHubOperation::MarkReady,
        },
        GitHubError::TooManyPages {
            operation: GitHubOperation::ListComments,
            pages: 10,
        },
        GitHubError::ResponseTooLarge {
            operation: GitHubOperation::ListComments,
            limit: 1024,
        },
        GitHubError::NothingToUpdate,
    ];
    for error in cases {
        assert_eq!(error.outlook(), Outlook::WillNotPass, "{error:?}");
    }
}
