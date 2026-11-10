package main

// RoundPRReference identifies a round's pull request number and optional branch.
type RoundPRReference struct {
	// Number is the pull request number string, or empty if none.
	Number string
	// Branch is the branch name from task metadata, or empty if derived from ROUND_PR.
	Branch string
}

// RoundPROfTasks selects the round's PR reference across ordered task metadata:
// pass 1 checks PR_NUMBER across tasks (with task BRANCH);
// pass 2 checks ROUND_PR across tasks (with empty branch);
// returns zero RoundPRReference if none found.
func RoundPROfTasks(_ []MetaFile) RoundPRReference {
	return RoundPRReference{}
}

// FormatRoundPRNote formats the resume guidance note for a round's pull request.
// Matches the exact Bash round_pr_note text, including trailing newline.
func FormatRoundPRNote(_ bool, _ RoundPRReference) string {
	return ""
}
