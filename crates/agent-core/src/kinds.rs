//! Kinds: the model CLI an agent runs, the tier of the configuration it is in, and the role it has in a task.
//!
//! These and the statuses of [`crate::status`] are keyword enums: each variant has one text, which `Display` prints
//! and `FromStr` and serde accept; any other text is an [`UnknownKeyword`].

/// Declares a keyword enum: a fieldless enum whose variants each have one text, used by `Display`, `FromStr`,
/// `TryFrom<String>` (which serde deserializes through) and `From<$name> for &str` (which serde serializes through).
/// `ALL` lists the variants in declaration order.
macro_rules! keyword_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $what:literal {
            $( $(#[$variant_meta:meta])* $variant:ident => $text:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "&'static str")]
        pub enum $name {
            $( $(#[$variant_meta])* $variant, )+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The variant's text: what `Display` prints and `FromStr` and serde accept.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::kinds::UnknownKeyword;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    _ => Err($crate::kinds::UnknownKeyword {
                        what: $what,
                        value: text.to_owned(),
                        expected: &[$($text),+],
                    }),
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::kinds::UnknownKeyword;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                text.parse()
            }
        }

        impl From<$name> for &'static str {
            fn from(value: $name) -> Self {
                value.as_str()
            }
        }
    };
}

pub(crate) use keyword_enum;

/// Why a text is not one of a keyword enum's texts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {what} {value:?}: expected one of {}", .expected.join(", "))]
pub struct UnknownKeyword {
    /// What was being parsed, for the message: `CLI kind`, `task status`, ...
    pub what: &'static str,
    /// The refused text.
    pub value: String,
    /// The texts that would have been accepted, in declaration order.
    pub expected: &'static [&'static str],
}

keyword_enum! {
    /// The model CLI an agent or investigator runs.
    pub enum CliKind: "CLI kind" {
        /// Claude Code, the `claude` command.
        Claude => "claude",
        /// Codex CLI, the `codex` command.
        Codex => "codex",
        /// Google Antigravity with Gemini models, the `agy` command.
        Gemini => "gemini",
        /// opencode, the `opencode` command.
        Opencode => "opencode",
    }
}

keyword_enum! {
    /// The list of the configuration an agent is in, which decides what it may do.
    pub enum Tier: "tier" {
        /// Works on task-list items, solo or as a team member, and may coordinate a team.
        Normal => "normal",
        /// Serves `delegate` requests of team members; never takes items and never coordinates.
        Worker => "worker",
        /// Reviews the pull requests of teams it is not in; in a team it may coordinate.
        Investigator => "investigator",
    }
}

