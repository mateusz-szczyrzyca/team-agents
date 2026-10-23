package main

// Tests of the round size counter — the worked example of docs/tdd.md. They were written before the code, in this
// order, and every step was committed separately (see `git log -- agentbus/roundsize*.go`):
//
//  1. RED: this file plus roundsize.go with the signatures only (bodies return zero values). Everything compiles,
//     every test fails on an assertion — not on a compile error, which would say nothing about the behaviour.
//  2. GREEN: the simplest code that passes these tests.
//  3. REFACTOR: cleaner code, the tests unchanged and still green.
//
// What to notice:
//   - Each test states one rule of the round limit in its name and in its table: the tests are the specification
//     a reviewer reads first.
//   - The counter is a pure function of git's output: the tests feed it bytes instead of running git, so they are
//     fast and cannot touch a real repository. Only the thin subcommand runs git (TestRoundSizeCommand, with a
//     temporary repository).
//   - Failures are checked by their cause (errors.As and the cause enum), never by comparing messages.

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// numstat builds `git diff --numstat -z` output: each record ends with NUL; a rename has an empty path field
// followed by the old and the new path, each ending with NUL.
func numstat(records ...string) []byte {
	out := ""
	for _, r := range records {
		out += r + "\x00"
	}
	return []byte(out)
}

// Rule: every line git reports is read — ordinary files, binary files ("-"), renames (the new path counts).
func TestParseNumstat(t *testing.T) {
	out := numstat("12\t3\tagentbus/main.go", "-\t-\tdocs/logo.png", "4\t4\t", "old/name.go", "new/name.go", "0\t7\tREADME.md")
	got, err := parseNumstat(out)
	if err != nil {
		t.Fatalf("parseNumstat: %v", err)
	}
	want := []RoundFile{
		{Path: "agentbus/main.go", Added: 12, Deleted: 3},
		{Path: "docs/logo.png", Binary: true},
		{Path: "new/name.go", Added: 4, Deleted: 4},
		{Path: "README.md", Deleted: 7},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("parseNumstat:\n got %+v\nwant %+v", got, want)
	}
}

// Rule: output we cannot read is an error with its own cause — a limit computed from garbage would be worse than
// no limit at all.
func TestParseNumstatRejectsGarbage(t *testing.T) {
	for _, bad := range [][]byte{
		numstat("12\tagentbus/main.go"),   // a field missing
		numstat("x\t3\tagentbus/main.go"), // not a number
		numstat("4\t4\t", "old/name.go"),  // a rename without its new path
		[]byte("12\t3\tagentbus/main.go"), // no terminating NUL (not -z output)
	} {
		_, err := parseNumstat(bad)
		var e *CmdRoundSizeError
		if !errors.As(err.asError(), &e) || e.Cause != CmdRoundSizeBadNumstat {
			t.Errorf("parseNumstat(%q): %v, want cause BadNumstat", bad, err)
		}
	}
	if files, err := parseNumstat(nil); err != nil || len(files) != 0 {
		t.Errorf("no changes: %v %v", files, err)
	}
}

// Rule: three kinds of exclusion patterns —
//   - without "/": the file name ("*_gen.go", "go.sum");
//   - ending in "/": a directory anywhere in the path ("vendor/");
//   - any other: the whole path (path.Match: "docs/*.png").
func TestExcluded(t *testing.T) {
	patterns := []string{"*_gen.go", "go.sum", "vendor/", "docs/*.png"}
	cases := map[string]bool{
		"agentbus/errors_gen.go":             true,
		"agentbus/go.sum":                    true,
		"live/web/static/vendor/htmx.min.js": true,
		"vendor/x.go":                        true,
		"docs/logo.png":                      true,
		"agentbus/main.go":                   false,
		"docs/img/logo.png":                  false, // "docs/*.png" is the whole path: * does not cross "/"
		"vendors/x.go":                       false, // "vendor/" is a whole directory name
		"agentbus/gen.go":                    false,
	}
	for file, want := range cases {
		if got := excluded(patterns, file); got != want {
			t.Errorf("excluded(%q) = %v, want %v", file, got, want)
		}
	}
}

