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
	"reflect"
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
