//! Bounded answer deadline for investigator requests.

/// Investigation answer policy; old configurations use thirty minutes.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Investigations {
    /// Validated whole minutes, in `10..=1440`.
    #[serde(deserialize_with = "read_minutes")]
    answer_minutes: u16,
}
impl Default for Investigations {
    fn default() -> Self {
        Self { answer_minutes: 30 }
    }
}
impl Investigations {
    /// Whole minutes allowed from creation of the current investigator task.
    #[must_use]
    pub fn answer_minutes(self) -> u16 {
        self.answer_minutes
    }
}
/// Checks the bounded interval once, at the configuration boundary.
fn read_minutes<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u16, D::Error> {
    let minutes = <u16 as serde::Deserialize>::deserialize(de)?;
    if !(10..=1440).contains(&minutes) {
        return Err(serde::de::Error::custom(
            "investigations.answer_minutes must be in 10..=1440",
        ));
    }
    Ok(minutes)
}
