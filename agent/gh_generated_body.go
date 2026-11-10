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
	// Optional leading emoji and its trailing Bash whitespace are stripped before matching.
	generatedRe := regexp.MustCompile(`^Generated with \[?Claude Code\]?(?:\([^)]*\))?$`)

	lines := strings.Split(body, "\n")
	var result []string
	drop := false

	for _, line := range lines {
		// Strip leading and trailing Bash whitespace for matching
		trimmed := trimBashSpaces(line)
		target := trimmed
		if strings.HasPrefix(target, "🤖") {
			target = strings.TrimLeftFunc(strings.TrimPrefix(target, "🤖"), isBashSpace)
		}
		if generatedRe.MatchString(target) {
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
	return strings.TrimFunc(s, isBashSpace)
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
