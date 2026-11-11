//! The prompt catalog and the strict `{{name}}` renderer.
//!
//! Everything a model reads, and everything written into a file the human answers (questions-file instructions,
//! notification texts), lives in `prompts/` as a template; no `.rs` file holds such a text. The catalog is
//! [`PromptId`]: each entry names its file under [`PROMPTS_DIR`], who reads it ([`Audience`]) and the variables its
//! text uses. This module only decides: reading the files is the runtime's job, so it hands the text in.
//!
//! # Template syntax
//!
//! - `{{name}}` is replaced by the value of the variable `name` (see [`VariableName`]: lowercase letters, digits and
//!   `_`). No spaces inside the braces.
//! - `\{{` is a literal `{{`: the backslash goes, and what follows is plain text, so `\{{name}}` renders as
//!   `{{name}}`. Only a backslash directly before `{{` is special; to put a backslash in front of an escaped `{{`,
//!   write two. A single `{`, a `}}` on its own and every other backslash are plain text.
//! - A value is inserted as it is and never searched for placeholders.
//!
//! The renderer is strict: a variable without a value, a value for a variable the text does not use, a placeholder
//! that is never closed and a name that is not valid are errors, never silently left in the text.
//!
//! # Languages
//!
//! Only the texts the human reads ([`Audience::Human`]) have per-language versions: for the language `Polish`
//! the file `prompts/lang/polish/<path>` replaces `prompts/<path>` when it exists, and English is the fallback
//! ([`PromptSpec::candidates`] lists the files to try, in order). What a model reads is English only.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use crate::config::Language;

/// The directory of the prompts, in the repository and in an installation; every path of the catalog is relative
/// to it.
pub const PROMPTS_DIR: &str = "prompts";

/// The directory, under [`PROMPTS_DIR`], that holds a subdirectory with the overrides of each language.
const LANGUAGE_DIR: &str = "lang";

/// Longest variable name, in bytes.
const MAX_VARIABLE_NAME_LEN: usize = 64;

/// What opens a placeholder.
const PLACEHOLDER_OPEN: &str = "{{";

/// What closes a placeholder.
const PLACEHOLDER_CLOSE: &str = "}}";

/// The character that, directly before [`PLACEHOLDER_OPEN`], makes it plain text.
const ESCAPE: char = '\\';

/// The rule a variable name breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum NameRule {
    /// The name is empty.
    #[error("it is empty")]
    Empty,
    /// The name is longer than `max` bytes.
    #[error("it is longer than {max} bytes")]
    TooLong {
        /// The longest allowed name, in bytes.
        max: usize,
    },
    /// The name does not start with a lowercase ASCII letter.
    #[error("it must start with a lowercase letter")]
    FirstNotLowercaseLetter,
    /// The name contains this character; only lowercase letters, digits and `_` are allowed.
    #[error("{0:?} is not allowed: use lowercase letters, digits and '_'")]
    ForbiddenChar(char),
}

/// Why a text is not a variable name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid variable name {name:?}: {rule}")]
pub struct InvalidVariableName {
    /// The refused text; at most one byte over [`MAX_VARIABLE_NAME_LEN`] characters of it, so that a huge text
    /// does not become a huge message.
    pub name: String,
    /// The rule the text breaks.
    pub rule: NameRule,
}

/// The name of a template variable: 1 to 64 bytes of lowercase ASCII letters, digits and `_`, starting with a letter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableName(String);

impl VariableName {
    /// The name's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VariableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for VariableName {
    type Err = InvalidVariableName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.to_owned()))
    }
}

/// Why a value could not be added to [`Variables`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VariablesError {
    /// The name is not a valid variable name.
    #[error(transparent)]
    InvalidName(#[from] InvalidVariableName),
    /// The variable already has a value: two values for one variable are a mistake, not an override.
    #[error("variable {name} has a value already")]
    Duplicate {
        /// The variable that was given twice.
        name: VariableName,
    },
}

