// Round size: how big the change of a round is, counted the way the round limit counts it (README "Rounds"):
// added plus deleted lines of every changed file, tests included, except the files an exclusion pattern leaves out
// (generated code, vendored files, lock files). Binary files have no lines; they are listed, not counted.
//
// The counting is a pure function of `git diff --numstat -z` output and the patterns, so it is tested without git
// (roundsize_test.go — written first, see docs/tdd.md); `agentbus round-size` only runs git and prints the result.
package main

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
	return nil, nil // not implemented yet: the tests in roundsize_test.go come first
}

// checkPatterns validates exclusion patterns (see excluded for their meaning).
func checkPatterns(patterns []string) *CmdRoundSizeError {
	return nil // not implemented yet
}

// excluded reports whether a file is left out by one of the patterns.
func excluded(patterns []string, file string) bool {
	return false // not implemented yet
}

// countRound adds up the files that no pattern excludes.
func countRound(files []RoundFile, patterns []string) RoundSize {
	return RoundSize{} // not implemented yet
}
