package main

import "fmt"

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
func RoundPROfTasks(tasks []MetaFile) RoundPRReference {
	for _, task := range tasks {
		if prNum, ok := task.Lookup("PR_NUMBER"); ok && prNum != "" {
			branch, _ := task.Lookup("BRANCH")
			return RoundPRReference{
				Number: prNum,
				Branch: branch,
			}
		}
	}
	for _, task := range tasks {
		if roundPR, ok := task.Lookup("ROUND_PR"); ok && roundPR != "" {
			return RoundPRReference{
				Number: roundPR,
				Branch: "",
			}
		}
	}
	return RoundPRReference{}
}

// FormatRoundPRNote formats the resume guidance note for a round's pull request.
// Matches the exact Bash round_pr_note text, including trailing newline.
func FormatRoundPRNote(merged bool, pr RoundPRReference) string {
	if pr.Number == "" {
		return ""
	}
	branchText := ""
	if pr.Branch != "" {
		branchText = " (branch " + pr.Branch + ")"
	}
	if merged {
		return fmt.Sprintf("This round's PR #%s%s is merged. Its follow-up goes in one new PR, opened by the coordinator from its branch; a member pushes its own branch and tells the coordinator what is on it (a board item or a message), and the coordinator merges it. A member never pushes to the coordinator's branch, and never opens, merges or closes a PR.\n", pr.Number, branchText)
	}
	pushTarget := "<its branch>"
	mergeSource := ""
	if pr.Branch != "" {
		pushTarget = pr.Branch
		mergeSource = "origin/" + pr.Branch
	}
	return fmt.Sprintf("This round's PR is #%s%s. Every commit of this round goes there, through the coordinator: a member pushes its own branch and tells the coordinator what is on it (a board item or a message); the coordinator merges the members' branches into its own and pushes it there (git push origin HEAD:%s) — a new coordinator first merges %s into its branch. A member never pushes to the coordinator's branch, and never opens, merges or closes a PR. Never open another PR for this round: the checks send a round with two open PRs back.\n", pr.Number, branchText, pushTarget, mergeSource)
}
