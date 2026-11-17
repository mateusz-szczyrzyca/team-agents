use std::str::FromStr;

use super::link_commits;
use crate::state::CommitId;

fn commit(id: &str) -> CommitId {
    CommitId::from_str(id).unwrap()
}

#[test]
fn link_commits_table() {
    let short_full = "1234567890abcdef1234567890abcdef12345678";
    let full_id = "abcdef0123456789abcdef0123456789abcdef01";

    let known_commits = [
        ("1234567", commit(short_full)),
        (full_id, commit(full_id)),
        ("7654321", commit("7654321098765432109876543210987654321098")),
    ];

    let resolver = |word: &str| {
        known_commits
            .iter()
            .find(|(candidate, _)| *candidate == word)
            .map(|(_, id)| id.clone())
    };

    let cases = [
        (
            "bare ID",
            "commit 1234567 is ready",
            format!("commit [1234567](https://github.com/octo/widgets/commit/{short_full}) is ready"),
        ),
        (
            "one in backticks",
            "commit `1234567` is ready",
            format!("commit [`1234567`](https://github.com/octo/widgets/commit/{short_full}) is ready"),
        ),
        (
            "short one",
            "at 1234567",
            format!("at [1234567](https://github.com/octo/widgets/commit/{short_full})"),
        ),
        (
            "full one",
            "at abcdef0123456789abcdef0123456789abcdef01",
            format!("at [{full_id}](https://github.com/octo/widgets/commit/{full_id})"),
        ),
        (
            "at start of text",
            "1234567 is the commit",
            format!("[1234567](https://github.com/octo/widgets/commit/{short_full}) is the commit"),
        ),
        (
            "at end of text",
            "the commit is 1234567",
            format!("the commit is [1234567](https://github.com/octo/widgets/commit/{short_full})"),
        ),
        (
            "next to punctuation",
            "(1234567), and `1234567`. Also 1234567: check!",
            format!(
                "([1234567](https://github.com/octo/widgets/commit/{short_full})), and [`1234567`](https://github.com/octo/widgets/commit/{short_full}). Also [1234567](https://github.com/octo/widgets/commit/{short_full}): check!"
            ),
        ),
        (
            "two in one line",
            "from 1234567 to 7654321",
            format!(
                "from [1234567](https://github.com/octo/widgets/commit/{short_full}) to [7654321](https://github.com/octo/widgets/commit/7654321098765432109876543210987654321098)"
            ),
        ),
        ("names no commit", "a deadbeef hash", "a deadbeef hash".to_owned()),
        (
            "inside a fenced block",
            "```rust\nlet c = 1234567;\n```",
            "```rust\nlet c = 1234567;\n```".to_owned(),
        ),
        (
            "inside an existing link",
            "[1234567](https://example.com/commit/1234567)",
            "[1234567](https://example.com/commit/1234567)".to_owned(),
        ),
        (
            "part of a longer word",
            "prefix_1234567 and 1234567_suffix and 1234567890123456789012345678901234567890abc",
            "prefix_1234567 and 1234567_suffix and 1234567890123456789012345678901234567890abc".to_owned(),
        ),
        (
            "text without any",
            "no commits in this sentence.",
            "no commits in this sentence.".to_owned(),
        ),
    ];

    for (label, input, expected) in cases {
        let actual = link_commits(input, "octo", "widgets", resolver);
        assert_eq!(actual, expected, "case: {label}");
    }
}
