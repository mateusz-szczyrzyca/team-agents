//! `agent say`: say something to a task or a team through the bus.

use std::io::Write;

use agent_core::protocol::{Command, SayError, SayRequest, SayText, Target};

use super::control::{ControlError, applied};
use crate::client::Connection;

/// The words to say, joined by spaces.
///
/// # Errors
/// [`SayError`] when they are blank, too long or hold a NUL.
pub(crate) fn text(words: &[String]) -> Result<SayText, SayError> {
    SayText::try_from(words.join(" "))
}

/// Asks the daemon to say `text` to `target`, and prints who heard it.
///
/// # Errors
/// [`ControlError`].
pub(crate) async fn say(
    connection: &mut Connection,
    target: Target,
    text: SayText,
    out: &mut impl Write,
) -> Result<(), ControlError> {
    applied(connection, Command::Say(SayRequest { target, text }), "Said it to", out).await
}
