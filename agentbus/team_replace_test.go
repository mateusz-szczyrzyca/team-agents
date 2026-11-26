package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

// replacementFixture creates a team with a complete member record and a claimed item.
func replacementFixture(t *testing.T, b *Bus) TeamMember {
	t.Helper()
	createTeam(t, b, "T", map[string][3]string{"L": {"normal", "", "active"}, "M": {"normal", "openai", "active"}})
	original := TeamMember{Agent: "M", Task: "task-M", Branch: "agents/member", Worktree: "/tmp/member", State: MemberActive, Tier: TierNormal, Provider: "openai", BoardSeen: 7}
	mustWith(t, b, func(st *State) {
		team := st.Teams["T"]
		team.Lead = "L"
		*team.Members["M"] = original
		team.Board = map[string]*BoardItem{
			"B1": {ID: "B1", Status: BoardClaimed, Assignee: "M", Creator: "L", Level: LevelNormal, Progress: "in progress"},
			"B2": {ID: "B2", Status: BoardClaimed, Assignee: "L", Creator: "L", Level: LevelNormal},
		}
	})
	return original
}

// replacementMessage appends a quiet message without changing the fixture's member state.
func replacementMessage(t *testing.T, b *Bus, team, to, text string) Message {
	t.Helper()
	var m Message
	mustWith(t, b, func(st *State) {
		var err *StorageError
		m, err = b.postTo(st, binding{team: team}, "L", to, EventChat{Body: text, Quiet: true})
		if err != nil {
			t.Fatal(err)
		}
	})
	return m
}

// replacementState returns the persisted state for whole-value assertions.
func replacementState(t *testing.T, b *Bus) *State {
	t.Helper()
	st, err := b.load()
	if err != nil {
		t.Fatal(err)
	}
	return st
}

func TestTeamReplaceMovesMemberClaimsAndOnlyUnreadTeamMessages(t *testing.T) {
	b := newBus(t)
	original := replacementFixture(t, b)
	read := replacementMessage(t, b, "T", "M", "already read")
	chk(t).ok(toolResult(server(b, "M", inTeam("T")).read(0)))
	unread := replacementMessage(t, b, "T", "M", "still unread")
	other := replacementMessage(t, b, "other", "M", "other team")
	legacy := replacementMessage(t, b, "", "M", "unbound history")
	mustWith(t, b, func(st *State) { st.Cursors[messageCursor("N", "T")] = 999 })
	logs := captureLogs(t)

	out, err := teamReplace(b, "T", "M", "N")
	if err != nil {
		t.Fatalf("replacement failed: %v", err)
	}
	if out != "REPLACED M N" {
		t.Fatalf("output = %q", out)
	}
	st := replacementState(t, b)
	team := st.Teams["T"]
	original.Agent = "N"
	if !reflect.DeepEqual(team.Members["N"], &original) || team.Members["M"] != nil {
		t.Fatalf("members = %#v, want moved record %+v", team.Members, original)
	}
	if team.Lead != "L" || team.Board["B1"].Assignee != "N" || team.Board["B1"].Progress != "in progress" || team.Board["B2"].Assignee != "L" {
		t.Fatalf("team or claims changed incorrectly: %+v", team)
	}
	msgs, serr := b.messages()
	if serr != nil {
		t.Fatal(serr)
	}
	wantRecipients := map[int64]string{read.ID: "M", unread.ID: "N", other.ID: "M", legacy.ID: "M"}
	notices := 0
	for _, m := range msgs {
		if want, ok := wantRecipients[m.ID]; ok && m.To != want {
			t.Fatalf("message %d recipient = %q, want %q", m.ID, m.To, want)
		}
		if m.Kind == MessageKind("member_replaced") {
			notices++
			if m.To != "team:T" || m.Text != "M was replaced by N; send to N from now on." {
				t.Fatalf("notice = %+v", m)
			}
		}
	}
	if notices != 1 {
		t.Fatalf("replacement notices = %d, want 1", notices)
	}
	inherited := chk(t).ok(toolResult(server(b, "N", inTeam("T"), func(s *Server) { s.info.Task = "task-M" }).read(0)))
	if !strings.Contains(inherited, "still unread") || strings.Contains(inherited, "already read") || strings.Contains(inherited, "other team") {
		t.Fatalf("inherited unread = %q", inherited)
	}
	lead := server(b, "L", inTeam("T"))
	if got := chk(t).ok(toolResult(lead.read(0))); !strings.Contains(got, "M was replaced by N; send to N from now on.") {
		t.Fatalf("lead read = %q", got)
	}
	chk(t).ok(toolResult(lead.send("N", "new member")))
	_, _, refusal := bindSend(st, "L", "M", "", "T")
	failsWith(t, CmdSendNoSuchMember)("", refusal)
	count := 0
	for _, r := range records(t, logs) {
		if r["event"] == "member_replaced" {
			count++
			if r["team"] != "T" || r["from"] != "M" || r["to"] != "N" {
				t.Fatalf("replacement log = %v", r)
			}
		}
	}
	if count != 1 {
		t.Fatalf("replacement logs = %d, want 1", count)
	}
}