keyword_enum! {
    /// What a task does: alone, in its team, or for another team.
    pub enum Role: "role" {
        /// The only agent of a task without a team.
        Solo => "solo",
        /// Leads a team: integrates the members' branches, gets the gate green, opens the pull request.
        Coordinator => "coordinator",
        /// Works on the items the daemon assigns to it.
        Member => "member",
        /// Serves a `delegate` request of a team member.
        Worker => "worker",
        /// Reviews a pull request of a team it is not in.
        Reviewer => "reviewer",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fmt::Debug;
    use std::str::FromStr;

    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use serde_json::error::Category;

    use super::*;

    /// Texts no keyword enum accepts: empty, blank, padded, other case, non-ASCII.
    const NEVER_KEYWORDS: &[&str] = &["", " ", "x", "żółw", "done!", "\u{0}"];

    /// Asserts that `K` has exactly the variants and texts of `table`, in its order, and that each text round trips
    /// through `Display`, `FromStr`, `TryFrom<String>` and JSON.
    pub(crate) fn assert_keywords<K>(table: &[(K, &str)])
    where
        K: Copy + PartialEq + Debug + std::fmt::Display + Serialize + DeserializeOwned,
        K: FromStr<Err = UnknownKeyword> + TryFrom<String, Error = UnknownKeyword> + Into<&'static str>,
    {
        for &(value, text) in table {
            assert_eq!(value.to_string(), text);
            assert_eq!(value.into(), text);
            assert_eq!(text.parse::<K>(), Ok(value));
            assert_eq!(K::try_from(text.to_owned()), Ok(value));
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(json, serde_json::to_string(text).unwrap());
            assert_eq!(serde_json::from_str::<K>(&json).unwrap(), value);
        }
    }

    /// Asserts that `K`, parsed as `what`, refuses `texts`, the texts of `table` in other case and padded, and the
    /// texts no keyword enum accepts, each with the exact [`UnknownKeyword`], also in JSON.
    pub(crate) fn assert_refuses<K>(what: &'static str, table: &[(K, &'static str)], texts: &[&str])
    where
        K: PartialEq + Debug + DeserializeOwned + FromStr<Err = UnknownKeyword>,
    {
        let expected: Vec<&str> = table.iter().map(|&(_, text)| text).collect();
        let variants = table.iter().flat_map(|&(_, text)| {
            let other_case = if text.to_lowercase() == text {
                text.to_uppercase()
            } else {
                text.to_lowercase()
            };
            [other_case, format!(" {text}"), format!("{text} "), format!("{text}\n")]
        });
        let refused = texts
            .iter()
            .chain(NEVER_KEYWORDS)
            .map(|&t| t.to_owned())
            .chain(variants);
        for text in refused {
            let Err(err) = text.parse::<K>() else {
                panic!("{text:?} is accepted as a {what}");
            };
            assert_eq!(
                (err.what, err.value.as_str(), err.expected),
                (what, text.as_str(), expected.as_slice())
            );
            let json_err = serde_json::from_str::<K>(&serde_json::to_string(&text).unwrap()).unwrap_err();
            assert_eq!(json_err.classify(), Category::Data, "{text:?}");
        }
    }

    const CLI_KINDS: &[(CliKind, &str)] = &[
        (CliKind::Claude, "claude"),
        (CliKind::Codex, "codex"),
        (CliKind::Gemini, "gemini"),
        (CliKind::Opencode, "opencode"),
    ];

    const TIERS: &[(Tier, &str)] = &[
        (Tier::Normal, "normal"),
        (Tier::Worker, "worker"),
        (Tier::Investigator, "investigator"),
    ];

    const ROLES: &[(Role, &str)] = &[
        (Role::Solo, "solo"),
        (Role::Coordinator, "coordinator"),
        (Role::Member, "member"),
        (Role::Worker, "worker"),
        (Role::Reviewer, "reviewer"),
    ];

    #[test]
    fn cli_kinds_have_their_texts_and_round_trip() {
        assert_eq!(CliKind::ALL, CLI_KINDS.iter().map(|&(k, _)| k).collect::<Vec<_>>());
        assert_keywords(CLI_KINDS);
    }

    #[test]
    fn cli_kinds_refuse_unknown_texts() {
        assert_refuses(
            "CLI kind",
            CLI_KINDS,
            &["agy", "antigravity", "gemini-cli", "claude-code", "open-code"],
        );
    }

    #[test]
    fn tiers_have_their_texts_and_round_trip() {
        assert_eq!(Tier::ALL, TIERS.iter().map(|&(t, _)| t).collect::<Vec<_>>());
        assert_keywords(TIERS);
    }

    #[test]
    fn tiers_refuse_unknown_texts() {
        assert_refuses("tier", TIERS, &["helper", "workers", "lead"]);
    }

    #[test]
    fn roles_have_their_texts_and_round_trip() {
        assert_eq!(Role::ALL, ROLES.iter().map(|&(r, _)| r).collect::<Vec<_>>());
        assert_keywords(ROLES);
    }

    #[test]
    fn roles_refuse_unknown_texts() {
        assert_refuses("role", ROLES, &["lead", "helper", "investigator", "merger"]);
    }
}
