package main

import (
	"os"
	"strconv"
	"strings"
	"testing"
)

// TestGitHubNameChecksum compares the native CRC, including length folding, with POSIX cksum.
func TestGitHubNameChecksum(t *testing.T) {
	for _, name := range []string{"", "a", "abc", "unknown", "inv_one@example", "Unicode-λ", strings.Repeat("long-name", 100)} {
		t.Run(name, func(t *testing.T) {
			// want is independent of fallback palette size, so CRC bugs cannot hide in a modulus.
			want := strings.Fields(runMetaBash(t, "C.UTF-8", `printf '%s' "$1" | cksum`, name))[0]
			if got := strconv.FormatUint(uint64(gitHubNameChecksum(name)), 10); got != want {
				t.Errorf("checksum = %s, POSIX = %s", got, want)
			}
		})
	}
}

// renderOracle extracts pure helpers without executing the installed CLI's startup.
func renderOracle(t *testing.T) string {
	t.Helper()
	// source is the frozen production Bash oracle, never edited by native helpers.
	source, err := os.ReadFile("../bin/agent")
	if err != nil {
		t.Fatal(err)
	}
	// out defines only the three functions needed for pure formatting parity.
	out := ""
	for _, name := range []string{"gh_colour_names", "gh_colour_recommendation", "gh_publish_text"} {
		// start and end delimit a function rather than sourcing CLI configuration.
		start := strings.Index(string(source), "\n"+name+"() {")
		if start < 0 {
			t.Fatalf("missing oracle %s", name)
		}
		end := strings.Index(string(source)[start:], "\n}\n")
		if end < 0 {
			t.Fatalf("unterminated oracle %s", name)
		}
		out += string(source)[start:start+end+2] + "\n"
	}
	return out
}

// TestRenderGitHubNames checks shape matching, cksum fallback and per-body allocation.
func TestRenderGitHubNames(t *testing.T) {
	// oracle preserves the exact current Bash rendering contract.
	oracle := renderOracle(t)
	// cases deliberately include raw whitespace, malformed-but-accepted colors and exhaustion.
	cases := []struct {
		// name labels the behavioral rule.
		name string
		// body is untrusted Markdown, never executed by the test.
		body string
		// colors and palette are raw runner environment values.
		colors, palette string
	}{
		{name: "disabled keeps bytes", body: "- agent: `alice` (model)\n\n", palette: "red blue"},
		{name: "known", body: "- coordinator: `alice` (model)", colors: "alice=#123abc"},
		{name: "underscore escape", body: "\t- member: `agent_one` (model)", colors: "agent_one=#ABCDEF"},
		{name: "investigator", body: "*Review by the team-agents investigator `inv-one@example` (model).*", colors: "inv-one@example=red"},
		{name: "checksum fallback", body: "- agent: `unknown` (model)", colors: "known=red", palette: "red green blue yellow purple"},
		{name: "collision and repeated name", body: "- member: `alice` (a)\n- member: `bob` (b)\n- member: `alice` (a)", colors: "alice=red bob=red", palette: "red green blue"},
		{name: "exhausted palette", body: "- member: `a` (a)\n- member: `b` (b)\n- member: `c` (c)", colors: "a=red b=red c=red", palette: "red blue"},
		{name: "duplicate last wins", body: "- agent: `a` (a)", colors: "a=red a=blue"},
		{name: "empty known falls back", body: "- agent: `a` (a)", colors: "a=", palette: "red blue"},
		{name: "no fallback", body: "- agent: `unknown` (a)\n", colors: "a=red"},
		{name: "only allowed names and exact roles", body: "- member: `a+b` (a)\n- Member: `a` (a)\nmember: `a` (a)\n- agent: `a`(a)\n- agent: `a` (a)", colors: "a=red"},
		{name: "IFS excludes CR and unicode", body: "- agent: `a` (a)", colors: "a=red\r\u2003b=blue", palette: "red\r blue"},
		{name: "palette read uses first line", body: "- agent: `b` (a)", colors: "a=red", palette: "red\nblue green"},
		{name: "tabs in environment", body: "- agent: `a` (a)\n- member: `b` (b)", colors: "a=red\tb=blue", palette: "red\tblue"},
		{name: "unicode prefix whitespace", body: "\u2003-\u2009member:\u3000`a` (a)", colors: "a=red"},
		{name: "excluded NBSP", body: "\u00a0- member: `a` (a)", colors: "a=red"},
		{name: "trailing newlines", body: "- agent: `a` (a)\n\n", colors: "a=red"},
		{name: "ambiguous investigator prefix", body: "- member: `a` (Review by the team-agents investigator `b` (b))", colors: "a=red b=blue"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// want is byte-exact output including the original trailing newlines.
			want := runMetaBash(t, "C.UTF-8", oracle+`AGENT_GITHUB_NAME_COLORS=$2; AGENT_GITHUB_COLORS=$3; gh_colour_names "$1"`, tc.body, tc.colors, tc.palette)
			if got := RenderGitHubNames(tc.body, GitHubColorConfig{NameColors: tc.colors, Palette: tc.palette}); got != want {
				t.Errorf("names = %q, Bash = %q", got, want)
			}
		})
	}
}