func TestTeamReplaceRefusalsLeaveStateAndMessagesUnchanged(t *testing.T) {
	cases := []struct {
		name     string              // refusal being exercised
		team     string              // selected team
		from     string              // outgoing member
		to       string              // incoming member
		finished bool                // whether the selected team has finished
		cause    CmdTeamReplaceCause // exact operation cause
	}{
		{"unknown team", "missing", "M", "N", false, CmdTeamReplaceNoSuchTeam},
		{"finished team", "T", "M", "N", true, CmdTeamReplaceNoSuchTeam},
		{"missing member", "T", "absent", "N", false, CmdTeamReplaceNoSuchMember},
		{"already member", "T", "M", "L", false, CmdTeamReplaceAlreadyMember},
		{"same agent", "T", "M", "M", false, CmdTeamReplaceSameAgent},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			b := newBus(t)
			replacementFixture(t, b)
			replacementMessage(t, b, "T", "M", "unchanged")
			mustWith(t, b, func(st *State) { st.Teams["T"].Done = tc.finished })
			before := replacementState(t, b)
			messages, err := os.ReadFile(b.path("messages.jsonl"))
			if err != nil {
				t.Fatal(err)
			}
			failsWith(t, tc.cause)(teamReplace(b, tc.team, tc.from, tc.to))
			if after := replacementState(t, b); !reflect.DeepEqual(after, before) {
				t.Fatalf("state changed: before %+v, after %+v", before, after)
			}
			after, err := os.ReadFile(b.path("messages.jsonl"))
			if err != nil {
				t.Fatal(err)
			}
			if string(after) != string(messages) {
				t.Fatal("refusal changed messages")
			}
		})
	}
}

func TestTeamReplacePreservesSleepingStateAndLeadRoleWithUnicodeNames(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"旧": {"normal", "", "sleeping"}})
	mustWith(t, b, func(st *State) { st.Teams["T"].Lead = "旧" })
	wakes := captureWakes(t)
	out, err := teamReplace(b, "T", "旧", "新")
	if err != nil {
		t.Fatal(err)
	}
	if out != "REPLACED 旧 新" {
		t.Fatalf("output = %q", out)
	}
	team := replacementState(t, b).Teams["T"]
	if team.Lead != "新" || team.Members["新"].State != MemberSleeping || len(*wakes) != 0 {
		t.Fatalf("role/state/wakes = %q, %+v, %v", team.Lead, team.Members["新"], *wakes)
	}
}

func TestTeamReplaceReportsStorageFailure(t *testing.T) {
	b := newBus(t)
	replacementFixture(t, b)
	if err := os.Mkdir(b.path("messages.jsonl"), 0700); err != nil {
		t.Fatal(err)
	}
	failsWith(t, CmdTeamReplaceStorage)(teamReplace(b, "T", "M", "N"))
	if st := replacementState(t, b); st.Teams["T"].Members["M"] == nil || st.Teams["T"].Members["N"] != nil {
		t.Fatal("failed storage moved the member")
	}
}

