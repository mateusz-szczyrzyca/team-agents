// Round size: how big the change of a round is, counted the way the round limit counts it (README "Rounds"):
// added plus deleted lines of every changed file, tests included, except the files an exclusion pattern leaves out
// (generated code, vendored files, lock files). Binary files have no lines; they are listed, not counted.
//
// The counting is a pure function of `git diff --numstat -z` output and the patterns, so it is tested without git
// (roundsize_test.go — written first, see docs/tdd.md); `agentbus round-size` only runs git and prints the result.
package main

import (
	"bytes"
	"os/exec"
	"path"
	"sort"
	"strconv"
	"strings"
)

// RoundFile is one changed file as git reports it.
type RoundFile struct {
	Path    string // the path after the change (for a rename: the new path)
	Added   int    // added lines (0 for a binary file)
	Deleted int    // deleted lines (0 for a binary file)
	Binary  bool   // git cannot count its lines ("-" in --numstat)
}

// RoundSize is a counted change.
type RoundSize struct {
	Lines    int      `json:"lines"`    // added + deleted lines of the counted files: what the limit compares
	Added    int      `json:"added"`    // added lines of the counted files
	Deleted  int      `json:"deleted"`  // deleted lines of the counted files
	Files    int      `json:"files"`    // counted files
	Excluded []string `json:"excluded"` // files left out by an exclusion pattern, sorted
	Binary   []string `json:"binary"`   // binary files (no lines), sorted
}

// parseNumstat parses the output of `git diff --numstat -z`.
func parseNumstat(out []byte) ([]RoundFile, *CmdRoundSizeError) {
	if len(out) == 0 {
		return nil, nil
	}
	if out[len(out)-1] != 0 {
		return nil, opErr(CmdRoundSizeBadNumstat, "git diff --numstat output does not end with NUL (not -z output)")
	}
	records := strings.Split(string(out[:len(out)-1]), "\x00")
	var files []RoundFile
	for i := 0; i < len(records); i++ {
		fields := strings.SplitN(records[i], "\t", 3)
		if len(fields) != 3 {
			return nil, opErr(CmdRoundSizeBadNumstat, "malformed --numstat record %q", records[i])
		}
		f := RoundFile{Path: fields[2]}
		if f.Path == "" { // a rename or a copy: the old and the new path follow
			if i+2 >= len(records) {
				return nil, opErr(CmdRoundSizeBadNumstat, "a rename without its paths after %q", records[i])
			}
			f.Path = records[i+2]
			i += 2
		}
		var ok bool
		if f.Added, f.Deleted, f.Binary, ok = parseCounts(fields[0], fields[1]); !ok {
			return nil, opErr(CmdRoundSizeBadNumstat, "line counts of %s are not numbers: %q %q", f.Path, fields[0], fields[1])
		}
		files = append(files, f)
	}
	return files, nil
}

// parseCounts reads the two counts of a --numstat record; "-" "-" is a binary file. ok is false for anything else.
func parseCounts(a, d string) (added, deleted int, binary, ok bool) {
	if a == "-" && d == "-" {
		return 0, 0, true, true
	}
	added, aerr := strconv.Atoi(a)
	deleted, derr := strconv.Atoi(d)
	return added, deleted, false, aerr == nil && derr == nil
}

// checkPatterns validates exclusion patterns (see excluded for their meaning).
func checkPatterns(patterns []string) *CmdRoundSizeError {
	for _, p := range patterns {
		glob := strings.TrimSuffix(p, "/")
		if glob == "" {
			return opErr(CmdRoundSizeBadPattern, "empty exclusion pattern %q", p)
		}
		if _, err := path.Match(glob, ""); err != nil {
			return opErr(CmdRoundSizeBadPattern, "exclusion pattern %q: %v", p, err)
		}
	}
	return nil
}

// patternKind is what an exclusion pattern is matched against.
type patternKind int

const (
	patternName patternKind = iota // no "/": the file name ("*_gen.go", "go.sum")
	patternDir                     // ends in "/": a directory anywhere in the path ("vendor/")
	patternPath                    // any other: the whole path, with path.Match ("docs/*.png")
)

// String returns the kind's name.
func (k patternKind) String() string {
	switch k {
	case patternName:
		return "name"
	case patternDir:
		return "directory"
	case patternPath:
		return "path"
	}
	return "patternKind(" + strconv.Itoa(int(k)) + ")"
}

