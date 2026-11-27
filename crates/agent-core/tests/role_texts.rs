//! Each role text tells the run what to keep for the next session of its task: the plan file and the notebook.

/// The role texts a run is given, by file name, each with the text of the file.
const ROLE_TEXTS: [(&str, &str); 5] = [
    (
        "role-coordinator.md",
        include_str!("../../../prompts/harness/run/role-coordinator.md"),
    ),
    (
        "role-member.md",
        include_str!("../../../prompts/harness/run/role-member.md"),
    ),
    (
        "role-solo.md",
        include_str!("../../../prompts/harness/run/role-solo.md"),
    ),
    (
        "role-worker.md",
        include_str!("../../../prompts/harness/run/role-worker.md"),
    ),
    (
        "role-reviewer.md",
        include_str!("../../../prompts/harness/run/role-reviewer.md"),
    ),
];

#[test]
fn every_role_text_names_the_plan_file_and_the_notebook_for_the_next_session() {
    for (name, text) in ROLE_TEXTS {
        assert!(text.contains("`.agent/plan.md`"), "{name} does not name the plan file");
        assert!(text.contains("notebook"), "{name} does not name the notebook");
    }
}

const FINDINGS_PARAGRAPH: &str = "\
When you find something the next agent on these files must know — a test that fails without a cause in the
code (`flaky_test`), code or a document that misleads (`trap`), a tool or the machine behaving unexpectedly
(`environment`) — call `finding_add` with the files it concerns and two or three sentences: what happens, how
to recognise it, what to do. Progress notes stay in `notebook_append`. When you fix the cause of a finding, or
see that it is wrong, call `finding_resolve` with its id and the reason.";

#[test]
fn solo_worker_and_reviewer_role_texts_contain_the_findings_paragraph() {
    for name in ["role-solo.md", "role-worker.md", "role-reviewer.md"] {
        let text = ROLE_TEXTS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t).unwrap();
        assert!(
            text.contains(FINDINGS_PARAGRAPH),
            "{name} does not contain the findings paragraph"
        );
    }
}
