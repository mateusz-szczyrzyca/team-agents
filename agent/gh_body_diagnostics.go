package main

import (
	"fmt"
	"regexp"
	"strings"
)

// PRFormatIssues returns the Bash-compatible, newline-terminated PR-body diagnostics.
// An empty repository URL uses the documentation placeholder; clean bodies return empty text.
func PRFormatIssues(body, repoURL string) string {
	if repoURL == "" {
		repoURL = "https://github.com/<owner>/<repo>"
	}
	// POSIX matching preserves Bash's leftmost-longest diagnostic selection.
	fence := regexp.MustCompilePOSIX("^[[:space:]]*```(.*)$")
	blank := regexp.MustCompilePOSIX("^[[:space:]]*$")
	commit := regexp.MustCompilePOSIX("(^|[^[])`([0-9a-f]{7,40})`(.*)$")
	pr := regexp.MustCompilePOSIX("(^|[^[])`#([0-9]+)`(.*)$")
	tdd := regexp.MustCompilePOSIX(`(^|[^a-zA-Z{])(\*\*)?(RED|GREEN)(\*\*)?:`)
	// issues follows Bash's per-line ordering: commits, PRs, then TDD labels.
	var issues []string
	// inFence suppresses prose checks until the next backtick fence.
	inFence := false
	for _, line := range strings.Split(body, "\n") {
		if match := fence.FindStringSubmatch(line); match != nil {
			if !inFence && blank.MatchString(match[1]) {
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