// kindOf classifies an exclusion pattern.
func kindOf(p string) patternKind {
	switch {
	case strings.HasSuffix(p, "/"):
		return patternDir
	case strings.Contains(p, "/"):
		return patternPath
	}
	return patternName
}

// excluded reports whether a file is left out by one of the patterns.
func excluded(patterns []string, file string) bool {
	for _, p := range patterns {
		var ok bool
		var err error
		switch kindOf(p) {
		case patternDir:
			ok = strings.Contains("/"+path.Dir(file)+"/", "/"+p)
		case patternName:
			ok, err = path.Match(p, path.Base(file))
		case patternPath:
			ok, err = path.Match(p, file)
		}
		if err != nil {
			continue // cannot happen: checkPatterns refuses malformed patterns before anything is counted
		}
		if ok {
			return true
		}
	}
	return false
}

// countRound adds up the files that no pattern excludes.
func countRound(files []RoundFile, patterns []string) RoundSize {
	size := RoundSize{Excluded: []string{}, Binary: []string{}}
	for _, f := range files {
		switch {
		case excluded(patterns, f.Path):
			size.Excluded = append(size.Excluded, f.Path)
		case f.Binary:
			size.Binary = append(size.Binary, f.Path)
		default:
			size.Added += f.Added
			size.Deleted += f.Deleted
			size.Files++
		}
	}
	size.Lines = size.Added + size.Deleted
	sort.Strings(size.Excluded)
	sort.Strings(size.Binary)
	return size
}

// roundSizeOf counts the change of HEAD against base in a repository: `git diff --numstat -z -M base...HEAD`
// (from the merge base, so commits that reached the base meanwhile do not count).
func roundSizeOf(repo, base string, patterns []string) (RoundSize, *CmdRoundSizeError) {
	if repo == "" || base == "" {
		return RoundSize{}, opErr(CmdRoundSizeMissingArgument, "%s", missing("repo", "base"))
	}
	if strings.HasPrefix(base, "-") {
		return RoundSize{}, opErr(CmdRoundSizeBadBase, "base %q is not a ref name", base)
	}
	if err := checkPatterns(patterns); err != nil {
		return RoundSize{}, err
	}
	// --no-color and --no-ext-diff: the user's git configuration must not change the format read below
	cmd := exec.Command("git", "-C", repo, "diff", "--no-color", "--no-ext-diff", "--numstat", "-z", "-M", base+"...HEAD") // #nosec G204 -- fixed git arguments; base is checked not to be an option
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return RoundSize{}, opErr(CmdRoundSizeGit, "git diff %s...HEAD in %s: %v: %s", base, repo, err, strings.TrimSpace(stderr.String()))
	}
	files, perr := parseNumstat(out)
	if perr != nil {
		return RoundSize{}, perr
	}
	return countRound(files, patterns), nil
}

// cmdRoundSize — round-size: the size of a round's change (README "Rounds"), as text or JSON.
//
//	agentbus round-size --repo DIR --base REF [--exclude "*_gen.go vendor/"] [--json]
func cmdRoundSize(args []string) {
	fs := newFlagSet("round-size")
	repo := fs.String("repo", ".", "the repository (a worktree)")
	base := fs.String("base", "", "the base ref the round is measured against (e.g. origin/main)")
	exclude := fs.String("exclude", "", "exclusion patterns, separated by spaces")
	asJSON := fs.Bool("json", false, "print JSON")
	parseFlags(fs, args)
	size, err := roundSizeOf(*repo, *base, strings.Fields(*exclude))
	if err != nil {
		exitOn(err.asError())
		return
	}
	if *asJSON {
		exitOn(printJSON(size).asError())
		return
	}
	var out cliWriter
	out.printf("round size: %d lines (+%d -%d) in %d files\n", size.Lines, size.Added, size.Deleted, size.Files)
	if len(size.Excluded) > 0 {
		out.printf("not counted (excluded): %s\n", strings.Join(size.Excluded, ", "))
	}
	if len(size.Binary) > 0 {
		out.printf("not counted (binary): %s\n", strings.Join(size.Binary, ", "))
	}
	exitOn(out.err.asError())
}
