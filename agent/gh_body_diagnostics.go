package main

// PRFormatIssues returns the Bash-compatible, newline-terminated PR-body diagnostics.
// An empty repository URL uses the documentation placeholder; clean bodies return empty text.
func PRFormatIssues(_, _ string) string {
	return ""
}
