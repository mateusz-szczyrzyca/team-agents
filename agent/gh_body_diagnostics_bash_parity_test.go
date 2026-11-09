package main

import (
	"os"
	"strings"
	"testing"
)

// TestPRFormatIssuesBashParity verifies exact parity with Bash C.UTF-8 [[:space:]] behavior.
// Some Unicode spaces are matched by Bash (U+2003 em space) and some are rejected (U+00A0 NBSP).
func TestPRFormatIssuesBashParity(t *testing.T) {
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

	// C.UTF-8 parity cases: distinguish Bash [[:space:]] matches from broader Unicode spaces.
	cases := []struct {
		name string
		body string
		want []string
	}{
		{
			name: "fence with em space (U+2003) - matches Bash [[:space:]]",
			body: "``` \n",
			want: []string{fenceIssue},
		},
		{
			name: "fence with no-break space (U+00A0) - NOT in Bash [[:space:]]",
			body: "``` \n",
			want: []string{}, // NBSP is NOT a Bash space, so fence is considered labeled
		},
		{
			name: "fence with figure space (U+2007) - NOT in Bash [[:space:]]",
			body: "``` \n",
			want: []string{}, // Figure space is NOT a Bash space
		},
		{
			name: "fence with narrow no-break space (U+202F) - NOT in Bash [[:space:]]",
			body: "``` \n",
			want: []string{}, // Narrow NBSP is NOT a Bash space
		},
		{
			name: "leading em space before fence (U+2003)",
			body: " ```\n",
			want: []string{fenceIssue},
		},
		{
			name: "leading no-break space before fence (U+00A0)",
			body: " ```\n",
			want: []string{}, // NBSP is NOT a Bash [[:space:]], so line doesn't match fence regex at all
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			want := strings.Join(tc.want, "\n")
			if len(tc.want) > 0 {
				want += "\n"
			}
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
