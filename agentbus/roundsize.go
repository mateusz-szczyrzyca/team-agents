// Round size: how big the change of a round is, counted the way the round limit counts it (README "Rounds"):
// added plus deleted lines of every changed file, tests included, except the files an exclusion pattern leaves out
// (generated code, vendored files, lock files). Binary files have no lines; they are listed, not counted.
//
// The counting is a pure function of `git diff --numstat -z` output and the patterns, so it is tested without git
// (roundsize_test.go — written first, see docs/tdd.md); `agentbus round-size` only runs git and prints the result.
package main

import (
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
		if fields[0] == "-" && fields[1] == "-" {
			f.Binary = true
		} else {
			added, aerr := strconv.Atoi(fields[0])
			deleted, derr := strconv.Atoi(fields[1])
			if aerr != nil || derr != nil {
				return nil, opErr(CmdRoundSizeBadNumstat, "line counts of %s are not numbers: %q %q", f.Path, fields[0], fields[1])
			}
			f.Added, f.Deleted = added, deleted
		}
		files = append(files, f)
	}
	return files, nil
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

// excluded reports whether a file is left out by one of the patterns.
func excluded(patterns []string, file string) bool {
	for _, p := range patterns {
		if strings.HasSuffix(p, "/") { // a directory anywhere in the path
			if strings.Contains("/"+path.Dir(file)+"/", "/"+p) {
				return true
			}
			continue
		}
		subject := file // a pattern with "/" matches the whole path…
		if !strings.Contains(p, "/") {
			subject = path.Base(file) // …one without it, the file name
		}
		ok, err := path.Match(p, subject)
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
