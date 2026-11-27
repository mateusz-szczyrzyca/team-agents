//! The findings section is a model prompt with exactly one substitution.
use agent_core::prompt::{Audience, HarnessPrompt, Variables};

#[test]
fn findings_catalog_entry_renders_the_specified_section() {
    assert!(HarnessPrompt::ALL.contains(&HarnessPrompt::Findings));
    let spec = HarnessPrompt::Findings.spec();
    assert_eq!(spec.path(), "harness/run/findings.md");
    assert_eq!(spec.audience(), Audience::Model);
    assert_eq!(spec.variables(), &["findings"]);
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../prompts")
            .join(spec.path()),
    )
    .unwrap();
    let values = Variables::new()
        .with("findings", "- F-1 [trap] docs/x — lesson (agent-a, 2026-11-27)")
        .unwrap();
    assert_eq!(
        spec.render(&text, &values).unwrap(),
        "## Findings of other agents on the files of your work\n\nOther agents left these notes about files your work touches. Check what applies before you rely on the code; if a\nfinding is wrong or you fixed its cause, call `finding_resolve` with its id and the reason.\n\n- F-1 [trap] docs/x — lesson (agent-a, 2026-11-27)\n"
    );
}