/// The values of the variables of one rendering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Variables(BTreeMap<VariableName, String>);

impl Variables {
    /// No values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the value of the variable `name`.
    ///
    /// # Errors
    /// [`VariablesError::InvalidName`] when `name` is not a valid [`VariableName`]; [`VariablesError::Duplicate`]
    /// when the variable has a value already.
    pub fn with(self, name: &str, value: impl Into<String>) -> Result<Self, VariablesError> {
        let _ = (name, value.into());
        Ok(self)
    }

    /// The value of `name`, if it has one.
    #[must_use]
    pub fn get(&self, name: &VariableName) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// The names that have a value, in order.
    pub fn names(&self) -> impl Iterator<Item = &VariableName> {
        self.0.keys()
    }
}

/// Why a text is not a valid template.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// A placeholder is opened and never closed.
    #[error("line {line}: a placeholder is not closed: end it with }}}} or escape its opening braces as \\{{{{")]
    Unclosed {
        /// The line, counted from 1, where the placeholder opens.
        line: usize,
    },
    /// A placeholder does not hold a valid variable name.
    #[error("line {line}: a placeholder does not hold a variable name")]
    InvalidName {
        /// The line, counted from 1, where the placeholder opens.
        line: usize,
        /// What is wrong with the name.
        #[source]
        source: InvalidVariableName,
    },
}

/// Why a template could not be rendered with the values given.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The text uses a variable that has no value.
    #[error("no value for variable {name}")]
    Missing {
        /// The variable without a value.
        name: VariableName,
    },
    /// A value was given for a variable the text does not use.
    #[error("a value for variable {name}, which the text does not use")]
    Unknown {
        /// The variable that was given and is not used.
        name: VariableName,
    },
}

/// A parsed template: text and placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// The text in order, with the escapes resolved.
    parts: Vec<Part>,
}

/// One piece of a [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// Plain text, rendered as it is.
    Text(String),
    /// A placeholder, replaced by its variable's value.
    Variable(VariableName),
}

impl Template {
    /// Parses `text` as a template.
    ///
    /// # Errors
    /// [`TemplateError::Unclosed`] for a `{{` that is never closed, [`TemplateError::InvalidName`] for a
    /// placeholder that does not hold a valid name (`{{ name }}`, `{{Name}}`, `{{}}`); both say the line.
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        let _ = text;
        Ok(Self { parts: Vec::new() })
    }

    /// The variables the text uses, in order, each once.
    #[must_use]
    pub fn variables(&self) -> BTreeSet<&VariableName> {
        BTreeSet::new()
    }

    /// The text with every placeholder replaced by its value.
    ///
    /// # Errors
    /// [`RenderError::Missing`] when the text uses a variable that has no value, [`RenderError::Unknown`] when a
    /// value is for a variable the text does not use.
    pub fn render(&self, values: &Variables) -> Result<String, RenderError> {
        let _ = values;
        Ok(String::new())
    }
}

/// Who reads a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Audience {
    /// A model: the text exists in English only.
    Model,
    /// The human (questions-file instructions, notifications): the text may have a version per language.
    Human,
}

/// Why a text is not a valid prompt of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptError {
    /// The text is not a valid template.
    #[error(transparent)]
    Template(#[from] TemplateError),
    /// The text uses a variable the catalog does not list for this prompt.
    #[error("the text uses variable {name}, which the catalog does not list for this prompt")]
    Undeclared {
        /// The variable the text uses.
        name: VariableName,
    },
    /// The text does not use a variable the catalog lists for this prompt.
    #[error("the text does not use variable {name}, which the catalog lists for this prompt")]
    Unused {
        /// The variable the catalog lists.
        name: &'static str,
    },
    /// The values do not match the variables of the text.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// What the catalog knows about one prompt: where it is, who reads it and which variables it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptSpec {
    /// The English file, relative to [`PROMPTS_DIR`], with `/` as the separator.
    path: &'static str,
    /// Who reads the text.
    audience: Audience,
    /// The variables the text uses, each a valid [`VariableName`]; the text and this list name the same set.
    variables: &'static [&'static str],
}

