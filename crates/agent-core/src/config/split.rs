//! Policy for checking split proposals, independent of deliverable kind.

/// Limits used by the pure split approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Split {
    /// Largest estimate in the repository's unit; zero disables size checks.
    pub part_limit: u32,
    /// Send-backs allowed before asking the human; zero asks at the first fault.
    pub corrections: u32,
}
impl Default for Split {
    fn default() -> Self {
        Self {
            part_limit: 1500,
            corrections: 2,
        }
    }
}

/// Configuration boundary retaining the unit separately from the copyable policy.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    /// Maximum amount in the configured unit; zero disables comparison.
    part_limit: u32,
    /// Maximum send-backs before consulting the human.
    corrections: u32,
    /// Nonblank unit, validated once by the deserializer.
    #[serde(deserialize_with = "read_unit")]
    unit: String,
}
impl Default for Settings {
    fn default() -> Self {
        let policy = Split::default();
        Self {
            part_limit: policy.part_limit,
            corrections: policy.corrections,
            unit: "lines".to_owned(),
        }
    }
}
impl Settings {
    pub(super) fn policy(&self) -> Split {
        Split {
            part_limit: self.part_limit,
            corrections: self.corrections,
        }
    }
    pub(super) fn unit(&self) -> &str {
        &self.unit
    }
}
/// Rejects blank units at the configuration boundary while allowing non-ASCII names.
fn read_unit<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let unit = <String as serde::Deserialize>::deserialize(deserializer)?;
    if unit.trim().is_empty() {
        return Err(serde::de::Error::custom(super::InvalidValue {
            what: "split unit",
            value: unit,
            rule: super::ValueRule::Empty,
        }));
    }
    Ok(unit)
}

#[cfg(test)]
mod tests;
