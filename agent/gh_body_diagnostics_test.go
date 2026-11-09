package main

import (
	"os"
	"strings"
	"testing"
)

// TestPRFormatIssues preserves the source-level github_format contract and its heuristics.
func TestPRFormatIssues(t *testing.T) {
	// Extract the self-contained frozen function so CLI startup cannot affect its output.
	source, err := os.ReadFile("../bin/agent")
	if err != nil {
		t.Fatal(err)
	}
	oracleSource := string(source)
	start := strings.Index(oracleSource, "\npr_format_issues() {")
	if start < 0 {
		t.Fatal("Bash oracle function missing")
	}
	end := strings.Index(oracleSource[start:], "\n}\n")
	if end < 0 {
		t.Fatal("Bash oracle function terminator missing")
	}
	function := oracleSource[start : start+end+2]
	// defaultRepo is the Bash fallback used in diagnostic examples.
	const defaultRepo = "https://github.com/<owner>/<repo>"
	// fenceIssue names the exact diagnostic for an unlabeled backtick fence.
	const fenceIssue = "a code block without its language: open it with ```go, ```bash, ```json…"
	// redIssue and greenIssue retain the public formatting instructions verbatim.
	const redIssue = "RED as a plain word: write $\\color{red}{\\textbf{RED}}$ and $\\color{green}{\\textbf{GREEN}}$ (bold, in colour)"
	const greenIssue = "GREEN as a plain word: write $\\color{red}{\\textbf{RED}}$ and $\\color{green}{\\textbf{GREEN}}$ (bold, in colour)"
	// cases exercise complete diagnostics rather than just matching token substrings.
	cases := []struct {
		// name identifies the source behavior.
		name string
		// body is the Markdown supplied by a caller.
		body string
		// repo selects diagnostic links; empty exercises the default.
		repo string
		// want lists diagnostic lines in their observable order.
		want []string
	}{
		{name: "empty"},
		{name: "valid linked body", body: "Fixed in [`53b8659`](https://github.com/o/r/commit/53b8659f00d) after [#21](https://github.com/o/r/pull/21), `1727` lines.\n\n```go\nfunc f() {}\n```\n\n$\\color{red}{\\textbf{RED}}$, then $\\color{green}{\\textbf{GREEN}}$."},
		{name: "all original unlinked tokens", body: "The commits `53b8659` and `b2bc702d5892b546ba0d013023543824b9898a62` fix it; see `#21`.", want: []string{
			"commit `53b8659` without a link: write [`53b8659`](" + defaultRepo + "/commit/<its full SHA>)",
			"commit `b2bc702d5892b546ba0d013023543824b9898a62` without a link: write [`b2bc702`](" + defaultRepo + "/commit/<its full SHA>)",
			"PR `#21` without a link: write [#21](" + defaultRepo + "/pull/21)",
		}},
		{name: "custom URL and order", repo: "https://github.com/example/project", body: "`#9` `abcdef0` `#10` `abcdef1` GREEN:", want: []string{
			"commit `abcdef0` without a link: write [`abcdef0`](https://github.com/example/project/commit/<its full SHA>)",
			"commit `abcdef1` without a link: write [`abcdef1`](https://github.com/example/project/commit/<its full SHA>)",
			"PR `#9` without a link: write [#9](https://github.com/example/project/pull/9)",
			"PR `#10` without a link: write [#10](https://github.com/example/project/pull/10)",
			greenIssue,
		}},
		{name: "unlabeled fence", body: "Run:\n```\nmake gate\n```", want: []string{fenceIssue}},
		{name: "whitespace fence and CRLF", body: "\t``` \t\r\n`abcdef0` RED:\r\n```\r\n**GREEN**:\r\n", want: []string{fenceIssue, greenIssue}},
		{name: "fenced contents ignored", body: "```bash\necho `53b8659` RED:\n```"},
		{name: "unclosed labeled fence", body: " ```bash\n`#2` RED:"},
		{name: "plain TDD", body: "RED: the test fails.\n**GREEN**: it passes.", want: []string{redIssue, greenIssue}},
		{name: "one TDD diagnostic per line", body: "RED: GREEN:\n{RED:\nwordRED:\nred:\nGREEN :", want: []string{redIssue}},
		{name: "color anywhere suppresses TDD", body: "RED: \\color custom\n**GREEN**: \\color"},
		{name: "numeric short uppercase and too long tokens", body: "`1234567` `abc123` `ABCDEF0` `abcdef01234567890123456789012345678901234`"},
		{name: "bracket heuristic", body: "[`abcdef0`] [`#12`]\n`#0012`", want: []string{
			"PR `#0012` without a link: write [#0012](" + defaultRepo + "/pull/0012)",
		}},
		{name: "digit token does not stop scan", body: "`1234567` and `abcdef0`", want: []string{
			"commit `abcdef0` without a link: write [`abcdef0`](" + defaultRepo + "/commit/<its full SHA>)",
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// want includes Bash's final newline only when there are diagnostics.
			want := strings.Join(tc.want, "\n")
			if len(tc.want) > 0 {
				want += "\n"
			}
			// The established helper isolates startup hooks and fixes POSIX character classes.
			oracle := runMetaBash(t, "C", function+"\n"+`pr_format_issues "$1" "$2"`, tc.body, tc.repo)
			if oracle != want {
				t.Fatalf("Bash oracle = %q, want %q", oracle, want)
			}
			if got := PRFormatIssues(tc.body, tc.repo); got != want {
				t.Errorf("PRFormatIssues() = %q, want %q", got, want)
			}
		})
	}
}
