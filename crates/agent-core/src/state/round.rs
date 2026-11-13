//! Rounds: the work of a team on one item of its task list, from the item's start to its end — or the whole work of a
//! team whose list has no items.
//!
//! A round is a thing with an ID, so that whatever happens can say which round it belongs to. A team holds its rounds,
//! oldest first, and a run names the rounds it works for. A round is open or closed: closed with a time and how it
//! ended, and never open again. What ties a round to its item, its team and its runs is checked by the operations of
//! [`Team`](super::Team) and [`State`](super::State) and again when a state is read; this module is the round itself.

use serde::{Deserialize, Serialize};

use super::strict::{checked, strict_enum};
use super::values::Timestamp;
use crate::ids::{ItemId, RoundId};
use crate::kinds::{Keyword, UnknownKeyword};

/// One round: the work of a team on one item, or on its whole list when the list has no items.
///
/// Made by [`Team::open_round`](super::Team::open_round) and closed by [`Team::close_round`](super::Team::close_round),
/// so a round is never closed before it was opened; reading a state checks that too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Round {
    /// The round's ID.
    id: RoundId,
    /// The item the round is for; none for the whole work of a team whose task list has no items.
    #[serde(skip_serializing_if = "Option::is_none")]
    item: Option<ItemId>,
    /// When the round was opened.
    opened_at: Timestamp,
    /// Whether it is open, or when and how it was closed.
    state: RoundLife,
}

/// How a [`Round`] is read; see [`checked!`].
#[derive(Deserialize)]
#[serde(remote = "Round", deny_unknown_fields)]
struct RoundWire {
    /// See [`Round`].
    id: RoundId,
    /// See [`Round`]; absent for a round without an item.
    item: Option<ItemId>,
    /// See [`Round`].
    opened_at: Timestamp,
    /// See [`Round`].
    state: RoundLife,
}

checked!(Round, RoundWire, |round: &Round| match &round.state {
    RoundLife::Closed { at, .. } => check_close(&round.id, round.opened_at, *at),
    RoundLife::Open => Ok(()),
});

/// Whether a round is open, or when and how it was closed.
///
/// Deserialization refuses fields that the variant does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RoundLife {
    /// The round goes on.
    Open,
    /// The round is over.
    Closed {
        /// When it was closed.
        at: Timestamp,
        /// How it ended.
        how: RoundEnd,
    },
}

/// How a [`RoundLife`] is read; see [`strict_enum!`].
#[derive(Deserialize)]
#[serde(remote = "RoundLife", tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum RoundLifeWire {
    /// See [`RoundLife::Open`].
    Open,
    /// See [`RoundLife::Closed`].
    Closed {
        /// See [`RoundLife::Closed`].
        at: Timestamp,
        /// See [`RoundLife::Closed`].
        how: RoundEnd,
    },
}

strict_enum!(RoundLife, RoundLifeWire);

/// How a round ended.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<RoundEnd>)]
pub enum RoundEnd {
    /// The pull request of the item was merged; its item is done as merged.
    Merged,
    /// Both reviews approved and the team does not merge by itself (`--no-merge`): the pull request is the human's.
    Approved,
    /// The team was stopped or ended without a merge or an approval.
    Abandoned,
}

impl Keyword for RoundEnd {
    const WHAT: &'static str = "round ending";
}

impl TryFrom<String> for RoundEnd {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl RoundEnd {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The variant's text: what `Display` prints and `FromStr` and serde accept.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// A round closed before it was opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("round {round} cannot be closed at {at}, before it was opened at {opened_at}")]
pub struct ClosedBeforeOpened {
    /// The round.
    pub round: RoundId,
    /// When it was opened.
    pub opened_at: Timestamp,
    /// The refused time of the end.
    pub at: Timestamp,
}

impl Round {
    /// A round that has just been opened.
    pub(super) const fn opened(id: RoundId, item: Option<ItemId>, opened_at: Timestamp) -> Self {
        Self {
            id,
            item,
            opened_at,
            state: RoundLife::Open,
        }
    }

    getter!("The round's ID.", pub const fn id(&self) -> &RoundId = &self.id);

    getter!(
        "The item the round is for; none for the whole work of a team whose task list has no items.",
        pub const fn item(&self) -> Option<ItemId> = self.item
    );

    getter!("When the round was opened.", pub const fn opened_at(&self) -> Timestamp = self.opened_at);

    getter!("Whether the round is open, or when and how it was closed.", pub const fn life(&self) -> &RoundLife = &self.state);

    /// Whether the round goes on.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, RoundLife::Open)
    }

    /// How the round ended; none while it is open.
    #[must_use]
    pub const fn ended(&self) -> Option<RoundEnd> {
        match self.state {
            RoundLife::Open => None,
            RoundLife::Closed { how, .. } => Some(how),
        }
    }

    /// Closes the round. A round that is closed already stays as it is: whoever calls this checks that.
    pub(super) fn close(&mut self, at: Timestamp, how: RoundEnd) -> Result<(), ClosedBeforeOpened> {
        check_close(&self.id, self.opened_at, at)?;
        self.state = RoundLife::Closed { at, how };
        Ok(())
    }
}

/// Checks that a round opened at `opened_at` may be closed at `at`.
fn check_close(id: &RoundId, opened_at: Timestamp, at: Timestamp) -> Result<(), ClosedBeforeOpened> {
    if at < opened_at {
        return Err(ClosedBeforeOpened {
            round: id.clone(),
            opened_at,
            at,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
