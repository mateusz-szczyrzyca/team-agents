//! Pull request size under the developer rules, independent of the checkout.
use super::{Git, GitError, GitOperation, Reach, Revision, arg, text};
use agent_core::state::CommitId;
impl Git {
    /// Counts added and deleted lines outside tests using the PR-1 diff and exclusions.
    ///
    /// # Errors
    /// Returns the errors of the bounded git diff command.
    pub async fn pull_request_size(&self, base: &Revision, head: &CommitId) -> Result<usize, GitError> {
        let range = format!("{base}...{head}");
        let operation = GitOperation::Diff;
        let captured = self
            .run_ok(
                operation,
                Reach::Local,
                &[
                    arg("diff"),
                    arg("-U100000"),
                    arg("--no-ext-diff"),
                    arg("--no-textconv"),
                    arg("-M"),
                    arg(&range),
                    arg("--"),
                    arg("."),
                    arg(":!*Cargo.lock"),
                    arg(":!*.snap"),
                    arg(":!tests/*"),
                    arg(":!tests.rs"),
                    arg(":!*/tests/*"),
                    arg(":!*/tests.rs"),
                    arg(":!benches/*"),
                    arg(":!*/benches/*"),
                ],
            )
            .await?;
        let patch = text(operation, &captured.stdout)?;
        let mut header = true;
        let mut test = false;
        let mut armed = false;
        let mut count = 0usize;
        for line in patch.lines() {
            if line.starts_with("diff --git ") {
                header = true;
                test = false;
                armed = false;
                continue;
            }
            if header {
                if line.starts_with("@@") {
                    header = false;
                }
                continue;
            }
            let source = line.get(1..).unwrap_or_default();
            if !test && source.starts_with("#[cfg(test)]") {
                armed = true;
                continue;
            }
            if !test && armed && !source.is_empty() && !source.starts_with([' ', '#', ')']) {
                let declaration = source
                    .strip_prefix("pub ")
                    .or_else(|| source.strip_prefix("pub(crate) "))
                    .unwrap_or(source);
                if declaration.starts_with("mod ") && declaration.contains('{') {
                    test = true;
                }
                armed = false;
            }
            if !test && !armed && line.starts_with(['+', '-']) {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }
}
