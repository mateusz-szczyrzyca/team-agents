//! A questions file through its whole life, with the harness's own templates: composed, given its instructions, answered
//! with `ok` and with own text, marked ANSWERED, read back and handed to the agent.

// The helpers unwrap: a panic there is a failed test, which only `#[cfg(test)]` code may rely on (clippy.toml).
#[cfg(test)]
mod questions_file {
    use std::num::NonZeroU32;

    use agent_core::questions::{Answer, Choice, Entry, QuestionsFile, Verdict};

    /// The instructions template, as shipped.
    const INSTRUCTIONS: &str = include_str!("../../../prompts/harness/questions/instructions.md");

    /// The words that tell the agent a recommendation was accepted, as shipped.
    const ACCEPTED: &str = include_str!("../../../prompts/harness/questions/accepted.md");

    /// A question number from a literal.
    fn n(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).unwrap()
    }

    /// What the renderer of templates does (item 2.3) for the one variable these two templates use.
    fn fill(template: &str, name: &str, value: &str) -> String {
        template.replace(&format!("{{{{{name}}}}}"), value)
    }

    /// A file with two questions that have recommendations, with the instructions of the template.
    fn asked() -> QuestionsFile {
        let first = Entry::new(n(1), "Which store?", "a) in memory\nb) on disk")
            .unwrap()
            .recommend("a) — simpler (inv-opus)")
            .unwrap();
        let second = Entry::new(n(2), "Żółć or bile?", "")
            .unwrap()
            .recommend("bile — the key is taken")
            .unwrap();
        let mut file = QuestionsFile::new("Questions — task-20261111-153601-k3x9", vec![first, second]).unwrap();
        file.set_instructions(&fill(INSTRUCTIONS, "task_id", "task-20261111-153601-k3x9"));
        file
    }

    #[test]
    fn the_instructions_template_names_the_marker_and_the_command_and_fits_a_block_quote() {
        let note = fill(INSTRUCTIONS, "task_id", "task-1");

        assert!(note.contains("**Verdict:**"));
        assert!(note.contains("**ANSWERED**"));
        assert!(note.contains("`agent answer task-1`"));
        assert!(!note.contains("{{"));
        let mut file = asked();
        file.set_instructions(&note);
        let parsed = QuestionsFile::parse(&file.render()).unwrap();
        assert_eq!(parsed.instructions(), note.trim());
    }

    #[test]
    fn a_file_answered_with_ok_and_with_own_text_and_marked_answered_parses_back_to_those_answers() {
        let mut file = asked();
        assert!(file.render().contains("> Answer every question after **Verdict:**"));
        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();
        file.set_verdict(
            n(2),
            Verdict::from_text("Bile, and please rename the key.\nIt is confusing.").unwrap(),
        )
        .unwrap();
        file.mark_answered(true);

        // What the human saved in the editor is what the daemon reads.
        let saved = QuestionsFile::parse(&file.render()).unwrap();

        assert!(saved.is_answered());
        assert_eq!(
            saved.answers().unwrap(),
            [
                Answer {
                    number: n(1),
                    title: String::from("Which store?"),
                    choice: Choice::Recommendation(String::from("a) — simpler (inv-opus)"))
                },
                Answer {
                    number: n(2),
                    title: String::from("Żółć or bile?"),
                    choice: Choice::Own(String::from("Bile, and please rename the key.\nIt is confusing."))
                },
            ]
        );
    }

    #[test]
    fn the_file_before_the_human_answers_is_not_answered_and_cannot_be_handed_to_the_agent() {
        let saved = QuestionsFile::parse(&asked().render()).unwrap();

        assert!(!saved.is_answered());
        assert_eq!(saved.answers().unwrap_err().problems.len(), 2);
        assert_eq!(saved.expand(str::to_owned).unwrap_err().problems.len(), 2);
    }

    #[test]
    fn the_agent_gets_the_answers_without_the_instructions_and_the_marker() {
        let mut file = asked();
        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();
        file.set_verdict(n(2), Verdict::from_text("bile").unwrap()).unwrap();
        file.mark_answered(true);
        let saved = QuestionsFile::parse(&file.render()).unwrap();

        let handed = saved
            .expand(|recommendation| fill(ACCEPTED, "recommendation", recommendation).trim().to_owned())
            .unwrap();

        assert_eq!(
            handed,
            [
                "# Questions — task-20261111-153601-k3x9",
                "",
                "## 1. Which store?",
                "a) in memory",
                "b) on disk",
                "",
                "**Recommended:** a) — simpler (inv-opus)",
                "",
                "**Verdict:** ok — the human accepts the recommendation: a) — simpler (inv-opus)",
                "",
                "## 2. Żółć or bile?",
                "",
                "**Recommended:** bile — the key is taken",
                "",
                "**Verdict:** bile",
                "",
            ]
            .join("\n")
        );
    }

    #[test]
    fn a_file_the_human_edited_with_a_stray_marker_in_the_middle_is_not_answered() {
        let mut file = asked();
        file.set_verdict(n(1), Some(Verdict::Accept)).unwrap();
        let text = file.render().replacen("## 2.", "**ANSWERED**\n## 2.", 1);

        let saved = QuestionsFile::parse(&text).unwrap();

        assert!(!saved.is_answered());
        assert_eq!(saved.entries().len(), 2);
    }
}