// TestRenderGitHubRecommendation protects the state machine and exact verdict replacement.
func TestRenderGitHubRecommendation(t *testing.T) {
	// oracle runs independently of native verdict classification.
	oracle := renderOracle(t)
	// bodies cover headings, bold removal, icons, case, priority and whitespace boundaries.
	bodies := []string{
		"", "plain body\n\n", "Recommendation: Ready to merge", "## Recommendation\n\n**Needs changes**: fix it\n",
		"__Recommendation:__ 🧩 __needs fixes__ (incomplete)\n", "Recommendation: NEEDS FIXES then Ready to merge",
		"Recommendation:\nNo verdict\nReady to merge\nRecommendation: Needs changes", "RECOMMENDATION: Ready to merge",
		"Recommendationary: Ready to merge", "Recommendationé: Ready to merge", "Recommendation1: Ready to merge",
		"Recommendation\u0345: Ready to merge", "Recommendation\u2160: Ready to merge", "Recommendation\u00b2: Ready to merge",
		"####### Recommendation: Ready to merge", "#Recommendation: Ready to merge", "*Recommendation: ready to merge*",
		"Recommendation: ✅ **Ready to merge** now", "Recommendation: 🔧 Needs changes; incomplete implementation",
		"Recommendation: $\\color{red}{old}$ Ready to merge", "Recommendation:\n\t\u2003\nneeds changes\nNeeds fixes",
		"Recommendation:\n\u00a0\nReady to merge", "\u2003Recommendation:\u2009Needs fixes",
		"Recommendation: **Needs changes** and **more**", "Recommendation: __Ready to merge__\n\n",
		"Recommendation:\n## Recommendation\nready to merge", "Recommendation: ready to merge\nRecommendation: Needs changes",
		"Recommendation: needs fixes __later__", "Recommendation: **ready to merge unmatched",
		"Recommendation: İ says **Ready to merge**", "Recommendation: 😀 needs fixes (INCOMPLETE)",
		"Recommendation: 🧩🔧✅ **Ready to merge**", "Recommendation: **needs changes** (Ready to merge)\r\n",
	}
	for _, body := range bodies {
		t.Run(body, func(t *testing.T) {
			// want is emitted without shell command substitution, retaining terminal newlines.
			want := runMetaBash(t, "C.UTF-8", oracle+`gh_colour_recommendation "$1"`, body)
			if got := RenderGitHubRecommendation(body); got != want {
				t.Errorf("recommendation = %q, Bash = %q", got, want)
			}
		})
	}
}

// TestTransformGitHubBodies freezes publication order, including prepended byline coloring.
func TestTransformGitHubBodies(t *testing.T) {
	// oracle models the Bash sentinel-preserving review and publish paths.
	oracle := renderOracle(t)
	// cfg provides both a remembered collision and an available alternative.
	cfg := GitHubColorConfig{NameColors: "alice=red inv_one=red", Palette: "red blue"}
	// byline has the exact syntax consumed by the name-rendering helper.
	byline := "*Review by the team-agents investigator `inv_one` (model).*"
	// bodies expose blank-line removal, recommendation ordering, and idempotent byline insertion.
	bodies := []string{"", "\n", "Generated with Claude Code\n\nRecommendation: Needs changes\n- member: `alice` (model)\n\n", byline + "\n\nRecommendation: Ready to merge", "\n" + byline + "\nRecommendation:\nNeeds fixes"}
	for _, body := range bodies {
		t.Run(body, func(t *testing.T) {
			// setup installs only test-provided raw color values.
			setup := oracle + `AGENT_GITHUB_NAME_COLORS=$3; AGENT_GITHUB_COLORS=$4; `
			wantPublish := runMetaBash(t, "C.UTF-8", setup+`gh_publish_text "$1"`, body, byline, cfg.NameColors, cfg.Palette)
			if got := TransformPublishBody(body, cfg); got != wantPublish {
				t.Errorf("publish = %q, Bash = %q", got, wantPublish)
			}
			wantReview := runMetaBash(t, "C.UTF-8", setup+`body=$1; first=${body#$'\n'}; first=${first%%$'\n'*}; if [[ $first != *"Review by the team-agents investigator"* ]]; then body=$2$'\n\n'$body; fi; body=$(gh_colour_recommendation "$body"; printf .); gh_publish_text "${body%.}"`, body, byline, cfg.NameColors, cfg.Palette)
			if got := TransformReviewBody(body, byline, cfg); got != wantReview {
				t.Errorf("review = %q, Bash = %q", got, wantReview)
			}
		})
	}
}
