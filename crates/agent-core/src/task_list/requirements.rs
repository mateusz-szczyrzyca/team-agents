//! The requirements of an item: what its `Expected result:` and `Checks:` parts ask for, each with a number.
//!
//! The harness numbers them when it gives the item to an agent, so that a plan step, a commit and a report can say
//! which requirement they are about. The top-level bullets of the expected result are `R1`, `R2`, ...; the lines of the
//! checks keep the `K1`, `K2`, ... they start with, and a line without one gets the next free number.

use std::fmt;
use std::num::NonZeroU16;
use std::str::FromStr;

use super::Item;

/// Which part of an item a requirement comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequirementKind {
    /// A bullet of the expected result: `R1`.
    Result,
    /// A line of the checks: `K1`.
    Check,
}

impl RequirementKind {
    /// The letter that starts the requirement's number in a text.
    const fn letter(self) -> char {
        match self {
            Self::Result => 'R',
            Self::Check => 'K',
        }
    }
}

/// The number of a requirement of an item, written `R3` or `K2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequirementId {
    /// The part of the item it comes from.
    kind: RequirementKind,
    /// Its number within that part, from 1.
    number: NonZeroU16,
}

impl RequirementId {
    /// The requirement `number` of the part `kind`.
    #[must_use]
    pub const fn new(kind: RequirementKind, number: NonZeroU16) -> Self {
        Self { kind, number }
    }

    /// The part of the item it comes from.
    #[must_use]
    pub const fn kind(self) -> RequirementKind {
        self.kind
    }

    /// Its number within that part, from 1.
    #[must_use]
    pub const fn number(self) -> NonZeroU16 {
        self.number
    }
}

impl fmt::Display for RequirementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.kind.letter(), self.number)
    }
}

/// A text that is not the number of a requirement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not the number of a requirement: it is R or K and a number from 1, such as R1 or K2")]
pub struct InvalidRequirementId {
    /// The text that was refused.
    pub value: String,
}

impl FromStr for RequirementId {
    type Err = InvalidRequirementId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidRequirementId { value: text.to_owned() };
        let kind = match text.chars().next() {
            Some('R') => RequirementKind::Result,
            Some('K') => RequirementKind::Check,
            _ => return Err(invalid()),
        };
        let digits = text.get(1..).ok_or_else(invalid)?;
        let number = parse_number(digits).ok_or_else(invalid)?;
        Ok(Self::new(kind, number))
    }
}

/// A number from 1 to 65535 written with ASCII digits and no leading zero; `None` for any other text.
fn parse_number(digits: &str) -> Option<NonZeroU16> {
    if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// One requirement of an item: its number and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// Its number.
    pub id: RequirementId,
    /// What it asks for, as the item says it, without its bullet and its `K` label.
    pub text: String,
}

impl Item {
    /// The requirements of the item, in order: the `R` ones, then the `K` ones.
    ///
    /// The `R` ones are the top-level bullets of the expected result (its whole text when it has none), or the item's
    /// title as `R1` when the item has no such part. The `K` ones are the lines of the checks: a line that starts with
    /// `K<n>.` or `K<n>:` keeps that number, and any other line gets the smallest number no line names.
    #[must_use]
    pub fn requirements(&self) -> Vec<Requirement> {
        let results = match &self.expected_result {
            Some(text) => {
                let found = bullets(text);
                if found.is_empty() {
                    vec![text.trim().to_owned()]
                } else {
                    found
                }
            }
            None => vec![self.title.clone()],
        };
        let mut all = numbered_results(results);
        if let Some(text) = &self.checks {
            all.extend(numbered_checks(&check_lines(text)));
        }
        all
    }
}

impl Item {
    /// The expected result with each requirement written with its number: `R1. text` in place of the bullet. The text
    /// around the requirements is kept; `None` when the item has no expected result.
    #[must_use]
    pub fn expected_numbered(&self) -> Option<String> {
        None
    }

    /// The checks with each line written with its number: `K1. text`; `None` when the item has no checks.
    #[must_use]
    pub fn checks_numbered(&self) -> Option<String> {
        None
    }
}

/// The texts `results` as the requirements `R1`, `R2`, ... in order.
fn numbered_results(results: Vec<String>) -> Vec<Requirement> {
    numbers()
        .zip(results)
        .map(|(number, text)| Requirement {
            id: RequirementId::new(RequirementKind::Result, number),
            text,
        })
        .collect()
}

/// The numbers from 1 to 65535: more lines than that are not read, and the byte limit of a task list keeps any text far
/// below it.
fn numbers() -> impl Iterator<Item = NonZeroU16> {
    (1..=u16::MAX).filter_map(NonZeroU16::new)
}

/// The text of a bullet line, without its marker; `None` for a line that is not one.
fn bullet_text(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line == "-" || line == "*" {
        return Some("");
    }
    line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
}

/// The top-level bullets of `text`, each with the lines under it (nested bullets included, relative indentation
/// kept); lines before the first bullet are not part of any. Empty when `text` has no bullet.
fn bullets(text: &str) -> Vec<String> {
    let Some(top) = text
        .lines()
        .filter(|line| bullet_text(line).is_some())
        .map(super::indentation)
        .min()
    else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        match bullet_text(line).filter(|_| super::indentation(line) == top) {
            Some(rest) => found.push(rest.trim().to_owned()),
            None => {
                if let Some(open) = found.last_mut().filter(|_| !line.trim().is_empty()) {
                    open.push('\n');
                    open.push_str(super::dedent(line, top).trim_end());
                }
            }
        }
    }
    found
}

/// The lines of the checks: its top-level bullets, or its non-blank lines when it has no bullet.
fn check_lines(text: &str) -> Vec<String> {
    let found = bullets(text);
    if !found.is_empty() {
        return found;
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The number a check line starts with (`K2.` or `K2:`) and the rest of the line.
fn split_label(line: &str) -> (Option<NonZeroU16>, &str) {
    let Some(after) = line.strip_prefix('K') else {
        return (None, line);
    };
    let digits_end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    let (digits, rest) = after.split_at(digits_end);
    let Some(rest) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(':')) else {
        return (None, line);
    };
    match parse_number(digits) {
        Some(number) => (Some(number), rest.trim()),
        None => (None, line),
    }
}

/// The check `lines` as the requirements `K1`, `K2`, ...: a line keeps the number it names unless an earlier line took
/// it, and the others get the smallest numbers that no line names.
fn numbered_checks(lines: &[String]) -> Vec<Requirement> {
    let split: Vec<(Option<NonZeroU16>, String)> = lines
        .iter()
        .map(|line| {
            let (label, rest) = split_label(line);
            (label, rest.to_owned())
        })
        .collect();
    let named: Vec<NonZeroU16> = split.iter().filter_map(|(label, _)| *label).collect();
    let mut taken: Vec<NonZeroU16> = Vec::new();
    let mut found = Vec::new();
    for (label, text) in split {
        let number = match label.filter(|number| !taken.contains(number)) {
            Some(number) => number,
            None => match numbers().find(|number| !named.contains(number) && !taken.contains(number)) {
                Some(number) => number,
                // Every number is in use: a text cannot hold so many lines, so this is not reached.
                None => break,
            },
        };
        taken.push(number);
        found.push(Requirement {
            id: RequirementId::new(RequirementKind::Check, number),
            text,
        });
    }
    found
}

#[cfg(test)]
mod tests;
