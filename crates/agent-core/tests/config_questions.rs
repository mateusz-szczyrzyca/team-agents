//! `[questions]` of the configuration: how a new questions file is opened and which editor `agent answer` starts.

use agent_core::config::{Config, ConfigError, OpenQuestions};

/// The smallest configuration with a section `questions` appended.
fn with(questions: &str) -> String {
    format!("{questions}\n")
}

#[test]
fn without_the_section_nothing_is_opened_and_no_editor_is_named() {
    let config = Config::from_toml_str("").unwrap();

    let questions = config.questions();
    assert_eq!(questions.open, OpenQuestions::Off);
    assert_eq!(questions.vscode_server_dirs, []);
    assert_eq!(questions.editor, None);
}

#[test]
fn the_section_sets_the_way_to_open_the_servers_and_the_editor() {
    let config = Config::from_toml_str(&with(
        "[questions]\nopen = \"vscode\"\nvscode_server_dirs = [\"/home/you/.vscode-server\"]\neditor = [\"nano\", \"-w\"]",
    ))
    .unwrap();

    let questions = config.questions();
    assert_eq!(questions.open, OpenQuestions::Vscode);
    assert_eq!(
        questions
            .vscode_server_dirs
            .iter()
            .map(|dir| dir.as_path().to_str().unwrap())
            .collect::<Vec<_>>(),
        ["/home/you/.vscode-server"]
    );
    let editor = questions.editor.as_ref().unwrap();
    assert_eq!(editor.program().as_path().to_str(), Some("nano"));
    assert_eq!(editor.args().as_slice(), ["-w"]);
}

#[test]
fn a_way_to_open_that_is_not_known_is_refused_and_the_error_lists_the_known_ones() {
    let error = Config::from_toml_str("[questions]\nopen = \"emacs\"\n").unwrap_err();

    let ConfigError::Parse(source) = &error else {
        panic!("{error:?}");
    };
    let text = source.to_string();
    assert!(text.contains("off") && text.contains("vscode"), "{text}");
}

#[test]
fn a_relative_server_directory_is_refused() {
    let error = Config::from_toml_str("[questions]\nvscode_server_dirs = [\".vscode-server\"]\n").unwrap_err();

    assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
}

#[test]
fn an_unknown_key_of_the_section_is_refused() {
    let error = Config::from_toml_str("[questions]\nopne = \"off\"\n").unwrap_err();

    assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
}
