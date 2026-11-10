package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestRoundPROfTasks verifies two-pass PR_NUMBER and ROUND_PR metadata selection.
func TestRoundPROfTasks(t *testing.T) {
	tests := []struct {
		name       string
		tasks      []MetaFile
		wantNumber string
		wantBranch string
	}{
		{
			name:       "empty_tasks",
			tasks:      nil,
			wantNumber: "",
			wantBranch: "",
		},
		{
			name: "no_pr_in_tasks",
			tasks: []MetaFile{
				parseMeta([]byte("AGENT=lead\nSTATUS=done\n")),
				parseMeta([]byte("AGENT=member\nSTATUS=done\n")),
			},
			wantNumber: "",
			wantBranch: "",
		},
		{
			name: "single_task_pr_number_with_branch",
			tasks: []MetaFile{
				parseMeta([]byte("PR_NUMBER=42\nBRANCH=agents/lead/1\n")),
			},
			wantNumber: "42",
			wantBranch: "agents/lead/1",
		},
		{
			name: "single_task_pr_number_without_branch",
			tasks: []MetaFile{
				parseMeta([]byte("PR_NUMBER=42\n")),
			},
			wantNumber: "42",
			wantBranch: "",
		},
		{
			name: "single_task_round_pr",
			tasks: []MetaFile{
				parseMeta([]byte("ROUND_PR=55\nBRANCH=agents/lead/1\n")),
			},
			wantNumber: "55",
			wantBranch: "",
		},
		{
			name: "single_task_both_pr_number_and_round_pr",
			tasks: []MetaFile{
				parseMeta([]byte("ROUND_PR=55\nPR_NUMBER=42\nBRANCH=agents/lead/1\n")),
			},
			wantNumber: "42",
			wantBranch: "agents/lead/1",
		},
		{
			name: "precedence_pr_number_on_later_task_beats_earlier_round_pr",
			tasks: []MetaFile{
				parseMeta([]byte("ROUND_PR=10\nBRANCH=agents/t1\n")),
				parseMeta([]byte("PR_NUMBER=20\nBRANCH=agents/t2\n")),
			},
			wantNumber: "20",
			wantBranch: "agents/t2",
		},
		{
			name: "precedence_first_pr_number_wins",
			tasks: []MetaFile{
				parseMeta([]byte("PR_NUMBER=10\nBRANCH=agents/t1\n")),
				parseMeta([]byte("PR_NUMBER=20\nBRANCH=agents/t2\n")),
			},
			wantNumber: "10",
			wantBranch: "agents/t1",
		},
		{
			name: "precedence_first_round_pr_wins_when_no_pr_number",
			tasks: []MetaFile{
				parseMeta([]byte("STATUS=done\nBRANCH=agents/t0\n")),
				parseMeta([]byte("ROUND_PR=30\nBRANCH=agents/t1\n")),
				parseMeta([]byte("ROUND_PR=40\nBRANCH=agents/t2\n")),
			},
			wantNumber: "30",
			wantBranch: "",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := RoundPROfTasks(tc.tasks)
			if got.Number != tc.wantNumber || got.Branch != tc.wantBranch {
				t.Fatalf("RoundPROfTasks() = {%q, %q}, want {%q, %q}", got.Number, got.Branch, tc.wantNumber, tc.wantBranch)
			}
		})
	}
}

// TestRoundPROfTasksBashParity verifies exact parity with the Bash oracle round_pr_of_tasks.
func TestRoundPROfTasksBashParity(t *testing.T) {
	cases := []struct {
		name  string
		metas []string
	}{
		{
			name:  "empty",
			metas: nil,
		},
		{
			name:  "only_round_pr",
			metas: []string{"ROUND_PR=67\nBRANCH=agents/lead/1\n"},
		},
		{
			name:  "only_pr_number",
			metas: []string{"PR_NUMBER=72\nBRANCH=agents/lead/2\n"},
		},
		{
			name: "round_pr_then_pr_number",
			metas: []string{
				"ROUND_PR=67\nBRANCH=agents/t1\n",
				"PR_NUMBER=80\nBRANCH=agents/t2\n",
			},
		},
		{
			name: "pr_number_then_round_pr",
			metas: []string{
				"PR_NUMBER=80\nBRANCH=agents/t1\n",
				"ROUND_PR=67\nBRANCH=agents/t2\n",
			},
		},
		{
			name: "no_prs_at_all",
			metas: []string{
				"AGENT=lead\nSTATUS=done\n",
				"AGENT=worker\nSTATUS=done\n",
			},
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			tempDir := t.TempDir()
			var taskIDs []string
			var parsedTasks []MetaFile
			for i, meta := range tc.metas {
				id := "task" + string(rune('a'+i))
				taskIDs = append(taskIDs, id)
				taskDir := filepath.Join(tempDir, id)
				if err := os.MkdirAll(taskDir, 0o700); err != nil {
					t.Fatalf("failed to create task dir: %v", err)
				}
				if err := os.WriteFile(filepath.Join(taskDir, "meta.env"), []byte(meta), 0o600); err != nil {
					t.Fatalf("failed to write meta: %v", err)
				}
				parsedTasks = append(parsedTasks, parseMeta([]byte(meta)))
			}

			script := `
TASKS=$1
task_field() { sed -n "s/^$2=//p" "$TASKS/$1/meta.env"; }
round_pr_of_tasks() {
  local t p
  for t in "$@"; do
    p=$(task_field "$t" PR_NUMBER)
    if [[ -n $p ]]; then printf '%s %s' "$p" "$(task_field "$t" BRANCH)"; return 0; fi
  done
  for t in "$@"; do
    p=$(task_field "$t" ROUND_PR)
    if [[ -n $p ]]; then printf '%s' "$p"; return 0; fi
  done
}
shift
round_pr_of_tasks "$@"
`
			args := append([]string{tempDir}, taskIDs...)
			oracleOut := runMetaBash(t, "C.UTF-8", script, args...)

			got := RoundPROfTasks(parsedTasks)
			var gotFormatted string
			if got.Number != "" && got.Branch != "" {
				gotFormatted = got.Number + " " + got.Branch
			} else if got.Number != "" {
				gotFormatted = got.Number
			}

			if gotFormatted != oracleOut {
				t.Fatalf("RoundPROfTasks formatted = %q, oracle = %q", gotFormatted, oracleOut)
			}
		})
	}
}

