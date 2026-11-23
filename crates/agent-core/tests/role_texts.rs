//! Each role text tells the run what to keep for the next session of its task: the plan file and the notebook.

/// The role texts a run is given, by file name, each with the text of the file.
const ROLE_TEXTS: [(&str, &str); 5] = [
    (
        "role-coordinator.md",
        include_str!("../../../prompts/harness/run/role-coordinator.md"),
    ),
    ("role-member.md", include_str!("../../../prompts/harness/run/role-member.md")),
    ("role-solo.md", include_str!("../../../prompts/harness/run/role-solo.md")),
    ("role-worker.md", include_str!("../../../prompts/harness/run/role-worker.md")),
    ("role-reviewer.md", include_str!("../../../prompts/harness/run/role-reviewer.md")),
];

#[test]
fn every_role_text_names_the_plan_file_and_the_notebook_for_the_next_session() {
    for (name, text) in ROLE_TEXTS {
        assert!(text.contains("`.agent/plan.md`"), "{name} does not name the plan file");
        assert!(text.contains("notebook"), "{name} does not name the notebook");
    }
}
