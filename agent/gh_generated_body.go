package main

// RemoveGeneratedLines removes lines matching "Generated with Claude Code" from the body text,
// along with any trailing blank line after such a line. It matches the Bash gh_publish_text logic
// for removing Claude Code's own attribution line before publishing to GitHub.
func RemoveGeneratedLines(_ string) string {
	return ""
}