impl PromptSpec {
    /// A catalog entry.
    pub(crate) const fn new(path: &'static str, audience: Audience, variables: &'static [&'static str]) -> Self {
        Self {
            path,
            audience,
            variables,
        }
    }

    /// The English file, relative to [`PROMPTS_DIR`], with `/` as the separator.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Who reads the text.
    #[must_use]
    pub const fn audience(&self) -> Audience {
        self.audience
    }

    /// The variables the text uses: the values a rendering must give, no more and no less.
    #[must_use]
    pub const fn variables(&self) -> &'static [&'static str] {
        self.variables
    }

    /// The files to try for `language`, relative to [`PROMPTS_DIR`], the first that exists wins: the language's
    /// own version first, then English. Only a text for the human has a version per language.
    #[must_use]
    pub fn candidates(&self, language: &Language) -> Vec<PathBuf> {
        let _ = language;
        Vec::new()
    }

    /// Renders `text`, the content of one of this prompt's files, with `values`.
    ///
    /// # Errors
    /// [`PromptError::Template`] when `text` is not a valid template; [`PromptError::Undeclared`] when it uses a
    /// variable the catalog does not list; [`PromptError::Unused`] when it leaves one out;
    /// [`PromptError::Render`] when `values` give no value for a listed variable or one for another.
    pub fn render(&self, text: &str, values: &Variables) -> Result<String, PromptError> {
        let _ = (text, values);
        Ok(String::new())
    }
}

/// Every prompt and human-facing text of the harness.
///
/// Later items of the plan add the texts the harness composes (`prompts/harness/<module>/<name>.md`); each is a
/// variant here, so that a missing file or a variable that drifted is caught by the catalog test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PromptId {
    /// The clarification rules appended to a request, `clarification.md`.
    Clarification,
    /// The Rust developer rules for a normal agent, `agent/rust-developer.md`.
    AgentRustDeveloper,
    /// The Rust developer rules for a worker, `worker-agent/rust-developer.md`: a delegated task, no pull request.
    WorkerAgentRustDeveloper,
    /// The Rust developer rules for an investigator, `investigator/rust-developer.md`.
    InvestigatorRustDeveloper,
    /// The review rules for an investigator reviewing a pull request, `investigator/code-review.md`.
    InvestigatorCodeReview,
}

