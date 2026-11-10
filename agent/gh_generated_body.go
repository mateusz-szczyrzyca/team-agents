package main

import (
	"regexp"
	"strings"
)

// RemoveGeneratedLines removes lines matching "Generated with Claude Code" from the body text,
// along with any trailing blank line after such a line. It matches the Bash gh_publish_text logic
// for removing Claude Code's own attribution line before publishing to GitHub.
// Uses Bash [[:space:]] semantics for whitespace matching via isBashSpace.
func RemoveGeneratedLines(body string) string {
	// Regex pattern matches the core pattern after stripping leading/trailing Bash whitespace.
	// Pattern: optional emoji+whitespace, "Generated with", optional brackets, optional parentheses.
	// Handles variations like:
	// - Generated with Claude Code
	// - 🤖 Generated with Claude Code
	// - Generated with [Claude Code]
	// - Generated with Claude Code(v1.0)
	// Leading/trailing whitespace is handled separately using isBashSpace to match Bash [[:space:]].
	generatedRe := regexp.MustCompile(`^(?:🤖[ \t\v\f\r]*)?Generated with \[?Claude Code\]?(?:\([^)]*\))?$`)

	lines := strings.Split(body, "\n")
	var result []string
	drop := false

	for _, line := range lines {
		// Strip leading and trailing Bash whitespace for matching
		trimmed := trimBashSpaces(line)
		if generatedRe.MatchString(trimmed) {
			drop = true
			continue
		}

		// If we just removed a Generated line and this line is all Bash whitespace, skip it.
		// Use isBashSpace to match Bash [[:space:]] exactly.
		if drop && isBashBlankLine(line) {
			drop = false
			continue
		}

		drop = false
		result = append(result, line)
	}

	return strings.Join(result, "\n")
}

// trimBashSpaces removes leading and trailing characters that match Bash's [[:space:]] set.
func trimBashSpaces(s string) string {
	start := 0
	for i, r := range s {
		if !isBashSpace(r) {
			start = i
			break
		}
	}
	if start >= len(s) {
		return ""
	}
	end := len(s)
	for i := len(s) - 1; i >= start; i-- {
		if !isBashSpace(rune(s[i])) {
			end = i + 1
			break
		}
	}
	return s[start:end]
}

// isBashBlankLine returns true if all characters in the line are Bash whitespace.
// Matches Bash [[:space:]] semantics: ASCII whitespace plus most Unicode spaces,
// excluding U+0085, U+00A0, U+2007, U+202F.
func isBashBlankLine(line string) bool {
	for _, r := range line {
		if !isBashSpace(r) {
			return false
		}
	}
	return true
}
