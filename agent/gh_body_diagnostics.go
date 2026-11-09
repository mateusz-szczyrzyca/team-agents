package main

import (
	"fmt"
	"regexp"
	"strings"
	"unicode"
)

// isBashSpace reports whether c is a space character in Bash's C.UTF-8 [[:space:]] set.
// Matches ASCII whitespace and most Unicode spaces, but excludes U+0085, U+00A0, U+2007, U+202F.
func isBashSpace(r rune) bool {
	// ASCII whitespace: space, tab, newline, vertical tab, form feed, carriage return.
	if r == ' ' || r == '\t' || r == '\n' || r == '\v' || r == '\f' || r == '\r' {
		return true
	}
	// Explicitly exclude Unicode spaces not in Bash glibc's [[:space:]].
	// U+0085 (NEL), U+00A0 (NBSP), U+2007 (Figure Space), U+202F (Narrow NBSP).
	if r == '\u0085' || r == ' ' || r == ' ' || r == ' ' {
		return false
	}
	// Include other Unicode spaces via unicode.IsSpace, excluding the ones above.
	return unicode.IsSpace(r)
}

// stripLeadingBashSpace removes leading characters that match Bash's [[:space:]] set.
func stripLeadingBashSpace(s string) string {
	for i, c := range s {
		if !isBashSpace(c) {
			return s[i:]
		}
	}
	return ""
}

// PRFormatIssues returns the Bash-compatible, newline-terminated PR-body diagnostics.
// An empty repository URL uses the documentation placeholder; clean bodies return empty text.
func PRFormatIssues(body, repoURL string) string {
	if repoURL == "" {
		repoURL = "https://github.com/<owner>/<repo>"
	}
	// Custom fence detector: strip leading Bash spaces, then check for backtick fence.
	fenceRegex := regexp.MustCompile("^```(.*)$")
	isBlankFence := func(fenceLanguage string) bool {
		for _, c := range fenceLanguage {
			if !isBashSpace(c) {
				return false
			}
		}
		return true
	}
	commit := regexp.MustCompilePOSIX("(^|[^[])`([0-9a-f]{7,40})`(.*)$")
	pr := regexp.MustCompilePOSIX("(^|[^[])`#([0-9]+)`(.*)$")
	tdd := regexp.MustCompilePOSIX(`(^|[^a-zA-Z{])(\*\*)?(RED|GREEN)(\*\*)?:`)
	// issues follows Bash's per-line ordering: commits, PRs, then TDD labels.
	var issues []string
	// inFence suppresses prose checks until the next backtick fence.
	inFence := false
	for _, line := range strings.Split(body, "\n") {
		// Check for fence: strip Bash spaces first, then match backticks.
		stripped := stripLeadingBashSpace(line)
		if match := fenceRegex.FindStringSubmatch(stripped); match != nil {
			if !inFence && isBlankFence(match[1]) {
				issues = append(issues, "a code block without its language: open it with ```go, ```bash, ```json…")
			}
			inFence = !inFence
			continue
		}
		if inFence {
			continue
		}
		for rest := line; ; {
			match := commit.FindStringSubmatch(rest)
			if match == nil {
				break
			}
			rest = match[3]
			if strings.ContainsAny(match[2], "abcdef") {
				issues = append(issues, fmt.Sprintf("commit `%s` without a link: write [`%s`](%s/commit/<its full SHA>)", match[2], match[2][:7], repoURL))
			}
		}
		for rest := line; ; {
			match := pr.FindStringSubmatch(rest)
			if match == nil {
				break
			}
			rest = match[3]
			issues = append(issues, fmt.Sprintf("PR `#%s` without a link: write [#%s](%s/pull/%s)", match[2], match[2], repoURL, match[2]))
		}
		if match := tdd.FindStringSubmatch(line); match != nil && !strings.Contains(line, `\color`) {
			issues = append(issues, fmt.Sprintf(`%s as a plain word: write $\color{red}{\textbf{RED}}$ and $\color{green}{\textbf{GREEN}}$ (bold, in colour)`, match[3]))
		}
	}
	if len(issues) == 0 {
		return ""
	}
	return strings.Join(issues, "\n") + "\n"
}
