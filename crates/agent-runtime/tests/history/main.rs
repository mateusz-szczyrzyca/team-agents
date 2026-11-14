//! The history database through the public API of `agent-runtime`: its schema, the migration mechanism, every record
//! written and read back, the writer behind its queue, the failures that must never stop the work, and a writer killed
//! in the middle of a batch. Generated values come from a seeded source: a failure names its seed.

#[cfg(test)]
mod docs;
#[cfg(test)]
mod gate_rounds;
#[cfg(test)]
mod invariants;
#[cfg(test)]
mod kill;
#[cfg(test)]
mod ledger;
#[cfg(test)]
mod records;
#[cfg(test)]
mod schema;
#[cfg(test)]
mod sessions;
#[cfg(test)]
mod states;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tampered;
#[cfg(test)]
mod writer;

#[cfg(test)]
mod redaction;