impl PromptId {
    /// Every prompt of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::Clarification,
        Self::AgentRustDeveloper,
        Self::WorkerAgentRustDeveloper,
        Self::InvestigatorRustDeveloper,
        Self::InvestigatorCodeReview,
    ];

    /// What the catalog knows about this prompt.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::Clarification => PromptSpec::new("clarification.md", Audience::Model, &[]),
            Self::AgentRustDeveloper => PromptSpec::new("agent/rust-developer.md", Audience::Model, &[]),
            Self::WorkerAgentRustDeveloper => PromptSpec::new("worker-agent/rust-developer.md", Audience::Model, &[]),
            Self::InvestigatorRustDeveloper => PromptSpec::new("investigator/rust-developer.md", Audience::Model, &[]),
            Self::InvestigatorCodeReview => PromptSpec::new("investigator/code-review.md", Audience::Model, &[]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spec of a text for the human with two variables, the shape the harness texts will have.
    const HUMAN: PromptSpec = PromptSpec::new("harness/questions/header.md", Audience::Human, &["task", "file"]);

    fn values(pairs: &[(&str, &str)]) -> Variables {
        pairs.iter().fold(Variables::new(), |values, (name, value)| {
            values.with(name, *value).unwrap()
        })
    }

    fn render(text: &str, pairs: &[(&str, &str)]) -> Result<String, RenderError> {
        Template::parse(text).unwrap().render(&values(pairs))
    }

    fn name(text: &str) -> VariableName {
        text.parse().unwrap()
    }

    fn language(text: &str) -> Language {
        text.parse().unwrap()
    }

    // Rule: text without placeholders renders as it is.
    #[test]
    fn plain_text_renders_unchanged() {
        for text in ["", "one line", "two\nlines\n", "żółć 日本語 🦀", "a { b } c }} d { e"] {
            assert_eq!(render(text, &[]).as_deref(), Ok(text), "{text:?}");
        }
    }

    // Rule: a placeholder is replaced by its value, each time it appears.
    #[test]
    fn placeholders_are_replaced_by_their_values() {
        let text = "Task {{task}} in {{repo}}; again {{task}}.\n";

        let rendered = render(text, &[("task", "2.3"), ("repo", "team-agents")]);

        assert_eq!(rendered.as_deref(), Ok("Task 2.3 in team-agents; again 2.3.\n"));
    }

    // Rule: a value is inserted as it is, never searched for placeholders (no injection through a value).
    #[test]
    fn a_value_is_not_expanded_again() {
        let rendered = render("[{{a}}]", &[("a", "{{b}} \\{{c}} {{")]);

        assert_eq!(rendered.as_deref(), Ok("[{{b}} \\{{c}} {{]"));
    }

    // Rule: a variable without a value is an error that names it.
    #[test]
    fn a_variable_without_a_value_is_missing() {
        let rendered = render("{{task}} {{file}}", &[("task", "x")]);

        assert_eq!(rendered, Err(RenderError::Missing { name: name("file") }));
    }

    // Rule: a value for a variable the text does not use is an error that names it.
    #[test]
    fn a_value_for_an_unused_variable_is_unknown() {
        let rendered = render("{{task}}", &[("task", "x"), ("extra", "y")]);

        assert_eq!(rendered, Err(RenderError::Unknown { name: name("extra") }));
    }

    // Rule: `\{{` is a literal `{{`, and what follows is plain text.
    #[test]
    fn an_escaped_opening_is_literal() {
        assert_eq!(render("\\{{task}}", &[]).as_deref(), Ok("{{task}}"));
        assert_eq!(render("a \\{{ b", &[]).as_deref(), Ok("a {{ b"));
        assert_eq!(render("\\{{x}} and {{x}}", &[("x", "1")]).as_deref(), Ok("{{x}} and 1"));
    }

    // Rule: only the backslash directly before `{{` is special; two backslashes leave one.
    #[test]
    fn only_the_backslash_before_the_braces_is_an_escape() {
        assert_eq!(render("\\\\{{x}}", &[]).as_deref(), Ok("\\{{x}}"));
        assert_eq!(render("C:\\dir\\file \\n", &[]).as_deref(), Ok("C:\\dir\\file \\n"));
        assert_eq!(render("\\ {{x}}", &[("x", "1")]).as_deref(), Ok("\\ 1"));
    }

    // Rule: a placeholder that is never closed is an error with its line.
    #[test]
    fn an_unclosed_placeholder_is_refused_with_its_line() {
        for (text, line) in [
            ("{{task", 1),
            ("a\nb\n{{task}} {{file\n", 3),
            ("{{ a\nb", 1),
            ("x {{", 1),
        ] {
            assert_eq!(Template::parse(text), Err(TemplateError::Unclosed { line }), "{text:?}");
        }
    }

    // Rule: a placeholder holds exactly a valid name: no spaces, no capitals, nothing empty.
    #[test]
    fn a_placeholder_with_an_invalid_name_is_refused_with_its_line() {
        let cases = [
            ("{{}}", "", NameRule::Empty),
            ("{{ task }}", " task ", NameRule::FirstNotLowercaseLetter),
            ("{{Task}}", "Task", NameRule::FirstNotLowercaseLetter),
            ("{{1st}}", "1st", NameRule::FirstNotLowercaseLetter),
            ("{{_x}}", "_x", NameRule::FirstNotLowercaseLetter),
            ("{{a-b}}", "a-b", NameRule::ForbiddenChar('-')),
            ("{{a.b}}", "a.b", NameRule::ForbiddenChar('.')),
            ("{{aB}}", "aB", NameRule::ForbiddenChar('B')),
            ("{{é}}", "é", NameRule::FirstNotLowercaseLetter),
            ("{{{task}}}", "{task", NameRule::FirstNotLowercaseLetter),
            ("{{a\nb}}", "a\nb", NameRule::ForbiddenChar('\n')),
        ];
        for (text, shown, rule) in cases {
            let source = InvalidVariableName {
                name: shown.to_owned(),
                rule,
            };
            assert_eq!(
                Template::parse(text),
                Err(TemplateError::InvalidName { line: 1, source }),
                "{text:?}"
            );
        }
        let err = Template::parse("one\ntwo\n{{Bad}}").unwrap_err();
        assert!(matches!(err, TemplateError::InvalidName { line: 3, .. }), "{err:?}");
    }

    // Rule: a huge text in a placeholder does not become a huge error.
    #[test]
    fn a_long_name_is_refused_and_shown_cut() {
        let long = "a".repeat(MAX_VARIABLE_NAME_LEN + 1);
        let huge = "a".repeat(100_000);

        let err = Template::parse(&format!("{{{{{long}}}}}")).unwrap_err();
        assert_eq!(
            err,
            TemplateError::InvalidName {
                line: 1,
                source: InvalidVariableName {
                    name: long,
                    rule: NameRule::TooLong {
                        max: MAX_VARIABLE_NAME_LEN
                    }
                }
            }
        );
        let TemplateError::InvalidName { source, .. } = Template::parse(&format!("{{{{{huge}}}}}")).unwrap_err() else {
            panic!("a long name is an invalid name");
        };
        assert_eq!(source.name.len(), MAX_VARIABLE_NAME_LEN + 1);
    }

    // Rule: a variable name is 1 to 64 bytes of a-z, 0-9 and `_`, starting with a letter.
    #[test]
    fn variable_names_follow_their_rule() {
        for text in ["a", "task", "task_id", "a1", "x_", &"a".repeat(MAX_VARIABLE_NAME_LEN)] {
            assert_eq!(text.parse::<VariableName>().unwrap().as_str(), text);
            assert_eq!(name(text).to_string(), text);
        }
        for text in [
            "",
            "A",
            "1a",
            "_a",
            "a b",
            "a-b",
            "a/b",
            "é",
            "a\0",
            &"a".repeat(MAX_VARIABLE_NAME_LEN + 1),
        ] {
            assert!(text.parse::<VariableName>().is_err(), "{text:?}");
        }
    }

    // Rule: values refuse an invalid name and a variable given twice.
    #[test]
    fn variables_refuse_invalid_and_duplicate_names() {
        let err = Variables::new().with("Bad", "x").unwrap_err();
        assert!(matches!(err, VariablesError::InvalidName(ref e) if e.rule == NameRule::FirstNotLowercaseLetter));

        let err = Variables::new().with("a", "1").unwrap().with("a", "2").unwrap_err();
        assert_eq!(err, VariablesError::Duplicate { name: name("a") });
    }

    #[test]
    fn variables_hold_their_values() {
        let vars = values(&[("b", "2"), ("a", "1")]);

        assert_eq!(vars.get(&name("a")), Some("1"));
        assert_eq!(vars.get(&name("c")), None);
        assert_eq!(vars.names().map(VariableName::as_str).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn a_template_lists_its_variables_once_each() {
        let template = Template::parse("{{b}} {{a}} {{b}} \\{{c}}").unwrap();

        let names: Vec<_> = template.variables().into_iter().map(VariableName::as_str).collect();

        assert_eq!(names, ["a", "b"]);
    }

    // Rule: a catalog entry renders only text that uses exactly its variables.
    #[test]
    fn a_prompt_renders_with_its_variables() {
        let text = "Answer {{task}} in {{file}}.";

        let rendered = HUMAN.render(text, &values(&[("task", "2.3"), ("file", "questions-1.md")]));

        assert_eq!(rendered.as_deref(), Ok("Answer 2.3 in questions-1.md."));
    }

    #[test]
    fn a_prompt_refuses_a_variable_the_catalog_does_not_list() {
        let rendered = HUMAN.render("{{task}} {{file}} {{other}}", &values(&[("task", "x"), ("file", "y")]));

        assert_eq!(rendered, Err(PromptError::Undeclared { name: name("other") }));
    }

    #[test]
    fn a_prompt_refuses_a_text_that_leaves_a_listed_variable_out() {
        let rendered = HUMAN.render("{{task}}", &values(&[("task", "x"), ("file", "y")]));

        assert_eq!(rendered, Err(PromptError::Unused { name: "file" }));
    }

    #[test]
    fn a_prompt_refuses_missing_and_unknown_values() {
        let text = "{{task}} {{file}}";

        assert_eq!(
            HUMAN.render(text, &values(&[("task", "x")])),
            Err(PromptError::Render(RenderError::Missing { name: name("file") }))
        );
        assert_eq!(
            HUMAN.render(text, &values(&[("task", "x"), ("file", "y"), ("z", "z")])),
            Err(PromptError::Render(RenderError::Unknown { name: name("z") }))
        );
    }

    #[test]
    fn a_prompt_refuses_text_that_is_not_a_template() {
        let rendered = HUMAN.render("{{task", &Variables::new());

        assert_eq!(
            rendered,
            Err(PromptError::Template(TemplateError::Unclosed { line: 1 }))
        );
    }

    // Rule: English is the fallback; a text for the human has a version per language, a text for a model never.
    #[test]
    fn the_language_version_comes_first_and_english_is_the_fallback() {
        let english = PathBuf::from("harness/questions/header.md");

        assert_eq!(HUMAN.candidates(&language("English")), [english.clone()]);
        assert_eq!(HUMAN.candidates(&language("ENGLISH")), [english.clone()]);
        assert_eq!(
            HUMAN.candidates(&language("Polish")),
            [
                PathBuf::from("lang/polish/harness/questions/header.md"),
                english.clone()
            ]
        );
        assert_eq!(
            HUMAN.candidates(&language("pt-BR")),
            [PathBuf::from("lang/pt-br/harness/questions/header.md"), english]
        );
    }

    #[test]
    fn a_text_for_a_model_has_no_language_versions() {
        let spec = PromptId::AgentRustDeveloper.spec();

        assert_eq!(
            spec.candidates(&language("Polish")),
            [PathBuf::from("agent/rust-developer.md")]
        );
    }

    #[test]
    fn the_catalog_lists_each_prompt_once_with_a_relative_markdown_path() {
        let mut paths = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for &id in PromptId::ALL {
            let spec = id.spec();
            assert!(ids.insert(id), "{id:?} is listed twice");
            assert!(paths.insert(spec.path()), "{} is used by two prompts", spec.path());
            assert!(spec.path().ends_with(".md"), "{}", spec.path());
            assert!(
                !spec.path().starts_with('/') && !spec.path().split('/').any(|part| part.is_empty() || part == ".."),
                "{}",
                spec.path()
            );
        }
    }

    #[test]
    fn the_variables_of_the_catalog_are_valid_and_listed_once() {
        for &id in PromptId::ALL {
            let mut seen = BTreeSet::new();
            for &variable in id.spec().variables() {
                assert!(variable.parse::<VariableName>().is_ok(), "{id:?}: {variable}");
                assert!(seen.insert(variable), "{id:?}: {variable} is listed twice");
            }
        }
    }

    #[test]
    fn the_rule_files_of_the_layout_are_in_the_catalog() {
        let paths: Vec<_> = PromptId::ALL.iter().map(|id| id.spec().path()).collect();

        assert_eq!(
            paths,
            [
                "clarification.md",
                "agent/rust-developer.md",
                "worker-agent/rust-developer.md",
                "investigator/rust-developer.md",
                "investigator/code-review.md",
            ]
        );
    }
}