// TestFormatRoundPRNote verifies resume guidance text formatting across all states.
func TestFormatRoundPRNote(t *testing.T) {
	tests := []struct {
		name       string
		merged     bool
		pr         RoundPRReference
		wantSubstr []string
		wantEmpty  bool
	}{
		{
			name:      "empty_number_merged",
			merged:    true,
			pr:        RoundPRReference{Number: "", Branch: "main"},
			wantEmpty: true,
		},
		{
			name:      "empty_number_unmerged",
			merged:    false,
			pr:        RoundPRReference{Number: "", Branch: ""},
			wantEmpty: true,
		},
		{
			name:   "merged_with_branch",
			merged: true,
			pr:     RoundPRReference{Number: "76", Branch: "agents/codex-1/20261110"},
			wantSubstr: []string{
				"This round's PR #76 (branch agents/codex-1/20261110) is merged.",
				"Its follow-up goes in one new PR, opened by the coordinator from its branch",
				"A member never pushes to the coordinator's branch, and never opens, merges or closes a PR.\n",
			},
		},
		{
			name:   "merged_without_branch",
			merged: true,
			pr:     RoundPRReference{Number: "76", Branch: ""},
			wantSubstr: []string{
				"This round's PR #76 is merged.",
				"Its follow-up goes in one new PR, opened by the coordinator from its branch",
				"A member never pushes to the coordinator's branch, and never opens, merges or closes a PR.\n",
			},
		},
		{
			name:   "unmerged_with_branch",
			merged: false,
			pr:     RoundPRReference{Number: "76", Branch: "agents/codex-1/20261110"},
			wantSubstr: []string{
				"This round's PR is #76 (branch agents/codex-1/20261110).",
				"git push origin HEAD:agents/codex-1/20261110",
				"a new coordinator first merges origin/agents/codex-1/20261110 into its branch.",
				"Never open another PR for this round: the checks send a round with two open PRs back.\n",
			},
		},
		{
			name:   "unmerged_without_branch",
			merged: false,
			pr:     RoundPRReference{Number: "76", Branch: ""},
			wantSubstr: []string{
				"This round's PR is #76.",
				"git push origin HEAD:<its branch>",
				"a new coordinator first merges  into its branch.",
				"Never open another PR for this round: the checks send a round with two open PRs back.\n",
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := FormatRoundPRNote(tc.merged, tc.pr)
			if tc.wantEmpty {
				if got != "" {
					t.Fatalf("FormatRoundPRNote() = %q, want empty", got)
				}
				return
			}
			if !strings.HasSuffix(got, "\n") {
				t.Fatalf("FormatRoundPRNote() missing trailing newline: %q", got)
			}
			for _, sub := range tc.wantSubstr {
				if !strings.Contains(got, sub) {
					t.Fatalf("FormatRoundPRNote() does not contain %q; got: %q", sub, got)
				}
			}
		})
	}
}

// TestFormatRoundPRNoteBashParity verifies byte-for-byte identity with the Bash oracle round_pr_note.
func TestFormatRoundPRNoteBashParity(t *testing.T) {
	cases := []struct {
		name   string
		merged string
		num    string
		branch string
	}{
		{"merged_with_branch", "1", "42", "agents/lead/branch"},
		{"merged_without_branch", "1", "42", ""},
		{"unmerged_with_branch", "0", "42", "agents/lead/branch"},
		{"unmerged_without_branch", "0", "42", ""},
		{"empty_number", "0", "", "agents/lead/branch"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			script := `
round_pr_note() {
  local merged=$1 num=$2 branch=$3
  [[ -n $num ]] || return 0
  if ((merged)); then
    printf "This round's PR #%s%s is merged. Its follow-up goes in one new PR, opened by the coordinator from its branch; a member pushes its own branch and tells the coordinator what is on it (a board item or a message), and the coordinator merges it. A member never pushes to the coordinator's branch, and never opens, merges or closes a PR.\n" \
      "$num" "${branch:+ (branch $branch)}"
    return 0
  fi
  printf "This round's PR is #%s%s. Every commit of this round goes there, through the coordinator: a member pushes its own branch and tells the coordinator what is on it (a board item or a message); the coordinator merges the members' branches into its own and pushes it there (git push origin HEAD:%s) — a new coordinator first merges %s into its branch. A member never pushes to the coordinator's branch, and never opens, merges or closes a PR. Never open another PR for this round: the checks send a round with two open PRs back.\n" \
    "$num" "${branch:+ (branch $branch)}" "${branch:-<its branch>}" "${branch:+origin/$branch}"
}
round_pr_note "$1" "$2" "$3"
`
			oracleOut := runMetaBash(t, "C.UTF-8", script, tc.merged, tc.num, tc.branch)
			mergedBool := tc.merged == "1"
			got := FormatRoundPRNote(mergedBool, RoundPRReference{Number: tc.num, Branch: tc.branch})

			if got != oracleOut {
				t.Fatalf("FormatRoundPRNote parity mismatch:\ngot:    %q\noracle: %q", got, oracleOut)
			}
		})
	}
}
