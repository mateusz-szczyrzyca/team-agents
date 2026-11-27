//! The briefings of a worker, a reviewer and an investigator that take a job over from its last checkpoint, and the
//! paragraph that names a reviewer's findings file, rendered against their golden texts.

#[cfg(test)]
mod harness_job_briefing_prompts {
    use std::path::Path;

    use agent_core::prompt::{HarnessPrompt, PROMPTS_DIR, Variables};

    /// The rendering of `prompt` with `values`, and the golden text called `golden`.
    fn rendered_and_golden(prompt: HarnessPrompt, values: &Variables, golden: &str) -> (String, String) {
        let spec = prompt.spec();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
        let text = std::fs::read_to_string(root.join(PROMPTS_DIR).join(spec.path())).unwrap();
        let golden = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/job-briefings")
                .join(golden),
        )
        .unwrap();
        (spec.render(&text, values).unwrap(), golden)
    }

    fn checkpoint(meant: &str, done: &str, left: &str, cause: &str) -> String {
        format!(
            "## Checkpoint: {cause} at 2026-11-27T04:00:00Z, head 0123abc\n\n### What is to be done\n{meant}\n\n### What is done\n{done}\n\n### What is left\n{left}"
        )
    }

    #[test]
    fn the_briefing_of_a_worker_renders_as_its_golden_text() {
        let told = checkpoint(
            "R1. Count the headings.\nK1. test -s docs/count.txt",
            "Commits: 1\n- a1b2c3d",
            "K1 has not passed yet.\nnext: make the check pass and commit",
            "commit",
        );
        let values = Variables::new().with("checkpoint", told).unwrap();

        let (rendered, golden) = rendered_and_golden(HarnessPrompt::BriefingWorker, &values, "worker.txt");

        assert_eq!(rendered, golden);
    }

    #[test]
    fn the_briefing_of_a_reviewer_renders_as_its_golden_text() {
        let told = checkpoint(
            "R1. Review pull request #7.",
            "Findings written so far: 1\n1. [MAJOR] ERR-3 a.rs:4 — a panic → return an error",
            "next: finish the review of pass 2 and submit it with a verdict",
            "findings_written",
        );
        let values = Variables::new()
            .with("pull_request", "7")
            .unwrap()
            .with("pass", "2")
            .unwrap()
            .with("checkpoint", told)
            .unwrap();

        let (rendered, golden) = rendered_and_golden(HarnessPrompt::BriefingReview, &values, "review.txt");

        assert_eq!(rendered, golden);
    }

    #[test]
    fn the_briefing_of_an_investigator_renders_as_its_golden_text() {
        let told = checkpoint(
            "R1. Answer the question: Why?",
            "Commits: 0\nChanged files: notes.md",
            "next: answer the question",
            "run_ended",
        );
        let values = Variables::new().with("checkpoint", told).unwrap();

        let (rendered, golden) =
            rendered_and_golden(HarnessPrompt::BriefingInvestigation, &values, "investigation.txt");

        assert_eq!(rendered, golden);
    }

    #[test]
    fn the_paragraph_of_the_findings_file_names_the_file() {
        let values = Variables::new()
            .with("file", "/tmp/review-pr7-pass2-findings.md")
            .unwrap();

        let (rendered, golden) = rendered_and_golden(HarnessPrompt::ReviewFindingsFile, &values, "findings-file.txt");

        assert_eq!(rendered, golden);
    }
}
