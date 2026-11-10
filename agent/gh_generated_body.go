package main

import (
	"regexp"
	"strings"
	"unicode"
)

// RemoveGeneratedLines removes lines matching "Generated with Claude Code" from the body text,
// along with any trailing blank line after such a line. It matches the Bash gh_publish_text logic
// for removing Claude Code's own attribution line before publishing to GitHub.
func RemoveGeneratedLines(body string) string {
	// Regex pattern matches: optional spaces, optional emoji+spaces, "Generated with", optional brackets around "Claude Code", optional parentheses
	// This matches variations like:
	// - Generated with Claude Code
	// - 🤖 Generated with Claude Code
	// - Generated with [Claude Code]
	// - Generated with Claude Code(v1.0)
	generatedRe := regexp.MustCompile(`^\s*(?:🤖\s*)?Generated with \[?Claude Code\]?(?:\([^)]*\))?\s*$`)

	lines := strings.Split(body, "\n")
	var result []string
	drop := false

	for _, line := range lines {
		if generatedRe.MatchString(line) {
			drop = true
			continue
		}

		// If we just removed a Generated line and this line is all whitespace, skip it
		if drop && isBlankLine(line) {
			drop = false
			continue
		}

		drop = false
		result = append(result, line)
	}

	return strings.Join(result, "\n")
}

// isBlankLine returns true if the line contains only whitespace characters.
func isBlankLine(line string) bool {
	for _, r := range line {
		if !unicode.IsSpace(r) {
			return false
		}
	}
	return true
}