func TestTeamReplaceCLIOutputAndExitClasses(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	bin := filepath.Join(t.TempDir(), "agentbus")
	build := exec.CommandContext(ctx, "go", "build", "-o", bin, ".") // #nosec G204 -- fixed Go command builds only this package into a test-owned temporary directory.
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
	cases := []struct {
		name string // CLI outcome
		team string // team flag
		from string // outgoing member
		to   string // incoming member
		code int    // expected process exit status
	}{
		{"success", "T", "M", "N", 0},
		{"missing team", "missing", "M", "N", ClassNotFound.ExitCode()},
		{"missing member", "T", "absent", "N", ClassNotFound.ExitCode()},
		{"conflict", "T", "M", "L", ClassConflict.ExitCode()},
		{"same agent", "T", "M", "M", ClassInvalidArgument.ExitCode()},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			b := newBus(t)
			replacementFixture(t, b)
			cmd := exec.CommandContext(ctx, bin, "team-replace", "--dir", b.dir, "--team", tc.team, "--from", tc.from, "--to", tc.to) // #nosec G204 -- binary is built by this test into its temporary directory; all arguments are test fixtures.
			cmd.Env = append(os.Environ(), "AGENTS_HOME="+t.TempDir())
			out, err := cmd.Output()
			code := 0
			if err != nil {
				var exit *exec.ExitError
				if !errors.As(err, &exit) {
					t.Fatal(err)
				}
				code = exit.ExitCode()
			}
			if code != tc.code {
				t.Fatalf("exit = %d, want %d; error %v", code, tc.code, err)
			}
			if tc.code == 0 && string(out) != "REPLACED M N\n" {
				t.Fatalf("stdout = %q", out)
			}
			if tc.code != 0 && len(out) != 0 {
				t.Fatalf("refusal stdout = %q", out)
			}
		})
	}
}

// TestTeamReplacePreservesUnknownMessageFields covers forward-compatible log rewriting.
func TestTeamReplacePreservesUnknownMessageFields(t *testing.T) {
	b := newBus(t)
	replacementFixture(t, b)
	replacementMessage(t, b, "T", "M", "unread")
	path := b.path("messages.jsonl")
	raw, err := os.ReadFile(path) // #nosec G304 -- path is the message log in the test-owned temporary bus directory.
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		t.Fatal(err)
	}
	fields["future_field"] = json.RawMessage(`{"nested":true}`)
	raw, err = json.Marshal(fields)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, append(raw, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := teamReplace(b, "T", "M", "N"); err != nil {
		t.Fatal(err)
	}
	raw, err = os.ReadFile(path) // #nosec G304 -- path is the same test-owned temporary message log.
	if err != nil {
		t.Fatal(err)
	}
	first := strings.SplitN(string(raw), "\n", 2)[0]
	if err := json.Unmarshal([]byte(first), &fields); err != nil {
		t.Fatal(err)
	}
	if string(fields["future_field"]) != `{"nested":true}` || string(fields["to"]) != `"N"` {
		t.Fatalf("rewritten fields = %s", first)
	}
}

func TestTeamReplaceKeepsReceiptOffsetAtTheSameMessageBoundary(t *testing.T) {
	b := newBus(t)
	from := strings.Repeat("old-member-", 30)
	createTeam(t, b, "T", map[string][3]string{from: {"normal", "", "active"}})
	replacementMessage(t, b, "T", from, "unread before replacement")
	mustWith(t, b, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, Status: HelperRunning, AttemptRun: "run-1"}
	})
	if _, err := teamReplace(b, "T", from, "N"); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(b.path("state.json"))
	if err != nil {
		t.Fatal(err)
	}
	var st State
	if err := json.Unmarshal(raw, &st); err != nil {
		t.Fatal(err)
	}
	messages, err := os.ReadFile(b.path("messages.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	boundary := int64(strings.IndexByte(string(messages), '\n') + 1)
	if st.OwnedReceiptOffset != boundary {
		t.Fatalf("receipt offset = %d, want boundary %d", st.OwnedReceiptOffset, boundary)
	}
	replacementState(t, b)
}
