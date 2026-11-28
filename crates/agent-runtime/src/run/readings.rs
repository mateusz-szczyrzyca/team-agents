//! The rate-limit readings a run showed, handed from the engine to the daemon that keeps them.

use agent_core::ids::TaskId;
use agent_core::limit_reading::LimitReading;
use agent_core::state::Timestamp;

/// What the stream of one run showed of the rate-limit windows of its account, when the run ended.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReadings {
    /// The task the run belonged to.
    pub task: TaskId,
    /// When the run ended: the time the readings were read.
    pub read_at: Timestamp,
    /// The latest reading of each window the run showed, at least one.
    pub readings: Vec<LimitReading>,
}
