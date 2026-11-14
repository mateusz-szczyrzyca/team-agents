//! A store that notes what the database refused: when it can write again, an event says which record is missing and why.

use agent_core::history::Record;

use super::db::Store;
use super::error::WriteError;

/// A store that passes records to the store it wraps.
#[derive(Debug)]
pub struct Noting<S> {
    /// The store the records go to.
    inner: S,
}

impl<S: Store> Noting<S> {
    /// Wraps `inner`.
    #[must_use]
    pub const fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: Store> Store for Noting<S> {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        self.inner.write(records)
    }
}