// Rule: a malformed pattern is refused before anything is counted (a typo would silently count everything).
func TestCheckPatterns(t *testing.T) {
	if err := checkPatterns([]string{"*_gen.go", "vendor/", "docs/*.png"}); err != nil {
		t.Errorf("good patterns refused: %v", err)
	}
	for _, bad := range []string{"[", "docs/[a-", "/"} {
		err := checkPatterns([]string{bad})
		var e *CmdRoundSizeError
		if !errors.As(err.asError(), &e) || e.Cause != CmdRoundSizeBadPattern {
			t.Errorf("checkPatterns(%q): %v, want cause BadPattern", bad, err)
		}
	}
}

// Rule: the size is added + deleted lines of the counted files, tests included; excluded and binary files are
// listed (sorted) but add nothing.
func TestCountRound(t *testing.T) {
	files := []RoundFile{
		{Path: "agentbus/roundsize.go", Added: 120},
		{Path: "agentbus/roundsize_test.go", Added: 200, Deleted: 10},
		{Path: "agentbus/errors_gen.go", Added: 900},
		{Path: "live/web/static/vendor/htmx.min.js", Added: 1},
		{Path: "docs/logo.png", Binary: true},
	}
	got := countRound(files, []string{"*_gen.go", "vendor/"})
	want := RoundSize{Lines: 330, Added: 320, Deleted: 10, Files: 2,
		Excluded: []string{"agentbus/errors_gen.go", "live/web/static/vendor/htmx.min.js"}, Binary: []string{"docs/logo.png"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("countRound:\n got %+v\nwant %+v", got, want)
	}
}

// git runs git in a test repository, isolated from the user's configuration (a signing key or hooks there would
// make the test depend on the machine).
func git(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"}, args...)...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GIT_CONFIG_GLOBAL="+os.DevNull, "GIT_CONFIG_NOSYSTEM=1")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}

// Rule: the size is measured from the merge base of the base branch — a renamed file counts its changed lines,
// not its whole content, and excluded files are listed. The only test that runs git (a temporary repository).
func TestRoundSizeOfARepository(t *testing.T) {
	repo := t.TempDir()
	write := func(name, text string) {
		t.Helper()
		if err := os.MkdirAll(filepath.Dir(filepath.Join(repo, name)), 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(repo, name), []byte(text), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	git(t, repo, "init", "-q", "-b", "main")
	write("a.go", "package a\n\nfunc A() {}\n")
	write("old.go", strings.Repeat("// line\n", 40))
	git(t, repo, "add", ".")
	git(t, repo, "commit", "-q", "-m", "base")
	git(t, repo, "checkout", "-q", "-b", "round")
	write("a.go", "package a\n\nfunc A() { B() }\n")             // 1 changed line: +1 -1
	write("b.go", "package a\n\nfunc B() {}\n")                  // +3
	write("errors_gen.go", strings.Repeat("// generated\n", 50)) // excluded
	git(t, repo, "mv", "old.go", "renamed.go")                   // a pure rename: 0 lines
	git(t, repo, "add", ".")
	git(t, repo, "commit", "-q", "-m", "round 1")

	got, err := roundSizeOf(repo, "main", []string{"*_gen.go"})
	if err != nil {
		t.Fatalf("roundSizeOf: %v", err)
	}
	want := RoundSize{Lines: 5, Added: 4, Deleted: 1, Files: 3, Excluded: []string{"errors_gen.go"}, Binary: []string{}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("roundSizeOf:\n got %+v\nwant %+v", got, want)
	}

	for _, c := range []struct {
		repo, base string
		patterns   []string
		want       CmdRoundSizeCause
	}{
		{"", "main", nil, CmdRoundSizeMissingArgument},
		{repo, "", nil, CmdRoundSizeMissingArgument},
		{repo, "--output=/tmp/x", nil, CmdRoundSizeBadBase}, // would be an option of git diff
		{repo, "no-such-branch", nil, CmdRoundSizeGit},
		{t.TempDir(), "main", nil, CmdRoundSizeGit}, // not a repository
		{repo, "main", []string{"["}, CmdRoundSizeBadPattern},
	} {
		_, err := roundSizeOf(c.repo, c.base, c.patterns)
		var e *CmdRoundSizeError
		if !errors.As(err.asError(), &e) || e.Cause != c.want {
			t.Errorf("roundSizeOf(%q, %q, %q): %v, want cause %v", c.repo, c.base, c.patterns, err, c.want)
		}
	}
}
