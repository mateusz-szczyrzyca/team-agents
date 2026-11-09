package main

import (
	"os"
	"strings"
	"testing"
)

// TestPRFormatIssuesUTF8 verifies C.UTF-8 fence-whitespace parity with the Bash oracle.
// The Bash runner uses LC_ALL=C.UTF-8; this test ensures Go diagnostics match.
func TestPRFormatIssuesUTF8(t *testing.T) {
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

	const fenceIssue = "a code block without its language: open it with ```go, ```bash, ```json…"

	// C.UTF-8 regression cases: fence followed by Unicode whitespace under C.UTF-8 locale.
	cases := []struct {
		name string
		body string
		want []string
	}{
		{
			name: "fence with em space (U+2003)",
			body: "``` \n",
			want: []string{fenceIssue},
		},
		{
			name: "fence with em space before language",
			body: "```  go\n",
			want: []string{},
		},
		{
			name: "fence with multiple em spaces",
			body: "```   \n",
			want: []string{fenceIssue},
		},
		{
			name: "fence with tab and em space",
			body: "```\t \n",
			want: []string{fenceIssue},
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			want := strings.Join(tc.want, "\n")
			if len(tc.want) > 0 {
				want += "\n"
			}
			// Run with C.UTF-8 locale to match the runner's actual environment.
			oracle := runMetaBash(t, "C.UTF-8", function+"\n"+`pr_format_issues "$1" "$2"`, tc.body, "")
			if oracle != want {
				t.Fatalf("Bash C.UTF-8 oracle = %q, want %q", oracle, want)
			}
			if got := PRFormatIssues(tc.body, ""); got != want {
				t.Errorf("PRFormatIssues() = %q, want %q", got, want)
			}
		})
	}
}
